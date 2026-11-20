use super::*;

pub(super) struct PaletteState {
    pub focus: FocusHandle,
    pub overlay: FocusHandle,
    pub floating: bool,
    pub selection: usize,
    pub scroll: ScrollHandle,
}

impl PaletteState {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            overlay: cx.focus_handle(),
            floating: false,
            selection: 0,
            scroll: ScrollHandle::new(),
        }
    }
}

impl Shell {
    /// Search and command panels take the whole main area when the window cannot fit
    /// them beside a usable document, as narrow layouts show one surface at a time.
    pub(super) fn panel_takes_over(&self, narrow: bool, window: &Window) -> bool {
        let listing = match self.panel {
            Some(Panel::Files | Panel::Text) => !self.search_ui.collapsed,
            Some(Panel::Palette | Panel::Recovery) => true,
            Some(Panel::Path) => self.path_completions.is_some() || self.path_hint.is_some(),
            _ => false,
        };
        listing && (narrow || window.viewport_size().height < px(560. * self.settings.ui_scale))
    }

    pub(super) fn navigator_filter(&self, cx: &Context<Self>) -> impl IntoElement {
        let scale = self.settings.ui_scale;
        accessibility::group(
            "navigator-filter",
            "Filter This Folder".into(),
            div()
                .flex()
                .items_center()
                .gap(px(4. * scale))
                .px(px(8. * scale))
                .py(px(6. * scale))
                .flex_shrink_0()
                .bg(cx.theme().sidebar)
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .child(Input::new(&self.query).flex_1().min_w_0())
                .child(
                    Button::new("filter-close")
                        .icon(IconName::Close)
                        .tooltip("Close Filter")
                        .ghost()
                        .small()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.command("dismiss", window, cx)),
                        ),
                ),
        )
    }

    pub(super) fn panel_view(
        &self,
        panel: Panel,
        takeover: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let scale = self.settings.ui_scale;
        let title = match panel {
            Panel::Path => "Location",
            Panel::Filter => "Filter This Folder",
            Panel::Files => "Find Files",
            Panel::Text => "Find Text in Saved Files",
            Panel::Palette => "Commands",
            Panel::Recovery => "Recovery",
            Panel::Line => "Go to Line",
            Panel::NewFile => "New File Name",
            Panel::NewFolder => "New Folder Name",
            Panel::Rename => "New Name",
        };
        let naming = matches!(panel, Panel::NewFile | Panel::NewFolder | Panel::Rename);
        let search = matches!(panel, Panel::Files | Panel::Text);
        let viewport = window.viewport_size();
        let stacked = viewport.width < px(760. * scale);
        let submit = (!matches!(panel, Panel::Recovery | Panel::Filter)).then(|| {
            self.button(
                match panel {
                    Panel::Files | Panel::Text => "Search",
                    Panel::Palette => "Run",
                    Panel::Line => "Go",
                    Panel::NewFile | Panel::NewFolder => "Create",
                    Panel::Rename => "Rename",
                    _ => "Apply",
                },
                "submit",
                cx,
            )
            .when(naming && self.name_request.is_some(), |submit| {
                submit.disabled(true)
            })
            .when(panel == Panel::Palette, |run| {
                run.disabled(!self.palette_runnable(cx))
            })
        });
        let title = self
            .rename_path
            .as_ref()
            .filter(|_| panel == Panel::Rename)
            .map_or_else(
                || title.to_owned(),
                |path| {
                    let file = paths::name(path);
                    let measure = |text: &SharedString| Self::path_label_width(window, text);
                    let room = if stacked {
                        viewport.width - px(76. * scale)
                    } else {
                        viewport.width * 0.32
                    };
                    let filename = labels::filename(
                        &file,
                        "",
                        None,
                        room - measure(&"Rename ".into()),
                        &measure,
                    );
                    format!("Rename {filename}")
                },
            );
        let input = Input::new(&self.query).flex_1().min_w(px(120. * scale));
        let header = if stacked {
            let close = Button::new("panel-close")
                .icon(IconName::Close)
                .tooltip("Close")
                .ghost()
                .small()
                .on_click(cx.listener(|this, _, window, cx| this.command("dismiss", window, cx)));
            div()
                .flex()
                .flex_col()
                .gap(px(6. * scale))
                .px(px(8. * scale))
                .py(px(6. * scale))
                .flex_shrink_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8. * scale))
                        .map(|row| {
                            if search {
                                row.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .child(self.search_scope_button(true, window, cx)),
                                )
                            } else {
                                row.child(div().flex_1().min_w_0().truncate().child(title.clone()))
                            }
                        })
                        .child(close),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8. * scale))
                        .when(panel == Panel::Rename, |row| {
                            row.child(div().flex_shrink_0().child("New Name"))
                        })
                        .child(input)
                        .children(submit),
                )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(8. * scale))
                .px(px(8. * scale))
                .py(px(6. * scale))
                .flex_shrink_0()
                .child(div().flex_shrink_0().child(title.clone()))
                .when(panel == Panel::Rename, |row| {
                    row.child(div().flex_shrink_0().child("New Name"))
                })
                .child(input)
                .children(submit)
                .child(self.button("Close", "dismiss", cx))
        };
        let section = div()
            .id("panel-section")
            .track_focus(&self.palette.overlay)
            .flex()
            .flex_col()
            .min_h_0()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .map(|section| {
                if takeover {
                    section.flex_1().overflow_y_scroll()
                } else if search && !self.search_ui.collapsed {
                    section
                        .flex_shrink_0()
                        .h((viewport.height * 0.42).min(px(320. * scale)))
                } else {
                    section.flex_shrink_0().max_h(viewport.height * 0.5)
                }
            })
            .child(header);
        let section = section.when(panel == Panel::Line || naming, |section| {
            let scale = self.settings.ui_scale;
            section.child(
                div()
                    .px(px(8. * scale))
                    .h(px(24. * scale))
                    .flex_shrink_0()
                    .when_some(self.query_error.clone(), |row, error| {
                        row.child(accessibility::message(
                            "query-error",
                            error.clone(),
                            div().text_color(cx.theme().danger).child(error),
                        ))
                    }),
            )
        });
        let section = match panel {
            Panel::Files | Panel::Text if self.search_ui.collapsed => {
                section.child(self.collapsed_results(cx))
            }
            Panel::Files | Panel::Text => section.child(self.search_view(stacked, window, cx)),
            Panel::Palette => section.child(self.palette_view(takeover, window, cx)),
            Panel::Recovery => section.child(
                div()
                    .id("recovery-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.recovery_view(cx)),
            ),
            _ => section,
        };
        let element = accessibility::group("panel-section", title, section);
        if takeover {
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(element)
                .into_any_element()
        } else {
            element.into_any_element()
        }
    }

    fn collapsed_results(&self, cx: &Context<Self>) -> impl IntoElement {
        let scale = self.settings.ui_scale;
        let count = self.hits.len().min(1000);
        let label = format!(
            "{count} {} · {}",
            if count == 1 { "result" } else { "results" },
            self.search_ui.status
        );
        div()
            .flex()
            .items_center()
            .gap(px(8. * scale))
            .px(px(8. * scale))
            .pb(px(6. * scale))
            .flex_shrink_0()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12. * scale))
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(self.button("Show Results", "focus-results", cx))
    }

    pub(super) fn palette_matches(&self, cx: &App) -> Vec<&'static str> {
        let query = self.query.read(cx).value().to_lowercase();
        glimpse::commands::COMMANDS
            .iter()
            .filter(|c| c.label.to_lowercase().contains(&query) || c.id.contains(&query))
            .map(|c| c.id)
            .collect()
    }

    /// Run acts on the selected command, so it is unavailable when that command is.
    pub(super) fn palette_runnable(&self, cx: &App) -> bool {
        let matches = self.palette_matches(cx);
        matches
            .get(self.palette.selection.min(matches.len().saturating_sub(1)))
            .is_some_and(|id| self.command_enabled(id))
    }

    pub(super) fn focus_palette(
        &mut self,
        last: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.palette_matches(cx).len();
        if count == 0 {
            return;
        }
        self.palette.selection = if last { count - 1 } else { 0 };
        self.palette.scroll.scroll_to_item(self.palette.selection);
        window.focus(&self.palette.focus);
        cx.notify();
    }

    pub(super) fn palette_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if key.modifiers.control || key.modifiers.platform || key.modifiers.alt {
            return false;
        }
        let matches = self.palette_matches(cx);
        if matches.is_empty() {
            return false;
        }
        let index = self.palette.selection.min(matches.len() - 1);
        let next = match key.key.as_str() {
            "up" if index == 0 => {
                self.query.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
                return true;
            }
            "enter" | "space" => {
                self.palette_command(matches[index], window, cx);
                return true;
            }
            other => match keyboard::list_step(other, Some(index), matches.len(), 8) {
                Some(next) => next,
                None => return false,
            },
        };
        self.palette.selection = next;
        self.palette.scroll.scroll_to_item(next);
        cx.notify();
        true
    }

    pub(super) fn palette_overlay(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let scale = self.settings.ui_scale;
        let width = px(640. * scale).min(window.viewport_size().width - px(32. * scale));
        div()
            .absolute()
            .top(px(8. * scale))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(accessibility::popup(
                "palette-overlay",
                "Commands".into(),
                div()
                    .id("palette-overlay")
                    .track_focus(&self.palette.overlay)
                    .occlude()
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                        if this.panel == Some(Panel::Palette) {
                            this.command("dismiss", window, cx);
                        }
                    }))
                    .w(width)
                    .flex()
                    .flex_col()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius_lg)
                    .shadow_lg()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8. * scale))
                            .p(px(8. * scale))
                            .child(Input::new(&self.query).flex_1().min_w_0())
                            .child(
                                self.button("Run", "submit", cx)
                                    .disabled(!self.palette_runnable(cx)),
                            )
                            .child(self.button("Close", "dismiss", cx)),
                    )
                    .child(self.palette_view(false, window, cx)),
            ))
            .into_any_element()
    }

    fn palette_view(
        &self,
        takeover: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        accessibility::collection(
            "palette-list",
            "Matching commands".into(),
            self.palette_list(takeover, window, cx),
            self.palette.focus.clone(),
        )
    }

    fn palette_list(
        &self,
        takeover: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let scale = self.settings.ui_scale;
        let row_height = px(28. * scale);
        let matches = self.palette_matches(cx);
        let focused = self.palette.focus.is_focused(window);
        let selection = self.palette.selection.min(matches.len().saturating_sub(1));
        div()
            .id("palette")
            .track_focus(&self.palette.focus)
            .track_scroll(&self.palette.scroll)
            .flex()
            .flex_col()
            .mx(px(4. * scale))
            .mb(px(4. * scale))
            .p(px(2. * scale))
            .rounded(px(7. * scale))
            .border_2()
            .border_color(if focused {
                cx.theme().ring
            } else {
                gpui::transparent_black()
            })
            .min_h_0()
            .overflow_y_scroll()
            .map(|list| {
                if takeover {
                    list.flex_1()
                } else {
                    list.max_h(px(280. * scale))
                }
            })
            .when(matches.is_empty(), |list| {
                list.child(accessibility::message(
                    "palette-empty",
                    "No matching commands".into(),
                    div()
                        .px(px(12. * scale))
                        .py(px(6. * scale))
                        .text_color(cx.theme().muted_foreground)
                        .child("No matching commands"),
                ))
            })
            .children(matches.into_iter().enumerate().map(|(index, id)| {
                let command = glimpse::commands::COMMANDS
                    .iter()
                    .find(|command| command.id == id)
                    .unwrap();
                let key = self
                    .settings
                    .keymaps
                    .get(command.id)
                    .map(String::as_str)
                    .unwrap_or(command.shortcut);
                let key = if key.is_empty() {
                    glimpse::commands::editing_hint(command.id).unwrap_or_default()
                } else {
                    key
                };
                let shortcut = if key.is_empty() {
                    SharedString::default()
                } else {
                    SharedString::from(platform::shortcut_label(key))
                };
                let enabled = self.command_enabled(command.id);
                let selected = index == selection;
                let active = focused || self.query.focus_handle(cx).is_focused(window);
                let (marker, fill) = theme::list_row(selected, active, cx);
                Button::new(SharedString::from(format!("palette-{}", command.id)))
                    .label(command.label)
                    .accessibility_role(gpui::accesskit::Role::ListBoxOption)
                    .accessibility_focus(self.palette.focus.clone())
                    .when(!shortcut.is_empty(), |row| {
                        row.accessibility_description(shortcut.clone())
                    })
                    .tab_stop(false)
                    .ghost()
                    .small()
                    .w_full()
                    .h(row_height)
                    .flex_shrink_0()
                    .justify_start()
                    .disabled(!enabled)
                    .selected(selected)
                    .border_l(px(3. * scale))
                    .border_y_0()
                    .border_r_0()
                    .border_color(marker)
                    .when_some(fill, |row, fill| row.bg(fill))
                    .child(
                        div()
                            .ml_auto()
                            .pl(px(16. * scale))
                            .flex_shrink_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(shortcut),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.palette_command(command.id, window, cx)
                    }))
            }))
    }
}
