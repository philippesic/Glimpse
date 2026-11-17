use std::{fs, io::Write, path::Path};
pub fn create(path: &Path, folder: bool, text: &str) -> Result<(), String> {
    if folder {
        return fs::create_dir(path).map_err(|e| e.to_string());
    }
    if text.len() as u64 > crate::document::MAX_BYTES {
        return Err("New file exceeds the 10 MiB editing limit".into());
    }
    let parent = path.parent().ok_or("Missing parent folder")?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    staged
        .write_all(text.as_bytes())
        .and_then(|_| staged.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    staged.persist_noclobber(path).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn rename(source: &Path, destination: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    let source =
        std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let destination =
        std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}
pub fn trash(path: &Path) -> Result<(), String> {
    trash_location(path).map(|_| ())
}
#[cfg(target_os = "macos")]
fn trash_location(path: &Path) -> Result<Option<std::path::PathBuf>, String> {
    use objc2_foundation::{NSFileManager, NSURL};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let directory = fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir();
    let value = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let url = unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            std::ptr::NonNull::new(value.as_ptr().cast_mut()).expect("CString is non-null"),
            directory,
            None,
        )
    };
    let mut result = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut result))
        .map_err(|e| e.to_string())?;
    Ok(result.map(|url| {
        std::ffi::OsString::from_vec(
            unsafe { std::ffi::CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }
                .to_bytes()
                .to_vec(),
        )
        .into()
    }))
}

#[cfg(target_os = "linux")]
fn trash_location(path: &Path) -> Result<Option<std::path::PathBuf>, String> {
    trash::delete(path).map_err(|error| error.to_string())?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn trash_is_recoverable_and_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("glimpse-trash-test-link");
        fs::write(&target, "preserved").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let trashed = trash_location(&link)
            .unwrap()
            .expect("OS Trash returns the recovery path");
        assert!(!link.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserved");
        assert!(
            fs::symlink_metadata(&trashed)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        rename(&trashed, &link).unwrap();
        assert!(link.exists());
        assert!(!trashed.exists());
        assert!(trash(&dir.path().join("missing")).is_err());
    }
    #[test]
    fn new_destination_keeps_bom_and_refuses_oversize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.txt");
        create(&path, false, "\u{feff}text\r\n").unwrap();
        let (disk, text) = crate::document::DiskFile::open(path.clone()).unwrap();
        assert!(disk.bom);
        assert_eq!(text.as_ref(), "text\r\n");
        assert_eq!(fs::read(path).unwrap(), b"\xef\xbb\xbftext\r\n");
        let large = dir.path().join("large");
        assert!(
            create(
                &large,
                false,
                &"x".repeat(crate::document::MAX_BYTES as usize + 1)
            )
            .is_err()
        );
        assert!(!large.exists());
    }
    #[test]
    fn creation_and_rename_never_clobber() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        create(&a, false, "one").unwrap();
        create(&b, false, "two").unwrap();
        assert!(create(&a, false, "lost").is_err());
        assert!(create(&a, true, "").is_err());
        assert!(rename(&a, &b).is_err());
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        assert_eq!(fs::read_to_string(b).unwrap(), "two");
        let c = dir.path().join("c");
        rename(&a, &c).unwrap();
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(c).unwrap(), "one");
    }
}
