use std::{
    collections::{HashMap, HashSet},
    hash::BuildHasher,
    ops::Range,
    sync::Arc,
};

use gpui::{
    accessibility::TextRunIds,
    accesskit::{Node, NodeId, Role, TextPosition, TextSelection},
};

use super::node::{InlineText, Node as Block, Paragraph};
use crate::semantic_text::{MAX_RUN_CHARACTERS, Segment, text_run_with_boundaries, visit_line_segments};

const MAX_CACHED_RUNS: usize = 256;
const MAX_CACHED_LINE_BYTES: usize = 1024;
const PACKED_TEXT_BYTES: usize = 8 * 1024;

#[derive(PartialEq)]
pub(super) struct DocumentText {
    pub(super) owner: TextRunIds,
    pub(super) nodes: Arc<[(NodeId, Node)]>,
    pub(super) selection: Option<TextSelection>,
}

impl DocumentText {
    pub(super) fn new(source: &DocumentSource, owner: TextRunIds) -> Self {
        let mut builder = TextBuilder::default();
        for text in source.0.iter() {
            builder.emit(text.clone());
        }
        let mut nodes = Vec::with_capacity(builder.runs.len() + 1);
        let mut content = Node::new(Role::GenericContainer);
        content.set_hidden();
        content.set_children(
            (1..=builder.runs.len())
                .map(|key| owner.get(key))
                .collect::<Vec<_>>(),
        );
        nodes.push((owner.get(0), content));
        nodes.extend(
            builder
                .runs
                .into_iter()
                .enumerate()
                .map(|(index, run)| (owner.get(index + 1), run)),
        );
        let selection = nodes.get(1).map(|(node, _)| {
            let start = TextPosition {
                node: *node,
                character_index: 0,
            };
            TextSelection {
                anchor: start,
                focus: start,
            }
        });
        Self {
            owner,
            nodes: nodes.into(),
            selection,
        }
    }

    fn contains(&self, position: TextPosition) -> bool {
        let Some(index) = position.node.0.checked_sub(self.owner.get(0).0) else {
            return false;
        };
        usize::try_from(index)
            .ok()
            .and_then(|index| self.nodes.get(index))
            .is_some_and(|(id, run)| {
                *id == position.node
                    && run.role() == Role::TextRun
                    && position.character_index <= run.character_lengths().len()
            })
    }

