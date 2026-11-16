//! Bounded line alignment for read-only conflict excerpts; never a merge or save input.
const BYTE_LIMIT: usize = 16 * 1024;
const LINE_LIMIT: usize = 160;
const CHAR_LIMIT: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Equal,
    Removed,
    Added,
    Uncompared,
}

pub struct Line {
    pub number: usize,
    pub text: String,
    complete: bool,
    terminated: bool,
}

impl Line {
    pub fn no_final_newline(&self) -> bool {
        self.complete && !self.terminated
    }
}

pub struct Excerpt {
    pub lines: Vec<Line>,
    pub truncated: bool,
}

pub struct Row {
    pub local: Option<usize>,
    pub disk: Option<usize>,
    pub change: Change,
}

pub struct Comparison {
    pub local: Excerpt,
    pub disk: Excerpt,
    pub rows: Vec<Row>,
}

fn excerpt(text: &str) -> Excerpt {
    let end = text.floor_char_boundary(text.len().min(BYTE_LIMIT));
    let mut lines = Vec::new();
    let mut offset = 0;
    let mut truncated = end < text.len();
    for part in text[..end].split_inclusive('\n').take(LINE_LIMIT) {
        offset += part.len();
        let terminated = part.ends_with('\n');
        let content = part.strip_suffix('\n').unwrap_or(part);
        let content = if terminated {
            content.strip_suffix('\r').unwrap_or(content)
        } else {
            content
        };
        let mut chars = content.chars();
        let mut shown: String = chars.by_ref().take(CHAR_LIMIT).collect();
        let complete = chars.next().is_none() && (terminated || offset == text.len());
        if !complete {
            shown.push('…');
            truncated = true;
        }
        lines.push(Line {
            number: lines.len() + 1,
            text: shown,
            complete,
            terminated,
        });
    }
    truncated |= offset < text.len();
    Excerpt { lines, truncated }
}

impl Comparison {
    pub fn new(local: &str, disk: &str) -> Self {
        let local = excerpt(local);
        let disk = excerpt(disk);
        let (n, m) = (local.lines.len(), disk.lines.len());
        let equal = |i: usize, j: usize| {
            let (a, b) = (&local.lines[i], &disk.lines[j]);
            a.complete && b.complete && a.text == b.text && a.terminated == b.terminated
        };
        // At most 161² u16 cells; omitted suffixes and clipped lines never count as equal.
        let mut lengths = vec![0u16; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lengths[i * (m + 1) + j] = if equal(i, j) {
                    1 + lengths[(i + 1) * (m + 1) + j + 1]
                } else {
                    lengths[(i + 1) * (m + 1) + j].max(lengths[i * (m + 1) + j + 1])
                };
            }
        }
        let mut rows = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && equal(i, j) {
                rows.push(Row {
                    local: Some(i),
                    disk: Some(j),
                    change: Change::Equal,
                });
                i += 1;
                j += 1;
                continue;
            }
            let (start_i, start_j) = (i, j);
            while i < n || j < m {
                if i < n && j < m && equal(i, j) {
                    break;
                }
                if i < n
                    && (j == m || lengths[(i + 1) * (m + 1) + j] >= lengths[i * (m + 1) + j + 1])
                {
                    i += 1;
                } else {
                    j += 1;
                }
            }
            for index in start_i..i {
                rows.push(Row {
                    local: Some(index),
                    disk: None,
                    change: if local.lines[index].complete {
                        Change::Removed
                    } else {
                        Change::Uncompared
                    },
                });
            }
            for index in start_j..j {
                rows.push(Row {
                    local: None,
                    disk: Some(index),
                    change: if disk.lines[index].complete {
                        Change::Added
                    } else {
                        Change::Uncompared
                    },
                });
            }
        }
        Self { local, disk, rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_replacements_insertions_and_repeated_unicode_lines() {
        let diff = Comparison::new(
            "same\nold 🦀\nrepeat\nrepeat\nlast",
            "same\nnew 👋\nrepeat\ninsert\nrepeat\nlast",
        );
        assert_eq!(
            diff.rows.iter().map(|r| r.change).collect::<Vec<_>>(),
            [
                Change::Equal,
                Change::Removed,
                Change::Added,
                Change::Equal,
                Change::Added,
                Change::Equal,
                Change::Equal
            ]
        );
        let local: Vec<_> = diff.rows.iter().filter_map(|r| r.local).collect();
        let disk: Vec<_> = diff.rows.iter().filter_map(|r| r.disk).collect();
        assert_eq!(local, (0..5).collect::<Vec<_>>());
        assert_eq!(disk, (0..6).collect::<Vec<_>>());
    }

    #[test]
    fn bounds_work_and_never_labels_clipped_prefixes_equal() {
        let a = format!("{}a", "🦀".repeat(100_000));
        let b = format!("{}b", "🦀".repeat(100_000));
        let diff = Comparison::new(&a, &b);
        assert!(diff.local.truncated && diff.disk.truncated);
        assert_eq!(diff.rows.len(), 2);
        assert!(diff.rows.iter().all(|r| r.change == Change::Uncompared));
        assert!(diff.local.lines[0].text.chars().count() <= CHAR_LIMIT + 1);
        let diff = Comparison::new(&"line\n".repeat(10_000), &"line\n".repeat(10_000));
        assert_eq!(diff.rows.len(), LINE_LIMIT);
        assert!(diff.local.truncated && diff.disk.truncated);
        assert!(Comparison::new("", "").rows.is_empty());
        assert_eq!(Comparison::new("line", "line\n").rows.len(), 2);
    }
}
