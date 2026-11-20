use super::{Shell, labels};
use glimpse::{commands, paths};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, IconName, Side, Sizable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(super) fn command_item(
    shell: WeakEntity<Shell>,
    id: &'static str,
    label: String,
    disabled: bool,
) -> PopupMenuItem {
    PopupMenuItem::new(label)
        .disabled(disabled)
        .on_click(move |_, window, cx| {
            let shell = shell.clone();
            window.defer(cx, move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.command(id, window, cx));
            });
        })
}

fn location_item(shell: WeakEntity<Shell>, path: PathBuf) -> PopupMenuItem {
    location_item_labelled(
        shell,
        if let Some(parent) = path.parent() {
            format!("{} — {}", paths::name(&path), paths::display(parent))
        } else {
            paths::display(&path)
        },
        path,
    )
}

/// Mount points are identified by their full path rather than a name and parent.
fn mount_item(shell: WeakEntity<Shell>, path: PathBuf) -> PopupMenuItem {
    location_item_labelled(shell, paths::display(&path), path)
}

fn location_item_labelled(shell: WeakEntity<Shell>, label: String, path: PathBuf) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        let shell = shell.clone();
        let path = path.clone();
        window.defer(cx, move |_, cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.open(path, false, None);
                cx.notify();
            });
        });
    })
}

fn shortcut(settings: &glimpse::storage::Settings, id: &str) -> Option<SharedString> {
    let command = commands::COMMANDS.iter().find(|command| command.id == id)?;
    let key = settings
        .keymaps
        .get(id)
        .map(String::as_str)
        .unwrap_or(command.shortcut);
    let key = if key.is_empty() {
        commands::editing_hint(id).unwrap_or_default()
    } else {
        key
    };
    (!key.is_empty()).then(|| super::platform::shortcut_label(key).into())
}

