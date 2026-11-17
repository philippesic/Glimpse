use std::ops::Range;

use gpui::{App, Font, LineFragment, Pixels, Point, ShapedLine, Size, Window, point, px, size};
use ropey::Rope;
use smallvec::SmallVec;

use crate::input::RopeExt;

/// A line with soft wrapped lines info.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LineItem {
    length: usize,
    indent: Option<u32>,
    word_columns: u32,
    word_width: Pixels,
    /// The soft wrapped lines relative byte range (0..line.len) of this line (Include first line).
    ///
    /// Not contains the line end `\n`.
    pub(super) wrapped_lines: SmallVec<[Range<usize>; 1]>,
}

impl LineItem {
    /// Get the bytes length of this line.
    #[inline]
    pub(super) fn len(&self) -> usize {
        self.length
    }

    /// Get number of soft wrapped lines of this line (include the first line).
    #[inline]
    pub(super) fn lines_len(&self) -> usize {
        self.wrapped_lines.len()
    }

    /// Get the height of this line item with given line height.
    pub(super) fn height(&self, line_height: Pixels) -> Pixels {
        self.lines_len() as f32 * line_height
    }
}

#[derive(Debug, Default)]
pub(super) struct LongestRow {
    /// The 0-based row index.
    pub row: usize,
    /// The bytes length of the longest line.
    pub len: usize,
}

/// Used to prepare the text with soft wrap to be get lines to displayed in the Editor.
///
/// After use lines to calculate the scroll size of the Editor.
pub(super) struct TextWrapper {
    text: Rope,
    /// Total wrapped lines (Inlucde the first line), value is start and end index of the line.
    soft_lines: usize,
    font: Font,
    font_size: Pixels,
    /// If is none, it means the text is not wrapped
    wrap_width: Option<Pixels>,
    /// The longest (row, bytes len) in characters, used to calculate the horizontal scroll width.
    pub(super) longest_row: LongestRow,
    /// The lines by split \n
    pub(super) lines: Vec<LineItem>,

    _initialized: bool,
}

#[allow(unused)]
impl TextWrapper {
    pub(super) fn new(font: Font, font_size: Pixels, wrap_width: Option<Pixels>) -> Self {
        Self {
            text: Rope::new(),
            font,
            font_size,
            wrap_width,
            soft_lines: 0,
            longest_row: LongestRow::default(),
            lines: Vec::new(),
            _initialized: false,
        }
    }

    #[inline]
    pub(super) fn set_default_text(&mut self, text: &Rope) {
        self.text = text.clone();
    }

    /// Get the total number of lines including wrapped lines.
    #[inline]
    pub(super) fn len(&self) -> usize {
        self.soft_lines
    }

    /// Get the line item by row index.
    #[inline]
    pub(super) fn line(&self, row: usize) -> Option<&LineItem> {
        self.lines.iter().skip(row).next()
    }

    pub(super) fn set_wrap_width(&mut self, wrap_width: Option<Pixels>, cx: &mut App) {
        if wrap_width == self.wrap_width {
            return;
        }

        self.wrap_width = wrap_width;
        self.update_all(&self.text.clone(), cx);
    }

    pub(super) fn prepare(
        &mut self,
        text: &Rope,
        font: Font,
        font_size: Pixels,
        wrap_width: Option<Pixels>,
        cx: &mut App,
    ) {
        if self._initialized
            && self.font == font
            && self.font_size == font_size
            && self.wrap_width == wrap_width
        {
            return;
        }
        self._initialized = true;
        self.font = font;
        self.font_size = font_size;
        self.wrap_width = wrap_width;
        self.update_all(text, cx);
    }

