use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, PixelsExt};

const MIN_WIDTH: Pixels = px(180.);
const STEP: Pixels = px(20.);

pub struct Divider {
    width: Pixels,
    dragging: Option<(Pixels, Pixels)>,
    focus: FocusHandle,
}

impl Divider {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            width: px(280.),
            dragging: None,
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    pub fn width(&self, window: &Window) -> Pixels {
        self.width.clamp(MIN_WIDTH, Self::maximum(window))
    }

    fn maximum(window: &Window) -> Pixels {
        px(600.).min((window.viewport_size().width - px(208.)).max(MIN_WIDTH))
    }
}

impl Render for Divider {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let focused = self.focus.is_focused(window);
        let divider = div()
            .id("navigator-divider")
            .w(px(8.))
            .h_full()
            .flex_shrink_0()
            .cursor_col_resize()
            .track_focus(&self.focus)
            .map(|style| {
                if focused {
                    style.bg(cx.theme().ring)
                } else {
                    style.hover(|style| style.bg(cx.theme().title_bar_border))
                }
            })
            .border_l_1()
            .border_color(cx.theme().title_bar_border)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.dragging = Some((event.position.x, this.width(window)));
                    window.focus(&this.focus);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let width = this.width(window);
                let maximum = Self::maximum(window);
                this.width = match event.keystroke.key.as_str() {
                    "left" => (width - STEP).max(MIN_WIDTH),
                    "right" => (width + STEP).min(maximum),
                    "home" => MIN_WIDTH,
                    "end" => maximum,
                    _ => return,
                };
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let moving = weak.clone();
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if !phase.capture() {
                                return;
                            }
                            let _ = moving.update(cx, |this, cx| {
                                let Some((start, width)) = this.dragging else {
                                    return;
                                };
                                if !event.dragging() {
                                    this.dragging = None;
                                } else {
                                    this.width = (width + event.position.x - start)
                                        .clamp(MIN_WIDTH, Self::maximum(window));
                                }
                                cx.notify();
                            });
                        });
                        let releasing = weak.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase.capture() {
                                let _ = releasing.update(cx, |this, _| this.dragging = None);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            );
        let focus = self.focus.clone();
        let width = self.width(window);
        let view = cx.weak_entity();
        accessible("navigator-width", divider, move |_, window, _| {
            use gpui::accesskit::{Action, Node, Orientation, Role};
            let mut node = Node::new(Role::Splitter);
            node.set_label("Navigator divider");
            node.set_description("Use the Left and Right arrow keys to adjust the width.");
            node.set_orientation(Orientation::Vertical);
            node.set_numeric_value(width.as_f32() as f64);
            node.set_min_numeric_value(MIN_WIDTH.as_f32() as f64);
            node.set_max_numeric_value(Self::maximum(window).as_f32() as f64);
            node.set_numeric_value_step(STEP.as_f32() as f64);
            node.add_action(Action::Focus);
            node.add_action(Action::SetValue);
            let mut info = AccessibilityInfo::new(node);
            info.focused = focus.is_focused(window);
            info
        })
        .on_action(move |request, _, window, cx| {
            use gpui::accesskit::{Action, ActionData};
            let _ = view.update(cx, |this, cx| match (request.action, request.data) {
                (Action::Focus, _) => this.focus.focus(window),
                (Action::SetValue, Some(ActionData::NumericValue(value))) if value.is_finite() => {
                    this.width = px(value as f32).clamp(MIN_WIDTH, Self::maximum(window));
                    cx.notify();
                }
                _ => {}
            });
        })
    }
}
