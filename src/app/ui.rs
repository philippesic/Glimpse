use super::*;
use appearance::{Symbol, icon_button};
use commands::Action;

#[derive(Clone, Default)]
struct Presentation {
    narrow_document: bool,
    query_changed: bool,
    query_submit: bool,
    replace_open: bool,
    go_line: Option<String>,
    find_cache: Option<FindCache>,
    operation_shown: Option<u8>,
    recovery_notice_dismissed: bool,
    navigator_hidden: bool,
    path_open: bool,
    filter_open: bool,
    search_selected: Option<(PathBuf, usize)>,
    search_results_focus: bool,
    search_anchor: Option<egui::Rect>,
    search_down: bool,
    search_up: bool,
    search_activate: bool,
    settings_page: usize,
    settings_draft: Option<storage::Settings>,
    settings_error: Option<String>,
    shortcut_filter: String,
    was_overlay: bool,
    was_search: bool,
    decision: Option<ConflictDecision>,
    comparison: Option<ConflictPreview>,
    record_to_trash: Option<PathBuf>,
    was_popup: bool,
    restore_after_popup: bool,
    restore_editor_next_pass: bool,
}

#[derive(Clone)]
struct FindCache {
    key: (u64, u64, String, bool, bool),
    regex: regex::Regex,
    ranges: Vec<std::ops::Range<usize>>,
    truncated: bool,
}

fn select_field(ctx: &egui::Context, response: &egui::Response, length: usize) {
    focus_field(ctx, response.id, length);
}

fn focus_field(ctx: &egui::Context, id: egui::Id, length: usize) {
    ctx.memory_mut(|memory| memory.request_focus(id));
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(length),
        )));
    state.store(ctx, id);
}

#[derive(Clone)]
struct ConflictPreview {
    id: u64,
    generation: u64,
    disk_prefix: String,
    disk_len: usize,
    diff: std::sync::Arc<crate::comparison::Comparison>,
}

#[derive(Clone)]
struct ConflictDecision {
    id: u64,
    generation: u64,
    disk: std::sync::Arc<DiskFile>,
    overwrite: bool,
}

impl Glimpse {
    fn menu_action(&mut self, ui: &mut egui::Ui, action: Action) {
        if let Some(command) = commands::COMMANDS.iter().find(|c| c.action == action) {
            let key = self
                .settings
                .keymaps
                .get(command.id)
                .map(String::as_str)
                .unwrap_or(command.shortcut);
            if ui
                .add(egui::Button::new(command.label).shortcut_text(key))
                .clicked()
            {
                self.command(action, ui.ctx());
                ui.close();
            }
        }
    }

