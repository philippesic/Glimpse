use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, BackgroundExecutor, Font, LineFragment, Pixels, Point, ShapedLine, Size, TextSystem,
    Window, point, px, size,
};
use ropey::{LineType, Rope, RopeSlice};
use smallvec::SmallVec;

use crate::input::RopeExt;

/// A line with soft wrapped lines info.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LineItem {
    length: usize,
    wraps: Option<Box<LineWraps>>,
}

#[derive(Debug, Clone, PartialEq)]
struct LineWraps {
    indent: Option<u32>,
    word_columns: u32,
    word_width: Pixels,
    ranges: Vec<Range<usize>>,
}

impl LineItem {
    fn new(length: usize, indent: Option<u32>, ranges: SmallVec<[Range<usize>; 1]>) -> Self {
        let wraps = (ranges.len() > 1).then(|| {
            Box::new(LineWraps {
                indent,
                word_columns: 0,
                word_width: px(0.),
                ranges: ranges.into_vec(),
            })
        });
        Self { length, wraps }
    }

    #[cfg(test)]
    fn ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.wraps
            .is_none()
            .then_some(0..self.length)
            .into_iter()
            .chain(
                self.wraps
                    .iter()
                    .flat_map(|wraps| wraps.ranges.iter().cloned()),
            )
    }

    pub(super) fn range(&self, row: usize) -> Option<Range<usize>> {
        match &self.wraps {
            Some(wraps) => wraps.ranges.get(row).cloned(),
            None => (row == 0).then_some(0..self.length),
        }
    }

    pub(super) fn row_at(&self, byte: usize) -> usize {
        self.wraps.as_ref().map_or(0, |wraps| {
            wraps
                .ranges
                .partition_point(|range| range.end <= byte)
                .min(wraps.ranges.len() - 1)
        })
    }

    /// Get number of soft wrapped lines of this line (include the first line).
    #[inline]
    pub(super) fn lines_len(&self) -> usize {
        self.wraps.as_ref().map_or(1, |wraps| wraps.ranges.len())
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
    block_starts: Vec<usize>,

    checkpoints: std::collections::BTreeMap<usize, Vec<WrapCheckpoint>>,
    _initialized: bool,
}

#[derive(Clone, PartialEq)]
pub(super) struct LayoutConfig {
    pub(super) font: Font,
    pub(super) font_size: Pixels,
    pub(super) wrap_width: Option<Pixels>,
}

#[derive(Clone)]
struct WrapCheckpoint {
    state: gpui::LineWrapState,
    rows: usize,
}

#[allow(unused)]
impl TextWrapper {
    const LINES_PER_BLOCK: usize = 128;

