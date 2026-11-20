use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

mod revisions;

pub fn data_dir() -> Result<PathBuf, &'static str> {
    if let Some(path) = std::env::var_os("GLIMPSE_DATA_DIR") {
        return Ok(path.into());
    }
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(path).join("glimpse"));
    }
    let home = std::env::home_dir().ok_or(
        "Glimpse needs a home folder for settings and recovery. Set HOME or GLIMPSE_DATA_DIR and try again.",
    )?;
    #[cfg(target_os = "linux")]
    let data = home.join(".local/share/glimpse");
    #[cfg(target_os = "macos")]
    let data = home.join("Library/Application Support/Glimpse");
    Ok(data)
}
pub fn private_dir(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())
}
pub fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic_write(path, |file| {
        file.write_all(bytes).map_err(|error| error.to_string())
    })
}
struct RecordWriter<'a> {
    file: &'a mut fs::File,
    hash: &'a mut Sha256,
}

impl Write for RecordWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.hash.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    atomic_write(path, |file| {
        let output = RecordWriter {
            file,
            hash: &mut hash,
        };
        let mut writer = std::io::BufWriter::with_capacity(65536, output);
        serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
    })?;
    Ok(hash.finalize().into())
}
fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> Result<(), String>,
) -> Result<(), String> {
    private_dir(path.parent().ok_or("Missing parent")?)?;
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().unwrap())
        .map_err(|error| error.to_string())?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    write(temp.as_file_mut())?;
    temp.as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temp.persist(path).map_err(|error| error.to_string())?;
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
pub struct LocationVisit {
    #[serde(
        serialize_with = "native_path::serialize",
        deserialize_with = "native_path::deserialize"
    )]
    pub path: PathBuf,
    pub last: u64,
    pub score: f64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub font_size: f32,
    pub line_height: f32,
    pub wrap: bool,
    pub indent: usize,
    pub dark: bool,
    pub system_theme: bool,
    pub compact: bool,
    pub reduce_motion: bool,
    pub ui_scale: f32,
    pub tabs: bool,
    pub proportional: bool,
    pub show_hidden: bool,
    pub search_ignored: bool,
    #[serde(
        serialize_with = "native_path::serialize_many",
        deserialize_with = "native_path::deserialize_many"
    )]
    pub pins: Vec<PathBuf>,
    #[serde(
        serialize_with = "native_path::serialize_many",
        deserialize_with = "native_path::deserialize_many"
    )]
    pub recents: Vec<PathBuf>,
    #[serde(
        serialize_with = "native_path::serialize_optional",
        deserialize_with = "native_path::deserialize_optional"
    )]
    pub last_location: Option<PathBuf>,
    pub location_visits: Vec<LocationVisit>,
    pub keymaps: std::collections::BTreeMap<String, String>,
    pub font_family: String,
    pub line_numbers: bool,
    pub current_line: bool,
    pub auto_indent: bool,
    pub syntax: bool,
    pub markdown_font_size: f32,
    pub remember_locations: bool,
    pub search_metadata: bool,
    pub search_mounts: bool,
    pub search_directory_links: bool,
    pub search_exclude: String,
    pub search_file_mib: usize,
    #[serde(skip)]
    pub disk_base: Option<Vec<u8>>,
    #[serde(skip)]
    pub warning: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            font_size: 14.5,
            line_height: 1.4,
            wrap: true,
            indent: 4,
            dark: true,
            system_theme: true,
            compact: false,
            reduce_motion: false,
            ui_scale: 1.0,
            tabs: false,
            proportional: false,
            show_hidden: true,
            search_ignored: false,
            pins: Vec::new(),
            recents: Vec::new(),
            last_location: None,
            location_visits: Vec::new(),
            keymaps: Default::default(),
            font_family: String::new(),
            line_numbers: true,
            current_line: true,
            auto_indent: true,
            syntax: true,
            markdown_font_size: 16.,
            remember_locations: true,
            search_metadata: false,
            search_mounts: false,
            search_directory_links: false,
            search_exclude: String::new(),
            search_file_mib: 1,
            disk_base: None,
            warning: None,
        }
    }
}
fn location_score(visit: &LocationVisit, now: u64) -> f64 {
    visit.score * 2_f64.powf(-(now.saturating_sub(visit.last) as f64) / (7. * 86400.))
}

impl Settings {
    pub fn record_location(&mut self, path: &Path, now: u64) -> bool {
        if !self.remember_locations {
            return false;
        }
        if let Some(visit) = self.location_visits.iter_mut().find(|v| v.path == path) {
            // Repeated file opens in one visit do not reshuffle the list.
            if now.saturating_sub(visit.last) < 1800 {
                return false;
            }
            visit.score = location_score(visit, now) + 1.;
            visit.last = now;
        } else {
            self.location_visits.push(LocationVisit {
                path: path.to_path_buf(),
                last: now,
                score: 1.,
            });
        }
        self.location_visits.sort_by(|a, b| {
            location_score(b, now)
                .total_cmp(&location_score(a, now))
                .then_with(|| a.path.cmp(&b.path))
        });
        self.location_visits.truncate(32);
        self.recents.retain(|p| p != path);
        self.recents.insert(0, path.to_path_buf());
        self.recents.truncate(20);
        true
    }

