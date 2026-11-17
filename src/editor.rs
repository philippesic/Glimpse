#[cfg(test)]
use egui::{
    self,
    text::{CCursor, CCursorRange},
};
use unicode_segmentation::UnicodeSegmentation;

pub fn position(text: &str, line: usize, column: usize) -> (usize, bool) {
    if line <= 1 && column <= 1 {
        return (0, false);
    }
    let mut ranges = line_ranges(text);
    let mut selected = 0..0;
    let mut clamped = false;
    for index in 0..line.max(1) {
        if let Some(range) = ranges.next() {
            selected = range;
        } else {
            clamped = true;
            selected = text.len()..text.len();
            break;
        }
        if index + 1 == line.max(1) {
            break;
        }
    }
    let offset = selected.start;
    let value = &text[selected];
    let requested = value
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(value.len()))
        .nth(column.saturating_sub(1))
        .unwrap_or_else(|| {
            clamped = true;
            value.len()
        });
    let boundary = value
        .grapheme_indices(true)
        .map(|v| v.0)
        .chain(std::iter::once(value.len()))
        .take_while(|&v| v <= requested)
        .last()
        .unwrap_or(0);
    clamped |= boundary != requested;
    (text[..offset + boundary].chars().count(), clamped)
}
#[cfg(test)]
pub fn replace(ctx: &egui::Context, id: egui::Id, old: &str, new: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let cursor = state
            .cursor
            .char_range()
            .unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
        let mut undo = state.undoer();
        undo.add_undo(&(cursor, old.to_owned()));
        let maximum = new.chars().count();
        let mut next = cursor;
        next.primary.index = usize::from(next.primary.index).min(maximum).into();
        next.secondary.index = usize::from(next.secondary.index).min(maximum).into();
        undo.add_undo(&(next, new.to_owned()));
        state.set_undoer(undo);
        state.cursor.set_char_range(Some(next));
        state.store(ctx, id);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_metadata_tracks_long_lines_and_resets_after_edit() {
        let mut metadata = Metadata::default();
        let text = format!("α\r\n{}", "x".repeat(20_000));
        metadata.update(&text, 1);
        assert_eq!(metadata.longest_line, 20_000);
        assert_eq!(metadata.position_utf16(&text, 3), (2, 1));
        metadata.update("short\n", 2);
        assert_eq!(metadata.longest_line, 6);
        assert_eq!(metadata.starts, vec![0, 6]);
    }
    #[test]
    fn positions_clamp_to_graphemes() {
        assert_eq!(position("a👨‍👩‍👧b\nlast", 1, 4), (1, true));
        assert_eq!(position("a\nlast", 2, 3), (4, false));
        assert_eq!(position("a", 9, 9), (1, true));
        assert_eq!(position("", 1, 1), (0, false));
        assert_eq!(position("🦀", 1, 2), (1, false));
        assert_eq!(position("🦀", 1, 3), (1, true));
    }
    #[test]
    fn reload_is_one_undoable_transition() {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("test");
        egui::text_edit::TextEditState::default().store(&ctx, id);
        replace(&ctx, id, "old", "new");
        let state = egui::TextEdit::load_state(&ctx, id).unwrap();
        let mut undo = state.undoer();
        assert_eq!(
            undo.undo(&(CCursorRange::one(CCursor::new(0)), "new".into()))
                .unwrap()
                .1,
            "old"
        );
    }
}

pub fn utf16_to_byte(text: &str, index: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units + ch.len_utf16() > index {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}
pub fn byte_to_utf16(text: &str, index: usize) -> usize {
    text[..index.min(text.len())].encode_utf16().count()
}

pub fn match_ranges(
    regex: &regex::Regex,
    text: &str,
    limit: usize,
) -> (Vec<std::ops::Range<usize>>, bool) {
    let mut matches = regex.find_iter(text);
    let ranges = matches.by_ref().take(limit).map(|m| m.range()).collect();
    (ranges, matches.next().is_some())
}

pub struct Metadata {
    pub generation: u64,
    pub starts: Vec<usize>,
    pub utf16_starts: std::sync::Arc<Vec<usize>>,
    pub longest_line: usize,
    pub endings: &'static str,
    cached: Option<(usize, usize, usize)>,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            generation: u64::MAX,
            starts: vec![0],
            utf16_starts: std::sync::Arc::new(vec![0]),
            longest_line: 0,
            endings: "LF",
            cached: None,
        }
    }
}
impl Metadata {
    pub fn update(&mut self, text: &str, generation: u64) {
        if self.generation == generation {
            return;
        }
        let mut starts = vec![0];
        let mut utf16_starts = vec![0];
        let mut units = 0;
        let mut lf = 0;
        let mut cr = 0;
        let mut crlf = 0;
        for (byte, ch) in text.char_indices() {
            units += ch.len_utf16();
            if ch == '\r' {
                cr += 1;
                if text.as_bytes().get(byte + 1) == Some(&b'\n') {
                    crlf += 1;
                } else {
                    starts.push(byte + 1);
                    utf16_starts.push(units);
                }
            }
            if ch == '\n' {
                lf += 1;
                starts.push(byte + 1);
                utf16_starts.push(units);
            }
        }
        self.endings = if cr == 0 {
            "LF"
        } else if cr == crlf && lf == crlf {
            "CRLF"
        } else if lf == 0 {
            "CR"
        } else {
            "Mixed"
        };
        self.longest_line = starts
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .chain(std::iter::once(
                text.len() - starts.last().copied().unwrap_or(0),
            ))
            .max()
            .unwrap_or(0);
        self.starts = starts;
        self.utf16_starts = std::sync::Arc::new(utf16_starts);
        self.generation = generation;
        self.cached = None;
    }
    pub fn position_utf16(&mut self, text: &str, index: usize) -> (usize, usize) {
        if let Some((last, line, column)) = self.cached
            && last == index
        {
            return (line, column);
        }
        let row = self
            .utf16_starts
            .partition_point(|v| *v <= index)
            .saturating_sub(1);
        let remaining = index.saturating_sub(self.utf16_starts[row]);
        let byte = self.starts[row];
        let column = text[byte..]
            .chars()
            .scan(0, |units, ch| {
                *units += ch.len_utf16();
                Some(*units)
            })
            .take_while(|units| *units <= remaining)
            .count()
            + 1;
        self.cached = Some((index, row + 1, column));
        (row + 1, column)
    }
}