    pub(super) fn new(font: Font, font_size: Pixels, wrap_width: Option<Pixels>) -> Self {
        Self {
            text: Rope::new(),
            font,
            font_size,
            wrap_width,
            soft_lines: 0,
            longest_row: LongestRow::default(),
            lines: Vec::new(),
            block_starts: Vec::new(),
            checkpoints: std::collections::BTreeMap::new(),
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
        self.lines.get(row)
    }

    pub(super) fn soft_row_start(&self, row: usize) -> usize {
        if row >= self.lines.len() {
            return self.soft_lines;
        }
        let block = row / Self::LINES_PER_BLOCK;
        let start = block * Self::LINES_PER_BLOCK;
        self.block_starts[block]
            + self.lines[start..row]
                .iter()
                .map(LineItem::lines_len)
                .sum::<usize>()
    }

    pub(super) fn hard_row_at(&self, soft_row: usize) -> Option<(usize, usize)> {
        if soft_row >= self.soft_lines {
            return None;
        }
        let block = self.block_starts.partition_point(|&row| row <= soft_row) - 1;
        let first = block * Self::LINES_PER_BLOCK;
        let mut start = self.block_starts[block];
        self.lines[first..]
            .iter()
            .take(Self::LINES_PER_BLOCK)
            .enumerate()
            .find_map(|(index, line)| {
                let end = start + line.lines_len();
                let found = (soft_row < end).then_some((first + index, start));
                start = end;
                found
            })
    }

    fn rebuild_index_from(&mut self, row: usize) {
        let block = row / Self::LINES_PER_BLOCK;
        let start = (block * Self::LINES_PER_BLOCK).min(self.lines.len());
        let mut top = self
            .block_starts
            .get(block)
            .copied()
            .unwrap_or(self.soft_lines);
        self.block_starts.truncate(block);
        for lines in self.lines[start..].chunks(Self::LINES_PER_BLOCK) {
            self.block_starts.push(top);
            top += lines.iter().map(LineItem::lines_len).sum::<usize>();
        }
        self.soft_lines = top;
    }

    fn update_soft_rows(&mut self, row: usize, before: usize, after: usize) {
        if before == after {
            return;
        }
        let next_block = row / Self::LINES_PER_BLOCK + 1;
        for start in &mut self.block_starts[next_block..] {
            *start = *start - before + after;
        }
        self.soft_lines = self.soft_lines - before + after;
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

    pub(super) fn matches_config(&self, config: &LayoutConfig) -> bool {
        self._initialized
            && self.font == config.font
            && self.font_size == config.font_size
            && self.wrap_width == config.wrap_width
    }

    pub(super) fn prepare_initial(
        config: LayoutConfig,
        text: &Rope,
        text_system: &Arc<TextSystem>,
    ) -> Self {
        let mut layout = Self::new(config.font, config.font_size, config.wrap_width);
        layout._initialized = true;
        layout.prepare_replacement(text, text_system);
        layout
    }

    pub(super) async fn prepare_initial_on_worker(
        config: LayoutConfig,
        text: Rope,
        text_system: Arc<TextSystem>,
        executor: BackgroundExecutor,
    ) -> Self {
        let rows = text.lines_len();
        let workers = executor.num_cpus().min(4);
        if workers < 2
            || text.len() < 2 * 1024 * 1024
            || rows > 8 * 1024 * 1024 / std::mem::size_of::<LineItem>()
        {
            return Self::prepare_initial(config, &text, &text_system);
        }

        let mut boundaries = vec![0];
        for part in 1..workers {
            let row = text.byte_to_line_idx(text.len() / workers * part, LineType::LF);
            if boundaries.last() != Some(&row) {
                boundaries.push(row);
            }
        }
        if boundaries.len() == 1 {
            return Self::prepare_initial(config, &text, &text_system);
        }
        boundaries.push(rows);

        let tasks: Vec<_> = boundaries
            .windows(2)
            .map(|range| {
                let (start, end) = (range[0], range[1]);
                let slice = text.slice(text.line_start_offset(start)..text.line_start_offset(end));
                let part = ropey::extra::esoterica::disconnect_slice(slice)
                    .unwrap_or_else(|| Rope::from(slice));
                let config = config.clone();
                let text_system = text_system.clone();
                executor.spawn(async move {
                    let mut layout = Self::prepare_initial(config, &part, &text_system);
                    if end < rows {
                        layout.lines.pop();
                    }
                    (start, layout)
                })
            })
            .collect();

        let mut layout = Self::new(config.font, config.font_size, config.wrap_width);
        layout._initialized = true;
        layout.text = text;
        layout.lines.reserve_exact(rows);
        for task in tasks {
            let (start, mut part) = task.await;
            if part.longest_row.len > layout.longest_row.len {
                layout.longest_row = LongestRow {
                    row: start + part.longest_row.row,
                    len: part.longest_row.len,
                };
            }
            layout.lines.append(&mut part.lines);
            layout.checkpoints.extend(
                part.checkpoints
                    .into_iter()
                    .map(|(row, points)| (row + start, points)),
            );
        }
        layout.rebuild_index_from(0);
        layout
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
        self.update_with_text_system(changed_text, range, new_text, cx.text_system());
    }

    pub(super) fn replacement_layout(&self) -> Self {
        let mut result = Self::new(self.font.clone(), self.font_size, self.wrap_width);
        result._initialized = self._initialized;
        result
    }

    pub(super) fn uses_same_layout(&self, prepared: &Self) -> bool {
        self.font == prepared.font
            && self.font_size == prepared.font_size
            && self.wrap_width == prepared.wrap_width
            && self._initialized == prepared._initialized
    }

    pub(super) fn matches_revision(&self, text: &Rope, layout: &Self) -> bool {
        self.uses_same_layout(layout) && self.text == *text
    }

    pub(super) fn retain_for_history(self) -> Option<Self> {
        const MAX_LAYOUT_BYTES: usize = 4 * 1024 * 1024;
        let lines = self.lines.capacity() * std::mem::size_of::<LineItem>()
            + self.block_starts.capacity() * std::mem::size_of::<usize>();
        let wraps: usize = self
            .lines
            .iter()
            .filter_map(|line| line.wraps.as_ref())
            .map(|wraps| {
                std::mem::size_of::<LineWraps>()
                    + wraps.ranges.capacity() * std::mem::size_of::<Range<usize>>()
            })
            .sum();
        let checkpoints: usize = self
            .checkpoints
            .values()
            .map(|points| {
                points.capacity() * std::mem::size_of::<WrapCheckpoint>()
                    + 4 * std::mem::size_of::<(usize, Vec<WrapCheckpoint>)>()
            })
            .sum();
        (self._initialized && lines + wraps + checkpoints <= MAX_LAYOUT_BYTES).then_some(self)
    }

    pub(super) fn prepare_replacement(&mut self, text: &Rope, text_system: &Arc<TextSystem>) {
        self.set_default_text(text);
        self.update_with_text_system(text, &(0..text.len()), text, text_system);
    }

    fn update_with_text_system(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        text_system: &Arc<TextSystem>,
    ) {
        let mut line_wrapper = text_system.line_wrapper(self.font.clone(), self.font_size);
        if self.edit_uniform_word(changed_text, range, new_text, &mut line_wrapper)
            || self.resume_edit(changed_text, range, new_text, &mut line_wrapper)
            || self.append(changed_text, range, new_text, &mut line_wrapper)
        {
            return;
        }
        self._update(changed_text, range, new_text, &mut |line, wrap_width| {
            let mut boundaries = Vec::new();
            let mut checkpoints = Vec::new();
            let mut next_checkpoint = 1024;
            let fragments: SmallVec<[LineFragment<'_>; 4]> =
                line.chunks().map(LineFragment::text).collect();
            for (boundary, state) in
                line_wrapper.resume_line(&fragments, wrap_width, gpui::LineWrapState::default())
            {
                boundaries.push(boundary);
                if state.consumed_bytes() >= next_checkpoint {
                    checkpoints.push(WrapCheckpoint {
                        state,
                        rows: boundaries.len(),
                    });
                    next_checkpoint = state.consumed_bytes() + 1024;
                }
            }
            (boundaries, checkpoints)
        });
        if self.wrap_width.is_some() {
            let start = changed_text.offset_to_point(range.start).row;
            let end = changed_text
                .offset_to_point(range.start + new_text.len())
                .row;
            for row in start..=end {
                let Some(line) = self.lines.get_mut(row) else {
                    continue;
                };
                if line.length >= 1024
                    && let Some(wraps) = line.wraps.as_mut()
                    && let Some(width) = line_wrapper
                        .uniform_word_width_for_bytes(changed_text.slice_line(row).bytes())
                {
                    wraps.word_width = width;
                    wraps.word_columns = wraps.ranges[0].len() as u32;
                }
            }
        }
    }

    fn resume_edit(
        &mut self,
        changed_text: &Rope,
        range: &Range<usize>,
        new_text: &Rope,
        wrapper: &mut gpui::LineWrapper,
    ) -> bool {
        let Some(width) = self.wrap_width else {
            return false;
        };
        if !self._initialized || new_text.lines_len() != 1 {
            return false;
        }
        let row = self.text.offset_to_point(range.start).row;
        if self.text.offset_to_point(range.end).row != row
            || range.end > self.text.line_end_offset(row)
        {
            return false;
        }
        let Some(checkpoints) = self
            .checkpoints
            .get(&row)
            .filter(|points| !points.is_empty())
        else {
            return false;
        };
        let origin = self.text.line_start_offset(row);
        let local = range.start - origin..range.end - origin;
        let before =
            checkpoints.partition_point(|point| point.state.consumed_bytes() <= local.start);
        let start = before.checked_sub(1).map(|index| &checkpoints[index]);
        let state = start.map_or_else(gpui::LineWrapState::default, |point| point.state);
        let rows = start.map_or(0, |point| point.rows);
        let line = &self.lines[row];
        let Some(wraps) = &line.wraps else {
            return false;
        };
        let length = line.length - range.len() + new_text.len();
        let tail = changed_text.slice(origin + state.consumed_bytes()..origin + length);
        let fragments: Vec<_> = tail.chunks().map(LineFragment::text).collect();
        let mut wrapped = Vec::new();
        let mut updated = checkpoints[..before].to_vec();
        let mut previous = state.boundary();
        let mut next_checkpoint = updated
            .last()
            .map_or(1024, |point| point.state.consumed_bytes() + 1024);
        let mut old_index =
            checkpoints.partition_point(|point| point.state.consumed_bytes() < local.end);
        let mut reused = false;
        let mut indent = start.map_or(0, |point| point.state.continuation_indent());
        for (boundary, state) in wrapper.resume_line(&fragments, width, state) {
            indent = boundary.next_indent;
            wrapped.push(previous..boundary.ix);
            previous = boundary.ix;
            if state.consumed_bytes() >= local.start + new_text.len() {
                while old_index < checkpoints.len() {
                    let point = &checkpoints[old_index];
                    let shifted = point.state.after_edit(local.clone(), new_text.len());
                    let Some(shifted) = shifted else {
                        old_index += 1;
                        continue;
                    };
                    if shifted.consumed_bytes() < state.consumed_bytes() {
                        old_index += 1;
                        continue;
                    }
                    if shifted.same_continuation(state) && point.state.boundary() >= local.end {
                        let shift = |offset: usize| offset - range.len() + new_text.len();
                        let row_delta = (rows + wrapped.len()) as isize - point.rows as isize;
                        wrapped.extend(
                            wraps.ranges[point.rows..]
                                .iter()
                                .map(|range| shift(range.start)..shift(range.end)),
                        );
                        for point in &checkpoints[old_index..] {
                            if let Some(state) =
                                point.state.after_edit(local.clone(), new_text.len())
                            {
                                updated.push(WrapCheckpoint {
                                    state,
                                    rows: point.rows.checked_add_signed(row_delta).unwrap(),
                                });
                            }
                        }
                        reused = true;
                    }
                    break;
                }
            }
            if reused {
                break;
            }
            if state.consumed_bytes() >= next_checkpoint {
                updated.push(WrapCheckpoint {
                    state,
                    rows: rows + wrapped.len(),
                });
                next_checkpoint = state.consumed_bytes() + 1024;
            }
        }
        if !reused && (previous < length || (rows == 0 && wrapped.is_empty())) {
            wrapped.push(previous..length);
        }
        self.update_soft_rows(row, self.lines[row].lines_len(), rows + wrapped.len());
        let line = &mut self.lines[row];
        line.length = length;
        if rows + wrapped.len() <= 1 {
            line.wraps = None;
        } else if let Some(wraps) = line.wraps.as_mut() {
            wraps.ranges.truncate(rows);
            wraps.ranges.extend(wrapped);
            wraps.word_columns = 0;
            wraps.word_width = px(0.);
            let text = changed_text.slice(origin..origin + length);
            wraps.indent = (text.bytes().take_while(|&byte| byte == b' ').count()
                < wraps.ranges[0].end)
                .then_some(indent);
            if length >= 1024
                && changed_text.byte(origin).is_ascii()
                && let Some(advance) = wrapper.uniform_word_width_for_bytes(text.bytes())
            {
                wraps.word_width = advance;
                wraps.word_columns = wraps.ranges[0].len() as u32;
            }
        }
        let mut next_checkpoint = 1024;
        updated.retain(|point| {
            if point.state.consumed_bytes() < next_checkpoint {
                return false;
            }
            next_checkpoint = point.state.consumed_bytes() + 1024;
            true
        });
        if updated.is_empty() {
            self.checkpoints.remove(&row);
        } else {
            self.checkpoints.insert(row, updated);
        }
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
        self.text = changed_text.clone();
        true
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
        let Some(wraps) = line.wraps.as_mut() else {
            return false;
        };
        if wraps.word_columns == 0
            || (new_text.len() != 0
                && wrapper.uniform_word_width_for_bytes(new_text.bytes()) != Some(wraps.word_width))
        {
            return false;
        }
        let length = line.length - range.len() + new_text.len();
        let columns = wraps.word_columns as usize;
        let rows = length.div_ceil(columns).max(1);
        let old_rows = wraps.ranges.len();
        self.checkpoints.remove(&row);
        line.length = length;
        if rows == 1 {
            line.wraps = None;
        } else {
            wraps.ranges.resize(rows, 0..0);
            for (row, range) in wraps.ranges.iter_mut().enumerate() {
                *range = row * columns..((row + 1) * columns).min(length);
            }
            if length < 1024 {
                wraps.word_columns = 0;
                wraps.word_width = px(0.);
            }
        }
        self.update_soft_rows(row, old_rows, rows);
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
        if range.start != self.text.line_end_offset(row) {
            return false;
        }
        let Some(wraps) = line.wraps.as_mut() else {
            return false;
        };
        let Some(continuation_indent) = wraps.indent else {
            return false;
        };
        self.checkpoints.remove(&row);
        let old_rows = wraps.ranges.len();
        let prefix = old_rows - 2;
        let start = wraps.ranges[prefix].start;
        let classify = line.length < 1024;
        let length = line.length + new_text.len();
        let origin = self.text.line_start_offset(row);
        let tail = changed_text
            .slice(origin + start..origin + length)
            .to_string();
        let fragments = [LineFragment::text(&tail)];
        let indent = (prefix > 0).then_some(continuation_indent);
        wraps.ranges.truncate(prefix);
        let mut previous = start;
        for boundary in wrapper.wrap_line_with_indent(&fragments, width, indent) {
            let end = start + boundary.ix;
            if prefix == 0 {
                wraps.indent = Some(boundary.next_indent);
            }
            wraps.ranges.push(previous..end);
            previous = end;
        }
        if previous < length || previous == start {
            wraps.ranges.push(previous..length);
        }
        line.length = length;
        wraps.word_columns = 0;
        wraps.word_width = px(0.);
        if wraps.ranges.len() <= 1 {
            line.wraps = None;
        } else if classify
            && length >= 1024
            && let Some(advance) = wrapper
                .uniform_word_width_for_bytes(changed_text.slice(origin..origin + length).bytes())
        {
            wraps.word_width = advance;
            wraps.word_columns = wraps.ranges[0].len() as u32;
        }
        let rows = line.lines_len();
        self.update_soft_rows(row, old_rows, rows);
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
        F: FnMut(RopeSlice<'_>, Pixels) -> (Vec<gpui::Boundary>, Vec<WrapCheckpoint>),
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

        let mut new_lines = Vec::with_capacity(new_end_row - new_start_row + 1);
        let mut new_checkpoints = std::collections::BTreeMap::new();
        let wrap_width = self.wrap_width;

        // line not contains `\n`.
        for (ix, line) in changed_text
            .slice(new_range)
            .lines(LineType::LF)
            .enumerate()
        {
            let line = if line.len() != 0 && line.byte(line.len() - 1) == b'\n' {
                line.slice(..line.len() - 1)
            } else {
                line
            };
            let mut wrapped_lines: SmallVec<[Range<usize>; 1]> = SmallVec::new();
            let mut prev_boundary_ix = 0;
            let mut indent = 0;

            if line.len() > longest_row_len {
                longest_row_ix = new_start_row + ix;
                longest_row_len = line.len();
            }

            // If wrap_width is Pixels::MAX, skip wrapping to disable word wrap
            if let Some(wrap_width) = wrap_width {
                // Here only have wrapped line, if there is no wrap meet, the `line_wraps` result will empty.
                let (boundaries, checkpoints) = wrap_line(line, wrap_width);
                if !checkpoints.is_empty() {
                    new_checkpoints.insert(new_start_row + ix, checkpoints);
                }
                for boundary in boundaries {
                    indent = boundary.next_indent;
                    wrapped_lines.push(prev_boundary_ix..boundary.ix);
                    prev_boundary_ix = boundary.ix;
                }
            }

            // Reset of the line
            if prev_boundary_ix < line.len() || prev_boundary_ix == 0 {
                wrapped_lines.push(prev_boundary_ix..line.len());
            }

            let indent = if wrapped_lines.len() > 1 {
                (line.bytes().take_while(|&byte| byte == b' ').count()
                    < wrapped_lines.first().map_or(0, |range| range.end))
                .then_some(indent)
            } else {
                None
            };
            new_lines.push(LineItem::new(line.len(), indent, wrapped_lines));
        }

        if self.lines.is_empty() {
            self.lines = new_lines;
            self.checkpoints = new_checkpoints;
            self.rebuild_index_from(0);
        } else {
            let one_row_changed = rows_range.start() == rows_range.end() && new_lines.len() == 1;
            let previous_rows = self.lines[start_row].lines_len();
            self.checkpoints.retain(|row, _| !rows_range.contains(row));
            let suffix = self.checkpoints.split_off(&(end_row + 1));
            let delta = new_lines.len() as isize - rows_range.clone().count() as isize;
            self.checkpoints.extend(
                suffix
                    .into_iter()
                    .map(|(row, points)| (row.checked_add_signed(delta).unwrap(), points)),
            );
            self.checkpoints.extend(new_checkpoints);
            self.lines.splice(rows_range, new_lines);
            if one_row_changed {
                self.update_soft_rows(start_row, previous_rows, self.lines[start_row].lines_len());
            } else {
                self.rebuild_index_from(start_row);
            }
        }

        self.text = changed_text.clone();
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

        let wrapped_row = self.soft_row_start(row);

        let local_offset = offset.saturating_sub(start);
        let ix = line.row_at(local_offset);
        let range = line.range(ix).unwrap();
        DisplayPoint::new(
            wrapped_row + ix,
            ix,
            local_offset.min(range.end) - range.start,
        )
    }

    /// Return byte offset in the text from the given display point (with soft wrap).
    ///
    /// Panics if the `point.row` is out of bounds.
    pub(crate) fn display_point_to_offset(&self, point: DisplayPoint) -> usize {
        let Some((row, start)) = self.hard_row_at(point.row) else {
            return self.text.len();
        };
        let range = self.lines[row].range(point.row - start).unwrap();
        self.text.line_start_offset(row) + (range.start + point.column).min(range.end)
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
    fn single_row_uses_the_full_line_without_wrap_storage() {
        for length in [0, "日本語\r".len(), usize::MAX] {
            let line = LineItem::new(length, None, smallvec::smallvec![0..length]);
            assert!(line.wraps.is_none());
            assert_eq!(line.lines_len(), 1);
            assert_eq!(line.row_at(length), 0);
            assert_eq!(line.range(0), Some(0..length));
            assert_eq!(line.range(1), None);
        }
        let line = LineItem::new(12, Some(0), smallvec::smallvec![0..5, 5..12]);
        assert_eq!(line.row_at(4), 0);
        assert_eq!(line.row_at(5), 1);
        assert_eq!(line.row_at(12), 1);
        assert_eq!(line.range(1), Some(5..12));
    }

    #[test]
    fn row_index_tracks_wraps_and_line_edits_across_blocks() {
        let font = Font {
            family: "monospace".into(),
            features: FontFeatures::default(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            fallbacks: None,
        };
        let mut wrapper = TextWrapper::new(font, px(14.), Some(px(64.)));
        let mut text = Rope::new();
        let mut wrap = |line: RopeSlice<'_>, _| {
            let boundaries = (8..line.len())
                .step_by(8)
                .map(|ix| Boundary { ix, next_indent: 0 })
                .collect();
            (boundaries, Vec::new())
        };
        for (row, count, replacement) in [
            (0, 0, "A short line\n".repeat(400)),
            (127, 0, "A word that increases the wrap count ".into()),
            (128, 0, "new first row\nnew second row\n".into()),
            (256, 3, String::new()),
            (0, 399, "Final".into()),
        ] {
            let start = text.line_start_offset(row);
            let end = text.line_start_offset(row + count);
            let range = start..end;
            let inserted = Rope::from(replacement.as_str());
            text.replace(range.clone(), &replacement);
            wrapper._update(&text, &range, &inserted, &mut wrap);
            let mut expected = 0;
            for (row, line) in wrapper.lines.iter().enumerate() {
                assert_eq!(wrapper.soft_row_start(row), expected);
                for local in 0..line.lines_len() {
                    assert_eq!(wrapper.hard_row_at(expected + local), Some((row, expected)));
                }
                expected += line.lines_len();
            }
            assert_eq!(wrapper.len(), expected);
            assert_eq!(wrapper.hard_row_at(expected), None);
        }
    }

    #[gpui::test]
    fn partitioned_initial_layout_matches_full_layout(cx: &mut gpui::TestAppContext) {
        let (system, executor) =
            cx.update(|cx| (cx.text_system().clone(), cx.background_executor().clone()));
        let mut text = String::new();
        for index in 0..22_000 {
            use std::fmt::Write;
            let length = if index % 1_013 == 0 {
                410
            } else {
                12 + index % 4
            };
            write!(text, "{index:05} 日本語 é {}\r\n", "a café ".repeat(length)).unwrap();
            if index % 13 == 0 {
                text.push('\n');
            }
        }
        text.push_str("Unterminated final line");
        let text = Rope::from(text.as_str());
        let config = LayoutConfig {
            font: gpui::font(".ZedMono"),
            font_size: px(16.),
            wrap_width: Some(px(319.)),
        };
        let exact = TextWrapper::prepare_initial(config.clone(), &text, &system);
        let actual = executor.block_test(TextWrapper::prepare_initial_on_worker(
            config,
            text,
            system,
            executor.clone(),
        ));
        assert_eq!(actual.lines, exact.lines);
        assert_eq!(actual.len(), exact.len());
        assert_eq!(actual.longest_row.row, exact.longest_row.row);
        assert_eq!(actual.longest_row.len, exact.longest_row.len);
        let checkpoint_states = |layout: &TextWrapper| {
            layout
                .checkpoints
                .iter()
                .flat_map(|(row, points)| {
                    points.iter().map(|point| (*row, point.rows, point.state))
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(checkpoint_states(&actual), checkpoint_states(&exact));
    }

    #[gpui::test]
    fn wrapping_rope_chunks_matches_contiguous_text(cx: &mut gpui::TestAppContext) {
        let cx = cx.add_empty_window();
        cx.update(|_, cx| {
            let text = "    a café 日本語 👩‍👩‍👧‍👦 é\t words and an unbroken-word ".repeat(2_000);
            let rope = Rope::from(text.as_str());
            let chunks: Vec<_> = rope.chunks().map(LineFragment::text).collect();
            assert!(chunks.len() > 20);
            let mut wrapper = cx
                .text_system()
                .line_wrapper(gpui::font(".ZedMono"), px(16.));
            for width in [px(75.), px(311.), px(880.)] {
                let contiguous = wrapper
                    .resume_line(
                        &[LineFragment::text(&text)],
                        width,
                        gpui::LineWrapState::default(),
                    )
                    .collect::<Vec<_>>();
                let streamed = wrapper
                    .resume_line(&chunks, width, gpui::LineWrapState::default())
                    .collect::<Vec<_>>();
                assert_eq!(streamed, contiguous);
            }
        });
    }

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
                                assert!(
                                    incremental.lines[0].wraps.as_ref().unwrap().word_columns > 0
                                );
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
    fn original_gpui_wrapping_contract(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            let mut wrapper = cx
                .text_system()
                .line_wrapper(gpui::font(".ZedMono"), px(16.));

            assert_eq!(
                wrapper
                    .wrap_line(&[LineFragment::text("aa bbb cccc ddddd eeee")], px(72.))
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(7, 0), (12, 0), (18, 0)],
            );
            assert_eq!(
                wrapper
                    .wrap_line(&[LineFragment::text("aaa aaaaaaaaaaaaaaaaaa")], px(72.0))
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(4, 0), (11, 0), (18, 0)],
            );
            assert_eq!(
                wrapper
                    .wrap_line(&[LineFragment::text("     aaaaaaa")], px(72.))
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(7, 5), (9, 5), (11, 5),]
            );
            assert_eq!(
                wrapper
                    .wrap_line(
                        &[LineFragment::text("                            ")],
                        px(72.)
                    )
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(7, 0), (14, 0), (21, 0)]
            );
            assert_eq!(
                wrapper
                    .wrap_line(&[LineFragment::text("          aaaaaaaaaaaaaa")], px(72.))
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(7, 0), (14, 3), (18, 3), (22, 3),]
            );

            assert_eq!(
                wrapper
                    .wrap_line(
                        &[
                            LineFragment::text("aa bbb "),
                            LineFragment::text("cccc ddddd eeee")
                        ],
                        px(72.)
                    )
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(7, 0), (12, 0), (18, 0)],
            );

            assert_eq!(
                wrapper
                    .wrap_line(
                        &[
                            LineFragment::text("aa "),
                            LineFragment::element(px(20.), 1),
                            LineFragment::text(" bbb "),
                            LineFragment::element(px(30.), 1),
                            LineFragment::text(" cccc")
                        ],
                        px(72.)
                    )
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(5, 0), (9, 0), (11, 0)],
            );

            assert_eq!(
                wrapper
                    .wrap_line(
                        &[
                            LineFragment::element(px(50.), 1),
                            LineFragment::text(" aaaa bbbb cccc dddd")
                        ],
                        px(72.)
                    )
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(2, 0), (7, 0), (12, 0), (17, 0)],
            );

            assert_eq!(
                wrapper
                    .wrap_line(
                        &[
                            LineFragment::text("short text "),
                            LineFragment::element(px(100.), 1),
                            LineFragment::text(" more text")
                        ],
                        px(72.)
                    )
                    .map(|boundary| (boundary.ix, boundary.next_indent))
                    .collect::<Vec<_>>(),
                &[(6, 0), (11, 0), (12, 0), (18, 0)],
            );
        });
    }

    #[gpui::test]
    fn exact_states_resume_across_consumed_lookahead(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for family in [".ZedMono", "Arial"] {
                for width in [25., 75., 120., 311.] {
                    let font = gpui::font(family);
                    let mut wrapper = cx.text_system().line_wrapper(font, px(16.));
                    for text in [
                        "a b longer_word.with-punctuation 中文🙂é ".repeat(80),
                        format!("{}firstword {}", " ".repeat(400), "abcdef ".repeat(200)),
                        "אבג 👩‍👩‍👧‍👦 日本語\t café\r ".repeat(80),
                    ] {
                        let fragments = [LineFragment::text(&text)];
                        let complete: Vec<_> = wrapper
                            .resume_line(&fragments, px(width), gpui::LineWrapState::default())
                            .collect();
                        for (index, (_, state)) in complete.iter().enumerate() {
                            let tail = &text[state.consumed_bytes()..];
                            let fragments: Vec<_> = tail
                                .char_indices()
                                .map(|(byte, character)| {
                                    LineFragment::text(&tail[byte..byte + character.len_utf8()])
                                })
                                .collect();
                            let resumed: Vec<_> =
                                wrapper.resume_line(&fragments, px(width), *state).collect();
                            assert_eq!(resumed, complete[index + 1..], "{family} {width} {index}");
                        }
                    }
                }
            }
        });
    }

    #[gpui::test]
    fn checkpoint_edits_preserve_all_rows_and_positions(cx: &mut gpui::TestAppContext) {
        use gpui::AppContext;
        let _entity = cx.new(|cx: &mut gpui::Context<()>| {
            for family in [".ZedMono", "Arial"] {
                for width in [75., 311.] {
                    let font = gpui::font(family);
                    let mut text = Rope::from(
                        format!(
                            "{}{}",
                            " ".repeat(350),
                            "a café 日本語 👩‍👩‍👧‍👦 é\t words ".repeat(500)
                        )
                        .as_str(),
                    );
                    let mut incremental = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                    incremental.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                    let mut random = 7331u64;
                    for step in 0..200 {
                        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                        let start = text
                            .clip_offset(random as usize % (text.len() + 1), sum_tree::Bias::Left);
                        let end = text.clip_offset(
                            (start + (random >> 32) as usize % 103).min(text.len()),
                            sum_tree::Bias::Right,
                        );
                        let inserted =
                            ["🙂x", "", " ", "é", "日本語 ", "long_word.with-punctuation"]
                                [step % 6];
                        text.replace(start..end, inserted);
                        let mut wrapper = cx.text_system().line_wrapper(font.clone(), px(16.));
                        assert!(incremental.resume_edit(
                            &text,
                            &(start..end),
                            &Rope::from(inserted),
                            &mut wrapper
                        ));
                        let mut complete = TextWrapper::new(font.clone(), px(16.), Some(px(width)));
                        complete.prepare(&text, font.clone(), px(16.), Some(px(width)), cx);
                        assert_eq!(incremental.lines, complete.lines, "{family} {width} {step}");
                        assert_eq!(incremental.len(), complete.len());
                        for range in incremental.lines[0].ranges() {
                            for offset in [range.start, range.end] {
                                assert_eq!(
                                    incremental.offset_to_display_point(offset),
                                    complete.offset_to_display_point(offset)
                                );
                            }
                        }
                        let points = &incremental.checkpoints[&0];
                        assert!(points.len() <= text.len() / 1024 + 2);
                        assert!(points.windows(2).all(|pair| pair[0].state.consumed_bytes()
                            < pair[1].state.consumed_bytes()
                            && pair[0].rows < pair[1].rows));
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

        fn fake_wrap_line(
            _line: RopeSlice<'_>,
            _wrap_width: Pixels,
        ) -> (Vec<Boundary>, Vec<WrapCheckpoint>) {
            (vec![], vec![])
        }

        #[track_caller]
        fn assert_wrapper_lines(text: &Rope, wrapper: &TextWrapper, expected_lines: &[&[&str]]) {
            let mut actual_lines = vec![];
            let mut offset = 0;
            for line in wrapper.lines.iter() {
                actual_lines.push(
                    line.ranges()
                        .map(|range| text.slice(offset + range.start..offset + range.end))
                        .collect::<Vec<_>>(),
                );
                // +1 \n
                offset += line.length + 1;
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
        assert_eq!(wrapper.lines[0].ranges().collect::<Vec<_>>(), [0..0]);

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
            LineItem::new("Hello, 世界!\r".len(), Some(0), smallvec::smallvec![0..15]),
            // range: 16..36
            LineItem::new(
                "This is second line.".len(),
                Some(0),
                smallvec::smallvec![0..10, 10..20],
            ),
            // range: 37..56
            LineItem::new(
                "This is third line.".len(),
                Some(0),
                smallvec::smallvec![0..9, 9..15, 15..20],
            ),
            // range: 57..79
            LineItem::new("这里是第 4 行。".len(), Some(0), smallvec::smallvec![0..22]),
        ];
        wrapper.rebuild_index_from(0);

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
