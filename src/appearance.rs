//! Semantic colors, system typography and static native UI components.

use egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// Logical-point baselines. Layouts must grow to fit the actual text height.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    /// Total row pitch, including vertical item spacing.
    pub row_height: f32,
    pub toolbar_height: f32,
    pub control_height: f32,
    pub pane_margin: i8,
}

impl Metrics {
    pub const fn for_density(density: Density) -> Self {
        match density {
            Density::Comfortable => Self {
                row_height: 32.0,
                toolbar_height: 36.0,
                control_height: 24.0,
                pane_margin: 12,
            },
            Density::Compact => Self {
                row_height: 24.0,
                toolbar_height: 28.0,
                control_height: 20.0,
                pane_margin: 8,
            },
        }
    }
}

/// Opaque semantic colors; selection keeps the normal text foreground.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub canvas: Color32,
    pub chrome: Color32,
    pub sidebar: Color32,
    pub raised: Color32,
    pub faint: Color32,
    pub text: Color32,
    pub secondary: Color32,
    /// Decorative separators, not a control's only visible boundary.
    pub separator: Color32,
    pub control_border: Color32,
    pub hover: Color32,
    pub selection: Color32,
    pub inactive_selection: Color32,
    /// Links, focus rings and the modified-document indicator.
    pub accent: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub success: Color32,
}

