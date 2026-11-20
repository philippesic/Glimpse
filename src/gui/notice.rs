use super::*;

pub(super) struct Failure {
    pub path: PathBuf,
    pub message: String,
    pub return_focus: Option<FocusHandle>,
}

impl Shell {
    pub(super) fn clear_open_failure(&mut self) {
        self.open_failure = None;
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

    pub(super) fn open_failure_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
