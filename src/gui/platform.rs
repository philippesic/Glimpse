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
