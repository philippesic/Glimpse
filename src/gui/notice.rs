use super::*;

pub(super) struct Failure {
    pub path: PathBuf,
    message: String,
    pub(super) return_focus: Option<FocusHandle>,
    above_row: bool,
}

impl Shell {
    pub(super) fn clear_open_failure(&mut self) {
        self.open_failure = None;
    }

    pub(super) fn record_open_failure(
        &mut self,
        path: PathBuf,
        message: String,
        window: &Window,
        cx: &App,
    ) {
        let row_below_middle = |scroll: &UniformListScrollHandle, index, height: Pixels| {
            let scroll = scroll.0.borrow();
            let viewport = scroll.base_handle.bounds();
            let row = viewport.top() + scroll.base_handle.offset().y + height * index;
            row + height / 2. > viewport.top() + viewport.size.height / 2.
        };
        let above_row = if self.nav_focus.is_focused(window) {
            let visible = self.visible_rows();
            visible
                .iter()
                .position(|&index| self.entries[index].path == path)
                .zip(self.scrolls.get(&self.current))
                .is_some_and(|(index, scroll)| {
                    row_below_middle(scroll, index, self.navigation_row_height())
                })
        } else {
            self.search_ui.focus.is_focused(window)
                && self.search_ui.selection.is_some_and(|index| {
                    row_below_middle(
                        &self.search_ui.scroll,
                        index,
                        px(32. * self.settings.ui_scale),
                    )
                })
        };
        let previous = self
            .open_failure
            .as_ref()
            .filter(|failure| failure.path == path);
        let inside = self.notice_focus.contains_focused(window, cx);
        self.open_failure = Some(Failure {
            path,
            message,
            return_focus: if inside {
                previous.and_then(|failure| failure.return_focus.clone())
            } else {
                window.focused(cx)
            },
            above_row: if inside {
                previous.is_some_and(|failure| failure.above_row)
            } else {
                above_row
            },
        });
    }

    pub(super) fn notice(&self, cx: &App) -> Div {
        let scale = self.settings.ui_scale;
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .flex_shrink_0()
            .mx(px(8. * scale))
            .mb(px(6. * scale))
            .px(px(8. * scale))
            .py(px(4. * scale))
            .gap(px(8. * scale))
            .rounded(px(7. * scale))
            .bg(cx.theme().secondary)
            .border_l(px(3. * scale))
            .border_color(cx.theme().warning)
    }

    pub(super) fn notice_label(
        &self,
        id: &'static str,
        title: String,
        reason: &str,
        show_reason: bool,
        cx: &App,
    ) -> impl IntoElement {
        let full = format!("{title}. {reason}");
        let tooltip = full.clone();
        let reason = reason.to_owned();
        div()
            .flex()
            .items_center()
            .gap(px(8. * self.settings.ui_scale))
            .map(|label| {
                if show_reason {
                    label.w_full().min_w_0()
                } else {
                    label.flex_1().min_w(px(140. * self.settings.ui_scale))
                }
            })
            .child(
                gpui_component::Icon::new(IconName::TriangleAlert)
                    .with_size(px(16. * self.settings.ui_scale))
                    .flex_shrink_0()
                    .text_color(cx.theme().warning),
            )
            .child(accessibility::message(
                id,
                full,
                div()
                    .id(id)
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .whitespace_normal()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(show_reason, |label| {
                        label.child(div().text_sm().whitespace_normal().child(reason))
                    })
                    .tooltip(move |window, cx| {
                        gpui_component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                    }),
            ))
    }

