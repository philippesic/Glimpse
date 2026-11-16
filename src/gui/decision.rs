use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Sizable, button::Button};

pub struct Decision {
    title: String,
    detail: String,
    actions: Vec<&'static str>,
    focus: FocusHandle,
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
            actions,
            focus,
            _subscription: subscription,
        }
    }
}
impl Render for Decision {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
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
                    .child(div().text_xl().child(self.title.clone()))
                    .child(
                        div()
                            .id("decision-detail")
                            .min_h_0()
                            .overflow_y_scroll()
                            .child(self.detail.clone()),
                    )
                    .child(div().flex().flex_wrap().gap_2().children(
                        self.actions.iter().enumerate().map(|(i, label)| {
                            Button::new(i)
                                .label(*label)
                                .small()
                                .outline()
                                .on_click(cx.listener(move |_, _, _, cx| cx.emit(Choice(i))))
                        }),
                    )),
            )
    }
}
