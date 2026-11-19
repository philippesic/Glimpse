use super::Shell;
use glimpse::{commands, paths};
use gpui::{prelude::*, *};
use gpui_component::{
    Side, Sizable,
    button::Button,
    menu::{DropdownMenu, PopupMenuItem},
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
    PopupMenuItem::new(if let Some(parent) = path.parent() {
        format!("{} — {}", paths::name(&path), paths::display(parent))
    } else {
        paths::display(&path)
    })
    .on_click(move |_, window, cx| {
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

impl Shell {
    pub(super) fn open_documents_menu(
        &self,
        requested: bool,
        footer_height: Pixels,
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
            .label("Open Documents")
            .display_label("Documents")
            .dropdown_caret(true)
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

    pub(super) fn locations_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let pins = self.settings.pins.clone();
        let recents = self.settings.recents.clone();
        let frequent = self.settings.frequent_locations();
        let volumes = self.volumes.clone();
        let pinned = pins.contains(&self.current);
        let busy = self.settings_write.is_some() || self.settings_reloading || self.request.pick;
        Button::new("locations-menu")
            .label("Locations")
            .small()
            .compact()
            .outline()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu
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
                        menu = menu.item(location_item(shell.clone(), path.clone()));
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

    pub(super) fn main_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let shell = cx.entity().downgrade();
        let settings = self.settings.clone();
        let disabled: std::collections::HashSet<_> = commands::COMMANDS
            .iter()
            .filter(|command| !self.command_enabled(command.id))
            .map(|command| command.id)
            .collect();
        Button::new("main-menu")
            .label("Menu")
            .small()
            .compact()
            .outline()
            .dropdown_caret(true)
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu
                    .scrollable(true)
                    .max_w((window.viewport_size().width - px(16.)).min(px(620.)))
                    .max_h((window.viewport_size().height - px(96.)).max(px(100.)));
                for (group, ids) in [
                    (
                        "File",
                        &[
                            "new",
                            "open",
                            "folder",
                            "keep-tab",
                            "save",
                            "save-as",
                            "save-copy",
                            "named-file",
                            "new-folder",
                            "rename",
                            "trash",
                            "close",
                            "quit",
                        ][..],
                    ),
                    ("Edit", &["find", "indent", "outdent"][..]),
                    (
                        "Navigate",
                        &[
                            "path",
                            "open-documents",
                            "back",
                            "forward",
                            "parent",
                            "home",
                            "refresh",
                            "pin",
                            "scope",
                            "go-scope",
                            "reveal",
                            "copy-path",
                            "copy-relative",
                            "external",
                            "finder",
                        ][..],
                    ),
                    (
                        "Search and view",
                        &[
                            "find-files",
                            "find-text",
                            "search-here",
                            "index-folder",
                            "remove-index",
                            "search-details",
                            "stop-search",
                            "focus-results",
                            "read",
                            "wrap",
                            "recovery",
                            "settings",
                            "commands",
                        ][..],
                    ),
                    ("CLI request", &["finish", "abort", "choose", "cancel"][..]),
                ] {
                    menu = menu.label(group);
                    for id in ids {
                        let Some(command) =
                            commands::COMMANDS.iter().find(|command| command.id == *id)
                        else {
                            continue;
                        };
                        let shortcut = settings
                            .keymaps
                            .get(*id)
                            .map(String::as_str)
                            .unwrap_or(command.shortcut);
                        let item = command_item(
                            shell.clone(),
                            command.id,
                            command.label.to_owned(),
                            disabled.contains(id),
                        )
                        .when(!shortcut.is_empty(), |item| {
                            item.shortcut_hint(super::platform::shortcut_label(shortcut))
                        });
                        menu = menu.item(item);
                    }
                    menu = menu.separator();
                }
                menu
            })
    }
}
