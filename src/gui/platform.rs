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