impl Palette {
    pub const fn for_theme(theme: Theme) -> Self {
        match theme {
            Theme::Light => Self {
                canvas: rgb(0xffffff),
                chrome: rgb(0xf2f4f7),
                sidebar: rgb(0xf1f6fa),
                raised: rgb(0xffffff),
                faint: rgb(0xe6e8ec),
                text: rgb(0x17213a),
                secondary: rgb(0x647085),
                separator: rgb(0xe1e6ed),
                control_border: rgb(0xcbd3df),
                hover: rgb(0xe1e7ee),
                selection: rgb(0xdcebfc),
                inactive_selection: rgb(0xe5ebf2),
                accent: rgb(0x007aff),
                warning: rgb(0x805600),
                error: rgb(0xb42332),
                success: rgb(0x256b3d),
            },
            Theme::Dark => Self {
                canvas: rgb(0x15181e),
                chrome: rgb(0x1d2027),
                sidebar: rgb(0x1c2027),
                raised: rgb(0x252a33),
                faint: rgb(0x343b45),
                text: rgb(0xf1f3f5),
                secondary: rgb(0xb5bec9),
                separator: rgb(0x343b45),
                control_border: rgb(0x8b96a5),
                hover: rgb(0x39424f),
                selection: rgb(0x243e60),
                inactive_selection: rgb(0x3a424d),
                accent: rgb(0x409cff),
                warning: rgb(0xebc378),
                error: rgb(0xff9eaa),
                success: rgb(0x89d6a3),
            },
        }
    }
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Resolve against the drawing UI, including a locally overridden light/dark mode.
pub fn palette(ui: &egui::Ui) -> Palette {
    Palette::for_theme(Theme::from_dark_mode(ui.visuals().dark_mode))
}

/// Install at startup or a density change, then apply configured font sizes.
/// Keeps the caller's System/Light/Dark preference and registered font families.
pub fn install(ctx: &egui::Context, density: Density) {
    // Native typography is registered once per context; fallback families remain intact.
    let font_key = egui::Id::unique("glimpse_system_fonts");
    if !ctx.data(|data| data.get_temp::<bool>(font_key).unwrap_or(false)) {
        let mut fonts = egui::FontDefinitions::default();
        for (name, path, family) in [
            (
                "Glimpse UI",
                "/System/Library/Fonts/SFNS.ttf",
                egui::FontFamily::Proportional,
            ),
            (
                "Glimpse Bold",
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                egui::FontFamily::Name("Glimpse Bold".into()),
            ),
            (
                "Glimpse Mono",
                "/System/Library/Fonts/SFNSMono.ttf",
                egui::FontFamily::Monospace,
            ),
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, name.into());
            }
        }
        let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
        fonts
            .families
            .entry(egui::FontFamily::Name("Glimpse Bold".into()))
            .or_default()
            .extend(fallback);
        ctx.set_fonts(fonts);
        ctx.data_mut(|data| data.insert_temp(font_key, true));
    }
    let metrics = Metrics::for_density(density);
    for theme in [Theme::Light, Theme::Dark] {
        let colors = Palette::for_theme(theme);
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = theme.default_visuals();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.interact_size = egui::vec2(metrics.control_height, metrics.control_height);
        style.spacing.window_margin = egui::Margin::same(metrics.pane_margin);
        style.spacing.indent = 16.0;
        style.spacing.icon_width = 16.0;
        style.spacing.icon_width_inner = 10.0;
        style.spacing.icon_spacing = 4.0;
        style.spacing.button_padding.y = match density {
            Density::Comfortable => 4.0,
            Density::Compact => 2.0,
        };
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(18., egui::FontFamily::Name("Glimpse Bold".into())),
        );
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(15.0));
        style.animation_time = 0.10;
        style.scroll_animation = egui::style::ScrollAnimation::none();
        style.spacing.scroll = egui::style::ScrollStyle::floating();
        style.spacing.scroll.bar_width = 6.;
        style.spacing.scroll.floating_width = 2.;
        style.spacing.scroll.dormant_handle_opacity = 0.;
        style.spacing.scroll.dormant_background_opacity = 0.;

        let visuals = &mut style.visuals;
        visuals.override_text_color = None;
        visuals.weak_text_color = Some(colors.secondary);
        visuals.panel_fill = colors.chrome;
        visuals.window_fill = colors.raised;
        visuals.window_stroke = Stroke::new(1.0, colors.separator);
        visuals.extreme_bg_color = colors.canvas;
        visuals.text_edit_bg_color = Some(colors.canvas);
        visuals.faint_bg_color = colors.faint;
        visuals.code_bg_color = colors.faint;
        visuals.hyperlink_color = colors.accent;
        visuals.warn_fg_color = colors.warning;
        visuals.error_fg_color = colors.error;
        visuals.selection.bg_fill = colors.selection;
        visuals.selection.stroke = Stroke::new(1.0, colors.text);
        visuals.window_corner_radius = CornerRadius::same(12);
        visuals.menu_corner_radius = CornerRadius::same(9);
        visuals.text_cursor.stroke = Stroke::new(2.0, colors.accent);
        // A steady caret remains visible without waking an otherwise idle editor.
        visuals.text_cursor.blink = false;
        visuals.image_loading_spinners = false;

        let widgets = &mut visuals.widgets;
        for widget in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.corner_radius = CornerRadius::same(7);
            widget.fg_stroke = Stroke::new(1.0, colors.text);
            widget.expansion = 0.0;
        }
        widgets.noninteractive.bg_fill = colors.chrome;
        widgets.noninteractive.weak_bg_fill = Color32::TRANSPARENT;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.separator);
        widgets.inactive.bg_fill = colors.faint;
        widgets.inactive.weak_bg_fill = colors.raised;
        widgets.inactive.bg_stroke = Stroke::new(1.0, colors.separator);
        widgets.hovered.bg_fill = colors.hover;
        widgets.hovered.weak_bg_fill = colors.hover;
        widgets.hovered.bg_stroke = Stroke::new(1.0, colors.control_border);
        // egui uses the active visuals for keyboard-focused standard widgets too.
        widgets.active.bg_fill = colors.selection;
        widgets.active.weak_bg_fill = colors.selection;
        widgets.active.bg_stroke = Stroke::new(2.0, colors.accent);
        widgets.open.bg_fill = colors.selection;
        widgets.open.weak_bg_fill = colors.selection;
        widgets.open.bg_stroke = Stroke::new(1.0, colors.accent);
        ctx.set_style_of(theme, style);
    }
}

/// A small vector symbol family, drawn at the display's actual scale.
#[derive(Clone, Copy)]
pub enum Symbol {
    Back,
    Forward,
    Up,
    Folder,
    File,
    Search,
    Plus,
    Close,
    Sidebar,
    ChevronDown,
    Clock,
    Sliders,
    Brand,
}

