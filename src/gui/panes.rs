use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, PixelsExt};

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
        self.width.clamp(px(180.), Self::maximum(window))
    }

    fn maximum(window: &Window) -> Pixels {
        px(600.).min((window.viewport_size().width - px(208.)).max(px(180.)))
    }
}

impl Render for Divider {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        div()
            .id("navigator-divider")
            .w(px(8.))
            .h_full()
            .flex_shrink_0()
            .cursor_col_resize()
            .track_focus(&self.focus)
            .when(self.focus.is_focused(window), |style| {
                style.bg(cx.theme().ring)
            })
            .hover(|style| style.bg(cx.theme().border))
            .border_x_1()
            .border_color(cx.theme().border)
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
                let delta = match event.keystroke.key.as_str() {
                    "left" => -20.,
                    "right" => 20.,
                    "home" => 180. - this.width.as_f32(),
                    "end" => Self::maximum(window).as_f32() - this.width.as_f32(),
                    _ => return,
                };
                this.width =
                    (this.width(window) + px(delta)).clamp(px(180.), Self::maximum(window));
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
                                        .clamp(px(180.), Self::maximum(window));
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
            )
    }
}
