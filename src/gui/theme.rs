use glimpse::storage::Settings;
use gpui::{App, Window, px};
use gpui_component::{ActiveTheme, Colorize, Theme, ThemeMode};

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
    theme.colors.scrollbar_thumb = theme.colors.muted_foreground;
    theme.colors.scrollbar_thumb_hover = theme.colors.foreground;
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

pub fn source_canvases(cx: &App) -> [[[u8; 3]; 5]; 2] {
    let theme = cx.theme();
    let style = &theme.highlight_theme.style;
    let canvas = style.editor_background.unwrap_or(theme.background).to_rgb();
    let active_line = style
        .editor_active_line
        .map_or(canvas, |color| canvas.blend(color.to_rgb()));
    let selection = theme.selection.to_rgb();
    let matches = theme.selection.saturation(0.1).to_rgb();
    let states = |base: gpui::Rgba| {
        let other_match = base.blend(matches);
        let current_match = other_match.blend(selection);
        [
            base,
            base.blend(selection),
            other_match,
            current_match,
            current_match.blend(selection),
        ]
        .map(|color| {
            [
                (color.r * 255.).round() as u8,
                (color.g * 255.).round() as u8,
                (color.b * 255.).round() as u8,
            ]
        })
    };
    [states(canvas), states(active_line)]
}
