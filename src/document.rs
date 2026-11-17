#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const BOM: &[u8] = b"\xef\xbb\xbf";
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub enum FileBytes {
    Raw(Arc<[u8]>),
    WithBom(Arc<str>),
    OwnedText { text: Arc<String>, bom: bool },
}

impl FileBytes {
    fn from_text(text: Arc<str>, bom: bool) -> Self {
        if bom {
            Self::WithBom(text)
        } else {
            Self::Raw(text.into())
        }
    }

    pub fn parts(&self) -> (&[u8], &[u8]) {
        match self {
            Self::Raw(bytes) => (&[], bytes),
            Self::WithBom(text) => (BOM, text.as_bytes()),
            Self::OwnedText { text, bom } => (if *bom { BOM } else { &[] }, text.as_bytes()),
        }
    }

    pub fn len(&self) -> usize {
        let (prefix, bytes) = self.parts();
        prefix.len() + bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        let (prefix, bytes) = self.parts();
        prefix.is_empty() && bytes.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn as_bytes(&self) -> std::borrow::Cow<'_, [u8]> {
        let (prefix, bytes) = self.parts();
        if prefix.is_empty() {
            std::borrow::Cow::Borrowed(bytes)
        } else {
            std::borrow::Cow::Owned([prefix, bytes].concat())
        }
    }

    pub(crate) fn has_bom(&self) -> bool {
        let (prefix, bytes) = self.parts();
        !prefix.is_empty() || bytes.starts_with(BOM)
    }

    pub(crate) fn content_bytes(&self, bom: bool) -> &[u8] {
        let (prefix, bytes) = self.parts();
        if prefix.is_empty() && bom {
            bytes.strip_prefix(BOM).unwrap_or(bytes)
        } else {
            bytes
        }
    }

    fn write_to(&self, mut writer: impl Write) -> std::io::Result<()> {
        let (prefix, body) = self.parts();
        writer.write_all(prefix)?;
        writer.write_all(body)
    }
}

impl Default for FileBytes {
    fn default() -> Self {
        Self::Raw(Arc::default())
    }
}

impl From<Vec<u8>> for FileBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Raw(bytes.into())
    }
}

impl From<&[u8]> for FileBytes {
    fn from(bytes: &[u8]) -> Self {
        Self::Raw(bytes.into())
    }
}

impl PartialEq for FileBytes {
    fn eq(&self, other: &Self) -> bool {
        let (left_prefix, left) = self.parts();
        let (right_prefix, right) = other.parts();
        if left_prefix == right_prefix {
            left == right
        } else if left_prefix.is_empty() {
            left.strip_prefix(right_prefix) == Some(right)
        } else {
            right.strip_prefix(left_prefix) == Some(left)
        }
    }
}

#[derive(Clone)]
pub struct DiskFile {
    pub path: PathBuf,
    pub target: PathBuf,
    pub bytes: FileBytes,
    pub bom: bool,
    identity: (u64, u64),
}

pub enum Observation {
    Unchanged(DiskFile),
    Changed(DiskFile, Arc<str>),
}

fn identity(meta: &Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

fn open_regular(path: &Path) -> Result<(fs::File, Metadata)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Only regular text files can be opened.".into());
    }
    if meta.len() > MAX_BYTES {
        return Err("Files larger than 10 MiB are not supported yet.".into());
    }
    Ok((file, meta))
}

fn read_chunks(
    mut file: fs::File,
    cancelled: &dyn Fn() -> bool,
    mut consume: impl FnMut(usize, &[u8]),
) -> Result<usize> {
    let mut length = 0;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let read = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if read == 0 {
            return Ok(length);
        }
        if (length + read) as u64 > MAX_BYTES {
            return Err("File grew beyond 10 MiB while reading.".into());
        }
        consume(length, &chunk[..read]);
        length += read;
    }
}

