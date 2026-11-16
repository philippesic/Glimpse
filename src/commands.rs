#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modifiers {
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Key(String);
impl Key {
    fn from_name(name: &str) -> Option<Self> {
        ((name.len() == 1 && name.chars().all(|c| c.is_ascii_alphanumeric()))
            || matches!(
                name,
                "ArrowLeft"
                    | "ArrowRight"
                    | "ArrowUp"
                    | "ArrowDown"
                    | "Enter"
                    | "Comma"
                    | "Escape"
                    | "Tab"
                    | "Space"
                    | "Backspace"
                    | "Delete"
                    | "Home"
                    | "End"
                    | "PageUp"
                    | "PageDown"
            )
            || name
                .strip_prefix('F')
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=24).contains(&n)))
        .then(|| {
            Self(if name.len() == 1 {
                name.to_ascii_uppercase()
            } else {
                name.to_owned()
            })
        })
    }
    pub fn name(&self) -> &str {
        &self.0
    }
}
use std::collections::{BTreeMap, HashSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    New,
    Open,
    Folder,
    Save,
    SaveAs,
    SaveCopy,
    Close,
    Quit,
    Back,
    Forward,
    Home,
    Parent,
    Refresh,
    Files,
    Text,
    Find,
    Read,
    Settings,
    Recovery,
    Palette,
    Path,
    Pin,
    Scope,
    GoScope,
    SearchHere,
    IndexFolder,
    RemoveIndex,
    SearchDetails,
    StopSearch,
    FocusResults,
    CopyPath,
    CopyRelative,
    Reveal,
    External,
    Finder,
    NewFolder,
    NamedFile,
    Rename,
    Trash,
    Finish,
    Abort,
    Wrap,
    Indent,
    Outdent,
    Choose,
    Cancel,
    RetryCleanup,
    CancelClosing,
    Navigator,
    ToggleNavigator,
    Line,
    Filter,
    KeepPreview,
    RefreshVolumes,
}
pub struct Command {
    pub action: Action,
    pub id: &'static str,
    pub label: &'static str,
    pub shortcut: &'static str,
}
macro_rules! commands{($(($action:ident,$id:literal,$label:literal,$key:literal)),*$(,)?)=>{pub const COMMANDS:&[Command]=&[$(Command{action:Action::$action,id:$id,label:$label,shortcut:$key}),*];};}
commands![
    (New, "new", "New File", "Cmd+N"),
    (Open, "open", "Open File…", "Cmd+O"),
    (Folder, "folder", "Go to Folder…", "Cmd+Shift+O"),
    (Save, "save", "Save", "Cmd+S"),
    (SaveAs, "save-as", "Save As…", "Cmd+Shift+S"),
    (SaveCopy, "save-copy", "Save Copy…", "Cmd+Alt+S"),
    (Close, "close", "Close Document", "Cmd+W"),
    (Quit, "quit", "Quit", "Cmd+Q"),
    (Back, "back", "Back", "Cmd+Alt+ArrowLeft"),
    (Forward, "forward", "Forward", "Cmd+Alt+ArrowRight"),
    (Home, "home", "Home", "Cmd+Shift+H"),
    (Parent, "parent", "Parent Folder", "Cmd+Alt+ArrowUp"),
    (Refresh, "refresh", "Refresh Folder", "Cmd+R"),
    (Files, "find-files", "Find Files in Search Folder", "Cmd+P"),
    (Text, "find-text", "Find Text in Saved Files", "Cmd+Shift+F"),
    (Find, "find", "Find / Replace in Document", "Cmd+F"),
    (Read, "read", "Read / Edit Markdown", ""),
    (Settings, "settings", "Preferences", "Cmd+Comma"),
    (Recovery, "recovery", "Recovery and Saved Versions", ""),
    (Palette, "commands", "Commands", "Cmd+Shift+P"),
    (Path, "path", "Enter Path", "Cmd+L"),
    (Navigator, "navigator", "Focus Navigator", "Cmd+Shift+E"),
    (
        ToggleNavigator,
        "toggle-navigator",
        "Show / Hide Navigator",
        "Cmd+Alt+E"
    ),
    (Line, "line", "Go to Line…", "Cmd+G"),
    (Filter, "filter", "Filter This Folder", ""),
    (KeepPreview, "keep-tab", "Keep Preview Tab", ""),
    (
        RefreshVolumes,
        "refresh-volumes",
        "Refresh Mounted Folders",
        ""
    ),
    (Pin, "pin", "Pin / Unpin Current Folder", ""),
    (Scope, "scope", "Pin / Unpin Search Scope Here", ""),
    (GoScope, "go-scope", "Go to Search Folder", ""),
    (SearchHere, "search-here", "Search This Folder", ""),
    (
        IndexFolder,
        "index-folder",
        "Index / Rebuild Search Folder",
        ""
    ),
    (RemoveIndex, "remove-index", "Remove Filename Index", ""),
    (
        SearchDetails,
        "search-details",
        "Search Scope and Coverage",
        ""
    ),
    (StopSearch, "stop-search", "Stop Recursive Search", ""),
    (FocusResults, "focus-results", "Focus Search Results", ""),
    (CopyPath, "copy-path", "Copy Absolute Path", ""),
    (CopyRelative, "copy-relative", "Copy Relative Path", ""),
    (Reveal, "reveal", "Reveal Document in Navigator", ""),
    (External, "external", "Open Selected Externally", ""),
    (Finder, "finder", "Reveal Selected in File Manager", ""),
    (NewFolder, "new-folder", "New Folder…", "Cmd+Shift+N"),
    (NamedFile, "named-file", "New Named File…", ""),
    (Rename, "rename", "Rename Selected…", ""),
    (Trash, "trash", "Move Selected to Trash…", ""),
    (
        Finish,
        "finish",
        "Finish Editing Requested Files",
        "Cmd+Enter"
    ),
    (Abort, "abort", "Abort CLI Request", ""),
    (Wrap, "wrap", "Toggle Soft Wrap", ""),
    (Indent, "indent", "Indent Selection", ""),
    (Outdent, "outdent", "Unindent Selection", ""),
    (Choose, "choose", "Choose Selection", ""),
    (Cancel, "cancel", "Cancel Active Operation", ""),
    (RetryCleanup, "retry-cleanup", "Retry Recovery Cleanup", ""),
    (
        CancelClosing,
        "cancel-closing",
        "Cancel Closing Documents",
        ""
    ),
];
#[derive(Clone, PartialEq)]
pub struct Binding {
    pub action: Action,
    pub key: Key,
    pub modifiers: Modifiers,
}
pub fn parse(value: &str) -> Result<(Modifiers, Key), String> {
    let mut modifiers = Modifiers::default();
    let mut key = None;
    for part in value.split('+') {
        match part {
            "Cmd" => modifiers.command = true,
            "Shift" => modifiers.shift = true,
            "Alt" => modifiers.alt = true,
            "Ctrl" => modifiers.ctrl = true,
            part => {
                if key.is_some() {
                    return Err(format!("Invalid shortcut: {value}"));
                }
                key = Key::from_name(part);
                if key.is_none() {
                    return Err(format!("Unknown key: {part}"));
                }
            }
        }
    }
    if !cfg!(target_os = "macos") {
        modifiers.ctrl = false;
    }
    let key = key.ok_or_else(|| format!("Missing key: {value}"))?;
    if ["Copy", "Cut", "Paste", "BrowserBack"].contains(&key.name()) {
        return Err("Use a keyboard key name for an app shortcut.".into());
    }
    if !modifiers.command {
        return Err("App shortcuts require Cmd to preserve text input.".into());
    }
    if !modifiers.alt
        && !modifiers.ctrl
        && matches!(
            key.name().to_ascii_uppercase().as_str(),
            "A" | "C" | "V" | "X" | "Z"
        )
    {
        return Err(
            "Select All, clipboard and Undo/Redo shortcuts are reserved for text editing.".into(),
        );
    }
    Ok((modifiers, key))
}
pub fn bindings(overrides: &BTreeMap<String, String>) -> Result<Vec<Binding>, String> {
    for id in overrides.keys() {
        if !COMMANDS.iter().any(|c| c.id == id) {
            return Err(format!("Unknown command: {id}"));
        }
    }
    let mut used = HashSet::new();
    let mut result = vec![];
    for command in COMMANDS {
        let value = overrides
            .get(command.id)
            .map(String::as_str)
            .unwrap_or(command.shortcut);
        if value.is_empty() {
            continue;
        }
        let (modifiers, key) = parse(value)?;
        let signature = format!("{:?}:{:?}", modifiers, key);
        if !used.insert(signature) {
            return Err(format!("Duplicate shortcut: {value}"));
        }
        result.push(Binding {
            action: command.action,
            key,
            modifiers,
        });
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effective_shortcuts_cannot_collide_through_case_or_primary_control_aliases() {
        let mut overrides = BTreeMap::new();
        overrides.insert("new".into(), "Cmd+s".into());
        assert!(bindings(&overrides).is_err());
        if !cfg!(target_os = "macos") {
            overrides.insert("new".into(), "Cmd+Ctrl+S".into());
            assert!(bindings(&overrides).is_err());
            assert!(parse("Cmd+Ctrl+A").is_err());
        }
    }

    #[test]
    fn defaults_and_overrides_preserve_editing_keys() {
        assert!(bindings(&BTreeMap::new()).is_ok());
        let mut map = BTreeMap::new();
        map.insert("save".into(), "Cmd+X".into());
        assert!(bindings(&map).is_err());
        map.insert("save".into(), "Cmd+Shift+O".into());
        assert!(bindings(&map).is_err());
        map.insert("save".into(), "Cmd+Alt+J".into());
        assert!(bindings(&map).is_ok());
        map.insert("typo".into(), "".into());
        assert!(bindings(&map).is_err());
    }
}
