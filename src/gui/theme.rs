use glimpse::storage::Settings;
use gpui::{App, Window, px};
use gpui_component::{ActiveTheme, Theme, ThemeMode};

pub fn apply(settings: &Settings, window: &mut Window, cx: &mut App) {
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
    if !settings.font_family.is_empty() {
        theme.mono_font_family = settings.font_family.clone().into();
    }
    theme.colors.muted_foreground =
        gpui::rgb(if theme.is_dark() { 0xaab5c5 } else { 0x4d596a }).into();
    theme.colors.border = gpui::rgb(if theme.is_dark() { 0x65758b } else { 0x7b8797 }).into();
    theme.colors.input = theme.colors.border;
    theme.colors.sidebar = theme.colors.muted;
    theme.colors.sidebar_foreground = theme.colors.foreground;
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.editor_background = Some(theme.colors.background);
    highlight.style.editor_active_line = settings
        .current_line
        .then_some(theme.colors.foreground.opacity(0.06));
    theme.highlight_theme = std::sync::Arc::new(highlight);
    window.refresh();
}

pub fn canvas(cx: &App) -> [u8; 3] {
    let color = cx.theme().background.to_rgb();
    [
        (color.r * 255.).round() as u8,
        (color.g * 255.).round() as u8,
        (color.b * 255.).round() as u8,
    ]
}
