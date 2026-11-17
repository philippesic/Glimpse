use ropey::Rope;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use super::RopeExt;

pub(super) fn floor(text: &Rope, offset: usize) -> usize {
    let offset = text.floor_char_boundary(offset.min(text.len()));
    if offset == text.len() {
        return offset;
    }
    boundary(
        text,
        offset + text.char_at(offset).unwrap().len_utf8(),
        true,
    )
}

pub(super) fn boundary(text: &Rope, offset: usize, previous: bool) -> usize {
    let offset = offset.min(text.len());
    if text.len() == 0 || (previous && offset == 0) {
        return 0;
    }
    if !previous && offset == text.len() {
        return text.len();
    }
    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
    let mut index = if previous { offset - 1 } else { offset };
    loop {
        let (mut chunks, start) = text.chunks_at(index);
        let chunk = chunks.next().unwrap();
        let result = if previous {
            cursor.prev_boundary(chunk, start)
        } else {
            cursor.next_boundary(chunk, start)
        };
        match result {
            Ok(boundary) => return boundary.unwrap_or(if previous { 0 } else { text.len() }),
            Err(GraphemeIncomplete::PrevChunk) => index = start - 1,
            Err(GraphemeIncomplete::NextChunk) => index = start + chunk.len(),
            Err(GraphemeIncomplete::PreContext(end)) => {
                let (mut context, start) = text.chunks_at(end - 1);
                let context = context.next().unwrap();
                cursor.provide_context(&context[..end - start], start);
            }
            Err(GraphemeIncomplete::InvalidOffset) => unreachable!("Invalid rope grapheme offset"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn boundaries_match_contiguous_unicode_across_rope_chunks() {
        for text in [
            String::new(),
            "start e\u{301}👩‍👩‍👧‍👦🙂\r\nnext\rline".into(),
            format!(
                "{}{}tail",
                "x".repeat(1000),
                "🇦🇺🇫🇷e\u{301}👩‍👩‍👧‍👦\r\n".repeat(100)
            ),
            format!("{}a{}b", "x".repeat(1000), "\u{301}".repeat(4000)),
        ] {
            let rope = Rope::from(text.as_str());
            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain([text.len()])
                .collect();
            for offset in text
                .char_indices()
                .map(|(offset, _)| offset)
                .chain([text.len()])
            {
                for previous in [false, true] {
                    let mut cursor = GraphemeCursor::new(offset, text.len(), true);
                    let expected = if previous {
                        cursor.prev_boundary(&text, 0)
                    } else {
                        cursor.next_boundary(&text, 0)
                    }
                    .unwrap()
                    .unwrap_or(if previous { 0 } else { text.len() });
                    assert_eq!(
                        boundary(&rope, offset, previous),
                        expected,
                        "offset {offset}, previous {previous}"
                    );
                }
            }
            for offset in 0..=text.len() {
                let expected = *boundaries
                    .iter()
                    .take_while(|&&boundary| boundary <= offset)
                    .last()
                    .unwrap();
                assert_eq!(floor(&rope, offset), expected, "floor offset {offset}");
            }
        }
    }
}
