use std::path::Path;

fn escaped(value: &std::ffi::OsStr) -> String {
    value
        .to_str()
        .filter(|s| {
            !s.chars().any(|c| {
                c.is_control()
                    || matches!(c,'"'|'\\'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')
            })
        })
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{value:?}"))
}
pub fn display(path: &Path) -> String {
    escaped(path.as_os_str())
}
pub fn name(path: &Path) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    display(Path::new(name))
}

pub fn tab_label<'a>(path: &Path, others: impl IntoIterator<Item = &'a Path>) -> String {
    let duplicates: Vec<_> = others
        .into_iter()
        .filter(|other| *other != path && other.file_name() == path.file_name())
        .collect();
    if duplicates.is_empty() {
        return name(path);
    }
    let suffix = |path: &Path, depth: usize| {
        let components: Vec<_> = path.parent().unwrap_or(path).components().collect();
        components[components.len().saturating_sub(depth)..]
            .iter()
            .collect::<std::path::PathBuf>()
    };
    for depth in 1..=path.components().count() {
        let parent = suffix(path, depth);
        if duplicates
            .iter()
            .all(|other| suffix(other, depth) != parent)
        {
            return format!("{} — {}", name(path), display(&parent));
        }
    }
    display(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;
    #[test]
    fn unusual_names_remain_distinct_and_never_inject_lines() {
        assert_eq!(name(Path::new("/tmp/café")), "café");
        let newline = display(Path::new("/tmp/a\nb"));
        assert!(!newline.contains('\n'));
        assert_ne!(newline, display(Path::new("/tmp/a\\nb")));
        let invalid =
            std::path::PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/a\xff".to_vec()));
        assert_ne!(display(&invalid), display(Path::new("/tmp/a�")));
    }

    #[test]
    fn tabs_use_the_shortest_distinguishing_parent_suffix() {
        let paths = [
            Path::new("/work/alpha/src/main.rs"),
            Path::new("/work/beta/src/main.rs"),
            Path::new("/work/beta/other.txt"),
        ];
        assert_eq!(tab_label(paths[0], paths), "main.rs — alpha/src");
        assert_eq!(tab_label(paths[1], paths), "main.rs — beta/src");
        assert_eq!(tab_label(paths[2], paths), "other.txt");
        let parents = [
            Path::new("/work/alpha/main.rs"),
            Path::new("/work/beta/main.rs"),
        ];
        assert_eq!(tab_label(parents[0], parents), "main.rs — alpha");
    }
}

pub fn entry(path: &Path) -> String {
    if let Some(value) = path.to_str() {
        if value.chars().any(char::is_control) {
            serde_json::to_string(value).unwrap()
        } else {
            value.to_owned()
        }
    } else {
        use std::os::unix::ffi::OsStrExt;
        const ESCAPE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
            .add(b' ')
            .add(b'%')
            .add(b'#')
            .add(b'?')
            .add(b'"')
            .add(b'\\');
        format!(
            "file://{}",
            percent_encoding::percent_encode(path.as_os_str().as_bytes(), ESCAPE)
        )
    }
}
pub fn parse_entry(value: &str, current: &Path) -> Result<std::path::PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;
    let path = if let Some(encoded) = value.strip_prefix("file://") {
        if !encoded.starts_with('/') {
            return Err("Only local file URLs are supported".into());
        }
        std::path::PathBuf::from(std::ffi::OsString::from_vec(
            percent_encoding::percent_decode_str(encoded).collect(),
        ))
    } else if value.starts_with('"') {
        std::path::PathBuf::from(
            serde_json::from_str::<String>(value)
                .map_err(|error| format!("Invalid quoted path: {error}"))?,
        )
    } else {
        std::path::PathBuf::from(value)
    };
    Ok(if path.is_absolute() {
        path
    } else {
        current.join(path)
    })
}
pub fn single_name(value: &str) -> bool {
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(std::path::Component::Normal(name)) if name == value)
        && components.next().is_none()
}

#[cfg(test)]
mod entry_tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    #[test]
    fn entry_roundtrips_unicode_quotes_backslashes_controls_and_native_bytes() {
        for path in [
            std::path::PathBuf::from("/tmp/a\"b\\c🌱"),
            std::path::PathBuf::from("/tmp/a\nb"),
            std::path::PathBuf::from(OsString::from_vec(b"/tmp/a\xff%#?".to_vec())),
        ] {
            assert_eq!(
                parse_entry(&entry(&path), Path::new("/unused")).unwrap(),
                path
            );
        }
    }
    #[test]
    fn relative_entries_resolve_from_the_displayed_folder() {
        assert_eq!(
            parse_entry("child", Path::new("/current")).unwrap(),
            Path::new("/current/child")
        );
        assert!(parse_entry("file://other-machine/path", Path::new("/current")).is_err());
    }
    #[test]
    fn mutation_names_cannot_change_folders() {
        for invalid in ["", ".", "..", "/", "/name", "a/b", "a/"] {
            assert!(!single_name(invalid));
        }
        assert!(single_name("a\"b\\c🌱"));
    }
}
