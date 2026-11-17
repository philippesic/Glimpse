use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
};

pub struct Decision {
    title: String,
    detail: String,
    summary: Option<String>,
    primary: Option<usize>,
    destructive: Option<usize>,
    actions: Vec<&'static str>,
    focus: FocusHandle,
    comparison: Option<[Entity<InputState>; 2]>,
    _subscription: Subscription,
}
pub struct Choice(pub usize);
impl EventEmitter<Choice> for Decision {}
impl Decision {
    pub fn new(
        title: String,
        detail: String,
        actions: Vec<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus);
        let view = cx.weak_entity();
        let owner = window.window_handle();
        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != owner {
                return;
            }
            let _ = view.update(cx, |this: &mut Self, cx| {
                if super::modal::tab(&event.keystroke, &this.focus, window, cx) {
                    return;
                }
                if event.keystroke.key == "escape" {
                    cx.emit(Choice(0));
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
        });
        Self {
            title,
            detail,
            summary: None,
            primary: None,
            destructive: None,
            actions,
            focus,
            comparison: None,
            _subscription: subscription,
        }
    }
    pub fn close_actions(&mut self) {
        self.summary = Some(
            "Save writes these documents. Discard closes them and removes their automatic drafts. Cancel keeps them open.".into(),
        );
        self.primary = Some(1);
        self.destructive = Some(2);
    }
    pub fn compare(
        &mut self,
        local: &str,
        observed: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comparison = Some(
            [("Local text", local), ("Observed disk text", observed)].map(|(label, text)| {
                cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .code_editor("text")
                        .default_value(text.to_owned());
                    input.set_cursor_blink(false, cx);
                    input.set_accessibility_label(label, cx);
                    input
                })
            }),
        );
        cx.notify();
    }
}
impl Render for Decision {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let semantic_title = self.title.clone();
        let semantic_detail = self.summary.as_ref().map_or_else(
            || self.detail.clone(),
            |summary| format!("{summary}\n\n{}", self.detail),
        );
        let rendered = div()
            .id("decision-overlay")
            .absolute()
            .inset_0()
            .occlude()
            .bg(cx.theme().overlay)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("decision")
                    .track_focus(&self.focus)
                    .occlude()
                    .w(px(640.).min(window.viewport_size().width - px(32.)))
                    .max_h(window.viewport_size().height - px(32.))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(div().flex_shrink_0().text_xl().child(self.title.clone()))
                    .when_some(self.summary.as_ref(), |panel, summary| {
                        panel.child(div().flex_shrink_0().text_sm().child(summary.clone()))
                    })
                    .child(
                        div()
                            .id("decision-detail")
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.detail.clone()),
                    )
                    .when_some(self.comparison.as_ref(), |panel, inputs| {
                        panel.children(inputs.iter().enumerate().map(|(i, input)| {
                            div()
                                .min_h_0()
                                .flex()
                                .flex_col()
                                .child(if i == 0 {
                                    "Local text (read only)"
                                } else {
                                    "Observed disk text (read only)"
                                })
                                .child(
                                    div()
                                        .h(px(160.).min(window.viewport_size().height * 0.22))
                                        .min_h_0()
                                        .child(Input::new(input).h_full().disabled(true)),
                                )
                        }))
                    })
                    .child(div().flex().flex_wrap().gap_2().children(
                        self.actions.iter().enumerate().map(|(i, label)| {
                            Button::new(i)
                                .label(*label)
                                .small()
                                .when(self.primary != Some(i), |button| button.outline())
                                .when(self.primary == Some(i), |button| button.primary())
                                .when(self.destructive == Some(i), |button| button.danger())
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(Choice(i))))
                        }),
                    )),
            );
        gpui::accessible("modal-semantics", rendered, move |_, _, _| {
            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Dialog);
            node.set_label(semantic_title.clone());
            node.set_description(semantic_detail.clone());
            let mut info = gpui::AccessibilityInfo::new(node);
            info.modal = true;
            info
        })
    }
}
