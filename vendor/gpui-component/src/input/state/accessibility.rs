use super::*;
use crate::semantic_text::{text_run, visit_line_segments};
use gpui::accesskit::{
    Action, ActionData, ActionRequest, Node, NodeId, Role, TextPosition, TextSelection,
};
use gpui::accessibility::TextRunIds;
use gpui::WeakEntity;
use ropey::{LineType, extra::esoterica::ropes_are_instances};
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
pub(super) struct Mapping {
    owner: Option<TextRunIds>,
    current: TextRuns,
    running: bool,
}

#[derive(Clone, Default)]
struct TextRuns {
    revision: usize,
    text: Rope,
    runs: Arc<[Run]>,
    nodes: Arc<[(NodeId, Node)]>,
    next_key: usize,
}

pub(super) struct PrepareTextRuns {
    owner: TextRunIds,
    previous: TextRuns,
    text: Rope,
    changed: Option<(Range<usize>, Range<usize>)>,
}

#[derive(Clone)]
struct Run {
    id: NodeId,
    range: Range<usize>,
}
impl TextRuns {
    #[cfg(test)]
    fn update(&mut self, text: Rope, mut node_id: impl FnMut(usize) -> NodeId) {
        let changed = changed_lines(&self.text, &text);
        self.update_changed(text, changed, &mut node_id);
    }

    fn matches(&self, text: &Rope) -> bool {
        !self.nodes.is_empty() && ropes_are_instances(&self.text, text)
    }

    fn update_changed(
        &mut self,
        text: Rope,
        (old, new): (Range<usize>, Range<usize>),
        mut node_id: impl FnMut(usize) -> NodeId,
    ) {
        let prefix = self
            .runs
            .partition_point(|run| run.range.end <= old.start && run.range.start < old.start);
        let suffix = if old.end == self.text.len() {
            self.runs.len()
        } else {
            self.runs.partition_point(|run| run.range.start < old.end)
        };
        let mut nodes = Vec::with_capacity(self.nodes.len());
        let mut runs = Vec::with_capacity(self.runs.len());
        nodes.extend(self.nodes[..prefix].iter().cloned());
        runs.extend(self.runs[..prefix].iter().cloned());
        let mut previous: Option<(usize, NodeId, Node)> = None;
        let mut offset = new.start;
        for (line_index, line) in text.slice(new.clone()).lines(LineType::LF).enumerate() {
            if line.len() == 0 && new.end < text.len() {
                break;
            }
            let line = line
                .as_str()
                .map_or_else(|| Cow::Owned(line.to_string()), Cow::Borrowed);
            visit_line_segments(&line, line_index, |part| {
                let id = if runs.len() < suffix {
                    self.runs[runs.len()].id
                } else {
                    let key = self.next_key;
                    self.next_key += 1;
                    node_id(key)
                };
                let range = offset + part.range.start..offset + part.range.end;
                let mut run = text_run(&line[part.range], part.lengths, part.words);
                if let Some((line, previous_id, mut previous)) = previous.take() {
                    if line == part.line {
                        previous.set_next_on_line(id);
                        run.set_previous_on_line(previous_id);
                    }
                    nodes.push((previous_id, previous));
                }
                previous = Some((part.line, id, run));
                runs.push(Run { id, range });
            });
            offset += line.len();
        }
        if let Some((_, id, node)) = previous {
            nodes.push((id, node));
        }
        nodes.extend(self.nodes[suffix..].iter().cloned());
        runs.extend(self.runs[suffix..].iter().map(|run| Run {
            id: run.id,
            range: run.range.start - old.end + new.end..run.range.end - old.end + new.end,
        }));
        self.text = text;
        self.nodes = nodes.into();
        self.runs = runs.into();
        static NEXT_REVISION: AtomicUsize = AtomicUsize::new(1);
        self.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
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

impl Mapping {
    fn prepare(&mut self, owner: TextRunIds, text: &Rope) -> Option<PrepareTextRuns> {
        if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.current = TextRuns::default();
        }
        if self.current.matches(text) || self.running {
            return None;
        }
        let changed = (self.current.text.len().max(text.len()) <= 512 * 1024)
            .then(|| changed_lines(&self.current.text, text));
        if let Some(changed) = changed.as_ref()
            && changed.0.len().max(changed.1.len()) <= 64 * 1024
            && self.current.nodes.len() <= 4096
            && text.slice(changed.1.clone()).len_lines(LineType::LF) <= 4096
        {
            self.current
                .update_changed(text.clone(), changed.clone(), |key| owner.get(key));
            return None;
        }
        self.running = true;
        Some(PrepareTextRuns {
            owner,
            previous: self.current.clone(),
            text: text.clone(),
            changed,
        })
    }

    fn finish(&mut self, owner: TextRunIds, current: TextRuns, text: &Rope) {
        self.running = false;
        if self.owner == Some(owner) && current.matches(text) {
            self.current = current;
        }
    }
}

impl PrepareTextRuns {
    fn build(mut self) -> TextRuns {
        let changed = self
            .changed
            .unwrap_or_else(|| changed_lines(&self.previous.text, &self.text));
        self.previous
            .update_changed(self.text, changed, |key| self.owner.get(key));
        self.previous
    }

