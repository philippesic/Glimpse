use glimpse::storage::Settings;
use gpui::{App, Window, px};
use gpui_component::{ActiveTheme, Theme, ThemeMode};

pub fn apply(settings: &Settings, window: &mut Window, cx: &mut App) -> Option<String> {
    let names = cx.text_system().all_font_names();
    let configured = names
        .iter()
        .find(|name| name.eq_ignore_ascii_case(&settings.font_family));
    let fallback = super::platform::MONOSPACE_FAMILIES
        .iter()
        .find_map(|family| names.iter().find(|name| name.eq_ignore_ascii_case(family)));
    let family = configured
        .or(fallback)
        .cloned()
        .unwrap_or_else(|| ".SystemUIFont".into());
    let warning=(!settings.font_family.is_empty() && configured.is_none()).then(||format!("Source font ‘{}’ is unavailable; using {family}. Choose an installed font in Preferences.",settings.font_family));
    if std::env::var_os("GLIMPSE_TRACE").is_some() {
        let system = cx.text_system();
        let id = system.resolve_font(&gpui::font(family.clone()));
        eprintln!(
            "GLIMPSE_FONT {:?} {:?} {:?}",
            family,
            system.advance(id, px(settings.font_size), 'i'),
            system.advance(id, px(settings.font_size), 'M')
        );
    }
    if settings.system_theme {
        Theme::sync_system_appearance(Some(window), cx);
    } else {
        Theme::change(
            if settings.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            Some(window),
            cx,
        );
    }
    let theme = Theme::global_mut(cx);
    theme.font_size = px(16. * settings.ui_scale);
    theme.mono_font_size = px(settings.font_size * settings.ui_scale);
    theme.mono_font_family = family.into();
    theme.colors.muted_foreground =
        gpui::rgb(if theme.is_dark() { 0xaab5c5 } else { 0x4d596a }).into();
    theme.colors.border = gpui::rgb(if theme.is_dark() { 0x65758b } else { 0x7b8797 }).into();
    theme.colors.input = theme.colors.border;
    theme.colors.danger = gpui::rgb(if theme.is_dark() { 0xffb4ab } else { 0x9b1c1c }).into();
    theme.colors.danger_foreground = theme.colors.background;
    theme.colors.sidebar = theme.colors.muted;
    theme.colors.sidebar_foreground = theme.colors.foreground;
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.editor_background = Some(theme.colors.background);
    highlight.style.editor_active_line = settings
        .current_line
        .then_some(theme.colors.foreground.opacity(0.06));
    theme.highlight_theme = std::sync::Arc::new(highlight);
    window.refresh();
    warning
}

pub fn canvas(cx: &App) -> [u8; 3] {
    let color = cx.theme().background.to_rgb();
    [
        (color.r * 255.).round() as u8,
        (color.g * 255.).round() as u8,
        (color.b * 255.).round() as u8,
    ]
}
