use std::rc::Rc;

use gpui::{
    Context, Corner, DismissEvent, ElementId, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, MouseButton, RenderOnce, SharedString, StyleRefinement,
    Styled, Window, Pixels, px,
};

use crate::{button::Button, menu::PopupMenu, popover::Popover, Selectable};

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Corner::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Corner>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let style = self.style().clone();
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f).trigger_style(style)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + InteractiveElement + IntoElement + 'static> {
    id: ElementId,
    style: StyleRefinement,
    anchor: Corner,
    trigger: T,
    open_requested: bool,
    bottom_inset: Option<Pixels>,
    builder: Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + InteractiveElement + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Corner>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            style: StyleRefinement::default(),
            anchor: anchor.into(),
            trigger,
            open_requested: false,
            bottom_inset: None,
            builder: Rc::new(builder),
        }
    }

    pub fn request_open(mut self, requested: bool) -> Self {
        self.open_requested = requested;
        self
    }

    pub fn keep_clear_of_bottom(mut self, inset: Pixels) -> Self {
        self.bottom_inset = Some(inset);
        self
    }

    /// Set the anchor corner for the dropdown menu popover.
    pub fn anchor(mut self, anchor: impl Into<Corner>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Set the style refinement for the dropdown menu trigger.
    fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.style = style;
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
    open: bool,
    return_focus: Option<FocusHandle>,
    keyboard_armed: bool,
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + InteractiveElement + IntoElement + 'static,
{
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        if self.open_requested && !menu_state.read(cx).open {
            let return_focus = window.focused(cx);
            menu_state.update(cx, |state, _| {
                state.return_focus = return_focus;
                state.open = true;
            });
        }
        let open = menu_state.read(cx).open;
        let trigger = self
            .trigger
            .on_mouse_down(MouseButton::Left, {
                let state = menu_state.clone();
                move |_, window, cx| {
                    state.update(cx, |state, cx| state.return_focus = window.focused(cx));
                }
            })
            .on_key_down({
                let state = menu_state.clone();
                move |event: &KeyDownEvent, _, cx| {
                    if !event.keystroke.modifiers.modified()
                        && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
                    {
                        state.update(cx, |state, _| state.keyboard_armed = true);
                    }
                }
            })
            .on_key_up({
                let state = menu_state.clone();
                move |event: &KeyUpEvent, window, cx| {
                    if !event.keystroke.modifiers.modified()
                        && matches!(event.keystroke.key.as_str(), "enter" | "space" | "down")
                    {
                        state.update(cx, |state, cx| {
                            if std::mem::take(&mut state.keyboard_armed) {
                                state.return_focus = window.focused(cx);
                                state.open = true;
                            }
                        });
                        window.refresh();
                        cx.stop_propagation();
                    }
                }
            });
        Popover::new(SharedString::from(format!("popover:{}", self.id)))
            .open(open)
            .on_open_change({
                let state = menu_state.clone();
                move |open, window, cx| {
                    state.update(cx, |state, cx| {
                        state.open = *open;
                        if !open {
                            let contains_focus = state.menu.as_ref().is_some_and(|menu| {
                                menu.focus_handle(cx).contains_focused(window, cx)
                            });
                            if contains_focus && let Some(focus) = state.return_focus.take() {
                                window.focus(&focus);
                            }
                            state.menu = None;
                        }
                    });
                    window.refresh();
                }
            })
            .appearance(false)
            .overlay_closable(false)
            .trigger(trigger)
            .trigger_style(self.style)
            .anchor(self.anchor)
            .wait_for_anchor(self.bottom_inset.is_some())
            .content(move |popover, window, cx| {
                // Here is special logic to only create the PopupMenu once and reuse it.
                // Because this `content` will called in every time render, so we need to store the menu
                // in state to avoid recreating at every render.
                //
                // And we also need to rebuild the menu when it is dismissed, to rebuild menu items
                // dynamically for support `dropdown_menu` method, so we listen for DismissEvent below.
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        let popover_state = cx.entity();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.clone();
                                move |_, _: &DismissEvent, window, cx| {
                                    popover_state.update(cx, |state, cx| {
                                        state.dismiss(window, cx);
                                    });
                                    menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                };

                if let Some(inset) = self.bottom_inset
                    && let Some(bounds) = popover.trigger_bounds
                {
                    let origin = Popover::resolved_corner(self.anchor, bounds);
                    let height = window.viewport_size().height - origin.y - inset - px(16.);
                    menu.update(cx, |menu, cx| menu.limit_available_height(height, cx));
                }
                menu
            })
    }
}