fn add_commands(
    mut menu: PopupMenu,
    shell: &WeakEntity<Shell>,
    settings: &glimpse::storage::Settings,
    disabled: &std::collections::HashMap<&'static str, Option<&'static str>>,
    ids: &[&'static str],
    navigator_visible: bool,
) -> PopupMenu {
    for id in ids {
        if *id == "-" {
            menu = menu.separator();
            continue;
        }
        let Some(command) = commands::COMMANDS.iter().find(|command| command.id == *id) else {
            continue;
        };
        let label = match command.id {
            "toggle-navigator" if navigator_visible => "Hide Navigator",
            "toggle-navigator" => "Show Navigator",
            _ => command.label,
        };
        let reason = disabled.get(id).copied().flatten();
        let accessible_label =
            reason.map_or_else(|| label.to_owned(), |reason| format!("{label} · {reason}"));
        let item = command_item(
            shell.clone(),
            command.id,
            accessible_label,
            disabled.contains_key(id),
        )
        .when_some(reason, |item, reason| {
            item.display_label(label).detail(reason)
        })
        .when_some(shortcut(settings, id), |item, hint| {
            item.shortcut_hint(hint)
        });
        menu = menu.item(item);
    }
    menu
}

impl Shell {
    fn disabled_commands(&self) -> std::collections::HashMap<&'static str, Option<&'static str>> {
        commands::COMMANDS
            .iter()
            .filter(|command| !self.command_enabled(command.id))
            .map(|command| {
                let reason =
                    (command.id == "trash" && self.trash_needs_selection && !self.request.pick)
                        .then_some("Select an item first");
                (command.id, reason)
            })
            .collect()
    }

    pub(super) fn document_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let navigator_visible = self.navigator_visible;
        let shell = cx.entity().downgrade();
        let settings = self.settings.clone();
        let disabled = self.disabled_commands();
        Button::new("document-menu")
            .icon(IconName::Ellipsis)
            .tooltip("More Document Actions")
            .ghost()
            .text_color(cx.theme().muted_foreground)
            .small()
            .dropdown_menu_with_anchor(Corner::TopRight, move |menu, window, _| {
                let menu = menu
                    .accessible_name("More Document Actions")
                    .scrollable(true)
                    .max_h((window.viewport_size().height - px(96.)).max(px(100.)));
                add_commands(
                    menu,
                    &shell,
                    &settings,
                    &disabled,
                    &[
                        "new",
                        "save-as",
                        "save-copy",
                        "-",
                        "find",
                        "line",
                        "wrap",
                        "-",
                        "reload",
                        "reveal",
                        "copy-path",
                        "copy-relative",
                        "external",
                        "-",
                        "close",
                    ],
                    navigator_visible,
                )
            })
    }

    pub(super) fn conflict_menu(&self, warning: bool, cx: &Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let disabled = self.disabled_commands();
        Button::new("conflict-actions")
            .with_focus_handle(self.conflict_focus.clone())
            .map(|button| {
                if warning {
                    button
                        .icon(IconName::TriangleAlert)
                        .tooltip("Resolve change on disk")
                        .warning()
                } else {
                    button.label("Resolve").dropdown_caret(true)
                }
            })
            .accessibility_description("Changed on disk; choose how to resolve it")
            .small()
            .ghost()
            .dropdown_menu(move |menu, _, _| {
                let item = |id: &'static str, label: &str| {
                    command_item(shell.clone(), id, label.into(), disabled.contains_key(id))
                };
                menu.accessible_name("Resolve change on disk")
                    .item(item("compare", "Compare"))
                    .item(item("keep-editing", "Keep Editing"))
                    .separator()
                    .item(item("reload", "Reload Disk Version"))
                    .item(item("save-copy", "Save Copy…"))
                    .item(item("overwrite", "Overwrite Disk Version…"))
            })
    }

    pub(super) fn search_scope_menu(
        &self,
        label: String,
        indexed: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let pinned = self.scope.is_some();
        let tooltip = paths::display(self.effective_scope());
        let disabled: std::collections::HashSet<_> = [
            "search-here",
            "scope",
            "go-scope",
            "index-folder",
            "remove-index",
            "search-details",
        ]
        .into_iter()
        .filter(|id| !self.command_enabled(id))
        .collect();
        Button::new("search-scope-heading")
            .label(label)
            .min_w_0()
            .max_w_full()
            .flex_shrink()
            .accessibility_description(format!("Search folder {tooltip}"))
            .tooltip(tooltip)
            .small()
            .compact()
            .ghost()
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let item = |id: &'static str, label: &str| {
                    command_item(shell.clone(), id, label.into(), disabled.contains(id))
                };
                menu.accessible_name("Search folder")
                    .item(item("search-here", "Search This Folder"))
                    .item(item(
                        "scope",
                        if pinned {
                            "Unpin Search Folder"
                        } else {
                            "Pin Search Folder Here"
                        },
                    ))
                    .item(item("go-scope", "Go to Search Folder"))
                    .separator()
                    .item(item(
                        "index-folder",
                        if indexed {
                            "Rebuild Filename Index"
                        } else {
                            "Index This Folder"
                        },
                    ))
                    .when(indexed, |menu| {
                        menu.item(item("remove-index", "Remove Filename Index"))
                    })
                    .separator()
                    .item(item("search-details", "Search Scope and Coverage…"))
            })
    }

    pub(super) fn open_documents_menu(
        &self,
        requested: bool,
        footer_height: Pixels,
        icon_only: bool,
        parents: &HashMap<PathBuf, paths::ParentHint>,
        filenames: &HashMap<String, paths::LabelCue>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let items = self
            .documents
            .iter()
            .map(|document| {
                let title = paths::tab_label(
                    &document.disk.path,
                    self.documents.iter().map(|doc| doc.disk.path.as_path()),
                );
                let current = Some(document.id) == self.active;
                let dirty = document.dirty(cx);
                let status = match (dirty, document.preview, current) {
                    (true, _, true) => Some("Unsaved · Current"),
                    (true, _, false) => Some("Unsaved"),
                    (false, true, true) => Some("Preview · Current"),
                    (false, true, false) => Some("Preview"),
                    (false, false, true) => Some("Current"),
                    (false, false, false) => None,
                };
                let file = paths::name(&document.disk.path);
                let parent = parents.get(&document.disk.path).cloned();
                let filename = filenames.get(&file).cloned();
                (document.id, title, status, current, file, parent, filename)
            })
            .collect::<Vec<_>>();
        Button::new("open-documents-menu")
            .map(|button| {
                if icon_only {
                    button
                        .icon(IconName::ChevronsUpDown)
                        .tooltip("Open Documents")
                } else {
                    button
                        .label("Open Documents")
                        .display_label("Documents")
                        .dropdown_caret(true)
                }
            })
            .small()
            .compact()
            .ghost()
            .text_color(cx.theme().muted_foreground)
            .flex_shrink_0()
            .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, window, _| {
                let viewport = window.viewport_size();
                let rem = window.rem_size();
                let measure = |text: &SharedString| Self::path_label_width(window, text);
                let natural = items
                    .iter()
                    .map(|(_, title, ..)| measure(&title.clone().into()))
                    .max()
                    .unwrap_or_default();
                let width = (natural + rem * 4.5)
                    .max(rem * 14.)
                    .min(rem * 26.)
                    .min(viewport.width - px(24.));
                let room = width - rem * 4.5;
                menu = menu
                    .accessible_name("Open Documents")
                    .scrollable(true)
                    .check_side(Side::Right)
                    .min_w(width)
                    .max_w(width)
                    .max_h((viewport.height - px(96.)).max(px(100.)));
                for (id, title, status, current, file, parent, filename) in &items {
                    let id = *id;
                    let shell = shell.clone();
                    let caption = match parent {
                        Some(folder) => labels::document(file, "", folder, " — ", room, &measure),
                        None => labels::filename(file, "", filename.as_ref(), room, &measure),
                    };
                    let visible_status = status
                        .map(|status| status.trim_end_matches("Current").trim_end_matches(" · "));
                    let item = match status {
                        Some(status) => PopupMenuItem::new(format!("{title} · {status}"))
                            .display_label(caption)
                            .when_some(
                                visible_status.filter(|status| !status.is_empty()),
                                |item, status| item.detail(status),
                            ),
                        None => PopupMenuItem::new(title.clone()).display_label(caption),
                    };
                    menu = menu.item(item.checked(*current).on_click(move |_, window, cx| {
                        let shell = shell.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.select_tab(id, false, window, cx);
                            });
                        });
                    }));
                }
                if let Some(index) = items.iter().position(|(_, _, _, current, ..)| *current) {
                    menu.highlight(index)
                } else {
                    menu
                }
            })
            .request_open(requested)
            .keep_clear_of_bottom(footer_height)
    }

    pub(super) fn locations_menu(&self, icon_only: bool, cx: &Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let pins = self.settings.pins.clone();
        let recents = self.settings.recents.clone();
        let frequent = self.settings.frequent_locations();
        let volumes = self.volumes.clone();
        let pinned = pins.contains(&self.current);
        let busy = self.settings_write.is_some() || self.settings_reloading || self.request.pick;
        Button::new("locations-menu")
            .map(|button| {
                if icon_only {
                    button.icon(IconName::Star).tooltip("Locations")
                } else {
                    button.label("Locations").dropdown_caret(true)
                }
            })
            .small()
            .ghost()
            .text_color(cx.theme().muted_foreground)
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu
                    .accessible_name("Locations")
                    .scrollable(true)
                    .max_w((window.viewport_size().width - px(16.)).min(px(620.)))
                    .max_h((window.viewport_size().height - px(96.)).max(px(100.)))
                    .item(command_item(shell.clone(), "home", "Home".into(), false))
                    .item(command_item(
                        shell.clone(),
                        "pin",
                        if pinned {
                            "Unpin Current Folder"
                        } else {
                            "Pin Current Folder"
                        }
                        .into(),
                        busy,
                    ));
                if !pins.is_empty() {
                    menu = menu.separator().label("Pinned");
                    for path in &pins {
                        menu = menu.item(location_item(shell.clone(), path.clone()));
                    }
                }
                if !frequent.is_empty() {
                    menu = menu.separator().label("Frequent");
                    for path in frequent.iter().take(8) {
                        menu = menu.item(location_item(shell.clone(), path.clone()));
                    }
                }
                if !recents.is_empty() {
                    menu = menu.separator().label("Recent");
                    for path in recents.iter().take(20) {
                        menu = menu.item(location_item(shell.clone(), path.clone()));
                    }
                }
                menu = menu.separator().label("Mounted locations");
                if let Some(volumes) = &volumes {
                    for path in &volumes.paths {
                        menu = menu.item(mount_item(shell.clone(), path.clone()));
                    }
                    if let Some(note) = &volumes.note {
                        menu = menu.label(note.clone());
                    }
                } else {
                    menu = menu.label("Loading; reopen this menu when ready");
                }
                menu.item(command_item(
                    shell.clone(),
                    "refresh-volumes",
                    "Refresh Mounted Folders".into(),
                    false,
                ))
            })
    }

    pub(super) fn main_menu(&self, requested: bool, cx: &Context<Self>) -> impl IntoElement {
        let navigator_visible = self.navigator_visible;
        let shell = cx.entity().downgrade();
        let settings = self.settings.clone();
        let disabled = self.disabled_commands();
        let request = self.request.wait || self.request.pick;
        Button::new("main-menu")
            .icon(IconName::Menu)
            .tooltip("Menu")
            .ghost()
            .text_color(cx.theme().muted_foreground)
            .small()
            .dropdown_menu_with_anchor(Corner::TopRight, move |menu, window, cx| {
                let groups: [(&str, &[&'static str]); 5] = [
                    (
                        "File",
                        &[
                            "new",
                            "open",
                            "folder",
                            "-",
                            "save",
                            "save-as",
                            "save-copy",
                            "keep-tab",
                            "-",
                            "named-file",
                            "new-folder",
                            "rename",
                            "trash",
                            "-",
                            "close",
                        ],
                    ),
                    (
                        "Edit",
                        &[
                            "undo",
                            "redo",
                            "-",
                            "cut",
                            "copy",
                            "paste",
                            "select-all",
                            "-",
                            "find",
                            "line",
                            "-",
                            "indent",
                            "outdent",
                            "wrap",
                        ],
                    ),
                    (
                        "Go",
                        &[
                            "path",
                            "open-documents",
                            "navigator",
                            "-",
                            "back",
                            "forward",
                            "parent",
                            "home",
                            "refresh",
                            "-",
                            "pin",
                            "reveal",
                            "copy-path",
                            "copy-relative",
                            "external",
                            "finder",
                        ],
                    ),
                    (
                        "Search",
                        &[
                            "filter",
                            "find-files",
                            "find-text",
                            "-",
                            "search-here",
                            "scope",
                            "go-scope",
                            "index-folder",
                            "remove-index",
                            "search-details",
                            "stop-search",
                        ],
                    ),
                    ("View", &["toggle-navigator", "read", "recovery"]),
                ];
                let height = (window.viewport_size().height - px(16.)).max(px(100.));
                let scale = cx.theme().font_size / px(16.);
                let viewport = window.viewport_size();
                let flat = viewport.width < px(540.) * scale || viewport.height < px(600.) * scale;
                let mut menu = menu
                    .accessible_name("Menu")
                    .scrollable(flat)
                    .min_w(px(220.))
                    .max_h(height);
                // Submenus need room for two menus side by side and for the longest
                // submenu; otherwise one scrolling list with named sections keeps every
                // command reachable without overlap.
                for (label, ids) in groups {
                    if flat {
                        menu = add_commands(
                            menu.label(label),
                            &shell,
                            &settings,
                            &disabled,
                            ids,
                            navigator_visible,
                        )
                        .separator();
                        continue;
                    }
                    let shell = shell.clone();
                    let settings = settings.clone();
                    let disabled = disabled.clone();
                    menu = menu.submenu(label, window, cx, move |submenu, _, _| {
                        add_commands(
                            submenu.accessible_name(label).min_w(px(260.)),
                            &shell,
                            &settings,
                            &disabled,
                            ids,
                            navigator_visible,
                        )
                    });
                }
                let cli = ["finish", "abort", "choose", "cancel"];
                if request && flat {
                    menu = add_commands(
                        menu.label("CLI Request"),
                        &shell,
                        &settings,
                        &disabled,
                        &cli,
                        navigator_visible,
                    )
                    .separator();
                } else if request {
                    let shell = shell.clone();
                    let settings = settings.clone();
                    let disabled = disabled.clone();
                    menu = menu.submenu("CLI Request", window, cx, move |submenu, _, _| {
                        add_commands(
                            submenu.min_w(px(260.)),
                            &shell,
                            &settings,
                            &disabled,
                            &cli,
                            navigator_visible,
                        )
                    });
                }
                add_commands(
                    if flat { menu } else { menu.separator() },
                    &shell,
                    &settings,
                    &disabled,
                    &["commands", "settings", "-", "quit"],
                    navigator_visible,
                )
            })
            .request_open(requested)
    }
}

pub(super) struct NavigatorMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    corner: Corner,
    _dismiss: Subscription,
}

