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
    theme.radius = px(7. * settings.ui_scale);
    theme.radius_lg = px(12. * settings.ui_scale);
    let palette = if theme.is_dark() { &DARK } else { &LIGHT };
    let color = |value: u32| -> gpui::Hsla { gpui::rgb(value).into() };
    let colors = &mut theme.colors;
    colors.background = color(palette.canvas);
    colors.foreground = color(palette.text);
    colors.muted = color(palette.chrome);
    colors.muted_foreground = color(palette.secondary);
    colors.title_bar = color(palette.chrome);
    colors.title_bar_border = color(palette.separator);
    colors.sidebar = color(palette.sidebar);
    colors.sidebar_foreground = color(palette.text);
    colors.sidebar_border = color(palette.separator);
    colors.popover = color(palette.raised);
    colors.popover_foreground = color(palette.text);
    colors.border = color(palette.control_border);
    colors.input = color(palette.control_border);
    colors.ring = color(palette.accent);
    colors.caret = color(palette.text);
    colors.link = color(palette.accent);
    colors.link_hover = color(palette.accent);
    colors.link_active = color(palette.accent);
    colors.primary = color(palette.action);
    colors.primary_hover = color(palette.action_hover);
    colors.primary_active = color(palette.action_hover);
    colors.primary_foreground = color(palette.action_text);
    colors.secondary = color(palette.faint);
    colors.secondary_hover = color(palette.hover);
    colors.secondary_active = color(palette.hover);
    colors.secondary_foreground = color(palette.text);
    colors.accent = color(palette.hover);
    colors.accent_foreground = color(palette.text);
    colors.list_hover = color(palette.hover);
    colors.list_active = color(palette.selection);
    colors.list_active_border = color(palette.accent);
    colors.selection = color(palette.selection);
    colors.tab_bar = color(palette.chrome);
    colors.tab = color(palette.chrome);
    colors.tab_active = color(palette.canvas);
    colors.tab_foreground = color(palette.secondary);
    colors.tab_active_foreground = color(palette.text);
    colors.scrollbar_thumb = color(palette.secondary);
    colors.scrollbar_thumb_hover = color(palette.text);
    colors.danger = color(palette.error);
    colors.danger_hover = color(palette.error);
    colors.danger_active = color(palette.error);
    colors.danger_foreground = color(palette.canvas);
    colors.warning = color(palette.warning);
    colors.warning_foreground = color(palette.canvas);
    colors.success = color(palette.success);
    colors.success_foreground = color(palette.canvas);
    colors.window_border = color(palette.separator);
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.editor_background = Some(theme.colors.background);
    highlight.style.editor_active_line = settings
        .current_line
        .then_some(theme.colors.foreground.opacity(0.06));
    theme.highlight_theme = std::sync::Arc::new(highlight);
    window.refresh();
    warning
}

/// Opaque palette from docs/ui-design.md. Text, controls and focus meet WCAG 2.2 contrast
/// on every surface they are drawn on; `action` darkens the accent where it fills a button
/// behind white text.
struct Palette {
    canvas: u32,
    chrome: u32,
    sidebar: u32,
    raised: u32,
    faint: u32,
    text: u32,
    secondary: u32,
    separator: u32,
    control_border: u32,
    hover: u32,
    selection: u32,
    accent: u32,
    action: u32,
    action_hover: u32,
    action_text: u32,
    warning: u32,
    error: u32,
    success: u32,
}

const LIGHT: Palette = Palette {
    canvas: 0xFFFFFF,
    chrome: 0xF2F4F7,
    sidebar: 0xF1F6FA,
    raised: 0xFFFFFF,
    faint: 0xE6E8EC,
    text: 0x17213A,
    secondary: 0x56637A,
    separator: 0xE1E6ED,
    control_border: 0x77839A,
    hover: 0xE1E7EE,
    selection: 0xDCEBFC,
    accent: 0x0A6CDF,
    action: 0x0A63CE,
    action_hover: 0x0957B5,
    action_text: 0xFFFFFF,
    warning: 0x805600,
    error: 0xB42332,
    success: 0x256B3D,
};

const DARK: Palette = Palette {
    canvas: 0x15181E,
    chrome: 0x1D2027,
    sidebar: 0x1C2027,
    raised: 0x252A33,
    faint: 0x343B45,
    text: 0xF1F3F5,
    secondary: 0xB5BEC9,
    separator: 0x343B45,
    control_border: 0x8B96A5,
    hover: 0x39424F,
    selection: 0x243E60,
    accent: 0x409CFF,
    action: 0x1F6FD8,
    action_hover: 0x2A7CE6,
    action_text: 0xFFFFFF,
    warning: 0xEBC378,
    error: 0xFF9EAA,
    success: 0x89D6A3,
};

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
        .map(rgb_bytes)
    };
    [states(canvas), states(active_line)]
}

pub(super) fn reading_code_canvases(cx: &App) -> [[u8; 3]; 2] {
    let theme = cx.theme();
    let code = theme.muted.to_rgb();
    [code, code.blend(theme.selection.to_rgb())].map(rgb_bytes)
}

fn rgb_bytes(color: gpui::Rgba) -> [u8; 3] {
    [color.r, color.g, color.b].map(|channel| (channel * 255.).round() as u8)
}
