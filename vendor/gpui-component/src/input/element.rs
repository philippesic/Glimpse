use std::{ops::Range, rc::Rc};

use gpui::{
    App, Bounds, Corners, Element, ElementId, ElementInputHandler, Entity, GlobalElementId, Half,
    HighlightStyle, Hitbox, Hsla, IntoElement, LayoutId, MouseButton, MouseMoveEvent, Path, Pixels,
    Point, ShapedLine, SharedString, Size, Style, TextRun, TextStyle, UnderlineStyle, Window, fill,
    point, px, relative, size,
};
use ropey::Rope;
use smallvec::SmallVec;

use crate::{
    ActiveTheme as _, Colorize, PixelsExt, Root,
    input::{
        RopeExt as _,
        blink_cursor::CURSOR_WIDTH,
        text_wrapper::{LayoutConfig, LineLayout, WrappedLine},
    },
};

use super::{InputEvent, InputState, LastLayout, mode::InputMode, search::MatchRanges};

const BOTTOM_MARGIN_ROWS: usize = 3;
pub(super) const RIGHT_MARGIN: Pixels = px(10.);
pub(super) const LINE_NUMBER_RIGHT_MARGIN: Pixels = px(10.);

pub(super) struct TextElement {
    pub(crate) state: Entity<InputState>,
    placeholder: SharedString,
}

impl TextElement {
    pub(super) fn new(state: Entity<InputState>) -> Self {
        Self {
            state,
            placeholder: SharedString::default(),
        }
    }