    pub fn frequent_locations(&self) -> Vec<PathBuf> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut visits: Vec<_> = self
            .location_visits
            .iter()
            .filter(|v| !self.pins.contains(&v.path))
            .collect();
        visits.sort_by(|a, b| {
            location_score(b, now)
                .total_cmp(&location_score(a, now))
                .then_with(|| a.path.cmp(&b.path))
        });
        visits.into_iter().map(|v| v.path.clone()).collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !(10.0..=32.0).contains(&self.font_size)
            || !(1.2..=2.0).contains(&self.line_height)
            || !(1..=8).contains(&self.indent)
            || !(0.8..=2.0).contains(&self.ui_scale)
            || !(12.0..=28.0).contains(&self.markdown_font_size)
            || !(1..=10).contains(&self.search_file_mib)
            || self.pins.len() > 128
            || self.recents.len() > 20
        {
            return Err(
                "Unsupported or invalid settings; the last valid values are retained.".into(),
            );
        }
        if self.font_family.len() > 512 || self.search_exclude.len() > 32768 {
            return Err("Font family or search exclusions exceed the settings size limit.".into());
        }
        if serde_json::to_vec_pretty(self)
            .map_err(|error| error.to_string())?
            .len()
            > 65536
        {
            return Err("Settings exceed 64 KiB; the saved settings are retained.".into());
        }
        crate::commands::bindings(&self.keymaps).map_err(|error| error.to_string())?;
        self.validate_exclusion_patterns()
            .map_err(|(_, error)| error.to_string())
    }

    pub fn validate_exclusion_patterns(&self) -> Result<(), (usize, ignore::Error)> {
        let mut globs = ignore::overrides::OverrideBuilder::new("/");
        for (index, line) in self.search_exclude.lines().enumerate() {
            if !line.trim().is_empty() {
                globs
                    .add(&format!("!{line}"))
                    .map_err(|error| (index + 1, error))?;
            }
        }
        Ok(())
    }

    pub fn load() -> Result<Self, String> {
        Self::load_in(&data_dir()?)
    }
    pub(crate) fn load_in(root: &Path) -> Result<Self, String> {
        use std::io::Read;
        let path = root.join("settings.json");
        let bytes = match fs::File::open(&path) {
            Ok(f) => {
                let mut bytes = vec![];
                f.take(65537)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 65536 {
                    return Err("Settings exceed 64 KiB; file retained.".into());
                }
                bytes
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let parse = |bytes: &[u8]| -> Result<Self, String> {
            let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
            value.validate()?;
            Ok(value)
        };
        let mut value = match parse(&bytes) {
            Ok(value) => value,
            Err(error) => {
                let previous = fs::File::open(root.join("settings.previous.json"))
                    .ok()
                    .and_then(|file| {
                        let mut bytes = Vec::new();
                        file.take(65537).read_to_end(&mut bytes).ok()?;
                        (bytes.len() <= 65536).then_some(bytes)
                    })
                    .and_then(|bytes| parse(&bytes).ok());
                let source = if previous.is_some() {
                    "previous valid settings"
                } else {
                    "defaults"
                };
                let mut value = previous.unwrap_or_default();
                value.warning = Some(format!(
                    "Settings are invalid ({error}); {source} loaded. Apply Preferences explicitly to replace the invalid file."
                ));
                value
            }
        };
        value.disk_base = Some(bytes);
        Ok(value)
    }
    pub fn save_checked(&self, root: &Path, expected: &mut Option<Vec<u8>>) -> Result<(), String> {
        self.validate()?;
        if self.warning.is_some() {
            return Err(
                "Invalid settings file retained; Apply Settings or Reload Settings explicitly."
                    .into(),
            );
        }
        private_dir(root)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("settings.lock"))
            .map_err(|e| e.to_string())?;
        if unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
            return Err("Another instance is writing settings; retry.".into());
        }
        let path = root.join("settings.json");
        let current = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if &current != expected {
            return Err(
                "Settings changed outside this session. Reload Settings before replacing them."
                    .into(),
            );
        }
        if let Some(bytes) = current {
            let valid = serde_json::from_slice::<Self>(&bytes)
                .ok()
                .is_some_and(|s| s.validate().is_ok());
            atomic(
                &root.join(if valid {
                    "settings.previous.json"
                } else {
                    "settings.rejected.json"
                }),
                &bytes,
            )?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        atomic(&path, &bytes)?;
        *expected = Some(bytes);
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct Draft<Text = std::sync::Arc<str>> {
    #[serde(with = "native_path")]
    pub path: PathBuf,
    pub text: Text,
    #[serde(with = "encoded_bytes")]
    pub base: crate::document::FileBytes,
    #[serde(
        default,
        serialize_with = "native_path::serialize_optional",
        deserialize_with = "native_path::deserialize_optional"
    )]
    pub target: Option<PathBuf>,
    pub generation: u64,
}

impl<Text> Draft<Text> {
    pub(crate) fn map_text<Output>(self, map: impl FnOnce(Text) -> Output) -> Draft<Output> {
        Draft {
            path: self.path,
            text: map(self.text),
            base: self.base,
            target: self.target,
            generation: self.generation,
        }
    }
}
#[derive(Clone)]
pub struct Record {
    pub path: PathBuf,
    pub original: PathBuf,
    pub bytes: u64,
    pub generation: u64,
    pub history: bool,
    pub candidate: bool,
}
#[derive(Deserialize)]
struct Header {
    #[serde(with = "native_path")]
    path: PathBuf,
    generation: u64,
}
#[derive(Default)]
pub struct RecoveryUsage {
    pub bytes: u64,
    pub files: u64,
    pub partial: bool,
}
#[derive(Default)]
pub struct Inventory {
    pub records: Vec<Record>,
    pub usage: RecoveryUsage,
    pub records_truncated: bool,
}
fn session_lock(directory: &Path) -> Result<fs::File, String> {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("lock"))
        .map_err(|e| e.to_string())?;
    if unsafe {
        libc::flock(
            std::os::fd::AsRawFd::as_raw_fd(&lock),
            libc::LOCK_EX | libc::LOCK_NB,
        )
    } != 0
    {
        return Err("This recovery session is active; its records were left intact.".into());
    }
    Ok(lock)
}
pub fn inventory(root: &Path) -> Result<Inventory, String> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Inventory::default()),
        Err(e) => return Err(e.to_string()),
    };
    let mut result = Inventory::default();
    let mut scanned = 0;
    for directory in entries {
        scanned += 1;
        if scanned > 100_000 {
            result.usage.partial = true;
            break;
        }
        let Ok(directory) = directory else {
            result.usage.partial = true;
            continue;
        };
        let Ok(kind) = directory.file_type() else {
            result.usage.partial = true;
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        let history = directory.file_name() == "history";
        let lock = (!history)
            .then(|| session_lock(&directory.path()).ok())
            .flatten();
        let available = history || lock.is_some();
        let files = match fs::read_dir(directory.path()) {
            Ok(files) => files,
            Err(_) => {
                result.usage.partial = true;
                continue;
            }
        };
        for file in files {
            scanned += 1;
            if scanned > 100_000 {
                result.usage.partial = true;
                break;
            }
            let Ok(file) = file else {
                result.usage.partial = true;
                continue;
            };
            let Ok(kind) = file.file_type() else {
                result.usage.partial = true;
                continue;
            };
            let path = file.path();
            let revision = !history && kind.is_dir() && revisions::is_revision(&path);
            // Do not follow symlinks out of private recovery storage.
            if !kind.is_file() && !revision {
                continue;
            }
            let Ok(meta) = file.metadata() else {
                result.usage.partial = true;
                continue;
            };
            let (files, bytes) = if revision {
                match revisions::sizes(&path) {
                    Ok(sizes) => sizes,
                    Err(_) => {
                        result.usage.partial = true;
                        (0, 0)
                    }
                }
            } else {
                (1, meta.len())
            };
            result.usage.bytes = result.usage.bytes.saturating_add(bytes);
            result.usage.files += files;
            if !available || !revision && path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            if result.records.len() >= 256 {
                result.records_truncated = true;
                continue;
            }
            let candidate =
                !history && file.file_name().to_string_lossy().starts_with("candidate-");
            let header = if revision {
                revisions::header(&path).ok()
            } else if bytes <= MAX_RECOVERY_RECORD_BYTES {
                fs::File::open(&path).ok().and_then(|f| {
                    serde_json::from_reader::<_, Header>(std::io::BufReader::new(f)).ok()
                })
            } else {
                None
            };
            let (original, generation) = header
                .map(|h| (h.path, h.generation))
                .unwrap_or_else(|| (PathBuf::from("Unreadable recovery record"), 0));
            result.records.push(Record {
                path,
                original,
                bytes,
                generation,
                history,
                candidate,
            });
        }
        if scanned > 100_000 {
            break;
        }
    }
    result.records.sort_by(|a, b| {
        a.history
            .cmp(&b.history)
            .then(a.original.cmp(&b.original))
            .then(a.path.cmp(&b.path))
    });
    Ok(result)
}
const MAX_RECOVERY_RECORD_BYTES: u64 = 70 * 1024 * 1024;