    fn toolbar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, view: &mut Presentation) {
        let narrow = ui.available_width() < 760.;
        egui::Panel::top("window_toolbar")
            .frame(appearance::toolbar_frame(ui).inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 8,
            }))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 32.),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 6.;
                        ui.spacing_mut().interact_size.y = 30.;
                        // Keep only the native traffic-light region clear.
                        ui.add_space(72. / ctx.zoom_factor());
                        if icon_button(
                            ui,
                            Symbol::Sidebar,
                            "Show / Hide Navigator",
                            if narrow {
                                !view.narrow_document
                            } else {
                                !view.navigator_hidden
                            },
                        )
                        .clicked()
                        {
                            if narrow {
                                view.narrow_document = !view.narrow_document;
                                self.focus_editor = view.narrow_document;
                            } else {
                                view.navigator_hidden = !view.navigator_hidden;
                                self.focus_editor = view.navigator_hidden;
                            }
                            if !self.focus_editor {
                                self.native.focus_host();
                            }
                        }
                        for (symbol, label, action, available) in [
                            (
                                Symbol::Back,
                                "Previous directory · ⌘⌥←",
                                Action::Back,
                                !self.history.is_empty(),
                            ),
                            (
                                Symbol::Forward,
                                "Next directory · ⌘⌥→",
                                Action::Forward,
                                !self.future.is_empty(),
                            ),
                        ] {
                            let response = ui
                                .scope_builder(egui::UiBuilder::new(), |ui| {
                                    if !available {
                                        ui.disable();
                                    }
                                    appearance::symbol_button(
                                        ui,
                                        symbol,
                                        label,
                                        false,
                                        egui::vec2(30., 30.),
                                    )
                                })
                                .inner;
                            if response.clicked() {
                                self.command(action, ctx);
                            }
                        }
                        let search_width = if narrow { 120. } else { 240. };
                        let path_width = (ui.available_width() - search_width - 52.).max(70.);
                        ui.allocate_ui_with_layout(
                            egui::vec2(path_width, 30.),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_clip_rect(ui.max_rect());
                                if self.path_focus {
                                    view.path_open = true;
                                }
                                if view.path_open {
                                    self.path_bar(ctx, ui, view);
                                } else {
                                    egui::Frame::new()
                                        .fill(appearance::palette(ui).raised.gamma_multiply(0.65))
                                        .stroke(egui::Stroke::new(
                                            1.,
                                            appearance::palette(ui).separator,
                                        ))
                                        .corner_radius(7.)
                                        .inner_margin(egui::Margin::symmetric(6, 0))
                                        .show(ui, |ui| {
                                            ui.set_width((path_width - 14.).max(40.));
                                            let (icon, _) = ui.allocate_exact_size(
                                                egui::vec2(16., 16.),
                                                egui::Sense::hover(),
                                            );
                                            appearance::symbol(
                                                ui,
                                                icon,
                                                Symbol::Folder,
                                                appearance::palette(ui).accent,
                                            );
                                            self.breadcrumbs(ui);
                                            let blank = ui.allocate_response(
                                                egui::vec2(ui.available_width().max(12.), 30.),
                                                egui::Sense::click(),
                                            );
                                            if blank.clicked() {
                                                self.command(Action::Path, ctx);
                                            }
                                            blank.on_hover_text("Enter path · ⌘L");
                                        });
                                }
                            },
                        );
                        if self.search_focus || (self.search_open && !view.was_search) {
                            self.native.focus_host();
                            focus_field(
                                ctx,
                                egui::Id::unique("scoped_query"),
                                self.query.chars().count(),
                            );
                            self.search_focus = false;
                            self.focus_editor = false;
                        }
                        let response = ui.add_sized(
                            [search_width, 30.],
                            egui::TextEdit::singleline(&mut self.query)
                                .id(egui::Id::unique("scoped_query"))
                                .hint_text(if self.content_search {
                                    "Search text · ⌘⇧F"
                                } else {
                                    "Search files · ⌘P"
                                }),
                        );
                        if response.clicked() || response.gained_focus() {
                            self.search_open = true;
                            self.navigator.focused = false;
                            self.native.focus_host();
                        }
                        if response.clicked() || response.gained_focus() || response.changed() {
                            view.search_results_focus = false;
                            view.search_selected = None;
                        }
                        view.search_anchor = Some(response.rect);
                        view.search_down = response.has_focus()
                            && ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                            });
                        view.search_up = response.has_focus()
                            && ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                            });
                        view.search_activate = view.search_results_focus
                            && (response.has_focus() || response.lost_focus())
                            && ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                        view.query_changed = response.changed();
                        view.query_submit = !view.search_results_focus
                            && (response.has_focus() || response.lost_focus())
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.scope(|ui| {
                            ui.visuals_mut().widgets.inactive.weak_bg_fill =
                                egui::Color32::TRANSPARENT;
                            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            ui.menu_button(RichText::new("⋯").size(22.), |ui| {
                                for action in [
                                    Action::New,
                                    Action::Back,
                                    Action::Forward,
                                    Action::Open,
                                    Action::Folder,
                                    Action::Save,
                                    Action::SaveAs,
                                    Action::SaveCopy,
                                    Action::Find,
                                    Action::Read,
                                ] {
                                    self.menu_action(ui, action);
                                }
                                ui.separator();
                                for action in [
                                    Action::NewFolder,
                                    Action::NamedFile,
                                    Action::Rename,
                                    Action::Trash,
                                    Action::CopyPath,
                                    Action::CopyRelative,
                                    Action::Finder,
                                    Action::External,
                                ] {
                                    self.menu_action(ui, action);
                                }
                                ui.separator();
                                self.menu_action(ui, Action::Palette);
                                self.menu_action(ui, Action::Settings);
                                self.menu_action(ui, Action::Recovery);
                                if self.wait_session {
                                    self.menu_action(ui, Action::Finish);
                                    self.menu_action(ui, Action::Abort);
                                }
                            });
                        });
                    },
                );
            });
    }

    fn breadcrumbs(&mut self, ui: &mut egui::Ui) {
        let path = self.navigator.current.clone();
        let home = std::env::home_dir().unwrap_or_default();
        let root = if path.starts_with(&home) {
            home
        } else {
            PathBuf::from("/")
        };
        let mut parts: Vec<_> = path
            .ancestors()
            .take_while(|p| p.starts_with(&root))
            .map(Path::to_path_buf)
            .collect();
        parts.reverse();
        let count = ((ui.available_width() - 32.) / 100.).floor().max(1.) as usize;
        let visible = count.min(4).min(parts.len());
        let segment_max = ((ui.available_width() - 32. - visible.saturating_sub(1) as f32 * 18.)
            / visible.max(1) as f32)
            .max(24.);
        let start = parts.len().saturating_sub(visible);
        for (index, component) in parts.into_iter().skip(start).enumerate() {
            if index > 0 {
                ui.weak("›");
            }
            let label = if component == root && root == std::env::home_dir().unwrap_or_default() {
                "Home".into()
            } else {
                name(&component)
            };
            let width = (label.chars().count() as f32 * 7.5 + 14.).clamp(24., segment_max);
            let response = ui.add_sized(
                [width, 30.],
                egui::Button::new(label).frame(false).truncate(),
            );
            if response
                .on_hover_text(crate::paths::display(&component))
                .clicked()
            {
                self.navigator.navigate(component);
                self.native.focus_host();
                self.focus_editor = false;
            }
        }
    }

    fn close_search(&mut self) {
        self.search_open = false;
        self.temporary_scope = None;
        self.search_id = self.search.cancel();
        self.focus_editor = self.active.is_some();
    }

    fn path_bar(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui, view: &mut Presentation) {
        let complete = ui.memory(|m| m.has_focus(egui::Id::unique("path_input")))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        if self.path_focus {
            self.native.focus_host();
            self.navigator.focused = false;
            focus_field(
                ui.ctx(),
                egui::Id::unique("path_input"),
                self.path_input.chars().count(),
            );
            self.path_focus = false;
        }
        let response = ui.add_sized(
            [(ui.available_width() - 38.).max(40.), 28.],
            egui::TextEdit::singleline(&mut self.path_input)
                .id(egui::Id::unique("path_input"))
                .lock_focus(true)
                .hint_text("Folder or file path"),
        );
        if response.changed() {
            self.completion = None;
            self.completed_paths.clear();
            self.completions.retain(&[]);
        }
        if complete {
            response.request_focus();
            let expanded = if self.path_input.starts_with("~/") {
                std::env::home_dir().unwrap().join(&self.path_input[2..])
            } else {
                self.navigator.current.join(&self.path_input)
            };
            let (parent, prefix) = if self.path_input.ends_with('/') {
                (expanded, String::new())
            } else {
                (
                    expanded.parent().unwrap_or(Path::new("/")).to_path_buf(),
                    expanded
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_owned(),
                )
            };
            self.completions.retain(&[]);
            self.completed_paths.clear();
            match self.completions.request(parent.clone()) {
                Ok(id) => self.completion = Some((id, parent, prefix)),
                Err(e) => self.status = Some(e),
            }
        }
        if ui.small_button("Go").clicked()
            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            let path = if self.path_input.starts_with("~/") {
                std::env::home_dir().unwrap().join(&self.path_input[2..])
            } else {
                self.navigator.current.join(&self.path_input)
            };
            self.io.send(Job::Visit(path));
            view.path_open = false;
            self.completed_paths.clear();
            self.completion = None;
            self.completions.retain(&[]);
        }
        if !self.completed_paths.is_empty() {
            egui::Area::new(egui::Id::unique("path_completions"))
                .order(egui::Order::Foreground)
                .fixed_pos(response.rect.left_bottom())
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(
                            (ui.ctx().content_rect().right() - response.rect.left() - 36.)
                                .clamp(100., 360.),
                        );
                        egui::ScrollArea::vertical()
                            .max_height(280.)
                            .show(ui, |ui| {
                                for (path, _) in self.completed_paths.clone() {
                                    if ui
                                        .add(
                                            egui::Button::new((name(&path), egui::Atom::grow()))
                                                .truncate()
                                                .min_size(egui::vec2(ui.available_width(), 28.)),
                                        )
                                        .on_hover_text(crate::paths::display(&path))
                                        .clicked()
                                    {
                                        self.io.send(Job::Visit(path));
                                        view.path_open = false;
                                        self.completed_paths.clear();
                                        self.completion = None;
                                        self.completions.retain(&[]);
                                    }
                                }
                            });
                    });
                });
        }
    }

    fn navigator_pane(&mut self, ui: &mut egui::Ui, view: &mut Presentation, enabled: bool) {
        ui.horizontal(|ui| {
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
            ui.set_min_height(30.);
            ui.add_sized(
                [(ui.available_width() - 108.).max(40.), 30.],
                egui::Label::new(
                    RichText::new(name(&self.navigator.current))
                        .font(appearance::heading_font(14.)),
                )
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("★", |ui| {
                    if self.settings.pins.is_empty() {
                        ui.weak("No favorites yet");
                    }
                    for pin in self.settings.pins.clone() {
                        if ui
                            .button(name(&pin))
                            .on_hover_text(crate::paths::display(&pin))
                            .clicked()
                        {
                            self.io.send(Job::Visit(pin));
                            self.native.focus_host();
                            ui.close();
                        }
                    }
                    ui.separator();
                    self.menu_action(ui, Action::Pin);
                    if let Some(path) = self
                        .documents
                        .iter()
                        .find(|doc| Some(doc.id) == self.active)
                        .map(|doc| doc.disk.path.clone())
                        .filter(|path| !path.as_os_str().is_empty())
                    {
                        let pinned = self.settings.pins.contains(&path);
                        if ui
                            .button(if pinned {
                                "Unpin Active File"
                            } else {
                                "Pin Active File"
                            })
                            .clicked()
                        {
                            if pinned {
                                self.settings.pins.retain(|pin| pin != &path);
                            } else {
                                self.settings.pins.push(path);
                            }
                            self.io.send(Job::Settings(self.settings.clone()));
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Favorites");
                ui.menu_button("+", |ui| {
                    self.menu_action(ui, Action::NamedFile);
                    self.menu_action(ui, Action::NewFolder);
                });
                ui.menu_button(
                    RichText::new("⌄").color(appearance::palette(ui).secondary),
                    |ui| {
                        if ui
                            .selectable_label(view.filter_open, "Filter this folder")
                            .clicked()
                        {
                            view.filter_open = !view.filter_open;
                            ui.close();
                        }
                        for action in [Action::Home, Action::Parent, Action::Refresh, Action::Pin] {
                            self.menu_action(ui, action);
                        }
                        if ui.button("Volumes").clicked() {
                            self.navigator.navigate(PathBuf::from("/Volumes"));
                            ui.close();
                        }
                        if !self.settings.pins.is_empty() {
                            ui.separator();
                            ui.weak("Pinned");
                        }
                        for pin in self.settings.pins.clone() {
                            if ui
                                .button(name(&pin))
                                .on_hover_text(crate::paths::display(&pin))
                                .clicked()
                            {
                                self.io.send(Job::Visit(pin));
                                ui.close();
                            }
                        }
                        if !self.settings.location_visits.is_empty() {
                            ui.separator();
                            ui.weak("Frequent & recent");
                        }
                        for recent in self.settings.frequent_locations() {
                            if ui
                                .button(name(&recent))
                                .on_hover_text(crate::paths::display(&recent))
                                .clicked()
                            {
                                self.io.send(Job::Visit(recent));
                                ui.close();
                            }
                        }
                    },
                );
            });
        });
        if view.filter_open || !self.filter.is_empty() {
            let filter = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter this folder")
                    .desired_width(f32::INFINITY),
            );
            if filter.has_focus() {
                self.navigator.focused = false;
            }
            if !self.filter.is_empty() {
                ui.horizontal(|ui| {
                    ui.small("Filtered names");
                    if ui.small_button("Clear").clicked() {
                        self.filter.clear();
                    }
                });
            }
        }
        ui.add_space(4.);
        self.navigator.row_height = appearance::Metrics::for_density(if self.compact {
            appearance::Density::Compact
        } else {
            appearance::Density::Comfortable
        })
        .row_height;
        self.navigator.reduce_motion = self.settings.reduce_motion;
        self.navigator.filter = self.filter.clone();
        self.navigator.show_hidden = self.settings.show_hidden;
        let action = self.navigator.show(ui, enabled);
        if let Some(back) = action.back {
            self.command(if back { Action::Back } else { Action::Forward }, ui.ctx());
            self.native.focus_host();
        }
        if let Some((path, focus)) = action.open {
            self.open(path, focus);
            view.narrow_document = true;
        }
        if action.edit && self.active.is_some() {
            self.focus_editor = true;
            view.narrow_document = true;
            self.navigator.leave_for_editor();
        }
    }

    fn search_pane(&mut self, ui: &mut egui::Ui, view: &mut Presentation) {
        let scope = self
            .temporary_scope
            .clone()
            .or_else(|| self.scope.clone())
            .unwrap_or(self.navigator.current.clone());
        if !view.was_search {
            view.search_selected = None;
            view.search_results_focus = false;
        }
        let mut keyboard_hit = None;
        let mut scroll_selected = false;
        let down = view.search_down
            || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let up = view.search_up
            || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
        if (down || up) && !self.hits.is_empty() {
            let current = view.search_selected.as_ref().and_then(|(path, line)| {
                self.hits
                    .iter()
                    .position(|hit| hit.path == *path && hit.line == *line)
            });
            let index = match current {
                Some(index) if down => (index + 1).min(self.hits.len() - 1),
                Some(index) => index.saturating_sub(1),
                None => 0,
            };
            let hit = &self.hits[index];
            view.search_selected = Some((hit.path.clone(), hit.line));
            view.search_results_focus = true;
            scroll_selected = true;
        }
        if view.search_results_focus
            && (view.search_activate
                || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)))
        {
            keyboard_hit = self
                .hits
                .iter()
                .find(|hit| view.search_selected.as_ref() == Some(&(hit.path.clone(), hit.line)))
                .cloned();
        }
        ui.add_space(4.);

        ui.horizontal(|ui| {
            let changed = ui
                .selectable_value(&mut self.content_search, false, "Files")
                .changed()
                | ui.selectable_value(&mut self.content_search, true, "Text")
                    .changed();
            if changed {
                self.search_id = self.search.cancel();
                self.hits.clear();
                self.search_state = "Ready".into();
            }
        });
        if view.query_changed {
            view.search_selected = None;
            view.search_results_focus = false;
            self.search_id = self.search.cancel();
            self.hits.clear();
            self.search_state = "Ready · press Return to search".into();
        }
        ui.small(if self.temporary_scope.is_some() {
            "This folder · temporary scope"
        } else if self.scope.is_some() {
            "Pinned scope"
        } else {
            "Scope follows this folder"
        });
        let filters = search::Filters {
            ignored: self.settings.search_ignored,
            metadata: self.settings.search_metadata,
            exclude: self.settings.search_exclude.clone(),
            file_bytes: self.settings.search_file_mib * 1024 * 1024,
        };
        let indexed = self.search.index_status(&scope);
        let compatible_index = self.search.index_compatible(&scope, &filters);
        if !self.content_search {
            if let Some(status) = &indexed {
                ui.small(status);
                if !compatible_index {
                    ui.small("Filters changed · rebuild to use indexed queries");
                }
            }
            if view.query_changed
                && compatible_index
                && !self.query.is_empty()
                && let Ok(id) = self.search.start_filtered(
                    scope.clone(),
                    self.query.clone(),
                    false,
                    filters.clone(),
                )
            {
                self.search_id = id;
            }
        }
        ui.horizontal_wrapped(|ui| {
            let run = appearance::primary_button(ui, "Search").clicked()
                || view.query_submit
                || view.query_changed;
            if run && !self.query.is_empty() {
                match self.search.start_filtered(
                    scope.clone(),
                    self.query.clone(),
                    self.content_search,
                    search::Filters {
                        ignored: self.settings.search_ignored,
                        metadata: self.settings.search_metadata,
                        exclude: self.settings.search_exclude.clone(),
                        file_bytes: self.settings.search_file_mib * 1024 * 1024,
                    },
                ) {
                    Ok(id) => {
                        self.search_id = id;
                        self.hits.clear();
                        self.search_state = "Searching".into();
                    }
                    Err(e) => self.search_state = e,
                }
            }
            if self.search_state.starts_with("Searching") && ui.button("Stop").clicked() {
                self.search_id = self.search.cancel();
                self.search_state = "Cancelled".into();
            }
            ui.menu_button("Options", |ui| {
                if !self.content_search {
                    ui.weak("Optional · selected folder only");
                    if ui
                        .small_button(if indexed.is_some() {
                            "Rebuild Filename Index"
                        } else {
                            "Index This Folder"
                        })
                        .clicked()
                        && let Ok(id) = self.search.start_filtered(
                            scope.clone(),
                            String::new(),
                            false,
                            filters.clone(),
                        )
                    {
                        self.search_id = id;
                        self.hits.clear();
                        self.search_state = "Building filename index…".into();
                    }
                    if indexed.is_some() && ui.small_button("Remove Index").clicked() {
                        self.search.clear_index();
                        self.hits.clear();
                        self.search_state = "Index removed".into();
                    }
                    ui.separator();
                }

                ui.label(crate::paths::display(&scope));
                if ui
                    .button(if self.scope.is_some() {
                        "Unpin Search Scope"
                    } else {
                        "Pin Search Scope Here"
                    })
                    .clicked()
                {
                    self.command(Action::Scope, ui.ctx());
                    self.search_state = "Ready · scope changed".into();
                    ui.close();
                }
                if ui.button("Search This Folder").clicked() {
                    self.temporary_scope = Some(self.navigator.current.clone());
                    self.search_id = self.search.cancel();
                    self.hits.clear();
                    self.search_state = "Ready · scope changed".into();
                    ui.close();
                }
                if ui.button("Go to Search Folder").clicked() {
                    self.navigator.navigate(scope.clone());
                    ui.close();
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "{} · {} {} · {}",
                if self.content_search {
                    "Saved files"
                } else {
                    "Filenames"
                },
                self.hits.len(),
                if self.content_search {
                    "matches"
                } else {
                    "results"
                },
                self.search_state
            ))
            .color(appearance::palette(ui).secondary),
        );
        ui.collapsing("Coverage and exclusions",|ui|{ui.small(format!("Saved disk contents. Ignored paths: {}. Version-control metadata: {}. No directory symlinks or nested mounts. Up to 100,000 entries, 1,000 hits and {} MiB per content file. {}",if self.settings.search_ignored{"included"}else{"excluded"},if self.settings.search_metadata{"included"}else{"excluded"},self.settings.search_file_mib,self.settings.search_exclude));});
        ui.separator();
        let mut hit = keyboard_hit;
        let result_height = if self.content_search { 72. } else { 48. };
        let mut results = egui::ScrollArea::vertical();
        if scroll_selected
            && let Some(index) = self.hits.iter().position(|hit| {
                view.search_selected.as_ref() == Some(&(hit.path.clone(), hit.line))
            })
        {
            results = results.vertical_scroll_offset(
                index as f32 * (result_height + ui.spacing().item_spacing.y),
            );
        }
        results
            .id_salt("search_results")
            .auto_shrink([false, false])
            .show_rows(ui, result_height, self.hits.len(), |ui, range| {
                for i in range {
                    let item = &self.hits[i];
                    ui.push_id(i, |ui| {
                        if ui
                            .add(
                                egui::Button::selectable(
                                    view.search_selected.as_ref()
                                        == Some(&(item.path.clone(), item.line)),
                                    (
                                        egui::Atom::paint(egui::vec2(18., 18.), |ui, args| {
                                            appearance::file_symbol(
                                                ui, args.rect, &item.path, false,
                                            )
                                        }),
                                        RichText::new(name(&item.path))
                                            .font(appearance::heading_font(14.)),
                                        egui::Atom::grow(),
                                        RichText::new(if self.content_search {
                                            format!("line {}", item.line)
                                        } else {
                                            String::new()
                                        })
                                        .small()
                                        .color(appearance::palette(ui).secondary),
                                    ),
                                )
                                .frame(false)
                                .truncate()
                                .min_size(egui::vec2(ui.available_width(), 22.)),
                            )
                            .on_hover_text(format!(
                                "{}:{}\n{}",
                                crate::paths::display(&item.path),
                                item.line,
                                item.excerpt
                            ))
                            .clicked()
                        {
                            hit = Some(item.clone());
                        }
                        let relative = item.path.strip_prefix(&scope).unwrap_or(&item.path);
                        ui.add(
                            egui::Label::new(
                                RichText::new(crate::paths::display(
                                    relative.parent().unwrap_or(relative),
                                ))
                                .small()
                                .color(appearance::palette(ui).secondary),
                            )
                            .truncate(),
                        );
                        if self.content_search {
                            let mut job = egui::text::LayoutJob::default();
                            let colors = appearance::palette(ui);
                            let normal = egui::TextFormat {
                                font_id: egui::FontId::monospace(12.),
                                color: colors.secondary,
                                ..Default::default()
                            };
                            if let Some(start) = item.excerpt.find(&item.query) {
                                let end = start + item.query.len();
                                job.append(&item.excerpt[..start], 0., normal.clone());
                                let mut matched = normal.clone();
                                (matched.background, matched.color) =
                                    appearance::search_match_colors(ui.visuals().dark_mode);
                                job.append(&item.excerpt[start..end], 0., matched);
                                job.append(&item.excerpt[end..], 0., normal);
                            } else {
                                job.append(&item.excerpt, 0., normal);
                            }
                            ui.add(egui::Label::new(job).truncate());
                        }
                    });
                }
            });
        if let Some(hit) = hit {
            let path = hit.path.clone();
            self.open_latency = Some((path.clone(), std::time::Instant::now()));
            self.pending_hit = Some(hit);
            self.open_token += 1;
            self.opening.insert(path.clone(), (true, self.open_token));
            self.io.send(Job::Open(path, self.open_token, false));
            self.search_open = false;
            self.temporary_scope = None;
            view.narrow_document = true;
        }
    }

    fn settings_dialog(&mut self, ctx: &egui::Context, view: &mut Presentation) {
        if !self.settings_open {
            view.settings_draft = None;
            view.settings_error = None;
            return;
        }
        let draft = view
            .settings_draft
            .get_or_insert_with(|| self.settings.clone());
        let width = (ctx.content_rect().width() - 48.).clamp(160., 1090.);
        let mut apply = false;
        let mut close = false;
        let mut reload = false;
        egui::Modal::new(egui::Id::unique("settings")).show(ctx,|ui|{
            ui.style_mut().text_styles.insert(egui::TextStyle::Body,egui::FontId::proportional(16.));
            ui.style_mut().text_styles.insert(egui::TextStyle::Button,egui::FontId::proportional(16.));
            ui.set_width(width);
            let categories=["General","Appearance","Editor","Search","Shortcuts","Recovery"];
            let wide = width >= 660. && ctx.content_rect().height() >= 600.;
            if !wide { egui::ComboBox::from_id_salt("settings_category").selected_text(categories[view.settings_page]).show_ui(ui,|ui| { for (index,title) in categories.iter().enumerate(){if ui.selectable_value(&mut view.settings_page,index,*title).clicked(){ui.close();}} }); }
            ui.allocate_ui_with_layout(egui::vec2(width, (ctx.content_rect().height()-160.).clamp(120.,620.)), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            if wide {
                egui::Frame::new().fill(appearance::palette(ui).sidebar).inner_margin(16.).corner_radius(9.).show(ui,|ui| {
                    ui.vertical(|ui| {
                    ui.set_min_height(380.);
                    ui.set_width(244.); ui.horizontal(|ui| {let (r,_) = ui.allocate_exact_size(egui::vec2(24.,24.),egui::Sense::hover());appearance::symbol(ui,r,Symbol::Brand,appearance::palette(ui).secondary);ui.vertical(|ui|{ui.strong("Glimpse");ui.weak("Settings");});});ui.add_space(20.);
                    for (index,title) in categories.iter().enumerate() { ui.add_sized([244.,44.],egui::Button::selectable(view.settings_page==index,(egui::Atom::paint(egui::vec2(20.,20.),|ui,args|appearance::symbol(ui,args.rect,match index{0=>Symbol::Sliders,1=>Symbol::Brand,2=>Symbol::File,3=>Symbol::Search,4=>Symbol::Sidebar,_=>Symbol::Clock},appearance::palette(ui).secondary)),RichText::new(*title).size(17.),egui::Atom::grow())).stroke(egui::Stroke::NONE)).clicked().then(|| view.settings_page=index); }
                    });
                });
            }
            ui.vertical(|ui| {
            let content_width = (width-if wide {300.} else {16.}).max(120.);
            ui.set_width(content_width);
            appearance::section(ui,categories[view.settings_page],match view.settings_page {1=>"Theme, typography and the space around your work.",2=>"Source text and editing behavior.",3=>"Choose what saved-file search includes.",_=>"Preferences for your work."});
            egui::Frame::new().stroke(egui::Stroke::new(1.,appearance::palette(ui).separator)).corner_radius(9.).inner_margin(16.).show(ui,|ui| {
            egui::ScrollArea::vertical().id_salt(("settings_page",view.settings_page)).max_height((ctx.content_rect().height()-330.).max(100.)).show(ui,|ui|{
                ui.set_min_width((content_width-40.).max(80.));
                ui.spacing_mut().slider_width=(width-160.).clamp(48.,200.);
                ui.add_space(8.);
                match view.settings_page{
                    0=>{
                        ui.strong("Browsing");
                        preference_toggle(ui, "Show hidden files", &mut draft.show_hidden);
                        preference_toggle(ui, "Remember recent locations", &mut draft.remember_locations);
                        if ui.button("Clear Recent Locations").clicked(){draft.recents.clear();draft.location_visits.clear();}
                        ui.add_space(12.);ui.strong("Favorites");
                        if draft.pins.is_empty(){ui.weak("Pin a file or folder from the star menu to keep it here.");}
                        for pin in draft.pins.clone(){ui.horizontal(|ui|{ui.add(egui::Label::new(crate::paths::display(&pin)).truncate());if ui.small_button("Remove").clicked(){draft.pins.retain(|p|p!=&pin);}});}
                    }
                    1=>{
                        ui.strong("Theme");
                        let mut theme=if draft.system_theme{0}else if draft.dark{2}else{1};
                        ui.horizontal_wrapped(|ui|{for(id,title)in[(1,"Light"),(2,"Dark"),(0,"System")]{ui.vertical(|ui|{if theme_tile(ui,title,id,theme==id,((content_width-64.)/3.).max(62.)){theme=id;}});}});
                        draft.system_theme=theme==0;if theme!=0{draft.dark=theme==2;}
                        ui.add_space(12.);ui.separator();ui.add_space(12.);ui.strong("Layout and reading");ui.horizontal(|ui|{ui.selectable_value(&mut draft.compact,false,"Comfortable");ui.selectable_value(&mut draft.compact,true,"Compact");});
                        ui.add_space(12.);
                        preference_toggle(ui, "Reduce motion", &mut draft.reduce_motion);
                        preference_row(ui,"Interface scale",|ui|{ui.add(egui::Slider::new(&mut draft.ui_scale,0.8..=2.));});
                        preference_row(ui,"Reading text size",|ui|{ui.add(egui::Slider::new(&mut draft.markdown_font_size,12.0..=28.));});
                        ui.add_space(16.);ui.separator();ui.add_space(12.);ui.label(RichText::new("Editor").font(appearance::heading_font(18.)));ui.weak("Source text appearance and everyday editing.");
                        preference_row(ui,"Font size",|ui|{ui.add(egui::Slider::new(&mut draft.font_size,10.0..=32.));});
                        preference_row(ui,"Line height",|ui|{ui.add(egui::Slider::new(&mut draft.line_height,1.2..=2.).fixed_decimals(2));});
                        preference_toggle(ui,"Show line numbers",&mut draft.line_numbers);preference_toggle(ui,"Wrap long lines",&mut draft.wrap);preference_toggle(ui,"Highlight current line",&mut draft.current_line);preference_toggle(ui,"Syntax colors",&mut draft.syntax);
                    }
                    2=>{
                        ui.strong("Source typography");
                        ui.label("Font name");ui.add(egui::TextEdit::singleline(&mut draft.font_family).hint_text("System font (or installed PostScript name)").desired_width(f32::INFINITY));
                        preference_row(ui,"Font size",|ui|{ui.add(egui::Slider::new(&mut draft.font_size,10.0..=32.));});
                        preference_row(ui,"Line height",|ui|{ui.add(egui::Slider::new(&mut draft.line_height,1.2..=2.).fixed_decimals(2));});
                        preference_toggle(ui,"Proportional system font",&mut draft.proportional);
                        ui.add_space(12.);ui.strong("Editing");
                        preference_toggle(ui, "Wrap long lines", &mut draft.wrap);
                        preference_toggle(ui, "Show line numbers", &mut draft.line_numbers);
                        preference_toggle(ui, "Highlight current line", &mut draft.current_line);
                        preference_toggle(ui, "Syntax colors", &mut draft.syntax);
                        ui.add_space(12.); ui.separator(); ui.add_space(12.);
                        ui.strong("Indentation");
                        preference_toggle(ui, "Continue indentation on Return", &mut draft.auto_indent);
                        preference_toggle(ui, "Indent using tab characters", &mut draft.tabs);
                        preference_row(ui,"Indent width",|ui|{ui.add(egui::Slider::new(&mut draft.indent,1..=8));});
                        if ui.button("Reset Editor Settings").clicked(){let defaults=storage::Settings::default();draft.font_family=defaults.font_family;draft.font_size=defaults.font_size;draft.line_height=defaults.line_height;draft.proportional=defaults.proportional;draft.wrap=defaults.wrap;draft.line_numbers=defaults.line_numbers;draft.current_line=defaults.current_line;draft.syntax=defaults.syntax;draft.auto_indent=defaults.auto_indent;draft.tabs=defaults.tabs;draft.indent=defaults.indent;}
                    }
                    3=>{
                        ui.strong("Saved-file coverage");
                        preference_toggle(ui, "Include ignored paths", &mut draft.search_ignored);
                        preference_toggle(ui, "Include version-control metadata", &mut draft.search_metadata);ui.add(egui::Slider::new(&mut draft.search_file_mib,1..=10).text("Content file limit (MiB)"));
                        ui.add_space(12.);ui.strong("Additional exclusions");ui.weak("One ignore-style pattern per line.");ui.add(egui::TextEdit::multiline(&mut draft.search_exclude).desired_rows(5).desired_width(f32::INFINITY));
                    }
                    4=>{
                        ui.weak("Cmd, Shift, Alt, Ctrl + key. Empty disables a command. Clipboard and Undo remain reserved.");
                        ui.add(egui::TextEdit::singleline(&mut view.shortcut_filter).hint_text("Filter commands").desired_width(f32::INFINITY));
                        let filter=view.shortcut_filter.to_lowercase();
                        let filtered: Vec<_> = commands::COMMANDS.iter().filter(|c| c.label.to_lowercase().contains(&filter)).collect();
                        let stacked = ui.available_width() < 360.;
                        egui::ScrollArea::vertical().id_salt("shortcut_rows").max_height((ctx.content_rect().height()-460.).max(100.)).show_rows(ui, if stacked { 64. } else { 36. }, filtered.len(), |ui, rows| {
                            for index in rows {
                                let command = filtered[index];
                                ui.push_id(command.id, |ui| {
                                    let key = draft.keymaps.entry(command.id.into()).or_insert_with(||command.shortcut.into());
                                    if stacked {
                                        let label = ui.label(command.label);
                                        ui.add(egui::TextEdit::singleline(key).desired_width(f32::INFINITY)).labelled_by(label.id);
                                    } else {
                                        ui.horizontal(|ui| {
                                            let label_width=(ui.available_width()-194.).max(80.);
                                            let label = ui.allocate_ui_with_layout(egui::vec2(label_width,30.),egui::Layout::left_to_right(egui::Align::Center),|ui|{ui.set_min_width(label_width);ui.add(egui::Label::new(command.label).truncate())}).inner;
                                            ui.add(egui::TextEdit::singleline(key).desired_width(180.)).labelled_by(label.id);
                                        });
                                    }
                                });
                            }
                        });
                        if ui.button("Reset Shortcuts").clicked(){draft.keymaps.clear();}
                    }
                    _=>{
                        ui.label("Unresolved drafts remain until you resolve them. Saved versions are retained for seven days within 128 MiB.");
                        if ui.button("Recovery and Saved Versions…").clicked(){self.recovery_open=true;self.io.send(Job::Inventory);close=true;}
                    }
                }
                ui.add_space(12.);
            });
            });
            });
            });
            if let Some(error)=&view.settings_error{ui.colored_label(appearance::palette(ui).error,error);}
            ui.separator();
            ui.horizontal_wrapped(|ui|{
                close|=ui.button("Close").clicked();
                apply=appearance::primary_button(ui,"Apply Settings").clicked();
                reload=ui.button("Reload from Disk").clicked();
            });
        });
        if apply {
            match draft.validate().and_then(|_|if native_editor::valid_font(&draft.font_family){Ok(())}else{Err("This font is unavailable. Enter an installed PostScript name or leave it empty.".into())}){
                Ok(())=>{
                    draft.warning=None;self.settings=draft.clone();self.compact=self.settings.compact;
                    self.bindings=commands::bindings(&self.settings.keymaps).expect("Settings validated");
                    self.io.send(Job::Settings(self.settings.clone()));
                    appearance::install(ctx,if self.compact{appearance::Density::Compact}else{appearance::Density::Comfortable});ctx.set_zoom_factor(self.settings.ui_scale);
                    ctx.set_theme(if self.settings.system_theme{egui::ThemePreference::System}else if self.settings.dark{egui::ThemePreference::Dark}else{egui::ThemePreference::Light});
                    view.settings_error=None;
                },Err(e)=>view.settings_error=Some(e)
            }
        }
        if reload {
            self.io.send(Job::LoadSettings);
            close = true;
        }
        if close {
            self.settings_open = false;
            view.settings_draft = None;
            view.settings_error = None;
            self.focus_editor = true;
        }
    }

    fn palette_dialog(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        let mut action = None;
        egui::Modal::new(egui::Id::unique("command_palette")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(160., 560.));
            ui.heading("Commands");
            if self.palette_focus && ui.is_visible() {
                self.native.focus_host();
                focus_field(
                    ctx,
                    egui::Id::unique("command_query"),
                    self.palette_query.chars().count(),
                );
            }
            let query = ui.add(
                egui::TextEdit::singleline(&mut self.palette_query)
                    .id(egui::Id::unique("command_query"))
                    .hint_text("Search commands")
                    .desired_width(f32::INFINITY),
            );
            if self.palette_focus && ui.is_visible() {
                query.request_focus();
                self.palette_focus = false;
                ctx.request_repaint();
            }
            let needle = self.palette_query.to_lowercase();
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 220.).max(80.))
                .show(ui, |ui| {
                    let mut first = true;
                    for command in commands::COMMANDS.iter().filter(|c| {
                        c.action != Action::Choose && c.label.to_lowercase().contains(&needle)
                    }) {
                        let key = self
                            .settings
                            .keymaps
                            .get(command.id)
                            .map(String::as_str)
                            .unwrap_or(command.shortcut);
                        if ui
                            .add(
                                egui::Button::new((command.label, egui::Atom::grow()))
                                    .shortcut_text(key)
                                    .frame(false)
                                    .min_size(egui::vec2(ui.available_width(), 28.)),
                            )
                            .clicked()
                            || (first
                                && (query.has_focus() || query.lost_focus())
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            action = Some(command.action);
                        }
                        first = false;
                    }
                    if first {
                        ui.weak("No matching commands");
                    }
                });
            if ui.button("Close").clicked() {
                self.palette_open = false;
                self.focus_editor = true;
            }
        });
        if let Some(action) = action {
            ctx.input_mut(|input| {
                input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            });
            self.palette_open = false;
            self.command(action, ctx);
        }
    }

    fn find_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui, view: &mut Presentation) {
        let Some(doc) = self
            .documents
            .iter_mut()
            .find(|d| Some(d.id) == self.active)
        else {
            self.find_open = false;
            return;
        };
        let mut previous = false;
        let mut next = false;
        let mut replace = false;
        let mut all = false;
        let mut go = false;
        let mut focus_line = false;
        let mut changed = false;
        egui::CentralPanel::default()
            .frame(
                appearance::toolbar_frame(ui)
                    .stroke(egui::Stroke::new(1., appearance::palette(ui).separator))
                    .inner_margin(egui::Margin::symmetric(8, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if icon_button(
                        ui,
                        if view.replace_open {
                            Symbol::ChevronDown
                        } else {
                            Symbol::Forward
                        },
                        "Show Replace",
                        view.replace_open,
                    )
                    .clicked()
                    {
                        view.replace_open = !view.replace_open;
                    }
                    if self.find_focus {
                        focus_field(ctx, egui::Id::unique("find_input"), self.find.chars().count());
                        self.find_focus = false;
                    }
                    let response = ui.add_sized(
                        [(ui.available_width() - 104.).max(70.), 26.],
                        egui::TextEdit::singleline(&mut self.find)
                            .hint_text("Find in document")
                            .id(egui::Id::unique("find_input")),
                    );
                    changed |= response.changed();
                    if (response.has_focus() || response.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        previous = ui.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if modifiers.shift)));
                        next = !previous;
                        response.request_focus();
                    }
                    previous |=
                        icon_button(ui, Symbol::Back, "Previous match · Shift-Return", false)
                            .clicked();
                    next |=
                        icon_button(ui, Symbol::Forward, "Next match · Return", false).clicked();
                    if icon_button(ui, Symbol::Close, "Close Find", false).clicked() {
                        self.find_open = false;
                        self.focus_editor = true;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    changed |= ui.checkbox(&mut self.find_case, "Match case").changed();
                    changed |= ui.checkbox(&mut self.find_word, "Whole word").changed();
                    if ui.selectable_label(view.go_line.is_some(), "Go to Line…").clicked() {
                        if view.go_line.is_some() {
                            view.go_line = None;
                        } else {
                            view.go_line = Some(self.line.to_string());
                            focus_line = true;
                        }
                    }
                    if let Some(cache) = &view.find_cache {
                        let selection = self.native.selection(doc.id).map(|r| {
                            editor::utf16_to_byte(&doc.text, r.location)..editor::utf16_to_byte(&doc.text, r.location + r.length)
                        }).unwrap_or(0..0);
                        let current = cache.ranges.iter().position(|range| *range == selection);
                        let label = if cache.ranges.is_empty() {
                            "No matches".into()
                        } else if let Some(index) = current {
                            format!("{} of {}{}", index + 1, cache.ranges.len(), if cache.truncated { "+" } else { "" })
                        } else {
                            format!("{}{} matches", cache.ranges.len(), if cache.truncated { "+" } else { "" })
                        };
                        ui.weak(label);
                    }
                });
                if let Some(input) = &mut view.go_line {
                    ui.horizontal_wrapped(|ui| {
                        let label = ui.label("Line");
                        let id = egui::Id::unique("go_line_input");
                        if focus_line {
                            self.native.focus_host();
                            focus_field(ctx, id, input.chars().count());
                        }
                        let response = ui.add_sized(
                            [100., 26.],
                            egui::TextEdit::singleline(input).id(id),
                        ).labelled_by(label.id);
                        if focus_line {
                            response.request_focus();
                            ctx.request_repaint();
                        }
                        let line = input.parse::<usize>().ok().filter(|line| *line > 0);
                        let enter = !focus_line && (response.has_focus() || response.lost_focus())
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (ui.add_enabled(line.is_some(), egui::Button::new("Go")).clicked() || enter)
                            && let Some(line) = line
                        {
                            self.line = line;
                            go = true;
                        }
                        if line.is_none() {
                            ui.weak("Enter a line number greater than zero.");
                        }
                    });
                }
                if view.replace_open {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_sized(
                            [(ui.available_width() - 190.).max(70.), 26.],
                            egui::TextEdit::singleline(&mut self.replacement)
                                .hint_text("Replace with"),
                        );
                        let editable = !self.find.is_empty() && !self.native.is_locked(doc.id);
                        replace = ui
                            .add_enabled(editable, egui::Button::new("Replace"))
                            .clicked();
                        all = ui
                            .add_enabled(editable, egui::Button::new("Replace All"))
                            .clicked();
                    });
                }
                if go {
                    self.find_open = false;
                    view.go_line = None;
                    self.goto = Some((self.line, 1));
                    self.goto_path = Some(doc.disk.path.clone());
                    self.focus_editor = true;
                }
                if self.find.is_empty() {
                    view.find_cache = None;
                    return;
                }
                let key = (
                    doc.id,
                    doc.generation,
                    self.find.clone(),
                    self.find_case,
                    self.find_word,
                );
                let refresh = view
                    .find_cache
                    .as_ref()
                    .is_none_or(|cache| cache.key != key);
                if refresh {
                    match editor::matcher(&self.find, self.find_case, self.find_word) {
                        Ok(regex) => {
                            let (ranges, truncated) =
                                editor::match_ranges(&regex, &doc.text, 10_000);
                            ctx.request_repaint();
                            view.find_cache = Some(FindCache {
                                key,
                                regex,
                                ranges,
                                truncated,
                            });
                        }
                        Err(error) => {
                            view.find_cache = None;
                            ui.colored_label(appearance::palette(ui).error, error);
                            return;
                        }
                    }
                }
                let cache = view.find_cache.as_ref().unwrap();
                let selection = self
                    .native
                    .selection(doc.id)
                    .map(|r| {
                        editor::utf16_to_byte(&doc.text, r.location)
                            ..editor::utf16_to_byte(&doc.text, r.location + r.length)
                    })
                    .unwrap_or(0..0);
                let regex = &cache.regex;
                let mut selected = None;
                let generation = doc.generation;
                if all {
                    let updated = regex
                        .replace_all(&doc.text, regex::NoExpand(&self.replacement))
                        .into_owned();
                    if updated != doc.text {
                        editor::replace(
                            ctx,
                            egui::Id::unique(("document", doc.id)),
                            &doc.text,
                            &updated,
                        );
                        doc.text = updated;
                        doc.generation += 1;
                        doc.preview = false;
                    }
                } else if replace
                    && regex
                        .find(&doc.text[selection.clone()])
                        .is_some_and(|m| m.range() == (0..selection.len()))
                {
                    let old = doc.text.clone();
                    doc.text.replace_range(selection.clone(), &self.replacement);
                    editor::replace(ctx, egui::Id::unique(("document", doc.id)), &old, &doc.text);
                    doc.generation += 1;
                    doc.preview = false;
                    selected = editor::next_match(
                        regex,
                        &doc.text,
                        selection.start + self.replacement.len(),
                        false,
                    );
                } else if next || previous || replace || changed || refresh {
                    selected = editor::next_match(
                        regex,
                        &doc.text,
                        if previous || changed || refresh {
                            selection.start
                        } else {
                            selection.end
                        },
                        previous,
                    );
                }
                if let Some(range) = selected {
                    self.native.select(
                        doc.id,
                        editor::byte_to_utf16(&doc.text, range.start),
                        editor::byte_to_utf16(&doc.text, range.end),
                    );
                    ctx.request_repaint();
                }
                if generation != doc.generation {
                    self.io.send(Job::Recover(
                        doc.id,
                        storage::Draft {
                            path: doc.disk.path.clone(),
                            text: doc.text.clone(),
                            base: doc.disk.bytes.clone(),
                            target: Some(doc.disk.target.clone()),
                            generation: doc.generation,
                        },
                    ));
                    view.find_cache = None;
                    ctx.request_repaint();
                }
            });
    }

    fn floating_panels(
        &mut self,
        ctx: &egui::Context,
        frame: &eframe::Frame,
        view: &mut Presentation,
    ) {
        let root = ctx
            .input(|i| i.viewport().inner_rect)
            .unwrap_or(ctx.content_rect());
        if self.search_open && !self.overlay_open(view) {
            let width = 440_f32.min(root.width() - 24.);
            let anchor = view.search_anchor.unwrap_or(egui::Rect::ZERO);
            let pos = egui::pos2(
                (root.left() + anchor.right() - width).max(root.left() + 8.),
                root.top() + anchor.bottom() + 4.,
            );
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("file_search_popup"),
                egui::ViewportBuilder::default()
                    .with_title("Search files")
                    .with_decorations(false)
                    .with_taskbar(false)
                    .with_visible(false)
                    .with_active(false)
                    .with_position(pos)
                    .with_inner_size([width, 420.])
                    .with_resizable(false),
                |ctx, _| {
                    match crate::native_accessibility::register(ctx, frame) {
                        Ok(true) => {}
                        Ok(false) => {}
                        Err(error) => {
                            self.status = Some(error);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        }
                    }
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(appearance::Palette::for_theme(ctx.theme()).raised)
                                .stroke(egui::Stroke::new(
                                    1.,
                                    appearance::Palette::for_theme(ctx.theme()).separator,
                                ))
                                .inner_margin(10.),
                        )
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.weak(format!("Search in {}", name(&self.last_scope)));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_button(
                                            ui,
                                            Symbol::Close,
                                            "Close search · Esc",
                                            false,
                                        )
                                        .clicked()
                                        {
                                            self.close_search();
                                        }
                                    },
                                );
                            });
                            self.search_pane(ui, view);
                        });
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.close_search();
                    }
                },
            );
        }
        if self.find_open && self.active.is_some() && !self.overlay_open(view) {
            let width = 460_f32.min(root.width() - 24.);
            let height = 108.
                + if view.replace_open { 36. } else { 0. }
                + if view.go_line.is_some() { 40. } else { 0. };
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("document_find_popup"),
                egui::ViewportBuilder::default()
                    .with_title("Find in document")
                    .with_decorations(false)
                    .with_taskbar(false)
                    .with_visible(false)
                    .with_active(false)
                    .with_position(egui::pos2(root.right() - width - 12., root.top() + 92.))
                    .with_inner_size([width, height])
                    .with_resizable(false),
                |ctx, _| {
                    match crate::native_accessibility::register(ctx, frame) {
                        Ok(true) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
                        Ok(false) => {}
                        Err(error) => {
                            self.status = Some(error);
                            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        }
                    }
                    let find_ctx = ctx.clone();
                    egui::CentralPanel::default()
                        .show(ctx, |ui| self.find_bar(&find_ctx, ui, view));
                    if ctx.input(|i| {
                        i.viewport().close_requested() || i.key_pressed(egui::Key::Escape)
                    }) {
                        self.find_open = false;
                        self.focus_editor = true;
                    }
                },
            );
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context, view: &mut Presentation) {
        self.palette_dialog(ctx);
        if self.recovery_open {
            egui::Modal::new(egui::Id::unique("recovery_inventory")).show(ctx,|ui|{
                ui.set_width((ctx.content_rect().width()-64.).clamp(160.,640.));
                appearance::section(ui,"Recovery and saved versions","Restore a draft or inspect a retained version.");
                let total:u64=self.recovered.iter().map(|r|r.bytes).sum();
                ui.weak(format!("{} records · {:.2} MiB shown",self.recovered.len(),total as f64/1048576.));
                ui.small(format!("Total storage: {:.2} MiB in {} files · includes live sessions and save snapshots",self.recovered_storage.bytes as f64/1048576.,self.recovered_storage.files));
                if self.recovered_storage.partial {ui.colored_label(appearance::palette(ui).warning,"Storage count is partial; some entries could not be counted or the scan limit was reached.");}
                if self.recovered_truncated {ui.colored_label(appearance::palette(ui).warning,"Showing the first 256 records. Additional records remain in storage.");}
                ui.collapsing("Retention",|ui|{ui.small("Up to 256 records are shown. Saved versions are retained for seven days within 128 MiB. Unresolved drafts do not expire.");});
                let row_height = if ui.available_width() < 400. { 200. } else if ui.available_width() < 540. { 152. } else { 112. };
                if self.recovered.is_empty(){ui.label("No recovery records to show.");}
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-280.).max(100.)).show_rows(ui,row_height,self.recovered.len(),|ui,rows|{
                    for record in self.recovered[rows].iter().cloned(){ui.push_id((&record.path,record.generation),|ui|{
                        ui.set_min_height(row_height);
                        ui.add_space(8.);ui.add(egui::Label::new(RichText::new(crate::name(&record.original)).strong()).truncate());
                        ui.add(egui::Label::new(egui::RichText::new(crate::paths::display(&record.original)).small().weak()).truncate()).on_hover_text(crate::paths::display(&record.original));
                        ui.small(format!("{} · {} bytes",if record.history{"Saved version"}else if record.candidate{"Save snapshot"}else{"Recovered draft"},record.bytes));
                        ui.horizontal_wrapped(|ui|{
                            if ui.button("Restore").clicked(){
                                if self.documents.iter().any(|d|d.disk.path==record.original&&d.dirty()){self.status=Some("An open dirty document already has this path. Close or save it before restoring.".into());}
                                else{let id=self.next_id;self.next_id+=1;self.io.send(if record.history||record.candidate{Job::RestoreHistory(id,record.path.clone())}else{Job::Import(id,record.path.clone())});self.recovery_open=false;self.focus_editor=true;}
                            }
                            if (record.history||record.candidate) && ui.button("Open Observed Disk Version").clicked(){let id=self.next_id;self.next_id+=1;self.io.send(Job::RestoreObserved(id,record.path.clone()));self.recovery_open=false;self.focus_editor=true;}
                            if ui.button("Move Record to Trash…").clicked(){view.record_to_trash=Some(record.path.clone());}
                        });ui.separator();
                    });}
                });
                if let Some(message)=&self.status{ui.colored_label(appearance::palette(ui).warning,message);}
                ui.horizontal_wrapped(|ui|{
                    if ui.button("Close").clicked(){self.recovery_open=false;self.focus_editor=true;}
                    if ui.button("Refresh Inventory").clicked(){self.io.send(Job::Inventory);}
                });
            });
        }
        if let Some(path) = view.record_to_trash.clone() {
            egui::Modal::new(egui::Id::unique("trash_recovery_record")).show(ctx,|ui|{
                ui.set_width((ctx.content_rect().width()-64.).clamp(160.,440.));
                ui.heading("Move this recovery record to Trash?");
                ui.label("The original file is unchanged. This record will no longer be available in Glimpse recovery.");
                ui.horizontal_wrapped(|ui|{
                    if ui.button("Cancel").clicked(){view.record_to_trash=None;}
                    if ui.button("Move Record to Trash").clicked(){self.io.send(Job::TrashRecord(path));view.record_to_trash=None;}
                });
            });
        }
        if self.recovery_retry.is_some() {
            egui::Modal::new(egui::Id::unique("recovery_failure")).show(ctx,|ui|{
            ui.set_width((ctx.content_rect().width()-64.).clamp(160.,580.));
            ui.heading("Recovery could not be persisted");ui.label(self.status.as_deref().unwrap_or("Recovery unavailable"));
            ui.label("Normal Save is blocked. Save Without Recovery skips the local candidate and disk-version history for this attempt only. Destination validation still applies.");
            let id=match self.recovery_retry.as_ref().unwrap(){Job::Save{id,..}=>*id,Job::SaveTo(d)=>d.id,_=>unreachable!()};
            let conflicted=self.documents.iter().any(|d|d.id==id&&d.external.is_some());
            ui.horizontal_wrapped(|ui|{
                if ui.button("Retry").clicked() && let Some(job)=self.recovery_retry.take(){if let Some(doc)=self.documents.iter_mut().find(|d|d.id==id){doc.saving=true;}self.io.send(job);}
                if ui.add_enabled(!conflicted,egui::Button::new("Save Without Recovery — Once")).clicked() && let Some(mut job)=self.recovery_retry.take(){match &mut job{Job::Save{without_recovery,..}=>*without_recovery=true,Job::SaveTo(d)=>d.without_recovery=true,_=>unreachable!()}
if let Some(doc)=self.documents.iter_mut().find(|d|d.id==id){doc.saving=true;}self.io.send(job);}
                if ui.button("Save Copy Elsewhere Without Recovery…").clicked(){self.recovery_retry=None;self.active=Some(id);self.copy_without_recovery=true;self.save_to(true);}
                if ui.button("Cancel").clicked(){self.recovery_retry=None;self.focus_editor=true;}
            });
        });
        }
        self.settings_dialog(ctx, view);
        if let Some(operation) = self.operation {
            egui::Modal::new(egui::Id::unique("file_operation")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.).clamp(160., 520.));
                ui.heading(match operation {
                    0 => "New Folder",
                    1 => "New Named File",
                    2 => "Rename Selected",
                    _ => "Move Selected to OS Trash",
                });
                let selected = self.navigator.selection();
                ui.label(crate::paths::display(if operation < 2 {
                    &self.navigator.current
                } else {
                    selected.as_ref().unwrap_or(&self.navigator.current)
                }));
                let mut submit = false;
                if operation < 3 {
                    let label = ui.label(if operation == 0 {
                        "Folder name"
                    } else {
                        "File name"
                    });
                    let field = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.operation_name)
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id);
                    if view.operation_shown != Some(operation) {
                        select_field(ctx, &field, self.operation_name.chars().count());
                    }
                    submit = view.operation_shown == Some(operation)
                        && (field.has_focus() || field.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                }
                ui.horizontal(|ui| {
                    let valid = operation == 3
                        || (!self.operation_name.is_empty()
                            && self.operation_name != "."
                            && self.operation_name != ".."
                            && !self.operation_name.contains('/'));
                    if ui
                        .add_enabled(
                            valid,
                            egui::Button::new(if operation == 3 {
                                "Move to Trash"
                            } else if operation == 2 {
                                "Rename"
                            } else if operation == 0 {
                                "Create Folder"
                            } else {
                                "Create File"
                            }),
                        )
                        .clicked()
                        || (valid && submit)
                    {
                        match operation {
                            0 | 1 => self.io.send(Job::Create(
                                self.navigator.current.join(&self.operation_name),
                                operation == 0,
                                String::new(),
                            )),
                            2 => {
                                if let Some(source) = selected {
                                    if self.documents.iter().any(|d| d.disk.path == source) {
                                        self.status = Some(
                                            "Close the open document before renaming it.".into(),
                                        );
                                    } else {
                                        self.io.send(Job::Rename(
                                            source.clone(),
                                            source.with_file_name(&self.operation_name),
                                        ));
                                    }
                                }
                            }
                            _ => {
                                if let Some(path) = selected {
                                    if self.documents.iter().any(|d| d.disk.path == path) {
                                        self.status = Some(
                                            "Close the open document before moving it to Trash."
                                                .into(),
                                        );
                                    } else {
                                        self.io.send(Job::Trash(path));
                                    }
                                }
                            }
                        }
                        self.operation = None;
                        self.focus_editor = self.active.is_some();
                    }
                    if ui.button("Cancel").clicked() {
                        self.operation = None;
                        self.focus_editor = self.active.is_some();
                    }
                });
            });
        }
    }

    fn overlay_open(&self, view: &Presentation) -> bool {
        self.settings_open
            || self.palette_open
            || self.recovery_open
            || self.compare_open
            || self.pending.is_some()
            || self.operation.is_some()
            || self.recovery_retry.is_some()
            || self.replace_destination.is_some()
            || self.abandon_destination.is_some()
            || view.decision.is_some()
            || view.record_to_trash.is_some()
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let mut activate = None;
        let mut close = None;
        let mut pin = None;
        egui::Panel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(appearance::palette(ui).chrome)
                    .inner_margin(egui::Margin::symmetric(0, 0)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("tabs_scroll")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_min_height(
                                appearance::Metrics::for_density(if self.compact {
                                    appearance::Density::Compact
                                } else {
                                    appearance::Density::Comfortable
                                })
                                .toolbar_height,
                            );
                            for doc in &self.documents {
                                let selected = Some(doc.id) == self.active;
                                let basename = name(&doc.disk.path);
                                let duplicate = self.documents.iter().any(|other| {
                                    other.id != doc.id && name(&other.disk.path) == basename
                                });
                                let title = if duplicate {
                                    format!(
                                        "{} — {}",
                                        basename,
                                        doc.disk.path.parent().map(name).unwrap_or_default()
                                    )
                                } else {
                                    basename
                                };
                                let label = format!(
                                    "{}{}{}",
                                    title,
                                    "",
                                    if doc.external.is_some() {
                                        " · Conflict"
                                    } else if doc.saving {
                                        " · Saving…"
                                    } else {
                                        ""
                                    }
                                );
                                let colors = appearance::palette(ui);
                                ui.push_id(doc.id, |ui| {
                                    egui::Frame::new()
                                        .fill(if selected {
                                            colors.canvas
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .corner_radius(egui::CornerRadius {
                                            nw: 9,
                                            ne: 9,
                                            sw: 0,
                                            se: 0,
                                        })
                                        .inner_margin(egui::Margin::symmetric(10, 5))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let (icon_rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(16., 16.),
                                                    egui::Sense::hover(),
                                                );
                                                appearance::file_symbol(
                                                    ui,
                                                    icon_rect,
                                                    &doc.disk.path,
                                                    false,
                                                );
                                                let mut text = RichText::new(label)
                                                    .size(13.)
                                                    .color(if selected {
                                                        colors.text
                                                    } else {
                                                        colors.secondary
                                                    });
                                                if selected {
                                                    text = text.font(appearance::heading_font(13.));
                                                }
                                                if doc.preview {
                                                    text = text.italics();
                                                }
                                                let response = ui
                                                    .add(
                                                        egui::Button::new(text)
                                                            .frame(false)
                                                            .truncate()
                                                            .min_size(egui::vec2(76., 28.)),
                                                    )
                                                    .on_hover_text(format!(
                                                        "{}{}",
                                                        crate::paths::display(&doc.disk.path),
                                                        if doc.preview {
                                                            "\nPreview · double-click to keep open"
                                                        } else {
                                                            ""
                                                        }
                                                    ));
                                                response.widget_info(|| {
                                                    egui::WidgetInfo::selected(
                                                        egui::Role::Tab,
                                                        ui.is_enabled(),
                                                        selected,
                                                        format!(
                                                            "{}{}{}",
                                                            title,
                                                            if doc.dirty() {
                                                                ", Modified"
                                                            } else {
                                                                ""
                                                            },
                                                            if doc.preview {
                                                                ", Preview"
                                                            } else {
                                                                ""
                                                            }
                                                        ),
                                                    )
                                                });
                                                if response.clicked() {
                                                    activate = Some(doc.id);
                                                }
                                                if response.double_clicked() {
                                                    pin = Some(doc.id);
                                                }
                                                if doc.dirty() {
                                                    let (dot, _) = ui.allocate_exact_size(
                                                        egui::vec2(8., 8.),
                                                        egui::Sense::hover(),
                                                    );
                                                    ui.painter().circle_filled(
                                                        dot.center(),
                                                        3.,
                                                        colors.accent,
                                                    );
                                                }
                                                if appearance::symbol_button(
                                                    ui,
                                                    Symbol::Close,
                                                    &format!("Close {}", title),
                                                    false,
                                                    egui::vec2(18., 24.),
                                                )
                                                .clicked()
                                                {
                                                    close = Some(doc.id);
                                                }
                                            });
                                        });
                                });
                            }
                            if icon_button(ui, Symbol::Plus, "New File · ⌘N", false).clicked() {
                                self.command(Action::New, ui.ctx());
                            }
                            if self.documents.is_empty() {
                                ui.weak("No document open");
                            }
                            if let Some(doc) = self
                                .documents
                                .iter()
                                .find(|doc| Some(doc.id) == self.active)
                            {
                                let markdown = doc.disk.path.extension().is_some_and(|ext| {
                                    ext.eq_ignore_ascii_case("md")
                                        || ext.eq_ignore_ascii_case("markdown")
                                });
                                if markdown {
                                    let read = ui.selectable_label(self.read, "Read").clicked();
                                    let edit = ui.selectable_label(!self.read, "Edit").clicked();
                                    if (read && !self.read) || (edit && self.read) {
                                        self.command(Action::Read, ui.ctx());
                                    }
                                }
                            }
                        });
                    });
            });
        if let Some(id) = activate {
            if let Some(doc) = self.documents.iter().find(|doc| doc.id == id) {
                self.open_latency = Some((doc.disk.path.clone(), std::time::Instant::now()));
                if !doc.disk.path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                }) {
                    self.read = false;
                }
            }
            self.active = Some(id);
            self.focus_editor = true;
            self.navigator.leave_for_editor();
        }
        if let Some(id) = pin
            && let Some(doc) = self.documents.iter_mut().find(|d| d.id == id)
        {
            doc.preview = false;
        }
        if let Some(id) = close {
            self.request_close(id);
        }
    }

    fn document_pane(
        &mut self,
        ui: &mut egui::Ui,
        frame: &mut eframe::Frame,
        view: &mut Presentation,
    ) {
        let ctx = ui.ctx().clone();
        self.tabs(ui);
        if self.read
            && self
                .documents
                .iter()
                .find(|doc| Some(doc.id) == self.active)
                .is_none_or(|doc| {
                    !doc.disk.path.extension().is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
                    })
                })
        {
            self.read = false;
        }
        egui::Panel::bottom("document_status")
            .frame(
                egui::Frame::new()
                    .fill(appearance::palette(ui).chrome)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ui, |ui| {
                if let Some(doc) = self
                    .documents
                    .iter_mut()
                    .find(|d| Some(d.id) == self.active)
                {
                    let index = self
                        .native
                        .selection(doc.id)
                        .map(|r| r.location)
                        .unwrap_or(0);
                    let (line, column) = doc.metadata.position_utf16(&doc.text, index);
                    let colors=appearance::palette(ui);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(name(&doc.disk.path)).size(11.).color(colors.secondary)).truncate());
                        if ui.available_width()>430. {let (r,_) = ui.allocate_exact_size(egui::vec2(16.,16.),egui::Sense::hover());appearance::file_symbol(ui,r,&doc.disk.path,false);if let Some(h)=&doc.highlight {ui.small(&h.state);}}
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                            ui.small(doc.metadata.endings);ui.separator();ui.small(if doc.disk.bom {"UTF-8 BOM"}else{"UTF-8"});
                            if ui.available_width()>160. {ui.separator();ui.small(format!("Spaces: {}",self.settings.indent));}
                            ui.separator();ui.small(format!("Ln {line}, Col {column}"));
                        });
                    });
                    if doc.metadata.longest_line>16384 {ui.small("Long lines · wrap and ruler paused").on_hover_text("Lines over 16 KiB use horizontal scrolling to bound native text layout. Source bytes and editing remain intact.");}

                }
            });
        if let Some(message) = &self.status {
            let mut dismiss = false;
            egui::Panel::bottom("notice")
                .frame(appearance::toolbar_frame(ui))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(appearance::palette(ui).warning, message);
                        dismiss = ui.small_button("Dismiss").clicked();
                    });
                });
            if dismiss {
                self.status = None;
            }
        }
        if let Some(warning) = &self.history_warning {
            egui::Panel::bottom("history_notice")
                .frame(appearance::toolbar_frame(ui))
                .show(ui, |ui| {
                    ui.colored_label(appearance::palette(ui).warning, warning);
                });
        }
        if let Some(doc) = self.documents.iter().find(|d| Some(d.id) == self.active)
            && doc.external.is_some()
        {
            let mut decision = None;
            let mut copy = false;
            egui::Panel::top("conflict_actions")
                .frame(appearance::toolbar_frame(ui))
                .show(ui, |ui| {
                    ui.colored_label(
                        appearance::palette(ui).warning,
                        "The file changed on disk. Your edits are preserved.",
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Compare").clicked() {
                            self.compare_open = true;
                        }
                        if ui.button("Save Copy…").clicked() {
                            copy = true;
                        }
                        ui.menu_button("Resolve", |ui| {
                            if ui.button("Reload Disk Version…").clicked() {
                                decision = Some(false);
                                ui.close();
                            }
                            if ui.button("Overwrite Disk Version…").clicked() {
                                decision = Some(true);
                                ui.close();
                            }
                        });
                    });
                });
            if let Some(overwrite) = decision
                && let Some((disk, _)) = &doc.external
            {
                view.decision = Some(ConflictDecision {
                    id: doc.id,
                    generation: doc.generation,
                    disk: std::sync::Arc::new(disk.clone()),
                    overwrite,
                });
            }
            if copy {
                self.save_to(true);
            }
        }
        self.conflict_dialog(&ctx, view);
        let hidden = self.overlay_open(view)
            || egui::Popup::is_any_open(&ctx)
            || !self.completed_paths.is_empty();
        let mut markdown_action = None;
        let mut focused = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(appearance::palette(ui).canvas).inner_margin(if self.read {egui::Margin::symmetric(32,22)}else{egui::Margin::ZERO})).show(ui,|ui|{
            if let Some(doc)=self.documents.iter_mut().find(|d|Some(d.id)==self.active){
                if doc.external.is_none() && let Some(conflict)=&doc.conflict{ui.colored_label(appearance::palette(ui).warning,conflict);}
                if hidden{return;}
                if self.read{
                    focused=self.focus_editor;self.focus_editor=false;
                    self.markdown.request(doc.id,doc.generation,&doc.text,&doc.disk.path);
                    ui.scope(|ui|{ui.style_mut().text_styles.insert(egui::TextStyle::Body,egui::FontId::proportional(self.settings.markdown_font_size));ui.style_mut().text_styles.insert(egui::TextStyle::Heading,egui::FontId::proportional(self.settings.markdown_font_size*2.));markdown_action=self.markdown.render(ui,doc.id,doc.generation,&doc.disk.path);});
                }else if self.use_native{
                    if let Err(error)=self.native.show(ui,frame,native_editor::Source{id:doc.id,text:&doc.text,generation:doc.generation,metadata:&doc.metadata,highlight:doc.highlight.as_ref(),read_only:false},&self.settings,self.focus_editor){self.status=Some(error);}
                    self.focus_editor=false;focused=self.native.focused(doc.id);
                    if self.goto_path.as_ref().is_none_or(|p|*p==doc.disk.path)&&let Some((line,column))=self.goto.take(){let(index,clamped)=editor::position(&doc.text,line,column);let offset=doc.text.chars().take(index).map(char::len_utf16).sum();self.native.select(doc.id,offset,offset);self.goto_path=None;if clamped{self.status=Some("Requested position clamped to document and grapheme bounds.".into());}}
                }else{
                    egui::ScrollArea::both().id_salt(("document_scroll",doc.id)).auto_shrink([false,false]).show(ui,|ui|{
                        let editor_id=egui::Id::unique(("document",doc.id));
                        if !self.settings.tabs&&ui.memory(|m|m.has_focus(editor_id)){ui.input_mut(|i|{for event in &mut i.events{if matches!(event,egui::Event::Key{key:egui::Key::Tab,pressed:true,modifiers,..}
if !modifiers.shift&&!modifiers.command&&!modifiers.alt){*event=egui::Event::Text(" ".repeat(self.settings.indent));}}});}
                        let response=ui.add_sized(ui.available_size(),egui::TextEdit::multiline(&mut doc.text).id(editor_id).font(if self.settings.proportional{egui::FontId::proportional(self.settings.font_size)}else{egui::FontId::monospace(self.settings.font_size)}).code_editor().interactive(!self.native.is_locked(doc.id)).frame(egui::Frame::NONE).desired_width(if self.settings.wrap{ui.available_width()}else{f32::INFINITY}));
                        if response.changed(){doc.generation+=1;doc.preview=false;self.io.send(Job::Recover(doc.id,storage::Draft{path:doc.disk.path.clone(),text:doc.text.clone(),base:doc.disk.bytes.clone(),target:Some(doc.disk.target.clone()),generation:doc.generation}));}
                        if self.focus_editor{response.request_focus();self.focus_editor=false;}
                        if self.goto_path.as_ref().is_none_or(|p|*p==doc.disk.path)&&let Some((line,column))=self.goto.take(){let(cursor,clamped)=editor::position(&doc.text,line,column);if clamped{self.status=Some("Requested position clamped to document and grapheme bounds.".into());}
if let Some(mut state)=egui::TextEdit::load_state(&ctx,response.id){state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(cursor))));state.store(&ctx,response.id);response.request_focus();ctx.request_repaint();}self.goto_path=None;}
                        focused=response.has_focus();
                    });
                }
            }else{
                ui.vertical_centered(|ui|{
                    ui.add_space((ui.available_height()*0.28).max(24.));
                    let(rect,_)=ui.allocate_exact_size(egui::vec2(40.,40.),egui::Sense::hover());appearance::symbol(ui,rect,Symbol::File,appearance::palette(ui).secondary);
                    ui.heading(if self.opening.is_empty(){"Select a file to edit"}else{"Opening…"});ui.label("Choose a file from the navigator, or open one directly.");ui.add_space(12.);
                    if appearance::primary_button(ui,"Open File…").clicked(){self.command(Action::Open,&ctx);}
                    if ui.button("New File").clicked(){self.command(Action::New,&ctx);}
                });
            }
        });
        if focused {
            if self.navigator.leave_for_editor() {
                ctx.request_repaint();
            }
            if let Some(doc) = self
                .documents
                .iter()
                .find(|doc| Some(doc.id) == self.active)
            {
                self.navigator.reveal_document(&doc.disk.path);
            }
        }
        if let Some(link) = markdown_action {
            match link {
                markdown::Link::Local(path) => {
                    self.read = false;
                    self.io.send(Job::Visit(path));
                }
                markdown::Link::External(url) => {
                    self.io.send(Job::External(PathBuf::from(url), false))
                }
                markdown::Link::Unsupported(url) => {
                    self.status = Some(format!("Unsupported link: {url}"))
                }
            }
        }
    }

    fn conflict_dialog(&mut self, ctx: &egui::Context, view: &mut Presentation) {
        if self.compare_open {
            let mut close = false;
            egui::Modal::new(egui::Id::unique("compare_versions")).show(ctx, |ui| {
                let width=(ctx.content_rect().width()-80.).clamp(160.,920.);ui.set_width(width);
                ui.horizontal(|ui|{let (r,_) = ui.allocate_exact_size(egui::vec2(48.,48.),egui::Sense::hover());appearance::symbol(ui,r,Symbol::File,appearance::palette(ui).warning);ui.vertical(|ui|{ui.label(RichText::new("File has been modified on disk").font(appearance::heading_font(23.)));ui.weak("Another application changed this file. Your local edits are preserved.");});});
                ui.add_space(16.);
                if let Some(doc)=self.documents.iter().find(|d|Some(d.id)==self.active){
                    egui::Frame::new().fill(appearance::palette(ui).chrome).stroke(egui::Stroke::new(1.,appearance::palette(ui).separator)).corner_radius(8.).inner_margin(14.).show(ui,|ui|{ui.set_width(width-30.);ui.strong(name(&doc.disk.path));ui.add(egui::Label::new(crate::paths::display(&doc.disk.path)).truncate());});
                    ui.add_space(12.);
                    if let Some((_, text)) = &doc.external {
                        let prefix=&text[..text.floor_char_boundary(text.len().min(16*1024))];
                        if view.comparison.as_ref().is_none_or(|cached| cached.id != doc.id || cached.generation != doc.generation || cached.disk_len != text.len() || cached.disk_prefix != prefix) {
                            view.comparison = Some(ConflictPreview { id: doc.id, generation: doc.generation, disk_prefix: prefix.to_owned(), disk_len: text.len(), diff: std::sync::Arc::new(crate::comparison::Comparison::new(&doc.text, text)) });
                        }
                        let diff = &view.comparison.as_ref().unwrap().diff;
                        let height=(ctx.content_rect().height()-350.).clamp(100.,320.);
                        let pane=|ui:&mut egui::Ui,local:bool| { comparison_pane(ui,diff,local,height); };
                        if width>=640. {ui.columns(2,|cols|{pane(&mut cols[0],true);pane(&mut cols[1],false);});} else {egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-300.).max(80.)).show(ui,|ui|{pane(ui,true);pane(ui,false);});}
                        ui.add_space(12.);ui.weak("− Local-only line   + Disk-only line   ≈ Clipped line, not compared");
                        ui.weak("Read-only line diff within displayed excerpts. Neither version is changed here.");
                    } else {ui.label("No pending disk version.");}

                }
                ui.add_space(12.);
                close = ui.button("Keep Editing").clicked();
            });
            if close {
                self.compare_open = false;
                view.comparison = None;
                self.focus_editor = true;
            }
        }
        if let Some(decision) = view.decision.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("resolve_conflict")).show(ctx,|ui|{
                ui.set_width((ctx.content_rect().width()-64.).clamp(160.,520.));
                ui.heading(if decision.overwrite{"Overwrite this disk version?"}else{"Reload the disk version?"});
                ui.label(crate::paths::display(&decision.disk.path));
                ui.label(if decision.overwrite{"Your current text will replace the observed disk version. Save validates the destination again."}else{"Your unsaved local edits will be replaced with the observed disk version."});
                ui.horizontal_wrapped(|ui|{cancel=ui.button("Cancel").clicked();confirm=ui.button(if decision.overwrite{"Overwrite Disk Version"}else{"Reload Disk Version"}).clicked();});
            });
            if confirm {
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == decision.id) {
                    if self.native.is_locked(doc.id)
                        || doc.generation != decision.generation
                        || doc.external.as_ref().is_none_or(|(disk, _)| {
                            disk.bytes != decision.disk.bytes || disk.target != decision.disk.target
                        })
                    {
                        self.status = Some(
                            "A version changed while deciding. Compare the current versions again."
                                .into(),
                        );
                    } else if let Some((disk, text)) = doc.external.take() {
                        if decision.overwrite {
                            doc.disk = disk;
                            doc.saving = true;
                            self.io.send(Job::Save {
                                id: doc.id,
                                disk: doc.disk.clone(),
                                text: doc.text.clone(),
                                generation: doc.generation,
                                without_recovery: false,
                            });
                        } else {
                            editor::replace(
                                ctx,
                                egui::Id::unique(("document", doc.id)),
                                &doc.text,
                                &text,
                            );
                            doc.disk = disk;
                            doc.generation += 1;
                            doc.text = text.clone();
                            doc.saved_text = text;
                            doc.restored = false;
                            doc.conflict = None;
                            self.status = Some(
                                "Disk version loaded; Undo restores the previous text.".into(),
                            );
                            self.io.send(Job::ClearThrough(doc.id, doc.generation));
                            if self.wait_owners.values().any(|owner| *owner == doc.id) {
                                self.request_abandoned = true;
                                EXIT_STATUS.store(
                                    if self.failed_wait.is_empty() { 1 } else { 2 },
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                        }
                    }
                }
                view.decision = None;
                self.focus_editor = true;
            }
            if cancel {
                view.decision = None;
                self.focus_editor = true;
            }
        }
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(Confirmation {
            id,
            saving,
            filename,
        }) = self.pending_document(ctx)
        else {
            return;
        };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(160., 440.));
            ui.heading("Save changes?");
            ui.label(format!("{filename} has unsaved changes."));
            if let Some(error) = self.save_errors.get(&id) {
                ui.colored_label(appearance::palette(ui).error, error);
            }
            if saving {
                ui.label("Saving…");
            }
            ui.add_space(12.);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!saving, egui::Button::new("Discard Changes"))
                    .clicked()
                {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(2);
                }
                ui.add_enabled_ui(!saving, |ui| {
                    if appearance::primary_button(ui, "Save").clicked() {
                        choice = Some(0);
                    }
                });
            });
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            choice = Some(2);
        }
        if let Some(choice) = choice {
            self.confirm_choice(id, choice);
        }
    }

    fn cancel_surface(&mut self, ctx: &egui::Context, view: &mut Presentation) {
        if egui::Popup::is_any_open(ctx) {
            egui::Popup::close_all(ctx);
        } else if view.record_to_trash.is_some() {
            view.record_to_trash = None;
        } else if view.decision.is_some() {
            view.decision = None;
            self.focus_editor = true;
        } else if self.compare_open {
            self.compare_open = false;
            view.comparison = None;
            self.focus_editor = true;
        } else if self.find_open {
            self.find_open = false;
            self.focus_editor = true;
        } else if self.search_open {
            self.close_search();
        } else if view.path_open {
            view.path_open = false;
            self.completed_paths.clear();
            self.completion = None;
            self.completions.retain(&[]);
            self.focus_editor = true;
        } else {
            self.command(Action::Cancel, ctx);
        }
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let view_id = egui::Id::unique("glimpse_presentation");
        let mut view = ctx
            .data(|d| d.get_temp::<Presentation>(view_id))
            .unwrap_or_default();
        if !self.find_open {
            view.go_line = None;
        }
        let overlay_at_start = self.overlay_open(&view);
        if view.restore_editor_next_pass {
            self.focus_editor = !self.overlay_open(&view)
                && !self.find_open
                && !self.search_open
                && !self.path_focus;
            view.restore_editor_next_pass = false;
        }
        let native_was_focused = self.active.is_some_and(|id| self.native.focused(id));
        if view.was_popup && !egui::Popup::is_any_open(&ctx) && view.restore_after_popup {
            self.focus_editor = true;
            view.restore_after_popup = false;
        }
        self.composing = ctx.input(|i| {
            i.events
                .iter()
                .fold(self.composing, |active, event| match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { .. }) => true,
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) => false,
                    _ => active,
                })
        });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.cancel_surface(&ctx, &mut view);
        }
        if self.use_native {
            for action in self.native.commands() {
                if action == Action::Cancel {
                    self.cancel_surface(&ctx, &mut view);
                } else if !self.overlay_open(&view) {
                    self.command(action, &ctx);
                }
            }
            self.native.set_bindings(self.bindings.clone());
            self.native
                .retain(&self.documents.iter().map(|d| d.id).collect::<Vec<_>>());
            self.native.begin();
        }
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            {
                let path = file.path().to_path_buf();
                self.io.send(Job::Visit(path));
            }
        }
        let scope = self
            .temporary_scope
            .clone()
            .or_else(|| self.scope.clone())
            .unwrap_or(self.navigator.current.clone());
        if scope != self.last_scope {
            self.last_scope = scope;
            self.search_id = self.search.cancel();
            self.hits.clear();
            self.search_state = "Cancelled — scope changed".into();
        }
        if self.navigator.current != self.last_location {
            if !self.focus_editor {
                view.narrow_document = false;
                view.navigator_hidden = false;
            }
            if !self.history_move {
                self.history.push(self.last_location.clone());
                self.future.clear();
            }
            self.history_move = false;
            self.last_location = self.navigator.current.clone();
            if !ctx.memory(|m| m.has_focus(egui::Id::unique("path_input"))) {
                self.path_input = self
                    .navigator
                    .current
                    .to_str()
                    .unwrap_or_default()
                    .to_owned();
            }
        }
        if !self.overlay_open(&view) {
            for action in commands::consume(&ctx, &self.bindings) {
                self.command(action, &ctx);
            }
        }
        let overlay = self.overlay_open(&view);
        if overlay && !view.was_overlay {
            self.native.focus_host();
            self.navigator.focused = false;
        }
        if !overlay && view.was_overlay && !self.find_open && !self.search_open && !self.path_focus
        {
            self.focus_editor = self.active.is_some();
        }
        if self.focus_editor && self.active.is_some() {
            view.narrow_document = true;
        }
        let animation_time = if self.settings.reduce_motion {
            0.
        } else {
            0.10
        };
        if ctx.style_of(ctx.theme()).animation_time != animation_time {
            for theme in [egui::Theme::Light, egui::Theme::Dark] {
                let mut style = (*ctx.style_of(theme)).clone();
                style.animation_time = animation_time;
                ctx.set_style_of(theme, style);
            }
        }
        self.toolbar(&ctx, ui, &mut view);
        if self.search_open
            && ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|pos| {
                        view.search_anchor
                            .is_none_or(|anchor| !anchor.contains(pos))
                    })
            })
        {
            self.search_open = false;
            self.temporary_scope = None;
            self.search_id = self.search.cancel();
        }
        let drafts = self
            .recovered
            .iter()
            .filter(|r| !r.history && !r.candidate)
            .count();
        if drafts > 0 && !view.recovery_notice_dismissed && !self.recovery_open {
            egui::Panel::top("recovered_drafts_notice")
                .frame(appearance::toolbar_frame(ui))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if drafts == 1 {
                            "A recovered draft is available.".into()
                        } else {
                            format!("{drafts} recovered drafts are available.")
                        });
                        if ui.button("Review Drafts…").clicked() {
                            self.recovery_open = true;
                            self.io.send(Job::Inventory);
                        }
                        if ui.small_button("Dismiss").clicked() {
                            view.recovery_notice_dismissed = true;
                        }
                    });
                });
        }
        self.dialogs(&ctx, &mut view);
        let narrow = ui.available_width() < 760.;
        let enabled = !self.overlay_open(&view) && !egui::Popup::is_any_open(&ctx);
        if !narrow && !view.navigator_hidden {
            egui::Panel::left("directory_list")
                .default_size(280.)
                .min_size(220.)
                .max_size((ui.available_width() - 320.).max(220.))
                .frame(
                    egui::Frame::new()
                        .fill(appearance::palette(ui).sidebar)
                        .inner_margin(8.),
                )
                .show(ui, |ui| self.navigator_pane(ui, &mut view, enabled));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(if narrow && !view.narrow_document {
                appearance::palette(ui).sidebar
            } else {
                appearance::palette(ui).canvas
            }))
            .show(ui, |ui| {
                if narrow && !view.narrow_document {
                    egui::Frame::new()
                        .inner_margin(12.)
                        .show(ui, |ui| self.navigator_pane(ui, &mut view, enabled));
                } else {
                    self.document_pane(ui, frame, &mut view);
                }
            });
        self.floating_panels(&ctx, frame, &mut view);
        self.flush_navigation();
        self.destination_dialogs(&ctx);
        self.confirm(&ctx);
        if self.use_native {
            self.native.set_available(
                &commands::COMMANDS
                    .iter()
                    .map(|c| c.action)
                    .filter(|action| {
                        use commands::Action::*;
                        (!self.overlay_open(&view) || *action == Cancel)
                            && !matches!(action, Choose)
                            && (!matches!(
                                action,
                                Save | SaveAs | SaveCopy | Close | Find | Read | Indent | Outdent
                            ) || self.active.is_some())
                            && (!matches!(action, Finish | Abort) || self.wait_session)
                    })
                    .collect::<Vec<_>>(),
            );
            self.native.finish();
        }
        if overlay_at_start
            && !self.overlay_open(&view)
            && (self.focus_editor || (!self.search_open && !self.navigator.focused))
            && self.active.is_some()
        {
            // Complete egui's modal focus restoration before handing focus back to AppKit.
            view.restore_editor_next_pass = true;
            ctx.request_repaint();
        }
        let popup = egui::Popup::is_any_open(&ctx);
        if popup && !view.was_popup {
            view.restore_after_popup = native_was_focused;
        }
        view.was_popup = popup;
        view.operation_shown = self.operation;
        view.was_overlay = self.overlay_open(&view);
        view.was_search = self.search_open;
        ctx.data_mut(|d| d.insert_temp(view_id, view));
        self.after_frame(&ctx);
    }

    fn destination_dialogs(&mut self, ctx: &egui::Context) {
        let mut decision = None;
        if let Some(destination) = &self.abandon_destination {
            egui::Modal::new(egui::Id::unique("abandon_requested")).show(ctx,|ui|{
                ui.set_width((ctx.content_rect().width()-64.).clamp(160.,560.));
                ui.heading("Leave the requested destination?");
                ui.label("Save As changes the destination and abandons the original CLI request. Save Copy keeps that request pending.");
                ui.label(crate::paths::display(&destination.original.path));ui.label(format!("New destination: {}",crate::paths::display(&destination.path)));
                ui.horizontal_wrapped(|ui|{
                    if ui.button("Cancel").clicked(){decision=Some(0);}
                    if ui.button("Save Copy Instead").clicked(){decision=Some(1);}
                    if ui.button("Continue Save As").clicked(){decision=Some(2);}
                });
            });
        }
        if let Some(choice) = decision
            && let Some(mut destination) = self.abandon_destination.take()
        {
            if choice == 0 {
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == destination.id) {
                    doc.saving = false;
                }
                self.focus_editor = true;
            } else {
                destination.copy = choice == 1;
                self.io.send(Job::Prepare(destination));
            }
        }
        decision = None;
        if let Some(destination) = &self.replace_destination {
            egui::Modal::new(egui::Id::unique("replace_destination")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.).clamp(160., 560.));
                ui.heading("Replace this disk version?");
                ui.add(egui::Label::new(crate::paths::display(&destination.path)).truncate())
                    .on_hover_text(crate::paths::display(&destination.path));
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 220.).max(60.))
                    .show(ui, |ui| {
                        if let Some(disk) = &destination.expected {
                            ui.weak(format!(
                                "Observed destination · {} bytes · excerpt only",
                                disk.bytes.len()
                            ));
                            let excerpt = String::from_utf8_lossy(
                                &disk.bytes[..disk.bytes.len().min(16 * 1024)],
                            );
                            appearance::bounded_preview(ui, "destination_preview", &excerpt);
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Cancel").clicked() {
                        decision = Some(0);
                    }
                    if ui.button("Replace This Version").clicked() {
                        decision = Some(1);
                    }
                });
            });
        }
        if let Some(choice) = decision
            && let Some(destination) = self.replace_destination.take()
        {
            if choice == 0 {
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == destination.id) {
                    doc.saving = false;
                }
                self.focus_editor = true;
            } else {
                self.io.send(Job::SaveTo(destination));
            }
        }
    }
}

