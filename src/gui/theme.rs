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
    gpui::set_monospace_family(family.clone());
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
    colors.border = color(palette.border);
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
    colors.secondary_active = color(palette.pressed);
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
    colors.scrollbar_thumb = color(palette.border);
    colors.scrollbar_thumb_hover = color(palette.control_border);
    colors.danger = color(palette.error);
    colors.danger_hover = color(palette.error);
    colors.danger_active = color(palette.error);
    colors.danger_foreground = color(palette.canvas);
    colors.warning = color(palette.warning);
    colors.warning_foreground = color(palette.canvas);
    colors.success = color(palette.success);
    colors.success_foreground = color(palette.canvas);
    colors.window_border = color(palette.separator);
    colors.overlay = gpui::hsla(0., 0., 0., palette.scrim);
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.syntax = reading_syntax(theme.is_dark());
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
    border: u32,
    control_border: u32,
    hover: u32,
    pressed: u32,
    selection: u32,
    accent: u32,
    action: u32,
    action_hover: u32,
    action_text: u32,
    warning: u32,
    error: u32,
    success: u32,
    scrim: f32,
}

const LIGHT: Palette = Palette {
    canvas: 0xFFFFFF,
    chrome: 0xF6F6F7,
    sidebar: 0xF6F6F7,
    raised: 0xFFFFFF,
    faint: 0xEDEDEF,
    text: 0x27272A,
    secondary: 0x5C5C65,
    separator: 0xE4E4E7,
    border: 0xE4E4E7,
    control_border: 0x8B8B94,
    hover: 0xE4E4E7,
    pressed: 0xD4D4D8,
    selection: 0xE2E8F8,
    accent: 0x1D4ED8,
    action: 0x1D4ED8,
    action_hover: 0x1E40AF,
    action_text: 0xFFFFFF,
    warning: 0x9A5800,
    error: 0xC81E2B,
    success: 0x0F7A55,
    scrim: 0.28,
};

const DARK: Palette = Palette {
    canvas: 0x0A0A0A,
    chrome: 0x111111,
    sidebar: 0x111111,
    raised: 0x171717,
    faint: 0x1F1F1F,
    text: 0xF5F5F5,
    secondary: 0x9C9C9C,
    separator: 0x1F1F1F,
    border: 0x2A2A2A,
    control_border: 0x6B6B6B,
    hover: 0x262626,
    pressed: 0x323232,
    selection: 0x1A2747,
    accent: 0x6B98FF,
    action: 0x2F63E6,
    action_hover: 0x2857D6,
    action_text: 0xFFFFFF,
    warning: 0xF5A524,
    error: 0xFF5C63,
    success: 0x34D399,
    scrim: 0.6,
};

/// The leading-edge bar and fill of a list row. Every list marks its selected row the same way:
/// a bar in the accent colour while the list or its search field has keyboard focus, in the
/// secondary text colour otherwise, each at least 3:1 against the row in both themes, so the row
/// that Enter acts on never depends on the pale fill alone.
pub(super) fn list_row(selected: bool, active: bool, cx: &App) -> (gpui::Hsla, Option<gpui::Hsla>) {
    match (selected, active) {
        (true, true) => (cx.theme().ring, Some(cx.theme().selection)),
        (true, false) => (cx.theme().muted_foreground, Some(inactive_selection(cx))),
        _ => (gpui::transparent_black(), None),
    }
}

/// Neutral fill for a selection whose pane does not have keyboard focus.
pub(super) fn inactive_selection(cx: &App) -> gpui::Hsla {
    gpui::rgb(if cx.theme().is_dark() {
        0x232323
    } else {
        0xE9E9EC
    })
    .into()
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

/// Reading code blocks use the Source palette, mapped from TextMate scopes to Tree-sitter captures.
fn reading_syntax(dark: bool) -> gpui_component::highlighter::SyntaxColors {
    let captures: [&[&str]; 7] = [
        &["keyword"],
        &["function"],
        &["type", "constructor"],
        &["string", "string.escape", "string.regex", "string.special"],
        &["number", "boolean", "constant"],
        &["comment", "comment.doc"],
        &["tag"],
    ];
    let mut syntax = serde_json::Map::new();
    for (names, (_, light, night)) in captures.iter().zip(glimpse::syntax::SOURCE_COLORS) {
        let mut style = serde_json::Map::new();
        style.insert(
            "color".into(),
            format!("#{:06x}", if dark { night } else { light }).into(),
        );
        if names[0] == "comment" {
            style.insert("font_style".into(), "italic".into());
        }
        for name in *names {
            syntax.insert((*name).into(), style.clone().into());
        }
    }
    serde_json::from_value(syntax.into()).expect("static reading syntax colours")
}