pub fn symbol(ui: &egui::Ui, rect: egui::Rect, symbol: Symbol, color: Color32) {
    let rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y);
    let stroke = Stroke::new(1.4, color);
    let painter = ui.painter();
    let line = |points: &[(f32, f32)]| {
        painter.add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match symbol {
        Symbol::Back => line(&[(10., 3.), (5., 8.), (10., 13.)]),
        Symbol::Forward => line(&[(6., 3.), (11., 8.), (6., 13.)]),
        Symbol::Up => {
            line(&[(3., 7.), (8., 2.), (13., 7.)]);
            line(&[(8., 2.), (8., 14.)]);
        }
        Symbol::ChevronDown => line(&[(3., 6.), (8., 11.), (13., 6.)]),
        Symbol::Folder => {
            // Separate back, lip and face preserve the reference's small folder depth.
            painter.rect_filled(
                egui::Rect::from_min_max(p(1., 3.), p(8., 7.)),
                1.5,
                rgb(0x0085f5),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(p(1., 5.), p(15., 14.)),
                2.,
                rgb(0x168cf5),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(p(1.5, 5.5), p(14.5, 12.5)),
                1.5,
                rgb(0x27a8ff),
            );
            painter.line_segment([p(2., 6.), p(14., 6.)], Stroke::new(1., rgb(0x70cdff)));
        }
        Symbol::Clock => {
            painter.circle_stroke(p(8., 8.), 6., stroke);
            line(&[(8., 4.), (8., 8.), (11., 8.)]);
        }
        Symbol::Sliders => {
            for y in [4., 8., 12.] {
                painter.circle_stroke(p(3., y), 0.7, stroke);
                line(&[(7., y), (14., y)]);
            }
        }
        Symbol::Brand => {
            painter.rect_filled(rect, 4., rgb(0x777e8b));
            painter.rect_filled(
                egui::Rect::from_min_max(p(4., 3.), p(12., 13.)),
                1.5,
                Color32::WHITE,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(p(9., 3.), p(12., 6.)),
                0.,
                rgb(0xc4c9d0),
            );
        }
        Symbol::File => {
            line(&[
                (3., 14.),
                (3., 2.),
                (9., 2.),
                (13., 6.),
                (13., 14.),
                (3., 14.),
            ]);
            line(&[(9., 2.), (9., 6.), (13., 6.)]);
        }
        Symbol::Search => {
            painter.circle_stroke(p(6.5, 6.5), 4.5, stroke);
            line(&[(10., 10.), (14., 14.)]);
        }
        Symbol::Plus => {
            line(&[(8., 3.), (8., 13.)]);
            line(&[(3., 8.), (13., 8.)]);
        }
        Symbol::Close => {
            line(&[(4., 4.), (12., 12.)]);
            line(&[(12., 4.), (4., 12.)]);
        }
        Symbol::Sidebar => {
            painter.rect_stroke(
                egui::Rect::from_min_max(p(1., 2.), p(15., 14.)),
                3.,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(6., 2.), (6., 14.)]);
        }
    }
}

pub fn icon_button(ui: &mut egui::Ui, icon: Symbol, label: &str, selected: bool) -> egui::Response {
    symbol_button(ui, icon, label, selected, egui::vec2(28., 28.))
}

pub fn symbol_button(
    ui: &mut egui::Ui,
    icon: Symbol,
    label: &str,
    selected: bool,
    size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        if matches!(icon, Symbol::Sidebar | Symbol::Search) {
            egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
        } else {
            egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
        }
    });
    let colors = palette(ui);
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            7.,
            if selected {
                colors.selection
            } else {
                colors.hover
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            7.,
            Stroke::new(2., colors.accent),
            egui::StrokeKind::Inside,
        );
    }
    symbol(
        ui,
        rect,
        icon,
        if ui.is_enabled() {
            colors.text
        } else {
            colors.secondary.gamma_multiply(0.55)
        },
    );
    response.on_hover_text(label)
}

pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let colors = palette(ui);
    ui.add(
        egui::Button::new(egui::RichText::new(label).color(colors.canvas).strong())
            .fill(colors.accent)
            .stroke(Stroke::NONE)
            .corner_radius(7.),
    )
}

pub fn toolbar_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(palette(ui).chrome)
        .inner_margin(egui::Margin::symmetric(12, 10))
}

pub fn section(ui: &mut egui::Ui, title: &str, detail: &str) {
    ui.label(egui::RichText::new(title).font(heading_font(18.)));
    if !detail.is_empty() {
        ui.label(egui::RichText::new(detail).color(palette(ui).secondary));
    }
    ui.add_space(12.);
}