fn read_cancellable(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<(Vec<u8>, Metadata)> {
    let (file, meta) = open_regular(path)?;
    let mut bytes = Vec::with_capacity(meta.len() as usize + 1);
    read_chunks(file, cancelled, |_, chunk| bytes.extend_from_slice(chunk))?;
    Ok((bytes, meta))
}

fn compare_regular(path: &Path, expected: &FileBytes) -> Result<(bool, Metadata)> {
    let (file, meta) = open_regular(path)?;
    let (header, body) = expected.parts();
    let mut equal = true;
    let length = read_chunks(file, &|| false, |offset, chunk| {
        let prefix = header.get(offset..).unwrap_or_default();
        let prefix_len = prefix.len().min(chunk.len());
        let rest = &chunk[prefix_len..];
        let start = offset.saturating_sub(header.len());
        equal &= prefix[..prefix_len] == chunk[..prefix_len]
            && body.get(start..start + rest.len()) == Some(rest);
    })?;
    Ok((equal && length == expected.len(), meta))
}

fn supported_text(bytes: &[u8]) -> Result<&str> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Only UTF-8 text is supported; the file was not changed.")?;
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("This file contains binary/control data and cannot be edited.".into());
    }
    Ok(text)
}

impl DiskFile {
    pub fn content_bytes(&self) -> &[u8] {
        self.bytes.content_bytes(self.bom)
    }

    pub fn draft() -> Self {
        Self {
            path: PathBuf::from("Untitled"),
            target: PathBuf::new(),
            bytes: Default::default(),
            bom: false,
            identity: (0, 0),
        }
    }

    pub fn restore(draft: &crate::storage::Draft) -> Self {
        let mut disk = if draft
            .target
            .as_ref()
            .is_some_and(|p| !p.as_os_str().is_empty())
        {
            Self::open(draft.path.clone())
                .map(|v| v.0)
                .unwrap_or_else(|_| Self::draft())
        } else {
            Self::draft()
        };
        disk.path = draft.path.clone();
        disk.bytes = draft.base.clone();
        disk.bom = draft.base.has_bom();
        disk.target = draft.target.clone().unwrap_or_default();
        disk
    }
    pub fn same_file(&self, other: &Self) -> bool {
        (!self.target.as_os_str().is_empty() && self.target == other.target)
            || (self.identity != (0, 0) && self.identity == other.identity)
    }

