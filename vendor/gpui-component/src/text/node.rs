use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, Range},
    sync::{Arc, Mutex, OnceLock},
};

use gpui::{
    Pixels, AnyElement, App, DefiniteLength, Div, Element, ElementId, FontStyle, FontWeight, Half,
    HighlightStyle, InteractiveElement as _, IntoElement, ListState, ObjectFit, ParentElement,
    ScrollHandle, SharedString, SharedUri, StatefulInteractiveElement, Styled, StyledImage as _,
    Window, div, img, prelude::FluentBuilder as _, px, relative, rems,
};
use pulldown_cmark::Alignment;
use ropey::Rope;

use crate::{
    ActiveTheme as _, Icon, IconName, StyledExt, h_flex,
    highlighter::{HighlightTheme, SyntaxHighlighter},
    scroll::{ScrollableElement, Scrollbar, ScrollbarShow},
    styled::FocusableExt,
    text::{
        CodeBlockActionsFn,
        inline::{Inline, InlineState},
    },
    tooltip::Tooltip,
    v_flex,
};

use super::{
    TextViewStyle,
    utils::{list_item_prefix, scroll_reading_list},
};

#[allow(unused)]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct LinkMark {
    pub url: SharedString,
    /// Optional identifier for footnotes.
    pub identifier: Option<SharedString>,
    pub title: Option<SharedString>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct TextMark {
    pub bold: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<Arc<LinkMark>>,
}

impl TextMark {
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    pub fn code(mut self) -> Self {
        self.code = true;
        self
    }

    pub fn link(mut self, link: impl Into<LinkMark>) -> Self {
        self.link = Some(Arc::new(link.into()));
        self
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl From<Span> for ElementId {
    fn from(value: Span) -> Self {
        ElementId::Name(format!("md-{}:{}", value.start, value.end).into())
    }
}

#[allow(unused)]
#[derive(Debug, Default, Clone)]
pub struct ImageNode {
    pub url: SharedUri,
    pub link: Option<LinkMark>,
    pub title: Option<SharedString>,
    pub alt: Option<SharedString>,
    pub width: Option<DefiniteLength>,
    pub height: Option<DefiniteLength>,
}

impl ImageNode {
    pub fn title(&self) -> String {
        self.title
            .clone()
            .unwrap_or_else(|| self.alt.clone().unwrap_or_default())
            .to_string()
    }
}

impl PartialEq for ImageNode {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.link == other.link
            && self.title == other.title
            && self.alt == other.alt
            && self.width == other.width
            && self.height == other.height
    }
}

pub(crate) type InlineMarks = Arc<[(Range<usize>, TextMark)]>;

#[derive(Clone, Debug)]
pub(crate) enum InlineText {
    Owned(SharedString),
    Pending(Box<str>),
    Shared { text: Arc<str>, range: Range<u32> },
}

impl InlineText {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Owned(text) => text.as_str(),
            Self::Pending(text) => text,
            Self::Shared { text, range } => &text[range.start as usize..range.end as usize],
        }
    }

    pub(super) fn shared(&self) -> Arc<str> {
        match self {
            Self::Owned(text) => text.clone().into(),
            Self::Pending(_) | Self::Shared { .. } => Arc::from(self.as_str()),
        }
    }
}

impl Default for InlineText {
    fn default() -> Self {
        Self::Owned(SharedString::default())
    }
}

impl Deref for InlineText {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for InlineText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

#[derive(Default, Clone, Debug)]
pub(crate) struct InlineNode {
    /// The text content.
    pub(crate) text: InlineText,
    pub(crate) image: Option<Box<ImageNode>>,
    /// The text styles, each tuple contains the range of the text and the style.
    pub(crate) marks: InlineMarks,

    state: OnceLock<Arc<Mutex<InlineState>>>,
}

impl PartialEq for InlineNode {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.image == other.image && self.marks == other.marks
    }
}

impl InlineNode {
    pub(crate) fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: InlineText::Owned(text.into()),
            image: None,
            marks: InlineMarks::default(),
            state: OnceLock::new(),
        }
    }

    pub(crate) fn image(image: ImageNode) -> Self {
        let mut this = Self::new("");
        this.image = Some(Box::new(image));
        this
    }

    pub(crate) fn marks(mut self, marks: impl Into<InlineMarks>) -> Self {
        self.marks = marks.into();
        self
    }

    #[cfg(test)]
    pub(crate) fn has_render_state(&self) -> bool {
        self.state.get().is_some()
    }
}

/// The paragraph element, contains multiple text nodes.
///
/// Unlike other Element, this is cloneable, because it is used in the Node AST.
/// We are keep the selection state inside this AST Nodes.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paragraph {
    pub(super) span: Option<Span>,
    pub(super) children: Vec<InlineNode>,
    pub(super) state: OnceLock<Arc<Mutex<InlineState>>>,
}

impl PartialEq for Paragraph {
    fn eq(&self, other: &Self) -> bool {
        self.span == other.span && self.children == other.children
    }
}

