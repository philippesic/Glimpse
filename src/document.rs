#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
use std::{
    fs::{self, Metadata, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;

#[derive(Clone)]
pub struct DiskFile {
    pub path: PathBuf,
    pub target: PathBuf,
    pub bytes: Vec<u8>,
    pub bom: bool,
    identity: (u64, u64),
}

fn identity(meta: &Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

fn read_regular(path: &Path) -> Result<(Vec<u8>, Metadata)> {
    read_cancellable(path, &|| false)
}
fn read_cancellable(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<(Vec<u8>, Metadata)> {
    let mut file = OpenOptions::new()
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
    let mut bytes = Vec::with_capacity(meta.len() as usize + 1);
    let mut chunk = [0u8; 65536];
    loop {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let read = file.read(&mut chunk).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > MAX_BYTES {
            break;
        }
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Err("File grew beyond 10 MiB while reading.".into());
    }
    Ok((bytes, meta))
}

impl DiskFile {
    pub fn draft() -> Self {
        Self {
            path: PathBuf::from("Untitled"),
            target: PathBuf::new(),
            bytes: Vec::new(),
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
        disk.bom = draft.base.starts_with(&[0xef, 0xbb, 0xbf]);
        disk.target = draft.target.clone().unwrap_or_default();
        disk
    }
    pub fn same_file(&self, other: &Self) -> bool {
        (!self.target.as_os_str().is_empty() && self.target == other.target)
            || (self.identity != (0, 0) && self.identity == other.identity)
    }

    pub fn open(path: PathBuf) -> Result<(Self, String)> {
        Self::open_cancellable(path, &|| false)
    }
    pub fn open_cancellable(path: PathBuf, cancelled: &dyn Fn() -> bool) -> Result<(Self, String)> {
        if cancelled() {
            return Err("Cancelled".into());
        }
        let target = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        let (bytes, meta) = read_cancellable(&target, cancelled)?;
        let bom = bytes.starts_with(&[0xef, 0xbb, 0xbf]);
        let content = if bom { &bytes[3..] } else { &bytes };
        let text = std::str::from_utf8(content)
            .map_err(|_| "Only UTF-8 text is supported; the file was not changed.")?
            .to_owned();
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("This file contains binary/control data and cannot be edited.".into());
        }
        Ok((
            Self {
                path,
                target,
                bytes,
                bom,
                identity: identity(&meta),
            },
            text,
        ))
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
        let (bytes, _) = read_regular(&self.target)?;
        if bytes != self.bytes {
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
        let (bytes, meta) = read_regular(&self.target)?;
        if identity(&meta) != self.identity || bytes != self.bytes {
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
    pub fn save_with_bom(&self, text: &str, bom: bool) -> Result<Self> {
        let meta = self.validate()?;
        let mut bytes = Vec::with_capacity(text.len() + 3);
        if bom {
            bytes.extend_from_slice(&[0xef, 0xbb, 0xbf]);
        }
        bytes.extend_from_slice(text.as_bytes());
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
        staged.write_all(&bytes).map_err(|e| e.to_string())?;
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
    fn untouched_bytes_and_edited_bom_are_preserved() {
        for bytes in [
            b"\xef\xbb\xbfhello\r\nworld\r\n".as_slice(),
            b"one\rtwo\nthree",
            "héllo 🦀".as_bytes(),
            b"",
        ] {
            let (_dir, path) = fixture(bytes);
            let (disk, text) = DiskFile::open(path.clone()).unwrap();
            disk.save(&text).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
            disk.save(&(text + "!")).unwrap();
            assert_eq!(fs::read(path).unwrap(), [bytes, b"!"].concat());
        }
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
