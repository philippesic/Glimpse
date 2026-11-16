//! Highlighting runs outside the event loop; obsolete documents never recolor a newer edit.
use eframe::egui;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use syntect::{easy::HighlightLines, util::LinesWithEndings};

fn luminance(rgb: [u8; 3]) -> f64 {
    let linear = rgb.map(|c| {
        let c = c as f64 / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
/// Retain a grammar's hue while meeting normal-text contrast on the actual canvas.
pub fn readable_rgb(rgb: [u8; 3], canvas: [u8; 3]) -> [u8; 3] {
    if contrast(rgb, canvas) >= 4.5 {
        return rgb;
    }
    let target = if contrast([0; 3], canvas) >= contrast([255; 3], canvas) {
        [0; 3]
    } else {
        [255; 3]
    };
    let mix = |weight: f64| {
        std::array::from_fn(|i| {
            (rgb[i] as f64 * (1. - weight) + target[i] as f64 * weight).round() as u8
        })
    };
    let (mut low, mut high) = (0., 1.);
    for _ in 0..12 {
        let middle = (low + high) / 2.;
        if contrast(mix(middle), canvas) >= 4.5 {
            high = middle;
        } else {
            low = middle;
        }
    }
    mix(high)
}
/// Reference-inspired source colors with contrast correction applied on the actual canvas.
fn source_theme(dark: bool) -> syntect::highlighting::Theme {
    use syntect::highlighting::{Color, StyleModifier, Theme, ThemeItem, ThemeSettings};
    let color = |hex: u32| Color {
        r: (hex >> 16) as u8,
        g: (hex >> 8) as u8,
        b: hex as u8,
        a: 255,
    };
    let mut theme = Theme {
        settings: ThemeSettings {
            foreground: Some(color(if dark { 0xd4dcee } else { 0x17335c })),
            ..Default::default()
        },
        ..Default::default()
    };
    for (scope, light, night) in [
        ("keyword, storage", 0x004dff, 0x34aaff),
        ("entity.name.function, support.function", 0x0059d9, 0x51acff),
        ("entity.name.type, support.type", 0x006c8f, 0x52c9ed),
        ("string", 0x08732b, 0xa9da73),
        ("constant.numeric, constant.language", 0x7145b8, 0xcba3fa),
        ("comment", 0x717f91, 0x939eaf),
        ("entity.name.tag", 0x004dff, 0x34aaff),
    ] {
        theme.scopes.push(ThemeItem {
            scope: scope.parse().expect("static syntax scope"),
            style: StyleModifier {
                foreground: Some(color(if dark { night } else { light })),
                font_style: if scope == "comment" {
                    Some(syntect::highlighting::FontStyle::ITALIC)
                } else {
                    None
                },
                ..Default::default()
            },
        });
    }
    theme
}
#[derive(Clone, Debug)]
pub struct Span {
    pub start: usize,
    pub length: usize,
    pub rgb: [u8; 3],
    pub italic: bool,
}
pub struct Highlight {
    pub id: u64,
    pub generation: u64,
    pub revision: u64,
    pub spans: Vec<Span>,
    pub state: String,
}
struct Request {
    id: u64,
    generation: u64,
    text: String,
    extension: &'static str,
    dark: bool,
    token: u64,
}
fn grammar_extension(path: &std::path::Path, text: &str) -> Option<&'static str> {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let extension = match extension.to_ascii_lowercase().as_str() {
        "md" | "markdown" | "mdown" => "md",
        "rs" => "rs",
        "py" | "pyw" => "py",
        "js" | "mjs" | "cjs" | "jsx" => "js",
        "ts" | "mts" | "cts" | "tsx" => "ts",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "sh" | "bash" | "zsh" | "fish" => "sh",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "css" => "css",
        _ => "",
    };
    if !extension.is_empty() {
        return Some(extension);
    }
    if path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s,
            ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" | ".profile"
        )
    }) {
        return Some("sh");
    }
    // Only inspect a bounded interpreter line; unknown files remain editable plain text.
    if text.starts_with("#!") {
        let line = &text.as_bytes()[..text.len().min(256)];
        let line = String::from_utf8_lossy(line);
        let line = line.lines().next().unwrap_or("");
        if line.contains("python") {
            return Some("py");
        }
        if line.contains("node") {
            return Some("js");
        }
        if ["/sh", "/bash", "/zsh", " sh", " bash", " zsh"]
            .iter()
            .any(|s| line.contains(s))
        {
            return Some("sh");
        }
    }
    None
}
pub struct Syntax {
    pending: Arc<Mutex<Option<Request>>>,
    ready: Arc<Mutex<Option<Highlight>>>,
    wake: Option<mpsc::SyncSender<()>>,
    token: Arc<AtomicU64>,
}
impl Syntax {
    pub fn new(ctx: egui::Context) -> Self {
        let pending = Arc::new(Mutex::new(None::<Request>));
        let ready = Arc::new(Mutex::new(None));
        let token = Arc::new(AtomicU64::new(0));
        let (wake, rx) = mpsc::sync_channel(1);
        let (input, output, current) = (pending.clone(), ready.clone(), token.clone());
        thread::spawn(move || {
            let mut assets = None;
            while rx.recv().is_ok() {
                let Some(request) = input.lock().unwrap().take() else {
                    continue;
                };
                let syntaxes = assets.get_or_insert_with(two_face::syntax::extra_newlines);
                let syntax = syntaxes
                    .find_syntax_by_extension(request.extension)
                    .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
                let theme = source_theme(request.dark);
                let mut parser = HighlightLines::new(syntax, &theme);
                let mut spans = Vec::new();
                let mut offset = 0;
                let started = Instant::now();
                let mut state = syntax.name.clone();
                for line in LinesWithEndings::from(&request.text) {
                    if current.load(Ordering::Relaxed) != request.token {
                        break;
                    }
                    if line.len() > 32768
                        || started.elapsed() > Duration::from_millis(200)
                        || spans.len() >= 30000
                    {
                        state.push_str(" · partial highlighting (work limit)");
                        break;
                    }
                    match parser.highlight_line(line, syntaxes) {
                        Ok(parts) => {
                            for (style, part) in parts {
                                if spans.len() == 30000 {
                                    state.push_str(" · partial highlighting (span limit)");
                                    break;
                                }
                                let length = part.encode_utf16().count();
                                spans.push(Span {
                                    start: offset,
                                    length,
                                    italic: style
                                        .font_style
                                        .contains(syntect::highlighting::FontStyle::ITALIC),
                                    rgb: [
                                        style.foreground.r,
                                        style.foreground.g,
                                        style.foreground.b,
                                    ],
                                });
                                offset += length;
                            }
                        }
                        Err(_) => {
                            state.push_str(" · plain fallback (grammar failed)");
                            break;
                        }
                    }
                }
                let mut ready = output.lock().unwrap();
                if current.load(Ordering::Relaxed) == request.token {
                    *ready = Some(Highlight {
                        id: request.id,
                        generation: request.generation,
                        revision: request.token,
                        spans,
                        state,
                    });
                    ctx.request_repaint();
                }
            }
        });
        Self {
            pending,
            ready,
            wake: Some(wake),
            token,
        }
    }
    pub fn request(
        &self,
        id: u64,
        generation: u64,
        text: &str,
        path: &std::path::Path,
        dark: bool,
    ) {
        let token = self.token.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(extension) = grammar_extension(path, text) else {
            self.pending.lock().unwrap().take();
            *self.ready.lock().unwrap() = Some(Highlight {
                id,
                generation,
                revision: token,
                spans: Vec::new(),
                state: "Plain text".into(),
            });
            return;
        };
        *self.pending.lock().unwrap() = Some(Request {
            id,
            generation,
            text: text.into(),
            extension,
            dark,
            token,
        });
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }
    pub fn take(&self) -> Option<Highlight> {
        self.ready.lock().unwrap().take()
    }
}
impl Drop for Syntax {
    fn drop(&mut self) {
        self.token.fetch_add(1, Ordering::Relaxed);
        self.wake.take();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_text_is_immediate_and_invalidates_pending_grammar_work() {
        let syntax = Syntax::new(egui::Context::default());
        syntax.request(1, 1, "fn old() {}", std::path::Path::new("x.rs"), true);
        syntax.request(1, 2, "plain 🦀", std::path::Path::new("x.txt"), false);
        let result = syntax.take().unwrap();
        assert_eq!(result.generation, 2);
        assert_eq!(result.state, "Plain text");
        assert!(result.spans.is_empty());
        assert!(syntax.pending.lock().unwrap().is_none());
        assert_eq!(
            grammar_extension(std::path::Path::new(".zshrc"), ""),
            Some("sh")
        );
        assert_eq!(
            grammar_extension(std::path::Path::new("script"), "#!/usr/bin/env python3\n"),
            Some("py")
        );
        for extension in [
            "md", "rs", "py", "js", "ts", "go", "c", "cpp", "sh", "json", "yaml", "toml", "html",
            "css",
        ] {
            assert!(
                grammar_extension(std::path::Path::new(&format!("x.{extension}")), "").is_some()
            );
        }
    }
    #[test]
    fn syntax_colors_remain_readable_on_light_and_dark_canvases() {
        for canvas in [[255; 3], [250, 249, 247], [22, 25, 30], [0; 3]] {
            for rgb in [
                [150, 163, 180],
                [192, 197, 206],
                [143, 161, 179],
                [208, 135, 112],
                [20, 80, 100],
            ] {
                let adjusted = readable_rgb(rgb, canvas);
                assert!(
                    contrast(adjusted, canvas) >= 4.5,
                    "{adjusted:?} on {canvas:?}"
                );
                if contrast(rgb, canvas) >= 4.5 {
                    assert_eq!(adjusted, rgb);
                }
            }
        }
    }
    #[test]
    fn latest_generation_has_matching_utf16_ranges() {
        let syntax = Syntax::new(egui::Context::default());
        syntax.request(1, 1, "obsolete", std::path::Path::new("x.rs"), true);
        syntax.request(1, 2, "let 🦀 = 3;\n", std::path::Path::new("x.rs"), true);
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = syntax.take() {
                assert_eq!(result.generation, 2);
                assert_eq!(result.spans.iter().map(|s| s.length).sum::<usize>(), 12);
                assert_eq!(result.spans.first().unwrap().start, 0);
                break;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
    }
}
