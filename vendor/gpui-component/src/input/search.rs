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
    App, AppContext as _, Context, Empty, Entity, FocusHandle, Focusable, Half,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Render, Styled,
    Subscription, Window, actions, canvas, div, prelude::FluentBuilder as _,
};
use ropey::Rope;

use crate::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable,
    actions::SelectUp,
    button::{Button, ButtonVariants},
    h_flex,
    input::{
        Enter, Escape, IndentInline, Input, InputEvent, InputState, Search, movement::MoveDirection,
    },
    label::Label,
    v_flex,
};

const CONTEXT: &'static str = "SearchPanel";

actions!(input, [Tab]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys(vec![KeyBinding::new(
        "shift-enter",
        SelectUp,
        Some(CONTEXT),
    )]);
}

#[derive(Debug, Clone)]
pub struct SearchMatcher {
    text: Rope,
    pub query: Option<Regex>,
    query_error: bool,

    pub(super) matched_ranges: Arc<Vec<Range<usize>>>,
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
            matched_ranges: Arc::new(Vec::new()),
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
        if self
            .cancellation
            .as_ref()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
        {
            return;
        }
        let mut new_ranges = Vec::new();
        if let Some(query) = &self.query {
            let text = self.text.to_string();
            for found in query.find_iter(&text) {
                if new_ranges.len() % 1024 == 0
                    && self
                        .cancellation
                        .as_ref()
                        .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
                {
                    return;
                }
                new_ranges.push(found.range());
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
        self.matched_ranges.get(self.current_match_ix + 1).cloned()
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

        self.matched_ranges.get(self.current_match_ix).cloned()
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
        let item = self.matched_ranges[self.current_match_ix].clone();

        Some(item)
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
    editor: Entity<InputState>,
    search_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    case_insensitive: bool,
    whole_word: bool,
    replace_mode: bool,
    matcher: SearchMatcher,
    input_width: Pixels,

    open: bool,
    scan_running: bool,
    pending_scan: Option<SearchRequest>,
    cancellation: Arc<AtomicBool>,
    searching: bool,
    _subscriptions: Vec<Subscription>,
}

impl InputState {
    /// Update the search matcher when text changes.
    pub(super) fn update_search(&mut self, cx: &mut App) {
        let Some(search_panel) = self.search_panel.as_ref() else {
            return;
        };

        let text = self.text.clone();
        let offset = self
            .last_layout
            .as_ref()
            .map_or(0, |layout| layout.visible_range_offset.start);
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
        if !self.searchable {
            return;
        }

        let search_panel = match self.search_panel.as_ref() {
            Some(panel) => panel.clone(),
            None => SearchPanel::new(cx.entity(), window, cx),
        };

        let text = self.text.clone();
        let offset = self
            .last_layout
            .as_ref()
            .map_or(0, |layout| layout.visible_range_offset.start);
        let editor = cx.entity();
        let selected_text = Rope::from(self.selected_text());
        search_panel.update(cx, |this, cx| {
            this.editor = editor;
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
                editor,
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
                searching: false,
                input_width: Pixels::ZERO,
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
        let editor = self.editor.read(cx);
        let text = editor.text.clone();
        let offset = editor
            .last_layout
            .as_ref()
            .map_or(0, |layout| layout.visible_range_offset.start);
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
        self.matcher.matched_ranges = Arc::new(Vec::new());
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
                this.editor.update(cx, |_, cx| cx.notify());
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
            self.editor.update(cx, |state, cx| {
                state.selected_range = range.clone().into();
                state.selection_reversed = false;
                state.scroll_to(range.end, Some(MoveDirection::Down), cx);
                cx.notify();
            });
        }
        self.editor.read(cx).focus_handle.focus(window);
        cx.notify();
    }

    fn on_action_prev(&mut self, _: &SelectUp, window: &mut Window, cx: &mut Context<Self>) {
        self.prev(window, cx);
    }

    fn on_action_next(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        self.next(window, cx);
    }

    fn on_action_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        self.hide(window, cx);
    }

    fn on_action_tab(&mut self, _: &IndentInline, window: &mut Window, _: &mut Context<Self>) {
        window.focus_next();
    }

    fn prev(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(range) = self.matcher.next_back() {
            self.editor.update(cx, |state, cx| {
                state.selected_range = range.clone().into();
                state.selection_reversed = false;
                state.scroll_to(range.start, Some(MoveDirection::Up), cx);
                cx.notify();
            });
        }
    }

    fn next(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(range) = self.matcher.next() {
            self.editor.update(cx, |state, cx| {
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
            .cloned()
        {
            let text_state = self.editor.clone();
            let original = text_state.read(cx).text.clone();

            let next_range = self.matcher.peek().unwrap_or(range.clone());
            cx.spawn_in(window, async move |_, cx| {
                cx.update(|window, cx| {
                    text_state.update(cx, |state, cx| {
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
        self.matcher.replacing = true;
        let ranges = self.matcher.matched_ranges.clone();
        if ranges.is_empty() {
            return;
        }

        let editor = self.editor.clone();
        let original = editor.read(cx).text.clone();
        let cancellation = self.cancellation.clone();
        let worker_cancel = cancellation.clone();
        let worker_original = original.clone();
        self.searching = true;
        self.matcher.matched_ranges = Arc::new(Vec::new());
        let task = cx.background_spawn(async move {
            let text = worker_original.to_string();
            let mut output = String::new();
            let mut offset = 0;
            for (index, range) in ranges.iter().enumerate() {
                if index % 1024 == 0 && worker_cancel.load(Ordering::Relaxed) {
                    return None;
                }
                output.push_str(&text[offset..range.start]);
                output.push_str(new_text.as_str());
                offset = range.end;
            }
            output.push_str(&text[offset..]);
            (!worker_cancel.load(Ordering::Relaxed)).then_some(output)
        });
        cx.spawn_in(window, async move |_, cx| {
            let Some(replaced) = task.await else {
                return;
            };
            _ = cx.update(|window, cx| {
                editor.update(cx, |state, cx| {
                    if cancellation.load(Ordering::Relaxed) || state.text != original {
                        return;
                    }
                    let full_range = state.range_to_utf16(&(0..state.text.len()));
                    state.history.new_group();
                    state.replace_text_in_range_silent(Some(full_range), &replaced, window, cx);
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

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return Empty.into_any_element();
        }

        let has_matches = self.matcher.len() > 0;
        let match_label = if self.searching {
            "Searching…".into()
        } else {
            self.matcher.label()
        };

        v_flex()
            .id("search-panel")
            .occlude()
            .track_focus(&self.focus_handle(cx))
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_prev))
            .on_action(cx.listener(Self::on_action_next))
            .on_action(cx.listener(Self::on_action_escape))
            .on_action(cx.listener(Self::on_action_tab))
            .font_family(cx.theme().font_family.clone())
            .items_center()
            .py_2()
            .px_3()
            .w_full()
            .gap_1()
            .bg(cx.theme().popover)
            .border_b_1()
            .rounded(cx.theme().radius.half())
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .gap_1()
                            .child(
                                Input::new(&self.search_input)
                                    .focus_bordered(false)
                                    .suffix(
                                        Button::new("case-insensitive")
                                            .selected(!self.case_insensitive)
                                            .xsmall()
                                            .compact()
                                            .ghost()
                                            .icon(IconName::CaseSensitive)
                                            .tooltip("Match case")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.case_insensitive = !this.case_insensitive;
                                                this.update_search_query(cx);
                                                cx.notify();
                                            })),
                                    )
                                    .small()
                                    .w_full()
                                    .shadow_none(),
                            )
                            .child(
                                canvas(
                                    {
                                        let view = cx.entity();
                                        move |bounds, _, cx| {
                                            view.update(cx, |r, _| {
                                                r.input_width = bounds.size.width
                                            })
                                        }
                                    },
                                    |_, _, _, _| {},
                                )
                                .absolute()
                                .size_full(),
                            ),
                    )
                    .child(
                        Button::new("whole-word")
                            .xsmall()
                            .ghost()
                            .label("Word")
                            .tooltip("Match whole words")
                            .selected(self.whole_word)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.whole_word = !this.whole_word;
                                this.update_search_query(cx);
                            })),
                    )
                    .child(
                        Button::new("replace-mode")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Replace)
                            .tooltip("Show replacement")
                            .selected(self.replace_mode)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.replace_mode = !this.replace_mode;
                                if this.replace_mode {
                                    this.replace_input.read(cx).focus_handle.focus(window);
                                } else {
                                    this.search_input.read(cx).focus_handle.focus(window);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("prev")
                            .xsmall()
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
                            .xsmall()
                            .ghost()
                            .icon(IconName::ChevronRight)
                            .tooltip("Next match")
                            .disabled(!has_matches)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.next(window, cx);
                            })),
                    )
                    .child(crate::semantic_text::text(
                        "match-count",
                        match_label.clone().into(),
                        Label::new(match_label)
                            .when(!has_matches, |this| {
                                this.text_color(cx.theme().muted_foreground)
                            })
                            .text_left()
                            .min_w_16(),
                    ))
                    .child(div().w_7())
                    .child(
                        Button::new("close")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Close Find")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_action_escape(&Escape, window, cx);
                            })),
                    ),
            )
            .when(self.replace_mode, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            Input::new(&self.replace_input)
                                .focus_bordered(false)
                                .small()
                                .w(self.input_width)
                                .shadow_none(),
                        )
                        .child(
                            Button::new("replace-one")
                                .small()
                                .label(t!("Input.Replace"))
                                .disabled(!has_matches)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.replace_next(window, cx);
                                })),
                        )
                        .child(
                            Button::new("replace-all")
                                .small()
                                .label(t!("Input.Replace All"))
                                .disabled(!has_matches)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.replace_all(window, cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
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
            .map(|r| &text[r.clone()])
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
        assert_eq!(matcher.matched_ranges.first(), Some(&(5..10)));
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
        matcher.matched_ranges = Arc::new(vec![5..10, 15..20, 25..30]);
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
