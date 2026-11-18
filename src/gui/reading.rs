use super::*;
use gpui_component::{
    highlighter::HighlightTheme,
    menu::DropdownMenu,
    text::{MarkdownLayout, MarkdownView, TextViewResources},
};
use std::sync::Arc;

pub(super) struct View {
    request: u64,
    id: u64,
    generation: u64,
    path: PathBuf,
    theme: Arc<HighlightTheme>,
    pending: Option<Box<Job>>,
    content: Option<MarkdownView>,
    source_lines: Option<gpui::PreparedTextLines>,
    release_memory: bool,
    notes: String,
    error: Option<String>,
}
impl Shell {
    pub(super) fn prepare_reading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .reading_view
            .as_ref()
            .is_some_and(|view| !self.reading || self.active != Some(view.id))
        {
            if let Some(view) = self.reading_view.take()
                && self.active == Some(view.id)
                && let Some(lines) = view.source_lines
            {
                window.text_system().adopt_prepared_lines(&lines);
            }
            self.io.cancel_reading();
            self.release_view_memory(window, cx);
        }
        if !self.reading {
            return;
        }
        let Some(doc) = self
            .documents
            .iter()
            .find(|doc| Some(doc.id) == self.active)
        else {
            return;
        };
        let theme = cx.theme().highlight_theme.clone();
        if self.reading_view.as_ref().is_some_and(|view| {
            view.id == doc.id
                && view.generation == doc.generation
                && view.path == doc.disk.path
                && Arc::ptr_eq(&view.theme, &theme)
        }) {
            return;
        }
        self.reading_request += 1;
        let request = self.reading_request;
        let release_memory = doc.input.read(cx).text().len() >= 1024 * 1024;
        if release_memory {
            self.syntax.retire_assets(platform::release_unused_memory);
        }
        let source_lines = self
            .reading_view
            .as_mut()
            .and_then(|view| view.source_lines.take())
            .or_else(|| release_memory.then(|| window.text_system().retain_recent_lines()));
        let mut text_style = window.text_style();
        text_style.font_size = self.reading_font_size().into();
        text_style.color = cx.theme().foreground;
        let layout = Box::new(MarkdownLayout::new(window, text_style, cx.theme()));
        let job = Job::ReadMarkdown {
            request,
            source: doc.input.read(cx).text().clone(),
            path: doc.disk.path.clone(),
            theme: theme.clone(),
            code_canvases: super::theme::reading_code_canvases(cx),
            layout,
            reclaim: release_memory.then_some(platform::release_unused_memory),
        };
        self.reading_view = Some(View {
            request,
            id: doc.id,
            generation: doc.generation,
            path: doc.disk.path.clone(),
            theme,
            pending: Some(Box::new(job)),
            content: None,
            source_lines,
            release_memory,
            notes: String::new(),
            error: None,
        });
        if release_memory && window.is_window_active() {
            cx.on_next_frame(window, move |this, _, cx| {
                this.dispatch_reading(request, cx)
            });
        } else {
            self.dispatch_reading(request, cx);
        }
    }

    pub(super) fn prepare_inactive_reading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            self.prepare_reading(window, cx);
            if let Some(request) = self.reading_view.as_ref().map(|view| view.request) {
                self.dispatch_reading(request, cx);
            }
        }
    }

    fn dispatch_reading(&mut self, request: u64, cx: &mut Context<Self>) {
        let Some(view) = self
            .reading_view
            .as_mut()
            .filter(|view| self.reading && self.active == Some(view.id) && view.request == request)
        else {
            return;
        };
        let Some(job) = view.pending.take() else {
            return;
        };
        if view.release_memory
            && let Some(doc) = self.documents.iter().find(|doc| doc.id == view.id)
        {
            doc.input
                .update(cx, |input, _| input.release_accessibility_cache());
        }
        self.io.send(*job);
    }
    pub(super) fn reading_ready(
        &mut self,
        request: u64,
        result: Result<Box<glimpse::reading::Prepared>, String>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        let Some(view) = self
            .reading_view
            .as_mut()
            .filter(|view| self.reading && self.active == Some(view.id) && view.request == request)
        else {
            if let Ok(retired) = result {
                cx.background_spawn(async move { drop(retired) }).detach();
            }
            return;
        };
        match result {
            Ok(prepared) => {
                let id = view.id;
                glimpse::metrics::event("reading-image-count", id, prepared.images.len() as u64);
                glimpse::metrics::event(
                    "reading-image-bytes",
                    id,
                    prepared
                        .images
                        .values()
                        .map(|image| image.bgra.len() as u64)
                        .sum(),
                );
                let base = view.path.clone();
                let label_base = base.clone();
                let resources = Arc::new(TextViewResources {
                    restrict_images: true,
                    images: prepared
                        .images
                        .into_iter()
                        .map(|(url, pixels)| {
                            (
                                url.into(),
                                Arc::new(RenderImage::new(vec![image::Frame::new(
                                    image::RgbaImage::from_raw(
                                        pixels.width,
                                        pixels.height,
                                        pixels.bgra,
                                    )
                                    .unwrap(),
                                )])),
                            )
                        })
                        .collect(),
                    on_link: Some(Arc::new(move |url, _, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            match glimpse::reading::resolve_link(&base, url) {
                                glimpse::reading::Link::Local(path) => {
                                    this.open(path, true, None);
                                }
                                glimpse::reading::Link::External(url) => cx.open_url(&url),
                                glimpse::reading::Link::Unsupported => {
                                    this.status =
                                        "This link type is not supported. Source text is unchanged."
                                            .into()
                                }
                            }
                            cx.notify();
                        });
                    })),
                    link_label: Some(Arc::new(move |url| {
                        match glimpse::reading::resolve_link(&label_base, url) {
                            glimpse::reading::Link::Local(path) => {
                                let relative = label_base
                                    .parent()
                                    .and_then(|folder| path.strip_prefix(folder).ok())
                                    .unwrap_or(&path);
                                paths::display(relative)
                            }
                            _ => url.to_owned(),
                        }
                    })),
                });
                view.content = Some(MarkdownView::new(
                    SharedString::from(format!("markdown-{}", view.request)),
                    prepared.markdown,
                    resources,
                    self.reading_focus.clone(),
                    view.release_memory
                        .then_some(platform::release_unused_memory),
                    cx,
                ));
                view.notes = prepared.notes.join("\n\n");
            }
            Err(error) => view.error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn reading_notices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.reading_view.as_ref().filter(|view| {
            self.documents.iter().any(|doc| {
                Some(doc.id) == self.active
                    && doc.id == view.id
                    && doc.generation == view.generation
                    && doc.disk.path == view.path
            })
        }) else {
            return;
        };
        let notes = view.notes.clone();
        if !notes.is_empty() {
            self.ask_decision(
                DecisionAction::Notice,
                "Reading image notices".into(),
                notes,
                vec!["Close"],
                window,
                cx,
            );
        }
    }

    fn reading_font_size(&self) -> Pixels {
        px(self.settings.markdown_font_size * self.settings.ui_scale)
    }

    fn reading_progress(&self) -> AnyElement {
        let message = "Preparing reading view…";
        let focus = self.reading_focus.clone();
        accessible(
            "reading-progress",
            div().track_focus(&focus).child(message),
            move |_, window, _| {
                let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
                node.set_label(message);
                node.set_busy();
                let mut info = AccessibilityInfo::new(node);
                info.focused = focus.is_focused(window);
                info
            },
        )
        .into_any_element()
    }

    pub(super) fn reading_element(
        &self,
        narrow: bool,
        short: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let font_size = self.reading_font_size();
        let mut area = div()
            .id("reading")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .max_w(px(960.))
            .mx_auto()
            .flex()
            .flex_col()
            .p_4()
            .when(narrow, |area| area.px_3())
            .when(short, |area| area.py_2())
            .text_size(font_size);
        let Some(view) = &self.reading_view else {
            return area.child(self.reading_progress()).into_any_element();
        };
        if let Some(error) = &view.error {
            return area
                .child(error.clone())
                .child(self.button("Retry reading view", "reading-retry", cx))
                .into_any_element();
        }
        let Some(content) = &view.content else {
            return area.child(self.reading_progress()).into_any_element();
        };
        let links = content.links_menu(cx);
        if links.is_some() || !view.notes.is_empty() {
            let mut controls = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .children(links);
            if !view.notes.is_empty() {
                if short {
                    let shell = cx.entity().downgrade();
                    let focus = self.reading_focus.clone();
                    controls = controls.child(
                        Button::new("reading-images")
                            .label("Images omitted")
                            .small()
                            .compact()
                            .outline()
                            .dropdown_caret(true)
                            .dropdown_menu(move |menu, _, _| {
                                menu.action_context(focus.clone())
                                    .item(super::menus::command_item(
                                        shell.clone(),
                                        "reading-notices",
                                        "Image details…".into(),
                                        false,
                                    ))
                                    .item(super::menus::command_item(
                                        shell.clone(),
                                        "reading-retry",
                                        "Reload images".into(),
                                        false,
                                    ))
                            }),
                    );
                } else {
                    let label = view.notes.clone();
                    controls = controls
                        .child(div().flex_1().min_w_0().child(accessible(
                            "reading-notices",
                            div().w_full().truncate().text_sm().when(!narrow, |label| {
                                label.child("Images omitted or unavailable")
                            }),
                            move |_, _, _| {
                                let mut node =
                                    gpui::accesskit::Node::new(gpui::accesskit::Role::Note);
                                node.set_label(label.clone());
                                AccessibilityInfo::new(node)
                            },
                        )))
                        .child(self.button(
                            if narrow {
                                "Images omitted…"
                            } else {
                                "Image details…"
                            },
                            "reading-notices",
                            cx,
                        ))
                        .child(self.button("Reload images", "reading-retry", cx));
                }
            }
            area = area.child(controls);
        }
        area.child(content.element(font_size * 36.))
            .into_any_element()
    }
}
