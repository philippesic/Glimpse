use super::*;

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
            Panel::Filter => "Filter this folder",
            Panel::Files => "Find files",
            Panel::Text => "Find saved text",
            Panel::Palette => "Commands",
            Panel::Recovery => "Recovery",
            Panel::Line => "Go to line",
            Panel::NewFile => "New file name",
            Panel::NewFolder => "New folder name",
            Panel::Rename => "New name",
        };
        let search = matches!(panel, Panel::Files | Panel::Text);
        let stacked = window.viewport_size().width < px(760. * scale);
        let actions = div()
            .flex()
            .gap(px(8. * scale))
            .flex_shrink_0()
            .when(!matches!(panel, Panel::Recovery | Panel::Filter), |row| {
                row.child(self.button(if search { "Search" } else { "Apply" }, "submit", cx))
            })
            .child(self.button("Close", "dismiss", cx));
        let input = Input::new(&self.query).flex_1().min_w(px(120. * scale));
        let header = if stacked {
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
                                        .child(self.search_scope_button(cx)),
                                )
                            } else {
                                row.child(div().flex_1().min_w_0().truncate().child(title))
                            }
                        })
                        .child(actions),
                )
                .child(div().flex().child(input))
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(8. * scale))
                .px(px(8. * scale))
                .py(px(6. * scale))
                .flex_shrink_0()
                .child(div().flex_shrink_0().child(title))
                .child(input)
                .child(actions)
        };
        let section = div()
            .id("panel-section")
            .flex()
            .flex_col()
            .min_h_0()
            .bg(cx.theme().background)
            .border_b_1()
            .border_color(cx.theme().border)
            .map(|section| {
                if takeover {
                    section.flex_1().overflow_y_scroll()
                } else {
                    section
                        .flex_shrink_0()
                        .max_h(window.viewport_size().height * 0.5)
                }
            })
            .child(header);
        let section = match panel {
            Panel::Files | Panel::Text if self.search_ui.collapsed => {
                section.child(self.collapsed_results(cx))
            }
            Panel::Files | Panel::Text => {
                section.child(self.search_view(takeover, stacked, window, cx))
            }
            Panel::Palette => section.child(self.palette_view(takeover, cx)),
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
        let element = accessibility::group("panel-section", title.into(), section);
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

    fn palette_view(&self, takeover: bool, cx: &Context<Self>) -> impl IntoElement {
        accessibility::group(
            "palette-list",
            "Matching commands".into(),
            self.palette_list(takeover, cx),
        )
    }

    fn palette_list(&self, takeover: bool, cx: &Context<Self>) -> impl IntoElement {
        let scale = self.settings.ui_scale;
        let query = self.query.read(cx).value().to_lowercase();
        let row_height = px(28. * scale);
        div()
            .id("palette")
            .flex()
            .flex_col()
            .px(px(4. * scale))
            .pb(px(4. * scale))
            .min_h_0()
            .overflow_y_scroll()
            .map(|list| {
                if takeover {
                    list.flex_1()
                } else {
                    list.max_h(px(280. * scale))
                }
            })
            .children(
                glimpse::commands::COMMANDS
                    .iter()
                    .filter(|c| c.label.to_lowercase().contains(&query) || c.id.contains(&query))
                    .map(|command| {
                        let key = self
                            .settings
                            .keymaps
                            .get(command.id)
                            .map(String::as_str)
                            .unwrap_or(command.shortcut);
                        let shortcut = (!key.is_empty()).then(|| platform::shortcut_label(key));
                        let enabled = self.command_enabled(command.id);
                        Button::new(SharedString::from(format!("palette-{}", command.id)))
                            .label(command.label)
                            .when_some(shortcut.clone(), |row, shortcut| {
                                row.accessibility_description(shortcut)
                            })
                            .ghost()
                            .small()
                            .w_full()
                            .h(row_height)
                            .flex_shrink_0()
                            .justify_start()
                            .disabled(!enabled)
                            .when_some(shortcut, |row, shortcut| {
                                row.child(
                                    div()
                                        .ml_auto()
                                        .pl(px(16. * scale))
                                        .flex_shrink_0()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(shortcut),
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.palette_command(command.id, window, cx)
                            }))
                    }),
            )
    }
}