    /// Update the text wrapper and recalculate the wrapped lines.
    ///
    /// If the `text` is the same as the current text, do nothing.
    ///
    /// - `changed_text`: The text [`Rope`] that has changed.
    /// - `range`: The `selected_range` before change.
    /// - `new_text`: The inserted text.
    /// - `force`: Whether to force the update, if false, the update will be skipped if the text is the same.
    /// - `cx`: The application context.
    pub(super) fn update(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        cx: &mut App,
    ) {
        let mut line_wrapper = cx
            .text_system()
            .line_wrapper(self.font.clone(), self.font_size);
        if self.edit_uniform_word(changed_text, range, new_text, &mut line_wrapper)
            || self.append(changed_text, range, new_text, &mut line_wrapper)
        {
            return;
        }
        self._update(
            changed_text,
            range,
            new_text,
            &mut |line_str, wrap_width| {
                line_wrapper
                    .wrap_line(&[LineFragment::text(line_str)], wrap_width)
                    .collect()
            },
        );
        if self.wrap_width.is_some() {
            let start = changed_text.offset_to_point(range.start).row;
            let end = changed_text
                .offset_to_point(range.start + new_text.len())
                .row;
            for row in start..=end {
                let Some(line) = self.lines.get_mut(row) else {
                    continue;
                };
                if line.length >= 1024 && line.wrapped_lines.len() > 1 {
                    if let Some(width) =
                        line_wrapper.uniform_word_width(&changed_text.slice_line(row).to_string())
                    {
                        line.word_width = width;
                        line.word_columns =
                            line.wrapped_lines.first().map_or(0, |range| range.len()) as u32;
                    }
                }
            }
        }
    }

    fn edit_uniform_word(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        wrapper: &mut gpui::LineWrapper,
    ) -> bool {
        if !self._initialized || self.wrap_width.is_none() {
            return false;
        }
        let row = self.text.offset_to_point(range.start).row;
        if self.text.offset_to_point(range.end).row != row
            || range.end > self.text.line_end_offset(row)
        {
            return false;
        }
        let Some(line) = self.lines.get_mut(row) else {
            return false;
        };
        if line.word_columns == 0
            || (new_text.len() != 0
                && wrapper.uniform_word_width(&new_text.to_string()) != Some(line.word_width))
        {
            return false;
        }
        let length = line.length - range.len() + new_text.len();
        let columns = line.word_columns as usize;
        let rows = length.div_ceil(columns).max(1);
        let old_rows = line.lines_len();
        line.wrapped_lines.resize(rows, 0..0);
        for (row, range) in line.wrapped_lines.iter_mut().enumerate() {
            *range = row * columns..((row + 1) * columns).min(length);
        }
        line.length = length;
        if length == 0 {
            line.indent = None;
        }
        if length < 1024 || rows < 2 {
            line.word_columns = 0;
            line.word_width = px(0.);
        }
        self.soft_lines = self.soft_lines - old_rows + rows;
        if self.longest_row.row == row || length >= self.longest_row.len {
            self.longest_row = self
                .lines
                .iter()
                .enumerate()
                .max_by_key(|(_, line)| line.length)
                .map(|(row, line)| LongestRow {
                    row,
                    len: line.length,
                })
                .unwrap_or_default();
        }
        self.text = changed_text.clone();
        true
    }

    fn append(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        wrapper: &mut gpui::LineWrapper,
    ) -> bool {
        let Some(width) = self.wrap_width else {
            return false;
        };
        if !self._initialized || range.start != range.end || new_text.lines_len() != 1 {
            return false;
        }
        let row = self.text.offset_to_point(range.start).row;
        let Some(line) = self.lines.get_mut(row) else {
            return false;
        };
        if line.wrapped_lines.len() < 2 || range.start != self.text.line_end_offset(row) {
            return false;
        }
        let Some(continuation_indent) = line.indent else {
            return false;
        };
        let prefix = line.wrapped_lines.len().saturating_sub(2);
        let start = line.wrapped_lines[prefix].start;
        let classify = line.length < 1024;
        let length = line.length + new_text.len();
        let origin = self.text.line_start_offset(row);
        let tail = changed_text
            .slice(origin + start..origin + length)
            .to_string();
        let fragments = [LineFragment::text(&tail)];
        let indent = (prefix > 0).then_some(continuation_indent);
        let old_rows = line.lines_len();
        line.wrapped_lines.truncate(prefix);
        let mut previous = start;
        for boundary in wrapper.wrap_line_with_indent(&fragments, width, indent) {
            let end = start + boundary.ix;
            if prefix == 0 {
                line.indent = Some(boundary.next_indent);
            }
            line.wrapped_lines.push(previous..end);
            previous = end;
        }
        if previous < length || previous == start {
            line.wrapped_lines.push(previous..length);
        }
        line.length = length;
        line.word_columns = 0;
        line.word_width = px(0.);
        if classify && length >= 1024 {
            if let Some(advance) =
                wrapper.uniform_word_width(&changed_text.slice(origin..origin + length).to_string())
            {
                line.word_width = advance;
                line.word_columns =
                    line.wrapped_lines.first().map_or(0, |range| range.len()) as u32;
            }
        }
        self.soft_lines = self.soft_lines - old_rows + line.lines_len();
        if length >= self.longest_row.len {
            self.longest_row = LongestRow { row, len: length };
        }
        self.text = changed_text.clone();
        true
    }

