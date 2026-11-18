// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit::{Point, Rect, ScrollHint};
use accesskit_consumer::{NodeRef, TextPosition, TextRange};
use atspi_common::{CoordType, Granularity, ScrollType};
use unicode_segmentation::UnicodeSegmentation;

use crate::Error;

#[derive(Clone, Copy, Default, Debug)]
pub struct WindowBounds {
    pub outer: Rect,
    pub inner: Rect,
}

impl WindowBounds {
    pub fn new(outer: Rect, inner: Rect) -> Self {
        Self { outer, inner }
    }

    pub(crate) fn accesskit_point_to_atspi_point(
        &self,
        point: Point,
        parent: Option<NodeRef>,
        coord_type: CoordType,
    ) -> Point {
        let origin = self.origin(parent, coord_type);
        Point::new(origin.x + point.x, origin.y + point.y)
    }

    pub(crate) fn atspi_point_to_accesskit_point(
        &self,
        point: Point,
        parent: Option<NodeRef>,
        coord_type: CoordType,
    ) -> Point {
        let origin = self.origin(parent, coord_type);
        Point::new(point.x - origin.x, point.y - origin.y)
    }

    fn origin(&self, parent: Option<NodeRef>, coord_type: CoordType) -> Point {
        match coord_type {
            CoordType::Screen => self.inner.origin(),
            CoordType::Window => Point::ZERO,
            CoordType::Parent => {
                if let Some(parent) = parent {
                    let parent_origin = parent.bounding_box().unwrap_or_default().origin();
                    Point::new(-parent_origin.x, -parent_origin.y)
                } else {
                    self.inner.origin()
                }
            }
        }
    }
}

pub(crate) fn text_position_from_offset<'a>(
    node: &'a NodeRef,
    offset: i32,
) -> Option<TextPosition<'a>> {
    let index = offset.try_into().ok()?;
    node.text_position_from_global_usv_index(index)
}

pub(crate) fn text_range_from_offset<'a>(
    node: &'a NodeRef,
    offset: i32,
    granularity: Granularity,
) -> Result<TextRange<'a>, Error> {
    let start_offset = text_position_from_offset(node, offset).ok_or(Error::IndexOutOfRange)?;
    let start = match granularity {
        Granularity::Char => start_offset,
        Granularity::Line if start_offset.is_line_start() => start_offset,
        Granularity::Line => start_offset.backward_to_line_start(),
        Granularity::Paragraph if start_offset.is_paragraph_start() => start_offset,
        Granularity::Paragraph => start_offset.backward_to_paragraph_start(),
        Granularity::Sentence => return Err(Error::UnsupportedTextGranularity),
        Granularity::Word if start_offset.is_word_start() => start_offset,
        Granularity::Word => start_offset.backward_to_word_start(),
    };
    let end = match granularity {
        Granularity::Char if start_offset.is_document_end() => start_offset,
        Granularity::Char => start.forward_to_character_end(),
        Granularity::Line => start.forward_to_line_end(),
        Granularity::Paragraph => start.forward_to_paragraph_end(),
        Granularity::Sentence => return Err(Error::UnsupportedTextGranularity),
        Granularity::Word => start.forward_to_word_end(),
    };
    let mut range = start.to_degenerate_range();
    range.set_end(end);
    Ok(range)
}

pub(crate) fn sentence_at_offset(node: &NodeRef, offset: i32) -> Result<(String, i32, i32), Error> {
    let position = text_position_from_offset(node, offset).ok_or(Error::IndexOutOfRange)?;
    if position.is_document_end() {
        return Ok((String::new(), offset, offset));
    }
    let paragraph_start = if position.is_paragraph_start() {
        position
    } else {
        position.backward_to_paragraph_start()
    };
    let mut paragraph = paragraph_start.to_degenerate_range();
    paragraph.set_end(paragraph_start.forward_to_paragraph_end());
    let mut start = paragraph_start.to_global_usv_index();
    for sentence in paragraph.text().split_sentence_bounds() {
        let end = start + sentence.chars().count();
        if (start..end).contains(&(offset as usize)) {
            return Ok((
                sentence.to_owned(),
                start.try_into().map_err(|_| Error::TooManyCharacters)?,
                end.try_into().map_err(|_| Error::TooManyCharacters)?,
            ));
        }
        start = end;
    }
    Err(Error::IndexOutOfRange)
}

