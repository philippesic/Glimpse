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
pub(crate) fn display(path: &Path) -> String {
    escaped(path.as_os_str())
}
pub(crate) fn name(path: &Path) -> String {
    let name = path.file_name().unwrap_or(path.as_os_str());
    display(Path::new(name))
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
}
