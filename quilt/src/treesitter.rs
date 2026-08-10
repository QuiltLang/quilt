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
    fn unwrap(&self, qterm: QTerm, _ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        Ok((qterm, Default::default()))
    }
    fn arity(&self, _tag: &str) -> Arity {
        Default::default()
    }
    /// The `InnerKind` a node with this tag denotes (used to derive
    /// [`Hole::ikind`] from the hole's position).
    fn typ(&self, _tag: &str) -> InnerKind {
        Default::default()
    }

    /// The [`InnerKind`] a hole at this tree-sitter node's position demands.
    ///
    /// Unlike [`typ`](TSProvider::typ), which only sees a node's *kind* (its
    /// tag), this is handed the node in its parse tree, so it can read context
    /// the tag alone can't express. The motivating case is [`InnerKind::Block`]:
    /// a Rust `block` is an expression by tag (`let x = { … }`), but in body
    /// position (`fn f() { … }`, `loop { … }`, `if c { … }`) it denotes a block
    /// body. The default ignores the extra context and falls back to
    /// `typ(node.kind())`.
    fn hole_kind(&self, node: tree_sitter::Node) -> InnerKind {
        self.typ(node.kind())
    }

    /// Classify a fully-parsed term to determine its kind.  Unlike [`typ`],
    /// which only sees the root tag, this can inspect the full term tree.
    /// The default falls back to `typ` on the root tag; override for languages
    /// where the wrapper node's tag is ambiguous (e.g. WGSL's `source_file`
    /// which can hold a statement + trailing `;` with `len == 2`).
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, .. } => self.typ(tag),
            _ => InnerKind::default(),
        }
    }

    /// See [`Language::ident_tag`](crate::lang::Language::ident_tag).
    fn ident_tag(&self) -> &'static str {
        "identifier"
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
                    // `hole_kind` (not `typ`) so the surrounding tree can refine
                    // the kind — e.g. a body-position `block` becomes `Block`
                    // rather than the `Expr` its tag alone implies.
                    ikind: Some(provider.hole_kind(node)),
                    prefix: prefix.clone().into(),
                });
                return qsym(hole_str);
            }

            // A multiline *token*: its lines are real content, not inter-child
            // whitespace, so write them verbatim — continuation lines minus the
            // current prefix. (The gap logic below would misread them as
            // indentation and drop them.)
            //
            // "Token" is `no named children`, not `no children at all`. A leaf
            // like HTML's `raw_text` has none; but Rust's `block_comment` has
            // two *anonymous* ones, `/*` and `*/`, and so used to fall through
            // to the gap logic — which read the text before `*/` on the last
            // line as an indentation prefix and repeated it over every line of
            // the comment. A node whose children are all anonymous has no
            // structure to recurse into, so nothing is lost by treating it as
            // one token. Found by `bin/fuzz`, issue #161.
            //
            // A hole *can* hide inside one, though — this comment used to say
            // otherwise, on the grounds that a hole parses as a named node. It
            // does only when it stands alone; inside a multi-line string or
            // comment the token swallows it (issue #221), which is why the rows
            // below go through [`write_run`] rather than `builder.write`.
            let all_anonymous = (0..node.child_count()).all(|i| {
                u32::try_from(i)
                    .ok()
                    .and_then(|i| node.child(i))
                    .is_some_and(|c| !c.is_named())
            });
            if all_anonymous && start.row != end.row {
                let mut builder = tb(node.kind());
                let first = &lines[start.row][start.column..];
                write_run(
                    provider,
                    node,
                    &mut builder,
                    lines,
                    hole_points,
                    holes,
                    prefix,
                    start.row,
                    start.column,
                    start.column + drop_last(first).len(),
                );
                let pre = prefix.concat();
                let rows = lines.iter().enumerate();
                for (row, line) in rows.take(end.row + 1).skip(start.row + 1) {
                    let c1 = if row == end.row {
                        end.column
                    } else {
                        drop_last(line).len()
                    };
                    // The continuation lines are written minus the current
                    // prefix, so the run starts after it — and a hole's column
                    // is an index into the *whole* line, which is why this is a
                    // column rather than a `strip_prefix` on the slice.
                    let c0 = usize::from(line[..c1].starts_with(&pre)) * pre.len();
                    builder.nl();
                    write_run(
                        provider,
                        node,
                        &mut builder,
                        lines,
                        hole_points,
                        holes,
                        prefix,
                        row,
                        c0,
                        c1,
                    );
                }
                return builder.build();
            }

            // Write `lines[row][c0..c1]`, splitting the run at any hole point
            // that falls inside it.
            //
            // `__QUILT_HOLE__` is spelled so it lexes as an ordinary
            // identifier/word, and hole detection matches a node whose range
            // *equals* the hole's. That holds only where the hole stands alone:
            // inside a string, inside a comment, or glued to neighbouring text
            // (`__QUILT_HOLE__.service` is one word), the token around it
            // swallows it, no node ever matches, and the point went unconsumed —
            // surfacing much later, and somewhere else, as "Ran out of holes"
            // (issue #221).
            //
            // Splitting the run is what a dedicated `quilt_hole` token in each
            // grammar would otherwise have to do — and cannot, since
            // `tree-sitter generate` panics on that rule (see the forks). It is
            // also strictly more general: it asks nothing of the grammar, so
            // every language gets it at once, and the resulting term is the one
            // you want either way — the hole becomes a child of the token it
            // sits in, with the text on each side written around it.
            //
            // This cannot steal a point from a node that would have matched it
            // exactly: a run is either a leaf's own text or a gap between
            // children, traversal is in document order, and an exact match is
            // consumed at the top of [`f`] before any run is written.
            #[allow(clippy::too_many_arguments)]
            fn write_run<P: TSProvider>(
                provider: &P,
                node: tree_sitter::Node,
                builder: &mut QTermBuilder,
                lines: &[&str],
                hole_points: &mut Peekable<IntoIter<(usize, usize, usize)>>,
                holes: &mut Vec<Hole>,
                prefix: &[Box<str>],
                row: usize,
                c0: usize,
                c1: usize,
            ) {
                let mut col = c0;
                while let Some(&(r, s, e)) = hole_points.peek() {
                    if r != row || s < col || e > c1 {
                        break;
                    }
                    hole_points.next();
                    if s > col {
                        builder.write(&lines[row][col..s]);
                    }
                    holes.push(Hole {
                        // The token the hole sits inside is the closest thing to
                        // a node of its own that it has.
                        otag: node.kind().into(),
                        ikind: Some(provider.hole_kind(node)),
                        prefix: prefix.to_vec().into(),
                    });
                    builder.child(&arc(qsym(provider.hole_str())));
                    col = e;
                }
                builder.write(&lines[row][col..c1]);
            }

            // otherwise, recurse into children
            #[allow(clippy::too_many_arguments)]
            fn process<'a, P: TSProvider>(
                provider: &P,
                node: tree_sitter::Node,
                builder: &'a mut QTermBuilder, // TODO: use TupleBuilder
                depth: &mut i32,
                lines: &[&str],
                hole_points: &mut Peekable<IntoIter<(usize, usize, usize)>>,
                holes: &mut Vec<Hole>,
                prefix: &mut Vec<Box<str>>,
                p0: Point,
                p1: Point,
            ) -> &'a mut QTermBuilder {
                if p0.row == p1.row {
                    write_run(
                        provider,
                        node,
                        builder,
                        lines,
                        hole_points,
                        holes,
                        prefix,
                        p0.row,
                        p0.column,
                        p1.column,
                    );
                    builder
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
                let child = node.child(u32::try_from(i).unwrap()).unwrap();
                let crange = child.range();
                process(
                    provider,
                    node,
                    &mut builder,
                    &mut depth,
                    lines,
                    hole_points,
                    holes,
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
            process(
                provider,
                node,
                &mut builder,
                &mut depth,
                lines,
                hole_points,
                holes,
                prefix,
                point,
                end,
            );
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
                    // A `Str` can carry raw newlines: a multi-line `/* … */`
                    // plain comment reaches here as one node, newlines and all.
                    // `lines` must stay one entry per *physical* line, because
                    // every `Point` tree-sitter hands back below indexes it by
                    // row — an embedded `\n` that did not push a new entry
                    // leaves the two counting differently, and the tree then
                    // addresses rows this vector does not have. Found by
                    // `bin/fuzz` (issue #161): `lines[p1.row]` panicked with an
                    // index out of bounds, and a plain `/* a\nb */` failed to
                    // parse at all.
                    let mut parts = s.split('\n');
                    let first = parts.next().unwrap_or_default();
                    lines.last_mut().unwrap().push_str(first);
                    col += first.len();
                    for part in parts {
                        lines.last_mut().unwrap().push('\n');
                        lines.push(part.to_string());
                        row += 1;
                        col = part.len();
                    }
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
        let (qterm, _ikind) = self.provider.unwrap(qterm, ikind)?;
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

    fn classify_term(&self, term: &QTerm) -> InnerKind {
        self.provider.classify_term(term)
    }

    fn ident_tag(&self) -> &'static str {
        self.provider.ident_tag()
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
                    span,
                } => arc(crate::qterm::qquote_at(
                    tag,
                    *index,
                    lang,
                    fill(term, plugs, hole_str),
                    cmds,
                    span.clone(),
                )),
                QTerm::Unquote {
                    tag,
                    index,
                    lang,
                    term,
                    cmds,
                    span,
                } => arc(crate::qterm::qunquote_at(
                    tag,
                    *index,
                    lang,
                    fill(term, plugs, hole_str),
                    cmds,
                    span.clone(),
                )),
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

    fn classify_term(&self, term: &QTerm) -> InnerKind {
        self.0.classify_term(term)
    }

    fn ident_tag(&self) -> &'static str {
        self.0.ident_tag()
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
