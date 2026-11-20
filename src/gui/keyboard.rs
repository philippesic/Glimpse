use super::*;
use gpui_component::actions::{Cancel, Confirm, SelectDown, SelectUp};

/// The surface that owns the keyboard. The topmost open layer receives every key: an open
/// menu, then a dialog, then an overlay that covers the panes (Commands, path entry, or a panel
/// that takes over the main area), and otherwise the window's regions.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Layer {
    Menu,
    Dialog,
    Overlay(Panel),
    Base,
}

/// One movement model for every list (navigator rows, Commands, search results): arrows move
/// one row, Page keys a page, Home and End to the ends. Returns the new row, if the key moves.
pub(super) fn list_step(
    key: &str,
    index: Option<usize>,
    count: usize,
    page: usize,
) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    let page = page.max(1);
    Some(match (key, index) {
        ("down", None) | ("home", _) | ("pagedown", None) => 0,
        ("up", None) | ("end", _) | ("pageup", None) => last,
        ("down", Some(i)) => (i + 1).min(last),
        ("up", Some(i)) => i.saturating_sub(1),
        ("pagedown", Some(i)) => (i + page).min(last),
        ("pageup", Some(i)) => i.saturating_sub(page),
        _ => return None,
    })
}

fn plain(key: &Keystroke) -> bool {
    !key.modifiers.control && !key.modifiers.platform && !key.modifiers.alt
}

impl Shell {
    pub(super) fn layer(&self, contexts: &[KeyContext]) -> Layer {
        if contexts.iter().any(|context| context.contains("PopupMenu")) {
            Layer::Menu
        } else if self.decision.is_some() || self.preferences.is_some() {
            Layer::Dialog
        } else {
            match self.panel {
                Some(panel @ (Panel::Palette | Panel::Path)) => Layer::Overlay(panel),
                Some(panel) if self.search_ui.takeover => Layer::Overlay(panel),
                _ => Layer::Base,
            }
        }
    }

    pub(super) fn route_key(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = &event.keystroke;
        match self.layer(&event.context_stack) {
            Layer::Menu => Self::menu_key(key, window, cx),
            // Dialogs contain their own focus and handle their keys.
            Layer::Dialog => {}
            Layer::Overlay(panel) => self.overlay_key(panel, key, window, cx),
            Layer::Base => self.base_key(key, window, cx),
        }
    }