    pub fn open(path: PathBuf) -> Result<(Self, Arc<str>)> {
        Self::open_cancellable(path, &|| false)
    }
    pub fn open_cancellable(
        path: PathBuf,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(Self, Arc<str>)> {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let target = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        let (bytes, meta) = read_cancellable(&target, cancelled)?;
        let bom = bytes.starts_with(BOM);
        let content = if bom { &bytes[3..] } else { &bytes };
        let text = supported_text(content)?;
        let text: Arc<str> = text.into();
        Ok((
            Self {
                path,
                target,
                bytes: FileBytes::from_text(text.clone(), bom),
                bom,
                identity: identity(&meta),
            },
            text,
        ))
    }

    pub(crate) fn observe(mut self) -> Result<Observation> {
        let target = fs::canonicalize(&self.path).map_err(|e| e.to_string())?;
        if target == self.target {
            let (equal, metadata) = compare_regular(&target, &self.bytes)?;
            if equal && self.bom == self.bytes.has_bom() {
                supported_text(self.content_bytes())?;
                self.identity = identity(&metadata);
                return Ok(Observation::Unchanged(self));
            }
        }
        let (disk, text) = Self::open(self.path)?;
        Ok(Observation::Changed(disk, text))
    }

    pub fn create(path: PathBuf, text: &str, bom: bool) -> Result<Self> {
        Self::create_with(path, text, bom, || {})
    }
    fn create_with(
        path: PathBuf,
        text: &str,
        bom: bool,
        after_publish: impl FnOnce(),
    ) -> Result<Self> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = fs::canonicalize(parent).map_err(|error| error.to_string())?;
        let target = directory.join(path.file_name().ok_or("Choose a filename")?);
        let bytes = FileBytes::from_text(text.into(), bom);
        if bytes.len() as u64 > MAX_BYTES {
            return Err("The new file exceeds the 10 MiB editing limit".into());
        }
        let mut staged =
            tempfile::NamedTempFile::new_in(&directory).map_err(|error| error.to_string())?;
        bytes
            .write_to(&mut staged)
            .and_then(|_| staged.as_file().sync_all())
            .map_err(|error| error.to_string())?;
        let expected_identity = identity(
            &staged
                .as_file()
                .metadata()
                .map_err(|error| error.to_string())?,
        );
        if fs::canonicalize(parent).map_err(|error| error.to_string())? != directory {
            return Err(
                "The destination folder changed before creation; nothing was published".into(),
            );
        }
        staged
            .persist_noclobber(&target)
            .map_err(|error| error.to_string())?;
        after_publish();
        let message = format!(
            "The destination changed after creating {}. The current buffer was retained; review the destination before saving again.",
            crate::paths::display(&target)
        );
        if fs::canonicalize(&path).map_err(|_| message.clone())? != target {
            return Err(message);
        }
        let (equal, meta) = compare_regular(&target, &bytes).map_err(|_| message.clone())?;
        if identity(&meta) != expected_identity || !equal {
            return Err(message);
        }
        Ok(Self {
            path,
            target,
            bytes,
            bom,
            identity: expected_identity,
        })
    }

    pub fn revalidate_paths(&self, paths: &[PathBuf]) -> Result<()> {
        for path in paths {
            if fs::canonicalize(path).map_err(|e| e.to_string())? != self.target {
                return Err(format!(
                    "Requested destination binding changed: {}",
                    path.display()
                ));
            }
        }
        let (equal, _) = compare_regular(&self.target, &self.bytes)?;
        if !equal {
            return Err(
                "Requested destination bytes changed after the last observed version".into(),
            );
        }
        Ok(())
    }
    fn validate(&self) -> Result<Metadata> {
        if fs::canonicalize(&self.path).map_err(|e| e.to_string())? != self.target {
            return Err("The file's destination changed. Your text is still in this tab.".into());
        }
        let (equal, meta) = compare_regular(&self.target, &self.bytes)?;
        if identity(&meta) != self.identity || !equal {
            return Err(
                "The file changed on disk. Saving was stopped; your text is still in this tab."
                    .into(),
            );
        }
        if meta.nlink() != 1 {
            return Err(
                "Saving hard-linked files is not supported; the links were left intact.".into(),
            );
        }
        if meta.permissions().readonly() {
            return Err("The file is read-only.".into());
        }
        Ok(meta)
    }

    pub fn save(&self, text: &str) -> Result<Self> {
        self.save_with_bom(text, self.bom)
    }

    pub fn save_snapshot(&self, text: Arc<str>) -> Result<Self> {
        self.save_bytes(FileBytes::from_text(text, self.bom), self.bom)
    }

    pub(crate) fn save_owned_snapshot(&self, text: String) -> Result<Self> {
        self.save_bytes(
            FileBytes::OwnedText {
                text: Arc::new(text),
                bom: self.bom,
            },
            self.bom,
        )
    }

    pub fn save_with_bom(&self, text: &str, bom: bool) -> Result<Self> {
        self.save_bytes(FileBytes::from_text(text.into(), bom), bom)
    }

