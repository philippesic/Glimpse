use gpui::{
    App, AppContext as _, Context, Corner, DismissEvent, Entity, IntoElement, MouseDownEvent,
    ParentElement as _, Pixels, Point, Render, Styled, Subscription, WeakEntity, Window, anchored,
    deferred, div, point, prelude::FluentBuilder as _, px,
};
use rust_i18n::t;

use crate::{
    ActiveTheme as _,
    input::{self, InputState, popovers::ContextMenu},
    menu::PopupMenu,
};

/// Context menu for mouse right clicks.
pub(crate) struct MouseContextMenu {
    editor: WeakEntity<InputState>,
    menu: Entity<PopupMenu>,
    mouse_position: Point<Pixels>,
    open: bool,

    _subscriptions: Vec<Subscription>,
}

impl InputState {
    pub(crate) fn handle_right_click_menu(
        &mut self,
        event: &MouseDownEvent,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Show Mouse context menu
        if !self.selected_range.contains(offset) {
            self.move_to(offset, None, cx);
        }

        if self.mode.is_code_editor() {
            self.handle_hover_definition(offset, window, cx);
        }

        self.show_context_menu(event.position, window, cx);
    }

    pub fn open_context_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.last_bounds else {
            return;
        };
        let cursor = self.cursor();
        let position = self
            .range_to_bounds(&(cursor..cursor))
            .map(|caret| point(caret.left(), caret.bottom()))
            .filter(|point| bounds.contains(point))
            .unwrap_or_else(|| bounds.center());
        self.show_context_menu(position, window, cx);
    }

    fn show_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.context_menu = Some(ContextMenu::MouseContext(self.mouse_context_menu.clone()));
        let is_enable = !self.disabled;
        let has_goto_definition =
            self.mode.is_code_editor() && self.lsp.definition_provider.is_some();
        let has_code_action =
            self.mode.is_code_editor() && !self.lsp.code_action_providers.is_empty();
        let is_selected = !self.selected_range.is_empty();
        let has_paste = is_enable && cx.read_from_clipboard().is_some();

        let action_context = self.focus_handle.clone();
        self.mouse_context_menu.update(cx, |this, cx| {
            this.mouse_position = position;
            this.open = true;
            this.menu.update(cx, |menu, cx| {
                let new_menu = PopupMenu::new(cx)
                    .when(has_goto_definition, |m| {
                        m.menu_with_enable(
                            t!("Input.Go to Definition"),
                            Box::new(input::GoToDefinition),
                            is_enable,
                        )
                    })
                    .when(has_code_action, |m| {
                        m.menu_with_enable(
                            t!("Input.Show Code Actions"),
                            Box::new(input::ToggleCodeActions),
                            is_enable,
                        )
                    })
                    .when(has_code_action || has_goto_definition, |m| m.separator())
                    .menu_with_enable(
                        t!("Input.Cut"),
                        Box::new(input::Cut),
                        is_enable && is_selected,
                    )
                    .menu_with_enable(t!("Input.Copy"), Box::new(input::Copy), is_selected)
                    .menu_with_enable(t!("Input.Paste"), Box::new(input::Paste), has_paste)
                    .separator()
                    .menu(t!("Input.Select All"), Box::new(input::SelectAll));

                menu.menu_items = new_menu.menu_items;
                menu.action_context = Some(action_context);
                menu.min_width = Some(window.rem_size() * 12.);
                menu.highlight_first_item(cx);
                let focus = menu.focus_handle.clone();
                window.defer(cx, move |window, _| window.focus(&focus));
                cx.notify();
            });

            cx.notify();
        });
    }
}

impl MouseContextMenu {
    pub(crate) fn new(
        editor: Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let menu = cx.new(|cx| PopupMenu::new(cx).small());

            let _subscriptions = vec![cx.subscribe_in(&menu, window, {
                move |this: &mut Self, _, _: &DismissEvent, window, cx| {
                    this.close(window, cx);
                }
            })];

            Self {
                editor: editor.downgrade(),
                menu,
                mouse_position: Point::default(),
                open: false,
                _subscriptions,
            }
        })
    }

    #[inline]
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    #[inline]
    pub(crate) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        let _ = self.editor.update(cx, |this, cx| {
            this.focus(window, cx);
        });
    }
}

impl Render for MouseContextMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        deferred(
            anchored()
                .snap_to_window_with_margin(px(8.))
                .anchor(Corner::TopLeft)
                .position(self.mouse_position)
                .child(
                    div()
                        .font_family(cx.theme().font_family.clone())
                        .cursor_default()
                        .child(self.menu.clone()),
                ),
        )
        .into_any_element()
    }
}
