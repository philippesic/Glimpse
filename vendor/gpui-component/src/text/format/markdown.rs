use gpui::SharedString;
use markdown::{
    mdast::{self, Node},
    ParseOptions,
};

use crate::{
    highlighter::HighlightTheme,
    text::{
        node::{
            self, CodeBlock, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph, Span, Table,
            TableRow, TextMark,
        },
        TextViewStyle,
    },
};

/// Parse Markdown into a tree of nodes.
pub(crate) fn parse(
    raw: &str,
    style: &TextViewStyle,
    cx: &mut NodeContext,
    highlight_theme: &HighlightTheme,
) -> Result<node::Node, SharedString> {
    let mut options = ParseOptions::gfm();
    options.constructs.gfm_footnote_definition = false;
    options.constructs.gfm_label_start_footnote = false;
    markdown::to_mdast(&raw, &options)
        .map(|n| ast_to_node(n, style, cx, highlight_theme))
        .map_err(|e| e.to_string().into())
}

fn parse_table_row(table: &mut Table, node: &mdast::TableRow) {
    let mut row = TableRow::default();
    node.children.iter().for_each(|c| {
        match c {
            Node::TableCell(cell) => {
                parse_table_cell(&mut row, cell);
            }
            _ => {}
        };
    });
    table.children.push(row);
}

fn parse_table_cell(row: &mut node::TableRow, node: &mdast::TableCell) {
    let mut paragraph = Paragraph::default();
    node.children.iter().for_each(|c| {
        parse_paragraph(&mut paragraph, c);
    });
    let table_cell = node::TableCell {
        children: paragraph,
        ..Default::default()
    };
    row.children.push(table_cell);
}