    /// Set the placeholder text of the input field.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    fn paint_mouse_listeners(&mut self, window: &mut Window, _: &mut App) {
        window.on_mouse_event({
            let state = self.state.clone();

            move |event: &MouseMoveEvent, _, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    state.update(cx, |state, cx| {
                        state.on_drag_move(event, window, cx);
                    });
                }
            }
        });
    }

    /// Returns the:
    ///
    /// - cursor bounds
    /// - scroll offset
    /// - current row index (No only the visible lines, but all lines)
    ///
    /// This method also will update for track scroll to cursor.
    fn layout_cursor(
        &self,
        last_layout: &LastLayout,
        bounds: &mut Bounds<Pixels>,
        _: &mut Window,
        cx: &mut App,
    ) -> (Option<Bounds<Pixels>>, Point<Pixels>, Option<usize>) {
        let state = self.state.read(cx);

        let line_height = last_layout.line_height;
        let visible_range = &last_layout.visible_range;
        let lines = &last_layout.lines;
        let text_wrapper = &state.text_wrapper;
        let line_number_width = last_layout.line_number_width;

        let mut selected_range = state.selected_range;

        if let Some(ime_marked_range) = &state.ime_marked_range {
            selected_range = (ime_marked_range.end..ime_marked_range.end).into();
        }
        let is_selected_all = selected_range.len() == state.text.len();

        let mut scroll_offset = state.scroll_handle.offset();

        // If the input has a fixed height (Otherwise is auto-grow), we need to add a bottom margin to the input.
        let top_bottom_margin = if state.mode.is_auto_grow() {
            line_height
        } else if visible_range.len() < BOTTOM_MARGIN_ROWS * 8 {
            line_height
        } else {
            BOTTOM_MARGIN_ROWS * line_height
        };

        let position = |offset| {
            let location = state.text.offset_to_point(offset);
            let row = location.row;
            let wrapped = text_wrapper.line(row)?;
            let column = if state.masked {
                state
                    .text
                    .slice(state.text.line_start_offset(row)..offset)
                    .chars()
                    .count()
            } else {
                location.column
            };
            let top = text_wrapper.soft_row_start(row) * line_height;
            let local = row
                .checked_sub(visible_range.start)
                .and_then(|index| lines.get(index))
                .and_then(|line| line.position_for_index(column, line_height))
                .unwrap_or_else(|| point(px(0.), wrapped.row_at(column) * line_height));
            Some((row, point(local.x, top + local.y)))
        };

        let mut current_row = None;
        let mut cursor_bounds = None;
        if let (Some((row, cursor_pos)), Some((_, cursor_start)), Some((_, cursor_end))) = (
            position(state.cursor()),
            position(selected_range.start),
            position(selected_range.end),
        ) {
            current_row = Some(row);
            let selection_changed = state.last_selected_range != Some(selected_range);
            if selection_changed && !is_selected_all {
                scroll_offset.x = if scroll_offset.x + cursor_pos.x
                    > (bounds.size.width - line_number_width - RIGHT_MARGIN)
                {
                    // cursor is out of right
                    bounds.size.width - line_number_width - RIGHT_MARGIN - cursor_pos.x
                } else if scroll_offset.x + cursor_pos.x < px(0.) {
                    // cursor is out of left
                    scroll_offset.x - cursor_pos.x
                } else {
                    scroll_offset.x
                };

                scroll_offset.y =
                    if scroll_offset.y + cursor_pos.y > bounds.size.height - top_bottom_margin {
                        bounds.size.height - top_bottom_margin - line_height - cursor_pos.y
                    } else if scroll_offset.y + cursor_pos.y < top_bottom_margin {
                        (top_bottom_margin - cursor_pos.y).min(px(0.))
                    } else {
                        scroll_offset.y
                    };

                if state.selection_reversed {
                    if scroll_offset.x + cursor_start.x < px(0.) {
                        // selection start is out of left
                        scroll_offset.x = -cursor_start.x;
                    }
                    if scroll_offset.y + cursor_start.y < px(0.) {
                        // selection start is out of top
                        scroll_offset.y = -cursor_start.y;
                    }
                } else {
                    if scroll_offset.x + cursor_end.x <= px(0.) {
                        // selection end is out of left
                        scroll_offset.x = -cursor_end.x;
                    }
                    if scroll_offset.y + cursor_end.y <= px(0.) {
                        // selection end is out of top
                        scroll_offset.y = -cursor_end.y;
                    }
                }
            }

            // cursor bounds
            let cursor_height = match state.size {
                crate::Size::Large => 1.,
                crate::Size::Small => 0.75,
                _ => 0.85,
            } * line_height;

            cursor_bounds = Some(Bounds::new(
                point(
                    bounds.left() + cursor_pos.x + line_number_width + scroll_offset.x,
                    bounds.top() + cursor_pos.y + ((line_height - cursor_height) / 2.),
                ),
                size(CURSOR_WIDTH, cursor_height),
            ));
        }

        if let Some(deferred_scroll_offset) = state.deferred_scroll_offset {
            scroll_offset = deferred_scroll_offset;
        }

        bounds.origin = bounds.origin + scroll_offset;

        (cursor_bounds, scroll_offset, current_row)
    }

    /// Layout the match range to a Path.
    pub(crate) fn layout_match_range(
        range: Range<usize>,
        last_layout: &LastLayout,
        bounds: &Bounds<Pixels>,
    ) -> Option<Path<Pixels>> {
        if range.is_empty() {
            return None;
        }

        if range.start < last_layout.visible_range_offset.start
            || range.end > last_layout.visible_range_offset.end
        {
            return None;
        }

        let line_height = last_layout.line_height;
        let visible_top = last_layout.visible_top;
        let visible_start_offset = last_layout.visible_range_offset.start;
        let lines = &last_layout.lines;
        let line_number_width = last_layout.line_number_width;

        let start_ix = range.start;
        let end_ix = range.end;

        let mut prev_lines_offset = visible_start_offset;
        let mut offset_y = visible_top;
        let mut line_corners = vec![];

        for line in lines.iter() {
            let line_size = line.size(line_height);
            let line_wrap_width = line_size.width;

            let line_origin = point(px(0.), offset_y);

            let line_cursor_start =
                line.position_for_index(start_ix.saturating_sub(prev_lines_offset), line_height);
            let line_cursor_end =
                line.position_for_index(end_ix.saturating_sub(prev_lines_offset), line_height);

            if line_cursor_start.is_some() || line_cursor_end.is_some() {
                let start = line_cursor_start
                    .unwrap_or_else(|| line.position_for_index(0, line_height).unwrap());

                let end = line_cursor_end
                    .unwrap_or_else(|| line.position_for_index(line.len(), line_height).unwrap());

                // Split the selection into multiple items
                let wrapped_lines =
                    (end.y / line_height).ceil() as usize - (start.y / line_height).ceil() as usize;

                let mut end_x = end.x;
                if wrapped_lines > 0 {
                    end_x = line_wrap_width;
                }

                // Ensure at least 6px width for the selection for empty lines.
                end_x = end_x.max(start.x + px(6.));

                line_corners.push(Corners {
                    top_left: line_origin + point(start.x, start.y),
                    top_right: line_origin + point(end_x, start.y),
                    bottom_left: line_origin + point(start.x, start.y + line_height),
                    bottom_right: line_origin + point(end_x, start.y + line_height),
                });

                // wrapped lines
                for i in 1..=wrapped_lines {
                    let start = point(px(0.), start.y + i as f32 * line_height);
                    let mut end = point(end.x, end.y + i as f32 * line_height);
                    if i < wrapped_lines {
                        end.x = line_size.width;
                    }

                    line_corners.push(Corners {
                        top_left: line_origin + point(start.x, start.y),
                        top_right: line_origin + point(end.x, start.y),
                        bottom_left: line_origin + point(start.x, start.y + line_height),
                        bottom_right: line_origin + point(end.x, start.y + line_height),
                    });
                }
            }

            if line_cursor_start.is_some() && line_cursor_end.is_some() {
                break;
            }

            offset_y += line_size.height;
            // +1 for skip the last `\n`
            prev_lines_offset += line.len() + 1;
        }

        let mut points = vec![];
        if line_corners.is_empty() {
            return None;
        }

        // Fix corners to make sure the left to right direction
        for corners in &mut line_corners {
            if corners.top_left.x > corners.top_right.x {
                std::mem::swap(&mut corners.top_left, &mut corners.top_right);
                std::mem::swap(&mut corners.bottom_left, &mut corners.bottom_right);
            }
        }

        for corners in &line_corners {
            points.push(corners.top_right);
            points.push(corners.bottom_right);
            points.push(corners.bottom_left);
        }

        let mut rev_line_corners = line_corners.iter().rev().peekable();
        while let Some(corners) = rev_line_corners.next() {
            points.push(corners.top_left);
            if let Some(next) = rev_line_corners.peek() {
                if next.top_left.x > corners.top_left.x {
                    points.push(point(next.top_left.x, corners.top_left.y));
                }
            }
        }

        // print_points_as_svg_path(&line_corners, &points);

        let path_origin = bounds.origin + point(line_number_width, px(0.));
        let first_p = *points.get(0).unwrap();
        let mut builder = gpui::PathBuilder::fill();
        builder.move_to(path_origin + first_p);
        for p in points.iter().skip(1) {
            builder.line_to(path_origin + *p);
        }

        builder.build().ok()
    }

    fn layout_search_matches(
        &self,
        last_layout: &LastLayout,
        bounds: &Bounds<Pixels>,
        cx: &mut App,
    ) -> Vec<(Bounds<Pixels>, bool)> {
        let search_panel = self.state.read(cx).search_panel.clone();
        let Some((ranges, current_match_ix)) = search_panel.and_then(|panel| {
            if let Some(matcher) = panel.read(cx).matcher() {
                Some((matcher.matched_ranges.clone(), matcher.current_match_ix))
            } else {
                None
            }
        }) else {
            return Vec::new();
        };

        let mut offset = last_layout.visible_range_offset.start;
        let mut visible = Vec::new();
        for line in last_layout.lines.iter() {
            visible.extend(
                line.wrapped_lines
                    .iter()
                    .map(|row| offset + row.range.start..offset + row.range.end + 1),
            );
            offset += line.len() + 1;
        }
        let mut matches = Vec::new();
        for index in matching_indices(&ranges, &visible) {
            let range = ranges.get(index).unwrap();
            let current = current_match_ix == index;
            Self::layout_search_range(&range, last_layout, bounds, |rect| {
                matches.push((rect, current));
            });
        }

        matches
    }

    fn layout_search_range(
        range: &Range<usize>,
        layout: &LastLayout,
        bounds: &Bounds<Pixels>,
        mut push: impl FnMut(Bounds<Pixels>),
    ) {
        let mut offset = layout.visible_range_offset.start;
        let mut top = layout.visible_top;
        for line in layout.lines.iter() {
            for row in &line.wrapped_lines {
                let start = offset + row.range.start;
                let end = offset + row.range.end;
                let from = range.start.max(start).min(end);
                let to = range.end.min(end);
                let newline = row.row + 1 == line.rows() && range.contains(&end);
                if from >= to && !newline {
                    continue;
                }
                let x = row.shaped.x_for_index(from - start);
                let right = row.shaped.x_for_index(to.max(from) - start).max(x + px(6.));
                let origin = bounds.origin
                    + point(layout.line_number_width, top + row.row * layout.line_height);
                push(Bounds::new(
                    origin + point(x, px(0.)),
                    size(right - x, layout.line_height),
                ));
            }
            offset += line.len() + 1;
            top += line.size(layout.line_height).height;
        }
    }

    fn layout_hover_highlight(
        &self,
        last_layout: &LastLayout,
        bounds: &Bounds<Pixels>,
        cx: &mut App,
    ) -> Option<Path<Pixels>> {
        let hover_popover = self.state.read(cx).hover_popover.clone();
        let Some(symbol_range) = hover_popover.map(|popover| popover.read(cx).symbol_range.clone())
        else {
            return None;
        };

        Self::layout_match_range(symbol_range, last_layout, bounds)
    }

    fn layout_document_colors(
        &self,
        document_colors: &[(Range<usize>, Hsla)],
        last_layout: &LastLayout,
        bounds: &Bounds<Pixels>,
    ) -> Vec<(Path<Pixels>, Hsla)> {
        let mut paths = vec![];
        for (range, color) in document_colors.iter() {
            if let Some(path) = Self::layout_match_range(range.clone(), last_layout, bounds) {
                paths.push((path, *color));
            }
        }

        paths
    }

    fn layout_selections(
        &self,
        last_layout: &LastLayout,
        bounds: &mut Bounds<Pixels>,
        cx: &mut App,
    ) -> Option<Path<Pixels>> {
        let state = self.state.read(cx);
        let mut selected_range = state.selected_range;
        if let Some(ime_marked_range) = &state.ime_marked_range {
            if !ime_marked_range.is_empty() {
                selected_range = (ime_marked_range.end..ime_marked_range.end).into();
            }
        }
        if selected_range.is_empty() {
            return None;
        }

        if state.masked {
            // Because masked use `*`, 1 char with 1 byte.
            selected_range.start = state.text.offset_to_char_index(selected_range.start);
            selected_range.end = state.text.offset_to_char_index(selected_range.end);
        }

        let (start_ix, end_ix) = if selected_range.start < selected_range.end {
            (selected_range.start, selected_range.end)
        } else {
            (selected_range.end, selected_range.start)
        };

        let range = start_ix.max(last_layout.visible_range_offset.start)
            ..end_ix.min(last_layout.visible_range_offset.end);

        Self::layout_match_range(range, &last_layout, bounds)
    }

    /// Calculate the visible range of lines in the viewport.
    ///
    /// Returns
    ///
    /// - visible_range: The visible range is based on unwrapped lines (Zero based).
    /// - visible_top: The top position of the first visible line in the scroll viewport.
    fn calculate_visible_range(
        &self,
        state: &InputState,
        line_height: Pixels,
        input_height: Pixels,
    ) -> (Range<usize>, Pixels) {
        if state.mode.is_single_line() {
            return (0..1, px(0.));
        }

        let scroll_top = if let Some(deferred_scroll_offset) = state.deferred_scroll_offset {
            deferred_scroll_offset.y
        } else {
            state.scroll_handle.offset().y
        };

        let text = &state.text_wrapper;
        let row_at = |edge: Pixels| {
            let row = (f32::from(edge) / f32::from(line_height)).ceil() as usize;
            text.hard_row_at(row.saturating_sub(1)).map(|(row, _)| row)
        };
        let start = row_at(-scroll_top).map_or(text.lines.len().saturating_sub(1), |row| {
            row.saturating_sub(1)
        });
        let end = row_at(input_height - scroll_top).map_or(text.lines.len(), |row| row + 1);
        (start..end, text.soft_row_start(start) * line_height)
    }

    /// Return (line_number_width, line_number_len)
    fn layout_line_numbers(
        state: &InputState,
        text: &Rope,
        font_size: Pixels,
        style: &TextStyle,
        window: &mut Window,
    ) -> (Pixels, usize) {
        let total_lines = text.lines_len();
        let line_number_len = match total_lines {
            0..=9999 => 5,
            10000..=99999 => 6,
            100000..=999999 => 7,
            _ => 8,
        };

        let line_number_width = if state.mode.line_number() {
            let empty_line_number = window.text_system().shape_line(
                "+".repeat(line_number_len).into(),
                font_size,
                &[TextRun {
                    len: line_number_len,
                    font: style.font(),
                    color: gpui::black(),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );

            empty_line_number.width + px(6.) + LINE_NUMBER_RIGHT_MARGIN
        } else {
            px(0.)
        };

        (line_number_width, line_number_len)
    }

    /// Compute inline completion ghost lines for rendering.
    ///
    /// Returns (first_line, ghost_lines) where:
    /// - first_line: Shaped text for the first line (goes after cursor on same line)
    /// - ghost_lines: Shaped lines for subsequent lines (shift content down)
    fn layout_inline_completion(
        state: &InputState,
        visible_range: &Range<usize>,
        font_size: Pixels,
        window: &mut Window,
        cx: &App,
    ) -> (Option<ShapedLine>, Vec<ShapedLine>) {
        // Must be focused to show inline completion
        if !state.focus_handle.is_focused(window) {
            return (None, vec![]);
        }

        let Some(completion_item) = state.inline_completion.item.as_ref() else {
            return (None, vec![]);
        };

        // Get cursor row from cursor position
        let cursor_row = state.cursor_position().line as usize;

        // Only show if cursor row is visible
        if cursor_row < visible_range.start || cursor_row >= visible_range.end {
            return (None, vec![]);
        }

        let completion_text = &completion_item.insert_text;
        let completion_color = cx.theme().muted_foreground.opacity(0.5);

        let text_style = window.text_style();
        let font = text_style.font();

        let lines: Vec<&str> = completion_text.split('\n').collect();
        if lines.is_empty() {
            return (None, vec![]);
        }

        // Shape first line (goes after cursor)
        let first_text: SharedString = lines[0].to_string().into();
        let first_line = if !first_text.is_empty() {
            let first_run = TextRun {
                len: first_text.len(),
                font: font.clone(),
                color: completion_color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            Some(
                window
                    .text_system()
                    .shape_line(first_text, font_size, &[first_run], None),
            )
        } else {
            None
        };

        // Shape ghost lines (lines 2+ that shift content down)
        let ghost_lines: Vec<ShapedLine> = lines[1..]
            .iter()
            .map(|line_text| {
                let text: SharedString = line_text.to_string().into();
                let len = text.len().max(1); // Ensure at least 1 for empty lines
                let run = TextRun {
                    len,
                    font: font.clone(),
                    color: completion_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                // Use space for empty lines so they take up height
                let shaped_text = if text.is_empty() { " ".into() } else { text };
                window
                    .text_system()
                    .shape_line(shaped_text, font_size, &[run], None)
            })
            .collect();

        (first_line, ghost_lines)
    }

    fn layout_lines(
        state: &InputState,
        display_text: &Rope,
        last_layout: &LastLayout,
        font_size: Pixels,
        runs: &[TextRun],
        bg_segments: &[(Range<usize>, Hsla)],
        window: &mut Window,
    ) -> Vec<LineLayout> {
        let is_single_line = state.mode.is_single_line();
        let text_wrapper = &state.text_wrapper;
        let visible_range = &last_layout.visible_range;
        let visible_range_offset = &last_layout.visible_range_offset;

        if is_single_line {
            let shaped_line = window.text_system().shape_line(
                display_text.to_string().into(),
                font_size,
                &runs,
                None,
            );

            return vec![LineLayout::new().lines(smallvec::smallvec![shaped_line])];
        }

        // Empty to use placeholder, the placeholder is not in the text_wrapper map.
        if state.text.len() == 0 {
            return display_text
                .to_string()
                .split("\n")
                .map(|line| {
                    let shaped_line = window.text_system().shape_line(
                        line.to_string().into(),
                        font_size,
                        &runs,
                        None,
                    );
                    LineLayout::new().lines(smallvec::smallvec![shaped_line])
                })
                .collect();
        }

        let mut lines = vec![];
        let mut offset = 0;
        let mut line_top = last_layout.visible_top;
        let scroll_top = -state
            .deferred_scroll_offset
            .unwrap_or_else(|| state.scroll_handle.offset())
            .y;
        let scroll_bottom = scroll_top + last_layout.viewport_height;
        for row in visible_range.clone() {
            let Some(line_item) = text_wrapper.lines.get(row) else {
                break;
            };
            let line = display_text.slice_line(row);
            let first_row = ((f32::from(scroll_top - line_top)
                / f32::from(last_layout.line_height))
            .floor()
            .max(0.)) as usize;
            let last_row = (f32::from(scroll_bottom - line_top)
                / f32::from(last_layout.line_height))
            .ceil()
            .max(0.) as usize;
            let end = last_row.saturating_add(2).min(line_item.lines_len());
            let mut needed: std::collections::BTreeSet<usize> = (first_row..end).collect();
            for endpoint in [
                state.cursor(),
                state.selected_range.start,
                state.selected_range.end,
            ] {
                if endpoint >= visible_range_offset.start + offset
                    && endpoint <= visible_range_offset.start + offset + line.len()
                {
                    let local = endpoint - visible_range_offset.start - offset;
                    needed.insert(line_item.row_at(local));
                }
            }
            let mut wrapped_lines = SmallVec::new();
            for index in needed {
                let range = line_item.range(index).unwrap();
                let line_runs = runs_for_range(runs, offset, &range);
                let line_runs = if bg_segments.is_empty() {
                    line_runs
                } else {
                    split_runs_by_bg_segments(
                        visible_range_offset.start + offset,
                        &line_runs,
                        bg_segments,
                    )
                };
                let sub_line: SharedString = line.slice(range.clone()).to_string().into();
                let shaped = window
                    .text_system()
                    .shape_line(sub_line, font_size, &line_runs, None);
                wrapped_lines.push(WrappedLine {
                    row: index,
                    range,
                    shaped,
                });
            }
            let mut layout = LineLayout::new();
            layout.set_visible_lines(line.len(), line_item.lines_len(), wrapped_lines);
            line_top += line_item.height(last_layout.line_height);
            lines.push(layout);
            offset += line.len() + 1;
        }

        lines
    }

    /// First usize is the offset of skipped.
    fn highlight_lines(
        &mut self,
        visible_range: &Range<usize>,
        _visible_top: Pixels,
        visible_byte_range: Range<usize>,
        cx: &mut App,
    ) -> Option<Vec<(Range<usize>, HighlightStyle)>> {
        let state = self.state.read(cx);
        if let Some(highlights) = &state.external_highlights {
            let base = vec![(
                visible_byte_range.clone(),
                HighlightStyle {
                    color: Some(cx.theme().foreground),
                    ..Default::default()
                },
            )];
            let highlights = highlights
                .iter()
                .filter_map(|(range, style)| {
                    let range = range.start.max(visible_byte_range.start)
                        ..range.end.min(visible_byte_range.end);
                    (range.start < range.end).then_some((range, *style))
                })
                .collect::<Vec<_>>();
            return Some(gpui::combine_highlights(base, highlights).collect());
        }
        let text = &state.text;
        let is_multi_line = state.mode.is_multi_line();

        let (highlighter, diagnostics) = match &state.mode {
            InputMode::CodeEditor {
                highlighter,
                diagnostics,
                ..
            } => (highlighter.borrow(), diagnostics),
            _ => return None,
        };
        let highlighter = highlighter.as_ref()?;

        let mut offset = visible_byte_range.start;
        let mut styles = vec![];

        for line in text
            .iter_lines()
            .skip(visible_range.start)
            .take(visible_range.len())
        {
            let line_len = if is_multi_line {
                // +1 for `\n`
                line.len() + 1
            } else {
                line.len()
            };

            let range = offset..offset + line_len;
            let line_styles = highlighter.styles(&range, &cx.theme().highlight_theme);
            styles = gpui::combine_highlights(styles, line_styles).collect();

            offset = range.end;
        }

        let diagnostic_styles = diagnostics.styles_for_range(&visible_byte_range, cx);

        // hover definition style
        if let Some(hover_style) = self.layout_hover_definition(cx) {
            styles.push(hover_style);
        }

        // Combine marker styles
        styles = gpui::combine_highlights(diagnostic_styles, styles).collect();

        Some(styles)
    }
}

pub(super) struct PrepaintState {
    /// The lines of entire lines.
    last_layout: LastLayout,
    /// The lines only contains the visible lines in the viewport, based on `visible_range`.
    ///
    /// The child is the soft lines.
    line_numbers: Option<Vec<(ShapedLine, usize)>>,
    /// Size of the scrollable area by entire lines.
    scroll_size: Size<Pixels>,
    cursor_bounds: Option<Bounds<Pixels>>,
    cursor_scroll_offset: Point<Pixels>,
    /// row index (zero based), no wrap, same line as the cursor.
    current_row: Option<usize>,
    selection_path: Option<Path<Pixels>>,
    hover_highlight_path: Option<Path<Pixels>>,
    search_match_bounds: Vec<(Bounds<Pixels>, bool)>,
    document_color_paths: Vec<(Path<Pixels>, Hsla)>,
    hover_definition_hitbox: Option<Hitbox>,
    indent_guides_path: Option<Path<Pixels>>,
    bounds: Bounds<Pixels>,
    // Inline completion rendering data
    /// Shaped ghost lines to paint after cursor row (completion lines 2+)
    ghost_lines: Vec<ShapedLine>,
    /// First line of inline completion (painted after cursor on same line)
    ghost_first_line: Option<ShapedLine>,
    ghost_lines_height: Pixels,
}

impl PrepaintState {
    /// Returns cursor bounds adjusted for scroll offset, if available.
    fn cursor_bounds_with_scroll(&self) -> Option<Bounds<Pixels>> {
        self.cursor_bounds.map(|mut bounds| {
            bounds.origin.y += self.cursor_scroll_offset.y;
            bounds
        })
    }
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// A debug function to print points as SVG path.
#[allow(unused)]
fn print_points_as_svg_path(
    line_corners: &Vec<Corners<Point<Pixels>>>,
    points: &Vec<Point<Pixels>>,
) {
    for corners in line_corners {
        println!(
            "tl: ({}, {}), tr: ({}, {}), bl: ({}, {}), br: ({}, {})",
            corners.top_left.x.as_f32() as i32,
            corners.top_left.y.as_f32() as i32,
            corners.top_right.x.as_f32() as i32,
            corners.top_right.y.as_f32() as i32,
            corners.bottom_left.x.as_f32() as i32,
            corners.bottom_left.y.as_f32() as i32,
            corners.bottom_right.x.as_f32() as i32,
            corners.bottom_right.y.as_f32() as i32,
        );
    }

    if points.len() > 0 {
        println!(
            "M{},{}",
            points[0].x.as_f32() as i32,
            points[0].y.as_f32() as i32
        );
        for p in points.iter().skip(1) {
            println!("L{},{}", p.x.as_f32() as i32, p.y.as_f32() as i32);
        }
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<PrepaintState>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = self.state.read(cx);
        let line_height = window.line_height();

        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if state.mode.is_multi_line() {
            style.flex_grow = 1.0;
            style.size.height = relative(1.).into();
            if state.mode.is_auto_grow() {
                // Auto grow to let height match to rows, but not exceed max rows.
                let rows = state.mode.max_rows().min(state.mode.rows());
                style.min_size.height = (rows * line_height).into();
            } else {
                style.min_size.height = line_height.into();
            }
        } else {
            // For single-line inputs, the minimum height should be the line height
            style.size.height = line_height.into();
        };

        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let style = window.text_style();
        let font = style.font();
        let text_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let opening_rows = ((f32::from(bounds.size.height) / f32::from(line_height)).ceil()
            as usize)
            .saturating_add(2);

        let state = self.state.read(cx);
        let (line_number_width, line_number_len) =
            Self::layout_line_numbers(state, &state.text, text_size, &style, window);
        let wrap_width = if state.mode.is_multi_line() && state.soft_wrap {
            Some(state.wrap_layout_width(bounds.size.width - line_number_width - RIGHT_MARGIN))
        } else {
            None
        };
        let prepared = self.state.update(cx, |state, cx| {
            let _wrap = gpui::work_trace::Span::new("editor_wrap");
            let prepared = state.prepare_layout(
                LayoutConfig {
                    font,
                    font_size: text_size,
                    wrap_width,
                },
                Some((window.text_system(), opening_rows)),
                cx,
            );
            if prepared && let Some(lines) = state.opening_lines.take() {
                window.text_system().adopt_prepared_lines(&lines);
            }
            prepared
        });
        if !prepared {
            return None;
        }

        let state = self.state.read(cx);
        let wrap_width = state.text_wrapper.wrap_width();

        let (visible_range, visible_top) =
            self.calculate_visible_range(&state, line_height, bounds.size.height);
        let visible_start_offset = state.text.line_start_offset(visible_range.start);
        let visible_end_offset = state
            .text
            .line_end_offset(visible_range.end.saturating_sub(1));

        let highlight_styles = self.highlight_lines(
            &visible_range,
            visible_top,
            visible_start_offset..visible_end_offset,
            cx,
        );

        let state = self.state.read(cx);
        let text = state.text.clone();
        let is_empty = text.len() == 0;
        let placeholder = self.placeholder.clone();

        let mut bounds = bounds;

        let (display_text, text_color) = if is_empty {
            (
                &Rope::from(placeholder.as_str()),
                cx.theme().muted_foreground,
            )
        } else if state.masked {
            (
                &Rope::from("*".repeat(text.chars().count())),
                cx.theme().foreground,
            )
        } else {
            (&text, cx.theme().foreground)
        };

        let text_style = window.text_style();

        let mut last_layout = LastLayout {
            visible_range,
            visible_top,
            viewport_height: bounds.size.height,
            visible_range_offset: visible_start_offset..visible_end_offset,
            line_height,
            wrap_width,
            line_number_width,
            lines: Rc::new(vec![]),
            cursor_bounds: None,
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_run = TextRun {
            len: 0,
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(text_color),
                wavy: false,
            }),
            strikethrough: None,
        };

        let runs = if !is_empty {
            if let Some(highlight_styles) = highlight_styles {
                let mut runs = vec![];

                runs.extend(highlight_styles.iter().map(|(range, style)| {
                    let mut run = text_style.clone().highlight(*style).to_run(range.len());
                    if let Some(ime_marked_range) = &state.ime_marked_range {
                        if range.start >= ime_marked_range.start
                            && range.end <= ime_marked_range.end
                        {
                            run.color = marked_run.color;
                            run.strikethrough = marked_run.strikethrough;
                            run.underline = marked_run.underline;
                        }
                    }

                    run
                }));

                runs.into_iter().filter(|run| run.len > 0).collect()
            } else {
                vec![run]
            }
        } else if let Some(ime_marked_range) = &state.ime_marked_range {
            // IME marked text
            vec![
                TextRun {
                    len: ime_marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: ime_marked_range.end - ime_marked_range.start,
                    underline: marked_run.underline,
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - ime_marked_range.end,
                    ..run.clone()
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let document_colors = state
            .lsp
            .document_colors_for_range(&text, &last_layout.visible_range);
        let layout = gpui::work_trace::Span::new("editor_shape");
        let lines = Self::layout_lines(
            &state,
            &display_text,
            &last_layout,
            text_size,
            &runs,
            &document_colors,
            window,
        );
        drop(layout);

        let mut longest_line_width = wrap_width.unwrap_or(px(0.));
        // 1. Single line
        // 2. Multi-line with soft wrap disabled.
        if state.mode.is_single_line() || !state.soft_wrap {
            let longest_row = state.text_wrapper.longest_row.row;
            let longest_line: SharedString = state.text.slice_line(longest_row).to_string().into();
            longest_line_width = window
                .text_system()
                .shape_line(
                    longest_line.clone(),
                    text_size,
                    &[TextRun {
                        len: longest_line.len(),
                        font: style.font(),
                        color: gpui::black(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    wrap_width,
                )
                .width;
        }
        last_layout.lines = Rc::new(lines);

        let (ghost_first_line, ghost_lines) = Self::layout_inline_completion(
            state,
            &last_layout.visible_range,
            text_size,
            window,
            cx,
        );
        let ghost_line_count = ghost_lines.len();
        let ghost_lines_height = ghost_line_count as f32 * line_height;

        let total_wrapped_lines = state.text_wrapper.len();
        let empty_bottom_height = if state.mode.is_code_editor() {
            bounds
                .size
                .height
                .half()
                .max(BOTTOM_MARGIN_ROWS * line_height)
        } else {
            px(0.)
        };

        let scroll_size = size(
            if longest_line_width + line_number_width + RIGHT_MARGIN > bounds.size.width {
                longest_line_width + line_number_width + RIGHT_MARGIN
            } else {
                longest_line_width
            },
            (total_wrapped_lines as f32 * line_height + empty_bottom_height + ghost_lines_height)
                .max(bounds.size.height),
        );

        // `position_for_index` for example
        //
        // #### text
        //
        // Hello 世界，this is GPUI component.
        // The GPUI Component is a collection of UI components for
        // GPUI framework, including Button, Input, Checkbox, Radio,
        // Dropdown, Tab, and more...
        //
        // wrap_width: 444px, line_height: 20px
        //
        // #### lines[0]
        //
        // | index | pos              | line |
        // |-------|------------------|------|
        // | 5     | (37 px, 0.0)     | 0    |
        // | 38    | (261.7 px, 20.0) | 0    |
        // | 40    | None             | -    |
        //
        // #### lines[1]
        //
        // | index | position              | line |
        // |-------|-----------------------|------|
        // | 5     | (43.578125 px, 0.0)   | 0    |
        // | 56    | (422.21094 px, 0.0)   | 0    |
        // | 57    | (11.6328125 px, 20.0) | 1    |
        // | 114   | (429.85938 px, 20.0)  | 1    |
        // | 115   | (11.3125 px, 40.0)    | 2    |

        // Calculate the scroll offset to keep the cursor in view

        let (cursor_bounds, cursor_scroll_offset, current_row) =
            self.layout_cursor(&last_layout, &mut bounds, window, cx);
        last_layout.cursor_bounds = cursor_bounds;

        let search_match_bounds = self.layout_search_matches(&last_layout, &mut bounds, cx);
        let selection_path = self.layout_selections(&last_layout, &mut bounds, cx);
        let hover_highlight_path = self.layout_hover_highlight(&last_layout, &mut bounds, cx);
        let document_color_paths =
            self.layout_document_colors(&document_colors, &last_layout, &bounds);

        let state = self.state.read(cx);
        let line_numbers = if state.mode.line_number() {
            let mut line_numbers = vec![];
            let other_line_runs = vec![TextRun {
                len: line_number_len,
                font: style.font(),
                color: cx.theme().muted_foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let current_line_runs = vec![TextRun {
                len: line_number_len,
                font: style.font(),
                color: cx.theme().foreground,
                background_color: None,
                underline: None,
                strikethrough: None,
            }];

            // build line numbers
            for (ix, line) in last_layout.lines.iter().enumerate() {
                let ix = last_layout.visible_range.start + ix;
                let line_no = format!("{:>width$}", ix + 1, width = line_number_len).into();

                let runs = if current_row == Some(ix) {
                    &current_line_runs
                } else {
                    &other_line_runs
                };

                let shaped = window
                    .text_system()
                    .shape_line(line_no, text_size, &runs, None);
                line_numbers.push((shaped, line.rows()));
            }
            Some(line_numbers)
        } else {
            None
        };

        let hover_definition_hitbox = self.layout_hover_definition_hitbox(state, window, cx);
        let indent_guides_path =
            self.layout_indent_guides(state, &bounds, &last_layout, &text_style, window);

        Some(PrepaintState {
            bounds,
            last_layout,
            scroll_size,
            line_numbers,
            cursor_bounds,
            cursor_scroll_offset,
            current_row,
            selection_path,
            search_match_bounds,
            hover_highlight_path,
            hover_definition_hitbox,
            document_color_paths,
            indent_guides_path,
            ghost_first_line,
            ghost_lines,
            ghost_lines_height,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        input_bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            let text: SharedString = "Preparing editor…".into();
            let style = window.text_style();
            let line = window.text_system().shape_line(
                text.clone(),
                style.font_size.to_pixels(window.rem_size()),
                &[TextRun {
                    len: text.len(),
                    font: style.font(),
                    color: cx.theme().muted_foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            _ = line.paint(input_bounds.origin, window.line_height(), window, cx);
            return;
        };
        let focus_handle = self.state.read(cx).focus_handle.clone();
        let show_cursor = self.state.read(cx).show_cursor(window, cx);
        let focused = focus_handle.is_focused(window);
        let bounds = prepaint.bounds;
        let selected_range = self.state.read(cx).selected_range;
        let visible_range = &prepaint.last_layout.visible_range;

        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.state.clone()),
            cx,
        );

        // Set Root focused_input when self is focused
        if focused {
            let state = self.state.clone();
            if Root::read(window, cx).focused_input.as_ref() != Some(&state) {
                Root::update(window, cx, |root, _, cx| {
                    root.focused_input = Some(state);
                    cx.notify();
                });
            }
        }

        // And reset focused_input when next_frame start
        window.on_next_frame({
            let state = self.state.clone();
            move |window, cx| {
                if !focused && Root::read(window, cx).focused_input.as_ref() == Some(&state) {
                    Root::update(window, cx, |root, _, cx| {
                        root.focused_input = None;
                        cx.notify();
                    });
                }
            }
        });

        // Paint multi line text
        let line_height = window.line_height();
        let origin = bounds.origin;

        let invisible_top_padding = prepaint.last_layout.visible_top;

        let mut mask_offset_y = px(0.);
        let state = self.state.read(cx);
        if state.masked && state.text.len() > 0 {
            // Move down offset for vertical centering the *****
            if cfg!(target_os = "macos") {
                mask_offset_y = px(3.);
            } else {
                mask_offset_y = px(2.5);
            }
        }

        let active_line_color = cx.theme().highlight_theme.style.editor_active_line;

        // Paint active line
        let mut offset_y = px(0.);
        if let Some(line_numbers) = prepaint.line_numbers.as_ref() {
            offset_y += invisible_top_padding;

            // Each item is the normal lines.
            for (ix, (_, rows)) in line_numbers.iter().enumerate() {
                let row = visible_range.start + ix;
                let is_active = prepaint.current_row == Some(row);
                let p = point(input_bounds.origin.x, origin.y + offset_y);
                let height = line_height * *rows as f32;
                // Paint the current line background
                if is_active {
                    if let Some(bg_color) = active_line_color {
                        window.paint_quad(fill(
                            Bounds::new(p, size(bounds.size.width, height)),
                            bg_color,
                        ));
                    }
                }
                offset_y += height;
            }
        }

        // Paint indent guides
        if let Some(path) = prepaint.indent_guides_path.take() {
            window.paint_path(path, cx.theme().border.opacity(0.85));
        }

        // Paint selections
        if window.is_window_active() {
            let secondary_selection = cx.theme().selection.saturation(0.1);
            for &(rect, is_active) in &prepaint.search_match_bounds {
                window.paint_quad(fill(rect, secondary_selection));

                if is_active {
                    window.paint_quad(fill(rect, cx.theme().selection));
                }
            }

            if let Some(path) = prepaint.selection_path.take() {
                window.paint_path(path, cx.theme().selection);
            }

            // Paint hover highlight
            if let Some(path) = prepaint.hover_highlight_path.take() {
                window.paint_path(path, secondary_selection);
            }

            for &(rect, current) in &prepaint.search_match_bounds {
                let height = rect.size.height.min(px(if current { 2. } else { 1. }));
                let origin = rect.origin + point(px(0.), rect.size.height - height);
                let underline = Bounds::new(origin, size(rect.size.width, height));
                let ink = if current {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                };
                window.paint_quad(fill(underline, ink));
            }
        }

        // Paint document colors
        for (path, color) in prepaint.document_color_paths.iter() {
            window.paint_path(path.clone(), *color);
        }

        // Paint text with inline completion ghost line support
        let mut offset_y = mask_offset_y + invisible_top_padding;
        let ghost_lines = &prepaint.ghost_lines;
        let has_ghost_lines = !ghost_lines.is_empty();

        for (ix, line) in prepaint.last_layout.lines.iter().enumerate() {
            let row = visible_range.start + ix;
            let p = point(
                origin.x + prepaint.last_layout.line_number_width,
                origin.y + offset_y,
            );

            // Paint the actual line
            _ = line.paint(p, line_height, window, cx);
            offset_y += line.size(line_height).height;

            // After the cursor row, paint ghost lines (which shifts subsequent content down)
            if has_ghost_lines && Some(row) == prepaint.current_row {
                let ghost_x = origin.x + prepaint.last_layout.line_number_width;

                for ghost_line in ghost_lines {
                    let ghost_p = point(ghost_x, origin.y + offset_y);

                    // Paint semi-transparent background for ghost line
                    let ghost_bounds = Bounds::new(
                        ghost_p,
                        size(
                            bounds.size.width - prepaint.last_layout.line_number_width,
                            line_height,
                        ),
                    );
                    window.paint_quad(fill(ghost_bounds, cx.theme().editor_background()));

                    // Paint ghost line text
                    _ = ghost_line.paint(ghost_p, line_height, window, cx);
                    offset_y += line_height;
                }
            }
        }

        // Paint blinking cursor
        if focused && show_cursor {
            if let Some(cursor_bounds) = prepaint.cursor_bounds_with_scroll() {
                window.paint_quad(fill(cursor_bounds, cx.theme().caret));
            }
        }

        // Paint line numbers
        let mut offset_y = px(0.);
        if let Some(line_numbers) = prepaint.line_numbers.as_ref() {
            offset_y += invisible_top_padding;

            window.paint_quad(fill(
                Bounds {
                    origin: input_bounds.origin,
                    size: size(
                        prepaint.last_layout.line_number_width - LINE_NUMBER_RIGHT_MARGIN,
                        input_bounds.size.height + prepaint.ghost_lines_height,
                    ),
                },
                cx.theme().editor_background(),
            ));

            // Each item is the normal lines.
            for (ix, (line, rows)) in line_numbers.iter().enumerate() {
                let row = visible_range.start + ix;

                let p = point(input_bounds.origin.x, origin.y + offset_y);
                let is_active = prepaint.current_row == Some(row);

                let height = line_height * *rows as f32;
                // paint active line number background
                if is_active {
                    if let Some(bg_color) = active_line_color {
                        window.paint_quad(fill(
                            Bounds::new(p, size(prepaint.last_layout.line_number_width, height)),
                            bg_color,
                        ));
                    }
                }

                _ = line.paint(p, line_height, window, cx);
                offset_y += line_height * *rows as f32;

                // Add ghost line height after cursor row for line numbers alignment
                if !prepaint.ghost_lines.is_empty() && prepaint.current_row.is_some() {
                    offset_y += prepaint.ghost_lines_height;
                }
            }
        }

        if self.state.read(cx).scroll_handle.offset() != prepaint.cursor_scroll_offset {
            let state = self.state.clone();
            window.on_next_frame(move |_, cx| state.update(cx, |_, cx| cx.notify()));
            window.request_animation_frame();
        }
        self.state.update(cx, |state, cx| {
            let initial_layout_completed = state.initial_layout_pending();
            // A panel opening above the text (Find) shortens the viewport after the
            // cursor was scrolled into view; keep the cursor visible in the new height.
            let shrunk = state
                .last_bounds
                .is_some_and(|last| bounds.size.height < last.size.height);
            state.last_layout = Some(prepaint.last_layout.clone());
            state.last_bounds = Some(bounds);
            state.last_cursor = Some(state.cursor());
            state.set_input_bounds(input_bounds, cx);
            state.last_selected_range = Some(selected_range);
            state.scroll_size = prepaint.scroll_size;
            state.update_scroll_offset(Some(prepaint.cursor_scroll_offset), cx);
            state.deferred_scroll_offset = None;
            if shrunk || std::mem::take(&mut state.reveal_cursor_after_layout) {
                let cursor = state.cursor();
                state.scroll_to(cursor, None, cx);
            }

            if initial_layout_completed {
                cx.emit(InputEvent::InitialLayoutReady);
            }
            cx.notify();
        });

        if let Some(hitbox) = prepaint.hover_definition_hitbox.as_ref() {
            window.set_cursor_style(gpui::CursorStyle::PointingHand, &hitbox);
        }

        // Paint inline completion first line suffix (after cursor on same line)
        if focused {
            if let Some(first_line) = &prepaint.ghost_first_line {
                if let Some(cursor_bounds) = prepaint.cursor_bounds_with_scroll() {
                    let first_line_x = cursor_bounds.origin.x + cursor_bounds.size.width;
                    let p = point(first_line_x, cursor_bounds.origin.y);

                    // Paint background to cover any existing text
                    let bg_bounds = Bounds::new(p, size(first_line.width + px(4.), line_height));
                    window.paint_quad(fill(bg_bounds, cx.theme().editor_background()));

                    // Paint first line completion text
                    _ = first_line.paint(p, line_height, window, cx);
                }
            }
        }

        self.paint_mouse_listeners(window, cx);
    }
}

fn matching_indices(ranges: &MatchRanges, visible: &[Range<usize>]) -> Vec<usize> {
    let mut indices = Vec::new();
    for interval in visible {
        if interval.is_empty() {
            continue;
        }
        indices.extend(ranges.intersecting(interval));
    }
    indices.sort_unstable();
    indices.dedup();
    indices
}

/// Get the runs for the given range.
///
/// The range is the byte range of the wrapped line.
pub(super) fn runs_for_range(
    runs: &[TextRun],
    line_offset: usize,
    range: &Range<usize>,
) -> Vec<TextRun> {
    let mut result = vec![];
    let range = (line_offset + range.start)..(line_offset + range.end);
    let mut cursor = 0;

    for run in runs {
        let run_start = cursor;
        let run_end = cursor + run.len;

        if run_end <= range.start {
            cursor = run_end;
            continue;
        }

        if run_start >= range.end {
            break;
        }

        let start = range.start.max(run_start) - run_start;
        let end = range.end.min(run_end) - run_start;
        let len = end - start;

        if len > 0 {
            result.push(TextRun { len, ..run.clone() });
        }

        cursor = run_end;
    }

    result
}

fn split_runs_by_bg_segments(
    start_offset: usize,
    runs: &[TextRun],
    bg_segments: &[(Range<usize>, Hsla)],
) -> Vec<TextRun> {
    let mut result = vec![];

    let mut cursor = start_offset;
    for run in runs {
        let mut run_start = cursor;
        let run_end = cursor + run.len;

        for (bg_range, bg_color) in bg_segments {
            if run_end <= bg_range.start || run_start >= bg_range.end {
                continue;
            }

            // Overlap exists
            if run_start < bg_range.start {
                // Add the part before the background range
                result.push(TextRun {
                    len: bg_range.start - run_start,
                    ..run.clone()
                });
            }

            // Add the overlapping part with background color
            let overlap_start = run_start.max(bg_range.start);
            let overlap_end = run_end.min(bg_range.end);
            let text_color = if bg_color.l >= 0.5 {
                gpui::black()
            } else {
                gpui::white()
            };

            let run_len = overlap_end.saturating_sub(overlap_start);
            if run_len > 0 {
                result.push(TextRun {
                    len: run_len,
                    color: text_color,
                    ..run.clone()
                });

                cursor = bg_range.end;
                run_start = cursor;
            }
        }

        if run_end > cursor {
            // Add the part after the background range
            result.push(TextRun {
                len: run_end - cursor,
                ..run.clone()
            });
        }

        cursor = run_end;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_matches_equal_full_scan_across_gaps_and_wrapped_boundaries() {
        for seed in 0..128 {
            let mut cursor = 0;
            let ranges = (0..400)
                .map(|index| {
                    cursor += (index + seed) % 3;
                    let start = cursor;
                    cursor += (index * seed + 1) % 17 + 1;
                    start..cursor
                })
                .collect::<Vec<_>>();
            let visible = (0..20)
                .map(|index| {
                    let start = (index * 317 + seed * 137) % (cursor + 100);
                    start..start + index * 7
                })
                .collect::<Vec<_>>();
            let expected = ranges
                .iter()
                .enumerate()
                .filter(|(_, range)| {
                    visible.iter().any(|interval| {
                        !interval.is_empty()
                            && range.start < interval.end
                            && range.end > interval.start
                    })
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let mut compact = MatchRanges::new(cursor);
            ranges.into_iter().for_each(|range| compact.push(range));
            assert_eq!(matching_indices(&compact, &visible), expected, "{seed}");
        }
        let mut ranges = MatchRanges::new(100_100);
        ranges.push(0..100_000);
        ranges.push(100_001..100_100);
        assert_eq!(
            matching_indices(&ranges, &[10..20, 50..60, 100_010..100_020]),
            vec![0, 1]
        );
    }

    #[test]
    fn search_bounds_cover_only_matching_shaped_rows() {
        let mut line = LineLayout::new();
        line.set_visible_lines(
            1000,
            100,
            smallvec::smallvec![
                WrappedLine {
                    row: 40,
                    range: 400..410,
                    shaped: ShapedLine::default().with_len(10)
                },
                WrappedLine {
                    row: 80,
                    range: 800..810,
                    shaped: ShapedLine::default().with_len(10)
                },
            ],
        );
        let layout = LastLayout {
            visible_range: 0..1,
            visible_top: px(0.),
            viewport_height: px(200.),
            visible_range_offset: 0..1000,
            lines: Rc::new(vec![line]),
            line_height: px(10.),
            wrap_width: Some(px(100.)),
            line_number_width: px(20.),
            cursor_bounds: None,
        };
        let bounds = Bounds::new(point(px(10.), px(30.)), size(px(300.), px(200.)));
        for (range, tops) in [
            (300..500, &[430.][..]),
            (405..900, &[430., 830.][..]),
            (0..1000, &[430., 830.][..]),
            (0..100, &[][..]),
            (500..600, &[][..]),
            (900..1000, &[][..]),
        ] {
            let mut actual = Vec::new();
            TextElement::layout_search_range(&range, &layout, &bounds, |rect| actual.push(rect));
            let expected = tops
                .iter()
                .map(|&top| Bounds::new(point(px(30.), px(top)), size(px(6.), px(10.))))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{range:?}");
        }
    }

    #[test]
    fn test_runs_for_range() {
        let run = TextRun {
            len: 0,
            font: gpui::font(".SystemUIFont"),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        // use hello this-is-test
        let runs = vec![
            // use
            TextRun {
                len: 3,
                ..run.clone()
            },
            // \s
            TextRun {
                len: 1,
                ..run.clone()
            },
            // hello
            TextRun {
                len: 5,
                ..run.clone()
            },
            // \s
            TextRun {
                len: 1,
                ..run.clone()
            },
            // this-is-test
            TextRun {
                len: 12,
                ..run.clone()
            },
        ];

        #[track_caller]
        fn assert_runs(actual: Vec<TextRun>, expected: &[usize]) {
            let left = actual.iter().map(|run| run.len).collect::<Vec<_>>();
            assert_eq!(left, expected);
        }

        assert_runs(runs_for_range(&runs, 0, &(0..0)), &[]);
        assert_runs(runs_for_range(&runs, 0, &(0..100)), &[3, 1, 5, 1, 12]);

        assert_runs(runs_for_range(&runs, 0, &(0..6)), &[3, 1, 2]);
        assert_runs(runs_for_range(&runs, 0, &(1..6)), &[2, 1, 2]);
        assert_runs(runs_for_range(&runs, 0, &(3..10)), &[1, 5, 1]);
        assert_runs(runs_for_range(&runs, 0, &(5..8)), &[3]);
        assert_runs(runs_for_range(&runs, 3, &(0..3)), &[1, 2]);
        assert_runs(runs_for_range(&runs, 3, &(2..10)), &[4, 1, 3]);
        assert_runs(runs_for_range(&runs, 9, &(0..8)), &[1, 7]);
    }

    #[test]
    fn test_split_runs_by_bg_segments() {
        let run = TextRun {
            len: 0,
            font: gpui::font(".SystemUIFont"),
            color: gpui::blue(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = vec![
            TextRun {
                len: 5,
                ..run.clone()
            },
            TextRun {
                len: 7,
                ..run.clone()
            },
            TextRun {
                len: 24,
                ..run.clone()
            },
        ];

        let bg_segments = vec![(8..12, gpui::red()), (12..18, gpui::blue())];
        let result = split_runs_by_bg_segments(5, &runs, &bg_segments);
        assert_eq!(
            result.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![3, 2, 2, 5, 1, 23]
        );
        assert_eq!(result[0].color, gpui::blue());
        assert_eq!(result[1].color, gpui::black());
        assert_eq!(result[2].color, gpui::black());
        assert_eq!(result[3].color, gpui::black());
        assert_eq!(result[4].color, gpui::black());
        assert_eq!(result[5].color, gpui::blue());
    }
}
