//! Expand Org's literal verse content into a document-owned inline syntax tree.
//!
//! Orgize remains the parser and exporter authority. A prose sentinel on every
//! line prevents block parsing without breaking cross-line inline objects; the
//! sentinel is removed from tokens before export. Rebuilding the whole document
//! gives every anonymous footnote one unique range, including outside verse.

use std::ops::Range;

use orgize::rowan::{GreenNode, GreenToken, NodeOrToken, ast::AstNode};
use orgize::{SyntaxKind, SyntaxNode, SyntaxToken};

type GreenElement = NodeOrToken<GreenNode, GreenToken>;

struct LinePrefix {
    sentinel: Range<usize>,
    indentation: Range<usize>,
}

pub(crate) fn expand(document: &SyntaxNode) -> SyntaxNode {
    SyntaxNode::new_root(rebuild(document))
}

fn raw_kind(kind: SyntaxKind) -> orgize::rowan::SyntaxKind {
    orgize::rowan::SyntaxKind(kind as u16)
}

fn rebuild(node: &SyntaxNode) -> GreenNode {
    let children = node
        .children_with_tokens()
        .map(|element| match element {
            NodeOrToken::Node(child) => NodeOrToken::Node(
                if node.kind() == SyntaxKind::VERSE_BLOCK
                    && child.kind() == SyntaxKind::BLOCK_CONTENT
                {
                    verse_content(&child)
                } else {
                    rebuild(&child)
                },
            ),
            NodeOrToken::Token(token) => NodeOrToken::Token(token.green().to_owned()),
        })
        .collect::<Vec<_>>();
    GreenNode::new(raw_kind(node.kind()), children)
}

fn verse_content(content: &SyntaxNode) -> GreenNode {
    let source = content.to_string();
    let indent = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(indentation)
        .min()
        .unwrap_or(0);
    let mut protected = String::new();
    let mut sentinels = Vec::new();
    for line in source.lines() {
        let start = protected.len();
        protected.push_str("x ");
        protected.push_str(&" ".repeat(indentation(line).saturating_sub(indent)));
        sentinels.push(LinePrefix {
            sentinel: start..start + 2,
            indentation: start + 2..protected.len(),
        });
        protected.push_str(line.trim_start_matches([' ', '\t']));
        protected.push('\n');
    }
    if protected.is_empty() {
        return content.green().into_owned();
    }
    let parsed = orgize::Org::parse(protected);
    let Some(paragraph) = parsed.first_node::<orgize::ast::Paragraph>() else {
        unreachable!("prose-prefixed verse lines form a paragraph");
    };
    let children = inline_children(paragraph.syntax(), &sentinels, false);
    GreenNode::new(raw_kind(SyntaxKind::BLOCK_CONTENT), children)
}

fn inline_children(
    node: &SyntaxNode,
    sentinels: &[LinePrefix],
    literal: bool,
) -> Vec<GreenElement> {
    let literal = literal || matches!(node.kind(), SyntaxKind::CODE | SyntaxKind::VERBATIM);
    node.children_with_tokens()
        .flat_map(|element| match element {
            NodeOrToken::Node(child) => vec![NodeOrToken::Node(GreenNode::new(
                raw_kind(child.kind()),
                inline_children(&child, sentinels, literal),
            ))],
            NodeOrToken::Token(token) => inline_token(&token, sentinels, literal),
        })
        .collect()
}

fn inline_token(token: &SyntaxToken, sentinels: &[LinePrefix], literal: bool) -> Vec<GreenElement> {
    let start = usize::from(token.text_range().start());
    let mut prefix_index = sentinels.partition_point(|prefix| prefix.indentation.end <= start);
    let text = token
        .text()
        .char_indices()
        .filter_map(|(offset, ch)| {
            let position = start + offset;
            while sentinels
                .get(prefix_index)
                .is_some_and(|prefix| prefix.indentation.end <= position)
            {
                prefix_index += 1;
            }
            match sentinels.get(prefix_index) {
                Some(prefix) if prefix.sentinel.contains(&position) => None,
                Some(prefix) if prefix.indentation.contains(&position) => Some('\u{a0}'),
                _ => Some(ch),
            }
        })
        .collect::<String>();
    if token.kind() != SyntaxKind::TEXT || literal {
        return vec![NodeOrToken::Token(GreenToken::new(
            raw_kind(token.kind()),
            &text,
        ))];
    }
    let mut children = Vec::new();
    for line in text.split_inclusive('\n') {
        let (text, break_line) = line
            .strip_suffix('\n')
            .map_or((line, false), |text| (text, true));
        children.push(NodeOrToken::Token(GreenToken::new(
            raw_kind(SyntaxKind::TEXT),
            text,
        )));
        if break_line {
            children.push(NodeOrToken::Node(GreenNode::new(
                raw_kind(SyntaxKind::LINE_BREAK),
                [NodeOrToken::Token(GreenToken::new(
                    raw_kind(SyntaxKind::TEXT),
                    "\n",
                ))],
            )));
        }
    }
    children
}

/// Org indentation uses eight-column tab stops and removes common indentation.
fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .fold(0, |columns, ch| {
            if ch == '\t' {
                columns + 8 - columns % 8
            } else {
                columns + 1
            }
        })
}
