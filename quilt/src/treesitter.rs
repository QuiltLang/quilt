use crate::lang::{Arity, FlatNode, Hole, InnerKind, Language, LanguagePost};
use crate::prelude::*;
use crate::qterm::{qsym, QTerm, QTermBuilder};
use miette::bail;
use std::fmt::Debug;
use std::iter::Peekable;
use std::vec::IntoIter;
use tree_sitter::{Parser, Point, Tree};

/**************************************************************/

/// Parse multiple lines of code into a tree-sitter tree.
/// Lines must end with a newline character, except possibly the last line.
fn ts_parse_lines(parser: &mut Parser, lines: &[&str]) -> Result<Tree> {
    let mut callback = |_byte_offset: usize, point: Point| -> &str {
        if point.row < lines.len() {
            &lines[point.row][point.column..]
        } else {
            ""
        }
    };

    let Some(tree) = parser.parse_with_options(&mut callback, None, None) else {
        bail!("Failed to parse: {lines:?}");
    };
    if tree.root_node().has_error() {
        bail!(
            "Parsed with errors: {lines:?} {}",
            tree.root_node().to_sexp()
        )
    }
    Ok(tree)
}

#[inline]
fn drop_last(s: &str) -> &str {
    &s[..s.len() - 1]
}

/**************************************************************/

/// Language provider used by `TSLanguage`
pub trait TSProvider {
    /// A tree-sitter parser
    fn parser(&mut self) -> &mut tree_sitter::Parser;
    /// A string representing a hole where another language is dropped in.
    /// Must not contain new-lines.
    fn hole_str(&self) -> &'static str;
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> (QTerm, InnerKind) {
        (qterm, Default::default())
    }
    fn arity(&self, _tag: &str) -> Arity {
        Default::default()
    }
    /// The `InnerKind` a node with this tag denotes (used to derive
    /// [`Hole::ikind`] from the hole's position).
    fn typ(&self, _tag: &str) -> InnerKind {
        Default::default()
    }

    fn hashbang(&self) -> Option<&'static str> {
        None
    }
}

#[derive(Default)]
pub struct TSLanguage<P: TSProvider> {
    provider: P,
}

#[derive(Debug)]
pub struct TSLanguagePost {
    pub holes: Box<[Hole]>,
    pub qterm: QTerm,
    pub hole_str: &'static str,
}

impl<P: TSProvider> Language for TSLanguage<P> {
    type Post = TSLanguagePost;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        // convert node to sterm while populating holes.
        fn f<P: TSProvider>(
            provider: &P,
            node: tree_sitter::Node,
            lines: &[&str],
            hole_points: &mut Peekable<IntoIter<(usize, usize, usize)>>,
            holes: &mut Vec<Hole>,
            prefix: &mut Vec<Box<str>>,
            root: bool,
        ) -> QTerm {
            let range = node.range();
            let (start, end) = if root {
                (
                    Point::default(),
                    Point {
                        row: lines.len() - 1,
                        column: lines.last().unwrap().len(),
                    },
                )
            } else {
                (range.start_point, range.end_point)
            };
            let hole_str = provider.hole_str();

            // check if this is a hole
            if start.row == end.row
                && hole_points.peek() == Some(&(start.row, start.column, end.column))
            {
                hole_points.next();
                holes.push(Hole {
                    otag: node.kind().into(),
                    ikind: Some(provider.typ(node.kind())),
                    prefix: prefix.clone().into(),
                });
                return qsym(hole_str);
            }

            // A multiline *leaf* token (e.g. HTML `raw_text`): its lines are
            // real content, not inter-child whitespace, so write them
            // verbatim — continuation lines minus the current prefix. (The
            // gap logic below would misread them as indentation and drop
            // them.)
            if node.child_count() == 0 && start.row != end.row {
                let mut builder = tb(node.kind());
                builder.write(drop_last(&lines[start.row][start.column..]));
                let pre = prefix.concat();
                let rows = lines.iter().enumerate();
                for (row, line) in rows.take(end.row + 1).skip(start.row + 1) {
                    let line = if row == end.row {
                        &line[..end.column]
                    } else {
                        drop_last(line)
                    };
                    builder.nl();
                    builder.write(line.strip_prefix(&pre).unwrap_or(line));
                }
                return builder.build();
            }

            // otherwise, recurse into children
            fn process<'a>(
                builder: &'a mut QTermBuilder, // TODO: use TupleBuilder
                depth: &mut i32,
                lines: &[&str],
                prefix: &mut Vec<Box<str>>,
                p0: Point,
                p1: Point,
            ) -> &'a mut QTermBuilder {
                if p0.row == p1.row {
                    builder.write(&lines[p0.row][p0.column..p1.column])
                } else {
                    // drop the trailing newline
                    builder.write(drop_last(&lines[p0.row][p0.column..]));
                    // TODO: this makes a bunch of assumptions about the language
                    let new_prefix = &lines[p1.row][..p1.column];
                    loop {
                        if let Some(push) = new_prefix.strip_prefix(&prefix.concat()) {
                            if !push.is_empty() {
                                // println!("push: '{push}'");
                                prefix.push(push.into());
                                builder.push(push);
                                *depth += 1;
                            }
                            break;
                        }
                        // println!("pop");
                        prefix.pop();
                        builder.pop();
                        *depth -= 1;
                    }
                    for _ in 0..(p1.row - p0.row) {
                        builder.nl();
                    }
                    builder
                }
            }

            let mut builder = tb(node.kind());
            let mut depth: i32 = 0;
            let mut point = start;
            for i in 0..node.child_count() {
                let child = node.child(i).unwrap();
                let crange = child.range();
                process(
                    &mut builder,
                    &mut depth,
                    lines,
                    prefix,
                    point,
                    crange.start_point,
                );
                point = crange.end_point;
                builder.child(&arc(f(
                    provider,
                    child,
                    lines,
                    hole_points,
                    holes,
                    prefix,
                    false,
                )));
            }
            process(&mut builder, &mut depth, lines, prefix, point, end);
            for _ in 0..depth {
                prefix.pop();
                builder.pop();
            }
            builder.build()
        }

        let mut hole_points = vec![];
        let mut row: usize = 0;
        let mut col: usize = 0;

        let mut lines = vec![String::new()];
        for c in code {
            match c {
                FlatNode::Hole => {
                    lines.last_mut().unwrap().push_str(self.provider.hole_str());
                    let new_col = lines.last().unwrap().len();
                    hole_points.push((row, col, new_col));
                    col = new_col;
                }
                FlatNode::Str(s) => {
                    lines.last_mut().unwrap().push_str(s);
                    col += s.len();
                }
                FlatNode::NewLine => {
                    // Lines must end with a newline character, except possibly the last line.
                    lines.last_mut().unwrap().push('\n');
                    lines.push(String::new());
                    row += 1;
                    col = 0;
                }
            }
        }
        let lines = lines.iter().map(|s| s.as_ref()).collect::<Box<[_]>>();
        let tree = ts_parse_lines(self.provider.parser(), &lines)?;

        let mut hole_points = hole_points.into_iter().peekable();
        let mut holes = vec![];
        let mut prefix = vec![];
        let qterm = f(
            &self.provider,
            tree.root_node(),
            &lines,
            &mut hole_points,
            &mut holes,
            &mut prefix,
            true,
        );
        let (qterm, _ikind) = self.provider.unwrap(qterm, ikind);
        let holes = holes.into();
        let hole_str = self.provider.hole_str();

        Ok(Self::Post {
            holes,
            qterm,
            hole_str,
        })
    }

    fn arity(&self, tag: &str) -> Arity {
        self.provider.arity(tag)
    }

    fn typ(&self, tag: &str) -> InnerKind {
        self.provider.typ(tag)
    }

    fn hashbang(&self) -> Option<&'static str> {
        self.provider.hashbang()
    }
}

