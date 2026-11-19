use super::*;
use std::path::PathBuf;

pub(super) struct State {
    pub temporary: Option<PathBuf>,
    pub suspended: Option<(Panel, String)>,
    pub status: String,
    announce_status: bool,
    announcement: Option<Task<()>>,
    pub started: bool,
    pub running: bool,
    pub building: bool,
    pub query: String,
    pub selection: Option<usize>,
    pub focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
    pub picker_selection: Option<PathBuf>,
    pub takeover: bool,
    pub collapsed: bool,
}
impl State {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            temporary: None,
            suspended: None,
            status: "Enter a query, then press Enter to search".into(),
            announce_status: true,
            announcement: None,
            started: false,
            running: false,
            building: false,
            query: String::new(),
            selection: None,
            focus: cx.focus_handle().tab_stop(true),
            scroll: Default::default(),
            picker_selection: None,
            takeover: false,
            collapsed: false,
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
            cross_mounts: self.settings.search_mounts,
            directory_links: self.settings.search_directory_links,
            exclude: self.settings.search_exclude.clone(),
            file_bytes: self.settings.search_file_mib * 1024 * 1024,
        }
    }
    pub(super) fn reset_search(&mut self, status: &str) {
        self.search_ui.collapsed = false;
        self.search_id = self.search.cancel();
        self.hits = Default::default();
        self.search_ui.selection = None;
        self.search_ui.started = false;
        self.search_ui.running = false;
        self.search_ui.building = false;
        self.search_ui.status = status.into();
        self.search_ui.announcement.take();
        self.search_ui.announce_status = true;
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
        self.announce_search_when_quiet(cx);
        cx.notify();
    }
    pub(super) fn announce_search_when_quiet(&mut self, cx: &mut Context<Self>) {
        self.search_ui.announce_status = false;
        self.search_ui.announcement = Some(cx.spawn(async move |this, cx| {
            Timer::after(std::time::Duration::from_millis(400)).await;
            let _ = this.update(cx, |this, cx| {
                this.search_ui.announce_status = true;
                cx.notify();
            });
        }));
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
        self.search_ui.collapsed = false;
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
        self.search_ui.collapsed = self.search_ui.takeover;
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
            "Search folder: {scope}\nScope: {}\nIgnore files: {}\nRepository metadata: {}\nAdditional exclusions: {exclusions}\nSaved-file content limit: {} MiB per file. Unsaved buffers are excluded from recursive text search; use Find in Document for the active buffer.\n{} {}\nCoverage is bounded to 100,000 entries and 1,000 displayed matches. Unreadable paths and reached limits produce Partial. A filename index is a session snapshot with a 16 MiB path budget; filesystem notifications can mark it stale. Rebuild refreshes it, and Remove releases it. Content searches always read saved files.\nCurrent result state: {}",
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
            if self.settings.search_directory_links {
                "Follows directory symlinks, which can lead outside the search folder."
            } else {
                "Skips directory symlinks."
            },
            if self.settings.search_mounts {
                "Includes nested mounted volumes."
            } else {
                "Skips nested mounted volumes."
            },
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
    pub(super) fn search_scope_button(&self, cx: &Context<Self>) -> impl IntoElement {
        let scope = self.effective_scope();
        let label = format!(
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
        );
        self.search_scope_menu(label, self.search.index_status(scope).is_some(), cx)
    }
    pub(super) fn search_view(
        &self,
        takeover: bool,
        stacked: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let scope = self.effective_scope();
        let index = self.search.index_status(scope);
        let compatible = self.search.index_compatible(scope, &self.search_filters());
        let hits = self.hits.clone();
        let count = hits.len().min(1000);
        let scale = self.settings.ui_scale;
        let row_height = px(32. * scale);
        let status = format!(
            "{} · {} displayed (limit 1,000)",
            self.search_ui.status, count
        );
        let coverage = format!(
            "{} · metadata {} · content ≤ {} MiB/file · exclusions: {}",
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
        );
        let muted = cx.theme().muted_foreground;
        let small = px(12. * scale);
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .when(takeover, |panel| panel.flex_1())
            .when(!stacked || self.search_ui.running, |panel| {
                panel.child(
                    div()
                        .flex()
                        .items_center()
                        .flex_wrap()
                        .gap(px(8. * scale))
                        .px(px(8. * scale))
                        .flex_shrink_0()
                        .when(!stacked, |row| row.child(self.search_scope_button(cx)))
                        .when(self.search_ui.running, |row| {
                            row.child(self.button("Stop Search", "stop-search", cx))
                        }),
                )
            })
            .child(
                div()
                    .px(px(8. * scale))
                    .pt(px(4. * scale))
                    .flex_shrink_0()
                    .text_size(small)
                    .text_color(muted)
                    .when(stacked, |panel| {
                        panel.child(accessibility::status(
                            if self.search_ui.announce_status {
                                "search-result-announcement"
                            } else {
                                "search-result-status"
                            },
                            status.clone(),
                            div()
                                .text_color(cx.theme().foreground)
                                .truncate()
                                .child(status.clone()),
                            self.search_ui.announce_status,
                        ))
                    })
                    .child(
                        div()
                            .id("search-coverage")
                            .truncate()
                            .child(coverage.clone())
                            .tooltip(move |window, cx| {
                                gpui_component::tooltip::Tooltip::new(coverage.clone())
                                    .build(window, cx)
                            }),
                    )
                    .when(
                        self.settings.search_directory_links || self.settings.search_mounts,
                        |panel| {
                            let expansion = match (
                                self.settings.search_directory_links,
                                self.settings.search_mounts,
                            ) {
                                (true, true) => "Includes directory symlinks and mounted volumes",
                                (true, false) => "Includes directory symlinks",
                                (false, _) => "Includes mounted volumes",
                            };
                            panel.child(accessibility::message(
                                "search-traversal",
                                expansion.into(),
                                div().child(expansion),
                            ))
                        },
                    )
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
                            div().child(status),
                        ))
                    }),
            )
            .when(!stacked, |panel| {
                panel.child(
                    div()
                        .px(px(8. * scale))
                        .py(px(4. * scale))
                        .flex_shrink_0()
                        .child(accessibility::status(
                            if self.search_ui.announce_status {
                                "search-result-announcement"
                            } else {
                                "search-result-status"
                            },
                            status.clone(),
                            div().text_size(small).child(status.clone()),
                            self.search_ui.announce_status,
                        )),
                )
            })
            .when(stacked, |panel| {
                panel.child(div().h(px(4. * scale)).flex_shrink_0())
            })
            .when(count > 0, |panel| {
                panel.child(accessibility::collection(
                    "search-results",
                    "Search results".into(),
                    div()
                        .id("results")
                        .track_focus(&self.search_ui.focus)
                        .mx(px(8. * scale))
                        .mb(px(8. * scale))
                        .rounded(px(7. * scale))
                        .overflow_hidden()
                        .map(|results| {
                            if takeover {
                                results.flex_1().min_h(row_height)
                            } else {
                                results
                                    .h(row_height * count.min(8) + px(2.))
                                    .flex_shrink()
                                    .min_h(row_height * count.min(2) + px(2.))
                            }
                        })
                        .border_1()
                        .border_color(if self.search_ui.focus.is_focused(window) {
                            cx.theme().ring
                        } else {
                            cx.theme().border
                        })
                        .when(self.search_ui.focus.is_focused(window), |results| {
                            results.border_2()
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
                                                .tab_stop(false)
                                                .tooltip(tooltip)
                                                .child(
                                                    div().flex_1().min_w_0().truncate().child(text),
                                                )
                                                .ghost()
                                                .small()
                                                .w_full()
                                                .h(row_height)
                                                .justify_start()
                                                .selected(this.search_ui.selection == Some(i))
                                                .when(this.search_ui.selection == Some(i), |row| {
                                                    row.bg(cx.theme().selection)
                                                })
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
                    self.search_ui.focus.clone(),
                ))
            })
            .into_any_element()
    }
}
