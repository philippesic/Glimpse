use std::ops::Range;

use ropey::Rope;
use sum_tree::Bias;

use super::RopeExt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct LineEndings {
    lf: usize,
    cr: usize,
    crlf: usize,
}
impl LineEndings {
    pub(super) fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        Self {
            lf: bytes.iter().filter(|&&byte| byte == b'\n').count(),
            cr: bytes.iter().filter(|&&byte| byte == b'\r').count(),
            crlf: bytes.windows(2).filter(|pair| *pair == b"\r\n").count(),
        }
    }
    pub(super) fn replace(&mut self, before: &Rope, after: &Rope, range: &Range<usize>) {
        let start = before.clip_offset(range.start.saturating_sub(1), Bias::Left);
        let end = before.clip_offset(range.end.saturating_add(1), Bias::Right);
        let after_end = after.len() - (before.len() - end);
        let removed = Self::new(&before.slice(start..end).to_string());
        let added = Self::new(&after.slice(start..after_end).to_string());
        self.lf = self.lf - removed.lf + added.lf;
        self.cr = self.cr - removed.cr + added.cr;
        self.crlf = self.crlf - removed.crlf + added.crlf;
    }
    pub(super) fn label(&self) -> &'static str {
        match (self.lf, self.cr, self.crlf) {
            (0, 0, _) => "No line breaks",
            (_, 0, _) => "LF",
            (0, _, _) => "CR",
            (lf, cr, pairs) if lf == pairs && cr == pairs => "CRLF",
            _ => "Mixed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endings_distinguish_empty_uniform_and_mixed_text() {
        for (text, expected) in [
            ("", "No line breaks"),
            ("café 🦀", "No line breaks"),
            ("a\nb\n", "LF"),
            ("a\rb\r", "CR"),
            ("a\r\nb\r\n", "CRLF"),
            ("a\r\nb\n", "Mixed"),
            ("a\rb\n", "Mixed"),
        ] {
            assert_eq!(LineEndings::new(text).label(), expected);
        }
    }
    #[test]
    fn replacements_match_full_counts_across_crlf_and_unicode_boundaries() {
        for original in ["", "\r\n", "a\r\nb\rc\nd", "🙂é\r\n中文", "\r\r\n\n"] {
            let before = Rope::from(original);
            let boundaries: Vec<_> = (0..=original.len())
                .filter(|&byte| original.is_char_boundary(byte))
                .collect();
            for &start in &boundaries {
                for &end in boundaries.iter().filter(|&&end| end >= start) {
                    for inserted in ["", "\r", "\n", "\r\n", "\n\r", "🌱é"] {
                        let mut after = before.clone();
                        after.replace(start..end, inserted);
                        let mut counts = LineEndings::new(original);
                        counts.replace(&before, &after, &(start..end));
                        assert_eq!(
                            counts,
                            LineEndings::new(&after.to_string()),
                            "{original:?}, {start}..{end}, {inserted:?}"
                        );
                    }
                }
            }
        }
    }
}
