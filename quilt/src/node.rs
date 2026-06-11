use crate::strcmd::PrefixWriter;
use crate::term::Term;
use crate::{prelude::*, term::STerm};
use std::{fmt::Debug, iter::empty, sync::Arc};

/**************************************************************/

pub const ARROW_LEN: usize = 3;
pub const ESCAPE_LEN: usize = 1;

/**************************************************************/

/// Raw Quilt AST with unparsed string content
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum Node {
    Content(Box<str>),
    NewLine,
    Quote {
        anno: Box<str>,
        nodes: Box<[Arc<Node>]>,
        /// Byte range of the whole `anno↖…↗` in the parsed source.
        span: Span,
    },
    Unquote {
        anno: Box<str>,
        nodes: Box<[Arc<Node>]>,
        /// Byte range of the whole `anno↙…↘` in the parsed source.
        span: Span,
    },
    Lift,
    Reduce,
    Emit,
    Type,
    Name,
}

impl Node {
    /// Parse a source string into a list of `Node`s.
    pub fn parse(code: &str) -> Box<[Self]> {
        let mut parser = tree_sitter::Parser::default();
        parser
            .set_language(&tree_sitter_quilt::LANGUAGE.into())
            .expect("Error loading Quilt grammar");
        let tree = parser.parse(code, None).unwrap();
        let root = tree.root_node();

        let mut nodes = Vec::new();
        for i in 0..root.child_count() {
            nodes.push(Self::from_ts(&root.child(i).unwrap(), code));
        }
        nodes.into()
    }

    /// Convert a tree-sitter node + source string to a `Node`.
    pub fn from_ts(node: &tree_sitter::Node, code: &str) -> Self {
        match node.kind() {
            "content" => {
                let range = node.range();
                Node::Content(code[range.start_byte..range.end_byte].into())
            }
            "escape" => {
                let range = node.range();
                Node::Content(code[range.start_byte + ESCAPE_LEN..range.end_byte].into())
            }
            "newline" => Node::NewLine,
            "quote" => {
                let range = node.child(0).unwrap().range();
                let anno = code[range.start_byte..range.end_byte - ARROW_LEN].into();
                let mut nodes = Vec::new();
                for i in 1..node.child_count() - 1 {
                    nodes.push(Self::from_ts(&node.child(i).unwrap(), code).into());
                }
                let nodes = nodes.into();
                let span = node.start_byte()..node.end_byte();
                Node::Quote { anno, nodes, span }
            }
            "unquote" => {
                let range = node.child(0).unwrap().range();
                let anno = code[range.start_byte..range.end_byte - ARROW_LEN].into();
                let mut nodes = Vec::new();
                for i in 1..node.child_count() - 1 {
                    nodes.push(Self::from_ts(&node.child(i).unwrap(), code).into());
                }
                let nodes = nodes.into();
                let span = node.start_byte()..node.end_byte();
                Node::Unquote { anno, nodes, span }
            }
            "lift" => Node::Lift,
            "reduce" => Node::Reduce,
            "emit" => Node::Emit,
            "type" => Node::Type,
            "name" => Node::Name,
            _ => unreachable!("unexpected node kind: {:?}", node.kind()),
        }
    }

    pub fn coparse(nodes: &[Self]) -> Box<str> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        let mut writer = PrefixWriter::new(&mut buf);
        for n in nodes {
            n.write(&mut writer);
        }
        let bytes = buf.into_inner().unwrap();
        String::from_utf8(bytes).unwrap().into()
    }
}

pub fn escape(s: &str) -> Box<str> {
    s.replace('↑', "\\↑").replace('↓', "\\↓").into()
}

pub fn unescape(s: &str) -> Box<str> {
    s.replace("\\↑", "↑").replace("\\↓", "↓").into()
}

/**************************************************************/

pub enum NodeTag {
    Content,
    NewLine,
    Quote,
    Unquote,
    Lift,
    Reduce,
    Emit,
    Name,
    Type,
}

impl Term for Node {
    type Tag = NodeTag;

    fn tag(&self) -> Self::Tag {
        match self {
            Node::Content(_) => NodeTag::Content,
            Node::NewLine => NodeTag::NewLine,
            Node::Quote { .. } => NodeTag::Quote,
            Node::Unquote { .. } => NodeTag::Unquote,
            Node::Lift => NodeTag::Lift,
            Node::Reduce => NodeTag::Reduce,
            Node::Emit => NodeTag::Emit,
            Node::Type => NodeTag::Type,
            Node::Name => NodeTag::Name,
        }
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        let ret: Box<dyn Iterator<Item = _>> = match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => {
                bx(nodes.iter().map(|x| x.as_ref()))
            }
            _ => bx(empty()),
        };
        ret
    }

    fn len(&self) -> usize {
        match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => nodes.len(),
            _ => 0,
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Node::Quote { nodes, .. } | Node::Unquote { nodes, .. } => nodes.is_empty(),
            _ => true,
        }
    }
}

impl STerm for Node {
    fn write<W: std::io::Write>(&self, writer: &mut crate::strcmd::PrefixWriter<'_, W>) {
        match self {
            Node::Content(s) => writer.write(&escape(s)),
            Node::NewLine => writer.newline(),
            Node::Quote { anno, nodes, .. } => {
                writer.write(anno);
                writer.write("↖");
                for n in nodes {
                    n.write(writer);
                }
                writer.write("↗");
            }
            Node::Unquote { anno, nodes, .. } => {
                writer.write(anno);
                writer.write("↙");
                for n in nodes {
                    n.write(writer);
                }
                writer.write("↘");
            }
            Node::Lift => writer.write("↑"),
            Node::Reduce => writer.write("↓"),
            Node::Emit => writer.write("←"),
            Node::Type => writer.write("⟨T⟩"),
            Node::Name => writer.write("⟨N⟩"),
        }
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node() {
        let source_code = indoc::indoc! {"
            Some Python: py↖1+2↗
            ↑↓
        "};
        let nodes = Node::parse(source_code);
        dbg!(&nodes);
        let source_code2 = &*Node::coparse(&nodes);
        assert_eq!(source_code, source_code2);
    }

    #[test]
    fn arrow_len() {
        assert_eq!("↖".len(), ARROW_LEN);
        assert_eq!("↗".len(), ARROW_LEN);
        assert_eq!("↙".len(), ARROW_LEN);
        assert_eq!("↘".len(), ARROW_LEN);
        assert_eq!("↑".len(), ARROW_LEN);
        assert_eq!("↓".len(), ARROW_LEN);
    }
}