impl Paragraph {
    pub(crate) fn new(text: String) -> Self {
        Self {
            span: None,
            children: vec![InlineNode::new(&text)],
            state: OnceLock::new(),
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();

        for state in self
            .children
            .iter()
            .filter_map(|child| child.state.get())
            .chain(self.state.get())
        {
            let state = state.lock().unwrap();
            if let Some(selection) = &state.selection {
                text.push_str(&state.text[selection.start..selection.end]);
            }
        }

        text
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Table {
    pub children: Vec<TableRow>,
    pub column_aligns: Vec<ColumnumnAlign>,
}

impl Table {
    pub(crate) fn column_align(&self, index: usize) -> ColumnumnAlign {
        self.column_aligns.get(index).copied().unwrap_or_default()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub(crate) enum ColumnumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl From<Alignment> for ColumnumnAlign {
    fn from(value: Alignment) -> Self {
        match value {
            Alignment::None => ColumnumnAlign::Left,
            Alignment::Left => ColumnumnAlign::Left,
            Alignment::Center => ColumnumnAlign::Center,
            Alignment::Right => ColumnumnAlign::Right,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableRow {
    pub children: Vec<TableCell>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TableCell {
    pub children: Paragraph,
    pub width: Option<DefiniteLength>,
}

impl Paragraph {
    pub(crate) fn take(&mut self) -> Paragraph {
        std::mem::take(self).compact()
    }

    pub(crate) fn compact(mut self) -> Self {
        self.children.shrink_to_fit();
        self
    }

    pub(crate) fn is_image(&self) -> bool {
        false
    }

    pub(crate) fn set_span(&mut self, span: Span) {
        self.span = Some(span);
    }

    pub(crate) fn push_str(&mut self, text: &str) {
        self.children.push(InlineNode::new(text.to_string()));
    }

    pub(crate) fn push(&mut self, text: InlineNode) {
        self.children.push(text);
    }

    pub(crate) fn push_image(&mut self, image: ImageNode) {
        self.children.push(InlineNode::image(image));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.children.is_empty()
            || self
                .children
                .iter()
                .all(|node| node.text.is_empty() && node.image.is_none())
    }

    /// Return length of children text.
    pub(crate) fn text_len(&self) -> usize {
        self.children
            .iter()
            .map(|node| node.text.len())
            .sum::<usize>()
    }

    pub(crate) fn merge(&mut self, other: Self) {
        self.children.extend(other.children);
    }
}

#[derive(Debug, Clone)]
pub struct CodeBlock {
    lang: Option<SharedString>,
    styles: Vec<(Range<usize>, HighlightStyle)>,
    state: Arc<Mutex<InlineState>>,
    page: Option<Box<CodePage>>,
    pages: Option<Arc<Vec<Self>>>,
    /// The fence's closing newline, kept for copying but not drawn as an empty last line.
    trailing_newline: bool,
}

#[derive(Debug, Clone)]
struct CodePage {
    source: SharedString,
    range: Range<usize>,
}

impl CodePage {
    fn text(&self) -> &str {
        &self.source[self.range.clone()]
    }

    fn break_after(&self) -> &str {
        if self.range.end == self.source.len() {
            ""
        } else if self.text().ends_with("\r\n") {
            "\r\n"
        } else if self.text().ends_with('\n') {
            "\n"
        } else {
            ""
        }
    }
}

impl PartialEq for CodeBlock {
    fn eq(&self, other: &Self) -> bool {
        self.lang == other.lang && self.styles == other.styles
    }
}

impl CodeBlock {
    /// Get the language of the code block.
    pub fn lang(&self) -> Option<SharedString> {
        self.lang.clone()
    }

    /// Get the code content of the code block.
    pub fn code(&self) -> SharedString {
        self.page.as_ref().map_or_else(
            || {
                let text = self.state.lock().unwrap().text.clone();
                if self.trailing_newline {
                    SharedString::from(format!("{text}\n"))
                } else {
                    text
                }
            },
            |page| SharedString::new(page.text()),
        )
    }

    fn display_code(&self) -> SharedString {
        let mut state = self.state.lock().unwrap();
        if state.text.is_empty()
            && let Some(page) = &self.page
        {
            let text = page.text();
            state.set_text(SharedString::new(
                &text[..text.len() - page.break_after().len()],
            ));
        }
        state.text.clone()
    }

    pub(crate) fn new(
        code: SharedString,
        lang: Option<SharedString>,
        _: &TextViewStyle,
        highlight_theme: &HighlightTheme,
    ) -> Self {
        let trailing_newline = code.len() <= LONG_READING_BLOCK_BYTES && code.ends_with('\n');
        let code = if trailing_newline {
            SharedString::from(code[..code.len() - 1].to_owned())
        } else {
            code
        };
        let mut styles = vec![];
        // Without the Tree-sitter language set every name resolves to JSON, which would
        // mis-colour other languages; leave those blocks plain.
        let supported = cfg!(feature = "tree-sitter-languages")
            || lang
                .as_ref()
                .is_some_and(|lang| matches!(lang.as_ref(), "json" | "jsonc"));
        if let Some(lang) = lang.as_ref().filter(|_| supported) {
            let mut highlighter = SyntaxHighlighter::new(&lang);
            highlighter.update(None, &Rope::from_str(code.as_str()));
            styles = highlighter.styles(&(0..code.len()), highlight_theme);
        };

        let state = Arc::new(Mutex::new(InlineState::default()));
        state.lock().unwrap().set_text(code);

        Self {
            lang,
            styles,
            state,
            page: None,
            pages: None,
            trailing_newline,
        }
    }

    fn needs_reading_pages(&self) -> bool {
        self.pages.is_none() && self.state.lock().unwrap().text.len() > LONG_READING_BLOCK_BYTES
    }

    fn paginate_nested(&mut self) {
        let mut pages = Vec::new();
        self.reading_pages(|page| pages.push(page));
        self.styles = Vec::new();
        self.pages = Some(Arc::new(pages));
    }

    fn map_colors(&mut self, map: &impl Fn(gpui::Hsla) -> gpui::Hsla) {
        for (_, style) in &mut self.styles {
            style.color = style.color.map(map);
        }
        if let Some(pages) = &mut self.pages {
            for page in Arc::make_mut(pages) {
                page.map_colors(map);
            }
        }
    }

    fn reading_pages(&self, mut emit: impl FnMut(Self)) {
        let source = self.code();
        let mut start = 0;
        while start < source.len() {
            let remaining = &source[start..];
            let limit = remaining.floor_char_boundary(READING_PAGE_BYTES.min(remaining.len()));
            let end = start
                + if limit == remaining.len() {
                    limit
                } else if let Some(line) = remaining[..limit].rfind('\n') {
                    line + 1
                } else {
                    reading_page_end(remaining, READING_PAGE_BYTES)
                };
            let page = CodePage {
                source: source.clone(),
                range: start..end,
            };
            let display_end = end - page.break_after().len();
            let first = self.styles.partition_point(|(range, _)| range.end <= start);
            let styles = self.styles[first..]
                .iter()
                .take_while(|(range, _)| range.start < display_end)
                .map(|(range, style)| {
                    (
                        range.start.max(start) - start..range.end.min(display_end) - start,
                        *style,
                    )
                })
                .collect();
            emit(Self {
                lang: self.lang.clone(),
                styles,
                state: Arc::new(Mutex::new(InlineState::default())),
                page: Some(Box::new(page)),
                pages: None,
                trailing_newline: false,
            });
            start = end;
        }
    }

    fn starts_block(&self) -> bool {
        self.page.as_ref().is_none_or(|page| page.range.start == 0)
    }

    fn ends_block(&self) -> bool {
        self.page
            .as_ref()
            .is_none_or(|page| page.range.end == page.source.len())
    }

    pub(super) fn selected_text(&self) -> String {
        if let Some(pages) = &self.pages {
            return pages.iter().map(Self::selected_text).collect();
        }
        let mut text = String::new();
        let state = self.state.lock().unwrap();
        if let Some(selection) = &state.selection {
            let part_text = state.text.clone();
            text.push_str(&part_text[selection.start..selection.end]);
            if let Some(page) = &self.page
                && selection.end == part_text.len()
            {
                text.push_str(page.break_after());
            }
        }
        text
    }

    fn render(
        &self,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let style = &node_cx.style;
        if let Some(pages) = &self.pages {
            return Self::render_pages(pages, options, node_cx, window, cx);
        }
        let paged = self.page.is_some();
        if paged {
            self.display_code();
        }

        div()
            .when(self.ends_block() && !options.is_last, |this| {
                this.pb(style.paragraph_gap)
            })
            .child(
                div()
                    .id("codeblock")
                    .when(!paged, |block| {
                        block.overflow_x_scroll().rounded(cx.theme().radius)
                    })
                    .when(paged, |block| block.w_full().min_w_0().whitespace_normal())
                    .px_3()
                    .when(self.starts_block(), |block| block.pt_3())
                    .when(self.ends_block(), |block| block.pb_3())
                    .bg(cx.theme().muted)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .relative()
                    .refine_style(&style.code_block)
                    .child(
                        Inline::new(
                            "code",
                            self.state.clone(),
                            vec![],
                            self.styles.clone(),
                            None,
                        )
                        .accessible(),
                    )
                    .when_some(node_cx.code_block_actions.clone(), |this, actions| {
                        this.child(
                            div()
                                .absolute()
                                .top_2()
                                .right_2()
                                .bg(cx.theme().muted)
                                .rounded(cx.theme().radius)
                                .child(actions(&self, window, cx)),
                        )
                    }),
            )
            .into_any_element()
    }
    fn render_pages(
        pages: &Arc<Vec<Self>>,
        options: &NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let id = ("nested-code", Arc::as_ptr(pages) as usize);
        let controls = window.use_keyed_state(id, cx, |_, cx| {
            (
                ListState::new(pages.len(), gpui::ListAlignment::Top, px(120.)),
                cx.focus_handle().tab_stop(true),
            )
        });
        let (list, focus) = controls.read(cx).clone();
        let height = px(260.).min(window.viewport_size().height / 2.);
        let page_options = options.is_last(true);
        let context = node_cx.clone();
        let rows = pages.clone();
        let content = gpui::list(list.clone(), move |index, window, cx| {
            div()
                .id(("code-page", index))
                .child(rows[index].render(&page_options, &context, window, cx))
                .into_any_element()
        });
        let input_list = list.clone();
        let input_focus = focus.clone();
        let panel = div()
            .id(id)
            .relative()
            .w_full()
            .min_w_0()
            .h(height)
            .rounded(cx.theme().radius)
            .bg(cx.theme().muted)
            .track_focus(&focus)
            .focus_ring(focus.is_focused(window), px(2.), window, cx)
            .child(content.size_full())
            .vertical_scrollbar(&list)
            .on_key_down(move |event, window, cx| {
                if input_focus.is_focused(window)
                    && scroll_reading_list(&input_list, event.keystroke.key.as_str(), height)
                {
                    cx.stop_propagation();
                    window.refresh();
                }
            });
        let focus_for_action = focus.clone();
        div()
            .when(!options.is_last, |block| {
                block.pb(node_cx.style.paragraph_gap)
            })
            .child(
                gpui::accessible(id, panel, move |_, window, _| {
                    let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Group);
                    node.set_label("Scrollable code block");
                    node.add_action(gpui::accesskit::Action::Focus);
                    let mut info = gpui::AccessibilityInfo::new(node);
                    info.focused = focus.is_focused(window);
                    info
                })
                .on_action(move |request, _, window, _| {
                    if request.action == gpui::accesskit::Action::Focus {
                        focus_for_action.focus(window);
                    }
                }),
            )
            .into_any_element()
    }
}

/// A context for rendering nodes, contains link references.
#[derive(Default, Clone)]
pub(crate) struct NodeContext {
    pub(crate) link_refs: HashMap<SharedString, LinkMark>,
    pub(crate) resources: Arc<super::TextViewResources>,
    pub(crate) style: TextViewStyle,
    pub(crate) prose_width: Option<Pixels>,
    pub(crate) code_block_actions: Option<Arc<CodeBlockActionsFn>>,
}

impl NodeContext {
    pub(super) fn add_ref(&mut self, identifier: SharedString, link: LinkMark) {
        self.link_refs.insert(identifier, link);
    }
}

impl PartialEq for NodeContext {
    fn eq(&self, other: &Self) -> bool {
        self.link_refs == other.link_refs
            && self.style == other.style
            && self.prose_width == other.prose_width
        // Note: code_block_buttons is intentionally not compared (closures can't be compared)
    }
}

/// The AST Node of the rich text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Root {
        children: Arc<Vec<Node>>,
    },
    Paragraph(Paragraph),
    ParagraphPage(Paragraph),
    Heading {
        level: u8,
        children: Paragraph,
    },
    Blockquote {
        children: Vec<Node>,
    },
    List {
        /// Only contains ListItem, others will be ignored
        children: Vec<Node>,
        start: Option<u64>,
    },
    ListItem {
        children: Vec<Node>,
        spread: bool,
        /// Whether the list item is checked, if None, it's not a checkbox
        checked: Option<bool>,
    },
    CodeBlock(CodeBlock),
    Table(Table),
    Break {
        html: bool,
    },
    Divider,
    Unknown,
}

const READING_PAGE_BYTES: usize = 2048;
const LONG_READING_BLOCK_BYTES: usize = 16 * 1024;

impl Paragraph {
    fn needs_reading_pages(&self) -> bool {
        self.text_len() > LONG_READING_BLOCK_BYTES
    }

    fn into_reading_pages(self, rows: &mut Vec<Node>) {
        let span = self.span;
        let mut page = Paragraph {
            span,
            ..Default::default()
        };
        let mut bytes = 0;
        for inline in self.children {
            let len = inline.text.len();
            let space = if len <= READING_PAGE_BYTES { len } else { 256 };
            if bytes + space > READING_PAGE_BYTES {
                rows.push(Node::ParagraphPage(page.take()));
                page.span = span;
                bytes = 0;
            }
            if len <= READING_PAGE_BYTES {
                bytes += len;
                page.children.push(inline);
                continue;
            }

            let (text, absolute_start) = match inline.text {
                InlineText::Owned(text) => (text.into(), 0),
                InlineText::Pending(text) => (Arc::from(text), 0),
                InlineText::Shared { text, range } => (text, range.start as usize),
            };
            let value = &text[absolute_start..absolute_start + len];
            let mut start = 0;
            while start < len {
                let end = start + reading_page_end(&value[start..], READING_PAGE_BYTES - bytes);
                let from = inline
                    .marks
                    .partition_point(|(range, _)| range.end <= start);
                let marks = inline.marks[from..]
                    .iter()
                    .take_while(|(range, _)| range.start < end)
                    .map(|(range, mark)| {
                        (
                            range.start.max(start) - start..range.end.min(end) - start,
                            mark.clone(),
                        )
                    })
                    .collect::<Vec<_>>();
                page.children.push(InlineNode {
                    text: InlineText::Shared {
                        text: text.clone(),
                        range: (absolute_start + start) as u32..(absolute_start + end) as u32,
                    },
                    marks: marks.into(),
                    ..Default::default()
                });
                bytes += end - start;
                start = end;
                if start < len {
                    rows.push(Node::ParagraphPage(page.take()));
                    page.span = span;
                    bytes = 0;
                }
            }
        }
        rows.push(Node::Paragraph(page.compact()));
    }
}

fn reading_page_end(text: &str, budget: usize) -> usize {
    use unicode_segmentation::UnicodeSegmentation;

    if text.len() <= budget {
        return text.len();
    }
    let limit = text.floor_char_boundary(budget);
    let mut end = 0;
    let mut whitespace = None;
    let lookahead = text.ceil_char_boundary((limit + 128).min(text.len()));
    for (start, grapheme) in text[..lookahead].grapheme_indices(true) {
        let next = start + grapheme.len();
        if next > limit {
            break;
        }
        end = next;
        if next >= limit - limit / 4 && grapheme.chars().all(char::is_whitespace) {
            whitespace = Some(next);
        }
    }
    whitespace.unwrap_or(if end > 0 { end } else { limit })
}

impl Node {
    pub(super) fn paginate_reading_blocks(&mut self) {
        let Self::Root { children } = self else {
            return;
        };
        for node in Arc::make_mut(children) {
            node.paginate_nested_code();
        }
        if !children.iter().any(|node| match node {
            Self::Paragraph(paragraph) => paragraph.needs_reading_pages(),
            Self::CodeBlock(code) => code.needs_reading_pages(),
            _ => false,
        }) {
            return;
        }
        let children = Arc::make_mut(children);
        let previous = std::mem::take(children);
        children.reserve(previous.len());
        for node in previous {
            match node {
                Self::Paragraph(paragraph) if paragraph.needs_reading_pages() => {
                    paragraph.into_reading_pages(children);
                }
                Self::CodeBlock(code) if code.needs_reading_pages() => {
                    code.reading_pages(|page| children.push(Self::CodeBlock(page)));
                }
                node => children.push(node),
            }
        }
        children.shrink_to_fit();
    }

    fn paginate_nested_code(&mut self) {
        let nested = match self {
            Self::Blockquote { children }
            | Self::List { children, .. }
            | Self::ListItem { children, .. } => children,
            _ => return,
        };
        for node in nested {
            if let Self::CodeBlock(code) = node
                && code.needs_reading_pages()
            {
                code.paginate_nested();
            } else {
                node.paginate_nested_code();
            }
        }
    }

    pub(super) fn has_nested_code_pages(&self) -> bool {
        match self {
            Self::CodeBlock(code) => code.pages.is_some(),
            Self::Blockquote { children }
            | Self::List { children, .. }
            | Self::ListItem { children, .. } => children.iter().any(Self::has_nested_code_pages),
            _ => false,
        }
    }
}

impl Node {
    pub(super) fn map_code_colors(&mut self, map: &impl Fn(gpui::Hsla) -> gpui::Hsla) {
        let children = match self {
            Self::Root { children } => Arc::make_mut(children),
            Self::Blockquote { children }
            | Self::List { children, .. }
            | Self::ListItem { children, .. } => children,
            Self::CodeBlock(block) => {
                block.map_colors(map);
                return;
            }
            _ => return,
        };
        for child in children {
            child.map_code_colors(map);
        }
    }

    pub(super) fn is_list_item(&self) -> bool {
        matches!(self, Self::ListItem { .. })
    }

    /// Combine all children, omitting the empt parent nodes.
    pub(super) fn compact(self) -> Node {
        match self {
            Self::Root { children } if children.len() == 1 => children[0].clone().compact(),
            _ => self,
        }
    }

    pub(super) fn selected_text(&self) -> String {
        let mut text = String::new();
        match self {
            Node::Root { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::ParagraphPage(paragraph) => text.push_str(&paragraph.selected_text()),
            Node::Paragraph(paragraph) => {
                let mut block_text = String::new();
                block_text.push_str(&paragraph.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Heading { children, .. } => {
                let mut block_text = String::new();
                block_text.push_str(&children.selected_text());
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::List { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::ListItem { children, .. } => {
                for c in children.iter() {
                    text.push_str(&c.selected_text());
                }
            }
            Node::Blockquote { children } => {
                let mut block_text = String::new();
                for c in children.iter() {
                    block_text.push_str(&c.selected_text());
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::Table(table) => {
                let mut block_text = String::new();
                for row in table.children.iter() {
                    let mut row_texts = vec![];
                    for cell in row.children.iter() {
                        row_texts.push(cell.children.selected_text());
                    }
                    if !row_texts.is_empty() {
                        block_text.push_str(&row_texts.join(" "));
                        block_text.push('\n');
                    }
                }

                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    text.push('\n');
                }
            }
            Node::CodeBlock(code_block) => {
                let block_text = code_block.selected_text();
                if !block_text.is_empty() {
                    text.push_str(&block_text);
                    if code_block.ends_block() {
                        text.push('\n');
                    }
                }
            }
            Node::Break { .. } | Node::Divider | Node::Unknown => {}
        }

        text
    }
}

impl TextMark {
    fn reading_highlight(&self, colors: &crate::theme::ThemeColor) -> HighlightStyle {
        let mut highlight = HighlightStyle::default();
        if self.bold {
            highlight.font_weight = Some(FontWeight::BOLD);
        }
        if self.italic {
            highlight.font_style = Some(FontStyle::Italic);
        }
        if self.strikethrough {
            highlight.strikethrough = Some(gpui::StrikethroughStyle {
                thickness: px(1.),
                ..Default::default()
            });
        }
        if self.code {
            highlight.background_color = Some(colors.secondary);
            highlight.monospace = Some(true);
        }
        if self.link.is_some() {
            highlight.color = Some(colors.link);
            highlight.underline = Some(gpui::UnderlineStyle {
                thickness: px(1.),
                ..Default::default()
            });
        }
        highlight
    }
}

impl Paragraph {
    fn render(
        &self,
        node_cx: &NodeContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let span = self.span;
        let children = &self.children;

        let mut child_nodes: Vec<AnyElement> = vec![];

        let mut text = String::new();
        let mut highlights: Vec<(Range<usize>, HighlightStyle)> = vec![];
        let mut links: Vec<(Range<usize>, LinkMark)> = vec![];
        let mut offset = 0;

        let mut ix = 0usize;
        for inline_node in children {
            let text_len = inline_node.text.len();
            text.push_str(&inline_node.text);

            if let Some(image) = &inline_node.image {
                if text.len() > 0 {
                    let state = inline_node.state.get_or_init(Arc::default);
                    state.lock().unwrap().set_text(text.clone().into());
                    child_nodes.push(
                        Inline::new(
                            ix,
                            state.clone(),
                            links.clone(),
                            highlights.clone(),
                            node_cx.resources.on_link.clone(),
                        )
                        .accessible()
                        .into_any_element(),
                    );
                }
                if node_cx.resources.restrict_images
                    && !node_cx.resources.images.contains_key(&image.url)
                {
                    let label: SharedString =
                        format!("[Image unavailable: {}]", image.title()).into();
                    child_nodes.push(
                        crate::semantic_text::text(
                            ("image-omitted", ix),
                            label.clone(),
                            div().child(label),
                        )
                        .into_any_element(),
                    );
                } else {
                    let image_source = node_cx
                        .resources
                        .images
                        .get(&image.url)
                        .cloned()
                        .map(gpui::ImageSource::from)
                        .unwrap_or_else(|| image.url.clone().into());
                    let on_link = node_cx.resources.on_link.clone();
                    let alt = image.title();
                    let rendered = img(image_source)
                        .id(ix)
                        .object_fit(ObjectFit::Contain)
                        .max_w(relative(1.))
                        .when_some(image.width, |this, width| this.w(width))
                        .when_some(
                            image.link.as_ref().map(|link| {
                                link.identifier
                                    .as_ref()
                                    .and_then(|id| node_cx.link_refs.get(id))
                                    .unwrap_or(link)
                                    .clone()
                            }),
                            |this, link| {
                                let title = image.title();
                                this.cursor_pointer()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(title.clone()).build(window, cx)
                                    })
                                    .on_click(move |_, window, cx| {
                                        cx.stop_propagation();
                                        if let Some(on_link) = &on_link {
                                            on_link(&link.url, window, cx);
                                        } else {
                                            cx.open_url(&link.url);
                                        }
                                    })
                            },
                        );
                    child_nodes.push(
                        gpui::accessible(("image", ix), rendered, move |_, _, _| {
                            let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Image);
                            node.set_label(alt.clone());
                            gpui::AccessibilityInfo::new(node)
                        })
                        .into_any_element(),
                    );
                }
                text.clear();
                links.clear();
                highlights.clear();
                offset = 0;
            } else {
                let mut node_highlights = vec![];
                for (range, style) in inline_node.marks.iter() {
                    let inner_range = (offset + range.start)..(offset + range.end);

                    let highlight = style.reading_highlight(&cx.theme().colors);
                    if let Some(mut link_mark) = style.link.as_deref().cloned() {
                        // convert link references, replace link
                        if let Some(identifier) = link_mark.identifier.as_ref() {
                            if let Some(mark) = node_cx.link_refs.get(identifier) {
                                link_mark = mark.clone();
                            }
                        }

                        links.push((inner_range.clone(), link_mark));
                    }

                    node_highlights.push((inner_range, highlight));
                }

                highlights = gpui::combine_highlights(highlights, node_highlights).collect();
                offset += text_len;
            }
            ix += 1;
        }

        // Add the last text node
        if text.len() > 0 {
            let state = self.state.get_or_init(Arc::default);
            state.lock().unwrap().set_text(text.into());
            child_nodes.push(
                Inline::new(
                    ix,
                    state.clone(),
                    links,
                    highlights,
                    node_cx.resources.on_link.clone(),
                )
                .accessible()
                .into_any_element(),
            );
        }

        div().id(span.unwrap_or_default()).children(child_nodes)
    }
}

#[derive(Default, Clone, Copy)]
struct NodeRenderOptions {
    in_list: bool,
    todo: bool,
    list_start: Option<u64>,
    depth: usize,
    is_last: bool,
}

impl NodeRenderOptions {
    fn is_last(mut self, is_last: bool) -> Self {
        self.is_last = is_last;
        self
    }
}

impl Paragraph {
    fn to_markdown(&self) -> String {
        let mut text = self
            .children
            .iter()
            .map(|text_node| {
                let mut text = text_node.text.to_string();
                for (range, style) in text_node.marks.iter() {
                    if style.bold {
                        text = format!("**{}**", &text_node.text[range.clone()]);
                    }
                    if style.italic {
                        text = format!("*{}*", &text_node.text[range.clone()]);
                    }
                    if style.strikethrough {
                        text = format!("~~{}~~", &text_node.text[range.clone()]);
                    }
                    if style.code {
                        text = format!("`{}`", &text_node.text[range.clone()]);
                    }
                    if let Some(link) = &style.link {
                        text = format!("[{}]({})", &text_node.text[range.clone()], link.url);
                    }
                }

                if let Some(image) = &text_node.image {
                    let alt = image.alt.clone().unwrap_or_default();
                    let title = image
                        .title
                        .clone()
                        .map_or(String::new(), |t| format!(" \"{}\"", t));
                    text.push_str(&format!("![{}]({}{})", alt, image.url, title))
                }

                text
            })
            .collect::<Vec<_>>()
            .join("");

        text.push_str("\n\n");
        text
    }
}

pub(super) struct ShapeRequest {
    pub(super) style: gpui::TextStyle,
    pub(super) code_style: gpui::TextStyle,
    pub(super) colors: crate::theme::ThemeColor,
    pub(super) view_style: TextViewStyle,
    pub(super) rem_size: Pixels,
}

struct ShapeBuilder<'a> {
    request: &'a ShapeRequest,
    layouts: &'a gpui::WindowTextSystem,
    remaining_bytes: usize,
    remaining_nodes: usize,
}

impl ShapeRequest {
    pub(super) fn code_style(style: &gpui::TextStyle, theme: &crate::Theme) -> gpui::TextStyle {
        let mut style = style.clone();
        style.font_family = theme.mono_font_family.clone();
        style.font_size = theme.mono_font_size.into();
        style
    }

    pub(super) fn prepare<'a>(
        &self,
        children: impl IntoIterator<Item = &'a Node>,
        layouts: &gpui::WindowTextSystem,
    ) {
        let mut batch = ShapeBuilder {
            request: self,
            layouts,
            remaining_bytes: 48 * 1024,
            remaining_nodes: 180,
        };
        for node in children {
            batch.node(node, &self.style);
        }
    }
}

impl ShapeBuilder<'_> {
    fn node(&mut self, node: &Node, style: &gpui::TextStyle) {
        if self.remaining_nodes == 0 || self.remaining_bytes == 0 {
            return;
        }
        self.remaining_nodes -= 1;
        match node {
            Node::Paragraph(paragraph) | Node::ParagraphPage(paragraph) => {
                self.paragraph(paragraph, style)
            }
            Node::CodeBlock(block) if block.page.is_some() => self.code(block),
            Node::CodeBlock(block) => {
                if let Some(pages) = &block.pages {
                    for page in pages.iter().take(4).chain(pages.iter().rev().take(4)) {
                        self.code(page);
                    }
                }
            }
            Node::Heading { level, children } => {
                let body_size = style.font_size.to_pixels(self.request.rem_size);
                let (size, weight) = heading_style(*level, &self.request.view_style, body_size);
                let mut style = style.clone();
                style.font_size = size.into();
                style.font_weight = weight;
                self.paragraph(children, &style);
            }
            Node::Blockquote { children } => {
                let mut style = style.clone();
                style.color = self.request.colors.muted_foreground;
                for child in children.iter().take(24) {
                    self.node(child, &style);
                }
            }
            Node::List { children, .. } | Node::ListItem { children, .. } => {
                for child in children.iter().take(24) {
                    self.node(child, style);
                }
            }
            _ => {}
        }
    }

    fn code(&mut self, block: &CodeBlock) {
        let text = block.display_code();
        if text.len() <= self.remaining_bytes {
            self.remaining_bytes -= text.len();
            let style = &self.request.code_style;
            let runs = Inline::text_runs(&text, style, &block.styles);
            let _ = self.layouts.shape_text(
                text,
                style.font_size.to_pixels(self.request.rem_size),
                &runs,
                None,
                style.line_clamp,
            );
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, style: &gpui::TextStyle) {
        if paragraph.children.len() > 32 {
            return;
        }
        let limit = self.remaining_bytes.min(2 * 1024);
        let mut text = String::new();
        let mut highlights = Vec::new();
        for inline in &paragraph.children {
            let offset = text.len();
            if inline.image.is_some() || offset + inline.text.len() > limit {
                return;
            }
            text.push_str(&inline.text);
            let marks = inline
                .marks
                .iter()
                .map(|(range, mark)| {
                    (
                        (offset + range.start)..(offset + range.end),
                        mark.reading_highlight(&self.request.colors),
                    )
                })
                .collect::<Vec<_>>();
            highlights = gpui::combine_highlights(highlights, marks).collect();
        }
        if !text.is_empty() {
            self.remaining_bytes -= text.len();
            let runs = Inline::text_runs(&text, style, &highlights);
            let _ = self.layouts.shape_text(
                text.into(),
                style.font_size.to_pixels(self.request.rem_size),
                &runs,
                None,
                style.line_clamp,
            );
        }
    }
}

fn heading_style(level: u8, style: &TextViewStyle, body_size: Pixels) -> (Pixels, FontWeight) {
    let (size, weight) = match level {
        1 => (rems(2.), FontWeight::BOLD),
        2 => (rems(1.5), FontWeight::SEMIBOLD),
        3 => (rems(1.25), FontWeight::SEMIBOLD),
        4 => (rems(1.125), FontWeight::SEMIBOLD),
        5 => (rems(1.), FontWeight::SEMIBOLD),
        6 => (rems(1.), FontWeight::MEDIUM),
        _ => (rems(1.), FontWeight::NORMAL),
    };
    // The default scale is defined for 16 px prose; it follows larger or smaller reading text
    // so headings keep their proportion at every interface scale.
    let base = style.heading_base_font_size * (body_size / px(16.));
    let size = style.heading_font_size.as_ref().map_or_else(
        || size.to_pixels(base).max(body_size),
        |value| value(level, style.heading_base_font_size),
    );
    (size, weight)
}

impl Node {
    pub(crate) fn links(&self, context: &NodeContext) -> Vec<(SharedString, SharedString)> {
        let mut links = Vec::new();
        let mut seen = HashSet::new();
        self.visit_links(context, &mut |label, url| {
            if seen.insert((label, url)) {
                links.push((label.to_owned().into(), url.clone()));
            }
        });
        links
    }

    fn visit_links<'a>(
        &'a self,
        context: &'a NodeContext,
        visit: &mut impl FnMut(&'a str, &'a SharedString),
    ) {
        let mut paragraph = |paragraph: &'a Paragraph| {
            for child in &paragraph.children {
                for (range, mark) in child.marks.iter() {
                    if let Some(link) = &mark.link {
                        let link = link
                            .identifier
                            .as_ref()
                            .and_then(|id| context.link_refs.get(id))
                            .unwrap_or(link);
                        if !link.url.is_empty() {
                            visit(&child.text[range.clone()], &link.url);
                        }
                    }
                }
                if let Some(image) = &child.image {
                    if let Some(link) = &image.link {
                        let link = link
                            .identifier
                            .as_ref()
                            .and_then(|id| context.link_refs.get(id))
                            .unwrap_or(link);
                        if !link.url.is_empty() {
                            let label = image
                                .title
                                .as_ref()
                                .or(image.alt.as_ref())
                                .map(SharedString::as_str)
                                .unwrap_or_default();
                            visit(label, &link.url);
                        }
                    }
                }
            }
        };
        match self {
            Node::Root { children } => {
                for child in children.iter() {
                    child.visit_links(context, visit)
                }
            }
            Node::List { children, .. }
            | Node::ListItem { children, .. }
            | Node::Blockquote { children } => {
                for child in children {
                    child.visit_links(context, visit)
                }
            }
            Node::Paragraph(text)
            | Node::ParagraphPage(text)
            | Node::Heading { children: text, .. } => paragraph(text),
            Node::Table(table) => {
                for row in &table.children {
                    for cell in &row.children {
                        paragraph(&cell.children)
                    }
                }
            }
            _ => {}
        }
    }

    /// Converts the node to markdown format.
    ///
    /// This is used to generate markdown for test.
    #[allow(dead_code)]
    pub(crate) fn to_markdown(&self) -> String {
        match self {
            Node::Root { children } => children
                .iter()
                .map(|child| child.to_markdown())
                .collect::<Vec<_>>()
                .join("\n\n"),
            Node::Paragraph(paragraph) | Node::ParagraphPage(paragraph) => paragraph.to_markdown(),
            Node::Heading { level, children } => {
                let hashes = "#".repeat(*level as usize);
                format!("{} {}", hashes, children.to_markdown())
            }
            Node::Blockquote { children } => {
                let content = children
                    .iter()
                    .map(|child| child.to_markdown())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                content
                    .lines()
                    .map(|line| format!("> {}", line))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Node::List { children, start } => children
                .iter()
                .enumerate()
                .map(|(i, child)| {
                    let prefix = start.map_or_else(
                        || "- ".to_string(),
                        |start| list_item_prefix(i, Some(start), 0),
                    );
                    format!("{}{}", prefix, child.to_markdown())
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Node::ListItem {
                children, checked, ..
            } => {
                let checkbox = if let Some(checked) = checked {
                    if *checked {
                        "[x] "
                    } else {
                        "[ ] "
                    }
                } else {
                    ""
                };
                format!(
                    "{}{}",
                    checkbox,
                    children
                        .iter()
                        .map(|child| child.to_markdown())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
            Node::CodeBlock(code_block) => {
                format!(
                    "```{}\n{}\n```",
                    code_block.lang.clone().unwrap_or_default(),
                    code_block.code()
                )
            }
            Node::Table(table) => {
                let header = table
                    .children
                    .first()
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .unwrap_or_default();
                let alignments = table
                    .column_aligns
                    .iter()
                    .map(|align| {
                        match align {
                            ColumnumnAlign::Left => ":--",
                            ColumnumnAlign::Center => ":-:",
                            ColumnumnAlign::Right => "--:",
                        }
                        .to_string()
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                let rows = table
                    .children
                    .iter()
                    .skip(1)
                    .map(|row| {
                        row.children
                            .iter()
                            .map(|cell| cell.children.to_markdown())
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{}\n{}", header, alignments, rows)
            }
            Node::Break { html } => {
                if *html {
                    "<br>".to_string()
                } else {
                    "\n".to_string()
                }
            }
            Node::Divider => "---".to_string(),
            Node::Unknown => "".to_string(),
        }
        .trim()
        .to_string()
    }
}

impl Node {
    fn render_list_item(
        item: &Node,
        ix: usize,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        match item {
            Node::ListItem {
                children,
                spread,
                checked,
            } => v_flex()
                .id(("li", ix))
                .when(*spread, |this| this.child(div()))
                .children({
                    let mut items: Vec<Div> = Vec::with_capacity(children.len());

                    for (child_ix, child) in children.iter().enumerate() {
                        match child {
                            Node::Paragraph(paragraph) => {
                                let task_label: SharedString = paragraph
                                    .children
                                    .iter()
                                    .map(|node| node.text.as_str())
                                    .collect::<String>()
                                    .trim()
                                    .to_owned()
                                    .into();
                                let last_not_list = child_ix > 0
                                    && !matches!(children[child_ix - 1], Node::List { .. });

                                let text = child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                );

                                // merge content into last item.
                                if last_not_list {
                                    if let Some(item_item) = items.last_mut() {
                                        item_item.extend(vec![div()
                                            .overflow_hidden()
                                            .child(text)
                                            .into_any_element()]);
                                        continue;
                                    }
                                }

                                items.push(
                                    h_flex()
                                        .flex_1()
                                        .relative()
                                        .items_start()
                                        .content_start()
                                        .when(!options.todo && checked.is_none(), |this| {
                                            this.child(list_item_prefix(
                                                ix,
                                                options.list_start,
                                                options.depth,
                                            ))
                                        })
                                        .when_some(*checked, |this, checked| {
                                            // Todo list checkbox
                                            let marker = div()
                                                .flex()
                                                .mt(rems(0.4))
                                                .mr_1p5()
                                                .size(rems(0.875))
                                                .items_center()
                                                .justify_center()
                                                .rounded(cx.theme().radius.half())
                                                .border_1()
                                                .border_color(if checked {
                                                    cx.theme().primary
                                                } else {
                                                    cx.theme().input
                                                })
                                                .text_color(cx.theme().primary_foreground)
                                                .when(checked, |this| {
                                                    this.bg(cx.theme().primary).child(
                                                        Icon::new(IconName::Check)
                                                            .size_2()
                                                            .text_xs(),
                                                    )
                                                });
                                            let task_label = task_label.clone();
                                            this.child(gpui::accessible(
                                                ("task", child_ix),
                                                marker,
                                                move |_, _, _| {
                                                    let mut node = gpui::accesskit::Node::new(
                                                        gpui::accesskit::Role::CheckBox,
                                                    );
                                                    node.set_label(if task_label.is_empty() {
                                                        "Task".to_owned()
                                                    } else {
                                                        task_label.to_string()
                                                    });
                                                    node.set_read_only();
                                                    node.set_toggled(if checked {
                                                        gpui::accesskit::Toggled::True
                                                    } else {
                                                        gpui::accesskit::Toggled::False
                                                    });
                                                    gpui::AccessibilityInfo::new(node)
                                                },
                                            ))
                                        })
                                        .child(div().overflow_hidden().child(text)),
                                );
                            }
                            Node::Break { .. } | Node::Unknown => {}
                            _ => {
                                items.push(div().ml(rems(1.)).child(child.render_block(
                                    NodeRenderOptions {
                                        depth: options.depth + 1,
                                        todo: checked.is_some(),
                                        is_last: true,
                                        ..options
                                    },
                                    node_cx,
                                    window,
                                    cx,
                                )));
                            }
                        }
                    }
                    items
                })
                .map(|item| {
                    gpui::accessible(("list-item", ix), item, |_, _, _| {
                        gpui::AccessibilityInfo::new(gpui::accesskit::Node::new(
                            gpui::accesskit::Role::ListItem,
                        ))
                    })
                })
                .into_any_element(),
            _ => div().into_any_element(),
        }
    }

    fn render_table(
        item: &Node,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        const DEFAULT_LENGTH: usize = 5;
        const MAX_LENGTH: usize = 150;
        let col_lens = match item {
            Node::Table(table) => {
                let mut col_lens = vec![];
                for row in table.children.iter() {
                    for (ix, cell) in row.children.iter().enumerate() {
                        if col_lens.len() <= ix {
                            col_lens.push(DEFAULT_LENGTH);
                        }

                        let len = cell.children.text_len();
                        if len > col_lens[ix] {
                            col_lens[ix] = len;
                        }
                    }
                }
                col_lens
            }
            _ => vec![],
        };

        match item {
            Node::Table(table) => {
                let id = ("markdown-table", table as *const Table as usize);
                let controls = window.use_keyed_state(id, cx, |_, cx| {
                    (ScrollHandle::new(), cx.focus_handle().tab_stop(true))
                });
                let (scroll, focus) = controls.read(cx).clone();
                let input_scroll = scroll.clone();
                let input_focus = focus.clone();
                let layout_scroll = scroll.clone();
                let overflow = scroll.max_offset().width > px(0.);
                let scrollbar =
                    Scrollbar::horizontal(&scroll).scrollbar_show(ScrollbarShow::Always);
                div()
                .on_children_prepainted(move |_, window, _| {
                    if overflow != (layout_scroll.max_offset().width > px(0.)) {
                        window.request_animation_frame();
                    }
                })
                .id(id)
                .relative()
                .when(overflow, |table| table.pb(rems(1.)))
                .mb(rems(1.))
                .w_full()
                // Only a table that scrolls sideways needs to be a Tab stop.
                .when(overflow, |table| {
                    table
                        .track_focus(&focus)
                        .focus_ring(focus.is_focused(window), px(0.), window, cx)
                })
                .on_key_down(move |event, window, cx| {
                    let key = &event.keystroke;
                    if input_focus.is_focused(window)
                        && !key.modifiers.control
                        && !key.modifiers.platform
                        && !key.modifiers.alt
                        && Self::scroll_table(&input_scroll, &key.key)
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                        window.refresh();
                    }
                })
                .child(
                    div()
                        .id("table")
                        .w_full()
                        .overflow_x_scroll()
                        .track_scroll(&scroll)
                        .border_1()
                        .border_color(cx.theme().border)
                        .rounded(cx.theme().radius)
                        .children({
                            let mut rows = Vec::with_capacity(table.children.len());
                            for (row_ix, row) in table.children.iter().enumerate() {
                                rows.push(
                                    div()
                                        .id(("row", row_ix))
                                        .min_w_full()
                                        .w(rems(
                                            col_lens
                                                .iter()
                                                .map(|len| len.min(&MAX_LENGTH))
                                                .sum::<usize>()
                                                as f32
                                                * 0.6
                                                + col_lens.len() as f32,
                                        ))
                                        .when(row_ix < table.children.len() - 1, |this| {
                                            this.border_b_1()
                                        })
                                        .when(row_ix == 0, |this| {
                                            this.bg(cx.theme().muted)
                                                .rounded_t(cx.theme().radius)
                                                .font_semibold()
                                        })
                                        .border_color(cx.theme().border)
                                        .flex()
                                        .flex_row()
                                        .children({
                                            let mut cells = Vec::with_capacity(row.children.len());
                                            for (ix, cell) in row.children.iter().enumerate() {
                                                let align = table.column_align(ix);
                                                let len = col_lens
                                                    .get(ix)
                                                    .copied()
                                                    .unwrap_or(MAX_LENGTH)
                                                    .min(MAX_LENGTH);

                                                cells.push(
                                                    div()
                                                        .id(("cell", ix))
                                                        .flex()
                                                        .when(
                                                            align == ColumnumnAlign::Center,
                                                            |this| this.justify_center(),
                                                        )
                                                        .when(
                                                            align == ColumnumnAlign::Right,
                                                            |this| this.justify_end(),
                                                        )
                                                        .flex_shrink_0()
                                                        .flex_grow()
                                                        .w(rems(len as f32 * 0.6 + 1.))
                                                        .px_2()
                                                        .py_1()
                                                        .child(
                                                            cell.children.render(node_cx, window, cx),
                                                        )
                                                        .map(|rendered| {
                                                            gpui::accessible(
                                                                ("cell-role", ix),
                                                                rendered,
                                                                move |_, _, _| {
                                                                    let role = if row_ix == 0 {
                                                                        gpui::accesskit::Role::ColumnHeader
                                                                    } else {
                                                                        gpui::accesskit::Role::Cell
                                                                    };
                                                                    let mut node = gpui::accesskit::Node::new(role);
                                                                    node.set_row_index(row_ix);
                                                                    node.set_column_index(ix);
                                                                    gpui::AccessibilityInfo::new(node)
                                                                },
                                                            )
                                                            .into_any_element()
                                                        }),
                                                )
                                            }
                                            cells
                                        })
                                        .map(|rendered| {
                                            gpui::accessible(("row-role", row_ix), rendered, move |_, _, _| {
                                                let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Row);
                                                node.set_row_index(row_ix);
                                                gpui::AccessibilityInfo::new(node)
                                            })
                                        }),
                                )
                            }
                            rows
                        })
                        .map(|rendered| {
                            let row_count = table.children.len();
                            let column_count = col_lens.len();
                            let focus_action = focus.clone();
                            let scroll_action = scroll.clone();
                            gpui::accessible("table-role", rendered, move |_, window, _| {
                                use gpui::accesskit::{Action, Node, Role};
                                let mut node = Node::new(Role::Table);
                                node.set_row_count(row_count);
                                node.set_column_count(column_count);
                                node.add_action(Action::Focus);
                                if scroll.max_offset().width <= px(0.) {
                                    node.set_label("Table");
                                } else {
                                    node.set_label("Scrollable table");
                                    node.set_description("Use Left and Right to scroll. Home and End reach the first and last columns.");
                                    node.add_action(Action::ScrollLeft);
                                    node.add_action(Action::ScrollRight);
                                }
                                let mut info = gpui::AccessibilityInfo::new(node);
                                info.focused = focus.is_focused(window);
                                info
                            })
                            .on_checked_action(move |request, _, window, _| {
                                use gpui::accesskit::Action;
                                match request.action {
                                    Action::Focus => focus_action.focus(window),
                                    Action::ScrollLeft | Action::ScrollRight => {
                                        let key = if request.action == Action::ScrollLeft { "left" } else { "right" };
                                        if !Self::scroll_table(&scroll_action, key) {
                                            return false;
                                        }
                                        window.refresh();
                                    }
                                    _ => return false,
                                }
                                true
                            })
                        }),
                )
                .when(overflow, |table| {
                    table.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .right_0()
                            .h(rems(1.))
                            .child(scrollbar),
                    )
                })
                .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    fn scroll_table(scroll: &ScrollHandle, key: &str) -> bool {
        let maximum = scroll.max_offset().width;
        if maximum <= px(0.) {
            return false;
        }
        let mut offset = scroll.offset();
        let step = px(64.).min(scroll.bounds().size.width);
        offset.x = match key {
            "left" => (offset.x + step).min(px(0.)),
            "right" => (offset.x - step).max(-maximum),
            "home" => px(0.),
            "end" => -maximum,
            _ => return false,
        };
        scroll.set_offset(offset);
        true
    }

    pub(super) fn render_root(
        &self,
        list_state: Option<ListState>,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let options = NodeRenderOptions {
            is_last: true,
            ..Default::default()
        };

        let Some(list_state) = list_state else {
            return self
                .render_block(options, node_cx, window, cx)
                .into_any_element();
        };

        let children = match self {
            Node::Root { children } => children,
            _ => return div().into_any_element(),
        };

        let children = children.clone();
        let node_cx = node_cx.clone();

        if list_state.item_count() != children.len() {
            list_state.reset(children.len());
        }

        gpui::list(list_state, move |ix, window, cx| {
            let is_last = ix + 1 == children.len();
            div()
                .id(("block", ix))
                .child(children[ix].render_block(options.is_last(is_last), &node_cx, window, cx))
                .into_any_element()
        })
        .size_full()
        .into_any()
    }

    fn render_block(
        &self,
        options: NodeRenderOptions,
        node_cx: &NodeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let mb = if options.in_list || options.is_last {
            rems(0.)
        } else {
            node_cx.style.paragraph_gap
        };

        match self {
            Node::Root { children } => div()
                .id("div")
                .children(children.iter().enumerate().map(move |(index, node)| {
                    div()
                        .id(("block", index))
                        .child(node.render_block(options, node_cx, window, cx))
                }))
                .into_any_element(),
            Node::Paragraph(paragraph) | Node::ParagraphPage(paragraph) => div()
                .id("p")
                .when(matches!(self, Node::Paragraph(_)), |row| row.pb(mb))
                .when_some(node_cx.prose_width, |paragraph, width| {
                    paragraph.max_w(width)
                })
                .child(paragraph.render(node_cx, window, cx))
                .into_any_element(),
            Node::Heading { level, children } => {
                let body_size = window.text_style().font_size.to_pixels(window.rem_size());
                let (text_size, font_weight) = heading_style(*level, &node_cx.style, body_size);

                let level = *level;
                let label: String = children
                    .children
                    .iter()
                    .map(|node| node.text.as_str())
                    .collect();
                let heading = h_flex()
                    .id(("h", level as usize))
                    .pb(rems(0.3))
                    .whitespace_normal()
                    .text_size(text_size)
                    .font_weight(font_weight)
                    .child(children.render(node_cx, window, cx));
                gpui::accessible("heading", heading, move |_, _, _| {
                    let mut node = gpui::accesskit::Node::new(gpui::accesskit::Role::Heading);
                    node.set_level(level as usize);
                    node.set_label(label.clone());
                    gpui::AccessibilityInfo::new(node)
                })
                .into_any_element()
            }
            Node::Blockquote { children } => div()
                .w_full()
                .pb(mb)
                .child(
                    div()
                        .id("blockquote")
                        .w_full()
                        .text_color(cx.theme().muted_foreground)
                        .border_l_3()
                        .border_color(cx.theme().secondary_active)
                        .px_4()
                        .children({
                            let children_len = children.len();
                            children.into_iter().enumerate().map(move |(index, c)| {
                                let is_last = index == children_len - 1;
                                c.render_block(options.is_last(is_last), node_cx, window, cx)
                            })
                        }),
                )
                .into_any_element(),
            Node::List { children, start } => v_flex()
                .id(if start.is_some() { "ol" } else { "ul" })
                .pb(mb)
                .children({
                    let mut items = Vec::with_capacity(children.len());
                    let mut ix = 0;
                    for item in children.into_iter() {
                        let is_item = item.is_list_item();

                        items.push(Self::render_list_item(
                            item,
                            ix,
                            NodeRenderOptions {
                                list_start: *start,
                                ..options
                            },
                            node_cx,
                            window,
                            cx,
                        ));

                        if is_item {
                            ix += 1;
                        }
                    }
                    items
                })
                .map(|list| {
                    gpui::accessible("list", list, |_, _, _| {
                        gpui::AccessibilityInfo::new(gpui::accesskit::Node::new(
                            gpui::accesskit::Role::List,
                        ))
                    })
                })
                .into_any_element(),
            Node::CodeBlock(code_block) => code_block.render(&options, node_cx, window, cx),
            Node::Table { .. } => Self::render_table(self, node_cx, window, cx).into_any_element(),
            Node::Divider => div()
                .pb(mb)
                .child(div().id("divider").bg(cx.theme().border).h(px(2.)))
                .into_any_element(),
            Node::Break { .. } => div().id("break").into_any_element(),
            Node::Unknown => div().into_any_element(),
            _ => {
                if cfg!(debug_assertions) {
                    tracing::warn!("unknown implementation: {:?}", self);
                }

                div().into_any_element()
            }
        }
    }
}
#[cfg(test)]
mod reading_page_tests {
    use super::*;

    #[test]
    fn default_heading_sizes_keep_up_with_scaled_prose() {
        let style = TextViewStyle::default();
        assert_eq!(heading_style(1, &style, px(16.)).0, px(28.));
        assert_eq!(heading_style(1, &style, px(56.)).0, px(98.));
        assert_eq!(heading_style(2, &style, px(24.)).0, px(31.5));
        for level in 1..=6 {
            assert!(heading_style(level, &style, px(56.)).0 >= px(56.));
            assert!(heading_style(level, &style, px(12.)).0 >= px(12.));
        }
        let customized = style.heading_font_size(|_, _| px(34.));
        assert_eq!(heading_style(1, &customized, px(56.)).0, px(34.));
    }

    #[test]
    fn long_paragraph_pages_share_text_and_preserve_marks_and_copy() {
        let prefix = "A\u{301} 🪴 Café signal. ".repeat(1_000);
        let text: Arc<str> = format!("{prefix}Last signal.").into();
        let marked = "A\u{301} 🪴 ".len()..text.len() - 4;
        let link = TextMark::default().bold().link(LinkMark {
            url: "https://example.test".into(),
            ..Default::default()
        });
        let inline = InlineNode {
            text: InlineText::Shared {
                text: text.clone(),
                range: 0..text.len() as u32,
            },
            marks: vec![(marked.clone(), link.clone())].into(),
            ..Default::default()
        };
        let original = Paragraph {
            children: vec![inline],
            ..Default::default()
        };
        let mut root = Node::Root {
            children: Arc::new(vec![Node::Paragraph(original)]),
        };
        root.paginate_reading_blocks();
        let Node::Root { children } = &root else {
            panic!()
        };
        assert!(children.len() > 4);
        let mut offset = 0;
        for (index, row) in children.iter().enumerate() {
            let paragraph = match row {
                Node::ParagraphPage(paragraph) if index + 1 != children.len() => paragraph,
                Node::Paragraph(paragraph) if index + 1 == children.len() => paragraph,
                _ => panic!("unexpected paragraph page"),
            };
            assert!(paragraph.text_len() <= READING_PAGE_BYTES);
            let [inline] = paragraph.children.as_slice() else {
                panic!()
            };
            let InlineText::Shared { text: source, .. } = &inline.text else {
                panic!()
            };
            assert!(Arc::ptr_eq(source, &text));
            let end = offset + inline.text.len();
            assert_eq!(inline.text.as_str(), &text[offset..end]);
            let start = marked.start.max(offset);
            let mark_end = marked.end.min(end);
            if start < mark_end {
                assert_eq!(
                    &*inline.marks,
                    &[(start - offset..mark_end - offset, link.clone())]
                );
            } else {
                assert!(inline.marks.is_empty());
            }
            let state = paragraph.state.get_or_init(Arc::default);
            let mut state = state.lock().unwrap();
            state.set_text(inline.text.as_str().to_owned().into());
            state.selection = Some(crate::input::Selection::new(0, inline.text.len()));
            offset = end;
        }
        assert_eq!(offset, text.len());
        assert_eq!(root.selected_text(), format!("{text}\n\n"));
    }

    #[test]
    fn long_code_pages_share_source_and_preserve_styles_and_selection() {
        let prefix = "let start = 1;\r\n";
        let long_line = "A\u{301} 🪴 β code ".repeat(1_500);
        let text = SharedString::new(format!("{prefix}{long_line}\nlast line\n"));
        let emphasis = HighlightStyle {
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        };
        let marked = prefix.len()..prefix.len() + long_line.len();
        let mut state = InlineState::default();
        state.set_text(text.clone());
        let code = CodeBlock {
            lang: Some("text".into()),
            styles: vec![(marked.clone(), emphasis)],
            state: Arc::new(Mutex::new(state)),
            page: None,
            pages: None,
            trailing_newline: false,
        };
        let mut quote = Node::Blockquote {
            children: vec![Node::CodeBlock(code.clone())],
        };
        quote.paginate_nested_code();
        assert!(quote.has_nested_code_pages());
        let Node::Blockquote { children: quote } = &quote else {
            panic!()
        };
        let [Node::CodeBlock(nested)] = quote.as_slice() else {
            panic!()
        };
        assert!(std::ptr::eq(nested.code().as_str(), text.as_str()));
        assert!(nested.pages.as_ref().is_some_and(|pages| pages.len() > 4));
        let mut root = Node::Root {
            children: Arc::new(vec![Node::CodeBlock(code)]),
        };
        root.paginate_reading_blocks();
        let Node::Root { children } = &root else {
            panic!()
        };
        assert!(children.len() > 4);
        let mut offset = 0;
        for (index, row) in children.iter().enumerate() {
            let Node::CodeBlock(code) = row else { panic!() };
            let page = code.page.as_ref().unwrap();
            assert!(std::ptr::eq(page.source.as_str(), text.as_str()));
            assert_eq!(page.range.start, offset);
            assert!(page.range.len() <= READING_PAGE_BYTES);
            assert_eq!(code.starts_block(), index == 0);
            assert_eq!(code.ends_block(), index + 1 == children.len());
            assert_eq!(code.code().as_str(), &text[page.range.clone()]);
            assert_eq!(
                code.display_code().as_str(),
                &page.text()[..page.text().len() - page.break_after().len()]
            );
            let start = marked.start.max(page.range.start);
            let end = marked.end.min(page.range.end);
            if start < end {
                assert_eq!(code.styles, vec![(start - offset..end - offset, emphasis)]);
            } else {
                assert!(code.styles.is_empty());
            }
            let mut state = code.state.lock().unwrap();
            state.selection = Some(crate::input::Selection::new(0, state.text.len()));
            offset = page.range.end;
        }
        assert_eq!(offset, text.len());
        assert_eq!(root.selected_text(), format!("{text}\n\n"));
    }
}