    fn save_bytes(&self, bytes: FileBytes, bom: bool) -> Result<Self> {
        let meta = self.validate()?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("The edited file exceeds the 10 MiB limit.".into());
        }
        if bytes == self.bytes {
            return Ok(self.clone());
        }
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o7000 != 0 {
            return Err(
                "Saving files with another owner or special mode bits is not supported.".into(),
            );
        }
        let parent = self
            .target
            .parent()
            .ok_or("The file has no parent folder.")?;
        let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(&self.target)
            .map_err(|e| e.to_string())?;
        if identity(&source.metadata().map_err(|e| e.to_string())?) != self.identity {
            return Err("The file changed before saving.".into());
        }
        #[cfg(target_os = "macos")]
        {
            let copied = unsafe {
                libc::fcopyfile(
                    source.as_raw_fd(),
                    staged.as_file().as_raw_fd(),
                    std::ptr::null_mut(),
                    libc::COPYFILE_METADATA,
                )
            };
            if copied != 0 {
                return Err(format!(
                    "Cannot preserve file metadata: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;
            use xattr::FileExt;
            if unsafe { libc::fchown(staged.as_file().as_raw_fd(), meta.uid(), meta.gid()) } != 0 {
                return Err(format!(
                    "Cannot preserve ownership: {}",
                    std::io::Error::last_os_error()
                ));
            }
            staged
                .as_file()
                .set_permissions(meta.permissions())
                .map_err(|e| e.to_string())?;
            for key in source.list_xattr().map_err(|e| e.to_string())? {
                let value = source
                    .get_xattr(&key)
                    .map_err(|e| e.to_string())?
                    .ok_or("File metadata changed")?;
                staged
                    .as_file()
                    .set_xattr(&key, &value)
                    .map_err(|e| e.to_string())?;
            }
        }
        bytes.write_to(&mut staged).map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        self.validate()?;
        staged
            .persist(&self.target)
            .map_err(|e| e.error.to_string())?;
        let saved_meta = fs::symlink_metadata(&self.target).map_err(|e| e.to_string())?;
        Ok(Self {
            bytes,
            bom,
            identity: identity(&saved_meta),
            ..self.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        fs::write(&path, bytes).unwrap();
        (dir, path)
    }
    #[test]
    fn creating_a_file_never_adopts_a_post_publish_replacement_or_retarget() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.txt");
        let kept = root.path().join("created-copy.txt");
        let other = root.path().join("other.txt");
        fs::write(&other, "\u{feff}local β\r\n".as_bytes()).unwrap();
        let result = DiskFile::create_with(path.clone(), "local β\r\n", true, || {
            fs::rename(&path, &kept).unwrap();
            std::os::unix::fs::symlink(&other, &path).unwrap();
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&kept).unwrap(), "\u{feff}local β\r\n".as_bytes());
        assert_eq!(fs::read(&other).unwrap(), "\u{feff}local β\r\n".as_bytes());
        fs::remove_file(&path).unwrap();
        let result = DiskFile::create_with(path.clone(), "candidate", false, || {
            fs::rename(&path, root.path().join("second-created-copy.txt")).unwrap();
            fs::write(&path, "external version").unwrap();
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"external version");
        assert_eq!(
            fs::read(root.path().join("second-created-copy.txt")).unwrap(),
            b"candidate"
        );
    }
    #[test]
    fn creating_a_bound_file_is_exclusive_and_retains_exact_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.txt");
        let disk = DiskFile::create(path.clone(), "local β\r\n", true).unwrap();
        assert_eq!(
            disk.bytes.as_bytes().as_ref(),
            "\u{feff}local β\r\n".as_bytes()
        );
        disk.revalidate_paths(std::slice::from_ref(&path)).unwrap();
        assert!(DiskFile::create(path.clone(), "replacement", false).is_err());
        assert_eq!(
            fs::read(path).unwrap().as_slice(),
            disk.bytes.as_bytes().as_ref()
        );
    }

    #[test]
    fn save_shares_the_worker_snapshot_and_compares_exact_bytes() {
        let before = "\u{feff}original\n";
        let text = "local 🦀\r\nsecond\rthird\n";
        let (_dir, path) = fixture(before.as_bytes());
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        let snapshot = text.to_owned();
        let allocation = snapshot.as_ptr();
        let saved = disk.save_owned_snapshot(snapshot).unwrap();
        assert_eq!(saved.content_bytes().as_ptr(), allocation);
        let full_text = format!("\u{feff}{text}");
        assert_eq!(fs::read(path).unwrap(), full_text.as_bytes());
        assert_eq!(saved.bytes, FileBytes::from(full_text.as_bytes()));
        assert_eq!(saved.bytes, FileBytes::from_text(text.into(), true));
        assert_eq!(
            saved.bytes,
            FileBytes::OwnedText {
                text: Arc::new(full_text),
                bom: false
            }
        );
    }

    #[test]
    fn untouched_bytes_and_edited_bom_are_preserved() {
        for bytes in [
            b"\xef\xbb\xbfhello\r\nworld\r\n".as_slice(),
            b"one\rtwo\nthree",
            "héllo 🦀".as_bytes(),
            b"",
        ] {
            let (_dir, path) = fixture(bytes);
            let (disk, text) = DiskFile::open(path.clone()).unwrap();
            assert!(std::ptr::eq(disk.content_bytes(), text.as_bytes()));
            disk.save(&text).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
            disk.save(&format!("{text}!")).unwrap();
            assert_eq!(fs::read(path).unwrap(), [bytes, b"!"].concat());
        }
    }
    #[test]
    fn observation_reuses_equal_bytes_and_checks_new_content_and_destinations() {
        let text = "\u{feff}local 🦀\r\nsecond\rthird\n";
        let (dir, path) = fixture(text.as_bytes());
        let alias = dir.path().join("linked.txt");
        symlink(&path, &alias).unwrap();
        let (disk, _) = DiskFile::open(alias.clone()).unwrap();
        let equal = dir.path().join("replacement.txt");
        fs::write(&equal, text).unwrap();
        fs::rename(&equal, &path).unwrap();
        let Observation::Unchanged(matching) = disk.clone().observe().unwrap() else {
            panic!("Identical bytes should reuse the saved base");
        };
        assert_ne!(matching.identity, disk.identity);
        assert_eq!(
            matching.content_bytes().as_ptr(),
            disk.content_bytes().as_ptr()
        );
        matching.validate().unwrap();

        let changed = "\u{feff}local 🦀\r\nsecond\rThird\n";
        fs::write(&path, changed).unwrap();
        let Observation::Changed(new_disk, new_text) = matching.clone().observe().unwrap() else {
            panic!("Same-length changes must be observed");
        };
        assert_eq!(new_text.as_ref(), changed.trim_start_matches('\u{feff}'));
        assert_eq!(new_disk.bytes.as_bytes().as_ref(), changed.as_bytes());

        fs::write(&equal, text).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(&equal, &alias).unwrap();
        let Observation::Changed(retargeted, _) = matching.observe().unwrap() else {
            panic!("Retargeted paths must be observed even when their bytes match");
        };
        assert_eq!(retargeted.target, equal.canonicalize().unwrap());
        assert_eq!(retargeted.bytes.as_bytes().as_ref(), text.as_bytes());
    }

    #[test]
    fn unchanged_observation_preserves_file_decoding_rules() {
        let (_dir, path) = fixture(b"original");
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        let saved = disk
            .save_owned_snapshot("\u{feff}different".to_owned())
            .unwrap();
        let Observation::Changed(decoded, body) = saved.observe().unwrap() else {
            panic!("A leading BOM changes the decoded document");
        };
        assert!(decoded.bom);
        assert_eq!(body.as_ref(), "different");
        let unsupported = decoded.save_owned_snapshot("nul\0".to_owned()).unwrap();
        assert_eq!(unsupported.observe().err(), DiskFile::open(path).err(),);
    }

    #[test]
    fn same_length_external_change_and_replacement_block_save() {
        let (_dir, path) = fixture(b"base");
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        fs::write(&path, b"else").unwrap();
        assert!(disk.save("mine").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"else");
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"base").unwrap();
        assert!(disk.save("mine").is_err());
    }

    #[test]
    fn streamed_comparison_checks_bom_and_later_file_chunks() {
        let original = "\u{feff}".to_owned() + &"abcé\r\n".repeat(30_000);
        let (_dir, path) = fixture(original.as_bytes());
        let (disk, source) = DiskFile::open(path.clone()).unwrap();
        assert!(disk.save(&source).is_ok());
        let mut external = original.into_bytes();
        let changed_byte = external
            .iter_mut()
            .skip(64 * 1024)
            .find(|byte| **byte == b'a')
            .unwrap();
        *changed_byte = b'z';
        fs::write(&path, &external).unwrap();
        assert!(disk.revalidate_paths(std::slice::from_ref(&path)).is_err());
        assert!(disk.save("unsaved").is_err());
        assert_eq!(fs::read(&path).unwrap(), external);
    }
    #[test]
    fn symlink_is_preserved_and_retargeting_is_rejected() {
        let (dir, path) = fixture(b"base");
        let link = dir.path().join("alias.txt");
        symlink(&path, &link).unwrap();
        let (disk, _) = DiskFile::open(link.clone()).unwrap();
        disk.save("edited").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let other = dir.path().join("other.txt");
        fs::write(&other, b"base").unwrap();
        fs::remove_file(&link).unwrap();
        symlink(&other, &link).unwrap();
        assert!(disk.save("mine").is_err());
    }
    #[test]
    fn hardlinks_and_missing_or_readonly_destinations_are_preserved() {
        let (dir, path) = fixture(b"base");
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        let link = dir.path().join("hard.txt");
        fs::hard_link(&path, &link).unwrap();
        assert!(disk.save("mine").is_err());
        fs::remove_file(link).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();
        assert!(disk.save("mine").is_err());
        fs::remove_file(path).unwrap();
        assert!(disk.save("mine").is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn save_preserves_linux_metadata() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("metadata.txt");
        fs::write(&path, "before").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        xattr::set(&path, "user.glimpse", b"preserved").unwrap();
        let meta = fs::metadata(&path).unwrap();
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        disk.save("after").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"after");
        let saved = fs::metadata(&path).unwrap();
        assert_eq!(saved.mode() & 0o777, 0o640);
        assert_eq!((saved.uid(), saved.gid()), (meta.uid(), meta.gid()));
        assert_eq!(
            xattr::get(&path, "user.glimpse").unwrap().unwrap(),
            b"preserved"
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn save_preserves_mode_and_macos_extended_attributes() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::PermissionsExt},
        };
        let (_dir, path) = fixture(b"base");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let native_path = CString::new(path.as_os_str().as_bytes()).unwrap();
        let attribute = CString::new("com.glimpse.fixture").unwrap();
        let value = b"metadata";
        assert_eq!(
            unsafe {
                libc::setxattr(
                    native_path.as_ptr(),
                    attribute.as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            },
            0
        );
        let (disk, _) = DiskFile::open(path.clone()).unwrap();
        disk.save("edited").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"edited");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
        let mut result = [0_u8; 8];
        assert_eq!(
            unsafe {
                libc::getxattr(
                    native_path.as_ptr(),
                    attribute.as_ptr(),
                    result.as_mut_ptr().cast(),
                    result.len(),
                    0,
                    0,
                )
            },
            8
        );
        assert_eq!(&result, value);
    }
    #[test]
    fn unsupported_files_are_rejected_without_mutation() {
        for bytes in [b"\xff".as_slice(), b"zero\0byte"] {
            let (_dir, path) = fixture(bytes);
            assert!(DiskFile::open(path.clone()).is_err());
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        let (_dir, path) = fixture(b"");
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        assert!(DiskFile::open(path).is_err());
    }
}
