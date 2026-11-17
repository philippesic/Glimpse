use super::*;
use gpui_component::button::ButtonVariants;

pub(super) struct Completions {
    pub matches: glimpse::path_completion::Matches,
    pub value: String,
    pub next: usize,
}
impl Shell {
    pub(super) fn complete_path(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.query.read(cx).value().to_string();
        if let Some(completions) = self.path_completions.as_mut().filter(|c| c.value == value) {
            let count = completions.matches.choices.len();
            if count == 0 {
                return;
            }
            let index = if reverse {
                (completions.next + count * 2 - 2) % count
            } else {
                completions.next % count
            };
            let entry = completions.matches.choices[index].entry();
            completions.value = entry.clone();
            completions.next = (index + 1) % count;
            self.query
                .update(cx, |input, cx| input.set_value(entry, window, cx));
            cx.notify();
            return;
        }
        self.path_completions = None;
        self.path_completion_token += 1;
        self.io.send(Job::CompletePath(
            self.path_completion_token,
            value,
            self.current.clone(),
        ));
        self.status = "Completing this path…".into();
    }
    pub(super) fn path_completed(
        &mut self,
        token: u64,
        value: String,
        result: Result<glimpse::path_completion::Matches, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if token != self.path_completion_token
            || self.panel != Some(Panel::Path)
            || self.query.read(cx).value().as_str() != value
            || !self.query.focus_handle(cx).is_focused(window)
        {
            return;
        }
        match result {
            Ok(matches) => {
                let count = matches.choices.len();
                self.status = if count == 0 {
                    "No matching paths".into()
                } else if matches.partial {
                    "Partial path completions — narrow the name to see more".into()
                } else {
                    format!("{count} path completions · Tab / Shift-Tab cycles; Enter opens")
                };
                self.path_completions = Some(Completions {
                    matches,
                    value,
                    next: 0,
                });
                self.complete_path(false, window, cx);
            }
            Err(error) => self.status = format!("Path completion failed: {error}"),
        }
    }
    pub(super) fn path_bar(&mut self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        if self.panel == Some(Panel::Path) {
            return div()
                .flex()
                .items_center()
                .gap_2()
                .p_2()
                .child(Input::new(&self.query).flex_1().min_w_0())
                .child(self.button("Go", "submit", cx))
                .child(self.button("Cancel", "dismiss", cx))
                .into_any_element();
        }
        let mut ancestors: Vec<_> = self
            .current
            .ancestors()
            .map(|path| path.to_owned())
            .collect();
        ancestors.reverse();
        let count = ancestors.len();
        let view = (
            self.current.clone(),
            window.viewport_size().width,
            self.settings.ui_scale,
        );
        if self.breadcrumb_view.as_ref() != Some(&view) {
            let scroll = self.breadcrumb_scroll.clone();
            window.on_next_frame(move |window, _| {
                scroll.scroll_to_item(count.saturating_sub(1) * 2);
                window.refresh();
            });
            self.breadcrumb_view = Some(view);
        }
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .py_1()
            .child(
                div()
                    .id("breadcrumbs")
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.breadcrumb_scroll)
                    .children(ancestors.into_iter().enumerate().flat_map(|(i, path)| {
                        let label = paths::name(&path);
                        let button = Button::new(("crumb", i))
                            .label(label)
                            .tooltip(paths::display(&path))
                            .small()
                            .compact()
                            .ghost()
                            .flex_shrink_0()
                            .selected(i + 1 == count)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.navigate(path.clone());
                                this.navigation_only = true;
                                window.focus(&this.nav_focus);
                                cx.notify();
                            }))
                            .into_any_element();
                        if i == 0 {
                            vec![button]
                        } else {
                            vec![div().flex_shrink_0().child("›").into_any_element(), button]
                        }
                    })),
            )
            .child(
                self.command_button("edit-location", "Edit path…", "path", cx)
                    .flex_shrink_0()
                    .tooltip(paths::display(&self.current)),
            )
            .into_any_element()
    }

    pub(super) fn path_matches(&self, cx: &Context<Self>) -> AnyElement {
        let current = self.query.read(cx).value();
        let Some(completions) = self
            .path_completions
            .as_ref()
            .filter(|c| c.value == current.as_str())
        else {
            return div().into_any_element();
        };
        div()
            .flex()
            .flex_col()
            .items_start()
            .id("path-completions")
            .max_h(px(160.))
            .overflow_y_scroll()
            .px_2()
            .children(
                completions
                    .matches
                    .choices
                    .iter()
                    .enumerate()
                    .map(|(i, choice)| {
                        let entry = choice.entry();
                        Button::new(("path-match", i))
                            .label(format!(
                                "{}{}",
                                paths::name(&choice.path),
                                if choice.directory { "/" } else { "" }
                            ))
                            .small()
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.path_completions = None;
                                this.query.update(cx, |input, cx| {
                                    input.set_value(entry.clone(), window, cx);
                                    input.focus(window, cx);
                                });
                                cx.notify();
                            }))
                    }),
            )
            .into_any_element()
    }
}
