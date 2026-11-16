//! Accessibility and parent-window ownership for the two native search panels.
use accesskit_macos::SubclassingAdapter;
use eframe::egui::{self, accesskit};
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSView, NSWindow, NSWindowOrderingMode};
use objc2_foundation::MainThreadMarker;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex},
};

thread_local! {
    static PANELS: RefCell<HashMap<egui::ViewportId, Panel>> = RefCell::new(HashMap::new());
}

struct Panel {
    adapter: SubclassingAdapter,
    window: Retained<NSWindow>,
    tree: Arc<Mutex<Option<accesskit::TreeUpdate>>>,
    requests: Arc<Mutex<Vec<accesskit::ActionRequest>>>,
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(parent) = self.window.parentWindow() {
            parent.removeChildWindow(&self.window);
        }
    }
}
struct Activation(Arc<Mutex<Option<accesskit::TreeUpdate>>>);
impl accesskit::ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<accesskit::TreeUpdate> {
        self.0.lock().unwrap().clone()
    }
}
struct Actions {
    requests: Arc<Mutex<Vec<accesskit::ActionRequest>>>,
    ctx: egui::Context,
    viewport: egui::ViewportId,
}
impl accesskit::ActionHandler for Actions {
    fn do_action(&mut self, request: accesskit::ActionRequest) {
        self.requests.lock().unwrap().push(request);
        self.ctx.request_repaint_of(self.viewport);
        self.ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

#[derive(Default)]
struct Bridge {
    // Immediate viewport output is delivered after egui restores its parent context.
    passes: Vec<egui::ViewportId>,
}
impl egui::Plugin for Bridge {
    fn debug_name(&self) -> &'static str {
        "Native panel accessibility"
    }
    fn setup(&mut self, ctx: &egui::Context) {
        ctx.enable_accesskit();
    }
    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        self.passes.push(input.viewport_id);
        PANELS.with(|panels| {
            if let Some(panel) = panels.borrow().get(&input.viewport_id) {
                input.events.extend(
                    panel
                        .requests
                        .lock()
                        .unwrap()
                        .drain(..)
                        .map(egui::Event::AccessKitActionRequest),
                );
            }
        });
    }
    fn output_hook(&mut self, _ctx: &egui::Context, output: &mut egui::FullOutput) {
        let Some(viewport) = self.passes.pop() else {
            return;
        };
        let events = PANELS.with(|panels| {
            let mut panels = panels.borrow_mut();
            if viewport == egui::ViewportId::ROOT {
                panels.retain(|id, _| output.viewport_output.contains_key(id));
            }
            let Some(panel) = panels.get_mut(&viewport) else {
                return Vec::new();
            };
            let mut events = Vec::new();
            if let Some(update) = output.platform_output.accesskit_update.take() {
                *panel.tree.lock().unwrap() = Some(update.clone());
                if let Some(event) = panel.adapter.update_if_active(|| update) {
                    events.push(event);
                }
            }
            if let Some(event) = panel
                .adapter
                .update_view_focus_state(panel.window.isKeyWindow())
            {
                events.push(event);
            }
            events
        });
        // AX notifications may reenter the adapter through AppKit.
        for event in events {
            event.raise();
        }
    }
    fn on_exit(&mut self, _ctx: &egui::Context) {
        PANELS.with(|panels| panels.borrow_mut().clear());
        self.passes.clear();
    }
}

pub fn install(ctx: &egui::Context) {
    ctx.add_plugin(Bridge::default());
}

/// Register a hidden, inactive Find/Search viewport before its first presentation.
/// Returns true for a newly registered panel; the caller can then show it.
pub fn register(ctx: &egui::Context, frame: &eframe::Frame) -> Result<bool, String> {
    let viewport = ctx.viewport_id();
    let title = if viewport == egui::ViewportId::from_hash_of("document_find_popup") {
        "Find in document"
    } else if viewport == egui::ViewportId::from_hash_of("file_search_popup") {
        "Search files"
    } else {
        return Err("Accessibility registration requires a Find or Search panel".into());
    };
    PANELS.with(|panels| {
        if panels.borrow().contains_key(&viewport) {
            return Ok(false);
        }
        let mtm = MainThreadMarker::new().ok_or("Panel registration requires the main thread")?;
        let RawWindowHandle::AppKit(handle) =
            frame.window_handle().map_err(|e| e.to_string())?.as_raw()
        else {
            return Err("Panel registration requires an AppKit window".into());
        };
        let root_view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        let root = root_view
            .window()
            .ok_or("The editor window is unavailable")?;
        let windows: Vec<_> = NSApplication::sharedApplication(mtm)
            .windows()
            .to_vec()
            .into_iter()
            .filter(|window| {
                window.title().to_string() == title && !std::ptr::eq(&**window, &*root)
            })
            .collect();
        if windows.len() != 1 {
            return Err(format!("Could not identify the {title} panel"));
        }
        let window = windows.into_iter().next().unwrap();
        if window.isVisible() || window.isKeyWindow() {
            return Err(format!(
                "The {title} panel must initialize before presentation"
            ));
        }
        let view = window
            .contentView()
            .ok_or("The search panel has no content view")?;
        let tree = Arc::new(Mutex::new(None));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let adapter = unsafe {
            SubclassingAdapter::new(
                Retained::as_ptr(&view).cast_mut().cast(),
                Activation(tree.clone()),
                Actions {
                    requests: requests.clone(),
                    ctx: ctx.clone(),
                    viewport,
                },
            )
        };
        unsafe {
            root.addChildWindow_ordered(&window, NSWindowOrderingMode::Above);
        }
        if title == "Search files" {
            // Winit's Visible(true) makes a window key on macOS. A suggestions panel
            // must appear without taking keyboard input from the parent search field.
            window.orderFront(None);
            root.makeKeyWindow();
        }
        panels.borrow_mut().insert(
            viewport,
            Panel {
                adapter,
                window,
                tree,
                requests,
            },
        );
        Ok(true)
    })
}
