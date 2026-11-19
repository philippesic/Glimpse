use super::Shell;
use glimpse::{commands, paths};
use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, IconName, Side, Sizable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
};
use std::path::PathBuf;

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
    disabled: &std::collections::HashSet<&'static str>,
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
        let item = command_item(
            shell.clone(),
            command.id,
            label.to_owned(),
            disabled.contains(id),
        )
        .when_some(shortcut(settings, id), |item, hint| {
            item.shortcut_hint(hint)
        });
        menu = menu.item(item);
    }
    menu
}

impl Shell {
    fn disabled_commands(&self) -> std::collections::HashSet<&'static str> {
        commands::COMMANDS
            .iter()
            .filter(|command| !self.command_enabled(command.id))
            .map(|command| command.id)
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
            .outline()
            .dropdown_menu(move |menu, _, _| {
                let item = |id: &'static str, label: &str| {
                    command_item(shell.clone(), id, label.into(), disabled.contains(id))
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
            .outline()
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
                (document.id, title, status, current)
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
            .outline()
            .h(px(28. * self.settings.ui_scale))
            .flex_shrink_0()
            .dropdown_menu_with_anchor(Corner::TopRight, move |mut menu, window, _| {
                let viewport = window.viewport_size();
                let width = (window.rem_size() * 14.).min(viewport.width - px(24.));
                menu = menu
                    .accessible_name("Open Documents")
                    .scrollable(true)
                    .check_side(Side::Right)
                    .min_w(width)
                    .max_w(width)
                    .max_h((viewport.height - px(96.)).max(px(100.)));
                for (id, title, status, current) in &items {
                    let id = *id;
                    let shell = shell.clone();
                    let item = match status {
                        Some(status) => PopupMenuItem::new(format!("{title} · {status}"))
                            .display_label(title.clone())
                            .detail(*status),
                        None => PopupMenuItem::new(title.clone()),
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
                if let Some(index) = items.iter().position(|(_, _, _, current)| *current) {
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
