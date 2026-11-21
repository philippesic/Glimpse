use std::ops::Range;

use accesskit_consumer::{FullNodeId, NodeRef, TextSnapshot};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    Error, Result,
    util::{
        sentence_at_offset, sentence_in_paragraph, sentence_paragraph, text_range_from_offsets,
    },
};

const MIN_INDEXED_PARAGRAPH: i32 = 64 * 1024;
const MIN_INDEXED_OFFSET: i32 = 16 * 1024;
pub(crate) type Sentence = (String, i32, i32);

#[derive(Clone, Copy, PartialEq, Eq)]
struct Revision {
    node: FullNodeId,
    text: usize,
}

pub(crate) struct SentenceIndex {
    revision: Revision,
    paragraph: Range<i32>,
    starts: Vec<u64>,
}

pub(crate) enum Lookup {
    Sentence(Sentence),
    Index(IndexRequest),
}

pub(crate) struct IndexRequest {
    revision: Revision,
    paragraph: Range<i32>,
    text: TextSnapshot,
}

pub(crate) fn lookup(node: &NodeRef, offset: i32, index: Option<&SentenceIndex>) -> Result<Lookup> {
    let revision = node
        .data()
        .text_revision()
        .filter(|_| node.role() == accesskit::Role::Document && node.is_read_only())
        .map(|text| Revision {
            node: node.id(),
            text,
        });
    if let Some(index) = index.filter(|index| Some(index.revision) == revision)
        && let Some(range) = index.sentence_range(offset)
    {
        let text = text_range_from_offsets(node, range.start, range.end)
            .ok_or(Error::IndexOutOfRange)?
            .text();
        return Ok(Lookup::Sentence((text, range.start, range.end)));
    }

    if let Some(revision) = revision.filter(|_| offset >= MIN_INDEXED_OFFSET) {
        let Some(paragraph) = sentence_paragraph(node, offset)? else {
            return Ok(Lookup::Sentence((String::new(), offset, offset)));
        };
        let start = i32::try_from(paragraph.start().to_global_usv_index())
            .map_err(|_| Error::TooManyCharacters)?;
        if offset - start >= MIN_INDEXED_OFFSET {
            let end = i32::try_from(paragraph.end().to_global_usv_index())
                .map_err(|_| Error::TooManyCharacters)?;
            if end - start >= MIN_INDEXED_PARAGRAPH {
                return Ok(Lookup::Index(IndexRequest {
                    revision,
                    paragraph: start..end,
                    text: paragraph.snapshot_text(),
                }));
            }
        }
        return sentence_in_paragraph(&paragraph, start as usize, offset).map(Lookup::Sentence);
    }

    sentence_at_offset(node, offset).map(Lookup::Sentence)
}

impl IndexRequest {
    pub(crate) fn build(self, offset: i32) -> Result<(SentenceIndex, Sentence)> {
        let mut text = String::with_capacity(self.text.fragments().map(str::len).sum());
        for fragment in self.text.fragments() {
            text.push_str(fragment);
        }
        let paragraph_length = (self.paragraph.end - self.paragraph.start) as usize;
        let offset = (offset - self.paragraph.start) as usize;
        let mut index = SentenceIndex {
            revision: self.revision,
            starts: vec![0; (paragraph_length + 1).div_ceil(64)],
            paragraph: self.paragraph,
        };
        let mut relative = 0;
        let mut found = None;
        index.mark(0);
        for sentence in text.split_sentence_bounds() {
            let end = relative + sentence.chars().count();
            if end > paragraph_length {
                return Err(Error::IndexOutOfRange);
            }
            if (relative..end).contains(&offset) {
                let start = index.paragraph.start + relative as i32;
                let end = index.paragraph.start + end as i32;
                found = Some((sentence.to_owned(), start, end));
            }
            index.mark(end);
            relative = end;
        }
        Ok((index, found.ok_or(Error::IndexOutOfRange)?))
    }
}

impl SentenceIndex {
    fn mark(&mut self, offset: usize) {
        self.starts[offset / 64] |= 1 << (offset % 64);
    }

    fn sentence_range(&self, offset: i32) -> Option<Range<i32>> {
        if !self.paragraph.contains(&offset) {
            return None;
        }
        let relative = (offset - self.paragraph.start) as usize;
        let (block, bit) = (relative / 64, relative % 64);
        let mask = u64::MAX >> (63 - bit);
        let lower = self.starts[block] & mask;
        let upper = self.starts[block] & !mask;

        let (lower_block, lower) = if lower != 0 {
            (block, lower)
        } else {
            self.starts[..block]
                .iter()
                .copied()
                .enumerate()
                .rfind(|(_, bits)| *bits != 0)?
        };
        let (upper_block, upper) = if upper != 0 {
            (block, upper)
        } else {
            self.starts
                .iter()
                .copied()
                .enumerate()
                .skip(block + 1)
                .find(|(_, bits)| *bits != 0)?
        };

        let start =
            self.paragraph.start + (lower_block * 64 + 63 - lower.leading_zeros() as usize) as i32;
        let end =
            self.paragraph.start + (upper_block * 64 + upper.trailing_zeros() as usize) as i32;
        Some(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
    use accesskit_consumer::Tree;

    fn reading_document(text: &str, revision: usize) -> Tree {
        let mut doc = Node::new(Role::Document);
        doc.set_children([NodeId(2)]);
        doc.set_read_only();
        doc.set_text_revision(revision);
        let mut run = Node::new(Role::TextRun);
        run.set_value(text);
        run.set_character_lengths(
            text.graphemes(true)
                .map(|g| g.len() as u8)
                .collect::<Vec<_>>(),
        );
        Tree::new(
            TreeUpdate {
                nodes: vec![(NodeId(1), doc), (NodeId(2), run)],
                tree: Some(TreeInfo::new(NodeId(1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(1),
            },
            true,
        )
    }

    #[test]
    fn cached_sentences_keep_scalar_ranges_exact_and_follow_text_revisions() {
        let text = "Café with e\u{301}. 🪴 Another sentence. 日本語。終わり。 ".repeat(2_048);
        let tree = reading_document(&text, 1);
        let root = tree.state().root();
        let end = text.chars().count() as i32;
        let Lookup::Index(request) = lookup(&root, end - 8, None).unwrap() else {
            panic!("expected a long paragraph");
        };
        let (index, initial) = request.build(end - 8).unwrap();
        assert_eq!(initial, sentence_at_offset(&root, end - 8).unwrap());
        for offset in [0, 5, 63, 64, 65, 813, end / 2, end - 1, end] {
            let Lookup::Sentence(cached) = lookup(&root, offset, Some(&index)).unwrap() else {
                panic!("unexpected cache miss at {offset}");
            };
            assert_eq!(
                cached,
                sentence_at_offset(&root, offset).unwrap(),
                "offset {offset}"
            );
        }
        let next = reading_document(&text.replace("Café", "Noël"), 2);
        assert!(matches!(
            lookup(&next.state().root(), end - 8, Some(&index)).unwrap(),
            Lookup::Index(_)
        ));
    }
}
