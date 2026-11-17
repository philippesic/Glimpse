use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct Segment {
    pub(crate) range: Range<usize>,
    pub(crate) lengths: Vec<u8>,
    pub(crate) words: Vec<u8>,
    pub(crate) line: usize,
}
impl Segment {
    #[cfg(test)]
    pub(crate) fn boundaries(&self) -> Vec<usize> {
        std::iter::once(self.range.start)
            .chain(self.lengths.iter().scan(self.range.start, |byte, &length| {
                *byte += usize::from(length);
                Some(*byte)
            }))
            .collect()
    }
}
pub(crate) fn segments(text: &str) -> Vec<Segment> {
    let mut segments = vec![];
    let mut offset = 0;
    for (line_index, line) in text
        .split_inclusive('\n')
        .chain(text.ends_with('\n').then_some(""))
        .enumerate()
    {
        let mut words = line
            .unicode_word_indices()
            .map(|(byte, _)| offset + byte)
            .peekable();
        let mut start = offset;
        let mut byte = offset;
        let mut lengths = Vec::new();
        let mut starts = Vec::new();
        let pieces = line.graphemes(true).flat_map(|grapheme| {
            if grapheme.len() <= u8::MAX as usize {
                smallvec::SmallVec::<[u8; 1]>::from_slice(&[grapheme.len() as u8])
            } else {
                grapheme
                    .chars()
                    .map(|character| character.len_utf8() as u8)
                    .collect()
            }
        });
        for length in pieces {
            if lengths.len() == 255 {
                segments.push(Segment {
                    range: start..byte,
                    lengths,
                    words: starts,
                    line: line_index,
                });
                start = byte;
                lengths = Vec::new();
                starts = Vec::new();
            }
            while words.peek().is_some_and(|&word| word < byte) {
                words.next();
            }
            if words.peek() == Some(&byte) {
                starts.push(lengths.len() as u8);
                words.next();
            }
            lengths.push(length);
            byte += usize::from(length);
        }
        let end = offset + line.len();
        segments.push(Segment {
            range: start..end,
            lengths,
            words: starts,
            line: line_index,
        });
        offset = end;
    }
    if segments.is_empty() {
        segments.push(Segment {
            range: 0..0,
            lengths: vec![],
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
        let mut text_nodes = Vec::new();
        for (index, part) in parts.iter().enumerate() {
            let mut run = Node::new(Role::TextRun);
            run.set_value(text[part.range.clone()].to_owned());
            run.set_character_lengths(part.lengths.clone());
            run.set_word_starts(part.words.clone());
            run.set_text_direction(TextDirection::LeftToRight);
            if index > 0 && parts[index - 1].line == part.line {
                run.set_previous_on_line(ids[index - 1]);
            }
            if index + 1 < parts.len() && parts[index + 1].line == part.line {
                run.set_next_on_line(ids[index + 1]);
            }
            text_nodes.push((ids[index], run));
        }
        info.text = text_nodes.into();
        info
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn text_run_ids_are_stable_and_distinct_between_parents(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let mut first = gpui::GlobalElementId::default();
            first.push("first".into());
            let mut second = gpui::GlobalElementId::default();
            second.push("second".into());
            let mut ids = std::collections::HashSet::new();
            for index in 0..10_000usize {
                let id = window.accessibility_child_id(&first, ("run", index));
                assert_eq!(id, window.accessibility_child_id(&first, ("run", index)));
                assert!(ids.insert(id));
                assert!(ids.insert(window.accessibility_child_id(&second, ("run", index))));
            }
        });
    }

    #[test]
    #[ignore = "release text-metadata CPU and memory probe; excludes native rendering and presentation"]
    fn benchmark_supported_text_metadata() {
        let unit = "Paragraph café 🙂 with **strong**, *emphasis* and `inline` plus [sibling](sibling.md). ".to_owned() + &"ordinary text ".repeat(12) + "\n\n";
        let repetitions = (10 * 1024 * 1024 - 4096) / unit.len();
        let source = "# Supported Markdown size\n\n".to_owned()
            + &unit.repeat(repetitions)
            + "\n# FINAL SUPPORTED SIZE MARKER\n";
        let started = std::time::Instant::now();
        let parts = segments(&source);
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        let starts: Vec<_> = parts
            .iter()
            .flat_map(|part| {
                let boundaries = part.boundaries();
                part.words
                    .iter()
                    .map(move |&index| boundaries[index as usize])
            })
            .collect();
        assert_eq!(
            starts,
            source
                .unicode_word_indices()
                .map(|(byte, _)| byte)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| &source[part.range.clone()])
                .collect::<String>(),
            source
        );
        println!("complete text metadata: bytes={}, runs={}, words={}, elapsed={elapsed:.3}ms; excludes rendering/presentation", source.len(), parts.len(), starts.len());
        #[cfg(target_os = "linux")]
        println!(
            "{}",
            std::fs::read_to_string("/proc/self/status")
                .unwrap()
                .lines()
                .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
                .collect::<Vec<_>>()
                .join("; ")
        );
        std::hint::black_box(parts);
    }
    #[test]
    fn compact_lengths_preserve_unicode_clusters_words_and_empty_lines() {
        let units = [
            "plain ",
            "café é ",
            "👩‍👩‍👧‍👦🙂 ",
            "中文 हिन्दी ",
            "\r\n",
            "\n",
            "'word' foo_bar ",
            "a\u{301}",
        ];
        for seed in 0..100 {
            let text = (0..seed + 1)
                .map(|index| units[(seed + index * 3) % units.len()])
                .collect::<String>()
                + &format!("a{}\r\n", "́".repeat(seed * 7));
            let parts = segments(&text);
            let expected = text
                .graphemes(true)
                .flat_map(|cluster| {
                    if cluster.len() <= 255 {
                        vec![cluster.len() as u8]
                    } else {
                        cluster
                            .chars()
                            .map(|character| character.len_utf8() as u8)
                            .collect()
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(
                parts
                    .iter()
                    .flat_map(|part| part.lengths.iter().copied())
                    .collect::<Vec<_>>(),
                expected
            );
            let words = parts
                .iter()
                .flat_map(|part| {
                    let boundaries = part.boundaries();
                    part.words
                        .iter()
                        .map(move |&index| boundaries[index as usize])
                })
                .collect::<Vec<_>>();
            assert_eq!(
                words,
                text.unicode_word_indices()
                    .map(|(byte, _)| byte)
                    .collect::<Vec<_>>()
            );
            assert!(parts.iter().all(|part| part.lengths.len() <= 255));
            assert_eq!(
                parts
                    .iter()
                    .map(|part| &text[part.range.clone()])
                    .collect::<String>(),
                text
            );
            assert_eq!(parts.last().unwrap().range, text.len()..text.len());
        }
    }
    #[test]
    fn long_line_segments_preserve_every_word_and_grapheme() {
        let text = "🙂 café alpha_ 中文 é ".repeat(100_000);
        let segments = segments(&text);
        let starts: Vec<_> = segments
            .iter()
            .flat_map(|segment| {
                let boundaries = segment.boundaries();
                segment
                    .words
                    .iter()
                    .map(move |&index| boundaries[index as usize])
            })
            .collect();
        let expected: Vec<_> = text
            .unicode_word_indices()
            .map(|(offset, _)| offset)
            .collect();
        assert_eq!(starts, expected);
        assert_eq!(
            segments
                .iter()
                .map(|segment| &text[segment.range.clone()])
                .collect::<String>(),
            text
        );
        assert!(segments
            .iter()
            .all(|segment| segment.boundaries().len() <= 256));
    }
}
