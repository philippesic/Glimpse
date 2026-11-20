use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    ops::Range,
    path::{Path, PathBuf},
};
use unicode_segmentation::UnicodeSegmentation;

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

fn same_named_paths<'a>(path: &Path, others: impl IntoIterator<Item = &'a Path>) -> Vec<&'a Path> {
    others
        .into_iter()
        .filter(|other| *other != path && other.file_name() == path.file_name())
        .collect()
}

fn distinguishing_parent(path: &Path, duplicates: &[&Path]) -> Option<PathBuf> {
    let suffix = |path: &Path, depth: usize| {
        let components: Vec<_> = path.parent().unwrap_or(path).components().collect();
        components[components.len().saturating_sub(depth)..]
            .iter()
            .collect::<PathBuf>()
    };
    for depth in 1..=path.components().count() {
        let parent = suffix(path, depth);
        if duplicates
            .iter()
            .all(|other| suffix(other, depth) != parent)
        {
            return Some(parent);
        }
    }
    None
}

pub fn tab_label<'a>(path: &Path, others: impl IntoIterator<Item = &'a Path>) -> String {
    let duplicates = same_named_paths(path, others);
    if duplicates.is_empty() {
        return name(path);
    }
    distinguishing_parent(path, &duplicates).map_or_else(
        || display(path),
        |parent| format!("{} — {}", name(path), display(&parent)),
    )
}

pub fn parent_label<'a>(path: &Path, others: impl IntoIterator<Item = &'a Path>) -> String {
    let duplicates = same_named_paths(path, others);
    distinguishing_parent(path, &duplicates).map_or_else(
        || name(path.parent().unwrap_or(path)),
        |parent| display(&parent),
    )
}
fn shared_grapheme_bytes<'a>(
    left: impl Iterator<Item = &'a str>,
    right: impl Iterator<Item = &'a str>,
) -> usize {
    left.zip(right)
        .take_while(|(left, right)| left == right)
        .map(|(grapheme, _)| grapheme.len())
        .sum()
}

fn shared_path_edge(left: &str, right: &str, from_end: bool) -> usize {
    if left.is_ascii() && right.is_ascii() {
        let equal = |(left, right): &(u8, u8)| left == right;
        if from_end {
            left.bytes()
                .rev()
                .zip(right.bytes().rev())
                .take_while(equal)
                .count()
        } else {
            left.bytes().zip(right.bytes()).take_while(equal).count()
        }
    } else if from_end {
        shared_grapheme_bytes(left.graphemes(true).rev(), right.graphemes(true).rev())
    } else {
        shared_grapheme_bytes(left.graphemes(true), right.graphemes(true))
    }
}

#[derive(Clone)]
pub struct LabelCue {
    pub span: Range<usize>,
    pub ordinal: usize,
}

#[derive(Clone)]
pub struct ParentHint {
    pub text: String,
    pub cue: LabelCue,
}

fn label_cues<'a>(labels: &'a [&'a str]) -> impl Iterator<Item = LabelCue> + 'a {
    let first = labels[0];
    let prefix = labels.iter().skip(1).fold(first.len(), |prefix, label| {
        shared_path_edge(&first[..prefix], label, false)
    });
    let suffix = labels
        .iter()
        .skip(1)
        .fold(first.len() - prefix, |suffix, label| {
            shared_path_edge(&first[first.len() - suffix..], &label[prefix..], true)
        });
    labels
        .iter()
        .enumerate()
        .map(move |(index, label)| LabelCue {
            span: prefix..label.len() - suffix,
            ordinal: index + 1,
        })
}

fn label_edges(label: &str) -> Option<(&str, &str)> {
    let edge = 6;
    if label.len() < edge * 2 {
        return None;
    }
    if label.is_ascii() {
        return Some((&label[..edge], &label[label.len() - edge..]));
    }
    let (left, _) = label.grapheme_indices(true).nth(edge)?;
    let (right, _) = label.grapheme_indices(true).rev().nth(edge - 1)?;
    Some((&label[..left], &label[right..]))
}

pub fn filename_hints<'a>(names: impl IntoIterator<Item = &'a OsStr>) -> HashMap<String, LabelCue> {
    let mut names = names
        .into_iter()
        .filter(|name| name.len() >= 24)
        .map(escaped)
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    let mut groups = HashMap::<_, Vec<&str>>::new();
    for name in &names {
        if let Some(edges) = label_edges(name) {
            groups.entry(edges).or_default().push(name);
        }
    }
    groups
        .values()
        .filter(|labels| labels.len() > 1)
        .flat_map(|labels| labels.iter().zip(label_cues(labels)))
        .map(|(&name, cue)| (name.to_owned(), cue))
        .collect()
}

fn parent_cues(labels: &[&str]) -> Vec<LabelCue> {
    let mut cues: Vec<_> = label_cues(labels).collect();
    let mut groups = HashMap::<_, Vec<usize>>::new();
    for (index, label) in labels.iter().enumerate() {
        if let Some(edges) = label_edges(label) {
            groups.entry(edges).or_default().push(index);
        }
    }
    if groups.values().any(|group| group.len() == labels.len()) {
        return cues;
    }

    let mut unique = HashMap::new();
    for group in groups.values().filter(|group| group.len() > 1) {
        let similar = group.iter().map(|&index| labels[index]).collect::<Vec<_>>();
        for (&index, cue) in group.iter().zip(label_cues(&similar)) {
            if cue.span.is_empty() {
                continue;
            }
            let fragment = &labels[index][cue.span.clone()];
            unique
                .entry(fragment)
                .and_modify(|entry| *entry = None)
                .or_insert(Some((index, cue.span)));
        }
    }
    for (index, span) in unique.into_values().flatten() {
        cues[index].span = span;
    }
    cues
}

