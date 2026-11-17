use crate::{
    highlighter::HighlightTheme,
    text::{
        TextViewStyle,
        node::{
            self, CodeBlock, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph, Span, Table,
            TableCell, TableRow, TextMark,
        },
    },
};
use gpui::SharedString;
use linkify::{LinkFinder, LinkKind};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

enum Block {
    Root,
    Paragraph,
    Heading(u8),
    Quote,
    List(bool),
    Item,
    Code(Option<SharedString>),
    Html,
    Table(Vec<pulldown_cmark::Alignment>),
    Row,
    Cell,
}
struct Frame {
    kind: Block,
    start: usize,
    children: Vec<node::Node>,
    paragraph: Paragraph,
    buffer: String,
    rows: Vec<TableRow>,
    cells: Vec<TableCell>,
    checked: Option<bool>,
    spread: bool,
}
fn inline(text: String, marks: &[TextMark]) -> InlineNode {
    let length = text.len();
    InlineNode::new(text).marks(marks.iter().map(|mark| (0..length, mark.clone())).collect())
}
impl Frame {
    fn new(kind: Block, start: usize) -> Self {
        Self {
            kind,
            start,
            children: vec![],
            paragraph: Paragraph::default(),
            buffer: String::new(),
            rows: vec![],
            cells: vec![],
            checked: None,
            spread: false,
        }
    }
    fn flush(&mut self, marks: &[TextMark], finder: &LinkFinder) {
        if self.buffer.is_empty() || matches!(self.kind, Block::Code(_) | Block::Html) {
            return;
        }
        let text = std::mem::take(&mut self.buffer);
        if marks.iter().any(|mark| mark.code || mark.link.is_some())
            || !text.contains([':', '@']) && !text.contains("www.")
        {
            self.paragraph.push(inline(text, marks));
            return;
        }
        let mut offset = 0;
        for link in finder.links(&text) {
            let url = match link.kind() {
                LinkKind::Email => format!("mailto:{}", link.as_str()),
                LinkKind::Url if link.as_str().starts_with("www.") => {
                    format!("https://{}", link.as_str())
                }
                LinkKind::Url
                    if link.as_str().starts_with("http://")
                        || link.as_str().starts_with("https://") =>
                {
                    link.as_str().to_owned()
                }
                _ => continue,
            };
            if link.start() > offset {
                self.paragraph
                    .push(inline(text[offset..link.start()].to_owned(), marks));
            }
            let mut link_marks = marks.to_vec();
            link_marks.push(TextMark::default().link(LinkMark {
                url: url.into(),
                ..Default::default()
            }));
            self.paragraph
                .push(inline(link.as_str().to_owned(), &link_marks));
            offset = link.end();
        }
        if offset < text.len() {
            self.paragraph
                .push(inline(text[offset..].to_owned(), marks));
        }
    }
    fn finish_pending(&mut self, marks: &[TextMark], finder: &LinkFinder) {
        self.flush(marks, finder);
        if !self.paragraph.is_empty() {
            self.children
                .push(node::Node::Paragraph(self.paragraph.take()));
        }
    }
    fn finish(
        mut self,
        end: usize,
        marks: &[TextMark],
        finder: &LinkFinder,
        style: &TextViewStyle,
        theme: &HighlightTheme,
    ) -> node::Node {
        self.flush(marks, finder);
        self.paragraph.set_span(Span {
            start: self.start,
            end,
        });
        match self.kind {
            Block::Paragraph | Block::Cell => node::Node::Paragraph(self.paragraph),
            Block::Heading(level) => node::Node::Heading {
                level,
                children: self.paragraph,
            },
            Block::Code(lang) => {
                node::Node::CodeBlock(CodeBlock::new(self.buffer.into(), lang, style, theme))
            }
            Block::Html => node::Node::Paragraph(Paragraph::new(
                self.buffer.trim_end_matches(['\r', '\n']).to_owned(),
            )),
            Block::Table(alignments) => node::Node::Table(Table {
                children: self.rows,
                column_aligns: alignments.into_iter().map(Into::into).collect(),
            }),
            _ => {
                self.finish_pending(marks, finder);
                match self.kind {
                    Block::Root => node::Node::Root {
                        children: self.children.into(),
                    },
                    Block::Quote => node::Node::Blockquote {
                        children: self.children,
                    },
                    Block::List(ordered) => node::Node::List {
                        ordered,
                        children: self.children,
                    },
                    Block::Item => node::Node::ListItem {
                        children: self.children,
                        spread: self.spread,
                        checked: self.checked,
                    },
                    _ => node::Node::Unknown,
                }
            }
        }
    }
}
struct Image {
    image: ImageNode,
    alt: String,
}
pub(crate) fn parse(
    raw: &str,
    style: &TextViewStyle,
    cx: &mut NodeContext,
    theme: &HighlightTheme,
) -> Result<node::Node, SharedString> {
    let mut finder = LinkFinder::new();
    finder.url_must_have_scheme(false);
    let mut frames = vec![Frame::new(Block::Root, 0)];
    let mut marks = Vec::new();
    let mut images: Vec<Image> = Vec::new();
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    for (event, range) in Parser::new_ext(raw, options).into_offset_iter() {
        match event {
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                frames.last_mut().unwrap().flush(&marks, &finder);
                images.push(Image {
                    image: ImageNode {
                        url: dest_url.into_string().into(),
                        title: (!title.is_empty()).then(|| title.into_string().into()),
                        link: marks
                            .iter()
                            .rev()
                            .find_map(|mark: &TextMark| mark.link.clone()),
                        ..Default::default()
                    },
                    alt: String::new(),
                });
            }
            Event::End(TagEnd::Image) => {
                if let Some(mut image) = images.pop() {
                    if let Some(parent) = images.last_mut() {
                        parent.alt.push_str(&image.alt);
                    } else {
                        image.image.alt = Some(image.alt.into());
                        frames.last_mut().unwrap().paragraph.push_image(image.image);
                    }
                }
            }
            Event::Start(
                tag @ (Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Link { .. }),
            ) => {
                frames.last_mut().unwrap().flush(&marks, &finder);
                let mark = match tag {
                    Tag::Emphasis => TextMark::default().italic(),
                    Tag::Strong => TextMark::default().bold(),
                    Tag::Strikethrough => TextMark::default().strikethrough(),
                    Tag::Link {
                        dest_url,
                        title,
                        id,
                        ..
                    } => {
                        let link = LinkMark {
                            url: dest_url.into_string().into(),
                            title: (!title.is_empty()).then(|| title.into_string().into()),
                            identifier: (!id.is_empty()).then(|| id.into_string().into()),
                        };
                        if let Some(id) = &link.identifier {
                            cx.add_ref(id.clone(), link.clone());
                        }
                        TextMark::default().link(link)
                    }
                    _ => unreachable!(),
                };
                marks.push(mark);
            }
            Event::End(
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                frames.last_mut().unwrap().flush(&marks, &finder);
                marks.pop();
            }
            Event::Start(tag) => {
                frames.last_mut().unwrap().finish_pending(&marks, &finder);
                let kind = match tag {
                    Tag::Paragraph => {
                        if matches!(frames.last().unwrap().kind, Block::Item) {
                            frames.last_mut().unwrap().spread = true;
                        }
                        Block::Paragraph
                    }
                    Tag::Heading { level, .. } => Block::Heading(level as u8),
                    Tag::BlockQuote(_) => Block::Quote,
                    Tag::List(start) => Block::List(start.is_some()),
                    Tag::Item => Block::Item,
                    Tag::CodeBlock(code) => Block::Code(match code {
                        CodeBlockKind::Indented => None,
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .map(|language| SharedString::from(language.to_owned())),
                    }),
                    Tag::HtmlBlock => Block::Html,
                    Tag::Table(alignments) => Block::Table(alignments),
                    Tag::TableHead | Tag::TableRow => Block::Row,
                    Tag::TableCell => Block::Cell,
                    _ => return Err("Unsupported Markdown block; source text is unchanged.".into()),
                };
                frames.push(Frame::new(kind, range.start));
            }
            Event::End(tag) => {
                let mut frame = frames.pop().unwrap();
                if matches!(tag, TagEnd::TableHead | TagEnd::TableRow) {
                    frames.last_mut().unwrap().rows.push(TableRow {
                        children: frame.cells,
                    });
                } else if tag == TagEnd::TableCell {
                    frame.flush(&marks, &finder);
                    frame.paragraph.set_span(Span {
                        start: frame.start,
                        end: range.end,
                    });
                    frames.last_mut().unwrap().cells.push(TableCell {
                        children: frame.paragraph,
                        width: None,
                    });
                } else {
                    frames
                        .last_mut()
                        .unwrap()
                        .children
                        .push(frame.finish(range.end, &marks, &finder, style, theme));
                }
            }
            Event::Text(text) => {
                if let Some(image) = images.last_mut() {
                    image.alt.push_str(&text);
                } else {
                    frames.last_mut().unwrap().buffer.push_str(&text);
                }
            }
            Event::Code(text) | Event::InlineMath(text) => {
                if let Some(image) = images.last_mut() {
                    image.alt.push_str(&text);
                } else {
                    let frame = frames.last_mut().unwrap();
                    frame.flush(&marks, &finder);
                    let mut code_marks = marks.clone();
                    code_marks.push(TextMark::default().code());
                    frame
                        .paragraph
                        .push(inline(text.into_string(), &code_marks));
                }
            }
            Event::Html(text) => frames.last_mut().unwrap().buffer.push_str(&text),
            Event::InlineHtml(text) => {
                if let Some(image) = images.last_mut() {
                    image.alt.push_str(&text);
                    continue;
                }
                let frame = frames.last_mut().unwrap();
                frame.flush(&marks, &finder);
                frame.paragraph.push(inline(text.into_string(), &marks));
            }
            Event::SoftBreak | Event::HardBreak => {
                let text = if matches!(event, Event::SoftBreak) {
                    " "
                } else {
                    "\n"
                };
                if let Some(image) = images.last_mut() {
                    image.alt.push_str(text);
                } else {
                    frames.last_mut().unwrap().buffer.push_str(text);
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(frame) = frames
                    .iter_mut()
                    .rev()
                    .find(|frame| matches!(frame.kind, Block::Item))
                {
                    frame.checked = Some(checked);
                }
            }
            Event::Rule => {
                let frame = frames.last_mut().unwrap();
                frame.finish_pending(&marks, &finder);
                frame.children.push(node::Node::Divider);
            }
            Event::FootnoteReference(label) => frames
                .last_mut()
                .unwrap()
                .buffer
                .push_str(&format!("[^{label}]")),
            Event::DisplayMath(text) => frames.last_mut().unwrap().buffer.push_str(&text),
        }
    }
    Ok(frames
        .pop()
        .unwrap()
        .finish(raw.len(), &marks, &finder, style, theme))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paragraph(source: &str) -> Paragraph {
        let style = TextViewStyle::default();
        let mut context = NodeContext::default();
        let node::Node::Root { children } =
            parse(source, &style, &mut context, &style.highlight_theme).unwrap()
        else {
            panic!("expected root")
        };
        let node::Node::Paragraph(paragraph) = &children[0] else {
            panic!("expected paragraph")
        };
        paragraph.clone()
    }
    #[test]
    fn nested_inline_marks_keep_links_code_and_images() {
        let paragraph = paragraph("**outer *inner [link](next.md)* `code` ![alt](local.png)**");
        let link = paragraph
            .children
            .iter()
            .find(|child| child.text.as_str() == "link")
            .unwrap();
        assert!(link.marks.iter().any(|(_, mark)| mark.bold));
        assert!(link.marks.iter().any(|(_, mark)| mark.italic));
        assert!(link.marks.iter().any(|(_, mark)| mark
            .link
            .as_ref()
            .is_some_and(|link| link.url.as_str() == "next.md")));
        let code = paragraph
            .children
            .iter()
            .find(|child| child.text.as_str() == "code")
            .unwrap();
        assert!(code.marks.iter().any(|(_, mark)| mark.bold));
        assert!(code.marks.iter().any(|(_, mark)| mark.code));
        assert!(paragraph.children.iter().any(|child| child
            .image
            .as_ref()
            .is_some_and(|image| image.url.as_str() == "local.png")));
    }

    #[test]
    fn reference_links_keep_nested_formatting_and_linked_images() {
        let paragraph =
            paragraph("[*nested **bold** ![alt](local.png)*][target]\n\n[target]: next.md");
        let bold = paragraph
            .children
            .iter()
            .find(|child| child.text.as_str() == "bold")
            .unwrap();
        assert!(bold.marks.iter().any(|(_, mark)| mark.bold));
        assert!(bold.marks.iter().any(|(_, mark)| mark.italic));
        assert!(bold
            .marks
            .iter()
            .any(|(_, mark)| mark.link.as_ref().is_some_and(|link| link
                .identifier
                .as_ref()
                .is_some_and(|identifier| identifier.as_str() == "target"))));
        assert!(paragraph
            .children
            .iter()
            .any(|child| child.image.as_ref().is_some_and(|image| image
                .link
                .as_ref()
                .is_some_and(|link| link
                    .identifier
                    .as_ref()
                    .is_some_and(|identifier| identifier.as_str() == "target")))));
    }

    #[test]
    fn declared_dialect_keeps_html_literal() {
        let style = TextViewStyle::default();
        let mut context = NodeContext::default();
        let node=parse("<script>raw</script>\n\n**bold** *italic* ~~strike~~ [link](next.md)\n\n- [x] done\n\n| left | right |\n| --- | --- |\n| a | b |",&style,&mut context,&style.highlight_theme).unwrap();
        let node::Node::Root { children } = node else {
            panic!("expected blocks")
        };
        let node::Node::Paragraph(html) = &children[0] else {
            panic!("expected literal HTML")
        };
        assert_eq!(html.children[0].text.as_str(), "<script>raw</script>");
        assert!(children
            .iter()
            .any(|node| matches!(node, node::Node::Table(_))));
        assert!(children
            .iter()
            .any(|node| matches!(node, node::Node::List { .. })));
        assert!(context.link_refs.is_empty());
    }
    #[test]
    fn autolinks_preserve_punctuation_and_exclude_inline_code() {
        let source = "Visit http://example.test/a_(b). www.example.test a@example.test and `https://code.example.test`.";
        let style = TextViewStyle::default();
        let mut context = NodeContext::default();
        let root = parse(source, &style, &mut context, &style.highlight_theme).unwrap();
        let mut links = vec![];
        root.links(&context, &mut links);
        assert_eq!(
            links
                .iter()
                .map(|(_, url)| url.as_str())
                .collect::<Vec<_>>(),
            vec![
                "http://example.test/a_(b)",
                "https://www.example.test",
                "mailto:a@example.test"
            ]
        );
        let text = paragraph(source);
        let code = text
            .children
            .iter()
            .find(|node| node.text.as_str() == "https://code.example.test")
            .unwrap();
        assert!(code.marks.iter().any(|(_, mark)| mark.code));
        assert!(code.marks.iter().all(|(_, mark)| mark.link.is_none()));
        assert_eq!(text.children.iter().map(|node| node.text.as_str()).collect::<String>(), "Visit http://example.test/a_(b). www.example.test a@example.test and https://code.example.test.");
    }
    #[test]
    fn nested_blocks_keep_tasks_code_and_table_alignment() {
        let source = "# Title\n\n> quote **strong**\n>\n> 3. nested\n\n- [x] done\n\n```text\n<raw>&code\n```\n\n|a|b|c|\n|:---|:---:|---:|\n|left|middle|right|";
        let style = TextViewStyle::default();
        let mut context = NodeContext::default();
        let node::Node::Root { children } =
            parse(source, &style, &mut context, &style.highlight_theme).unwrap()
        else {
            panic!("expected blocks")
        };
        assert!(matches!(children[0], node::Node::Heading { level: 1, .. }));
        let node::Node::Blockquote { children: quote } = &children[1] else {
            panic!("expected quote")
        };
        assert!(quote
            .iter()
            .any(|node| matches!(node, node::Node::List { ordered: true, .. })));
        let node::Node::List {
            children: tasks, ..
        } = &children[2]
        else {
            panic!("expected tasks")
        };
        assert!(matches!(
            tasks[0],
            node::Node::ListItem {
                checked: Some(true),
                ..
            }
        ));
        let node::Node::CodeBlock(code) = &children[3] else {
            panic!("expected code")
        };
        assert_eq!(code.code().as_str(), "<raw>&code\n");
        let node::Node::Table(table) = &children[4] else {
            panic!("expected table")
        };
        assert_eq!(
            table.column_aligns,
            vec![
                node::ColumnumnAlign::Left,
                node::ColumnumnAlign::Center,
                node::ColumnumnAlign::Right
            ]
        );
        assert_eq!(table.children.len(), 2);
        assert_eq!(
            table.children[1]
                .children
                .iter()
                .map(|cell| cell
                    .children
                    .children
                    .iter()
                    .map(|node| node.text.as_str())
                    .collect::<String>())
                .collect::<Vec<_>>(),
            vec!["left", "middle", "right"]
        );
    }
    #[test]
    fn soft_and_hard_breaks_keep_distinct_reading_flow() {
        let paragraph = paragraph("first\nsecond  \nthird");
        assert_eq!(
            paragraph
                .children
                .iter()
                .map(|node| node.text.as_str())
                .collect::<String>(),
            "first second\nthird"
        );
    }
    #[test]
    #[ignore = "release parser CPU and memory probe; excludes native rendering and presentation"]
    fn benchmark_supported_markdown() {
        let unit = "Paragraph café 🙂 with **strong**, *emphasis* and `inline` plus [sibling](sibling.md). ".to_owned() + &"ordinary text ".repeat(12) + "\n\n";
        let repetitions = (10 * 1024 * 1024 - 4096) / unit.len();
        let source = "# Supported Markdown size\n\n".to_owned()
            + &unit.repeat(repetitions)
            + "\n# FINAL SUPPORTED SIZE MARKER\n";
        assert!(source.len() <= 10 * 1024 * 1024);
        let style = TextViewStyle::default();
        let mut samples = Vec::new();
        for _ in 0..10 {
            let mut context = NodeContext::default();
            let started = std::time::Instant::now();
            let parsed = parse(&source, &style, &mut context, &style.highlight_theme).unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.);
            let node::Node::Root { children } = &parsed else {
                panic!("expected blocks")
            };
            assert_eq!(children.len(), repetitions + 2);
            let node::Node::Heading {
                children: heading, ..
            } = children.last().unwrap()
            else {
                panic!("expected final heading")
            };
            assert_eq!(
                heading
                    .children
                    .iter()
                    .map(|node| node.text.as_str())
                    .collect::<String>(),
                "FINAL SUPPORTED SIZE MARKER"
            );
            std::hint::black_box(parsed);
        }
        samples.sort_by(f64::total_cmp);
        println!("native-node Markdown parse: bytes={}, n=10, p50={:.3}ms, p95={:.3}ms; excludes rendering/presentation", source.len(), samples[4], samples[9]);
        #[cfg(target_os = "linux")]
        println!(
            "{}",
            std::fs::read_to_string("/proc/self/status")
                .unwrap()
                .lines()
                .filter(|line| line.starts_with("VmRSS:") || line.starts_with("VmHWM:"))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
}