pub fn indent_lines(text: &str, unit: &str, outdent: bool) -> String {
    let mut result = String::with_capacity(text.len() + unit.len());
    for line in text.split_inclusive('\n') {
        if outdent {
            let remove = if line.starts_with('\t') {
                1
            } else {
                line.bytes()
                    .take(unit.len().max(1))
                    .take_while(|b| *b == b' ')
                    .count()
            };
            result.push_str(&line[remove..]);
        } else {
            result.push_str(unit);
            result.push_str(line);
        }
    }
    result
}
#[cfg(test)]
mod indentation_tests {
    use super::*;
    #[test]
    fn block_indent_round_trips_crlf_and_unicode() {
        let source = "α\r\nβ\n";
        assert_eq!(
            indent_lines(&indent_lines(source, "  ", false), "  ", true),
            source
        );
        assert_eq!(indent_lines("\tx\n  y", "    ", true), "x\ny");
    }
}

/// The unchanged prefix and suffix stay outside the native replacement range.
pub fn difference(old: &str, new: &str) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let prefix = old
        .graphemes(true)
        .zip(new.graphemes(true))
        .take_while(|(a, b)| a == b)
        .map(|(s, _)| s.len())
        .sum::<usize>();
    let suffix = old[prefix..]
        .graphemes(true)
        .rev()
        .zip(new[prefix..].graphemes(true).rev())
        .take_while(|(a, b)| a == b)
        .map(|(s, _)| s.len())
        .sum::<usize>();
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}
#[cfg(test)]
mod difference_tests {
    use super::*;
    #[test]
    fn changed_region_does_not_split_clusters() {
        assert_eq!(difference("a é z", "a è z"), (2..5, 2..5));
        assert_eq!(
            difference("before\nafter", "before\nnew\nafter"),
            (7..7, 7..11)
        );
    }
}

pub struct LineRanges<'a> {
    text: &'a str,
    next: usize,
}
pub fn line_ranges(text: &str) -> LineRanges<'_> {
    LineRanges { text, next: 0 }
}
impl Iterator for LineRanges<'_> {
    type Item = std::ops::Range<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next > self.text.len() {
            return None;
        }
        let start = self.next;
        let suffix = &self.text[start..];
        let end = start
            + suffix
                .bytes()
                .position(|b| b == b'\r' || b == b'\n')
                .unwrap_or(suffix.len());
        self.next = if end == self.text.len() {
            end + 1
        } else if self.text.as_bytes()[end] == b'\r'
            && self.text.as_bytes().get(end + 1) == Some(&b'\n')
        {
            end + 2
        } else {
            end + 1
        };
        Some(start..end)
    }
}

pub fn matcher(
    needle: &str,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<regex::Regex, String> {
    if needle.is_empty() {
        return Err("Enter text to find.".into());
    }
    let literal = regex::escape(needle);
    let pattern = if whole_word {
        format!(r"\b(?:{literal})\b")
    } else {
        literal
    };
    regex::RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(1024 * 1024)
        .build()
        .map_err(|e| e.to_string())
}
pub fn next_match(
    regex: &regex::Regex,
    text: &str,
    from: usize,
    backwards: bool,
) -> Option<std::ops::Range<usize>> {
    let mut from = from.min(text.len());
    while !text.is_char_boundary(from) {
        from -= 1;
    }
    if backwards {
        regex
            .find_iter(&text[..from])
            .last()
            .or_else(|| regex.find_iter(text).last())
            .map(|m| m.range())
    } else {
        regex
            .find_at(text, from)
            .or_else(|| regex.find(text))
            .map(|m| m.range())
    }
}
#[cfg(test)]
mod find_tests {
    use super::*;
    #[test]
    fn live_match_summary_is_bounded_and_uses_byte_ranges() {
        let r = matcher("é", false, false).unwrap();
        assert_eq!(match_ranges(&r, "é É é", 2), (vec![0..2, 3..5], true));
        assert_eq!(match_ranges(&r, "é É", 2), (vec![0..2, 3..5], false));
        let r = matcher("cat", false, true).unwrap();
        assert_eq!(
            match_ranges(&r, "cat scatter CAT", 10).0,
            vec![0..3, 12..15]
        );
        assert_eq!(next_match(&r, "cat scatter CAT", 0, true), Some(12..15));
    }
    #[test]
    fn literal_search_never_invents_a_range_or_splits_utf8() {
        let r = matcher("é", false, false).unwrap();
        assert_eq!(next_match(&r, "ab é", 4, false), Some(3..5));
        assert!(next_match(&r, "other", 0, false).is_none());
        let r = matcher("$1", true, false).unwrap();
        assert_eq!(r.replace_all("$1", regex::NoExpand("$2")), "$2");
        assert_eq!(position("a\r\nb\rc", 3, 1), (5, false));
    }
}
