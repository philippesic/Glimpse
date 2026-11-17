use super::*;
use crate::semantic_text::segments;
use gpui::accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextDirection, TextPosition,
    TextSelection,
};

#[derive(Default)]
pub(super) struct Mapping {
    owner: Option<gpui::accessibility::TextRunIds>,
    text: SharedString,
    runs: Vec<Run>,
    nodes: std::sync::Arc<[(NodeId, Node)]>,
    next_key: usize,
}
#[derive(Clone)]
struct Run {
    id: NodeId,
    range: Range<usize>,
}
impl Mapping {
    fn update(&mut self, text: SharedString, mut node_id: impl FnMut(usize) -> NodeId) {
        let (old, new) = changed_lines(&self.text, &text);
        let prefix = self
            .runs
            .partition_point(|run| run.range.end <= old.start && run.range.start < old.start);
        let suffix = if old.end == self.text.len() {
            self.runs.len()
        } else {
            self.runs.partition_point(|run| run.range.start < old.end)
        };
        let mut parts = segments(&text[new.clone()]);
        if new.end < text.len() && parts.last().is_some_and(|part| part.range.is_empty()) {
            parts.pop();
        }
        let ids: Vec<_> = (0..parts.len())
            .map(|index| {
                if let Some(run) = self
                    .runs
                    .get(prefix + index)
                    .filter(|_| prefix + index < suffix)
                {
                    run.id
                } else {
                    let key = self.next_key;
                    self.next_key += 1;
                    node_id(key)
                }
            })
            .collect();
        let mut nodes = Vec::with_capacity(prefix + parts.len() + self.nodes.len() - suffix);
        let mut runs = Vec::with_capacity(nodes.capacity());
        nodes.extend(self.nodes[..prefix].iter().cloned());
        runs.extend(self.runs[..prefix].iter().cloned());
        let mut parts = parts.into_iter().enumerate().peekable();
        let mut previous_line = None;
        while let Some((index, part)) = parts.next() {
            let mut run = Node::new(Role::TextRun);
            let range = new.start + part.range.start..new.start + part.range.end;
            run.set_value(text[range.clone()].to_owned());
            run.set_character_lengths(part.lengths);
            run.set_word_starts(part.words);
            run.set_text_direction(TextDirection::LeftToRight);
            if previous_line == Some(part.line) {
                run.set_previous_on_line(ids[index - 1]);
            }
            if parts.peek().is_some_and(|(_, next)| next.line == part.line) {
                run.set_next_on_line(ids[index + 1]);
            }
            previous_line = Some(part.line);
            nodes.push((ids[index], run));
            runs.push(Run {
                id: ids[index],
                range,
            });
        }
        nodes.extend(self.nodes[suffix..].iter().cloned());
        runs.extend(self.runs[suffix..].iter().map(|run| Run {
            id: run.id,
            range: run.range.start - old.end + new.end..run.range.end - old.end + new.end,
        }));
        self.text = text;
        self.nodes = nodes.into();
        self.runs = runs;
    }

