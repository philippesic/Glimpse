use std::path::{Path, PathBuf};

pub struct Choice {
    pub path: PathBuf,
    pub directory: bool,
}
impl Choice {
    pub fn entry(&self) -> String {
        let mut path = self.path.as_os_str().to_owned();
        if self.directory {
            path.push(std::path::MAIN_SEPARATOR_STR);
        }
        crate::paths::entry(Path::new(&path))
    }
}
pub struct Matches {
    pub choices: Vec<Choice>,
    pub partial: bool,
}
pub fn complete(value: &str, current: &Path) -> Result<Matches, String> {
    let path = crate::paths::parse_entry(value, current)?;
    let trailing = path
        .as_os_str()
        .as_encoded_bytes()
        .ends_with(std::path::MAIN_SEPARATOR_STR.as_bytes());
    let (parent, prefix) = if trailing {
        (path.as_path(), std::ffi::OsStr::new(""))
    } else {
        (
            path.parent().unwrap_or(current),
            path.file_name().unwrap_or_default(),
        )
    };
    let mut choices = Vec::new();
    let mut partial = false;
    let mut bytes = 0;
    for (count, item) in std::fs::read_dir(parent)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        if count >= 100_000 || bytes >= 4 * 1024 * 1024 {
            partial = true;
            break;
        }
        let item = item.map_err(|e| e.to_string())?;
        let name = item.file_name();
        bytes += name.as_encoded_bytes().len();
        if !name
            .as_encoded_bytes()
            .starts_with(prefix.as_encoded_bytes())
        {
            continue;
        }
        if choices.len() >= 64 {
            partial = true;
            continue;
        }
        let kind = item.file_type().map_err(|e| e.to_string())?;
        let directory = kind.is_dir() || (kind.is_symlink() && item.path().is_dir());
        choices.push(Choice {
            path: item.path(),
            directory,
        });
    }
    choices.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(Matches { choices, partial })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn completion_preserves_native_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let root = tempfile::tempdir().unwrap();
        let native = root.path().join(OsString::from_vec(b"child\xff".to_vec()));
        std::fs::create_dir(&native).unwrap();
        let result = complete("child", root.path()).unwrap();
        assert_eq!(result.choices.len(), 1);
        let parsed = crate::paths::parse_entry(&result.choices[0].entry(), root.path()).unwrap();
        assert_eq!(parsed, native);
    }
    #[test]
    fn completion_bounds_displayed_matches() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..100 {
            std::fs::write(root.path().join(format!("many{i}")), b"").unwrap();
        }
        let result = complete("many", root.path()).unwrap();
        assert_eq!(result.choices.len(), 64);
        assert!(result.partial);
        assert!(complete("missing/child", root.path()).is_err());
    }
}