fn open_recovery(path: &Path) -> std::io::Result<(fs::File, u64)> {
    use std::{
        io::{Error, ErrorKind},
        os::unix::fs::OpenOptionsExt,
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Recovery record is not a regular file",
        ));
    }
    if metadata.len() > MAX_RECOVERY_RECORD_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Recovery record exceeds the supported size",
        ));
    }
    Ok((file, metadata.len()))
}

fn read_recovery_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    let (file, length) = open_recovery(path)?;
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_RECOVERY_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECOVERY_RECORD_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Recovery record exceeds the supported size",
        ));
    }
    Ok(bytes)
}

fn read_recovery_json<T: serde::de::DeserializeOwned>(path: &Path) -> std::io::Result<T> {
    use std::io::{BufReader, Error, ErrorKind, Read};
    let (file, _) = open_recovery(path)?;
    let limit = MAX_RECOVERY_RECORD_BYTES + 1;
    let mut limited = file.take(limit);
    let result = serde_json::from_reader(BufReader::with_capacity(64 * 1024, &mut limited));
    if limited.limit() == 0 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Recovery record exceeds the supported size",
        ));
    }
    result.map_err(Into::into)
}

fn read_draft(path: &Path) -> Result<Draft, String> {
    if revisions::is_revision(path) {
        if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            return Err("Recovery draft is not a directory".into());
        }
        return revisions::read(path);
    }
    let bytes = read_recovery_bytes(path).map_err(|e| e.to_string())?;
    let draft: Draft = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    validate_record_limits(draft.generation, draft.text.len(), draft.base.len())?;
    Ok(draft)
}

fn validate_record_limits(
    generation: u64,
    text_bytes: usize,
    base_bytes: usize,
) -> Result<(), String> {
    if generation > i64::MAX as u64 {
        return Err("Recovery generation is invalid".into());
    }
    let limit = crate::document::MAX_BYTES as usize;
    if text_bytes > limit || base_bytes > limit {
        return Err("Recovery text exceeds the editing limit".into());
    }
    Ok(())
}

fn recovery_text_length<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    struct TextLength;

    impl serde::de::Visitor<'_> for TextLength {
        type Value = usize;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("recovered document text")
        }

        fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<usize, E> {
            Ok(value.len())
        }
    }

    d.deserialize_str(TextLength)
}

#[derive(Deserialize)]
struct RecoveryHeader {
    #[serde(rename = "path", with = "native_path")]
    _path: PathBuf,
    #[serde(deserialize_with = "recovery_text_length", rename = "text")]
    text_bytes: usize,
    #[serde(deserialize_with = "encoded_bytes::decoded_len", rename = "base")]
    base_bytes: usize,
    #[serde(
        default,
        deserialize_with = "native_path::deserialize_optional",
        rename = "target"
    )]
    _target: Option<PathBuf>,
    generation: u64,
}

fn read_recovery_header(path: &Path, validate_limits: bool) -> std::io::Result<u64> {
    let header: RecoveryHeader = read_recovery_json(path)?;
    if validate_limits {
        validate_record_limits(header.generation, header.text_bytes, header.base_bytes)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    }
    Ok(header.generation)
}
pub fn restore_snapshot(path: &Path, observed: bool) -> Result<Draft, String> {
    let parent = path.parent().ok_or("Invalid recovery record")?;
    let _lock = if parent.file_name().is_some_and(|n| n == "history") {
        None
    } else {
        Some(session_lock(parent)?)
    };
    let mut draft = read_draft(path)?;
    if observed {
        let bom = draft.base.has_bom();
        draft.text = std::str::from_utf8(&draft.base.content_bytes(bom))
            .map_err(|_| "Observed disk version is not UTF-8")?
            .into();
        draft.base = if bom {
            crate::document::BOM.into()
        } else {
            Default::default()
        };
        draft.target = None;
        draft.generation = 1;
    }
    Ok(draft)
}
pub fn trash_record(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid recovery record")?;
    let _lock = if parent.file_name().is_some_and(|n| n == "history") {
        None
    } else {
        Some(session_lock(parent)?)
    };
    crate::fileops::trash(path)
}
pub(crate) trait RecoveryText: Serialize {
    fn byte_len(&self) -> usize;
}

impl RecoveryText for str {
    fn byte_len(&self) -> usize {
        self.len()
    }
}

impl RecoveryText for std::sync::Arc<str> {
    fn byte_len(&self) -> usize {
        self.len()
    }
}

impl<Text: RecoveryText + ?Sized> RecoveryText for &Text {
    fn byte_len(&self) -> usize {
        RecoveryText::byte_len(*self)
    }
}

#[derive(Clone, Copy)]
struct WrittenRecord {
    digest: [u8; 32],
    generation: u64,
    text_bytes: usize,
    base_bytes: usize,
}

fn record_digest(path: &Path) -> std::io::Result<[u8; 32]> {
    use std::io::{Error, ErrorKind, Read};
    let (file, _) = open_recovery(path)?;
    let mut stream = file.take(MAX_RECOVERY_RECORD_BYTES + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let bytes = match stream.read(&mut buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            count => count?,
        };
        if bytes == 0 {
            break;
        }
        hash.update(&buffer[..bytes]);
    }
    if stream.limit() == 0 {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "Recovery record exceeds the supported size",
        ));
    }
    Ok(hash.finalize().into())
}