    fn offset(&self, position: &TextPosition) -> Option<usize> {
        let index = self.runs.iter().position(|run| run.id == position.node)?;
        let lengths = self.nodes[index].1.character_lengths();
        (position.character_index <= lengths.len()).then(|| {
            self.runs[index].range.start
                + lengths[..position.character_index]
                    .iter()
                    .map(|&length| usize::from(length))
                    .sum::<usize>()
        })
    }
    fn position(&self, offset: usize) -> TextPosition {
        let index = self
            .runs
            .partition_point(|run| run.range.end <= offset)
            .min(self.runs.len() - 1);
        let run = &self.runs[index];
        let mut byte = run.range.start;
        let character_index = self.nodes[index]
            .1
            .character_lengths()
            .iter()
            .take_while(|&&length| {
                byte += usize::from(length);
                byte <= offset
            })
            .count();
        TextPosition {
            node: run.id,
            character_index,
        }
    }
}
fn common_bytes(first: &[u8], second: &[u8], reverse: bool) -> usize {
    let length = first.len().min(second.len());
    let first = if reverse {
        &first[first.len() - length..]
    } else {
        &first[..length]
    };
    let second = if reverse {
        &second[second.len() - length..]
    } else {
        &second[..length]
    };
    let mut matched = 0;
    if reverse {
        for (first, second) in first.rchunks(1024).zip(second.rchunks(1024)) {
            if first != second {
                return matched
                    + first
                        .iter()
                        .rev()
                        .zip(second.iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count();
            }
            matched += first.len();
        }
    } else {
        for (first, second) in first.chunks(1024).zip(second.chunks(1024)) {
            if first != second {
                return matched + first.iter().zip(second).take_while(|(a, b)| a == b).count();
            }
            matched += first.len();
        }
    }
    matched
}
fn changed_lines(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let prefix = common_bytes(old.as_bytes(), new.as_bytes(), false);
    let suffix = common_bytes(&old.as_bytes()[prefix..], &new.as_bytes()[prefix..], true);
    let start = old.as_bytes()[..prefix]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |byte| byte + 1);
    let old_suffix = old.len() - suffix;
    let new_suffix = new.len() - suffix;
    let old_end = old.as_bytes()[old_suffix..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(old.len(), |byte| old_suffix + byte + 1);
    let new_end = new.as_bytes()[new_suffix..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(new.len(), |byte| new_suffix + byte + 1);
    (start..old_end, start..new_end)
}

impl InputState {
    pub fn release_accessibility_cache(&mut self) {
        let mut mapping = self.accessibility_mapping.borrow_mut();
        if !mapping.nodes.is_empty() {
            *mapping = Mapping::default();
        }
    }

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
        let owner = window.accessibility_text_run_ids(id);
        if mapping.owner != Some(owner) {
            *mapping = Mapping {
                owner: Some(owner),
                ..Mapping::default()
            };
        }
        if mapping.nodes.is_empty()
            || mapping.text.as_ptr() != text.as_ptr()
            || mapping.text.len() != text.len()
        {
            mapping.update(text.clone(), |index| owner.get(index));
        }
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
        node.set_children(mapping.nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
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
        info.text = mapping.nodes.clone();
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
    fn assert_complete_mapping(mapping: &Mapping, text: &str) {
        let parts = segments(text);
        assert_eq!(mapping.nodes.len(), parts.len());
        assert_eq!(mapping.runs.len(), parts.len());
        let mut unique = std::collections::HashSet::new();
        for (index, ((id, node), part)) in mapping.nodes.iter().zip(&parts).enumerate() {
            assert!(unique.insert(*id));
            assert_eq!(mapping.runs[index].id, *id);
            assert_eq!(mapping.runs[index].range, part.range);
            assert_eq!(node.value(), Some(&text[part.range.clone()]));
            assert_eq!(node.character_lengths(), part.lengths.clone());
            assert_eq!(node.word_starts(), part.words);
            assert_eq!(
                node.previous_on_line(),
                (index > 0 && parts[index - 1].line == part.line)
                    .then(|| mapping.runs[index - 1].id)
            );
            assert_eq!(
                node.next_on_line(),
                (index + 1 < parts.len() && parts[index + 1].line == part.line)
                    .then(|| mapping.runs[index + 1].id)
            );
            for &byte in &part.boundaries() {
                let position = mapping.position(byte);
                assert_eq!(mapping.offset(&position), Some(byte));
            }
        }
    }
    #[test]
    fn incremental_accessible_edits_preserve_complete_text_and_positions() {
        for initial in [
            String::new(),
            "α é 👨‍👩‍👧\r\nnext\n".into(),
            "first café\nsecond 🙂\nthird\nlast".repeat(60),
            "🙂 café 中文 ".repeat(200),
            format!("a{}\nend", "́".repeat(400)),
        ] {
            let mut text = initial;
            let mut mapping = Mapping::default();
            mapping.update(text.clone().into(), |key| NodeId(key as u64 + 10));
            assert_complete_mapping(&mapping, &text);
            let mut random = 23u64;
            for step in 0..100 {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                let boundaries: Vec<_> = text
                    .char_indices()
                    .map(|(byte, _)| byte)
                    .chain(std::iter::once(text.len()))
                    .collect();
                let start = random as usize % boundaries.len();
                let end = (start + (random >> 32) as usize % 9).min(boundaries.len() - 1);
                let inserted = ["x", "", "\nnew\r\n", "é🙂", "👩‍👩‍👧‍👦", "\r", "中文"][step % 7];
                text.replace_range(boundaries[start]..boundaries[end], inserted);
                mapping.update(text.clone().into(), |key| NodeId(key as u64 + 10));
                assert_complete_mapping(&mapping, &text);
            }
        }
    }
    #[test]
    fn unchanged_accessible_lines_keep_their_identity_across_edits() {
        let mut mapping = Mapping::default();
        let initial = "first\nsecond\nthird\nlast é🙂";
        mapping.update(initial.into(), |key| NodeId(key as u64 + 10));
        let last = mapping.nodes.last().unwrap().clone();
        mapping.update("changed first\nsecond\nthird\nlast é🙂".into(), |key| {
            NodeId(key as u64 + 10)
        });
        assert_eq!(mapping.nodes.last(), Some(&last));
        assert_complete_mapping(&mapping, &mapping.text);
    }

    #[test]
    fn compact_positions_match_grapheme_boundaries_and_reject_invalid_actions() {
        for text in [
            String::new(),
            "α é 👨‍👩‍👧\r\nnext\n".into(),
            "🙂 café 中文 ".repeat(1000),
            format!("a{}", "́".repeat(400)),
        ] {
            let segments = segments(&text);
            let mut mapping = Mapping::default();
            let mut nodes = vec![];
            for (index, segment) in segments.iter().enumerate() {
                let id = NodeId((segments.len() - index) as u64 + 10);
                let mut node = Node::new(Role::TextRun);
                node.set_character_lengths(segment.lengths.clone());
                nodes.push((id, node));
                mapping.runs.push(Run {
                    id,
                    range: segment.range.clone(),
                });
            }
            mapping.nodes = nodes.into();
            for offset in 0..=text.len() {
                let index = segments
                    .iter()
                    .position(|run| offset < run.range.end)
                    .unwrap_or(segments.len() - 1);
                let character = segments[index]
                    .boundaries()
                    .partition_point(|&byte| byte <= offset)
                    .saturating_sub(1);
                let position = mapping.position(offset);
                assert_eq!(position.node, mapping.runs[index].id);
                assert_eq!(position.character_index, character);
                assert_eq!(
                    mapping.offset(&position),
                    Some(segments[index].boundaries()[character])
                );
            }
            assert_eq!(
                mapping.offset(&TextPosition {
                    node: mapping.runs[0].id,
                    character_index: segments[0].boundaries().len(),
                }),
                None
            );
            assert_eq!(
                mapping.offset(&TextPosition {
                    node: NodeId(0),
                    character_index: 0
                }),
                None
            );
        }
    }
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
        assert!(runs.iter().all(|run| run.boundaries().len() <= 256));
        assert_eq!(&text[runs[0].boundaries()[2]..runs[0].boundaries()[3]], "é");
        assert_eq!(
            &text[runs[0].boundaries()[runs[0].boundaries().len() - 2]..runs[0].range.end],
            "\r\n"
        );
        assert_eq!(runs.last().unwrap().range, text.len()..text.len());
    }
    #[test]
    fn empty_text_and_oversized_combining_sequences_remain_representable() {
        assert_eq!(segments("")[0].boundaries(), vec![0]);
        let text = format!("a{}", "́".repeat(400));
        let runs = segments(&text);
        assert_eq!(
            runs.iter()
                .map(|run| &text[run.range.clone()])
                .collect::<String>(),
            text
        );
        assert!(runs.iter().all(|run| run
            .boundaries()
            .windows(2)
            .all(|pair| pair[1] - pair[0] <= 255)));
    }
}