    /// Menus take arrows, Home, End, Page keys, Enter and Escape themselves. Tab moves like the
    /// arrows, Space activates, F10 closes every open menu, and no other key reaches anything
    /// behind the menu.
    fn menu_key(key: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if plain(key)
            && !key.modifiers.shift
            && matches!(
                key.key.as_str(),
                "escape"
                    | "enter"
                    | "up"
                    | "down"
                    | "left"
                    | "right"
                    | "home"
                    | "end"
                    | "pageup"
                    | "pagedown"
            )
        {
            return;
        }
        if plain(key) {
            match key.key.as_str() {
                "tab" if key.modifiers.shift => window.dispatch_action(Box::new(SelectUp), cx),
                "tab" => window.dispatch_action(Box::new(SelectDown), cx),
                "space" => {
                    window.dispatch_action(Box::new(Confirm { secondary: false }), cx);
                }
                "f10" => window.dispatch_action(Box::new(Cancel), cx),
                _ => {}
            }
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn overlay_contains_focus(&self, panel: Panel, window: &Window, cx: &App) -> bool {
        self.query.focus_handle(cx).contains_focused(window, cx)
            || (panel != Panel::Path && self.palette.overlay.contains_focused(window, cx))
    }

    /// An overlay owns every key while open. Tab and F6 stay inside it, its own shortcut closes
    /// it, and no command acts on the content it covers.
    fn overlay_key(
        &mut self,
        panel: Panel,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stop = |window: &mut Window, cx: &mut Context<Self>| {
            window.prevent_default();
            cx.stop_propagation();
        };
        if !self.overlay_contains_focus(panel, window, cx) {
            // Focus left the overlay (for example after a click behind it): bring it back
            // rather than letting the key act on the covered content.
            self.query.update(cx, |input, cx| input.focus(window, cx));
            stop(window, cx);
            return;
        }
        let field = self.query.focus_handle(cx).is_focused(window);
        if plain(key) {
            match key.key.as_str() {
                "tab" if panel == Panel::Path && field => {
                    self.complete_path(key.modifiers.shift, window, cx);
                    stop(window, cx);
                    return;
                }
                "tab" if panel == Panel::Path => {
                    self.query.update(cx, |input, cx| input.focus(window, cx));
                    stop(window, cx);
                    return;
                }
                "tab" => {
                    modal::tab(key, &self.palette.overlay, window, cx);
                    return;
                }
                "f6" => {
                    self.overlay_region(panel, field, window, cx);
                    stop(window, cx);
                    return;
                }
                "f10" => {
                    stop(window, cx);
                    return;
                }
                _ => {}
            }
            if field && !key.modifiers.shift && self.field_into_results(panel, key, window, cx) {
                stop(window, cx);
            }
            return;
        }
        if self.opener(key) == Some(panel) {
            self.command("dismiss", window, cx);
            stop(window, cx);
        } else if key.key == "tab" || !field {
            stop(window, cx);
        }
    }

    /// F6 inside an overlay moves between its search field and its results.
    fn overlay_region(
        &mut self,
        panel: Panel,
        field: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let results = match panel {
            Panel::Palette => !self.palette_matches(cx).is_empty(),
            Panel::Files | Panel::Text => !self.hits.is_empty(),
            _ => false,
        };
        if field && results {
            match panel {
                Panel::Palette => self.focus_palette(false, window, cx),
                _ => self.focus_results(window, cx),
            }
        } else {
            self.query.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Down and Up in a search field move into its results, and step path completions.
    fn field_into_results(
        &mut self,
        panel: Panel,
        key: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let down = matches!(key.key.as_str(), "down" | "pagedown");
        let up = matches!(key.key.as_str(), "up" | "pageup");
        if !down && !up {
            return false;
        }
        match panel {
            Panel::Palette if !self.palette_matches(cx).is_empty() => {
                self.focus_palette(up, window, cx);
                true
            }
            Panel::Files | Panel::Text if down && !self.hits.is_empty() => {
                self.focus_results(window, cx);
                true
            }
            Panel::Path if self.path_completions.is_some() => {
                self.complete_path(up, window, cx);
                true
            }
            Panel::Filter if down => self.focus_filtered(window, cx),
            _ => false,
        }
    }

    /// Down from the filter field moves to the first matching row in the navigator.
    pub(super) fn focus_filtered(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let visible = self.visible_rows();
        if visible.is_empty() {
            return false;
        }
        let rows = self.entries.clone();
        let index = visible
            .iter()
            .position(|&i| Some(&rows[i].path) == self.selection.as_ref())
            .unwrap_or(0);
        self.select_navigation(rows[visible[index]].path.clone());
        if let Some(scroll) = self.scrolls.get(&self.current) {
            scroll.scroll_to_item(index, ScrollStrategy::Top);
        }
        window.focus(&self.nav_focus);
        cx.notify();
        true
    }

    /// The panel a shortcut opens, so the same shortcut can close it again.
    fn opener(&self, key: &Keystroke) -> Option<Panel> {
        match self.bound_command(key)? {
            "path" => Some(Panel::Path),
            "filter" => Some(Panel::Filter),
            "find-files" => Some(Panel::Files),
            "find-text" => Some(Panel::Text),
            "commands" => Some(Panel::Palette),
            "recovery" => Some(Panel::Recovery),
            "line" => Some(Panel::Line),
            _ => None,
        }
    }

    pub(super) fn bound_command(&self, key: &Keystroke) -> Option<&'static str> {
        let bindings = glimpse::commands::bindings(&self.settings.keymaps).unwrap_or_default();
        bindings
            .iter()
            .find(|binding| {
                let name = match binding.key.name() {
                    "ArrowLeft" => "left",
                    "ArrowRight" => "right",
                    "ArrowUp" => "up",
                    "ArrowDown" => "down",
                    "Comma" => ",",
                    other => other,
                };
                name.eq_ignore_ascii_case(&key.key)
                    && platform::shortcut(&binding.modifiers, &key.modifiers)
            })
            .and_then(|binding| {
                glimpse::commands::COMMANDS
                    .iter()
                    .find(|command| command.action == binding.action)
            })
            .map(|command| command.id)
    }

    /// The window's regions: F10 opens the menu, F6 moves between regions, Ctrl+Tab between
    /// documents, Down leaves a search field for its results, and shortcuts run commands.
    fn base_key(&mut self, key: &Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let stop = |window: &mut Window, cx: &mut Context<Self>| {
            window.prevent_default();
            cx.stop_propagation();
        };
        if plain(key) {
            match key.key.as_str() {
                "f10" if !key.modifiers.shift => {
                    self.main_menu_requested = true;
                    cx.notify();
                    stop(window, cx);
                    return;
                }
                "f6" => {
                    self.cycle_focus(key.modifiers.shift, window, cx);
                    stop(window, cx);
                    return;
                }
                "menu" | "f10"
                    if (key.key == "f10") == key.modifiers.shift
                        && let Some(document) = self.documents.iter().find(|document| {
                            document.input.focus_handle(cx).is_focused(window)
                        }) =>
                {
                    document
                        .input
                        .update(cx, |input, cx| input.open_context_menu(window, cx));
                    stop(window, cx);
                    return;
                }
                _ => {}
            }
            if let Some(panel) = self.panel
                && !key.modifiers.shift
                && self.query.focus_handle(cx).is_focused(window)
                && self.field_into_results(panel, key, window, cx)
            {
                stop(window, cx);
            }
            return;
        }
        if key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
            && key.key == "tab"
        {
            self.command(
                if key.modifiers.shift {
                    "previous-document"
                } else {
                    "next-document"
                },
                window,
                cx,
            );
            stop(window, cx);
            return;
        }
        if !key.modifiers.control && !key.modifiers.platform {
            return;
        }
        if let Some(command) = self.bound_command(key) {
            self.command(command, window, cx);
            stop(window, cx);
        } else if key.key.eq_ignore_ascii_case("f")
            && platform::shortcut(
                &glimpse::commands::Modifiers {
                    command: true,
                    ..Default::default()
                },
                &key.modifiers,
            )
        {
            stop(window, cx);
        }
    }
}