fn parse_paragraph(paragraph: &mut Paragraph, node: &mdast::Node) -> String {
    let span = node.position().map(|pos| Span {
        start: pos.start.offset,
        end: pos.end.offset,
    });
    if let Some(span) = span {
        paragraph.set_span(span);
    }

    let mut text = String::new();

    match node {
        Node::Paragraph(val) => {
            val.children.iter().for_each(|c| {
                text.push_str(&parse_paragraph(paragraph, c));
            });
        }
        Node::Text(val) => {
            text = val.value.clone();
            paragraph.push_str(&val.value)
        }
        Node::Emphasis(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in &val.children {
                text.push_str(&parse_paragraph(&mut child_paragraph, child));
            }
            for child in &mut child_paragraph.children {
                child
                    .marks
                    .push((0..child.text.len(), TextMark::default().italic()));
            }
            paragraph.merge(child_paragraph);
        }
        Node::Strong(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in &val.children {
                text.push_str(&parse_paragraph(&mut child_paragraph, child));
            }
            for child in &mut child_paragraph.children {
                child
                    .marks
                    .push((0..child.text.len(), TextMark::default().bold()));
            }
            paragraph.merge(child_paragraph);
        }
        Node::Delete(val) => {
            let mut child_paragraph = Paragraph::default();
            for child in &val.children {
                text.push_str(&parse_paragraph(&mut child_paragraph, child));
            }
            for child in &mut child_paragraph.children {
                child
                    .marks
                    .push((0..child.text.len(), TextMark::default().strikethrough()));
            }
            paragraph.merge(child_paragraph);
        }
        Node::InlineCode(val) => {
            text = val.value.clone();
            paragraph.push(
                InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        Node::Link(val) => {
            let link_mark = Some(LinkMark {
                url: val.url.clone().into(),
                title: val.title.clone().map(|s| s.into()),
                ..Default::default()
            });

            let mut child_paragraph = Paragraph::default();
            for child in val.children.iter() {
                text.push_str(&parse_paragraph(&mut child_paragraph, &child));
            }

            // FIXME: GPUI InteractiveText does not support inline images yet.
            // So here we push images to the paragraph directly.
            for child in child_paragraph.children.iter_mut() {
                if let Some(image) = child.image.as_mut() {
                    image.link = link_mark.clone();
                }

                child.marks.push((
                    0..child.text.len(),
                    TextMark {
                        link: link_mark.clone(),
                        ..Default::default()
                    },
                ));
            }

            paragraph.merge(child_paragraph);
        }
        Node::Image(raw) => {
            paragraph.push_image(ImageNode {
                url: raw.url.clone().into(),
                title: raw.title.clone().map(|t| t.into()),
                alt: Some(raw.alt.clone().into()),
                ..Default::default()
            });
        }
        Node::InlineMath(raw) => {
            text = raw.value.clone();
            paragraph.push(
                InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        Node::MdxTextExpression(raw) => {
            text = raw.value.clone();
            paragraph
                .push(InlineNode::new(&text).marks(vec![(0..text.len(), TextMark::default())]));
        }
        Node::Html(val) => {
            text = val.value.clone();
            paragraph.push_str(&text);
        }
        Node::FootnoteReference(foot) => {
            let prefix = format!("[{}]", foot.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));
        }
        Node::LinkReference(link) => {
            let mut child_paragraph = Paragraph::default();
            for child in &link.children {
                text.push_str(&parse_paragraph(&mut child_paragraph, child));
            }
            let link_mark = LinkMark {
                url: "".into(),
                title: link.label.clone().map(Into::into),
                identifier: Some(link.identifier.clone().into()),
            };
            for child in &mut child_paragraph.children {
                if let Some(image) = &mut child.image {
                    image.link = Some(link_mark.clone());
                }
                child.marks.push((
                    0..child.text.len(),
                    TextMark {
                        link: Some(link_mark.clone()),
                        ..Default::default()
                    },
                ));
            }
            paragraph.merge(child_paragraph);
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported inline node: {:#?}", node);
            }
        }
    }

    text
}

fn ast_to_node(
    value: mdast::Node,
    style: &TextViewStyle,
    cx: &mut NodeContext,
    highlight_theme: &HighlightTheme,
) -> node::Node {
    match value {
        Node::Root(val) => {
            let children: Vec<_> = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::Root {
                children: children.into(),
            }
        }
        Node::Paragraph(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c);
            });

            node::Node::Paragraph(paragraph)
        }
        Node::Blockquote(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::Blockquote { children }
        }
        Node::List(list) => {
            let children = list
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::List {
                ordered: list.ordered,
                children,
            }
        }
        Node::ListItem(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(c, style, cx, highlight_theme))
                .collect();
            node::Node::ListItem {
                children,
                spread: val.spread,
                checked: val.checked,
            }
        }
        Node::Break(_) => node::Node::Break { html: false },
        Node::Code(raw) => node::Node::CodeBlock(CodeBlock::new(
            raw.value.into(),
            raw.lang.map(|s| s.into()),
            style,
            highlight_theme,
        )),
        Node::Heading(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c);
            });

            node::Node::Heading {
                level: val.depth,
                children: paragraph,
            }
        }
        Node::Math(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            None,
            style,
            highlight_theme,
        )),
        Node::Html(val) => node::Node::Paragraph(Paragraph::new(val.value)),
        Node::MdxFlowExpression(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("mdx".into()),
            style,
            highlight_theme,
        )),
        Node::Yaml(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("yml".into()),
            style,
            highlight_theme,
        )),
        Node::Toml(val) => node::Node::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("toml".into()),
            style,
            highlight_theme,
        )),
        Node::MdxJsxTextElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::MdxJsxFlowElement(val) => {
            let mut paragraph = Paragraph::default();
            val.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::ThematicBreak(_) => node::Node::Divider,
        Node::Table(val) => {
            let mut table = Table::default();
            table.column_aligns = val
                .align
                .clone()
                .into_iter()
                .map(|align| align.into())
                .collect();
            val.children.iter().for_each(|c| {
                if let Node::TableRow(row) = c {
                    parse_table_row(&mut table, row);
                }
            });

            node::Node::Table(table)
        }
        Node::FootnoteDefinition(def) => {
            let mut paragraph = Paragraph::default();
            let prefix = format!("[{}]: ", def.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));

            def.children.iter().for_each(|c| {
                parse_paragraph(&mut paragraph, c);
            });
            node::Node::Paragraph(paragraph)
        }
        Node::Definition(def) => {
            cx.add_ref(
                def.identifier.clone().into(),
                LinkMark {
                    url: def.url.clone().into(),
                    identifier: Some(def.identifier.clone().into()),
                    title: def.title.clone().map(Into::into),
                },
            );

            node::Node::Definition {
                identifier: def.identifier.clone().into(),
                url: def.url.clone().into(),
                title: def.title.clone().map(|s| s.into()),
            }
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported node: {:#?}", value);
            }
            node::Node::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_inline_marks_keep_links_code_and_images() {
        let ast = markdown::to_mdast(
            "**outer *inner [link](next.md)* `code` ![alt](local.png)**",
            &ParseOptions::gfm(),
        )
        .unwrap();
        let Node::Root(root) = ast else {
            panic!("expected root")
        };
        let mut paragraph = Paragraph::default();
        parse_paragraph(&mut paragraph, &root.children[0]);
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
        let ast = markdown::to_mdast(
            "[*nested **bold** ![alt](local.png)*][target]\n\n[target]: next.md",
            &ParseOptions::gfm(),
        )
        .unwrap();
        let Node::Root(root) = ast else {
            panic!("expected root")
        };
        let mut paragraph = Paragraph::default();
        parse_paragraph(&mut paragraph, &root.children[0]);
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
}
