use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct Segment {
    pub(crate) range: Range<usize>,
    pub(crate) boundaries: Vec<usize>,
    pub(crate) words: Vec<u8>,
    pub(crate) line: usize,
}
pub(crate) fn segments(text: &str) -> Vec<Segment> {
    let mut segments = vec![];
    let mut offset = 0;
    for (line_index, line) in text
        .split_inclusive('\n')
        .chain(text.ends_with('\n').then_some(""))
        .enumerate()
    {
        let words: Vec<_> = line
            .unicode_word_indices()
            .map(|(byte, _)| offset + byte)
            .collect();
        let mut start = offset;
        let mut boundaries = vec![offset];
        let pieces = line.graphemes(true).flat_map(|grapheme| {
            if grapheme.len() <= u8::MAX as usize {
                smallvec::SmallVec::<[usize; 1]>::from_slice(&[grapheme.len()])
            } else {
                grapheme.chars().map(char::len_utf8).collect()
            }
        });
        for length in pieces {
            if boundaries.len() == 256 {
                let end = *boundaries.last().unwrap();
                let starts = words
                    .iter()
                    .filter_map(|word| {
                        boundaries
                            .binary_search(word)
                            .ok()
                            .filter(|&i| i < boundaries.len() - 1)
                            .map(|i| i as u8)
                    })
                    .collect();
                segments.push(Segment {
                    range: start..end,
                    boundaries,
                    words: starts,
                    line: line_index,
                });
                start = end;
                boundaries = vec![end];
            }
            boundaries.push(boundaries.last().unwrap() + length);
        }
        let end = offset + line.len();
        let starts = words
            .iter()
            .filter_map(|word| {
                boundaries
                    .binary_search(word)
                    .ok()
                    .filter(|&i| i < boundaries.len() - 1)
                    .map(|i| i as u8)
            })
            .collect();
        segments.push(Segment {
            range: start..end,
            boundaries,
            words: starts,
            line: line_index,
        });
        offset = end;
    }
    if segments.is_empty() {
        segments.push(Segment {
            range: 0..0,
            boundaries: vec![0],
            words: vec![],
            line: 0,
        });
    }
    segments
}
pub(crate) fn text(
    id: impl Into<gpui::ElementId>,
    text: gpui::SharedString,
    child: impl gpui::IntoElement,
) -> gpui::Accessible {
    gpui::accessible(id, child, move |id, window, _| {
        use gpui::accesskit::{Node, Role, TextDirection};
        let parts = segments(&text);
        let ids: Vec<_> = (0..parts.len())
            .map(|index| window.accessibility_child_id(id, ("run", index)))
            .collect();
        let mut node = Node::new(Role::Label);
        node.set_read_only();
        node.set_children(ids.clone());
        let mut info = gpui::AccessibilityInfo::new(node);
        for (index, part) in parts.iter().enumerate() {
            let mut run = Node::new(Role::TextRun);
            run.set_value(text[part.range.clone()].to_owned());
            run.set_character_lengths(
                part.boundaries
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]) as u8)
                    .collect::<Vec<_>>(),
            );
            run.set_word_starts(part.words.clone());
            run.set_text_direction(TextDirection::LeftToRight);
            if index > 0 && parts[index - 1].line == part.line {
                run.set_previous_on_line(ids[index - 1]);
            }
            if index + 1 < parts.len() && parts[index + 1].line == part.line {
                run.set_next_on_line(ids[index + 1]);
            }
            info.text.push((ids[index], run));
        }
        info
    })
}
