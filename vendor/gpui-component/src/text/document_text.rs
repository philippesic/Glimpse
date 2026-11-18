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
const MAX_CACHED_LAYOUTS: usize = 32;
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
        let mut source = DocumentSourceBuilder::default();
        let mut batch = SourceBuilder {
            source: &mut source,
            inlines: Vec::new(),
        };
        batch.block(root);
        batch.flush();
        source.finish()
    }
}

#[derive(Default)]
pub(crate) struct DocumentSourceBuilder {
    chunks: Vec<Arc<str>>,
    shared: HashSet<Arc<str>>,
    buffer: String,
    last: Option<char>,
}

impl DocumentSourceBuilder {
    pub(crate) fn append_blocks(&mut self, blocks: &mut [Block]) {
        let mut batch = SourceBuilder {
            source: self,
            inlines: Vec::new(),
        };
        batch.blocks(blocks);
        batch.flush();
    }

    pub(super) fn finish(self) -> DocumentSource {
        DocumentSource(self.chunks.into())
    }

    pub(super) fn finish_if_started(self) -> Option<DocumentSource> {
        (!self.chunks.is_empty()).then(|| self.finish())
    }
}

struct SourceBuilder<'a, 'storage> {
    source: &'storage mut DocumentSourceBuilder,
    inlines: Vec<(Range<u32>, &'a mut InlineText)>,
}

impl<'a> SourceBuilder<'a, '_> {
    fn block(&mut self, node: &'a mut Block) {
        match node {
            Block::Root { children } => self.blocks(Arc::make_mut(children).as_mut_slice()),
            Block::Blockquote { children }
            | Block::List { children, .. }
            | Block::ListItem { children, .. } => self.blocks(children),
            Block::ParagraphPage(p) => self.paragraph(p),
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
                        .source
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
        if self.source.last.is_some_and(|character| character != '\n') {
            self.write("\n");
        }
        if self.source.buffer.len() >= PACKED_TEXT_BYTES {
            self.flush();
        }
    }

    fn write(&mut self, value: &str) {
        self.append(value);
        self.flush_full_buffer();
    }

    fn append(&mut self, value: &str) {
        if let Some(last) = value.chars().next_back() {
            self.source.last = Some(last);
            self.source.buffer.push_str(value);
        }
    }

    fn write_inline(&mut self, value: &'a mut InlineText) {
        if value.len() >= PACKED_TEXT_BYTES {
            self.push_chunk(value.shared());
        } else {
            let start = self.source.buffer.len() as u32;
            self.append(value.as_str());
            let end = self.source.buffer.len() as u32;
            self.inlines.push((start..end, value));
            self.flush_full_buffer();
        }
    }

    fn flush_full_buffer(&mut self) {
        if self.source.buffer.len() >= 2 * PACKED_TEXT_BYTES {
            self.flush();
        }
    }

    fn push_chunk(&mut self, value: Arc<str>) {
        self.flush();
        self.source.last = value.chars().next_back();
        self.source.chunks.push(value);
    }

    fn flush(&mut self) {
        if self.source.buffer.is_empty() {
            return;
        }
        let text = if let Some(text) = self.source.shared.get(self.source.buffer.as_str()) {
            text.clone()
        } else {
            let text: Arc<str> = self.source.buffer.as_str().into();
            if self.source.shared.len() < MAX_CACHED_RUNS {
                self.source.shared.insert(text.clone());
            }
            text
        };
        self.source.buffer.clear();
        for (range, inline) in self.inlines.drain(..) {
            *inline = InlineText::Shared {
                text: text.clone(),
                range,
            };
        }
        self.source.chunks.push(text);
    }
}

#[derive(Default)]
struct TextBuilder {
    runs: Vec<Node>,
    text: HashMap<Arc<str>, Arc<[Node]>>,
    layouts: HashMap<u64, TextLayout>,
    layout_key: Vec<u8>,
    boundaries: HashMap<u64, Node>,
    lines: LineBoundaries,
}

struct TextLayout {
    sample: Arc<str>,
    runs: Arc<[Node]>,
}

impl TextBuilder {
    fn emit(&mut self, value: Arc<str>) {
        let templates = self.templates(value);
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

    fn templates(&mut self, value: Arc<str>) -> Arc<[Node]> {
        if let Some(cached) = self.text.get(&value) {
            return cached.clone();
        }
        let mut use_layout = (512..=2 * PACKED_TEXT_BYTES).contains(&value.len());
        if use_layout {
            normalize_digits(&mut self.layout_key, &value);
            use_layout = self.layout_key != value.as_bytes();
        }
        if use_layout {
            let key = self.layouts.hasher().hash_one(self.layout_key.as_slice());
            if let Some(layout) = self.layouts.get(&key) {
                if Arc::ptr_eq(&layout.sample, &value) {
                    return layout.runs.clone();
                }
                if layout
                    .sample
                    .bytes()
                    .map(normalize_digit)
                    .eq(self.layout_key.iter().copied())
                {
                    return layout.for_text(&value);
                }
            }
            let runs = self.build_templates(&value);
            if self.layouts.len() < MAX_CACHED_LAYOUTS {
                self.layouts.entry(key).or_insert_with(|| TextLayout {
                    sample: value,
                    runs: runs.clone(),
                });
            }
            return runs;
        }
        let templates = self.build_templates(&value);
        if self.text.len() < MAX_CACHED_RUNS {
            self.text.insert(value, templates.clone());
        }
        templates
    }

    fn build_templates(&mut self, value: &Arc<str>) -> Arc<[Node]> {
        let data = value.clone();
        let mut templates = Vec::new();
        let make_run = |range: Range<usize>, lengths: Vec<u8>, words: Vec<u16>, lines: Vec<u16>| {
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
        self.lines.visit_packed_segments(value, make_run);
        templates.into()
    }
}

impl TextLayout {
    fn for_text(&self, value: &Arc<str>) -> Arc<[Node]> {
        let mut offset = 0;
        let runs = self
            .runs
            .iter()
            .cloned()
            .map(|mut run| {
                let end = offset + run.value().unwrap().len();
                let assigned = run.set_shared_value(value.clone(), offset..end);
                debug_assert!(assigned);
                offset = end;
                run
            })
            .collect();
        debug_assert_eq!(offset, value.len());
        runs
    }
}

fn normalize_digit(byte: u8) -> u8 {
    if byte.is_ascii_digit() {
        b'0'
    } else {
        byte
    }
}

fn normalize_digits(key: &mut Vec<u8>, value: &str) {
    key.clear();
    key.extend(value.bytes().map(normalize_digit));
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
                normalize_digits(&mut self.key, line);
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
    fn packed_parser_batches_preserve_rendered_blocks_and_text() {
        use crate::text::{TextViewStyle, format::markdown::parse, node::NodeContext};

        let mut source = concat!(
            "# Title café\r\n\r\n",
            "Text **with a mark** and ![a raster](image.png) beside [a link][late].\n\n",
            "> Quoted العربية text with a hard break  \n> here.\n\n",
            "1. Code `mix`\n2. **Other** 🧪\n\n",
            "| Head | Body |\n| - | - |\n| 𐐀 | Note |\n\n",
            "```text\ncode before the next section\n```\n\n",
        )
        .repeat(300);
        for index in 0..400 {
            use std::fmt::Write;
            writeln!(source, "Distinct {index} café with **text**.\n").unwrap();
        }
        source.push_str("[late]: link.md \"Later title\"\n");
        let style = TextViewStyle::default();
        let parse = |builder| {
            parse(
                &source,
                &style,
                &mut NodeContext::default(),
                &style.highlight_theme,
                |_| {},
                builder,
            )
            .unwrap()
        };
        let mut original = parse(None);
        let original_text = DocumentSource::new(&mut original);
        let mut builder = DocumentSourceBuilder::default();
        let packed = parse(Some(&mut builder));
        let packed_text = builder.finish_if_started().unwrap();
        assert_eq!(original, packed);
        assert_eq!(
            original_text
                .0
                .iter()
                .map(AsRef::as_ref)
                .collect::<String>(),
            packed_text.0.iter().map(AsRef::as_ref).collect::<String>()
        );
    }

    #[test]
    fn reused_layouts_keep_exact_text_and_boundaries() {
        let original: Arc<str> = "Café 1\u{301}.e1, 3.1 🇪🇺 1\u{fe0f}\u{20e3} العربية 1𐐀\r\n"
            .repeat(100)
            .into();
        let changed: Arc<str> = original.replace('1', "8").into();
        let mut builder = TextBuilder::default();
        let expected = builder.build_templates(&changed);
        builder.templates(original);
        let reused = builder.templates(changed.clone());
        assert_eq!(builder.layouts.len(), 1);
        assert_eq!(expected, reused);
        assert_eq!(
            reused.iter().filter_map(Node::value).collect::<String>(),
            changed.as_ref()
        );
    }

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
