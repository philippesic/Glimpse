use super::*;
use appearance::{Symbol, icon_button};

#[derive(Clone, Default)]
struct Presentation {
    preview: bool,
    path: bool,
    was_search: bool,
    was_palette: bool,
}

impl Picker {
    fn search_pane(&mut self, ui: &mut egui::Ui, view: &mut Presentation) {
        ui.horizontal(|ui| {
            ui.strong("Search");
            if icon_button(ui, Symbol::Close, "Close Search", false).clicked() {
                self.search_open = false;
                self.search_id = self.search.cancel();
            }
        });
        let old_content = self.content;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.content, false, "Files");
            ui.selectable_value(&mut self.content, true, "Text");
        });
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text(if self.content {
                    "Text in saved files"
                } else {
                    "File name or path"
                })
                .desired_width(f32::INFINITY),
        );
        if !view.was_search {
            field.request_focus();
        }
        if field.changed() || old_content != self.content {
            self.search_id = self.search.cancel();
            self.hits.clear();
            self.search_state = "Ready · query changed".into();
        }
        let scope = self.scope();
        ui.horizontal_wrapped(|ui| {
            let run = appearance::primary_button(ui, "Search").clicked()
                || (field.has_focus()
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
            if run && !self.query.is_empty() {
                match self.search.start_filtered(
                    scope.clone(),
                    self.query.clone(),
                    self.content,
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
                    Err(error) => self.search_state = error,
                }
            }
            if ui.button("Stop").clicked() {
                self.search_id = self.search.cancel();
                self.search_state = "Cancelled".into();
            }
            ui.menu_button("Scope", |ui| {
                ui.label(crate::paths::display(&scope));
                if ui
                    .button(if self.scope.is_some() {
                        "Unpin Search Scope"
                    } else {
                        "Pin Scope Here"
                    })
                    .clicked()
                {
                    self.command(Action::Scope, ui.ctx());
                    ui.close();
                }
                if ui.button("Go to Search Folder").clicked() {
                    self.command(Action::GoScope, ui.ctx());
                    ui.close();
                }
            });
        });
        ui.add(
            egui::Label::new(format!(
                "In {} · {}",
                crate::name(&scope),
                if self.scope.is_some() {
                    "pinned"
                } else {
                    "follows location"
                }
            ))
            .truncate(),
        )
        .on_hover_text(crate::paths::display(&scope));
        ui.colored_label(
            if self.search_state.starts_with("Complete") {
                appearance::palette(ui).success
            } else {
                appearance::palette(ui).secondary
            },
            &self.search_state,
        );
        ui.collapsing("Coverage and exclusions",|ui| {
            ui.small(format!("Saved disk contents. Ignored paths: {}. Version-control metadata: {}. No directory symlinks or nested mounts. Up to 100,000 entries, 1,000 hits and {} MiB per content file. {}",
                if self.settings.search_ignored{"included"}else{"excluded"},if self.settings.search_metadata{"included"}else{"excluded"},self.settings.search_file_mib,self.settings.search_exclude));
        });
        ui.separator();
        let mut selected = None;
        egui::ScrollArea::vertical()
            .id_salt("picker_results")
            .auto_shrink([false, false])
            .show_rows(ui, 46., self.hits.len(), |ui, range| {
                for index in range {
                    let hit = &self.hits[index];
                    ui.push_id(index, |ui| {
                        ui.set_height(46.);
                        if ui
                            .add(
                                egui::Button::new((
                                    egui::RichText::new(crate::name(&hit.path)).strong(),
                                    egui::Atom::grow(),
                                ))
                                .frame_when_inactive(false)
                                .truncate()
                                .min_size(egui::vec2(ui.available_width(), 22.)),
                            )
                            .on_hover_text(format!(
                                "{}:{}\n{}",
                                crate::paths::display(&hit.path),
                                hit.line,
                                hit.excerpt
                            ))
                            .clicked()
                        {
                            selected = Some(hit.path.clone());
                        }
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(if self.content {
                                    format!(
                                        "{}:{} · {}",
                                        crate::paths::display(
                                            hit.path.strip_prefix(&scope).unwrap_or(&hit.path)
                                        ),
                                        hit.line,
                                        hit.excerpt
                                    )
                                } else {
                                    crate::paths::display(
                                        hit.path.strip_prefix(&scope).unwrap_or(&hit.path),
                                    )
                                })
                                .small()
                                .weak(),
                            )
                            .truncate(),
                        );
                    });
                }
            });
        if let Some(path) = selected {
            self.preview(path);
            self.search_open = false;
            view.preview = true;
        }
    }

    fn navigation_pane(&mut self, ui: &mut egui::Ui, view: &mut Presentation) {
        if self.choosing || self.palette {
            ui.disable();
        }
        if self.search_open {
            self.search_pane(ui, view);
            return;
        }
        ui.horizontal(|ui| {
            ui.strong("Folders");
            ui.menu_button("Locations", |ui| {
                for (label, action) in [
                    ("Home", Action::Home),
                    ("Parent Folder", Action::Parent),
                    ("Refresh", Action::Refresh),
                ] {
                    if ui.button(label).clicked() {
                        self.command(action, ui.ctx());
                        ui.close();
                    }
                }
            });
            if icon_button(ui, Symbol::Up, "Parent Folder", false).clicked() {
                self.navigator.up();
            }
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(crate::paths::display(&self.navigator.current))
                    .small()
                    .weak(),
            )
            .truncate(),
        )
        .on_hover_text(crate::paths::display(&self.navigator.current));
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Filter this folder")
                .desired_width(f32::INFINITY),
        );
        if field.has_focus() {
            self.navigator.focused = false;
        }
        self.navigator.filter = self.filter.clone();
        self.navigator.show_hidden = self.settings.show_hidden;
        self.navigator.row_height = appearance::Metrics::for_density(if self.settings.compact {
            appearance::Density::Compact
        } else {
            appearance::Density::Comfortable
        })
        .row_height;
        let action = self.navigator.show(ui, !self.choosing && !self.palette);
        if let Some((path, focus)) = action.open {
            self.preview(path);
            if focus {
                view.preview = true;
            }
        }
        if action.edit && self.preview.is_some() {
            view.preview = true;
        }
    }

    fn preview_pane(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(preview) = &self.preview {
            egui::Panel::top("picker_preview_header")
                .frame(appearance::toolbar_frame(ui))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong(crate::name(&preview.disk.path));
                        ui.weak("Read-only");
                        if ui.selectable_label(self.read, "Read").clicked() {
                            self.read = true;
                            self.markdown.refresh();
                        }
                        if ui.selectable_label(!self.read, "Source").clicked() {
                            self.read = false;
                        }
                    });
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(crate::paths::display(&preview.disk.path))
                                .small()
                                .weak(),
                        )
                        .truncate(),
                    )
                    .on_hover_text(crate::paths::display(&preview.disk.path));
                });
            if self.palette || egui::Popup::is_any_open(ui.ctx()) {
                return;
            }
            egui::Frame::new().inner_margin(20.).show(ui,|ui| {
                if self.read {
                    self.markdown.request(1,preview.generation,&preview.text,&preview.disk.path);
                    if self.markdown.render(ui,1,preview.generation,&preview.disk.path).is_some(){self.status="Links do not change a picker selection; navigate or search explicitly.".into();}
                } else {
                    let mut settings=self.settings.clone();settings.syntax=false;
                    if let Err(error)=self.native.show(ui,frame,native_editor::Source{id:1,text:&preview.text,generation:preview.generation,metadata:&preview.metadata,highlight:None,read_only:true},&settings,false){self.status=error;}
                }
            });
        } else {
            egui::Frame::new().inner_margin(28.).show(ui, |ui| {
                ui.add_space(30.);
                appearance::section(
                    ui,
                    "Preview before choosing",
                    "Select a file to inspect it. Your selection is returned only when you choose.",
                );
                ui.weak("The preview is read-only.");
            });
        }
    }

    fn palette_dialog(&mut self, ctx: &egui::Context, view: &Presentation) {
        if !self.palette {
            return;
        }
        let mut action = None;
        egui::Modal::new(egui::Id::unique("picker_commands")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 48.).clamp(240., 540.));
            ui.heading("Commands");
            let query = ui.add(
                egui::TextEdit::singleline(&mut self.palette_query)
                    .hint_text("Find a command")
                    .desired_width(f32::INFINITY),
            );
            if !view.was_palette {
                query.request_focus();
            }
            let filter = self.palette_query.to_lowercase();
            let mut first = None;
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 200.).max(80.))
                .show(ui, |ui| {
                    for command in commands::COMMANDS
                        .iter()
                        .filter(|c| allowed(c.action) && c.label.to_lowercase().contains(&filter))
                    {
                        first.get_or_insert(command.action);
                        if ui
                            .add(
                                egui::Button::new((command.label, egui::Atom::grow()))
                                    .shortcut_text(command.shortcut)
                                    .frame_when_inactive(false)
                                    .min_size(egui::vec2(ui.available_width(), 28.)),
                            )
                            .clicked()
                        {
                            action = Some(command.action);
                        }
                    }
                });
            if query.has_focus()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                action = first;
            }
            if ui.button("Close").clicked() {
                self.palette = false;
            }
        });
        if let Some(action) = action {
            self.palette = false;
            self.command(action, ctx);
        }
    }

    pub(super) fn render(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let key = egui::Id::unique("picker_presentation");
        let mut view = ctx
            .data(|d| d.get_temp::<Presentation>(key))
            .unwrap_or_default();
        self.native.begin();
        self.native.set_bindings(self.bindings.clone());
        self.native.set_available(
            &commands::COMMANDS
                .iter()
                .filter(|c| {
                    allowed(c.action)
                        && (!self.choosing || c.action == Action::Cancel)
                        && !self.palette
                })
                .map(|c| c.action)
                .collect::<Vec<_>>(),
        );
        for action in self.native.commands() {
            if !self.palette {
                self.command(action, &ctx);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            if self.palette {
                self.palette = false;
            } else if self.search_open {
                self.search_open = false;
                self.search_id = self.search.cancel();
            } else if view.path {
                view.path = false;
            } else {
                self.cancel(&ctx);
            }
        }
        if !self.palette {
            for action in commands::consume(&ctx, &self.bindings) {
                self.command(action, &ctx);
            }
        }
        if self.search_open && !view.was_search {
            view.preview = false;
            self.native.focus_host();
            self.navigator.focused = false;
        }
        if self.palette && !view.was_palette {
            self.native.focus_host();
            self.navigator.focused = false;
        }
        let narrow = ui.available_width() < 760.;
        egui::Panel::top("picker_toolbar")
            .frame(appearance::toolbar_frame(ui))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(76. / ctx.zoom_factor());
                    ui.strong(if self.directory {
                        "Choose a folder"
                    } else {
                        "Choose a file"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.menu_button("More", |ui| {
                            if ui.button("Commands…").clicked() {
                                self.palette = true;
                                ui.close();
                            }
                            if ui.button("Enter Path…").clicked() {
                                view.path = true;
                                self.path = self
                                    .navigator
                                    .current
                                    .to_str()
                                    .unwrap_or_default()
                                    .to_owned();
                                self.native.focus_host();
                                ui.close();
                            }
                        });
                        if icon_button(ui, Symbol::Search, "Search", self.search_open).clicked() {
                            self.search_open = !self.search_open;
                            if self.search_open {
                                view.preview = false;
                                self.native.focus_host();
                            }
                        }
                    });
                });
                if view.path {
                    ui.horizontal(|ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.path)
                                .hint_text("Folder or file path")
                                .desired_width((ui.available_width() - 90.).max(90.)),
                        );
                        if ui.button("Go").clicked()
                            || (response.has_focus()
                                && ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                                }))
                        {
                            let path = if self.path.starts_with("~/") {
                                std::env::home_dir()
                                    .unwrap_or_else(|| PathBuf::from("/"))
                                    .join(&self.path[2..])
                            } else {
                                self.navigator.current.join(&self.path)
                            };
                            self.io.send(Job::Visit(path));
                            view.path = false;
                        }
                        if icon_button(ui, Symbol::Close, "Close Path", false).clicked() {
                            view.path = false;
                        }
                    });
                }
                if narrow {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut view.preview, false, "Navigator");
                        ui.selectable_value(&mut view.preview, true, "Preview");
                    });
                }
            });
        egui::Panel::bottom("picker_choice")
            .frame(appearance::toolbar_frame(ui))
            .show(ui, |ui| {
                if !self.status.is_empty() {
                    ui.add(egui::Label::new(egui::RichText::new(&self.status).small()).truncate())
                        .on_hover_text(&self.status);
                }
                let candidate = if self.directory {
                    Some(&self.navigator.current)
                } else {
                    self.candidate.as_ref()
                };
                if let Some(path) = candidate {
                    ui.add(egui::Label::new(crate::paths::display(path)).truncate())
                        .on_hover_text(crate::paths::display(path));
                }
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.cancel(&ctx);
                    }
                    let enabled = !self.choosing
                        && (self.directory
                            || self.candidate.is_some()
                            || self.navigator.selection().is_some());
                    if ui
                        .add_enabled_ui(enabled, |ui| {
                            appearance::primary_button(
                                ui,
                                if self.directory {
                                    "Choose Current Folder"
                                } else {
                                    "Choose File"
                                },
                            )
                        })
                        .inner
                        .clicked()
                    {
                        self.choose();
                    }
                });
            });
        self.palette_dialog(&ctx, &view);
        if !narrow {
            egui::Panel::left("picker_folders")
                .default_size(320.)
                .min_size(220.)
                .max_size((ui.available_width() - 320.).max(220.))
                .frame(
                    egui::Frame::new()
                        .fill(appearance::palette(ui).sidebar)
                        .inner_margin(12.),
                )
                .show(ui, |ui| self.navigation_pane(ui, &mut view));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(if narrow && !view.preview {
                appearance::palette(ui).sidebar
            } else {
                appearance::palette(ui).canvas
            }))
            .show(ui, |ui| {
                if narrow && !view.preview {
                    egui::Frame::new()
                        .inner_margin(12.)
                        .show(ui, |ui| self.navigation_pane(ui, &mut view));
                } else {
                    self.preview_pane(ui, frame);
                }
            });
        self.native.finish();
        self.flush();
        view.was_search = self.search_open;
        view.was_palette = self.palette;
        ctx.data_mut(|d| d.insert_temp(key, view));
    }
}