impl LanguagePost for TSLanguagePost {
    fn holes(&self) -> &[Hole] {
        &self.holes
    }

    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        // fill the holes with plugs
        fn fill<'a>(
            qterm: &QTerm,
            plugs: &mut impl Iterator<Item = &'a Arc<QTerm>>,
            hole_str: &str,
        ) -> Arc<QTerm> {
            match qterm {
                QTerm::Quote {
                    tag,
                    index,
                    lang,
                    term,
                    cmds,
                } => quote(tag, *index, lang, fill(term, plugs, hole_str), cmds),
                QTerm::Unquote {
                    tag,
                    index,
                    lang,
                    term,
                    cmds,
                } => unquote(tag, *index, lang, fill(term, plugs, hole_str), cmds),
                QTerm::Tuple { tag, terms, cmds } => {
                    if &**tag == hole_str {
                        return plugs.next().unwrap().clone();
                    }
                    tuple(
                        tag,
                        &terms
                            .iter()
                            .map(|t| fill(t, plugs, hole_str))
                            .collect::<Vec<_>>(),
                        cmds,
                    )
                }
            }
        }

        assert_eq!(plugs.len(), self.holes.len());
        Ok(fill(&self.qterm, &mut plugs.iter(), self.hole_str))
    }
}

/**************************************************************/

#[derive(Default)]
pub struct DynTSLanguage<P: TSProvider>(TSLanguage<P>);

impl<P: TSProvider> Language for DynTSLanguage<P> {
    type Post = Box<dyn LanguagePost>;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        Ok(bx(self.0.parse_pre(ikind, code)?) as Self::Post)
    }

    fn arity(&self, tag: &str) -> Arity {
        self.0.arity(tag)
    }

    fn typ(&self, tag: &str) -> InnerKind {
        self.0.typ(tag)
    }

    fn hashbang(&self) -> Option<&'static str> {
        self.0.hashbang()
    }
}

impl<T: LanguagePost> LanguagePost for Box<T> {
    fn holes(&self) -> &[Hole] {
        self.as_ref().holes()
    }

    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        self.as_ref().parse_post(plugs)
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::langs::rust::lang::RustProvider;

    /// Parse a string of code into a tree-sitter tree.
    fn ts_parse(parser: &mut Parser, code: &str) -> Result<Tree> {
        let Some(tree) = parser.parse(code, None) else {
            bail!("Failed to parse: {code}");
        };
        if tree.root_node().has_error() {
            bail!("Parsed with errors: {code}")
        }
        Ok(tree)
    }

    #[test]
    fn test_ts_parse_lines() -> Result<()> {
        let lines = [
            "fn foo() {\n",
            "    println!(\"Hello\");\n",
            "    println!(\"World\");\n",
            "}",
        ];
        let mut provider = RustProvider::default();
        let parser = provider.parser();

        let code = lines.join("");
        let tree_1 = ts_parse(parser, &code)?;
        let tree_2 = ts_parse(parser, &code)?;
        assert_eq!(tree_1.root_node().to_sexp(), tree_2.root_node().to_sexp());
        Ok(())
    }

    #[test]
    fn test_ts_parse_lines_empty() -> Result<()> {
        let lines: [&'static str; 0] = [];
        let mut provider = RustProvider::default();
        let parser = provider.parser();

        let code = lines.join("");
        let tree_1 = ts_parse(parser, &code)?;
        let tree_2 = ts_parse(parser, &code)?;
        assert_eq!(tree_1.root_node().to_sexp(), tree_2.root_node().to_sexp());
        Ok(())
    }
}