fn preference_toggle(ui: &mut egui::Ui, label: &str, value: &mut bool) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.), egui::Sense::click());
    if response.clicked() {
        *value = !*value;
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), *value, label)
    });
    let colors = appearance::palette(ui);
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        colors.text,
    );
    let switch = egui::Rect::from_center_size(
        rect.right_center() - egui::vec2(18., 0.),
        egui::vec2(32., 18.),
    );
    ui.painter().rect_filled(
        switch,
        9.,
        if *value {
            colors.accent
        } else {
            colors.separator
        },
    );
    ui.painter().circle_filled(
        egui::pos2(
            if *value {
                switch.right() - 9.
            } else {
                switch.left() + 9.
            },
            switch.center().y,
        ),
        7.,
        egui::Color32::WHITE,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            switch.expand(2.),
            11.,
            egui::Stroke::new(1., colors.accent),
            egui::StrokeKind::Outside,
        );
    }
}

fn preference_row(ui: &mut egui::Ui, label: &str, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(38.);
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().slider_width = 120.;
            control(ui);
        });
    });
}
fn theme_tile(ui: &mut egui::Ui, title: &str, kind: usize, selected: bool, width: f32) -> bool {
    let colors = appearance::palette(ui);
    let (r, response) = ui.allocate_exact_size(egui::vec2(width, 92.), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, title));
    let card = egui::Rect::from_min_size(r.min, egui::vec2(width, 68.));
    let p = ui.painter();
    p.rect_filled(card, 7., colors.chrome);
    p.rect_stroke(
        card,
        7.,
        egui::Stroke::new(
            if selected { 2. } else { 1. },
            if selected {
                colors.accent
            } else {
                colors.separator
            },
        ),
        egui::StrokeKind::Inside,
    );
    let w = egui::Rect::from_center_size(card.center(), egui::vec2(64., 44.));
    p.rect_filled(
        w.translate(egui::vec2(0., 2.)),
        4.,
        egui::Color32::from_black_alpha(28),
    );
    p.rect_filled(
        w,
        4.,
        if kind == 2 {
            egui::Color32::from_gray(32)
        } else {
            egui::Color32::WHITE
        },
    );
    if kind == 0 {
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(w.center().x, w.top()), w.max),
            3.,
            egui::Color32::from_gray(32),
        );
    }
    for (i, c) in [0xff6058, 0xffbd2e, 0x28c840].into_iter().enumerate() {
        p.circle_filled(
            w.min + egui::vec2(7. + i as f32 * 7., 7.),
            2.,
            egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8),
        );
    }
    for row in 0..3 {
        let y = w.top() + 18. + row as f32 * 7.;
        p.line_segment(
            [
                egui::pos2(w.left() + 8., y),
                egui::pos2(w.right() - 8. - if row == 2 { 12. } else { 0. }, y),
            ],
            egui::Stroke::new(
                3.,
                if kind == 2 {
                    egui::Color32::from_gray(65)
                } else {
                    egui::Color32::from_gray(224)
                },
            ),
        );
    }
    p.text(
        egui::pos2(r.center().x, r.bottom() - 8.),
        egui::Align2::CENTER_CENTER,
        title,
        egui::FontId::proportional(13.),
        colors.text,
    );
    response.clicked()
}

