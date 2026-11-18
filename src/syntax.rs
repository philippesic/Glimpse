//! Highlighting runs outside the event loop; obsolete documents never recolor a newer edit.
use crate::wake;
use gpui_component::input::{LineType, Rope};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use syntect::easy::HighlightLines;

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
    text: Rope,
    extension: &'static str,
    dark: bool,
    token: u64,
}

#[derive(Default)]
struct Work {
    request: Option<Request>,
    after_retirement: Option<fn()>,
}

pub fn grammar_extension(path: &std::path::Path, text: &str) -> Option<&'static str> {
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

pub fn grammar_for_text(path: &std::path::Path, text: &Rope) -> Option<&'static str> {
    if let Some(extension) = grammar_extension(path, "") {
        return Some(extension);
    }
    if !text.chunks().next()?.starts_with('#') {
        return None;
    }
    let mut prefix = Vec::with_capacity(256);
    for chunk in text.chunks() {
        let count = (256 - prefix.len()).min(chunk.len());
        prefix.extend_from_slice(&chunk.as_bytes()[..count]);
        if prefix.len() == 256 {
            break;
        }
    }
    grammar_extension(path, &String::from_utf8_lossy(&prefix))
}
pub struct Syntax {
    pending: Arc<Mutex<Work>>,
    ready: Arc<Mutex<Option<Highlight>>>,
    wake: Option<mpsc::SyncSender<()>>,
    token: Arc<AtomicU64>,
}
impl Syntax {
    pub fn new(ctx: wake::Context) -> Self {
        let pending = Arc::new(Mutex::new(Work::default()));
        let ready = Arc::new(Mutex::new(None));
        let token = Arc::new(AtomicU64::new(0));
        let (wake, rx) = mpsc::sync_channel(1);
        let (input, output, current) = (pending.clone(), ready.clone(), token.clone());
        thread::spawn(move || {
            let mut assets = None;
            let mut superseded = false;
            while rx.recv().is_ok() {
                let mut work = std::mem::take(&mut *input.lock().unwrap());
                while superseded
                    && work.after_retirement.is_none()
                    && work.request.as_ref().is_some_and(|request| {
                        request.generation != 0 && request.text.len() >= 1024 * 1024
                    })
                {
                    match rx.recv_timeout(Duration::from_millis(24)) {
                        Ok(()) => {
                            let next = std::mem::take(&mut *input.lock().unwrap());
                            work.request = next.request.or(work.request);
                            work.after_retirement = next.after_retirement;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                superseded = false;
                if let Some(request) = work
                    .request
                    .filter(|request| current.load(Ordering::Relaxed) == request.token)
                {
                    let _work = gpui::work_trace::Span::new("syntax_worker");
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
                    for line in request
                        .text
                        .lines(LineType::LF)
                        .filter(|line| line.len() != 0)
                    {
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
                        let line = line.as_str().map_or_else(
                            || std::borrow::Cow::Owned(line.to_string()),
                            std::borrow::Cow::Borrowed,
                        );
                        match parser.highlight_line(&line, syntaxes) {
                            Ok(parts) => {
                                for (style, part) in parts {
                                    if spans.len() == 30000 {
                                        state.push_str(" · partial highlighting (span limit)");
                                        break;
                                    }
                                    let length = part.len();
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
                    } else {
                        superseded = true;
                    }
                }
                if let Some(after_retirement) = work.after_retirement {
                    {
                        let _work = gpui::work_trace::Span::new("syntax_assets_retire");
                        assets = None;
                    }
                    let _work = gpui::work_trace::Span::new("syntax_memory_release");
                    after_retirement();
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
        text: Rope,
        path: &std::path::Path,
        dark: bool,
    ) {
        let token = self.token.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(extension) = grammar_for_text(path, &text) else {
            self.pending.lock().unwrap().request.take();
            *self.ready.lock().unwrap() = Some(Highlight {
                id,
                generation,
                revision: token,
                spans: Vec::new(),
                state: "Plain text".into(),
            });
            return;
        };
        *self.pending.lock().unwrap() = Work {
            request: Some(Request {
                id,
                generation,
                text,
                extension,
                dark,
                token,
            }),
            after_retirement: None,
        };
        self.signal();
    }
    pub fn retire_assets(&self, after_retirement: fn()) {
        self.pending.lock().unwrap().after_retirement = Some(after_retirement);
        self.signal();
    }
    fn signal(&self) {
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
        let syntax = Syntax::new(wake::Context::default());
        syntax.request(
            1,
            1,
            Rope::from("fn old() {}"),
            std::path::Path::new("x.rs"),
            true,
        );
        syntax.request(
            1,
            2,
            Rope::from("plain 🦀"),
            std::path::Path::new("x.txt"),
            false,
        );
        let result = syntax.take().unwrap();
        assert_eq!(result.generation, 2);
        assert_eq!(result.state, "Plain text");
        assert!(result.spans.is_empty());
        assert!(syntax.pending.lock().unwrap().request.is_none());
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
    fn large_edit_burst_publishes_the_latest_syntax() {
        let syntax = Syntax::new(wake::Context::default());
        let reference = Syntax::new(wake::Context::default());
        let path = std::path::Path::new("notes.rs");
        let source = Rope::from("let earlier = 0; // comment\r\n".repeat(50_000));
        let wait_for = |syntax: &Syntax, generation| {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(result) = syntax.take()
                    && result.generation == generation
                {
                    break result;
                }
                assert!(Instant::now() < until);
                thread::sleep(Duration::from_millis(5));
            }
        };

        syntax.request(1, 0, source.clone(), path, true);
        assert!(!wait_for(&syntax, 0).spans.is_empty());
        for generation in 1..12 {
            let mut revision = source.clone();
            revision.insert(0, &format!("// pending {generation}\n"));
            syntax.request(1, generation, revision, path, true);
            thread::sleep(Duration::from_millis(2));
        }
        let prefix = "fn newest() { let value = \"café\"; }\r\n";
        let mut latest = source;
        latest.insert(0, prefix);
        syntax.request(1, 12, latest, path, true);
        reference.request(1, 12, Rope::from(prefix), path, true);
        let result = wait_for(&syntax, 12);
        let expected = wait_for(&reference, 12);
        let spans = |highlight: Highlight| {
            highlight
                .spans
                .into_iter()
                .take_while(|span| span.start < prefix.len())
                .map(|span| (span.start, span.length, span.italic, span.rgb))
                .collect::<Vec<_>>()
        };
        let expected = spans(expected);
        assert!(!expected.is_empty());
        assert_eq!(spans(result), expected);
    }

    #[test]
    fn latest_generation_has_matching_byte_ranges() {
        let syntax = Syntax::new(wake::Context::default());
        syntax.request(
            1,
            1,
            Rope::from("obsolete"),
            std::path::Path::new("x.rs"),
            true,
        );
        let text = format!(
            "let 🦀 = \"café 中文 é\";\r\n// {}\r\nlet done = true;\n",
            "é 字 café ".repeat(450)
        );
        syntax.request(
            1,
            2,
            Rope::from(text.as_str()),
            std::path::Path::new("x.rs"),
            true,
        );
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = syntax.take() {
                assert_eq!(result.generation, 2);
                let mut covered = 0;
                for span in &result.spans {
                    assert_eq!(span.start, covered);
                    covered += span.length;
                    assert!(text.is_char_boundary(span.start));
                    assert!(text.is_char_boundary(covered));
                }
                assert_eq!(covered, text.len());
                break;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(5));
        }
    }
}