pub fn parent_hints<'a>(paths: impl IntoIterator<Item = &'a Path>) -> HashMap<PathBuf, ParentHint> {
    let mut by_name = HashMap::<&OsStr, HashSet<&OsStr>>::new();
    for path in paths {
        if let Some(name) = path.file_name() {
            by_name.entry(name).or_default().insert(path.as_os_str());
        }
    }
    let mut parents = HashMap::new();
    for group in by_name.values().filter(|group| group.len() > 1) {
        let mut counts = HashMap::new();
        for &path in group {
            for suffix in parent_suffixes(Path::new(path)) {
                *counts.entry(suffix.as_os_str()).or_insert(0) += 1;
            }
        }
        let mut labels = group
            .iter()
            .filter_map(|&path| {
                let path = Path::new(path);
                let suffix = parent_suffixes(path)
                    .filter(|suffix| counts.get(suffix.as_os_str()) == Some(&1))
                    .last()?;
                Some((path, display(suffix)))
            })
            .collect::<Vec<_>>();
        labels.sort_unstable_by_key(|(path, _)| path.as_os_str());
        if labels.is_empty() {
            continue;
        }
        let values = labels
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        let cues = parent_cues(&values);
        parents.extend(
            labels
                .into_iter()
                .zip(cues)
                .map(|((path, text), cue)| (path.to_owned(), ParentHint { text, cue })),
        );
    }
    parents
}

fn parent_suffixes(path: &Path) -> impl Iterator<Item = &Path> {
    let mut parts = path.parent().unwrap_or(path).components();
    std::iter::from_fn(move || {
        let suffix = parts.as_path();
        parts.next().map(|_| suffix)
    })
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
        assert_eq!(parent_label(paths[0], paths), "alpha/src");
        assert_eq!(parent_label(paths[1], paths), "beta/src");
        assert_eq!(parent_label(paths[2], paths), "beta");
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
        if value.chars().any(char::is_control)
            || value.starts_with('"')
            || value.starts_with("b\"")
            || value.starts_with("file://")
        {
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
        if path.is_absolute() {
            format!(
                "file://{}",
                percent_encoding::percent_encode(path.as_os_str().as_bytes(), ESCAPE)
            )
        } else {
            let mut value = String::from("b\"");
            for &byte in path.as_os_str().as_bytes() {
                match byte {
                    b'\\' => value.push_str("\\\\"),
                    b'"' => value.push_str("\\\""),
                    b' '..=b'~' => value.push(byte as char),
                    _ => value.push_str(&format!("\\x{byte:02x}")),
                }
            }
            value.push('"');
            value
        }
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
    } else if let Some(literal) = value.strip_prefix("b\"") {
        let literal = literal
            .strip_suffix('"')
            .ok_or("Byte paths must end with a quote")?
            .as_bytes();
        let mut bytes = Vec::new();
        let mut index = 0;
        while index < literal.len() {
            let byte = literal[index];
            index += 1;
            if byte == b'"' {
                return Err("Quotes inside byte paths must be escaped".into());
            }
            if byte != b'\\' {
                bytes.push(byte);
                continue;
            }
            let escaped = *literal.get(index).ok_or("Incomplete path escape")?;
            index += 1;
            match escaped {
                b'\\' | b'"' => bytes.push(escaped),
                b'n' => bytes.push(b'\n'),
                b'r' => bytes.push(b'\r'),
                b't' => bytes.push(b'\t'),
                b'x' => {
                    let digits = literal
                        .get(index..index + 2)
                        .ok_or("Incomplete byte escape")?;
                    if !digits.iter().all(u8::is_ascii_hexdigit) {
                        return Err("Invalid byte escape".into());
                    }
                    let digits = std::str::from_utf8(digits).map_err(|_| "Invalid byte escape")?;
                    bytes.push(u8::from_str_radix(digits, 16).map_err(|_| "Invalid byte escape")?);
                    index += 2;
                }
                _ => return Err("Unsupported byte path escape".into()),
            }
        }
        std::path::PathBuf::from(std::ffi::OsString::from_vec(bytes))
    } else if value.starts_with('"') {
        std::path::PathBuf::from(
            serde_json::from_str::<String>(value)
                .map_err(|error| format!("Invalid quoted path: {error}"))?,
        )
    } else {
        std::path::PathBuf::from(value)
    };
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("Paths cannot contain a NUL byte".into());
    }
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
    #[test]
    fn escaped_relative_entries_preserve_bytes_and_reserved_prefixes() {
        let current = Path::new("/current");
        for path in [
            std::path::PathBuf::from("\"quoted"),
            std::path::PathBuf::from("b\"literal"),
            std::path::PathBuf::from("file://literal"),
            std::path::PathBuf::from("control\nname"),
            std::path::PathBuf::from(OsString::from_vec(b"caf\xc3\xa9\xff\\x41\"".to_vec())),
        ] {
            assert_eq!(
                parse_entry(&entry(&path), current).unwrap(),
                current.join(path)
            );
        }
        for invalid in [
            "b\"bad\\xgg\"",
            "b\"unterminated",
            "b\"a\\0b\"",
            "b\"a\\x00b\"",
            "b\"a\\x+1b\"",
            "file:///a%00b",
        ] {
            assert!(parse_entry(invalid, current).is_err());
        }
    }
}
