use super::*;

pub(super) struct Failure {
    pub path: PathBuf,
    pub message: String,
}

impl Shell {
    pub(super) fn open_failure_view(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let failure = self.open_failure.as_ref().unwrap();
        let path = failure.path.clone();
        let detail = format!(
            "File: {}\n{}\nGlimpse did not change this file. It cannot be edited until it opens successfully.",
            paths::display(&path),
            failure.message
        );
        let accessible_detail = detail.clone();
        let retry_path = path.clone();
        let banner = div()
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .truncate()
                            .child(format!("Cannot open {}", paths::display(&path))),
                    )
                    .child(div().text_sm().truncate().child(failure.message.clone())),
            )
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_failure = None;
                        cx.notify();
                    })),
            );
        gpui::accessible("open-failure", banner, move |_, _, _| {
            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Status);
            node.set_label(accessible_detail.clone());
            AccessibilityInfo::new(node)
        })
    }
}