    pub(super) async fn run(self, input: WeakEntity<InputState>, cx: &mut gpui::AsyncApp) {
        let owner = self.owner;
        let current = cx.background_spawn(async move { self.build() }).await;
        _ = input.update(cx, |state, cx| {
            state
                .accessibility_mapping
                .borrow_mut()
                .finish(owner, current, &state.text);
            cx.notify();
        });
    }
}
fn common_bytes(first: &Rope, second: &Rope, limit: usize, reverse: bool) -> usize {
    let mut matched = 0;
    while matched < limit {
        let first_index = if reverse {
            first.len() - matched - 1
        } else {
            matched
        };
        let second_index = if reverse {
            second.len() - matched - 1
        } else {
            matched
        };
        let (first_chunk, first_start) = first.chunk(first_index);
        let (second_chunk, second_start) = second.chunk(second_index);
        let first_local = first_index - first_start;
        let second_local = second_index - second_start;
        let remaining = limit - matched;
        let (first, second) = if reverse {
            let length = remaining.min(first_local + 1).min(second_local + 1);
            (
                &first_chunk.as_bytes()[first_local + 1 - length..=first_local],
                &second_chunk.as_bytes()[second_local + 1 - length..=second_local],
            )
        } else {
            let length = remaining
                .min(first_chunk.len() - first_local)
                .min(second_chunk.len() - second_local);
            (
                &first_chunk.as_bytes()[first_local..first_local + length],
                &second_chunk.as_bytes()[second_local..second_local + length],
            )
        };
        if first != second {
            return matched
                + if reverse {
                    first
                        .iter()
                        .rev()
                        .zip(second.iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count()
                } else {
                    first.iter().zip(second).take_while(|(a, b)| a == b).count()
                };
        }
        matched += first.len();
    }
    matched
}

fn changed_lines(old: &Rope, new: &Rope) -> (Range<usize>, Range<usize>) {
    let limit = old.len().min(new.len());
    let prefix = common_bytes(old, new, limit, false);
    let suffix = common_bytes(old, new, limit - prefix, true);
    let start = old.line_to_byte_idx(old.byte_to_line_idx(prefix, LineType::LF), LineType::LF);
    let end = |text: &Rope| {
        let line = text.byte_to_line_idx(text.len() - suffix, LineType::LF);
        text.line_to_byte_idx(line + 1, LineType::LF)
    };
    (start..end(old), start..end(new))
}

impl InputState {
    pub fn release_accessibility_cache(&mut self) {
        let mut mapping = self.accessibility_mapping.borrow_mut();
        if mapping.owner.take().is_some() {
            mapping.current = TextRuns::default();
        }
    }

    pub(super) fn accessibility_info(
        &self,
        id: &gpui::GlobalElementId,
        window: &mut Window,
        mapping: &mut Mapping,
    ) -> (gpui::AccessibilityInfo, Option<PrepareTextRuns>) {
        if self.masked {
            let mut node = Node::new(Role::PasswordInput);
            node.set_label(
                self.accessibility_label
                    .as_ref()
                    .unwrap_or(&self.placeholder)
                    .to_string(),
            );
            node.set_value("•".repeat(self.text.len_chars()));
            node.add_action(Action::Focus);
            if self.disabled {
                node.set_read_only();
            } else {
                node.add_action(Action::SetValue);
            }
            let mut info = gpui::AccessibilityInfo::new(node);
            info.focused = self.focus_handle.is_focused(window);
            return (info, None);
        }
        let owner = window.accessibility_text_run_ids(id);
        let work = mapping.prepare(owner, &self.text);
        let current = &mapping.current;
        let ready = current.matches(&self.text) && !self.initial_layout_pending();
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
        if self.disabled || !ready {
            node.set_read_only();
        } else {
            node.add_action(Action::SetValue);
            node.add_action(Action::ReplaceSelectedText);
        }
        node.add_action(Action::Focus);
        node.set_children(current.nodes.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        node.set_text_revision(current.revision);
        if ready {
            node.add_action(Action::SetTextSelection);
            let start = current.position(self.selected_range.start);
            let end = current.position(self.selected_range.end);
            let (anchor, focus) = if self.selection_reversed {
                (end, start)
            } else {
                (start, end)
            };
            node.set_text_selection(TextSelection { anchor, focus });
        } else {
            node.set_busy();
        }
        let mut info = gpui::AccessibilityInfo::new(node);
        info.text = current.nodes.clone();
        info.focused = self.focus_handle.is_focused(window);
        (info, work)
    }
    pub(super) fn accessibility_action(
        &mut self,
        request: ActionRequest,
        mapping: &Mapping,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if request.action == Action::Focus {
            self.focus(window, cx);
            return true;
        }
        if self.initial_layout_pending() || (!self.masked && !mapping.current.matches(&self.text)) {
            return false;
        }
        match (request.action, request.data) {
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
                let (Some(anchor), Some(focus)) = (
                    mapping.current.offset(&selection.anchor),
                    mapping.current.offset(&selection.focus),
                ) else {
                    return false;
                };
                self.selected_range = (anchor.min(focus)..anchor.max(focus)).into();
                self.selection_reversed = focus < anchor;
                self.focus(window, cx);
                self.update_preferred_column();
                cx.notify();
                true
            }
            (Action::SetValue | Action::ReplaceSelectedText, Some(ActionData::Value(value)))
                if !self.disabled =>
            {
                let range = (request.action == Action::SetValue).then(|| 0..self.text.len_utf16());
                self.history.new_group();
                let applied = self.replace_text_checked(range, &value, window, cx);
                self.history.new_group();
                applied
            }
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic_text::segments;

    #[gpui::test]
    fn large_accessible_updates_reject_stale_text_and_owners(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|window, _| {
            let owner = window.accessibility_text_run_ids(&gpui::GlobalElementId::default());
            let mut new_id = gpui::GlobalElementId::default();
            new_id.push("another input".into());
            let new_owner = window.accessibility_text_run_ids(&new_id);
            let initial = Rope::from("original café\r\n".repeat(5_000));
            let latest = "new é🙂\r\n".repeat(6_000);
            let latest_rope = Rope::from(latest.as_str());
            let mut mapping = Mapping::default();
            let pending = mapping.prepare(owner, &initial).unwrap();
            assert!(mapping.prepare(owner, &latest_rope).is_none());
            mapping.finish(owner, pending.build(), &latest_rope);
            assert!(mapping.current.nodes.is_empty());

            let pending = mapping.prepare(owner, &latest_rope).unwrap();
            assert!(mapping.prepare(new_owner, &latest_rope).is_none());
            mapping.finish(owner, pending.build(), &latest_rope);
            assert!(mapping.current.nodes.is_empty());

            let pending = mapping.prepare(new_owner, &latest_rope).unwrap();
            mapping.finish(new_owner, pending.build(), &latest_rope);
            assert_complete_mapping(&mapping.current, &latest);
            assert!(mapping.prepare(new_owner, &latest_rope).is_none());
        });
    }

    fn assert_complete_mapping(mapping: &TextRuns, text: &str) {
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
            let words = if node.wide_word_starts().is_empty() {
                node.word_starts()
                    .iter()
                    .copied()
                    .map(u16::from)
                    .collect::<Vec<_>>()
            } else {
                node.wide_word_starts().to_vec()
            };
            assert_eq!(words, part.words);
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
    fn accessible_diff_preserves_lf_boundaries_across_rope_chunks() {
        let base = "é🙂 first\r\n中 second\rlast\n".repeat(800);
        let previous = Rope::from(base.as_str());
        for (start, end, insert) in [
            (0, 1, "q"),
            (0, 0, "new\r\n"),
            (1599, 1602, "x"),
            (1600, 1600, "中文"),
            (base.len() - 1, base.len(), ""),
            (base.len(), base.len(), "新"),
            (0, base.len(), ""),
        ] {
            let mut start = start;
            let mut end = end;
            while !base.is_char_boundary(start) {
                start -= 1;
            }
            while !base.is_char_boundary(end) {
                end += 1;
            }
            let mut next = previous.clone();
            next.remove(start..end);
            next.insert(start, insert);
            let next_text = next.to_string();
            let (old, new) = changed_lines(&previous, &next);
            assert_eq!(old.start, new.start);
            assert_eq!(&base[..old.start], &next_text[..new.start]);
            assert_eq!(&base[old.end..], &next_text[new.end..]);
            if old.start > 0 {
                assert_eq!(base.as_bytes()[old.start - 1], b'\n');
            }
            if old.end < base.len() {
                assert_eq!(base.as_bytes()[old.end - 1], b'\n');
            }
            if new.end < next_text.len() {
                assert_eq!(next_text.as_bytes()[new.end - 1], b'\n');
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
            let mut mapping = TextRuns::default();
            mapping.update(Rope::from(text.as_str()), |key| NodeId(key as u64 + 10));
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
                mapping.update(Rope::from(text.as_str()), |key| NodeId(key as u64 + 10));
                assert_complete_mapping(&mapping, &text);
            }
        }
    }
    #[test]
    fn unchanged_accessible_lines_keep_their_identity_across_edits() {
        let mut mapping = TextRuns::default();
        let initial = "first\nsecond\nthird\nlast é🙂";
        mapping.update(Rope::from(initial), |key| NodeId(key as u64 + 10));
        let last = mapping.nodes.last().unwrap().clone();
        let changed = "changed first\nsecond\nthird\nlast é🙂";
        mapping.update(Rope::from(changed), |key| NodeId(key as u64 + 10));
        assert_eq!(mapping.nodes.last(), Some(&last));
        assert_complete_mapping(&mapping, changed);
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
            let mut mapping = TextRuns::default();
            let mut nodes = vec![];
            let mut runs = vec![];
            for (index, segment) in segments.iter().enumerate() {
                let id = NodeId((segments.len() - index) as u64 + 10);
                let mut node = Node::new(Role::TextRun);
                node.set_character_lengths(segment.lengths.clone());
                nodes.push((id, node));
                runs.push(Run {
                    id,
                    range: segment.range.clone(),
                });
            }
            mapping.nodes = nodes.into();
            mapping.runs = runs.into();
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
        assert!(runs
            .iter()
            .all(|run| run.lengths.len() <= crate::semantic_text::MAX_RUN_CHARACTERS));
        assert!(runs.iter().any(|run| run.lengths.len() > 255));
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
