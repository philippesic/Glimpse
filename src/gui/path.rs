use super::*;
use gpui_component::{
    IconName,
    button::ButtonVariants,
    menu::{DropdownMenu, PopupMenuItem},
};

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
            self.path_scroll.scroll_to_item(index);
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
        self.path_hint = Some("Completing this path…".into());
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
                self.path_hint = Some(if count == 0 {
                    "No matching paths".into()
                } else if matches.partial {
                    "Partial path completions — narrow the name to see more".into()
                } else {
                    format!("{count} path completions · Tab / Shift-Tab cycles; Enter opens")
                });
                self.path_completions = Some(Completions {
                    matches,
                    value,
                    next: 0,
                });
                self.complete_path(false, window, cx);
            }
            Err(error) => self.path_hint = Some(format!("Path completion failed: {error}")),
        }
    }
    pub(super) fn path_error_view(&self, error: String, cx: &App) -> impl IntoElement {
        let foreground = cx.theme().danger;
        let background = cx.theme().background;
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .occlude()
            .bg(background)
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(accessibility::message(
                "path-error",
                error.clone(),
                div().text_color(foreground).child(error),
            ))
    }

    pub(super) fn path_bar(
        &mut self,
        short: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        if self.panel == Some(Panel::Path) {
            return div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .when(!short, |bar| bar.py_1())
                .child(Input::new(&self.query).small().flex_1().min_w_0())
                .child(self.button("Go", "submit", cx))
                .child(self.button("Cancel", "dismiss", cx))
                .into_any_element();
        }
        let mut ancestors: Vec<_> = self
            .current
            .ancestors()
            .skip(1)
            .map(PathBuf::from)
            .collect();
        ancestors.reverse();
        let scale = self.settings.ui_scale;
        let segment_width = px(140. * scale);
        let room = (window.viewport_size().width - px(420. * scale)).max(px(0.));
        let visible = (room / (segment_width + px(24. * scale))) as usize;
        let first_visible = ancestors.len().saturating_sub(visible);
        let visible_ancestors = ancestors.split_off(first_visible);
        let rem = window.rem_size();
        let button_padding = rem * 0.75 + px(2.);
        let edit_label = SharedString::from("Edit path…");
        let edit_width = Self::path_label_width(window, &edit_label) + button_padding;
        let overflow_width = if ancestors.is_empty() {
            px(0.)
        } else {
            rem * 2.
        };
        let separator_width = Self::path_label_width(window, &"›".into()) + rem * 0.75;
        let visible_ancestors: Vec<_> = visible_ancestors
            .into_iter()
            .map(|path| {
                let name = paths::name(&path);
                let caption =
                    decision::abbreviate_folder(&name, segment_width - button_padding, &|text| {
                        Self::path_label_width(window, text)
                    });
                let width = Self::path_label_width(window, &caption) + button_padding;
                (path, caption, width + separator_width)
            })
            .collect();
        let ancestor_width = visible_ancestors
            .iter()
            .fold(px(0.), |total, (_, _, width)| total + *width);
        let current_width = window.viewport_size().width
            - rem * 1.5
            - edit_width
            - overflow_width
            - ancestor_width
            - button_padding;
        let current_caption =
            decision::abbreviate_folder(&paths::name(&self.current), current_width, &|text| {
                Self::path_label_width(window, text)
            });
        let shell = cx.entity().downgrade();
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .when(!short, |bar| bar.py_1())
            .when(!ancestors.is_empty(), |bar| {
                bar.child(
                    Button::new("ancestor-folders")
                        .icon(IconName::Ellipsis)
                        .tooltip("Ancestor folders")
                        .small()
                        .outline()
                        .dropdown_menu(move |mut menu, window, _| {
                            let viewport = window.viewport_size();
                            let menu_width = (viewport.width - px(24.)).min(px(620.));
                            let rem = window.rem_size();
                            let label_width = menu_width - rem * 0.5 - px(28.);
                            let measure =
                                |text: &SharedString| Self::path_label_width(window, text);
                            menu = menu
                                .scrollable(true)
                                .max_w(menu_width)
                                .max_h((viewport.height - px(96.)).max(px(100.)));
                            for path in ancestors.iter().rev() {
                                let name = paths::name(path);
                                let label = match path.parent() {
                                    Some(parent) => format!("{name} — {}", paths::display(parent)),
                                    None => paths::display(path),
                                };
                                let item = PopupMenuItem::new(label);
                                let caption =
                                    decision::abbreviate_folder(&name, label_width, &measure);
                                let item = if caption.as_ref() != name {
                                    item.display_label(caption)
                                } else {
                                    item
                                };
                                let path = path.clone();
                                let shell = shell.clone();
                                menu = menu.item(item.on_click(move |_, window, cx| {
                                    let shell = shell.clone();
                                    let path = path.clone();
                                    window.defer(cx, move |window, cx| {
                                        let _ = shell.update(cx, |shell, cx| {
                                            shell.activate_breadcrumb(path, window, cx);
                                        });
                                    });
                                }));
                            }
                            menu
                        }),
                )
            })
            .children(visible_ancestors.into_iter().map(|(path, caption, _)| {
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .gap_1()
                    .child(
                        Self::breadcrumb(path, false, cx)
                            .display_label(caption)
                            .max_w(segment_width),
                    )
                    .child("›")
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .child(
                        Self::breadcrumb(self.current.clone(), true, cx)
                            .display_label(current_caption)
                            .max_w_full(),
                    )
                    .child(div().id("edit-path-space").flex_1().h_6().on_click(
                        cx.listener(|this, _, window, cx| this.command("path", window, cx)),
                    )),
            )
            .child(
                self.command_button("edit-location", edit_label, "path", cx)
                    .flex_shrink_0()
                    .tooltip(paths::display(&self.current)),
            )
            .into_any_element()
    }

    pub(super) fn path_label_width(window: &Window, text: &SharedString) -> Pixels {
        let run = window.text_style().to_run(text.len());
        window
            .text_system()
            .shape_line(text.clone(), window.rem_size() * 0.875, &[run], None)
            .width
    }

    fn breadcrumb(path: PathBuf, current: bool, cx: &Context<Self>) -> Button {
        Button::new((
            ElementId::from("crumb"),
            SharedString::from(paths::display(&path)),
        ))
        .label(paths::name(&path))
        .tooltip(paths::display(&path))
        .small()
        .compact()
        .ghost()
        .selected(current)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.activate_breadcrumb(path.clone(), window, cx);
        }))
    }

    fn activate_breadcrumb(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.navigate(path);
        self.navigation_only = true;
        window.focus(&self.nav_focus);
        cx.notify();
    }

    pub(super) fn path_matches(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let current = self.query.read(cx).value();
        let completions = self
            .path_completions
            .as_ref()
            .filter(|c| c.value == current.as_str());
        let scale = self.settings.ui_scale;
        let floating = !self.panel_takes_over(self.narrow(window), window);
        if floating && completions.is_none() && self.path_hint.is_none() {
            return div().into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .px(px(8. * scale))
            .pb(px(4. * scale))
            .when(floating, |list| {
                list.absolute()
                    .top(px(2. * scale))
                    .left(px(8. * scale))
                    .w(px(600. * scale).min(window.viewport_size().width - px(16. * scale)))
                    .pt(px(6. * scale))
                    .occlude()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .shadow_lg()
            })
            .when_some(self.path_hint.clone(), |list, hint| {
                list.child(accessibility::message(
                    "path-completion-hint",
                    hint.clone(),
                    div()
                        .text_size(px(12. * scale))
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                ))
            })
            .when_some(completions, |list, completions| {
                let count = completions.matches.choices.len();
                // The completion Enter would open is the one last placed in the field.
                let current = (count > 0).then(|| (completions.next + count - 1) % count);
                let width = if floating {
                    px(600. * scale).min(window.viewport_size().width - px(16. * scale))
                } else {
                    window.viewport_size().width
                } - px(48. * scale);
                let measure = |text: &SharedString| Self::path_label_width(window, text);
                list.child(accessibility::listbox(
                    "path-completion-list",
                    "Path completions".into(),
                    div()
                        .id("path-completions")
                        .track_scroll(&self.path_scroll)
                        .when(self.path_hint.is_some(), |list| list.mt(px(4. * scale)))
                        .flex()
                        .flex_col()
                        .items_start()
                        .max_h(px(160. * scale).min(window.viewport_size().height * 0.5))
                        .overflow_y_scroll()
                        .children(completions.matches.choices.iter().enumerate().map(
                            |(i, choice)| {
                                let entry = choice.entry();
                                let name = format!(
                                    "{}{}",
                                    paths::name(&choice.path),
                                    if choice.directory { "/" } else { "" }
                                );
                                let caption = decision::abbreviate_folder(&name, width, &measure);
                                let selected = current == Some(i);
                                let (marker, fill) = theme::list_row(selected, true, cx);
                                Button::new(("path-match", i))
                                    .label(name)
                                    .display_label(caption)
                                    .accessibility_role(gpui::accesskit::Role::ListBoxOption)
                                    .tab_stop(false)
                                    .small()
                                    .ghost()
                                    .w_full()
                                    .justify_start()
                                    .selected(selected)
                                    .border_l(px(3. * scale))
                                    .border_color(marker)
                                    .when_some(fill, |row, fill| row.bg(fill))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.path_completions = None;
                                        this.path_hint = None;
                                        this.query.update(cx, |input, cx| {
                                            input.set_value(entry.clone(), window, cx);
                                            input.focus(window, cx);
                                        });
                                        cx.notify();
                                    }))
                            },
                        )),
                ))
            })
            .map(|list| {
                if floating {
                    accessibility::popup("path-completions-popup", "Path completions".into(), list)
                        .into_any_element()
                } else {
                    list.into_any_element()
                }
            })
    }
}
