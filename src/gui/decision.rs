use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    scroll::{Scrollbar, ScrollbarShow},
};

pub struct Decision {
    title: String,
    detail: String,
    subjects: Vec<String>,
    summary: Option<String>,
    primary: Option<usize>,
    destructive: Option<usize>,
    actions: Vec<&'static str>,
    focus: FocusHandle,
    comparison: Option<[Entity<InputState>; 2]>,
    detail_scroll: ScrollHandle,
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
                if this.comparison.is_none()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt
                {
                    let old = this.detail_scroll.offset();
                    let page = this.detail_scroll.bounds().size.height;
                    let maximum = this.detail_scroll.max_offset().height;
                    let offset = match event.keystroke.key.as_str() {
                        "pageup" => Some((old.y + page).min(px(0.))),
                        "pagedown" => Some((old.y - page).max(-maximum)),
                        "home" => Some(px(0.)),
                        "end" => Some(-maximum),
                        _ => None,
                    };
                    if let Some(offset) = offset {
                        this.detail_scroll.set_offset(point(old.x, offset));
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
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
            subjects: Vec::new(),
            summary: None,
            primary: None,
            destructive: None,
            actions,
            focus,
            comparison: None,
            detail_scroll: ScrollHandle::new(),
            _subscription: subscription,
        }
    }
    pub fn close_actions(&mut self) {
        self.summary = Some(
            "Save writes changes. Discard closes without saving and removes automatic drafts."
                .into(),
        );
        self.primary = Some(1);
        self.destructive = Some(2);
    }
    pub fn set_subjects(&mut self, subjects: Vec<String>, cx: &mut Context<Self>) {
        self.subjects = subjects;
        cx.notify();
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
        let mut semantic_detail = self.summary.as_ref().map_or_else(
            || self.detail.clone(),
            |summary| format!("{summary}\n\n{}", self.detail),
        );
        if !self.subjects.is_empty() {
            semantic_detail = format!(
                "Unsaved documents:\n{}\n\n{semantic_detail}",
                self.subjects.join("\n")
            );
        }
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
                            .relative()
                            .h(window.viewport_size().height * 0.4)
                            .min_h_0()
                            .overflow_hidden()
                            .child(
                                div()
                                    .id("decision-detail")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.detail_scroll)
                                    .pr_3()
                                    .children(self.subjects.iter().enumerate().map(
                                        |(index, label)| {
                                            let name = label.clone();
                                            gpui::accessible(
                                                ("decision-subject", index),
                                                div().pb_2().font_semibold().child(label.clone()),
                                                move |_, _, _| {
                                                    let mut node = gpui::accesskit::Node::new(
                                                        gpui::accesskit::Role::Label,
                                                    );
                                                    node.set_value(name.clone());
                                                    node.set_read_only();
                                                    AccessibilityInfo::new(node)
                                                },
                                            )
                                        },
                                    ))
                                    .child(self.detail.clone()),
                            )
                            .child(
                                div().absolute().inset_0().child(
                                    Scrollbar::vertical(&self.detail_scroll)
                                        .scrollbar_show(ScrollbarShow::Always),
                                ),
                            ),
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
