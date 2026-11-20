use regex::{Regex, RegexBuilder};
use rust_i18n::t;
use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::{
    App, AppContext as _, Context, Empty, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, KeyBinding, ParentElement as _, Render, Styled, Subscription, WeakEntity, Window,
    actions, prelude::FluentBuilder as _, div, px, rems,
};
use ropey::{LineType, Rope, RopeBuilder, extra::esoterica::ropes_are_instances};

use crate::{
    ActiveTheme, Disableable, IconName, Sizable,
    actions::SelectUp,
    button::{Button, ButtonVariants},
    h_flex,
    input::{
        Enter, Escape, IndentInline, Input, InputEvent, InputState, OutdentInline, Search,
        movement::MoveDirection,
    },
    label::Label,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use super::state::PreparedReplacement;

const CONTEXT: &'static str = "SearchPanel";
const MATCH_SCAN_BYTES: usize = 64 * 1024;

actions!(input, [Tab]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys(vec![KeyBinding::new(
        "shift-enter",
        SelectUp,
        Some(CONTEXT),
    )]);
}

#[derive(Debug)]
pub(super) enum MatchRanges {
    Uniform { starts: Vec<u32>, length: u32 },
    Variable { starts: Vec<u32>, lengths: Vec<u32> },
    Wide(Vec<Range<usize>>),
}

impl MatchRanges {
    pub(super) fn new(text_bytes: usize) -> Self {
        if text_bytes <= u32::MAX as usize {
            Self::Uniform {
                starts: Vec::new(),
                length: 0,
            }
        } else {
            Self::Wide(Vec::new())
        }
    }

    pub(super) fn push(&mut self, range: Range<usize>) {
        match self {
            Self::Uniform { starts, length } => {
                let match_length = range.len() as u32;
                if starts.is_empty() {
                    *length = match_length;
                }
                if match_length == *length {
                    starts.push(range.start as u32);
                } else {
                    let mut lengths = vec![*length; starts.len()];
                    lengths.push(match_length);
                    starts.push(range.start as u32);
                    *self = Self::Variable {
                        starts: std::mem::take(starts),
                        lengths,
                    };
                }
            }
            Self::Variable { starts, lengths } => {
                starts.push(range.start as u32);
                lengths.push(range.len() as u32);
            }
            Self::Wide(ranges) => ranges.push(range),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Uniform { starts, .. } | Self::Variable { starts, .. } => starts.len(),
            Self::Wide(ranges) => ranges.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn get(&self, index: usize) -> Option<Range<usize>> {
        let (start, length) = match self {
            Self::Uniform { starts, length } => (*starts.get(index)?, *length),
            Self::Variable { starts, lengths } => (*starts.get(index)?, lengths[index]),
            Self::Wide(ranges) => return ranges.get(index).cloned(),
        };
        Some(start as usize..start as usize + length as usize)
    }

    fn iter(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        (0..self.len()).map(|index| self.get(index).unwrap())
    }

    fn partition_point(&self, mut predicate: impl FnMut(Range<usize>) -> bool) -> usize {
        let mut low = 0;
        let mut high = self.len();
        while low < high {
            let middle = low + (high - low) / 2;
            if predicate(self.get(middle).unwrap()) {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        low
    }

    pub(super) fn intersecting(&self, visible: &Range<usize>) -> Range<usize> {
        self.partition_point(|range| range.end <= visible.start)
            ..self.partition_point(|range| range.start < visible.end)
    }
}

fn advance_source<'a>(
    chunks: &mut impl Iterator<Item = &'a str>,
    remaining: &mut &'a str,
    mut length: usize,
    mut visit: impl FnMut(&str),
) {
    while length != 0 {
        if remaining.is_empty() {
            *remaining = chunks.next().expect("Match ranges fit the source revision");
        }
        let count = length.min(remaining.len());
        let (source, rest) = remaining.split_at(count);
        visit(source);
        *remaining = rest;
        length -= count;
    }
}

fn replace_matches(
    source: &Rope,
    ranges: &MatchRanges,
    replacement: &str,
    cancel: &AtomicBool,
) -> Option<Rope> {
    let mut chunks = source.chunks();
    let mut remaining = "";
    const CHUNK_BYTES: usize = 32 * 1024;
    let mut output = RopeBuilder::new();
    let mut batch = String::with_capacity(CHUNK_BYTES);
    let mut append = |text: &str| {
        if batch.len() + text.len() > CHUNK_BYTES {
            output.append(&batch);
            batch.clear();
        }
        if text.len() >= CHUNK_BYTES {
            output.append(text);
        } else {
            batch.push_str(text);
        }
    };
    let mut offset = 0;
    for (index, range) in ranges.iter().enumerate() {
        if index % 1024 == 0 && cancel.load(Ordering::Relaxed) {
            return None;
        }
        advance_source(&mut chunks, &mut remaining, range.start - offset, |text| {
            append(text);
        });
        advance_source(&mut chunks, &mut remaining, range.len(), |_| {});
        append(replacement);
        offset = range.end;
    }
    advance_source(&mut chunks, &mut remaining, source.len() - offset, |text| {
        append(text);
    });
    (!cancel.load(Ordering::Relaxed)).then(|| {
        output.append(&batch);
        output.finish()
    })
}

#[derive(Debug, Clone)]
pub struct SearchMatcher {
    text: Rope,
    pub query: Option<Regex>,
    query_error: bool,

    pub(super) matched_ranges: Arc<MatchRanges>,
    pub(super) current_match_ix: usize,
    /// Is in replacing mode, if true, the next update will not reset the current match index.
    replacing: bool,
    cancellation: Option<Arc<AtomicBool>>,
}

impl SearchMatcher {
    pub fn new() -> Self {
        Self {
            text: "".into(),
            query: None,
            query_error: false,
            matched_ranges: Arc::new(MatchRanges::new(0)),
            current_match_ix: 0,
            replacing: false,
            cancellation: None,
        }
    }

    /// Update source text and re-match
    #[cfg(test)]
    pub(crate) fn update(&mut self, text: &Rope) {
        if self.text.eq(text) {
            return;
        }

        self.text = text.clone();
        self.update_matches();
    }

    fn update_matches(&mut self) {
        let cancelled = || {
            self.cancellation
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        };
        if cancelled() {
            return;
        }
        let mut new_ranges = MatchRanges::new(self.text.len());
        if let Some(query) = &self.query {
            let mut scan = |text: &str, offset: usize| {
                for found in query.find_iter(text) {
                    if new_ranges.len() % 1024 == 0 && cancelled() {
                        return false;
                    }
                    new_ranges.push(offset + found.start()..offset + found.end());
                }
                true
            };
            if let Some(text) = self.text.as_str() {
                if !scan(text, 0) {
                    return;
                }
            } else if !query.as_str().contains('\n') && self.text.len_lines(LineType::LF) > 1 {
                let mut buffer = String::with_capacity(MATCH_SCAN_BYTES);
                let mut offset = 0;
                let len = self.text.len();
                while offset < len {
                    if cancelled() {
                        return;
                    }
                    let limit = offset.saturating_add(MATCH_SCAN_BYTES).min(len);
                    let end = if limit == len {
                        len
                    } else {
                        let line = self.text.byte_to_line_idx(limit, LineType::LF);
                        let line_start = self.text.line_to_byte_idx(line, LineType::LF);
                        if line_start == offset {
                            self.text.line_to_byte_idx(line + 1, LineType::LF)
                        } else {
                            line_start
                        }
                    };
                    let slice = self.text.slice(offset..end);
                    let text = if let Some(text) = slice.as_str() {
                        text
                    } else {
                        buffer.clear();
                        buffer.reserve_exact(slice.len());
                        for chunk in slice.chunks() {
                            buffer.push_str(chunk);
                        }
                        &buffer
                    };
                    if !scan(text, offset) {
                        return;
                    }
                    offset = end;
                }
            } else if !scan(&String::from(&self.text), 0) {
                return;
            }
        }
        self.matched_ranges = Arc::new(new_ranges);
        if !self.replacing {
            self.current_match_ix = 0;
        }
        self.current_match_ix = self.current_match_ix.min(self.len().saturating_sub(1));
        self.replacing = false;
    }

    /// Update the search query and reset the current match index.
    pub fn update_query(&mut self, query: &str, case_insensitive: bool, whole_word: bool) {
        let escaped = regex::escape(query);
        let pattern = if whole_word {
            format!(r"\b(?:{escaped})\b")
        } else {
            escaped
        };
        self.query_error = false;
        self.query = if query.is_empty() {
            None
        } else {
            match RegexBuilder::new(&pattern)
                .case_insensitive(case_insensitive)
                .build()
            {
                Ok(query) => Some(query),
                Err(_) => {
                    self.query_error = true;
                    None
                }
            }
        };
        self.update_matches();
    }

    /// Returns the number of matches found.
    #[allow(unused)]
    #[inline]
    fn len(&self) -> usize {
        self.matched_ranges.len()
    }

    fn peek(&self) -> Option<Range<usize>> {
        self.matched_ranges.get(self.current_match_ix + 1)
    }

    fn label(&self) -> String {
        if self.query_error {
            return "Query is too large".into();
        }
        if self.len() == 0 {
            return "0/0".to_string();
        }
        format!("{}/{}", self.current_match_ix + 1, self.len())
    }

    /// Update the current match index based on the given offset.
    fn update_cursor_by_offset(&mut self, offset: usize) {
        self.current_match_ix = self
            .matched_ranges
            .partition_point(|range| range.end < offset)
            .min(self.len().saturating_sub(1));
    }
}

impl Iterator for SearchMatcher {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.matched_ranges.is_empty() {
            return None;
        }

        if self.current_match_ix < self.matched_ranges.len().saturating_sub(1) {
            self.current_match_ix += 1;
        } else {
            self.current_match_ix = 0;
        }

        self.matched_ranges.get(self.current_match_ix)
    }
}

impl DoubleEndedIterator for SearchMatcher {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.matched_ranges.is_empty() {
            return None;
        }

        if self.current_match_ix == 0 {
            self.current_match_ix = self.matched_ranges.len();
        }

        self.current_match_ix -= 1;
        self.matched_ranges.get(self.current_match_ix)
    }
}

struct SearchRequest {
    text: Rope,
    query: String,
    case_insensitive: bool,
    whole_word: bool,
    offset: usize,
    current_match_ix: Option<usize>,
}

pub(super) struct SearchPanel {
    editor: WeakEntity<InputState>,
    search_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    case_insensitive: bool,
    whole_word: bool,
    replace_mode: bool,
    matcher: SearchMatcher,

    open: bool,
    scan_running: bool,
    pending_scan: Option<SearchRequest>,
    cancellation: Arc<AtomicBool>,
    searching: bool,
    /// Width of the panel in the last frame, which decides whether the options share the
    /// query row.
    width: gpui::Pixels,
    /// Natural widths of the controls after the query and replacement fields in the last
    /// frame; both rows reserve the larger so the two fields share a right edge.
    trailing: [gpui::Pixels; 2],
    _subscriptions: Vec<Subscription>,
}

impl InputState {
    /// Update the search matcher when text changes.
    pub(super) fn update_search(&mut self, cx: &mut App) {
        let Some(search_panel) = self.search_panel.as_ref() else {
            return;
        };

        let text = self.text.clone();
        let offset = self.cursor();
        search_panel.update(cx, |this, cx| {
            if this.open {
                this.request_search(text, offset, cx);
            }
        });
    }

    pub(super) fn on_action_search(
        &mut self,
        _: &Search,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(window, cx);
    }

    pub fn show_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.searchable {
            return;
        }

        let search_panel = match self.search_panel.as_ref() {
            Some(panel) => panel.clone(),
            None => SearchPanel::new(cx.entity(), window, cx),
        };

        let text = self.text.clone();
        let offset = self.cursor();
        let editor = cx.entity();
        let selected_text = Rope::from(self.selected_text());
        search_panel.update(cx, |this, cx| {
            this.editor = editor.downgrade();
            this.show(&selected_text, window, cx);
            this.request_search(text, offset, cx);
        });
        self.search_panel = Some(search_panel);
        cx.notify();
    }
}

impl SearchPanel {
    pub fn new(editor: Entity<InputState>, window: &mut Window, cx: &mut App) -> Entity<Self> {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in document"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with"));

        cx.new(|cx| {
            let _subscriptions =
                vec![
                    cx.subscribe(&search_input, |this: &mut Self, _, ev: &InputEvent, cx| {
                        // Handle search input changes
                        match ev {
                            InputEvent::Change => {
                                this.update_search_query(cx);
                            }
                            _ => {}
                        }
                    }),
                ];

            Self {
                editor: editor.downgrade(),
                search_input,
                replace_input,
                case_insensitive: true,
                whole_word: false,
                replace_mode: false,
                matcher: SearchMatcher::new(),
                open: true,
                scan_running: false,
                pending_scan: None,
                cancellation: Arc::new(AtomicBool::new(false)),
                width: gpui::Pixels::ZERO,
                trailing: [gpui::Pixels::ZERO; 2],
                searching: false,
                _subscriptions,
            }
        })
    }

    pub(super) fn show(
        &mut self,
        selected_text: &Rope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open = true;
        self.search_input.read(cx).focus_handle.focus(window);

        self.search_input.update(cx, |this, cx| {
            if selected_text.len() > 0 {
                // Set value will emit to update_search_query
                this.set_value(selected_text.to_string(), window, cx);
            }
            this.select_all(&super::SelectAll, window, cx);
        });
    }

    fn update_search_query(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let editor = editor.read(cx);
        let text = editor.text.clone();
        let offset = editor.cursor();
        self.request_search(text, offset, cx);
    }

    fn request_search(&mut self, text: Rope, offset: usize, cx: &mut Context<Self>) {
        self.cancellation.store(true, Ordering::Relaxed);
        self.pending_scan = Some(SearchRequest {
            text,
            query: self.search_input.read(cx).value().to_string(),
            case_insensitive: self.case_insensitive,
            whole_word: self.whole_word,
            offset,
            current_match_ix: self
                .matcher
                .replacing
                .then_some(self.matcher.current_match_ix),
        });
        self.matcher.matched_ranges = Arc::new(MatchRanges::new(0));
        self.matcher.replacing = false;
        self.searching = true;
        self.start_search(cx);
        cx.notify();
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        if self.scan_running {
            return;
        }
        let Some(request) = self.pending_scan.take() else {
            return;
        };
        self.scan_running = true;
        let cancellation = Arc::new(AtomicBool::new(false));
        self.cancellation = cancellation.clone();
        let task = cx.background_spawn(async move {
            let mut matcher = SearchMatcher::new();
            matcher.text = request.text;
            matcher.cancellation = Some(cancellation.clone());
            matcher.update_query(&request.query, request.case_insensitive, request.whole_word);
            if cancellation.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(index) = request.current_match_ix {
                matcher.current_match_ix = index.min(matcher.len().saturating_sub(1));
            } else {
                matcher.update_cursor_by_offset(request.offset);
            }
            matcher.cancellation = None;
            Some(matcher)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            _ = this.update(cx, |this, cx| {
                this.scan_running = false;
                if this.open
                    && this.pending_scan.is_none()
                    && !this.cancellation.load(Ordering::Relaxed)
                {
                    if let Some(matcher) = result {
                        this.matcher = matcher;
                        this.searching = false;
                    }
                }
                this.start_search(cx);
                let _ = this.editor.update(cx, |_, cx| cx.notify());
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        self.cancellation.store(true, Ordering::Relaxed);
        self.pending_scan = None;
        self.searching = false;
        if let Some(range) = self
            .matcher
            .matched_ranges
            .get(self.matcher.current_match_ix)
        {
            let _ = self.editor.update(cx, |state, cx| {
                state.selected_range = range.clone().into();
                state.selection_reversed = false;
                state.scroll_to(range.end, Some(MoveDirection::Down), cx);
                cx.notify();
            });
        }
        if let Some(editor) = self.editor.upgrade() {
            editor.read(cx).focus_handle.focus(window);
        }
        cx.notify();
    }

    fn on_action_prev(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.prev(window, cx);
    }

    fn on_action_next(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_input.read(cx).focus_handle.is_focused(window)
            || self.replace_input.read(cx).focus_handle.is_focused(window)
        {
            self.next(window, cx);
        } else {
            cx.propagate();
        }
    }

    fn on_action_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        self.hide(window, cx);
    }

    fn on_action_tab(&mut self, _: &IndentInline, window: &mut Window, _: &mut Context<Self>) {
        window.focus_next();
    }

    fn on_action_tab_prev(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.focus_prev();
    }

    fn toggle_case(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.case_insensitive = !self.case_insensitive;
        self.update_search_query(cx);
    }

    fn toggle_whole_word(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.whole_word = !self.whole_word;
        self.update_search_query(cx);
    }

    fn toggle_replace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_mode = !self.replace_mode;
        let input = if self.replace_mode {
            &self.replace_input
        } else {
            &self.search_input
        };
        input.read(cx).focus_handle.focus(window);
        cx.notify();
    }

    fn compact_options(&self, cx: &Context<Self>) -> impl IntoElement {
        type Command = fn(&mut SearchPanel, &mut Window, &mut Context<SearchPanel>);
        let commands: [(&str, Command, bool, bool); 5] = [
            (
                "Match case",
                Self::toggle_case,
                !self.case_insensitive,
                false,
            ),
            (
                "Match whole words",
                Self::toggle_whole_word,
                self.whole_word,
                false,
            ),
            (
                if self.replace_mode {
                    "Hide replacement"
                } else {
                    "Show replacement"
                },
                Self::toggle_replace,
                false,
                false,
            ),
            ("Previous match", Self::prev, false, self.matcher.len() == 0),
            ("Next match", Self::next, false, self.matcher.len() == 0),
        ];
        let panel = cx.entity().downgrade();
        Button::new("find-options")
            .small()
            .ghost()
            .icon(IconName::Ellipsis)
            .tooltip("Find options")
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu.scrollable(true).max_h(
                    (window.viewport_size().height - window.rem_size() * 5.)
                        .max(window.rem_size() * 4.),
                );
                for (label, command, checked, disabled) in commands {
                    let panel = panel.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(checked)
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                let panel = panel.clone();
                                window.defer(cx, move |window, cx| {
                                    let _ =
                                        panel.update(cx, |panel, cx| command(panel, window, cx));
                                });
                            }),
                    );
                }
                menu
            })
    }

    fn prev(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(range) = self.matcher.next_back() {
            let _ = self.editor.update(cx, |state, cx| {
                state.selected_range = range.clone().into();
                state.selection_reversed = false;
                state.scroll_to(range.start, Some(MoveDirection::Up), cx);
                cx.notify();
            });
        }
    }

    fn next(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(range) = self.matcher.next() {
            let _ = self.editor.update(cx, |state, cx| {
                state.selected_range = range.clone().into();
                state.selection_reversed = false;
                state.scroll_to(range.end, Some(MoveDirection::Down), cx);
                cx.notify();
            });
        }
    }

    pub(super) fn matcher(&self) -> Option<&SearchMatcher> {
        if !self.open {
            return None;
        }

        Some(&self.matcher)
    }

    fn replace_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_text = self.replace_input.read(cx).value();
        self.matcher.replacing = true;
        if let Some(range) = self
            .matcher
            .matched_ranges
            .get(self.matcher.current_match_ix)
        {
            let Some(editor) = self.editor.upgrade() else {
                return;
            };
            let original = editor.read(cx).text.clone();
            let text_state = self.editor.clone();

            let next_range = self.matcher.peek().unwrap_or(range.clone());
            cx.spawn_in(window, async move |_, cx| {
                cx.update(|window, cx| {
                    let _ = text_state.update(cx, |state, cx| {
                        if state.text != original {
                            return;
                        }
                        let range_utf16 = state.range_to_utf16(&range);
                        state.scroll_to(next_range.end, Some(MoveDirection::Down), cx);
                        state.history.new_group();
                        state.replace_text_in_range_silent(
                            Some(range_utf16),
                            new_text.as_str(),
                            window,
                            cx,
                        );
                        state.history.new_group();
                    });
                })
            })
            .detach();
        }
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let new_text = self.replace_input.read(cx).value();
        let ranges = self.matcher.matched_ranges.clone();
        if ranges.is_empty() {
            return;
        }

        let Some(editor) = self.editor.upgrade() else {
            return;
        };
        let state = editor.read(cx);
        if !ropes_are_instances(&state.text, &self.matcher.text) {
            return;
        }
        let original = state.text.clone();
        let layout = state.replacement_layout();
        let viewport = state.last_layout.as_ref().map(|layout| {
            let rows =
                (f32::from(layout.viewport_height) / f32::from(layout.line_height)).ceil() as usize;
            (
                window.text_system().prepare_in_separate_cache(),
                rows.saturating_add(2),
            )
        });
        let editor = self.editor.clone();
        let cancellation = self.cancellation.clone();
        let worker_cancel = cancellation.clone();
        let worker_original = original.clone();
        let text_system = cx.text_system().clone();
        self.matcher.replacing = true;
        self.searching = true;
        self.matcher.matched_ranges = Arc::new(MatchRanges::new(0));
        let task = cx.background_spawn(async move {
            let output =
                replace_matches(&worker_original, &ranges, new_text.as_str(), &worker_cancel)?;
            if worker_cancel.load(Ordering::Relaxed) {
                return None;
            }
            let prepared = layout.map(|layout| {
                PreparedReplacement::new(output.clone(), layout, &text_system)
                    .with_viewport(viewport)
            });
            (!worker_cancel.load(Ordering::Relaxed)).then_some((output, prepared))
        });
        cx.spawn_in(window, async move |_, cx| {
            let Some((replaced, prepared)) = task.await else {
                return;
            };
            _ = cx.update(|window, cx| {
                let _ = editor.update(cx, |state, cx| {
                    if cancellation.load(Ordering::Relaxed) || state.text != original {
                        return;
                    }
                    state.history.new_group();
                    if !prepared
                        .is_some_and(|edit| state.apply_prepared_replacement(edit, window, cx))
                    {
                        let full_range = state.range_to_utf16(&(0..state.text.len()));
                        state.replace_text_in_range_silent(
                            Some(full_range),
                            &replaced.to_string(),
                            window,
                            cx,
                        );
                    }
                    state.history.new_group();
                    state.scroll_to(0, Some(MoveDirection::Down), cx);
                });
            });
        })
        .detach();
    }
}