    fn _update<F>(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        wrap_line: &mut F,
    ) where
        F: FnMut(&str, Pixels) -> Vec<gpui::Boundary>,
    {
        // Remove the old changed lines.
        let start_row = self.text.offset_to_point(range.start).row;
        let start_row = start_row.min(self.lines.len().saturating_sub(1));
        let end_row = self.text.offset_to_point(range.end).row;
        let end_row = end_row.min(self.lines.len().saturating_sub(1));
        let rows_range = start_row..=end_row;

        if rows_range.contains(&self.longest_row.row) {
            self.longest_row = LongestRow::default();
        }

        let mut longest_row_ix = self.longest_row.row;
        let mut longest_row_len = self.longest_row.len;

        // To add the new lines.
        let new_start_row = changed_text.offset_to_point(range.start).row;
        let new_start_offset = changed_text.line_start_offset(new_start_row);
        let new_end_row = changed_text
            .offset_to_point(range.start + new_text.len())
            .row;
        let new_end_offset = changed_text.line_end_offset(new_end_row);
        let new_range = new_start_offset..new_end_offset;

        let mut new_lines = vec![];
        let wrap_width = self.wrap_width;

        // line not contains `\n`.
        for (ix, line) in Rope::from(changed_text.slice(new_range))
            .iter_lines()
            .enumerate()
        {
            let line_str = line.to_string();
            let mut wrapped_lines: SmallVec<[Range<usize>; 1]> = SmallVec::new();
            let mut prev_boundary_ix = 0;
            let mut indent = 0;

            if line_str.len() > longest_row_len {
                longest_row_ix = new_start_row + ix;
                longest_row_len = line_str.len();
            }

            // If wrap_width is Pixels::MAX, skip wrapping to disable word wrap
            if let Some(wrap_width) = wrap_width {
                // Here only have wrapped line, if there is no wrap meet, the `line_wraps` result will empty.
                for boundary in wrap_line(&line_str, wrap_width) {
                    indent = boundary.next_indent;
                    wrapped_lines.push(prev_boundary_ix..boundary.ix);
                    prev_boundary_ix = boundary.ix;
                }
            }

            // Reset of the line
            if !line_str[prev_boundary_ix..].is_empty() || prev_boundary_ix == 0 {
                wrapped_lines.push(prev_boundary_ix..line.len());
            }

            new_lines.push(LineItem {
                length: line.len(),
                indent: (line_str.bytes().take_while(|&byte| byte == b' ').count()
                    < wrapped_lines.first().map_or(0, |range| range.end))
                .then_some(indent),
                word_columns: 0,
                word_width: px(0.),
                wrapped_lines,
            });
        }

        if self.lines.len() == 0 {
            self.lines = new_lines;
        } else {
            self.lines.splice(rows_range, new_lines);
        }

        self.text = changed_text.clone();
        self.soft_lines = self.lines.iter().map(|l| l.lines_len()).sum();
        self.longest_row = LongestRow {
            row: longest_row_ix,
            len: longest_row_len,
        }
    }

    /// Update the text wrapper and recalculate the wrapped lines.
    ///
    /// If the `text` is the same as the current text, do nothing.
    fn update_all(&mut self, text: &Rope, cx: &mut App) {
        self.update(text, &(0..text.len()), &text, cx);
    }

    /// Return display point (with soft wrap) from the given byte offset in the text.
    ///
    /// Panics if the `offset` is out of bounds.
    pub(crate) fn offset_to_display_point(&self, offset: usize) -> DisplayPoint {
        let row = self.text.offset_to_point(offset).row;
        let start = self.text.line_start_offset(row);
        let line = &self.lines[row];

        let mut wrapped_row = self
            .lines
            .iter()
            .take(row)
            .map(|l| l.lines_len())
            .sum::<usize>();

        let local_offset = offset.saturating_sub(start);
        for (ix, range) in line.wrapped_lines.iter().enumerate() {
            if range.contains(&local_offset) {
                return DisplayPoint::new(
                    wrapped_row + ix,
                    ix,
                    local_offset.saturating_sub(range.start),
                );
            }
        }

        // Otherwise return the eof of the line.
        let last_range = line.wrapped_lines.last().unwrap_or(&(0..0));
        let ix = line.lines_len().saturating_sub(1);
        return DisplayPoint::new(wrapped_row + ix, ix, last_range.len());
    }

