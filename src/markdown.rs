//! Markdown keeps authoritative source separate from its bounded reading presentation.
use eframe::egui::{
    self,
    load::{ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint},
};
use egui_commonmark::{CommonMarkCache, CommonMarkScrollOptions, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::{
    collections::HashMap,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};
type Images = Arc<Mutex<HashMap<String, Arc<egui::ColorImage>>>>;
struct LocalImages(Images);
impl ImageLoader for LocalImages {
    fn id(&self) -> &str {
        "glimpse::markdown::LocalImages"
    }
    fn load(&self, _: &egui::Context, uri: &str, _: SizeHint) -> ImageLoadResult {
        self.0
            .lock()
            .unwrap()
            .get(uri)
            .cloned()
            .map(|image| ImagePoll::Ready { image })
            .ok_or(LoadError::NotSupported)
    }
    fn forget(&self, uri: &str) {
        self.0.lock().unwrap().remove(uri);
    }
    fn forget_all(&self) {
        self.0.lock().unwrap().clear();
    }
    fn byte_size(&self) -> usize {
        self.0
            .lock()
            .unwrap()
            .values()
            .map(|i| i.pixels.len() * 4)
            .sum()
    }
}
type ImageResults = Arc<Mutex<HashMap<String, (u64, Result<Arc<egui::ColorImage>, String>)>>>;
struct ImageJob {
    token: u64,
    uri: String,
    path: PathBuf,
}
struct Request {
    id: u64,
    generation: u64,
    text: String,
    path: PathBuf,
    token: u64,
}
struct Reading {
    id: u64,
    generation: u64,
    text: String,
    pages: Vec<Range<usize>>,
    links: Vec<String>,
    images: HashMap<String, Arc<egui::ColorImage>>,
    image_jobs: Vec<(String, PathBuf)>,
    token: u64,
    warnings: Vec<String>,
}
pub enum Link {
    Local(PathBuf),
    External(String),
    Unsupported(String),
}
pub struct Markdown {
    pending: Arc<Mutex<Option<Request>>>,
    ready: Arc<Mutex<Option<Reading>>>,
    wake: Option<mpsc::SyncSender<()>>,
    token: Arc<AtomicU64>,
    current: Option<Reading>,
    image_results: ImageResults,
    images: Images,
    cache: CommonMarkCache,
    page: usize,
    requested: Option<(u64, u64)>,
}
impl Markdown {
    pub fn new(ctx: egui::Context) -> Self {
        let pending = Arc::new(Mutex::new(None::<Request>));
        let ready = Arc::new(Mutex::new(None));
        let token = Arc::new(AtomicU64::new(0));
        let images = Images::default();
        ctx.add_image_loader(Arc::new(LocalImages(images.clone())));
        let (wake, rx) = mpsc::sync_channel(1);
        let (input, output, current) = (pending.clone(), ready.clone(), token.clone());
        let (image_jobs, image_rx) = mpsc::sync_channel::<ImageJob>(32);
        let image_rx = Arc::new(Mutex::new(image_rx));
        let image_results = ImageResults::default();
        for _ in 0..2 {
            let (rx, results, current, ctx) = (
                image_rx.clone(),
                image_results.clone(),
                token.clone(),
                ctx.clone(),
            );
            thread::spawn(move || {
                loop {
                    let Ok(job) = rx.lock().unwrap().recv() else {
                        break;
                    };
                    if current.load(Ordering::Relaxed) != job.token {
                        continue;
                    }
                    let result = raster(&job.path);
                    if current.load(Ordering::Relaxed) == job.token {
                        results.lock().unwrap().insert(job.uri, (job.token, result));
                        ctx.request_repaint();
                    }
                }
            });
        }
        thread::spawn(move || {
            while rx.recv().is_ok() {
                let Some(request) = input.lock().unwrap().take() else {
                    continue;
                };
                if let Some(mut reading) = prepare(request, &current) {
                    for (uri, path) in std::mem::take(&mut reading.image_jobs) {
                        if image_jobs
                            .try_send(ImageJob {
                                token: reading.token,
                                uri,
                                path,
                            })
                            .is_err()
                        {
                            reading.warnings.push(
                                "Image queue occupied; re-enter Read to retry local images.".into(),
                            );
                            break;
                        }
                    }
                    *output.lock().unwrap() = Some(reading);
                    ctx.request_repaint();
                }
            }
        });
        Self {
            pending,
            ready,
            wake: Some(wake),
            token,
            current: None,
            image_results,
            images,
            cache: CommonMarkCache::default(),
            page: 0,
            requested: None,
        }
    }
    pub fn refresh(&mut self) {
        self.requested = None;
    }
    pub fn request(&mut self, id: u64, generation: u64, text: &str, path: &Path) {
        if self.requested == Some((id, generation)) {
            return;
        }
        self.requested = Some((id, generation));
        let token = self.token.fetch_add(1, Ordering::Relaxed) + 1;
        *self.pending.lock().unwrap() = Some(Request {
            id,
            generation,
            text: text.into(),
            path: path.into(),
            token,
        });
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        generation: u64,
        path: &Path,
    ) -> Option<Link> {
        if let Some(reading) = self.ready.lock().unwrap().take()
            && self.requested == Some((reading.id, reading.generation))
        {
            let uris = self
                .images
                .lock()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            for uri in uris {
                ui.ctx().forget_image(&uri);
            }
            *self.images.lock().unwrap() = reading.images.clone();
            self.cache = CommonMarkCache::default();
            for link in &reading.links {
                self.cache.add_link_hook(link);
            }
            self.current = Some(reading);
            self.page = 0;
        }
        let decoded = {
            let mut pending = self.image_results.lock().unwrap();
            let current = self.current.as_ref().map(|r| r.token);
            let latest = self.token.load(Ordering::Relaxed);
            let keys: Vec<_> = pending
                .iter()
                .filter(|(_, (token, _))| Some(*token) == current)
                .map(|(uri, _)| uri.clone())
                .collect();
            let decoded = keys
                .into_iter()
                .filter_map(|uri| pending.remove(&uri).map(|value| (uri, value)))
                .collect::<HashMap<_, _>>();
            pending.retain(|_, (token, _)| *token == latest);
            decoded
        };
        if let Some(reading) = self.current.as_mut() {
            for (uri, (token, result)) in decoded {
                if token == reading.token {
                    match result {
                        Ok(image) => {
                            let bytes: usize = self
                                .images
                                .lock()
                                .unwrap()
                                .values()
                                .map(|i| i.pixels.len() * 4)
                                .sum();
                            if bytes + image.pixels.len() * 4 <= 32 * 1024 * 1024 {
                                ui.ctx().forget_image(&uri);
                                self.images.lock().unwrap().insert(uri, image);
                            } else {
                                reading
                                    .warnings
                                    .push("Local image cache limit reached (32 MiB).".into());
                            }
                        }
                        Err(_) => reading.warnings.push(format!(
                            "Couldn’t load local image: {}. Check the file in Edit view.",
                            uri.strip_prefix("file://").unwrap_or(&uri)
                        )),
                    }
                }
            }
            reading.warnings.truncate(32);
        }
        let Some(reading) = self
            .current
            .as_ref()
            .filter(|r| r.id == id && r.generation == generation)
        else {
            ui.label("Preparing reading view…");
            return None;
        };
        for warning in &reading.warnings {
            ui.weak(warning);
        }
        if reading.pages.len() > 1 {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.page > 0, egui::Button::new("Previous page"))
                    .clicked()
                {
                    self.page -= 1;
                }
                ui.label(format!("Page {} of {}", self.page + 1, reading.pages.len()));
                if ui
                    .add_enabled(
                        self.page + 1 < reading.pages.len(),
                        egui::Button::new("Next page"),
                    )
                    .clicked()
                {
                    self.page += 1;
                }
            });
        }
        let page = &reading.text[reading.pages[self.page].clone()];
        let displayed =
            (page.len() <= 65536).then(|| image_source(page, &self.images.lock().unwrap()));
        let source = displayed.as_deref().unwrap_or(page);
        if source.len() > 65536 {
            ui.label("This Markdown block exceeds the 64 KiB reading-layout limit. Edit Source shows the complete block.");
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut end = 8192.min(source.len());
                while !source.is_char_boundary(end) {
                    end -= 1;
                }
                ui.monospace(&source[..end]);
            });
        } else if source.len() <= 16384 {
            let headings = plain_headings(source);
            egui::ScrollArea::vertical()
                .id_salt(("reading_page", id, self.page))
                .show(ui, |ui| {
                    let mut cursor = 0;
                    for (range, level, label) in headings {
                        if range.start > cursor {
                            ui.push_id(cursor, |ui| {
                                CommonMarkViewer::new()
                                    .explicit_image_uri_scheme(false)
                                    .max_image_width(Some(720))
                                    .show(ui, &mut self.cache, &source[cursor..range.start]);
                            });
                        }
                        ui.add_space(if cursor == 0 { 0. } else { 8. });
                        let size = match level {
                            1 => 28.,
                            2 => 21.,
                            3 => 18.,
                            _ => 16.,
                        };
                        ui.label(
                            egui::RichText::new(label).font(crate::appearance::heading_font(size)),
                        );
                        ui.add_space(3.);
                        cursor = range.end;
                    }
                    if cursor < source.len() {
                        ui.push_id(cursor, |ui| {
                            CommonMarkViewer::new()
                                .explicit_image_uri_scheme(false)
                                .max_image_width(Some(720))
                                .show(ui, &mut self.cache, &source[cursor..]);
                        });
                    }
                });
        } else {
            CommonMarkViewer::new()
                .explicit_image_uri_scheme(false)
                .max_image_width(Some(720))
                .show_scrollable(
                    egui::Id::unique(("markdown", id, self.page)),
                    ui,
                    &mut self.cache,
                    &CommonMarkScrollOptions::default().viewport_cache(true),
                    source,
                );
        }
        for link in &reading.links {
            if self.cache.get_link_hook(link) == Some(true) {
                self.cache.link_hooks_mut().insert(link.clone(), false);
                return Some(resolve_link(path, link));
            }
        }
        None
    }
}
impl Drop for Markdown {
    fn drop(&mut self) {
        self.token.fetch_add(1, Ordering::Relaxed);
        self.wake.take();
    }
}
fn local_path(base: &Path, value: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let value = value.strip_prefix("file://").unwrap_or(value);
    if value.contains("://") || value.starts_with("data:") {
        return None;
    }
    let bytes = percent_encoding::percent_decode_str(value.split('#').next().unwrap_or(value))
        .collect::<Vec<_>>();
    Some(
        base.parent()
            .unwrap_or(Path::new("/"))
            .join(std::ffi::OsString::from_vec(bytes)),
    )
}
pub fn resolve_link(base: &Path, value: &str) -> Link {
    if value.starts_with("https://") || value.starts_with("http://") || value.starts_with("mailto:")
    {
        return Link::External(value.into());
    }
    if (value.starts_with('#') || value.split('/').next().unwrap_or("").contains(':'))
        && !value.starts_with("file://")
    {
        return Link::Unsupported(value.into());
    }
    local_path(base, value)
        .map(Link::Local)
        .unwrap_or_else(|| Link::Unsupported(value.into()))
}
fn raster(path: &Path) -> Result<Arc<egui::ColorImage>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = vec![];
    file.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("file exceeds 8 MiB".into());
    }
    let reader = || {
        image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 || w as u64 * h as u64 > 8 * 1024 * 1024 {
        return Err("decoded image exceeds 8 megapixels or 8192 px per side".into());
    }
    let mut decoder = reader()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder.limits(limits);
    let rgba = decoder.decode().map_err(|e| e.to_string())?.to_rgba8();
    Ok(Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        rgba.as_raw(),
    )))
}
// Replace unavailable images only in the presentation; source and links remain authoritative.
fn image_source(source: &str, images: &HashMap<String, Arc<egui::ColorImage>>) -> String {
    let mut displayed = String::new();
    let mut cursor = 0;
    for (event, range) in Parser::new_ext(
        source,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
    .into_offset_iter()
    {
        if let Event::Start(Tag::Image { dest_url, .. }) = event {
            let uri = if dest_url.contains("://") {
                dest_url.to_string()
            } else {
                format!("file://{dest_url}")
            };
            if !images.contains_key(&uri) && range.start >= cursor {
                displayed.push_str(&source[cursor..range.start]);
                displayed.push_str(
                    if local_path(Path::new("/document.md"), &dest_url).is_some() {
                        "*Local image unavailable or loading*"
                    } else {
                        "*Remote image not loaded*"
                    },
                );
                cursor = range.end;
            }
        }
    }
    displayed.push_str(&source[cursor..]);
    displayed
}

fn prepare(request: Request, current: &AtomicU64) -> Option<Reading> {
    let mut links = vec![];
    let images = HashMap::new();
    let mut image_jobs = vec![];
    let mut warnings = vec![];
    let mut pages = vec![];
    let mut start = 0;
    let mut depth: usize = 0;
    for (event, range) in Parser::new_ext(
        &request.text,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
    .into_offset_iter()
    {
        if current.load(Ordering::Relaxed) != request.token {
            return None;
        }
        match &event {
            Event::Start(Tag::Link { dest_url, .. })
                if !links.iter().any(|s| s == dest_url.as_ref()) =>
            {
                links.push(dest_url.to_string());
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let uri = if dest_url.contains("://") {
                    dest_url.to_string()
                } else {
                    format!("file://{dest_url}")
                };
                if image_jobs.len() < 32
                    && !image_jobs.iter().any(|(known, _)| known == &uri)
                    && let Some(path) = local_path(&request.path, dest_url)
                {
                    image_jobs.push((uri, path));
                }
            }
            _ => {}
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 && range.end - start >= 16384 {
                    pages.push(start..range.end);
                    start = range.end;
                }
            }
            _ => {}
        }
    }
    if start < request.text.len() || pages.is_empty() {
        pages.push(start..request.text.len());
    }
    warnings.truncate(32);
    Some(Reading {
        id: request.id,
        generation: request.generation,
        text: request.text,
        pages,
        links,
        images,
        image_jobs,
        token: request.token,
        warnings,
    })
}

/// Style standalone headings while leaving nested blocks and linked headings to CommonMark.
/// The small-page path is bounded at 16 KiB; larger pages keep viewport virtualization.
fn plain_headings(source: &str) -> Vec<(Range<usize>, u8, String)> {
    let mut result = Vec::new();
    let mut current = None::<(usize, u8, String, bool)>;
    let mut nested = 0;
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        match event {
            // Splitting a page must not separate definitions from their references.
            Event::FootnoteReference(_) | Event::Start(Tag::FootnoteDefinition(_)) => {
                return Vec::new();
            }
            Event::Start(Tag::Link { link_type, .. })
                if !matches!(
                    link_type,
                    pulldown_cmark::LinkType::Inline
                        | pulldown_cmark::LinkType::Autolink
                        | pulldown_cmark::LinkType::Email
                ) =>
            {
                return Vec::new();
            }
            Event::Start(Tag::BlockQuote(_) | Tag::List(_) | Tag::Item) => nested += 1,
            Event::End(
                pulldown_cmark::TagEnd::BlockQuote(_)
                | pulldown_cmark::TagEnd::List(_)
                | pulldown_cmark::TagEnd::Item,
            ) => nested -= 1,
            Event::Start(Tag::Heading { level, .. }) if nested == 0 => {
                current = Some((range.start, level as u8, String::new(), false))
            }
            Event::End(pulldown_cmark::TagEnd::Heading(_)) => {
                if let Some((start, level, label, complex)) = current.take()
                    && !complex
                {
                    result.push((start..range.end, level, label));
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, label, _)) = &mut current {
                    label.push_str(&text);
                }
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => {
                if let Some((_, _, _, complex)) = &mut current {
                    *complex = true;
                }
            }
            _ => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn standalone_heading_style_preserves_nested_blocks_and_reference_definitions() {
        let source = "# Title 🦀\n\n> ## Quoted\n\n```md\n# literal\n```\n\n## Section\n\ntext";
        let headings = super::plain_headings(source);
        assert_eq!(
            headings
                .iter()
                .map(|(_, _, text)| text.as_str())
                .collect::<Vec<_>>(),
            ["Title 🦀", "Section"]
        );
        for (range, _, _) in headings {
            assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
        }
        assert!(
            super::plain_headings("# Title\n\n[open][file]\n\n## Next\n\n[file]: other.md")
                .is_empty()
        );
        assert!(
            super::plain_headings("# Title\n\nfootnote[^1]\n\n## Next\n\n[^1]: note").is_empty()
        );
    }

    use super::*;
    #[test]
    fn links_resolve_without_executing_schemes() {
        let base = Path::new("/tmp/docs/a.md");
        assert!(
            matches!(resolve_link(base,"other%20file.md"),Link::Local(p) if p==Path::new("/tmp/docs/other file.md"))
        );
        assert!(matches!(
            resolve_link(base, "javascript:alert(1)"),
            Link::Unsupported(_)
        ));
    }
    #[test]
    fn preparation_preserves_source_and_bounds_remote_images() {
        let text = "# Heading\n\n**bold** ~~strike~~ [link](other.md) ![remote](https://invalid.example/a.png)\n\n|a|b|\n|-|-|\n|1|2|\n";
        let result = prepare(
            Request {
                id: 1,
                generation: 2,
                text: text.into(),
                path: "/tmp/a.md".into(),
                token: 7,
            },
            &AtomicU64::new(7),
        )
        .unwrap();
        assert_eq!(result.text, text);
        assert_eq!(result.links, vec!["other.md"]);
        assert!(result.images.is_empty());
        assert!(result.image_jobs.is_empty());
        let displayed = image_source(text, &HashMap::new());
        assert!(displayed.contains("Remote image not loaded"));
        assert!(!displayed.contains("![remote]"));
        assert!(displayed.contains("[link](other.md)"));
        let local = "Before ![sample](sample.png) after";
        assert_eq!(
            image_source(local, &HashMap::new()),
            "Before *Local image unavailable or loading* after"
        );
        let mut images = HashMap::new();
        images.insert(
            "file://sample.png".into(),
            Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::BLUE)),
        );
        assert_eq!(image_source(local, &images), local);
        assert!(
            prepare(
                Request {
                    id: 1,
                    generation: 3,
                    text: text.into(),
                    path: "/tmp/a.md".into(),
                    token: 6
                },
                &AtomicU64::new(7)
            )
            .is_none()
        );
    }
}
