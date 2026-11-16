use glimpse::commands;
use gpui::Modifiers;

pub(super) fn open_path(path: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let opener = "xdg-open";
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

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

pub(super) fn configure_allocations() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    for (option, variable) in [
        (libc::M_MMAP_THRESHOLD, "MALLOC_MMAP_THRESHOLD_"),
        (libc::M_TRIM_THRESHOLD, "MALLOC_TRIM_THRESHOLD_"),
    ] {
        if std::env::var_os(variable).is_none() {
            unsafe { libc::mallopt(option, 1024 * 1024) };
        }
    }
}
