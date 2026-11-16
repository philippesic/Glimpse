use glimpse::commands;
use gpui::Modifiers;

pub fn shortcut(expected: &commands::Modifiers, actual: &Modifiers) -> bool {
    let primary = if cfg!(target_os = "macos") {
        actual.platform
    } else {
        actual.control && !actual.platform
    };
    primary == expected.command
        && (!cfg!(target_os = "macos") || actual.control == expected.ctrl)
        && actual.shift == expected.shift
        && actual.alt == expected.alt
}

pub fn shortcut_label(value: &str) -> String {
    if cfg!(target_os = "macos") {
        value.replace("Cmd", "⌘").replace("Alt", "Option")
    } else {
        value.replace("Cmd", "Ctrl")
    }
}

#[cfg(target_os = "macos")]
pub const MONOSPACE_FAMILIES: &[&str] = &["Menlo", "SF Mono", "Monaco"];
#[cfg(target_os = "windows")]
pub const MONOSPACE_FAMILIES: &[&str] = &["Cascadia Mono", "Consolas", "Courier New"];
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const MONOSPACE_FAMILIES: &[&str] = &[
    "Adwaita Mono",
    "Source Code Pro",
    "Liberation Mono",
    "DejaVu Sans Mono",
    "Noto Sans Mono",
];