pub struct Recovery {
    pub directory: PathBuf,
    _lock: fs::File,
    io: std::sync::Mutex<std::collections::HashMap<u64, u64>>,
    recent: std::sync::Mutex<std::collections::VecDeque<(PathBuf, WrittenRecord)>>,
    anchors: std::sync::Mutex<std::collections::HashMap<u64, revisions::Anchor>>,
}
impl Recovery {
    pub fn new(root: &Path) -> Result<Self, String> {
        private_dir(root)?;
        let directory = root.join(format!(
            "session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        private_dir(&directory)?;
        let lock = fs::File::create(directory.join("lock")).map_err(|e| e.to_string())?;
        if unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
            return Err("Cannot lock recovery session".into());
        }
        Ok(Self {
            directory,
            _lock: lock,
            io: std::sync::Mutex::new(std::collections::HashMap::new()),
            recent: Default::default(),
            anchors: Default::default(),
        })
    }

    fn write_record(&self, path: &Path, draft: &Draft<impl RecoveryText>) -> Result<(), String> {
        let digest = atomic_json(path, draft)?;
        let record = WrittenRecord {
            digest,
            generation: draft.generation,
            text_bytes: draft.text.byte_len(),
            base_bytes: draft.base.len(),
        };
        let mut recent = self.recent.lock().unwrap();
        recent.retain(|(entry, _)| entry != path);
        if recent.len() == 64 {
            recent.pop_front();
        }
        recent.push_back((path.to_owned(), record));
        Ok(())
    }

    fn known_record(&self, path: &Path) -> std::io::Result<Option<WrittenRecord>> {
        let record = self
            .recent
            .lock()
            .unwrap()
            .iter()
            .find_map(|(entry, record)| (entry == path).then_some(*record));
        let Some(record) = record else {
            return Ok(None);
        };
        Ok((record.digest == record_digest(path)?).then_some(record))
    }

    fn generation(&self, path: &Path) -> std::io::Result<u64> {
        if let Some(record) = self.known_record(path)? {
            return Ok(record.generation);
        }
        #[derive(Deserialize)]
        struct Generation {
            generation: u64,
        }
        Ok(read_recovery_json::<Generation>(path)?.generation)
    }

    fn verified_header(&self, path: &Path, validate_limits: bool) -> std::io::Result<u64> {
        let Some(record) = self.known_record(path)? else {
            return read_recovery_header(path, validate_limits);
        };
        if validate_limits {
            validate_record_limits(record.generation, record.text_bytes, record.base_bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        }
        Ok(record.generation)
    }

    pub(crate) fn write(&self, id: u64, draft: &Draft<impl RecoveryText>) -> Result<(), String> {
        self.write_draft(id, draft, None)
    }

    pub(crate) fn write_revision(
        &self,
        id: u64,
        draft: &Draft<impl RecoveryText>,
        text: &gpui_component::input::Rope,
    ) -> Result<(), String> {
        self.write_draft(id, draft, Some(text))
    }

    fn write_draft(
        &self,
        id: u64,
        draft: &Draft<impl RecoveryText>,
        text: Option<&gpui_component::input::Rope>,
    ) -> Result<(), String> {
        let finished = self.io.lock().unwrap();
        if finished
            .get(&id)
            .is_some_and(|generation| *generation >= draft.generation)
        {
            return Ok(());
        }
        let path = self.directory.join(format!("{id}.json"));
        let revision_path = self.directory.join(format!("{id}.draft"));
        let existing_revision = has_revision(&revision_path)?;
        if existing_revision
            || text.is_some_and(|text| text.len() >= revisions::MIN_SNAPSHOT_BYTES)
                && !path.try_exists().map_err(|error| error.to_string())?
        {
            validate_record_limits(draft.generation, draft.text.byte_len(), draft.base.len())?;
            let mut anchors = self.anchors.lock().unwrap();
            let anchor = anchors.get(&id);
            if existing_revision {
                let generation = match anchor {
                    Some(anchor) => anchor.generation(&revision_path)?,
                    None => revisions::generation(&revision_path)?,
                };
                if generation >= draft.generation {
                    return Ok(());
                }
                if let Some((anchor, text)) = anchor.zip(text)
                    && anchor.write_edit(&revision_path, draft, text)?
                {
                    return Ok(());
                }
            }
            anchors.remove(&id);
            let digest = if existing_revision {
                revisions::full(&revision_path, draft)?
            } else {
                revisions::create(&revision_path, draft)?
            };
            crate::metrics::event(
                "recovery-checkpoint-bytes",
                draft.generation,
                (draft.text.byte_len() + draft.base.len()) as u64,
            );
            if let Some(text) = text {
                anchors.insert(
                    id,
                    revisions::Anchor::capture(&revision_path, draft, text, digest)?,
                );
            }
            return Ok(());
        }
        match self.generation(&path) {
            Ok(existing) => {
                if existing > draft.generation {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Previous recovery record was retained: {error}")),
        }
        self.write_record(&path, draft)
    }
    pub fn claim(&self, source: &Path, id: u64) -> Result<(Draft, Option<String>), String> {
        let parent = source.parent().ok_or("Invalid recovery record")?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(parent.join("lock"))
            .map_err(|e| e.to_string())?;
        if unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&lock),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } != 0
        {
            return Err("This recovery session is active; nothing was imported.".into());
        }
        let draft = read_draft(source)?;
        let extension = if revisions::is_revision(source) {
            "draft"
        } else {
            "json"
        };
        let destination = self.directory.join(format!("{id}.{extension}"));
        crate::fileops::rename(source, &destination)?;
        let warning=fs::File::open(&destination).and_then(|f|f.sync_all()).err().map(|e|format!("Draft restored into this session, but its sync failed: {e}. Save a copy elsewhere."));
        Ok((draft, warning))
    }
    pub(crate) fn checkpoint(
        &self,
        id: u64,
        draft: &Draft<impl RecoveryText>,
    ) -> Result<PathBuf, String> {
        self.write(id, draft)?;
        self.checkpoint_version(id, draft)
    }
    pub(crate) fn checkpoint_version(
        &self,
        id: u64,
        draft: &Draft<impl RecoveryText>,
    ) -> Result<PathBuf, String> {
        let path = self.directory.join(format!(
            "candidate-{id}-{}-{}.json",
            draft.generation,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        self.write_record(&path, draft)?;
        Ok(path)
    }
    pub fn resolved(&self, candidate: &Path) -> Result<(), String> {
        let history = self.directory.parent().unwrap().join("history");
        private_dir(&history)?;
        let destination = history.join(format!(
            "{}-{}",
            self.directory.file_name().unwrap().to_string_lossy(),
            candidate.file_name().unwrap().to_string_lossy()
        ));
        match fs::rename(candidate, &destination) {
            Ok(()) => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && fs::symlink_metadata(&destination).is_ok_and(|meta| meta.is_file()) => {}
            Err(error) => return Err(error.to_string()),
        }
        self.prune_history()
    }
    pub fn prune_history(&self) -> Result<(), String> {
        let history = self.directory.parent().unwrap().join("history");
        let Ok(entries) = fs::read_dir(history) else {
            return Ok(());
        };
        let mut files: Vec<_> = entries
            .flatten()
            .filter_map(|e| {
                e.metadata().ok().filter(|m| m.is_file()).map(|m| {
                    (
                        e.path(),
                        m.len(),
                        m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    )
                })
            })
            .collect();
        files.sort_by_key(|f| f.2);
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (path, bytes, modified) in files {
            if total > 128 * 1024 * 1024
                || modified.elapsed().unwrap_or_default().as_secs() > 7 * 24 * 3600
            {
                fs::remove_file(path).map_err(|e| e.to_string())?;
                total = total.saturating_sub(bytes);
            }
        }
        Ok(())
    }
    pub fn rebind(
        &self,
        id: u64,
        generation: u64,
        from: &crate::document::DiskFile,
        to: &crate::document::DiskFile,
    ) -> Result<(), String> {
        let _finished = self.io.lock().unwrap();
        let revision_path = self.directory.join(format!("{id}.draft"));
        let revision = has_revision(&revision_path)?;
        let path = if revision {
            revision_path
        } else {
            self.directory.join(format!("{id}.json"))
        };
        let mut draft = match read_draft(&path) {
            Ok(draft) => draft,
            Err(_) if !path.try_exists().map_err(|error| error.to_string())? => return Ok(()),
            Err(error) => return Err(error),
        };
        if draft.generation > generation {
            if draft.path != from.path
                || draft.target.as_deref().unwrap_or(std::path::Path::new("")) != from.target
            {
                return Err("Newer recovery text has a different binding; it was retained".into());
            }
            draft.path = to.path.clone();
            draft.target = Some(to.target.clone());
            draft.base = to.bytes.clone();
            if revision {
                self.anchors.lock().unwrap().remove(&id);
                revisions::full(&path, &draft)?;
            } else {
                self.write_record(&path, &draft)?;
            }
        }
        Ok(())
    }
    pub fn saved(&self, id: u64, generation: u64) -> Result<(), String> {
        let mut finished = self.io.lock().unwrap();
        finished
            .entry(id)
            .and_modify(|g| *g = (*g).max(generation))
            .or_insert(generation);
        let revision_path = self.directory.join(format!("{id}.draft"));
        if has_revision(&revision_path)? {
            let mut anchors = self.anchors.lock().unwrap();
            let recovered_generation = match anchors.get(&id) {
                Some(anchor) => anchor.generation(&revision_path)?,
                None => revisions::generation(&revision_path)?,
            };
            if recovered_generation <= generation {
                revisions::retire(&revision_path)?;
                anchors.remove(&id);
            }
        }
        let path = self.directory.join(format!("{id}.json"));
        match self.verified_header(&path, false) {
            Ok(recovered_generation) => {
                if recovered_generation <= generation {
                    fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        for (index, entry) in fs::read_dir(&self.directory)
            .map_err(|error| error.to_string())?
            .enumerate()
        {
            if index >= 100_000 {
                return Err("Recovery cleanup exceeded its scan limit; records retained".into());
            }
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(candidate) = name
                .strip_prefix("candidate-")
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            let parts: Vec<_> = candidate.split('-').collect();
            if parts.len() != 3 || parts[0].parse::<u64>().ok() != Some(id) {
                continue;
            }
            let path = entry.path();
            if self
                .verified_header(&path, true)
                .map_err(|error| error.to_string())?
                <= generation
            {
                self.resolved(&path)?;
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn remove(&self, id: u64) -> Result<(), String> {
        self.saved(id, u64::MAX)
    }
    #[cfg(test)]
    pub fn abandoned(root: &Path) -> Vec<(PathBuf, Draft)> {
        let mut drafts = Vec::new();
        let Ok(sessions) = fs::read_dir(root) else {
            return drafts;
        };
        for session in sessions.flatten() {
            let Ok(lock) = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(session.path().join("lock"))
            else {
                continue;
            };
            if unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            } != 0
            {
                continue;
            }
            if let Ok(entries) = fs::read_dir(session.path()) {
                for entry in entries.flatten().filter(|e| {
                    (e.path().extension().is_some_and(|e| e == "json")
                        || revisions::is_revision(&e.path()))
                        && !e.file_name().to_string_lossy().starts_with("candidate-")
                }) {
                    if let Ok(draft) = read_draft(&entry.path()) {
                        drafts.push((entry.path(), draft));
                    }
                }
            }
        }
        drafts
    }
}

fn has_revision(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err("Recovery draft is not a directory; the existing entry was retained".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

impl Drop for Recovery {
    fn drop(&mut self) {
        if fs::read_dir(&self.directory)
            .ok()
            .is_some_and(|entries| entries.flatten().all(|e| e.file_name() == "lock"))
        {
            let _ = fs::remove_file(self.directory.join("lock"));
            let _ = fs::remove_dir(&self.directory);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_atomic_writes_leave_original_and_no_partial_record() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("record.json");
        fs::write(&path, b"original bytes").unwrap();
        let result = atomic_write(&path, |file| {
            file.write_all(b"partial replacement").unwrap();
            Err("injected interrupted write".into())
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original bytes");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        let draft = draft(9);
        atomic_json(&path, &draft).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            serde_json::to_vec(&draft).unwrap()
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn shared_snapshots_preserve_legacy_recovery_contents() {
        let record = r#"{"path":"original","text":"local β\r\n","base":[239,187,191,98,97,115,101,13,10],"target":null,"generation":4}"#;
        let draft: Draft = serde_json::from_str(record).unwrap();
        assert_eq!(draft.text.as_ref(), "local β\r\n");
        assert_eq!(draft.base.as_bytes().as_ref(), b"\xef\xbb\xbfbase\r\n");
        let disk = crate::document::DiskFile::restore(&draft);
        assert!(std::ptr::eq(
            draft.base.as_bytes().as_ref(),
            disk.bytes.as_bytes().as_ref()
        ));
        assert!(std::ptr::eq(
            disk.content_bytes().as_ref(),
            disk.clone().content_bytes().as_ref()
        ));
        let serialized = serde_json::to_value(&draft).unwrap();
        assert_eq!(serialized["text"], "local β\r\n");
        assert_eq!(serialized["base"], "77u/YmFzZQ0K");
        let restored: Draft = serde_json::from_value(serialized).unwrap();
        assert_eq!(restored.text, draft.text);
        assert_eq!(restored.base, draft.base);
    }

    #[test]
    fn streamed_recovery_bases_preserve_chunk_boundaries_and_wire_format() {
        use base64::Engine;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("record.json");
        for size in [0, 1, 2, 3, 1023, 1024, 1025, 4097, 1024 * 1024] {
            let bytes: Vec<_> = (0..size).map(|index| index as u8).collect();
            let draft = Draft {
                base: bytes.clone().into(),
                ..draft(8)
            };
            atomic_json(&path, &draft).unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                stored["base"],
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            );
            let restored: Draft = serde_json::from_value(stored).unwrap();
            assert_eq!(restored.base.as_bytes().as_ref(), bytes);
            assert_eq!(restored.text, draft.text);
        }
        let source = root.path().join("source.txt");
        let text = "\u{feff}β\r\nx\ry\n".repeat(1025);
        let bytes = [crate::document::BOM, text.as_bytes()].concat();
        fs::write(&source, &bytes).unwrap();
        let (disk, value) = crate::document::DiskFile::open(source).unwrap();
        assert!(std::ptr::eq(
            disk.content_bytes().as_ref(),
            value.as_bytes()
        ));
        let revision = gpui_component::input::Rope::from(value.as_ref());
        for base in [disk.bytes.clone(), disk.share_revision(revision).bytes] {
            let draft = Draft { base, ..draft(8) };
            atomic_json(&path, &draft).unwrap();
            let stored: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                stored["base"],
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            );
            let restored: Draft = serde_json::from_value(stored).unwrap();
            assert_eq!(restored.base, draft.base);
        }
    }

    #[test]
    fn malformed_recovery_generations_are_rejected_without_removing_records() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("record.json");
        for generation in [u64::MAX, u64::MAX - 1] {
            let draft: Draft = Draft {
                path: "original".into(),
                text: "local β".into(),
                base: Default::default(),
                target: None,
                generation,
            };
            let bytes = serde_json::to_vec(&draft).unwrap();
            fs::write(&path, &bytes).unwrap();
            assert!(read_draft(&path).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn recovery_cleanup_validates_large_and_legacy_base_data() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("record.json");
        let mut valid = draft(4);
        valid.base = vec![91; 35_773].into();
        atomic_json(&path, &valid).unwrap();
        assert_eq!(read_recovery_header(&path, true).unwrap(), valid.generation);
        assert_eq!(read_draft(&path).unwrap().base, valid.base);

        let mut wire = serde_json::to_value(&valid).unwrap();
        wire["base"] = serde_json::json!([255, 254, 0, 13, 10]);
        atomic_json(&path, &wire).unwrap();
        assert_eq!(read_recovery_header(&path, true).unwrap(), valid.generation);
        assert_eq!(
            read_draft(&path).unwrap().base.as_bytes().as_ref(),
            [255, 254, 0, 13, 10]
        );
    }

    #[test]
    fn active_recovery_keeps_records_with_unreadable_metadata_or_content() {
        let root = tempfile::tempdir().unwrap();
        let recovery = Recovery::new(root.path()).unwrap();
        let path = recovery.directory.join("1.json");
        let missing_generation = br#"{"text":"prior local text"}"#;
        fs::write(&path, missing_generation).unwrap();
        assert!(recovery.write(1, &draft(2)).is_err());
        assert_eq!(fs::read(&path).unwrap(), missing_generation);

        let mut encoded = "AAAA".repeat(9000);
        encoded.replace_range(17_000..17_004, "AA==");
        let mut malformed = serde_json::to_value(draft(4)).unwrap();
        malformed["base"] = encoded.into();
        let original = serde_json::to_vec(&malformed).unwrap();
        fs::write(&path, &original).unwrap();
        assert!(recovery.saved(1, 4).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn frequent_locations_coalesce_visits_and_decay_old_usage() {
        use std::os::unix::ffi::OsStringExt;
        let mut settings = Settings::default();
        let a = Path::new("/projects/a");
        let b = Path::new("/projects/b");
        assert!(settings.record_location(a, 10000));
        assert!(!settings.record_location(a, 10001));
        assert_eq!(settings.location_visits[0].score, 1.);
        assert!(settings.record_location(a, 12000));
        assert!(settings.record_location(b, 12001));
        assert_eq!(settings.location_visits[0].path, a);
        assert!(settings.record_location(b, 12001 + 86400 * 28));
        assert_eq!(settings.location_visits[0].path, b);
        let last = PathBuf::from(std::ffi::OsString::from_vec(
            b"/projects/last-\xff".to_vec(),
        ));
        settings.last_location = Some(last.clone());
        settings.remember_locations = false;
        assert!(!settings.record_location(a, 9999999));
        let serialized = serde_json::to_vec(&settings).unwrap();
        let restored: Settings = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored.location_visits.len(), 2);
        assert_eq!(restored.last_location, Some(last));
    }

    #[test]
    fn inventory_counts_live_storage_without_exposing_live_records() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        session.write(1, &draft(2)).unwrap();
        let candidate = session.checkpoint_version(1, &draft(3)).unwrap();
        let summary = inventory(root.path()).unwrap();
        assert!(summary.records.is_empty());
        assert_eq!(summary.usage.files, 3);
        assert!(summary.usage.bytes > 0);
        assert!(!summary.usage.partial);
        assert!(restore_snapshot(&candidate, false).is_err());
        assert!(trash_record(&candidate).is_err());
        assert!(candidate.exists());
        drop(session);
        let summary = inventory(root.path()).unwrap();
        assert_eq!(summary.records.len(), 2);
        assert_eq!(summary.records.iter().filter(|r| r.candidate).count(), 1);
    }
    #[test]
    fn inventory_row_cap_does_not_truncate_storage_totals() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        for id in 0..257 {
            session.write(id, &draft(id)).unwrap();
        }
        let expected: u64 = fs::read_dir(&session.directory)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        drop(session);
        let summary = inventory(root.path()).unwrap();
        assert_eq!(summary.records.len(), 256);
        assert!(summary.records_truncated);
        assert_eq!(summary.usage.files, 258);
        assert_eq!(summary.usage.bytes, expected);
        assert!(!summary.usage.partial);
    }
    #[test]
    fn observed_history_is_a_separate_unbound_draft_with_exact_text_and_bom() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        let mut value = draft(7);
        value.text = "candidate\n".into();
        value.base = b"\xef\xbb\xbfobserved\r\n".as_slice().into();
        let path = session.checkpoint_version(1, &value).unwrap();
        session.resolved(&path).unwrap();
        let summary = inventory(root.path()).unwrap();
        let path = &summary.records.iter().find(|r| r.history).unwrap().path;
        let observed = restore_snapshot(path, true).unwrap();
        assert_eq!(observed.text.as_ref(), "observed\r\n");
        assert_eq!(observed.base.as_bytes().as_ref(), [0xef, 0xbb, 0xbf]);
        assert!(observed.target.is_none());
        assert_eq!(
            restore_snapshot(path, false).unwrap().text.as_ref(),
            "candidate\n"
        );
        assert!(path.exists());
    }
    fn draft(generation: u64) -> Draft {
        Draft {
            path: "missing".into(),
            text: "local 🦀".into(),
            base: b"disk".as_slice().into(),
            target: None,
            generation,
        }
    }
    #[test]
    fn resolving_a_document_archives_only_its_owned_candidates_through_the_generation() {
        let root = tempfile::tempdir().unwrap();
        let recovery = Recovery::new(root.path()).unwrap();
        let old = recovery.checkpoint(7, &draft(3)).unwrap();
        let newer = recovery.checkpoint_version(7, &draft(4)).unwrap();
        let other = recovery.checkpoint_version(8, &draft(2)).unwrap();
        let legacy = recovery.directory.join("candidate-7-123456789.json");
        fs::write(&legacy, serde_json::to_vec(&draft(1)).unwrap()).unwrap();
        recovery.saved(7, 3).unwrap();
        assert!(!old.exists());
        assert!(newer.exists());
        assert!(other.exists());
        assert!(legacy.exists());
        recovery.resolved(&old).unwrap();
        let history = inventory(root.path())
            .unwrap()
            .records
            .into_iter()
            .find(|record| record.history)
            .unwrap();
        assert_eq!(read_draft(&history.path).unwrap().text, draft(3).text);
        assert_eq!(read_draft(&history.path).unwrap().base, draft(3).base);
    }

    #[test]
    fn written_record_with_unchanged_timestamp_is_revalidated_after_corruption() {
        let root = tempfile::tempdir().unwrap();
        let recovery = Recovery::new(root.path()).unwrap();
        let id = 7;
        let generation = 4;
        recovery.write(id, &draft(generation)).unwrap();
        let path = recovery.directory.join("7.json");
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut corrupt = fs::read(&path).unwrap();
        let last = corrupt.len() - 1;
        assert_eq!(corrupt[last], b'}');
        corrupt[last] = b'?';
        fs::write(&path, &corrupt).unwrap();
        fs::File::open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert!(recovery.write(id, &draft(generation + 1)).is_err());
        assert!(recovery.saved(id, generation).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn saved_generation_cannot_recur_or_remove_a_newer_edit() {
        let root = tempfile::tempdir().unwrap();
        let r = Recovery::new(root.path()).unwrap();
        r.write(1, &draft(4)).unwrap();
        r.saved(1, 4).unwrap();
        r.write(1, &draft(4)).unwrap();
        assert!(!r.directory.join("1.json").exists());
        r.write(1, &draft(5)).unwrap();
        r.saved(1, 4).unwrap();
        let value: Draft =
            serde_json::from_slice(&fs::read(r.directory.join("1.json")).unwrap()).unwrap();
        assert_eq!(value.generation, 5);
        r.remove(1).unwrap();
        r.write(1, &draft(6)).unwrap();
        assert!(!r.directory.join("1.json").exists());
    }
    #[test]
    fn typography_defaults_preserve_explicit_legacy_settings() {
        let defaults = Settings::default();
        assert_eq!(defaults.font_size, 14.5);
        assert_eq!(defaults.line_height, 1.4);
        let legacy: Settings =
            serde_json::from_str(r#"{"version":1,"font_size":19,"compact":true}"#).unwrap();
        assert_eq!(legacy.font_size, 19.);
        assert!(legacy.compact);
        assert_eq!(legacy.line_height, defaults.line_height);
        let mut value = legacy.clone();
        value.line_height = 1.7;
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(restored.line_height, 1.7);
        value.line_height = f32::NAN;
        assert!(value.validate().is_err());
        value.line_height = 0.9;
        assert!(value.validate().is_err());
    }

    #[test]
    fn settings_keep_last_valid_and_refuse_external_clobber() {
        let root = tempfile::tempdir().unwrap();
        let mut base = None;
        let first = Settings::default();
        first.save_checked(root.path(), &mut base).unwrap();
        let mut second = first.clone();
        second.font_size = 19.;
        second.save_checked(root.path(), &mut base).unwrap();
        fs::write(root.path().join("settings.json"), b"invalid").unwrap();
        assert!(second.save_checked(root.path(), &mut base).is_err());
        let loaded = Settings::load_in(root.path()).unwrap();
        assert!(loaded.warning.is_some());
        assert_eq!(loaded.font_size, first.font_size);
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            b"invalid"
        );
    }
    #[test]
    fn invalid_first_settings_require_explicit_replacement_and_keep_original_bytes() {
        let root = tempfile::tempdir().unwrap();
        let original = b"{invalid settings";
        fs::write(root.path().join("settings.json"), original).unwrap();
        fs::write(root.path().join("settings.previous.json"), b"also invalid").unwrap();
        let mut settings = Settings::load_in(root.path()).unwrap();
        assert!(
            settings
                .warning
                .as_ref()
                .unwrap()
                .contains("defaults loaded")
        );
        let mut base = settings.disk_base.clone();
        assert!(settings.save_checked(root.path(), &mut base).is_err());
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            original
        );
        settings.warning = None;
        settings.save_checked(root.path(), &mut base).unwrap();
        assert_eq!(
            fs::read(root.path().join("settings.rejected.json")).unwrap(),
            original
        );
        assert!(Settings::load_in(root.path()).unwrap().warning.is_none());
    }

    #[test]
    fn recovery_preserves_native_non_unicode_paths() {
        use std::os::unix::ffi::OsStringExt;
        let mut value = draft(9);
        value.path = std::ffi::OsString::from_vec(b"/tmp/non-utf8-\xff".to_vec()).into();
        value.target = Some(value.path.clone());
        let bytes = serde_json::to_vec(&value).unwrap();
        let restored: Draft = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.path, value.path);
        assert_eq!(restored.target, value.target);
    }
    #[test]
    fn oversized_settings_never_replace_a_readable_file() {
        let root = tempfile::tempdir().unwrap();
        let mut expected = None;
        let valid = Settings::default();
        valid.save_checked(root.path(), &mut expected).unwrap();
        let original = fs::read(root.path().join("settings.json")).unwrap();
        let mut large = valid.clone();
        large.search_exclude = "x".repeat(65536);
        assert!(large.save_checked(root.path(), &mut expected).is_err());
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            original
        );
        assert!(Settings::load_in(root.path()).is_ok());
        large = valid;
        large.pins = (0..128)
            .map(|i| PathBuf::from(format!("/{i}/{}", "x".repeat(1024))))
            .collect();
        assert!(large.save_checked(root.path(), &mut expected).is_err());
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            original
        );
        large.pins = vec![PathBuf::from("x".repeat(1024))];
        while serde_json::to_vec_pretty(&large).unwrap().len() <= 65536 {
            large.pins[0].as_mut_os_string().push("xxxxxxxxxxxxxxxx");
        }
        assert!(serde_json::to_vec(&large).unwrap().len() <= 65536);
        assert!(large.save_checked(root.path(), &mut expected).is_err());
        assert_eq!(
            fs::read(root.path().join("settings.json")).unwrap(),
            original
        );
    }

    #[test]
    fn settings_persist_atomically_with_private_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings {
            font_size: 19.0,
            wrap: false,
            compact: true,
            ..Default::default()
        };
        atomic(&path, &serde_json::to_vec(&settings).unwrap()).unwrap();
        let restored: Settings = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.font_size, 19.0);
        assert!(!restored.wrap);
        assert!(restored.compact);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn recovery_exact_generation_and_live_isolation() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        session.write(3, &draft(42)).unwrap();
        session.write(3, &draft(2)).unwrap();
        assert!(Recovery::abandoned(root.path()).is_empty());
        drop(session);
        let drafts = Recovery::abandoned(root.path());
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].1.generation, 42);
        assert_eq!(drafts[0].1.text.as_ref(), "local 🦀");
    }
    #[test]
    fn import_transfers_once_and_preserves_live_ownership() {
        let root = tempfile::tempdir().unwrap();
        let source = Recovery::new(root.path()).unwrap();
        source.write(7, &draft(19)).unwrap();
        let path = source.directory.join("7.json");
        let target = Recovery::new(root.path()).unwrap();
        assert!(target.claim(&path, 5).is_err());
        drop(source);
        target.claim(&path, 5).unwrap();
        assert!(!path.exists());
        assert!(target.claim(&path, 6).is_err());
        assert!(Recovery::abandoned(root.path()).is_empty());
        drop(target);
        assert_eq!(Recovery::abandoned(root.path()).len(), 1);
    }
    #[test]
    fn resolved_history_is_separate_from_unresolved_drafts() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        let path = session.checkpoint(0, &draft(1)).unwrap();
        session.resolved(&path).unwrap();
        assert!(!path.exists());
        session.remove(0).unwrap();
        drop(session);
        assert!(Recovery::abandoned(root.path()).is_empty());
        assert_eq!(
            fs::read_dir(root.path().join("history")).unwrap().count(),
            1
        );
    }
    #[test]
    fn recovery_failure_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let session = Recovery::new(root.path()).unwrap();
        fs::remove_dir_all(&session.directory).unwrap();
        fs::write(&session.directory, b"blocked").unwrap();
        assert!(session.write(0, &draft(0)).is_err());
    }
}

mod native_path {
    use super::*;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    #[derive(Deserialize)]
    #[serde(untagged)]
    pub enum Encoded {
        Bytes(Vec<u8>),
        Legacy(String),
    }
    fn decode(value: Encoded) -> PathBuf {
        match value {
            Encoded::Bytes(bytes) => std::ffi::OsString::from_vec(bytes).into(),
            Encoded::Legacy(text) => text.into(),
        }
    }
    pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        path.as_os_str().as_bytes().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        Encoded::deserialize(deserializer).map(decode)
    }
    pub fn serialize_optional<S: serde::Serializer>(
        path: &Option<PathBuf>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        path.as_ref()
            .map(|p| p.as_os_str().as_bytes())
            .serialize(serializer)
    }
    pub fn deserialize_optional<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<PathBuf>, D::Error> {
        Option::<Encoded>::deserialize(deserializer).map(|p| p.map(decode))
    }
    pub fn serialize_many<S: serde::Serializer>(
        paths: &[PathBuf],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        paths
            .iter()
            .map(|p| p.as_os_str().as_bytes())
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
    pub fn deserialize_many<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<PathBuf>, D::Error> {
        Vec::<Encoded>::deserialize(deserializer).map(|p| p.into_iter().map(decode).collect())
    }
}

mod encoded_bytes {
    use super::*;
    use base64::Engine;
    use std::{fmt, io};

    struct FormatWriter<'a, 'b>(&'a mut fmt::Formatter<'b>);

    impl io::Write for FormatWriter<'_, '_> {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let encoded = std::str::from_utf8(bytes).map_err(io::Error::other)?;
            self.0.write_str(encoded).map_err(io::Error::other)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct EncodedBytes<'a>(&'a crate::document::FileBytes);

    impl fmt::Display for EncodedBytes<'_> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let mut encoder = base64::write::EncoderWriter::new(
                FormatWriter(formatter),
                &base64::engine::general_purpose::STANDARD,
            );
            self.0.write_to(&mut encoder).map_err(|_| fmt::Error)?;
            encoder.finish().map_err(|_| fmt::Error)?;
            Ok(())
        }
    }

    pub fn serialize<S: serde::Serializer>(
        bytes: &crate::document::FileBytes,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&EncodedBytes(bytes))
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<crate::document::FileBytes, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Value {
            Encoded(String),
            Legacy(Vec<u8>),
        }
        match Value::deserialize(deserializer)? {
            Value::Legacy(bytes) => Ok(bytes.into()),
            Value::Encoded(value) => base64::engine::general_purpose::STANDARD
                .decode(value)
                .map(Into::into)
                .map_err(serde::de::Error::custom),
        }
    }

    pub fn decoded_len<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<usize, D::Error> {
        struct DecodedLength;

        impl<'de> serde::de::Visitor<'de> for DecodedLength {
            type Value = usize;

            fn expecting(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                fmt.write_str("a base64 string or an array of bytes")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<usize, E> {
                let engine = &base64::engine::general_purpose::STANDARD;
                let mut decoded = [0; 12 * 1024];
                let mut chunks = value.as_bytes().chunks(16 * 1024).peekable();
                let mut length = 0;
                while let Some(chunk) = chunks.next() {
                    if chunks.peek().is_some() && chunk.contains(&b'=') {
                        return Err(E::custom("Padding before the end of the recovery base"));
                    }
                    length += engine
                        .decode_slice(chunk, &mut decoded)
                        .map_err(E::custom)?;
                }
                Ok(length)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<usize, A::Error> {
                let mut length = 0;
                while seq.next_element::<u8>()?.is_some() {
                    length += 1;
                }
                Ok(length)
            }
        }

        deserializer.deserialize_any(DecodedLength)
    }
}