impl Focusable for SearchPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search_input.read(cx).focus_handle.clone()
    }
}

impl SearchPanel {
    fn close_button(cx: &mut Context<Self>) -> Button {
        Button::new("close")
            .small()
            .ghost()
            .flex_shrink_0()
            .icon(IconName::Close)
            .tooltip("Close Find")
            .on_click(cx.listener(|this, _, window, cx| {
                this.on_action_escape(&Escape, window, cx);
            }))
    }
}

impl Render for SearchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return Empty.into_any_element();
        }

        let has_matches = self.matcher.len() > 0;
        let match_label: gpui::SharedString = if self.searching {
            "Searching…".into()
        } else if has_matches {
            self.matcher.label().into()
        } else if self.search_input.read(cx).value().is_empty() {
            "".into()
        } else {
            "No matches".into()
        };

        let viewport = window.viewport_size();
        let rem = window.rem_size();
        let compact = viewport.height < rem * 16. || viewport.width < rem * 28.;
        let close_query = compact.then(|| Self::close_button(cx));
        let close_options = (!compact).then(|| Self::close_button(cx));
        let count = |compact| {
            crate::semantic_text::text(
                "match-count",
                match_label.clone(),
                Label::new(match_label.clone())
                    .when(!has_matches, |label| {
                        label.text_color(cx.theme().muted_foreground)
                    })
                    .text_left()
                    .when(compact, |label| label.max_w(rems(4.5)).truncate())
                    .when(!compact, |label| label.min_w_16()),
            )
        };

        // Rows never wrap: the options share the query row only when the panel is wide enough
        // for both, measured from the previous frame, and the replace row always fits.
        let one_row = !compact && self.width >= rem * 38.;
        let options = (!compact).then(|| {
            h_flex()
                .flex_shrink_0()
                .gap_2()
                .when(!one_row, |row| row.w_full())
                .child(
                    Button::new("case-sensitive")
                        .small()
                        .map(|button| {
                            if self.case_insensitive {
                                button.ghost()
                            } else {
                                button
                                    .ghost()
                                    .bg(cx.theme().selection)
                                    .text_color(cx.theme().ring)
                            }
                        })
                        .icon(IconName::CaseSensitive)
                        .tooltip("Match case")
                        .accessibility_toggled(!self.case_insensitive)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_case(window, cx);
                        })),
                )
                .child(
                    Button::new("whole-word")
                        .small()
                        .map(|button| {
                            if self.whole_word {
                                button
                                    .ghost()
                                    .bg(cx.theme().selection)
                                    .text_color(cx.theme().ring)
                            } else {
                                button.ghost()
                            }
                        })
                        .icon(IconName::WholeWord)
                        .tooltip("Match whole words")
                        .accessibility_toggled(self.whole_word)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_whole_word(window, cx);
                        })),
                )
                .child(
                    Button::new("replace-mode")
                        .small()
                        .map(|button| {
                            if self.replace_mode {
                                button
                                    .ghost()
                                    .bg(cx.theme().selection)
                                    .text_color(cx.theme().ring)
                            } else {
                                button.ghost()
                            }
                        })
                        .icon(IconName::Replace)
                        .tooltip(if self.replace_mode {
                            "Hide replacement"
                        } else {
                            "Show replacement"
                        })
                        .accessibility_expanded(self.replace_mode)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.toggle_replace(window, cx);
                        })),
                )
                .child(
                    Button::new("prev")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronLeft)
                        .tooltip("Previous match")
                        .disabled(!has_matches)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.prev(window, cx);
                        })),
                )
                .child(
                    Button::new("next")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronRight)
                        .tooltip("Next match")
                        .disabled(!has_matches)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.next(window, cx);
                        })),
                )
                .child(count(false))
                .when(!one_row, |row| row.child(div().flex_1()))
                .children(close_options)
        });
        let (inline_options, row_options) = if one_row {
            (options, None)
        } else {
            (None, options)
        };
        let align = one_row && self.replace_mode;
        let reserved = self.trailing[0].max(self.trailing[1]);
        let measured = cx.entity().downgrade();
        let trailing = |index: usize, controls: gpui::AnyElement| {
            let panel = measured.clone();
            div()
                .flex()
                .justify_end()
                .flex_shrink_0()
                .when(align, |slot| slot.min_w(reserved))
                .child(
                    div().relative().child(controls).child(
                        gpui::canvas(
                            move |bounds, window, cx| {
                                let _ = panel.update(cx, |panel, _| {
                                    if (panel.trailing[index] - bounds.size.width).abs() > px(0.5) {
                                        panel.trailing[index] = bounds.size.width;
                                        window.request_animation_frame();
                                    }
                                });
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    ),
                )
        };
        let inline_options = inline_options.map(|options| trailing(0, options.into_any_element()));
        let replace_controls = trailing(
            1,
            h_flex()
                .gap_2()
                .child(
                    Button::new("replace-one")
                        .when(compact, |button| button.xsmall())
                        .when(!compact, |button| button.small())
                        .flex_shrink_0()
                        .label(t!("Input.Replace"))
                        .disabled(!has_matches)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.replace_next(window, cx);
                        })),
                )
                .child(
                    Button::new("replace-all")
                        .when(compact, |button| button.xsmall())
                        .when(!compact, |button| button.small())
                        .flex_shrink_0()
                        .label(t!("Input.Replace All"))
                        .disabled(!has_matches)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.replace_all(window, cx);
                        })),
                )
                .into_any_element(),
        );
        let panel = cx.entity().downgrade();
        v_flex()
            .id("search-panel")
            .relative()
            .tab_group()
            .occlude()
            .track_focus(&self.focus_handle(cx))
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_prev))
            .on_action(cx.listener(Self::on_action_next))
            .on_action(cx.listener(Self::on_action_escape))
            .on_action(cx.listener(Self::on_action_tab))
            .on_action(cx.listener(Self::on_action_tab_prev))
            .font_family(cx.theme().font_family.clone())
            .items_center()
            .when(compact, |panel| panel.py(px(2.)).px_2())
            .when(!compact, |panel| panel.py_2().px_3())
            .w_full()
            .gap_1()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .child(
                gpui::canvas(
                    move |bounds, window, cx| {
                        let _ = panel.update(cx, |panel, _| {
                            if (panel.width - bounds.size.width).abs() > px(0.5) {
                                panel.width = bounds.size.width;
                                window.request_animation_frame();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(compact, |query| query.gap_1())
                    .child(
                        Input::new(&self.search_input)
                            .small()
                            .flex_1()
                            .min_w_0()
                            .shadow_none(),
                    )
                    .when(compact, |row| {
                        row.child(count(true)).child(self.compact_options(cx))
                    })
                    .children(close_query)
                    .children(inline_options),
            )
            .children(row_options)
            .when(self.replace_mode, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Input::new(&self.replace_input)
                                .small()
                                .flex_1()
                                .min_w_0()
                                .shadow_none(),
                        )
                        .child(replace_controls),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn streamed_replacement_keeps_precise_unicode_and_line_endings() {
        use super::{AtomicBool, MatchRanges, Regex, Rope, replace_matches};

        let text = format!(
            "{}\r\n{}",
            "🐋a".repeat(209),
            "K café\r\nk CAFÉ\nplain\r".repeat(1200)
        );
        let source = Rope::from(text.as_str());
        let pattern = Regex::new("(?i)k café").unwrap();
        let mut ranges = MatchRanges::new(text.len());
        for found in pattern.find_iter(&text) {
            ranges.push(found.range());
        }
        for replacement in ["", "🦀$5\r\n"] {
            let expected = pattern.replace_all(&text, regex::NoExpand(replacement));
            assert_eq!(
                replace_matches(&source, &ranges, replacement, &AtomicBool::new(false))
                    .unwrap()
                    .to_string(),
                expected
            );
        }
        assert!(replace_matches(&source, &ranges, "ignored", &AtomicBool::new(true)).is_none());
    }

    struct SearchFixture {
        editor: gpui::Entity<super::InputState>,
    }

    impl gpui::Render for SearchFixture {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            super::Input::new(&self.editor)
        }
    }

    #[gpui::test]
    fn find_controls_preserve_keyboard_navigation_and_return_focus(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let mut editor = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| {
                super::InputState::new(window, cx)
                    .multi_line(true)
                    .default_value("    unchanged source")
                    .searchable(true)
            });
            editor = Some(input.clone());
            let fixture = cx.new(|_| SearchFixture { editor: input });
            crate::Root::new(fixture, window, cx)
        });
        let editor = editor.unwrap();
        cx.update(|window, cx| {
            editor.update(cx, |state, cx| state.show_search(window, cx));
        });
        cx.run_until_parked();
        let query = cx.update(|window, cx| window.focused(cx));
        assert!(query.is_some());
        cx.simulate_keystrokes("tab");
        let case = cx.update(|window, cx| window.focused(cx));
        assert!(case.is_some());
        assert_ne!(case, query);
        cx.simulate_keystrokes("tab");
        let word = cx.update(|window, cx| window.focused(cx));
        assert!(word.is_some());
        assert_ne!(word, case);
        cx.simulate_keystrokes("tab");
        let replace = cx.update(|window, cx| window.focused(cx));
        assert!(replace.is_some());
        assert_ne!(replace, word);
        cx.simulate_keystrokes("tab");
        let close = cx.update(|window, cx| window.focused(cx));
        assert!(close.is_some());
        assert_ne!(close, replace);
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), replace);
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), word);
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), case);
        cx.simulate_keystrokes("shift-tab");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), query);
        editor.read_with(cx, |state, cx| {
            assert_eq!(state.value(), "    unchanged source");
            let panel = state.search_panel.as_ref().unwrap().read(cx);
            assert_eq!(panel.search_input.read(cx).value(), "");
        });
        cx.simulate_input("unchanged");
        editor.read_with(cx, |state, cx| {
            assert_eq!(
                state.search_panel.as_ref().unwrap().read(cx).matcher.len(),
                1
            );
        });
        cx.simulate_keystrokes("tab tab tab");
        assert_eq!(cx.update(|window, cx| window.focused(cx)), replace);
        cx.simulate_keystrokes("space");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("space").unwrap(),
        });
        assert!(cx.update(|window, cx| {
            editor
                .read(cx)
                .search_panel
                .as_ref()
                .unwrap()
                .read(cx)
                .replace_input
                .read(cx)
                .focus_handle
                .is_focused(window)
        }));
        cx.simulate_input("updated");
        cx.simulate_keystrokes("tab tab space");
        cx.simulate_event(gpui::KeyUpEvent {
            keystroke: gpui::Keystroke::parse("space").unwrap(),
        });
        editor.read_with(cx, |state, _| {
            assert_eq!(state.value(), "    updated source")
        });
        cx.simulate_keystrokes("escape");
        editor.read_with(cx, |state, cx| {
            assert!(!state.search_panel.as_ref().unwrap().read(cx).open);
        });
        assert!(cx.update(|window, cx| editor.read(cx).focus_handle.is_focused(window)));
    }

    #[gpui::test]
    fn closing_an_input_releases_its_menu_and_search_fields(cx: &mut gpui::TestAppContext) {
        cx.update(crate::init);
        let cx = cx.add_empty_window();
        for with_search in [false, true] {
            let (editor, menu, search_fields) = cx.update(|window, cx| {
                let editor = cx.new(|cx| {
                    super::InputState::new(window, cx)
                        .multi_line(true)
                        .default_value("document text")
                        .searchable(true)
                });
                if with_search {
                    editor.update(cx, |state, cx| state.show_search(window, cx));
                }
                let state = editor.read(cx);
                let menu = state.mouse_context_menu.downgrade();
                let search_fields = state.search_panel.as_ref().map(|panel| {
                    let search = panel.read(cx);
                    (
                        panel.downgrade(),
                        search.search_input.downgrade(),
                        search.replace_input.downgrade(),
                    )
                });
                (editor.downgrade(), menu, search_fields)
            });
            cx.run_until_parked();
            assert!(editor.upgrade().is_none());
            assert!(menu.upgrade().is_none());
            if let Some((panel, query, replacement)) = search_fields {
                assert!(panel.upgrade().is_none());
                assert!(query.upgrade().is_none());
                assert!(replacement.upgrade().is_none());
            }
        }
    }
    use super::*;

    #[test]
    fn test_search() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("Hello 世界 this is a Is test string."));
        matcher.update_query("Is", true, false);

        assert_eq!(matcher.len(), 3);
        let mut matches = matcher.clone();
        assert_eq!(matches.current_match_ix, 0);
        assert_eq!(matches.next(), Some(18..20));
        assert_eq!(matches.next(), Some(23..25));
        assert_eq!(matches.current_match_ix, 2);
        assert_eq!(matches.next(), Some(15..17));
        assert_eq!(matches.current_match_ix, 0);
        assert_eq!(matches.next_back(), Some(23..25));
        assert_eq!(matches.current_match_ix, 2);
        assert_eq!(matches.next_back(), Some(18..20));
        assert_eq!(matches.current_match_ix, 1);
        assert_eq!(matches.next_back(), Some(15..17));
        assert_eq!(matches.current_match_ix, 0);
        assert_eq!(matches.next_back(), Some(23..25));

        matcher.update_query("IS", false, false);
        assert_eq!(matcher.len(), 0);
        assert_eq!(matcher.next(), None);
        assert_eq!(matcher.next_back(), None);
    }

    #[test]
    fn unicode_case_and_whole_word_ranges() {
        let text = "🙂 CAFÉ café décafé café_ café\r\n";
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from(text));
        matcher.update_query("café", true, true);
        let matches: Vec<_> = matcher
            .matched_ranges
            .iter()
            .map(|range| &text[range])
            .collect();
        assert_eq!(matches, ["CAFÉ", "café", "café"]);
        matcher.update_query("café", false, false);
        assert_eq!(matcher.len(), 4);
        matcher.current_match_ix = 3;
        matcher.replacing = true;
        matcher.update(&Rope::from("café"));
        assert_eq!(matcher.label(), "1/1");
        matcher.update_query("[", true, false);
        assert_eq!(matcher.len(), 0);
    }

    #[test]
    fn bounded_scans_preserve_contiguous_regex_results() {
        let short = "🙂 K K CAFÉ café_ é\r\na\nb aaabaa\n[ \r café\u{2028}one\n";
        let long = "🌱café K\r".repeat(8500);
        let text = format!("{}\n{long}\n{short}\n{long}", short.repeat(1800));
        let mut matcher = SearchMatcher::new();
        matcher.text = Rope::from(text.as_str());
        for literal in [
            "café", "K", "\r", "a\nb", "\n", "é\r\n", "\u{2028}", "[", "aa",
        ] {
            for case_insensitive in [false, true] {
                for whole_word in [false, true] {
                    matcher.update_query(literal, case_insensitive, whole_word);
                    let expected: Vec<_> = matcher
                        .query
                        .as_ref()
                        .unwrap()
                        .find_iter(&text)
                        .map(|found| found.range())
                        .collect();
                    assert_eq!(
                        matcher.matched_ranges.iter().collect::<Vec<_>>(),
                        expected,
                        "{literal:?}, {case_insensitive}, {whole_word}"
                    );
                }
            }
        }
    }

    #[test]
    fn different_length_unicode_matches_keep_exact_offsets() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("K k K"));
        matcher.update_query("k", true, false);
        assert!(matches!(
            &*matcher.matched_ranges,
            MatchRanges::Uniform { length: 1, .. }
        ));
        matcher.update(&Rope::from("K K k"));
        assert!(matches!(
            &*matcher.matched_ranges,
            MatchRanges::Variable { .. }
        ));
        assert_eq!(
            matcher.matched_ranges.iter().collect::<Vec<_>>(),
            [0..1, 2..5, 6..7]
        );
        assert_eq!(matcher.matched_ranges.intersecting(&(4..6)), 1..2);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn large_offsets_use_wide_ranges() {
        let offset = u32::MAX as usize + 10;
        let mut matches = MatchRanges::new(offset + 10);
        matches.push(offset..offset + 2);
        matches.push(offset + 5..offset + 8);
        assert_eq!(matches.get(1), Some(offset + 5..offset + 8));
        assert_eq!(matches.intersecting(&(offset + 1..offset + 3)), 0..1);
    }

    #[test]
    fn cancelled_dense_search_never_publishes_partial_matches() {
        let mut matcher = SearchMatcher::new();
        matcher.text = Rope::from("🙂 café ".repeat(100_000));
        let cancel = Arc::new(AtomicBool::new(true));
        matcher.cancellation = Some(cancel.clone());
        matcher.update_query("café", true, false);
        assert!(matcher.matched_ranges.is_empty());
        cancel.store(false, Ordering::Relaxed);
        matcher.update_query("café", true, false);
        assert_eq!(matcher.len(), 100_000);
        assert_eq!(matcher.matched_ranges.get(0), Some(5..10));
        matcher.update_cursor_by_offset(1_099_999);
        assert_eq!(matcher.current_match_ix, 99_999);
    }

    #[test]
    fn test_search_label() {
        let mut matcher = SearchMatcher::new();
        matcher.update(&Rope::from("Hello 世界 this is a Is test string."));
        matcher.update_query("Is", true, false);
        assert_eq!(matcher.label(), "1/3");
        matcher.next();
        assert_eq!(matcher.label(), "2/3");
        matcher.next();
        assert_eq!(matcher.label(), "3/3");
        matcher.next();
        assert_eq!(matcher.label(), "1/3");

        matcher.update_query("IS", false, false);
        assert_eq!(matcher.label(), "0/0");
    }

    #[test]
    fn test_select_range_start() {
        let mut matcher = SearchMatcher::new();
        let mut ranges = MatchRanges::new(30);
        for range in [5..10, 15..20, 25..30] {
            ranges.push(range);
        }
        matcher.matched_ranges = Arc::new(ranges);
        matcher.update_cursor_by_offset(0);
        assert_eq!(matcher.current_match_ix, 0);

        matcher.update_cursor_by_offset(5);
        assert_eq!(matcher.current_match_ix, 0);

        matcher.update_cursor_by_offset(12);
        assert_eq!(matcher.current_match_ix, 1);

        matcher.update_cursor_by_offset(16);
        assert_eq!(matcher.current_match_ix, 1);

        matcher.update_cursor_by_offset(30);
        assert_eq!(matcher.current_match_ix, 2);

        matcher.update_cursor_by_offset(31);
        assert_eq!(matcher.current_match_ix, 2);
    }
}
