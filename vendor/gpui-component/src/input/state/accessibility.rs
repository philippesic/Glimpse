use super::*;
use gpui::accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextDirection, TextPosition,
    TextSelection,
};

#[derive(Default)]
pub(super) struct Mapping {
    text: SharedString,
    runs: Vec<Run>,
}
struct Run {
    id: NodeId,
    boundaries: Vec<usize>,
}
struct Segment {
    range: Range<usize>,
    boundaries: Vec<usize>,
    words: Vec<u8>,
    line: usize,
}
fn segments(text: &str) -> Vec<Segment> {
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
impl Mapping {
    fn offset(&self, position: &TextPosition) -> Option<usize> {
        self.runs
            .iter()
            .find(|run| run.id == position.node)?
            .boundaries
            .get(position.character_index)
            .copied()
    }
    fn position(&self, offset: usize) -> TextPosition {
        let run = self
            .runs
            .iter()
            .find(|run| offset < *run.boundaries.last().unwrap())
            .unwrap_or_else(|| self.runs.last().unwrap());
        let character_index = run
            .boundaries
            .partition_point(|&byte| byte <= offset)
            .saturating_sub(1);
        TextPosition {
            node: run.id,
            character_index,
        }
    }
}
impl InputState {
    pub(super) fn accessibility_info(
        &self,
        id: &gpui::GlobalElementId,
        window: &mut Window,
        mapping: &mut Mapping,
    ) -> gpui::AccessibilityInfo {
        let text = self.value();
        if self.masked {
            let mut node = Node::new(Role::PasswordInput);
            node.set_label(
                self.accessibility_label
                    .as_ref()
                    .unwrap_or(&self.placeholder)
                    .to_string(),
            );
            node.set_value("•".repeat(text.chars().count()));
            node.add_action(Action::Focus);
            if self.disabled {
                node.set_read_only();
            } else {
                node.add_action(Action::SetValue);
            }
            let mut info = gpui::AccessibilityInfo::new(node);
            info.focused = self.focus_handle.is_focused(window);
            return info;
        }
        let segments = segments(&text);
        let ids: Vec<_> = (0..segments.len())
            .map(|index| window.accessibility_child_id(id, ("text-run", index)))
            .collect();
        mapping.text = text.clone();
        mapping.runs = segments
            .iter()
            .enumerate()
            .map(|(index, segment)| Run {
                id: ids[index],
                boundaries: segment.boundaries.clone(),
            })
            .collect();
        let mut node = Node::new(if self.mode.is_multi_line() {
            Role::MultilineTextInput
        } else {
            Role::TextInput
        });
        node.set_label(
            self.accessibility_label
                .as_ref()
                .unwrap_or(&self.placeholder)
                .to_string(),
        );
        if self.disabled {
            node.set_read_only();
        } else {
            node.add_action(Action::SetValue);
            node.add_action(Action::ReplaceSelectedText);
        }
        node.add_action(Action::Focus);
        node.add_action(Action::SetTextSelection);
        node.set_children(ids.clone());
        let start = mapping.position(self.selected_range.start);
        let end = mapping.position(self.selected_range.end);
        node.set_text_selection(if self.selection_reversed {
            TextSelection {
                anchor: end,
                focus: start,
            }
        } else {
            TextSelection {
                anchor: start,
                focus: end,
            }
        });
        let mut info = gpui::AccessibilityInfo::new(node);
        for (index, segment) in segments.iter().enumerate() {
            let mut run = Node::new(Role::TextRun);
            run.set_value(text[segment.range.clone()].to_owned());
            run.set_character_lengths(
                segment
                    .boundaries
                    .windows(2)
                    .map(|pair| (pair[1] - pair[0]) as u8)
                    .collect::<Vec<_>>(),
            );
            run.set_word_starts(segment.words.clone());
            run.set_text_direction(TextDirection::LeftToRight);
            if index > 0 && segments[index - 1].line == segment.line {
                run.set_previous_on_line(ids[index - 1]);
            }
            if index + 1 < ids.len() && segments[index + 1].line == segment.line {
                run.set_next_on_line(ids[index + 1]);
            }
            info.text.push((ids[index], run));
        }
        info.focused = self.focus_handle.is_focused(window);
        info
    }
    pub(super) fn accessibility_action(
        &mut self,
        request: ActionRequest,
        mapping: &Mapping,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request.action {
            Action::Focus => self.focus(window, cx),
            Action::SetTextSelection => {
                if self.value() != mapping.text {
                    return;
                }
                if let Some(ActionData::SetTextSelection(selection)) = request.data {
                    let (Some(anchor), Some(focus)) = (
                        mapping.offset(&selection.anchor),
                        mapping.offset(&selection.focus),
                    ) else {
                        return;
                    };
                    self.selected_range = (anchor.min(focus)..anchor.max(focus)).into();
                    self.selection_reversed = focus < anchor;
                    self.focus(window, cx);
                    self.update_preferred_column();
                    cx.notify();
                }
            }
            Action::SetValue | Action::ReplaceSelectedText if !self.disabled => {
                if let Some(ActionData::Value(value)) = request.data {
                    let range =
                        (request.action == Action::SetValue).then(|| 0..self.text.len_utf16());
                    self.history.new_group();
                    self.replace_text_in_range(range, &value, window, cx);
                    self.history.new_group();
                }
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runs_preserve_every_byte_and_cluster_across_hard_lines() {
        let text = format!("α é 👨‍👩‍👧\r\n{}\n", "long ".repeat(200));
        let runs = segments(&text);
        assert_eq!(
            runs.iter()
                .map(|run| &text[run.range.clone()])
                .collect::<String>(),
            text
        );
        assert!(runs.iter().all(|run| run.boundaries.len() <= 256));
        assert_eq!(&text[runs[0].boundaries[2]..runs[0].boundaries[3]], "é");
        assert_eq!(
            &text[runs[0].boundaries[runs[0].boundaries.len() - 2]..runs[0].range.end],
            "\r\n"
        );
        assert_eq!(runs.last().unwrap().range, text.len()..text.len());
    }
    #[test]
    fn empty_text_and_oversized_combining_sequences_remain_representable() {
        assert_eq!(segments("")[0].boundaries, vec![0]);
        let text = format!("a{}", "́".repeat(400));
        let runs = segments(&text);
        assert_eq!(
            runs.iter()
                .map(|run| &text[run.range.clone()])
                .collect::<String>(),
            text
        );
        assert!(runs.iter().all(|run| run
            .boundaries
            .windows(2)
            .all(|pair| pair[1] - pair[0] <= 255)));
    }
}
