use pulldown_cmark::{Event, Options, Parser, Tag};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
pub struct Prepared {
    pub source: String,
    pub images: HashMap<String, Raster>,
    pub notes: Vec<String>,
}
#[derive(Debug, PartialEq)]
pub enum Link {
    Local(PathBuf),
    External(String),
    Unsupported,
}
pub fn resolve_link(base: &Path, value: &str) -> Link {
    if value.chars().any(char::is_control) || value.starts_with('#') || value.starts_with("//") {
        return Link::Unsupported;
    }
    let scheme = value.split(':').next().unwrap_or("").to_ascii_lowercase();
    if value.contains(':') && matches!(scheme.as_str(), "http" | "https" | "mailto") {
        return Link::External(value.into());
    }
    let path = value.split(['#', '?']).next().unwrap_or(value);
    if let Some(path) = path.strip_prefix("file://") {
        if !path.starts_with('/') {
            return Link::Unsupported;
        }
        return decoded_path(path)
            .map(Link::Local)
            .unwrap_or(Link::Unsupported);
    }
    if path.split('/').next().unwrap_or("").contains(':') || path.is_empty() {
        return Link::Unsupported;
    }
    decoded_path(path)
        .map(|path| Link::Local(base.parent().unwrap_or(Path::new("/")).join(path)))
        .unwrap_or(Link::Unsupported)
}
fn decoded_path(value: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let bytes: Vec<_> = percent_encoding::percent_decode_str(value).collect();
    (!bytes.contains(&0)).then(|| PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}
pub fn prepare(source: String, path: &Path) -> Prepared {
    let mut images = HashMap::new();
    let mut notes = vec![];
    let mut total = 0;
    let mut attempted = std::collections::HashSet::new();
    for event in Parser::new_ext(
        &source,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    ) {
        let Event::Start(Tag::Image { dest_url, .. }) = event else {
            continue;
        };
        if !attempted.insert(dest_url.to_string()) {
            continue;
        }
        if attempted.len() > 32 {
            notes.push("Additional images omitted: at most 32 local images per document.".into());
            break;
        }
        let resource: &str = &dest_url;
        let result = match resolve_link(path, &dest_url) {
            Link::Local(path) => raster(&path),
            _ => {
                notes.push(format!(
                    "Remote or unsupported image omitted: {resource:?}; no network request was made."
                ));
                continue;
            }
        };
        match result {
            Ok(image) if total + image.bgra.len() <= 32 * 1024 * 1024 => {
                total += image.bgra.len();
                images.insert(dest_url.to_string(), image);
            }
            Ok(_) => notes.push(format!(
                "Local image omitted: {resource:?}. The document image cache is limited to 32 MiB."
            )),
            Err(error) => notes.push(format!("Local image unavailable: {resource:?}. {error}")),
        }
    }
    notes.sort();
    notes.dedup();
    Prepared {
        source,
        images,
        notes,
    }
}
fn raster(path: &Path) -> Result<Raster, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("only regular raster files are supported".into());
    }
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
    if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 8 * 1024 * 1024 {
        return Err("decoded image exceeds 8 megapixels or 8192 px per side".into());
    }
    let mut decoder = reader()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder.limits(limits);
    let mut pixels = decoder
        .decode()
        .map_err(|e| e.to_string())?
        .to_rgba8()
        .into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2)
    }
    Ok(Raster {
        width: w,
        height: h,
        bgra: pixels,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn links_preserve_native_paths_and_reject_active_schemes() {
        let base = Path::new("/work/docs/read.md");
        assert_eq!(
            resolve_link(base, "../sibling%20file.md#part"),
            Link::Local("/work/docs/../sibling file.md".into())
        );
        assert_eq!(
            resolve_link(base, "file:///tmp/a%23b.md"),
            Link::Local("/tmp/a#b.md".into())
        );
        for value in [
            "javascript:alert(1)",
            "data:text/html,hi",
            "//host/file",
            "file://host/tmp/a",
            "foo%00.md",
            "#heading",
        ] {
            assert_eq!(resolve_link(base, value), Link::Unsupported, "{value}");
        }
        assert!(matches!(
            resolve_link(base, "https://example.test"),
            Link::External(_)
        ));
    }
    #[test]
    fn local_raster_is_bounded_and_source_stays_exact() {
        let dir = tempfile::tempdir().unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save(dir.path().join("local.png"))
            .unwrap();
        let source =
            "![local](local.png)\n![remote](http://127.0.0.1:1/trap.png)\n<script>raw</script>\n"
                .to_owned();
        let prepared = prepare(source.clone(), &dir.path().join("read.md"));
        assert_eq!(prepared.source, source);
        assert_eq!(
            (
                prepared.images["local.png"].width,
                prepared.images["local.png"].height
            ),
            (2, 2)
        );
        assert_eq!(&prepared.images["local.png"].bgra[..4], &[0, 0, 255, 255]);
        assert_eq!(prepared.images.len(), 1);
        assert!(
            prepared
                .notes
                .iter()
                .any(|note| note.contains("no network"))
        );
        let file = dir.path().join("large.png");
        std::fs::File::create(&file)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();
        assert!(raster(&file).err().unwrap().contains("8 MiB"));
    }
}
