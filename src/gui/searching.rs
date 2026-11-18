use super::*;
use std::path::PathBuf;

pub(super) struct State {
    pub temporary: Option<PathBuf>,
    pub suspended: Option<(Panel, String)>,
    pub status: String,
    pub started: bool,
    pub running: bool,
    pub building: bool,
    pub query: String,
    pub selection: Option<usize>,
    pub focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
    pub picker_selection: Option<PathBuf>,
}
impl State {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            temporary: None,
            suspended: None,
            status: "Enter a query, then press Enter to search".into(),
            started: false,
            running: false,
            building: false,
            query: String::new(),
            selection: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: Default::default(),
            picker_selection: None,
        }
    }
}
impl Shell {
    pub(super) fn resume_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((panel, value)) = self.search_ui.suspended.take() else {
            return false;
        };
        self.panel = Some(panel);
        self.query.update(cx, |input, cx| {
            input.set_placeholder(
                if panel == Panel::Files {
                    "Filename to find"
                } else {
                    "Text in saved files"
                },
                window,
                cx,
            );
            input.set_value(value, window, cx);
            input.focus(window, cx);
        });
        true
    }
    pub(super) fn palette_command(
        &mut self,
        command: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.resume_search(window, cx) {
            self.panel = None;
            self.focus(window, cx);
        }
        self.command(command, window, cx);
    }
    pub(super) fn effective_scope(&self) -> &PathBuf {
        self.search_ui
            .temporary
            .as_ref()
            .or(self.scope.as_ref())
            .unwrap_or(&self.current)
    }
    pub(super) fn search_filters(&self) -> search::Filters {
        search::Filters {
            ignored: self.settings.search_ignored,
            metadata: self.settings.search_metadata,
            exclude: self.settings.search_exclude.clone(),
            file_bytes: self.settings.search_file_mib * 1024 * 1024,
        }
    }
    pub(super) fn reset_search(&mut self, status: &str) {
        self.search_id = self.search.cancel();
        self.hits = Default::default();
        self.search_ui.selection = None;
        self.search_ui.started = false;
        self.search_ui.running = false;
        self.search_ui.building = false;
        self.search_ui.status = status.into();
    }
    pub(super) fn start_search(&mut self, cx: &App) {
        let query = self.query.read(cx).value().to_string();
        self.start_search_value(query, self.panel == Some(Panel::Text));
    }
    pub(super) fn start_search_value(&mut self, query: String, content: bool) {
        self.reset_search("Searching saved files…");
        self.search_ui.query = query.clone();
        if query.is_empty() {
            self.search_ui.status =
                "Enter a query. Index This Folder builds a filename snapshot explicitly.".into();
            return;
        }
        match self.search.start_filtered(
            self.effective_scope().clone(),
            query,
            content,
            self.search_filters(),
        ) {
            Ok(id) => {
                glimpse::metrics::event("search-request", id, u64::from(content));
                self.search_id = id;
                self.search_ui.started = true;
                self.search_ui.running = true;
            }
            Err(error) => self.search_ui.status = format!("Search failed: {error}"),
        }
    }
    pub(super) fn search_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        if query == self.search_ui.query {
            return;
        }
        let update = self.search_ui.started
            || self.search_ui.building
            || (self.panel == Some(Panel::Files)
                && self
                    .search
                    .index_compatible(self.effective_scope(), &self.search_filters()));
        self.search_ui.query = query;
        self.reset_search("Query changed; press Enter to search");
        if update {
            self.start_search(cx);
        }
        cx.notify();
    }
    pub(super) fn build_index(&mut self) {
        self.reset_search("Building filename snapshot…");
        match self.search.start_filtered(
            self.effective_scope().clone(),
            String::new(),
            false,
            self.search_filters(),
        ) {
            Ok(id) => {
                glimpse::metrics::event("index-request", id, 0);
                self.search_id = id;
                self.search_ui.running = true;
                self.search_ui.building = true;
            }
            Err(error) => self.search_ui.status = format!("Index failed: {error}"),
        }
    }
    pub(super) fn focus_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hits.is_empty() {
            self.search_ui.status = "No search results to focus".into();
            return;
        }
        let index = self
            .search_ui
            .selection
            .unwrap_or(0)
            .min(self.hits.len() - 1);
        self.search_ui.selection = Some(index);
        self.search_ui.picker_selection = Some(self.hits[index].path.clone());
        self.search_ui
            .scroll
            .scroll_to_item(index, ScrollStrategy::Top);
        window.focus(&self.search_ui.focus);
        cx.notify();
    }
    pub(super) fn open_hit(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(hit) = self.hits.get(index) else {
            return;
        };
        let (path, line, fingerprint) = (hit.path.clone(), hit.line, hit.fingerprint);
        self.search_ui.selection = Some(index);
        self.search_ui.picker_selection = Some(path.clone());
        if let Some(token) = self.open(path, false, fingerprint.map(|_| (line, 1)))
            && let Some(fingerprint) = fingerprint
        {
            self.hit_versions
                .insert(token, (fingerprint, self.search_id));
        }
        cx.notify();
    }
    pub(super) fn search_key(
        &mut self,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if key.modifiers.control || key.modifiers.platform {
            return false;
        }
        let count = self.hits.len();
        if count == 0 {
            return false;
        }
        let index = self.search_ui.selection.unwrap_or(0).min(count - 1);
        let next = match key.key.as_str() {
            "up" => index.saturating_sub(1),
            "down" => (index + 1).min(count - 1),
            "home" => 0,
            "end" => count - 1,
            "pageup" => index.saturating_sub(5),
            "pagedown" => (index + 5).min(count - 1),
            "enter" => {
                self.open_hit(index, cx);
                return true;
            }
            "escape" => {
                self.command("dismiss", window, cx);
                return true;
            }
            _ => return false,
        };
        self.search_ui.selection = Some(next);
        self.search_ui.picker_selection = Some(self.hits[next].path.clone());
        self.search_ui
            .scroll
            .scroll_to_item(next, ScrollStrategy::Top);
        cx.notify();
        true
    }
    pub(super) fn search_details(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope = paths::display(self.effective_scope());
        let exclusions = if self.settings.search_exclude.is_empty() {
            "None"
        } else {
            &self.settings.search_exclude
        };
        let detail = format!(
            "Search folder: {scope}\nScope: {}\nIgnore files: {}\nRepository metadata: {}\nAdditional exclusions: {exclusions}\nSaved-file content limit: {} MiB per file. Unsaved buffers are excluded from recursive text search; use Find in Document for the active buffer.\nRecursive traversal does not follow directory symlinks or cross nested mounts.\nCoverage is bounded to 100,000 entries and 1,000 displayed matches. Unreadable paths and reached limits produce Partial. A filename index is a session snapshot with a 16 MiB path budget; filesystem notifications can mark it stale. Rebuild refreshes it, and Remove releases it. Content searches always read saved files.\nCurrent result state: {}",
            if self.search_ui.temporary.is_some() {
                "temporary; existing pin retained"
            } else if self.scope.is_some() {
                "pinned"
            } else {
                "follows Location"
            },
            if self.settings.search_ignored {
                "include ignored paths"
            } else {
                "respect ignore rules"
            },
            if self.settings.search_metadata {
                "included"
            } else {
                "excluded"
            },
            self.settings.search_file_mib,
            self.search_ui.status
        );
        self.ask_decision(
            DecisionAction::Notice,
            "Search scope and coverage".into(),
            detail,
            vec!["Close"],
            window,
            cx,
        );
    }
    pub(super) fn search_view(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let scope = self.effective_scope();
        let index = self.search.index_status(scope);
        let compatible = self.search.index_compatible(scope, &self.search_filters());
        let hits = self.hits.clone();
        let count = hits.len().min(1000);
        let row_height = px(32. * self.settings.ui_scale);
        let status = format!(
            "{} · {} displayed (limit 1,000)",
            self.search_ui.status, count
        );
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .px_2()
                    .child(
                        self.command_button(
                            "search-scope-heading",
                            format!(
                                "{} in {} ({})",
                                if self.panel == Some(Panel::Text) {
                                    "Saved text"
                                } else {
                                    "Files"
                                },
                                paths::name(scope),
                                if self.search_ui.temporary.is_some() {
                                    "temporary"
                                } else if self.scope.is_some() {
                                    "pinned"
                                } else {
                                    "Location"
                                }
                            ),
                            "search-details",
                            cx,
                        )
                        .tooltip(paths::display(scope)),
                    )
                    .child(self.button("This Folder", "search-here", cx))
                    .child(self.button(
                        if self.scope.is_some() {
                            "Unpin"
                        } else {
                            "Pin Here"
                        },
                        "scope",
                        cx,
                    ))
                    .child(self.button("Go to Scope", "go-scope", cx))
                    .child(self.button(
                        if index.is_some() {
                            "Rebuild Index"
                        } else {
                            "Index This Folder"
                        },
                        "index-folder",
                        cx,
                    ))
                    .when(index.is_some(), |row| {
                        row.child(self.button("Remove Index", "remove-index", cx))
                    })
                    .when(self.search_ui.running, |row| {
                        row.child(self.button("Stop Search", "stop-search", cx))
                    }),
            )
            .child(div().px_2().text_sm().truncate().child(format!(
                "{} · metadata {} · symlinks excluded · content ≤ {} MiB/file · exclusions: {}",
                if self.settings.search_ignored {
                    "Ignore rules bypassed"
                } else {
                    "Ignore rules respected"
                },
                if self.settings.search_metadata {
                    "included"
                } else {
                    "excluded"
                },
                self.settings.search_file_mib,
                if self.settings.search_exclude.is_empty() {
                    "none".into()
                } else {
                    format!("{:?}", self.settings.search_exclude)
                }
            )))
            .when_some(index, |panel, index| {
                let status = format!(
                    "{index}{}",
                    if compatible {
                        ""
                    } else {
                        " · settings changed; rebuild to use"
                    }
                );
                panel.child(accessibility::message(
                    "filename-index-status",
                    status.clone(),
                    div().px_2().text_sm().child(status),
                ))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(accessibility::message(
                        "search-result-status",
                        status.clone(),
                        div().flex_1().min_w_0().text_sm().child(status),
                    ))
                    .when(count > 0, |row| {
                        row.child(self.button("Focus Results", "focus-results", cx))
                    })
                    .child(self.button("Coverage…", "search-details", cx)),
            )
            .when(count > 0, |panel| {
                panel.child(
                    div()
                        .id("results")
                        .track_focus(&self.search_ui.focus)
                        .h((row_height * count.min(5)).min(window.viewport_size().height * 0.22))
                        .min_h(row_height)
                        .border_1()
                        .border_color(if self.search_ui.focus.is_focused(window) {
                            cx.theme().primary
                        } else {
                            cx.theme().border
                        })
                        .child(
                            uniform_list(
                                "search-hits",
                                count,
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|i| {
                                            let hit = &hits[i];
                                            let name = paths::display(
                                                hit.path
                                                    .strip_prefix(this.effective_scope())
                                                    .unwrap_or(&hit.path),
                                            );
                                            let prefix = if hit.fingerprint.is_some() {
                                                format!("{name}:{}  ", hit.line)
                                            } else {
                                                format!("{name}  ")
                                            };
                                            let label = format!("{prefix}{}", hit.excerpt);
                                            let ranges = if hit.fingerprint.is_some() {
                                                hit.excerpt
                                                    .match_indices(&hit.query)
                                                    .map(|(start, text)| {
                                                        (prefix.len() + start)
                                                            ..(prefix.len() + start + text.len())
                                                    })
                                                    .collect()
                                            } else {
                                                search::highlight_ranges(&name, &hit.query)
                                            };
                                            let tooltip = if hit.fingerprint.is_some() {
                                                format!(
                                                    "{}:{}\n{}",
                                                    paths::display(&hit.path),
                                                    hit.line,
                                                    hit.excerpt
                                                )
                                            } else {
                                                paths::display(&hit.path)
                                            };
                                            let text = StyledText::new(label).with_highlights(
                                                ranges.into_iter().map(|range| {
                                                    (
                                                        range,
                                                        HighlightStyle {
                                                            font_weight: Some(FontWeight::BOLD),
                                                            ..Default::default()
                                                        },
                                                    )
                                                }),
                                            );
                                            Button::new(("search-hit", i))
                                                .accessibility_role(
                                                    gpui::accesskit::Role::ListBoxOption,
                                                )
                                                .accessibility_focus(this.search_ui.focus.clone())
                                                .tooltip(tooltip)
                                                .child(
                                                    div().flex_1().min_w_0().truncate().child(text),
                                                )
                                                .small()
                                                .w_full()
                                                .h(row_height)
                                                .justify_start()
                                                .selected(this.search_ui.selection == Some(i))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.open_hit(i, cx)
                                                }))
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .h_full()
                            .track_scroll(self.search_ui.scroll.clone()),
                        ),
                )
            })
            .into_any_element()
    }
}