pub(crate) fn text_range_from_offsets<'a>(
    node: &'a NodeRef,
    start_offset: i32,
    end_offset: i32,
) -> Option<TextRange<'a>> {
    let start = text_position_from_offset(node, start_offset)?;
    let end = if end_offset == -1 {
        node.document_range().end()
    } else {
        text_position_from_offset(node, end_offset)?
    };

    let mut range = start.to_degenerate_range();
    range.set_end(end);
    Some(range)
}

pub(crate) fn text_range_bounds_from_offsets(
    node: &NodeRef,
    start_offset: i32,
    end_offset: i32,
) -> Option<Rect> {
    text_range_from_offsets(node, start_offset, end_offset)?
        .bounding_boxes()
        .into_iter()
        .reduce(|rect1, rect2| rect1.union(rect2))
}

pub(crate) fn atspi_scroll_type_to_scroll_hint(scroll_type: ScrollType) -> Option<ScrollHint> {
    match scroll_type {
        ScrollType::TopLeft => Some(ScrollHint::TopLeft),
        ScrollType::BottomRight => Some(ScrollHint::BottomRight),
        ScrollType::TopEdge => Some(ScrollHint::TopEdge),
        ScrollType::BottomEdge => Some(ScrollHint::BottomEdge),
        ScrollType::LeftEdge => Some(ScrollHint::LeftEdge),
        ScrollType::RightEdge => Some(ScrollHint::RightEdge),
        ScrollType::Anywhere => None,
    }
}

#[cfg(test)]
mod sentence_tests {
    use super::*;
    use accesskit::{Node, NodeId, Role, TreeId, TreeInfo, TreeUpdate};
    use accesskit_consumer::Tree;

    #[test]
    fn sentences_preserve_unicode_offsets_and_paragraph_boundaries() {
        let paragraphs = [
            "Opening prose\r\n",
            "Cafe\u{301}! Next 👍.\n",
            "日本語。終わり。",
        ];
        let mut document = Node::new(Role::Document);
        document.set_children([NodeId(2), NodeId(3), NodeId(4)]);
        let mut nodes = vec![(NodeId(1), document)];
        for (index, paragraph) in paragraphs.into_iter().enumerate() {
            let mut run = Node::new(Role::TextRun);
            run.set_value(paragraph);
            run.set_character_lengths(
                paragraph
                    .graphemes(true)
                    .map(|cluster| cluster.len() as u8)
                    .collect::<Vec<_>>(),
            );
            nodes.push((NodeId(index as u64 + 2), run));
        }
        let tree = Tree::new(
            TreeUpdate {
                nodes,
                tree: Some(TreeInfo::new(NodeId(1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(1),
            },
            true,
        );
        let document = tree.state().root();
        let mut start = 0;
        for sentence in [
            paragraphs[0],
            "Cafe\u{301}! ",
            "Next 👍.\n",
            "日本語。",
            "終わり。",
        ] {
            let end = start + sentence.chars().count() as i32;
            for offset in start..end {
                assert_eq!(
                    sentence_at_offset(&document, offset).unwrap(),
                    (sentence.to_owned(), start, end),
                    "offset {offset}",
                );
            }
            start = end;
        }
        assert_eq!(
            sentence_at_offset(&document, start).unwrap(),
            (String::new(), start, start),
        );
        assert!(sentence_at_offset(&document, -1).is_err());
        assert!(sentence_at_offset(&document, start + 1).is_err());
    }
}
