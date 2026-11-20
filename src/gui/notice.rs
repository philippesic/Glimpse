use super::*;

pub(super) struct Failure {
    pub path: PathBuf,
    message: String,
    return_focus: Option<FocusHandle>,
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
        self.open_failure = Some(Failure {
            path,
            message,
            return_focus: window.focused(cx),
            above_row,
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
        let return_focus = failure.return_focus.clone();
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
                        this.clear_open_failure();
                        let navigator_shown = if this.narrow(window) {
                            this.navigation_only
                        } else {
                            this.navigator_visible
                        };
                        if let Some(focus) = &return_focus
                            && ((focus == &this.nav_focus && navigator_shown)
                                || (this.panel.is_some()
                                    && (focus == &this.query.focus_handle(cx)
                                        || focus == &this.search_ui.focus)))
                        {
                            window.focus(focus);
                        } else {
                            this.focus(window, cx);
                        }
                        cx.notify();
                    })),
            )
    }
}