    pub(super) fn open_failure_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let viewport = window.viewport_size();
        let scale = self.settings.ui_scale;
        let narrow = self.narrow(window);
        let navigator_width = if !narrow && self.navigator_visible {
            self.pane_state.read(cx).width(window) + px(8.)
        } else {
            px(0.)
        };
        let available = viewport.width - navigator_width;
        let width = if narrow {
            available
        } else {
            available.min(px(620. * scale))
        };
        div()
            .id("open-failure-overlay")
            .track_focus(&self.notice_focus)
            .absolute()
            .right_0()
            .w(width)
            .max_h(relative(0.48))
            .overflow_y_scroll()
            .map(|overlay| {
                if narrow
                    && self
                        .open_failure
                        .as_ref()
                        .is_some_and(|failure| failure.above_row)
                {
                    overlay.top_0().pt(px(6. * scale))
                } else {
                    overlay.bottom_0()
                }
            })
            .child(self.open_failure_view(cx))
    }

    fn open_failure_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let failure = self.open_failure.as_ref().unwrap();
        let path = failure.path.clone();
        let detail = format!(
            "File: {}\n{}\nGlimpse did not change this file. It cannot be edited until it opens successfully.",
            paths::display(&path),
            failure.message
        );
        let retry_path = path.clone();
        self.notice(cx)
            .occlude()
            .child(self.notice_label(
                "open-failure",
                format!("Cannot open {}", paths::name(&path)),
                &failure.message,
                true,
                cx,
            ))
            .child(
                Button::new("open-failure-details")
                    .with_focus_handle(self.notice_details_focus.clone())
                    .label("Details")
                    .small()
                    .outline()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ask_decision(
                            DecisionAction::Notice,
                            "File could not be opened".into(),
                            detail.clone(),
                            vec!["Close"],
                            window,
                            cx,
                        );
                    })),
            )
            .child(
                Button::new("open-failure-retry")
                    .label("Retry Open")
                    .small()
                    .outline()
                    .on_click(cx.listener(move |this, _, _, _| {
                        this.open(retry_path.clone(), false, None);
                    })),
            )
            .child(
                Button::new("open-failure-external")
                    .label("Open Externally")
                    .small()
                    .outline()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(error) = platform::open_path(&path) {
                            this.status = error;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("open-failure-dismiss")
                    .label("Dismiss")
                    .small()
                    .outline()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.dismiss_open_failure(window, cx);
                    })),
            )
    }

    fn focus_failure_invoker(
        &self,
        focus: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let navigator_shown = if self.narrow(window) {
            self.navigation_only || self.active.is_none()
        } else {
            self.navigator_visible
        };
        if let Some(focus) = focus
            && ((focus == self.nav_focus && navigator_shown)
                || (matches!(self.panel, Some(Panel::Files | Panel::Text))
                    && (focus == self.query.focus_handle(cx) || focus == self.search_ui.focus)))
        {
            window.focus(&focus);
        } else {
            self.focus(window, cx);
        }
        cx.notify();
    }

    fn dismiss_open_failure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .open_failure
            .take()
            .and_then(|failure| failure.return_focus);
        self.focus_failure_invoker(focus, window, cx);
    }

    pub(super) fn open_failure_key(
        &mut self,
        key: &Keystroke,
        overlay: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(failure) = &self.open_failure else {
            return false;
        };
        let inside = self.notice_focus.contains_focused(window, cx);
        let at_invoker = failure
            .return_focus
            .as_ref()
            .is_some_and(|focus| focus.is_focused(window));
        match key.key.as_str() {
            "escape" if !key.modifiers.shift && (inside || at_invoker) => {
                self.dismiss_open_failure(window, cx);
            }
            "tab" if inside => {
                let return_focus = failure.return_focus.clone();
                if key.modifiers.shift {
                    window.focus_prev();
                } else {
                    window.focus_next();
                }
                if !self.notice_focus.contains_focused(window, cx) {
                    self.focus_failure_invoker(return_focus, window, cx);
                }
            }
            "tab" if at_invoker && !key.modifiers.shift => window.focus(&self.notice_details_focus),
            "f6" if overlay && inside => {
                self.focus_failure_invoker(failure.return_focus.clone(), window, cx);
            }
            "f6" if overlay && at_invoker => window.focus(&self.notice_details_focus),
            _ => return false,
        }
        window.prevent_default();
        cx.stop_propagation();
        true
    }
}