impl Shell {
    pub(super) fn navigator_menu_from_keyboard(
        &mut self,
        visible: &[usize],
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = index else {
            return;
        };
        let Some(scroll) = self.scrolls.get(&self.current) else {
            return;
        };
        let scroll = scroll.0.borrow();
        let bounds = scroll.base_handle.bounds();
        let height = self.navigation_row_height();
        let top = bounds.top() + scroll.base_handle.offset().y + height * index;
        let margin = px(8.);
        let below = (top + height).clamp(bounds.top(), bounds.bottom());
        let above = top.clamp(bounds.top(), bounds.bottom());
        let below_space = window.viewport_size().height - below - margin;
        let above_space = above - margin;
        let menu_height = height * 5. + px(32.);
        let (y, corner, room) = if below_space >= menu_height || below_space >= above_space {
            (below, Corner::TopLeft, below_space)
        } else {
            (above, Corner::BottomLeft, above_space)
        };
        let position = point(bounds.left() + px(12. * self.settings.ui_scale), y);
        drop(scroll);
        let path = self.entries[visible[index]].path.clone();
        self.show_navigator_menu(&path, (position, corner, room), false, window, cx);
    }

    pub(super) fn navigator_menu_from_pointer(
        &mut self,
        path: PathBuf,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.decision.is_some() || self.preferences.is_some() {
            return;
        }
        if self.trash_needs_selection {
            self.selection = Some(path.clone());
        } else {
            self.select_navigation(path.clone());
        }
        let room = window.viewport_size().height - px(24.);
        self.show_navigator_menu(&path, (position, Corner::TopLeft, room), true, window, cx);
    }