/// Bound both long-line and many-line layout before handing content to egui.
pub fn preview_text(text: &str) -> (String, bool) {
    let end = text.floor_char_boundary(text.len().min(16 * 1024));
    let mut output = String::with_capacity(end);
    let mut truncated = end < text.len();
    for (index, line) in text[..end].split('\n').enumerate() {
        if index == 160 {
            truncated = true;
            break;
        }
        let mut chars = line.chars();
        output.extend(chars.by_ref().take(240));
        if chars.next().is_some() {
            output.push('…');
            truncated = true;
        }
        output.push('\n');
    }
    (output, truncated)
}

pub fn bounded_preview(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, text: &str) {
    let (preview, truncated) = preview_text(text);
    if truncated {
        ui.colored_label(
            palette(ui).warning,
            "Excerpt only · first 160 lines, 240 characters per line, up to 16 KiB",
        );
    }
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(180.)
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(preview).monospace()).selectable(true));
        });
}

/// File-type cues depend only on the path, so painting never reads the filesystem.
pub fn file_symbol(ui: &egui::Ui, rect: egui::Rect, path: &std::path::Path, directory: bool) {
    let colors = palette(ui);
    if directory {
        symbol(ui, rect, Symbol::Folder, colors.accent);
        return;
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let painter = ui.painter();
    let r = egui::Rect::from_center_size(rect.center(), egui::vec2(17., 18.));
    let rust = extension == "rs";
    if rust {
        let orange = rgb(0xe86c08);
        for n in 0..12 {
            let angle = n as f32 * std::f32::consts::TAU / 12.;
            let v = egui::vec2(angle.cos(), angle.sin());
            painter.line_segment(
                [r.center() + v * 6., r.center() + v * 8.],
                Stroke::new(2., orange),
            );
        }
        painter.circle_stroke(r.center(), 6.5, Stroke::new(1.5, orange));
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            "R",
            FontId::proportional(11.),
            orange,
        );
        return;
    }
    let (label, color) = match extension.as_str() {
        "md" | "markdown" => (
            "M",
            if ui.visuals().dark_mode {
                rgb(0xaad5ef)
            } else {
                rgb(0x315773)
            },
        ),
        "toml" | "json" | "yaml" | "yml" => ("≡", colors.secondary),
        "py" => ("Py", rgb(0x377fbc)),
        "js" | "ts" | "jsx" | "tsx" | "go" | "c" | "cpp" | "h" | "sh" | "html" | "css" => {
            ("‹›", colors.accent)
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" => ("▧", colors.success),
        _ => {
            symbol(ui, rect, Symbol::File, colors.secondary);
            return;
        }
    };
    painter.rect_filled(r, 2.5, colors.canvas);
    painter.rect_stroke(
        r,
        2.5,
        Stroke::new(1., colors.control_border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(if label.len() > 1 { 9. } else { 12. }),
        color,
    );
}

/// Search matches use yellow independently of selection and keyboard focus.
pub fn search_match_colors(dark: bool) -> (Color32, Color32) {
    if dark {
        (rgb(0x756018), rgb(0xfff4ca))
    } else {
        (rgb(0xffe878), rgb(0x3b3100))
    }
}

pub fn heading_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("Glimpse Bold".into()))
}

#[cfg(test)]
mod tests {
    use super::preview_text;

    #[test]
    fn yellow_search_matches_have_readable_text_in_both_themes() {
        let luminance = |c: super::Color32| {
            let f = |v: u8| {
                let x = v as f64 / 255.;
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        };
        for dark in [false, true] {
            let (bg, fg) = super::search_match_colors(dark);
            let (a, b) = (luminance(bg), luminance(fg));
            assert!((a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5);
            assert!(bg.r() > bg.b() && bg.g() > bg.b());
        }
    }

    #[test]
    fn compare_preview_bounds_unicode_and_long_lines() {
        let (preview, truncated) = preview_text(&"👨‍👩‍👧‍👦".repeat(400_000));
        assert!(truncated);
        assert!(preview.chars().count() <= 242);
        assert!(preview.ends_with("…\n"));
        let (preview, truncated) = preview_text(&"line\n".repeat(10_000));
        assert!(truncated);
        assert_eq!(preview.lines().count(), 160);
        assert_eq!(preview_text("hello 👋"), ("hello 👋\n".into(), false));
    }
}