    pub(super) fn set_selection(&mut self, selection: TextSelection) -> bool {
        if self.contains(selection.anchor) && self.contains(selection.focus) {
            self.selection = Some(selection);
            true
        } else {
            false
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct DocumentSource(Arc<[Arc<str>]>);

impl DocumentSource {
    pub(super) fn new(root: &mut Block) -> Self {
        let mut builder = SourceBuilder::default();
        builder.block(root);
        builder.flush();
        Self(builder.chunks.into())
    }
}

#[derive(Default)]
struct SourceBuilder<'a> {
    chunks: Vec<Arc<str>>,
    shared: HashSet<Arc<str>>,
    buffer: String,
    inlines: Vec<(Range<u32>, &'a mut InlineText)>,
    last: Option<char>,
}

impl<'a> SourceBuilder<'a> {
    fn block(&mut self, node: &'a mut Block) {
        match node {
            Block::Root { children } => self.blocks(Arc::make_mut(children).as_mut_slice()),
            Block::Blockquote { children }
            | Block::List { children, .. }
            | Block::ListItem { children, .. } => self.blocks(children),
            Block::Paragraph(p) | Block::Heading { children: p, .. } => {
                self.paragraph_break();
                self.paragraph(p);
            }
            Block::CodeBlock(code) => {
                self.paragraph_break();
                let code = code.code();
                if code.len() >= PACKED_TEXT_BYTES {
                    self.push_chunk(code.into());
                } else {
                    self.write(&code);
                }
            }
            Block::Table(table) => {
                for row in &mut table.children {
                    self.paragraph_break();
                    for (index, cell) in row.children.iter_mut().enumerate() {
                        if index != 0 {
                            self.write("\t");
                        }
                        self.paragraph(&mut cell.children);
                    }
                }
            }
            Block::Break { .. } | Block::Divider => self.paragraph_break(),
            Block::Unknown => {}
        }
    }

    fn blocks(&mut self, blocks: &'a mut [Block]) {
        for block in blocks {
            self.block(block);
        }
    }

    fn paragraph(&mut self, paragraph: &'a mut Paragraph) {
        let mut image_before = false;
        for inline in &mut paragraph.children {
            if let Some(image) = &inline.image {
                if let Some(alt) = image.alt.as_ref().or(image.title.as_ref())
                    && !alt.is_empty()
                {
                    if self
                        .last
                        .is_some_and(|character| !character.is_whitespace())
                    {
                        self.write(" ");
                    }
                    if alt.len() >= PACKED_TEXT_BYTES {
                        self.push_chunk(alt.clone().into());
                    } else {
                        self.write(alt);
                    }
                    image_before = true;
                }
            } else if !inline.text.is_empty() {
                if image_before && !inline.text.starts_with(char::is_whitespace) {
                    self.write(" ");
                }
                self.write_inline(&mut inline.text);
                image_before = false;
            }
        }
    }

    fn paragraph_break(&mut self) {
        if self.last.is_some_and(|character| character != '\n') {
            self.write("\n");
        }
        if self.buffer.len() >= PACKED_TEXT_BYTES {
            self.flush();
        }
    }

    fn write(&mut self, value: &str) {
        self.append(value);
        self.flush_full_buffer();
    }

    fn append(&mut self, value: &str) {
        if let Some(last) = value.chars().next_back() {
            self.last = Some(last);
            self.buffer.push_str(value);
        }
    }

    fn write_inline(&mut self, value: &'a mut InlineText) {
        if value.len() >= PACKED_TEXT_BYTES {
            self.push_chunk(value.shared());
        } else {
            let start = self.buffer.len() as u32;
            self.append(value.as_str());
            let end = self.buffer.len() as u32;
            self.inlines.push((start..end, value));
            self.flush_full_buffer();
        }
    }

    fn flush_full_buffer(&mut self) {
        if self.buffer.len() >= 2 * PACKED_TEXT_BYTES {
            self.flush();
        }
    }

    fn push_chunk(&mut self, value: Arc<str>) {
        self.flush();
        self.last = value.chars().next_back();
        self.chunks.push(value);
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        let text = if let Some(text) = self.shared.get(self.buffer.as_str()) {
            text.clone()
        } else {
            let text: Arc<str> = self.buffer.as_str().into();
            if self.shared.len() < MAX_CACHED_RUNS {
                self.shared.insert(text.clone());
            }
            text
        };
        self.buffer.clear();
        for (range, inline) in self.inlines.drain(..) {
            *inline = InlineText::Shared {
                text: text.clone(),
                range,
            };
        }
        self.chunks.push(text);
    }
}

#[derive(Default)]
struct TextBuilder {
    runs: Vec<Node>,
    text: HashMap<Arc<str>, Arc<[Node]>>,
    boundaries: HashMap<u64, Node>,
    lines: LineBoundaries,
}

impl TextBuilder {
    fn emit(&mut self, value: Arc<str>) {
        let templates = if let Some(cached) = self.text.get(&value) {
            cached.clone()
        } else {
            let data = value.clone();
            let mut templates = Vec::new();
            let make_run =
                |range: Range<usize>, lengths: Vec<u8>, words: Vec<u16>, lines: Vec<u16>| {
                    let hash = self
                        .boundaries
                        .hasher()
                        .hash_one((&lengths, &words, &lines));
                    let cached = self.boundaries.get(&hash).filter(|run| {
                        run.character_lengths() == lengths
                            && run.hard_line_starts() == lines
                            && if run.wide_word_starts().is_empty() {
                                run.word_starts()
                                    .iter()
                                    .copied()
                                    .map(u16::from)
                                    .eq(words.iter().copied())
                            } else {
                                run.wide_word_starts() == words
                            }
                    });
                    let mut run = if let Some(cached) = cached {
                        cached.clone()
                    } else {
                        let mut run = text_run_with_boundaries(lengths, words);
                        if !lines.is_empty() {
                            run.set_hard_line_starts(lines);
                        }
                        if self.boundaries.len() < MAX_CACHED_RUNS {
                            self.boundaries.entry(hash).or_insert_with(|| run.clone());
                        }
                        run
                    };
                    let assigned = run.set_shared_value(data.clone(), range);
                    debug_assert!(assigned);
                    templates.push(run);
                };
            self.lines.visit_packed_segments(&value, make_run);
            let templates: Arc<[Node]> = templates.into();
            if self.text.len() < MAX_CACHED_RUNS {
                self.text.insert(value, templates.clone());
            }
            templates
        };
        for mut run in templates.iter().cloned() {
            if let Some(previous) = self.runs.last_mut()
                && previous.value().is_some_and(|text| !text.ends_with('\n'))
            {
                previous.set_text_run_continues_after();
                run.set_text_run_continues_before();
            }
            self.runs.push(run);
        }
    }
}

#[derive(Default)]
struct PackedSegment {
    range: Range<usize>,
    lengths: Vec<u8>,
    words: Vec<u16>,
    lines: Vec<u16>,
    last_line: usize,
}

impl PackedSegment {
    fn push(&mut self, segment: &Segment, offset: usize, line: usize) {
        let characters = self.lengths.len() as u16;
        if self.range.is_empty() {
            self.range.start = offset + segment.range.start;
        } else if line != self.last_line {
            self.lines.push(characters);
            if segment.words.first() != Some(&0) {
                self.words.push(characters);
            }
        }
        self.last_line = line;
        self.range.end = offset + segment.range.end;
        self.lengths.extend_from_slice(&segment.lengths);
        self.words
            .extend(segment.words.iter().map(|word| characters + word));
    }

    fn finish(self, emit: &mut impl FnMut(Range<usize>, Vec<u8>, Vec<u16>, Vec<u16>)) {
        if !self.range.is_empty() {
            emit(self.range, self.lengths, self.words, self.lines);
        }
    }
}

#[derive(Default)]
struct LineBoundaries {
    key: Vec<u8>,
    templates: HashMap<Vec<u8>, Segment>,
}

impl LineBoundaries {
    fn visit(&mut self, text: &str, mut visit: impl FnMut(&Segment, usize, usize)) {
        let mut offset = 0;
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let mut visit_at_offset = |segment: &Segment| visit(segment, offset, index);
            if line.len() <= MAX_CACHED_LINE_BYTES {
                self.key.clear();
                self.key.extend(line.bytes().map(
                    |byte| {
                        if byte.is_ascii_digit() {
                            b'0'
                        } else {
                            byte
                        }
                    },
                ));
                if let Some(segment) = self.templates.get(self.key.as_slice()) {
                    visit_at_offset(segment);
                } else {
                    self.build(line, visit_at_offset);
                }
            } else {
                visit_line_segments(line, index, |segment| visit_at_offset(&segment));
            }
            offset += line.len();
        }
    }

    fn build(&mut self, line: &str, mut visit: impl FnMut(&Segment)) {
        let cache = self.templates.len() < MAX_CACHED_RUNS;
        visit_line_segments(line, 0, |segment| {
            visit(&segment);
            if cache {
                self.templates.insert(self.key.clone(), segment);
            }
        });
    }

    fn visit_packed_segments(
        &mut self,
        text: &str,
        mut emit: impl FnMut(Range<usize>, Vec<u8>, Vec<u16>, Vec<u16>),
    ) {
        let mut packed = PackedSegment::default();
        self.visit(text, |segment, offset, line| {
            if packed.lengths.len() + segment.lengths.len() > MAX_RUN_CHARACTERS {
                std::mem::take(&mut packed).finish(&mut emit);
            }
            packed.push(segment, offset, line);
        });
        packed.finish(&mut emit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_reading_line_boundaries_match_unicode_segmentation() {
        use crate::semantic_text::visit_segments;

        let mut cache = LineBoundaries::default();
        let mut text = String::new();
        for digit in 0..=9 {
            text.push_str(&format!(
                "Café {digit}\u{301}.e{digit}, 3.{digit} 🇪🇺 1\u{fe0f}\u{20e3} العربية {digit}𐐀\r\n"
            ));
        }
        text.push_str(&"long text 🧑🏽‍🔬 a2.b3 ".repeat(250));
        text.push_str("\n\nEnd 99");
        for input in ["", "\n", text.as_str(), text.as_str()] {
            let mut expected = Vec::new();
            visit_segments(input, |segment| {
                if !segment.range.is_empty() {
                    expected.push((segment.range, segment.lengths, segment.words, segment.line));
                }
            });
            let mut actual = Vec::new();
            cache.visit(input, |segment, offset, index| {
                actual.push((
                    offset + segment.range.start..offset + segment.range.end,
                    segment.lengths.clone(),
                    segment.words.clone(),
                    index,
                ));
            });
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn large_unmeasured_list_keeps_offsets_across_splice_and_reset() {
        use gpui::{ListAlignment, ListOffset, ListState, px};

        let state = ListState::new(50_003, ListAlignment::Top, px(0.));
        assert_eq!(state.item_count(), 50_003);
        state.scroll_to(ListOffset {
            item_ix: 50_001,
            offset_in_item: px(8.),
        });
        state.splice(600..800, 17);
        assert_eq!(state.item_count(), 49_820);
        assert_eq!(state.logical_scroll_top().item_ix, 49_818);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(8.));
        state.reset(88_813);
        assert_eq!(state.item_count(), 88_813);
        assert_eq!(state.logical_scroll_top().item_ix, 0);
    }

    #[test]
    fn document_storage_shares_rendered_slices_and_preserves_block_boundaries() {
        let mut root = Block::Root {
            children: Arc::new(vec![
                Block::Heading {
                    level: 1,
                    children: Paragraph::new("A heading".into()),
                },
                Block::Paragraph(Paragraph::new("Café e\u{301} prose".into())),
                Block::Paragraph(Paragraph::new("Final paragraph".into())),
            ]),
        };
        let source = DocumentSource::new(&mut root);
        let [text] = source.0.as_ref() else {
            panic!("expected one text chunk")
        };
        assert_eq!(
            text.as_ref(),
            "A heading\nCafé e\u{301} prose\nFinal paragraph"
        );
        let Block::Root { children } = root else {
            unreachable!()
        };
        let Block::Paragraph(paragraph) = &children[1] else {
            unreachable!()
        };
        let InlineText::Shared {
            text: rendered,
            range,
        } = &paragraph.children[0].text
        else {
            panic!("expected a shared rendered slice");
        };
        assert!(Arc::ptr_eq(text, rendered));
        assert_eq!(
            &rendered[range.start as usize..range.end as usize],
            "Café e\u{301} prose"
        );
    }
}