fn comparison_pane(
    ui: &mut egui::Ui,
    diff: &crate::comparison::Comparison,
    local: bool,
    height: f32,
) {
    use crate::comparison::Change;
    let excerpt = if local { &diff.local } else { &diff.disk };
    let colors = appearance::palette(ui);
    egui::Frame::new()
        .stroke(egui::Stroke::new(1., colors.separator))
        .corner_radius(7.)
        .inner_margin(10.)
        .show(ui, |ui| {
            ui.label(
                RichText::new(if local {
                    "Your version · unsaved local text"
                } else {
                    "Disk version · observed saved text"
                })
                .font(appearance::heading_font(14.)),
            );
            if excerpt.truncated {
                ui.colored_label(
                    colors.warning,
                    "Excerpt truncated · 160 lines / 240 characters per line / 16 KiB",
                );
            }
            ui.separator();
            egui::ScrollArea::both()
                .id_salt(("conflict_line_diff", local))
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.;
                    let dark = ui.visuals().dark_mode;
                    for row in &diff.rows {
                        let line =
                            if local { row.local } else { row.disk }.map(|i| &excerpt.lines[i]);
                        let (marker, fill) = match row.change {
                            Change::Removed if local => (
                                "−",
                                if dark {
                                    egui::Color32::from_rgb(81, 39, 46)
                                } else {
                                    egui::Color32::from_rgb(255, 229, 229)
                                },
                            ),
                            Change::Added if !local => (
                                "+",
                                if dark {
                                    egui::Color32::from_rgb(30, 65, 44)
                                } else {
                                    egui::Color32::from_rgb(225, 245, 231)
                                },
                            ),
                            Change::Uncompared if line.is_some() => ("≈", colors.chrome),
                            _ => (" ", colors.canvas),
                        };
                        egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(4, 1))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                // Empty opposite-side rows retain the same baseline as text rows.
                                ui.set_min_height(
                                    ui.text_style_height(&egui::TextStyle::Monospace),
                                );
                                let text = line
                                    .map(|line| {
                                        format!(
                                            "{marker} {:>3}  {}{}",
                                            line.number,
                                            line.text,
                                            if line.no_final_newline() {
                                                "  ⟪no final newline⟫"
                                            } else {
                                                ""
                                            }
                                        )
                                    })
                                    .unwrap_or_default();
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(text).monospace().color(colors.text),
                                    )
                                    .wrap_mode(egui::TextWrapMode::Extend)
                                    .selectable(true),
                                );
                            });
                    }
                });
        });
}