    /// Return byte offset in the text from the given display point (with soft wrap).
    ///
    /// Panics if the `point.row` is out of bounds.
    pub(crate) fn display_point_to_offset(&self, point: DisplayPoint) -> usize {
        let mut wrapped_row = 0;
        for (row, line) in self.lines.iter().enumerate() {
            if wrapped_row + line.lines_len() > point.row {
                let line_start = self.text.line_start_offset(row);
                let local_row = point.row.saturating_sub(wrapped_row);
                if let Some(range) = line.wrapped_lines.get(local_row) {
                    return line_start + (range.start + point.column).min(range.end);
                } else {
                    // If not found, return the end of the line.
                    return line_start + line.len();
                }
            }

            wrapped_row += line.lines_len();
        }

        return self.text.len();
    }

    pub(crate) fn display_point_to_point(&self, point: DisplayPoint) -> tree_sitter::Point {
        let offset = self.display_point_to_offset(point);
        self.text.offset_to_point(offset)
    }

    pub(crate) fn point_to_display_point(&self, point: tree_sitter::Point) -> DisplayPoint {
        let offset = self.text.point_to_offset(point);
        self.offset_to_display_point(offset)
    }
}

/// The actually display point in the text.
///
/// This is usually used to describe the
/// position in the text with `soft-wrap` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPoint {
    /// The 0-based soft wrapped row index in the text.
    pub row: usize,
    /// The 0-based row index in local line (include first line).
    ///
    /// This value only valid when return from [`TextWrapper::offset_to_display_point`], otherwise it will be ignored.
    pub local_row: usize,
    /// The 0-based column byte index in the display line (with soft wrap).
    pub column: usize,
}

impl DisplayPoint {
    pub fn new(row: usize, local_row: usize, column: usize) -> Self {
        Self {
            row,
            local_row,
            column,
        }
    }
}

pub(crate) struct WrappedLine {
    pub row: usize,
    pub range: Range<usize>,
    pub shaped: ShapedLine,
}

pub(crate) struct LineLayout {
    len: usize,
    rows: usize,
    pub(crate) wrapped_lines: SmallVec<[WrappedLine; 1]>,
    pub(crate) longest_width: Pixels,
}

impl LineLayout {
    pub(crate) fn new() -> Self {
        Self {
            len: 0,
            rows: 0,
            wrapped_lines: SmallVec::new(),
            longest_width: px(0.),
        }
    }

    pub(crate) fn lines(mut self, lines: SmallVec<[ShapedLine; 1]>) -> Self {
        self.set_wrapped_lines(lines);
        self
    }

    pub(crate) fn set_wrapped_lines(&mut self, lines: SmallVec<[ShapedLine; 1]>) {
        let mut offset = 0;
        let rows = lines.len();
        let lines = lines
            .into_iter()
            .enumerate()
            .map(|(row, shaped)| {
                let range = offset..offset + shaped.len();
                offset = range.end;
                WrappedLine { row, range, shaped }
            })
            .collect();
        self.set_visible_lines(offset, rows, lines);
    }

    pub(crate) fn set_visible_lines(
        &mut self,
        len: usize,
        rows: usize,
        lines: SmallVec<[WrappedLine; 1]>,
    ) {
        self.len = len;
        self.rows = rows;
        self.longest_width = lines
            .iter()
            .map(|line| line.shaped.width)
            .max()
            .unwrap_or_default();
        self.wrapped_lines = lines;
    }

    pub(crate) fn rows(&self) -> usize {
        self.rows
    }
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn position_for_index(
        &self,
        offset: usize,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        self.wrapped_lines
            .iter()
            .find(|line| {
                line.range.contains(&offset)
                    || (line.row + 1 == self.rows && offset == line.range.end)
            })
            .map(|line| {
                point(
                    line.shaped.x_for_index(offset - line.range.start),
                    line.row * line_height,
                )
            })
    }