    fn show_navigator_menu(
        &mut self,
        path: &Path,
        anchor: (Point<Pixels>, Corner, Pixels),
        from_pointer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.decision.is_some() || self.preferences.is_some() {
            return;
        }
        let (position, corner, room) = anchor;
        let title = format!("Actions for {}", paths::name(path));
        window.focus(&self.nav_focus);
        let shell = cx.weak_entity();
        let disabled = self.disabled_commands();
        let menu = PopupMenu::build(window, cx, |menu, window, cx| {
            let blocker = self.file_action_blocker(path, false);
            let item_height = (px(28.) * (cx.theme().font_size / px(16.))).max(px(24.));
            let detail_count =
                usize::from(blocker.is_none() && !from_pointer && self.trash_needs_selection);
            let total_height = item_height * (4 + usize::from(blocker.is_some()))
                + window.rem_size() * (detail_count as f32 + 1.)
                + px(2.);
            let mut menu = menu
                .accessible_name(title)
                .action_context(self.nav_focus.clone())
                .scrollable(total_height > room)
                .max_h(room);
            if let Some(reason) = blocker {
                menu = menu.label(reason);
            }
            let actions = [
                ("rename", "Rename…", "F2"),
                ("trash", "Move to Trash", "Delete"),
                ("copy-path", "Copy Path", ""),
                ("finder", "Reveal in File Manager", ""),
            ];
            let mut first = None;
            for (index, (id, label, key)) in actions.into_iter().enumerate() {
                let direct_trash = from_pointer && id == "trash";
                let enabled = if direct_trash {
                    blocker.is_none() && !self.request.pick
                } else {
                    !disabled.contains_key(id)
                };
                if enabled && first.is_none() {
                    first = Some(index + usize::from(blocker.is_some()));
                }
                let reason = disabled
                    .get(id)
                    .copied()
                    .flatten()
                    .filter(|_| blocker.is_none() && !direct_trash);
                let accessible_label =
                    reason.map_or_else(|| label.to_owned(), |reason| format!("{label} · {reason}"));
                let item = if direct_trash {
                    let path = path.to_owned();
                    let shell = shell.clone();
                    PopupMenuItem::new(accessible_label)
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| {
                            let path = path.clone();
                            let shell = shell.clone();
                            window.defer(cx, move |_, cx| {
                                let _ = shell.update(cx, |shell, cx| {
                                    shell.move_to_trash(path, false);
                                    cx.notify();
                                });
                            });
                        })
                } else {
                    command_item(shell.clone(), id, accessible_label, !enabled)
                };
                menu = menu.item(
                    item.when_some(reason, |item, reason| {
                        item.display_label(label).detail(reason)
                    })
                    .when(
                        !(key.is_empty() || direct_trash && self.trash_needs_selection),
                        |item| item.shortcut_hint(key),
                    ),
                );
            }
            menu.when_some(first, |menu, index| menu.highlight(index))
        });
        let dismiss = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, _, cx| {
            this.navigator_menu = None;
            cx.notify();
        });
        window.focus(&menu.focus_handle(cx));
        self.navigator_menu = Some(NavigatorMenu {
            menu,
            position,
            corner,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    pub(super) fn navigator_menu_view(&self) -> Option<impl IntoElement> {
        self.navigator_menu.as_ref().map(|menu| {
            deferred(
                div().absolute().inset_0().occlude().child(
                    anchored()
                        .position(menu.position)
                        .anchor(menu.corner)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu.menu.clone()),
                ),
            )
            .with_priority(1)
        })
    }
}