    pub(super) fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.wrapped_lines
            .iter()
            .find(|line| x <= line.shaped.width)
            .map_or(self.len, |line| self.closest_in_line(line, x))
    }

    fn closest_in_line(&self, line: &WrappedLine, x: Pixels) -> usize {
        let mut index = line.shaped.closest_index_for_x(x);
        if line.row + 1 < self.rows && index == line.range.len() {
            index = index.saturating_sub(line.shaped.text.chars().last().map_or(0, char::len_utf8));
        }
        line.range.start + index
    }

    pub(super) fn closest_index_for_position(
        &self,
        pos: Point<Pixels>,
        line_height: Pixels,
    ) -> Option<usize> {
        self.wrapped_lines
            .iter()
            .find(|line| pos.y >= line.row * line_height && pos.y < (line.row + 1) * line_height)
            .map(|line| self.closest_in_line(line, pos.x))
    }

    pub(super) fn index_for_position(
        &self,
        pos: Point<Pixels>,
        line_height: Pixels,
    ) -> Option<usize> {
        let line = self
            .wrapped_lines
            .iter()
            .find(|line| pos.y >= line.row * line_height && pos.y < (line.row + 1) * line_height)?;
        Some(line.range.start + line.shaped.index_for_x(pos.x)?)
    }

    pub(super) fn size(&self, line_height: Pixels) -> Size<Pixels> {
        size(self.longest_width, self.rows * line_height)
    }

    pub(super) fn paint(
        &self,
        pos: Point<Pixels>,
        line_height: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        for line in &self.wrapped_lines {
            _ = line.shaped.paint(
                pos + point(px(0.), line.row * line_height),
                line_height,
                window,
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Boundary, FontFeatures, FontStyle, FontWeight, px};

    #[test]
    fn sparse_layout_retains_offscreen_geometry_and_byte_offsets() {
        let mut layout = LineLayout::new();
        let mut shaped = ShapedLine::default().with_len(6);
        shaped.text = "a🌱b".into();
        layout.set_visible_lines(
            1206,
            201,
            smallvec::smallvec![
                WrappedLine {
                    row: 100,
                    range: 600..606,
                    shaped: shaped.clone()
                },
                WrappedLine {
                    row: 200,
                    range: 1200..1206,
                    shaped
                },
            ],
        );
        assert_eq!(layout.len(), 1206);
        assert_eq!(layout.size(px(20.)).height, px(4020.));
        assert_eq!(
            layout.position_for_index(602, px(20.)).unwrap().y,
            px(2000.)
        );
        assert_eq!(
            layout.position_for_index(1206, px(20.)).unwrap().y,
            px(4000.)
        );
        assert_eq!(layout.position_for_index(606, px(20.)), None);
        assert_eq!(
            layout.closest_index_for_position(point(px(0.), px(100.)), px(20.)),
            None
        );
        assert_eq!(
            layout.closest_index_for_position(point(px(100.), px(2000.)), px(20.)),
            Some(605)
        );
    }

    #[gpui::test]
    fn middle_edits_and_deletions_match_complete_wrapping(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for initial in [
                "x".repeat(4000),
                "a1_b-".repeat(800),
                "long words here ".repeat(200),
                "中🌱é café ".repeat(200),
            ] {
                let original = Rope::from(initial.as_str());
                for anchor in [0, original.len() / 2, original.len()] {
                    let start = original.clip_offset(anchor, sum_tree::Bias::Left);
                    for removed in [0, 5, original.len()] {
                        let end = original.clip_offset(
                            start.saturating_add(removed).min(original.len()),
                            sum_tree::Bias::Right,
                        );
                        for inserted in ["X", "", "ASCII", "中文", " ", "\n"] {
                            let font = gpui::font(".ZedMono");
                            let mut incremental =
                                TextWrapper::new(font.clone(), px(16.), Some(px(120.)));
                            incremental.prepare(
                                &original,
                                font.clone(),
                                px(16.),
                                Some(px(120.)),
                                cx,
                            );
                            if initial.starts_with('x') {
                                assert!(incremental.lines[0].word_columns > 0);
                            }
                            let mut edited = original.clone();
                            edited.replace(start..end, inserted);
                            incremental.update(&edited, &(start..end), &Rope::from(inserted), cx);
                            let mut complete =
                                TextWrapper::new(font.clone(), px(16.), Some(px(120.)));
                            complete.prepare(&edited, font, px(16.), Some(px(120.)), cx);
                            assert_eq!(
                                incremental.lines, complete.lines,
                                "{anchor}, {removed}, {inserted:?}"
                            );
                            assert_eq!(incremental.soft_lines, complete.soft_lines);
                        }
                    }
                }
            }
        });
    }

    #[gpui::test]
    fn repeated_unicode_middle_edits_match_full_layout(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for width in [75., 120., 311.] {
                for initial in [
                    "α🙂 beta ".repeat(500),
                    format!("    {}", "中🌱é café words ".repeat(400)),
                    format!("first\n{}\nlast", "abc אבג 👩‍👩‍👧‍👦 ".repeat(300)),
                    "中日本語🙂".repeat(500),
                ] {
                    let font = gpui::font(".ZedMono");
                    let mut text = Rope::from(initial.as_str());
                    let mut incremental = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                    incremental.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                    for step in 0usize..60 {
                        let anchor = text.len() * (step % 7 + 1) / 9;
                        let start = text.clip_offset(anchor, sum_tree::Bias::Left);
                        let end = text.clip_offset(
                            (start + step % 19).min(text.len()),
                            sum_tree::Bias::Right,
                        );
                        let inserted = ["x", "", " 中文 ", "é🙂", "  ", "\nnew line\n"][step % 6];
                        text.replace(start..end, inserted);
                        incremental.update(&text, &(start..end), &Rope::from(inserted), cx);
                        let mut complete = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                        complete.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                        assert_eq!(
                            incremental.lines, complete.lines,
                            "width {width}, step {step}, {start}..{end}, {inserted:?}"
                        );
                        assert_eq!(incremental.soft_lines, complete.soft_lines);
                    }
                }
            }
        });
    }

    #[gpui::test]
    fn long_line_repeated_edits_preserve_every_wrap_boundary(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for width in [75., 120., 311.] {
                for initial in [
                    "α🙂 beta ".repeat(800),
                    "中日本語🙂".repeat(600),
                    " a.b -xx   é 👩‍👩‍👧‍👦 אבג ".repeat(400),
                ] {
                    let font = gpui::font(".ZedMono");
                    let mut text = Rope::from(initial.as_str());
                    let mut incremental = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                    incremental.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                    let mut random = 17u64;
                    for step in 0..300 {
                        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                        let start = text
                            .clip_offset(random as usize % (text.len() + 1), sum_tree::Bias::Left);
                        let end = text.clip_offset(
                            (start + (random >> 32) as usize % 27).min(text.len()),
                            sum_tree::Bias::Right,
                        );
                        let inserted = [
                            "x",
                            "",
                            " 中文 ",
                            "é🙂",
                            "  ",
                            "long_word.with-punctuation",
                            "👩‍👩‍👧‍👦",
                        ][step % 7];
                        text.replace(start..end, inserted);
                        incremental.update(&text, &(start..end), &Rope::from(inserted), cx);
                        let mut complete = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                        complete.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                        assert_eq!(
                            incremental.lines, complete.lines,
                            "width {width}, step {step}, {start}..{end}, {inserted:?}"
                        );
                        assert_eq!(incremental.soft_lines, complete.soft_lines);
                    }
                }
            }
        });
    }

    #[gpui::test]
    fn incremental_appends_match_complete_wrapping(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for initial in [
                "alpha beta gamma delta ".repeat(80),
                format!("     {}", "alpha beta gamma delta ".repeat(80)),
                format!("{}a", " ".repeat(400)),
                " ".repeat(400),
                "x".repeat(4000),
                "x".repeat(1020),
                "中🌱é a café ".repeat(80),
                format!("prefix\n{}\nsuffix", "long words here ".repeat(80)),
            ] {
                let mut text = Rope::from(initial.as_str());
                let row = usize::from(initial.starts_with("prefix\n"));
                let font = gpui::font(".ZedMono");
                let mut incremental = TextWrapper::new(font.clone(), px(16.), Some(px(120.)));
                incremental.prepare(&text, font.clone(), px(16.), Some(px(120.)), cx);
                for appended in [
                    "word",
                    " ",
                    "中文",
                    "🌱",
                    " é ",
                    "longer words",
                    "\nnew line",
                    "x",
                ] {
                    let offset = text.line_end_offset(row);
                    text.insert(offset, appended);
                    incremental.update(&text, &(offset..offset), &Rope::from(appended), cx);
                    let mut complete = TextWrapper::new(font.clone(), px(16.), Some(px(120.)));
                    complete.prepare(&text, font.clone(), px(16.), Some(px(120.)), cx);
                    assert_eq!(
                        incremental.lines, complete.lines,
                        "{initial:?}, {appended:?}"
                    );
                    assert_eq!(incremental.soft_lines, complete.soft_lines);
                }
            }
        });
    }

    #[test]
    fn test_update() {
        let font = gpui::Font {
            family: "Arial".into(),
            weight: FontWeight::default(),
            style: FontStyle::Normal,
            features: FontFeatures::default(),
            fallbacks: None,
        };

        let mut wrapper = TextWrapper::new(font, px(14.), None);
        let mut text = Rope::from(
            "Hello, 世界!\r\nThis is second line.\nThis is third line.\n这里是第 4 行。",
        );

        fn fake_wrap_line(_line: &str, _wrap_width: Pixels) -> Vec<Boundary> {
            vec![]
        }

        #[track_caller]
        fn assert_wrapper_lines(text: &Rope, wrapper: &TextWrapper, expected_lines: &[&[&str]]) {
            let mut actual_lines = vec![];
            let mut offset = 0;
            for line in wrapper.lines.iter() {
                actual_lines.push(
                    line.wrapped_lines
                        .iter()
                        .map(|range| text.slice(offset + range.start..offset + range.end))
                        .collect::<Vec<_>>(),
                );
                // +1 \n
                offset += line.len() + 1;
            }
            assert_eq!(actual_lines, expected_lines);
        }

        wrapper._update(&text, &(0..text.len()), &text, &mut fake_wrap_line);
        assert_eq!(wrapper.lines.len(), 4);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["Hello, 世界!\r"],
                &["This is second line."],
                &["This is third line."],
                &["这里是第 4 行。"],
            ],
        );

        // Add a new text to end
        let range = text.len()..text.len();
        let new_text = "New text";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "Hello, 世界!\r\nThis is second line.\nThis is third line.\n这里是第 4 行。New text"
        );
        assert_eq!(wrapper.lines.len(), 4);
        assert_eq!(wrapper.lines.len(), 4);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["Hello, 世界!\r"],
                &["This is second line."],
                &["This is third line."],
                &["这里是第 4 行。New text"],
            ],
        );

        // Replace first line `Hello` to `AAA`
        let range = 0..5;
        let new_text = "AAA";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "AAA, 世界!\r\nThis is second line.\nThis is third line.\n这里是第 4 行。New text"
        );
        assert_eq!(wrapper.lines.len(), 4);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["AAA, 世界!\r"],
                &["This is second line."],
                &["This is third line."],
                &["这里是第 4 行。New text"],
            ],
        );

        // Remove the second line
        let start_offset = text.line_start_offset(1);
        let end_offset = text.line_end_offset(1);
        let range = start_offset..end_offset + 1;
        text.replace(range.clone(), "");
        wrapper._update(&text, &range, &Rope::from(""), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "AAA, 世界!\r\nThis is third line.\n这里是第 4 行。New text"
        );
        assert_eq!(wrapper.lines.len(), 3);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["AAA, 世界!\r"],
                &["This is third line."],
                &["这里是第 4 行。New text"],
            ],
        );

        // Replace the first 2 lines to "This is a new line."
        let range = text.line_start_offset(0)..text.line_end_offset(1) + 1;
        let new_text = "This is a new line.\nThis is new line 2.\n";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "This is a new line.\nThis is new line 2.\n这里是第 4 行。New text"
        );
        assert_eq!(wrapper.lines.len(), 3);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["This is a new line."],
                &["This is new line 2."],
                &["这里是第 4 行。New text"],
            ],
        );

        // Add a new line at the end
        let range = text.len()..text.len();
        let new_text = "\nThis is a new line at the end.";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "This is a new line.\nThis is new line 2.\n这里是第 4 行。New text\nThis is a new line at the end."
        );
        assert_eq!(wrapper.lines.len(), 4);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["This is a new line."],
                &["This is new line 2."],
                &["这里是第 4 行。New text"],
                &["This is a new line at the end."],
            ],
        );

        // Add a new line at the beginning
        let range = 0..0;
        let new_text = "This is a new line at the beginning.\n";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "This is a new line at the beginning.\nThis is a new line.\nThis is new line 2.\n这里是第 4 行。New text\nThis is a new line at the end."
        );
        assert_eq!(wrapper.lines.len(), 5);
        assert_wrapper_lines(
            &text,
            &wrapper,
            &[
                &["This is a new line at the beginning."],
                &["This is a new line."],
                &["This is new line 2."],
                &["这里是第 4 行。New text"],
                &["This is a new line at the end."],
            ],
        );

        // Remove all to at least one line in `lines`.
        let range = 0..text.len();
        let new_text = "";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &Rope::from(new_text), &mut fake_wrap_line);
        assert_eq!(text.to_string(), "");
        assert_eq!(wrapper.lines.len(), 1);
        assert_eq!(wrapper.lines[0].wrapped_lines.as_slice(), &[0..0]);

        // Test update_all
        let range = 0..text.len();
        let new_text = "This is a full text.\nThis is a second line.";
        text.replace(range.clone(), new_text);
        wrapper._update(&text, &range, &text, &mut fake_wrap_line);
        assert_eq!(
            text.to_string(),
            "This is a full text.\nThis is a second line."
        );
        assert_eq!(wrapper.lines.len(), 2);
    }

    #[test]
    fn test_line_layout() {
        let mut line_layout = LineLayout::new();

        let line1 = ShapedLine::default().with_len(100);
        let line2 = ShapedLine::default().with_len(50);
        let wrapped_lines = smallvec::smallvec![line1, line2];
        line_layout.set_wrapped_lines(wrapped_lines);
        assert_eq!(line_layout.len(), 150);
        assert_eq!(line_layout.wrapped_lines.len(), 2);
    }

    #[test]
    fn test_offset_to_display_point() {
        let font = gpui::Font {
            family: "Arial".into(),
            weight: FontWeight::default(),
            style: FontStyle::Normal,
            features: FontFeatures::default(),
            fallbacks: None,
        };

        let mut wrapper = TextWrapper::new(font, px(14.), None);
        wrapper.text = Rope::from(
            "Hello, 世界!\r\nThis is second line.\nThis is third line.\n这里是第 4 行。",
        );
        wrapper.lines = vec![
            // range: 0..15
            LineItem {
                length: "Hello, 世界!\r".len(),
                wrapped_lines: smallvec::smallvec![0..15],
                indent: Some(0),
                word_columns: 0,
                word_width: px(0.),
            },
            // range: 16..36
            LineItem {
                length: "This is second line.".len(),
                wrapped_lines: smallvec::smallvec![0..10, 10..20],
                indent: Some(0),
                word_columns: 0,
                word_width: px(0.),
            },
            // range: 37..56
            LineItem {
                length: "This is third line.".len(),
                wrapped_lines: smallvec::smallvec![0..9, 9..15, 15..20],
                indent: Some(0),
                word_columns: 0,
                word_width: px(0.),
            },
            // range: 57..79
            LineItem {
                length: "这里是第 4 行。".len(),
                wrapped_lines: smallvec::smallvec![0..22],
                indent: Some(0),
                word_columns: 0,
                word_width: px(0.),
            },
        ];

        assert_eq!(
            wrapper.offset_to_display_point(12),
            DisplayPoint::new(0, 0, 12)
        );
        assert_eq!(
            wrapper.offset_to_display_point(15),
            DisplayPoint::new(0, 0, 15)
        );

        assert_eq!(
            wrapper.offset_to_display_point(16),
            DisplayPoint::new(1, 0, 0)
        );
        assert_eq!(
            wrapper.offset_to_display_point(21),
            DisplayPoint::new(1, 0, 5)
        );
        assert_eq!(
            wrapper.offset_to_display_point(27),
            DisplayPoint::new(2, 1, 1)
        );
        assert_eq!(
            wrapper.offset_to_display_point(37),
            DisplayPoint::new(3, 0, 0)
        );
        assert_eq!(
            wrapper.offset_to_display_point(54),
            DisplayPoint::new(5, 2, 2)
        );
        assert_eq!(
            wrapper.offset_to_display_point(59),
            DisplayPoint::new(6, 0, 2)
        );

        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(6, 0, 2)),
            59
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(5, 2, 2)),
            54
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(3, 0, 0)),
            37
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(2, 1, 1)),
            27
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(1, 0, 5)),
            21
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(1, 0, 0)),
            16
        );
        assert_eq!(
            wrapper.display_point_to_offset(DisplayPoint::new(0, 0, 15)),
            15
        );
    }
}
