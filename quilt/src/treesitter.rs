use crate::lang::{Arity, FlatNode, Hole, InnerKind, Language, LanguagePost};
use crate::prelude::*;
use crate::qterm::{qsym, QTerm, QTermBuilder};
use crate::term::CmdOrHole;
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

/// Where a hole's placeholder sits in the flattened code: one row, a byte
/// range of it, and whether the hole is a [`FlatNode::Splice`].
#[derive(Debug, Clone, Copy)]
struct HolePoint {
    row: usize,
    start: usize,
    end: usize,
    splice: bool,
}

impl HolePoint {
    fn at(&self) -> (usize, usize, usize) {
        (self.row, self.start, self.end)
    }
}

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
    /// See [`Language::separator`].
    fn separator(&self, _tag: &str) -> Option<&'static str> {
        None
    }
    /// Whether a term spliced into a hole mid-line has its continuation lines
    /// aligned to the hole's *column*, not just to the line's indentation.
    ///
    /// Off by default: in most languages a multi-line splice reads fine at the
    /// line's indent, and aligning to the column would push every spliced
    /// block far to the right. A language whose grammar is column-sensitive
    /// turns it on — in Lean, `def x : S := ↙s↘` with `s` a two-line structure
    /// instance put the second field left of the first, which is a parse error
    /// (Lean's `checkColGe`).
    fn align_splices(&self) -> bool {
        false
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

    /// See [`Language::machine_spec`](crate::lang::Language::machine_spec).
    fn machine_spec(&self) -> Option<crate::machine::MachineSpec> {
        None
    }

    /// See [`Language::native_machine`](crate::lang::Language::native_machine).
    fn native_machine(&self) -> Option<Box<dyn crate::machine::Machine>> {
        None
    }

    /// See [`Language::repl_spec`](crate::lang::Language::repl_spec).
    fn repl_spec(&self) -> Option<crate::machine::ReplSpec> {
        None
    }

    /// The syntactic contexts a *fragment* — a match arm, an enum variant, a
    /// constructor, anything that is a valid piece of a file but not a file —
    /// is retried inside when it does not parse on its own. Tried in order;
    /// see [`FragmentWrapper`] and [`TSLanguage::parse_fragment`].
    fn fragment_wrappers(&self) -> &'static [FragmentWrapper] {
        &[]
    }
}

/// A synthetic context that makes a fragment parse, and how to find the
/// fragment in it again.
///
/// The root of every grammar is a whole file, so a quote of something that
/// only exists *inside* a construct — `rs↖Foo::A(a) => f(a),↗`, a match arm;
/// `lean↖| zero↗`, a constructor — is a parse error on its own. The fix is the
/// one Lean's `#check` and SQL's `SELECT` wrappers already use for bare terms:
/// parse the fragment inside the smallest construct that accepts it, then take
/// the construct away again.
///
/// The fragment sits on lines of its own between [`before`](Self::before) and
/// [`after`](Self::after), so it keeps its columns (and with them its indent
/// prefixes). After the parse, the first [`container`](Self::container) node
/// must hold exactly one child tagged one of [`fragments`](Self::fragments);
/// that child is the result. Comments directly before it, and the
/// [`leading`](Self::leading) kinds (Rust's `#[attr]`, which the grammar hangs
/// on the list rather than on the variant), are folded into it, so a doc
/// comment travels with the thing it documents.
///
/// The result must write back as the quoted text, byte for byte (modulo the
/// whitespace around it), or the wrapper is not used: a trailing `,` the
/// container would own, or a second fragment, falls through to the original
/// parse error.
#[derive(Debug, Clone, Copy)]
pub struct FragmentWrapper {
    /// Text on the line(s) before the fragment, e.g. `match q {`.
    pub before: &'static str,
    /// Text on the line(s) after the fragment, e.g. `}`.
    pub after: &'static str,
    /// The node kind whose child the fragment is, e.g. `match_block`.
    pub container: &'static str,
    /// The node kinds the fragment may be, e.g. `match_arm`.
    pub fragments: &'static [&'static str],
    /// Sibling kinds, beyond comments, that belong to the fragment after them.
    pub leading: &'static [&'static str],
}

impl FragmentWrapper {
    /// Find the fragment in `term`, the parse of the wrapped code: the one
    /// [`fragments`](Self::fragments) child of the first
    /// [`container`](Self::container), with the comments and
    /// [`leading`](Self::leading) siblings right before it folded in.
    fn extract(&self, term: &QTerm) -> Option<Arc<QTerm>> {
        let QTerm::Tuple { tag, terms, cmds } = term else {
            return None;
        };
        if &**tag != self.container {
            return terms.iter().find_map(|t| self.extract(t));
        }
        let tag_of = |t: &QTerm| match t {
            QTerm::Tuple { tag, .. } => Some(tag.clone()),
            _ => None,
        };
        let mut hits = terms
            .iter()
            .enumerate()
            .filter(|(_, t)| tag_of(t).is_some_and(|tag| self.fragments.contains(&&*tag)));
        let (j, _) = hits.next()?;
        if hits.next().is_some() {
            return None;
        }
        let mut i = j;
        while i > 0
            && tag_of(&terms[i - 1]).is_some_and(|tag| {
                crate::lang::is_comment_tag(&tag) || self.leading.contains(&&*tag)
            })
        {
            i -= 1;
        }
        if i == j {
            return Some(terms[j].clone());
        }
        // Fold children `i..j` into child `j`: its tag, their holes and the
        // text between them, then its own commands.
        let QTerm::Tuple {
            tag: jtag,
            terms: jterms,
            cmds: jcmds,
        } = &*terms[j]
        else {
            return None;
        };
        let holes: Vec<usize> = cmds
            .iter()
            .enumerate()
            .filter_map(|(k, c)| matches!(c, CmdOrHole::Hole).then_some(k))
            .collect();
        let between = &cmds[holes[i]..holes[j]];
        // A prefix pushed between them would be popped after the container,
        // not inside the fragment.
        if between
            .iter()
            .any(|c| matches!(c, CmdOrHole::Cmd(StrCmd::Push(_) | StrCmd::Pop)))
        {
            return None;
        }
        let mut new_terms = terms[i..j].to_vec();
        new_terms.extend(jterms.iter().cloned());
        let mut new_cmds = between.to_vec();
        new_cmds.extend(jcmds.iter().cloned());
        Some(tuple(jtag, &new_terms, &new_cmds))
    }
}

/// The text `code` stands for, with each hole spelled `hole_str`.
fn flat_text(code: &[FlatNode], hole_str: &str) -> String {
    code.iter()
        .map(|c| match c {
            FlatNode::Hole | FlatNode::Splice => hole_str,
            FlatNode::Str(s) => s,
            FlatNode::NewLine => "\n",
        })
        .collect()
}

/// How many holes `term` holds: tuples tagged `hole_str`.
fn count_holes(term: &QTerm, hole_str: &str) -> usize {
    match term {
        QTerm::Tuple { tag, .. } if &**tag == hole_str => 1,
        QTerm::Tuple { terms, .. } => terms.iter().map(|t| count_holes(t, hole_str)).sum(),
        _ => 0,
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

impl<P: TSProvider> TSLanguage<P> {
    /// Parse `code` as the grammar's root (a whole file) and nothing else:
    /// [`parse_pre`](Language::parse_pre) without the
    /// [fragment](Self::parse_fragment) retry. A language with retries of its
    /// own (Lean's `#check`) calls this, so that it can order them.
    pub fn parse_plain(
        &mut self,
        ikind: Option<InnerKind>,
        code: &[FlatNode],
    ) -> Result<TSLanguagePost> {
        // convert node to sterm while populating holes.
        fn f<P: TSProvider>(
            provider: &P,
            node: tree_sitter::Node,
            lines: &[&str],
            hole_points: &mut Peekable<IntoIter<HolePoint>>,
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
                && hole_points
                    .peek()
                    .is_some_and(|h| h.at() == (start.row, start.column, end.column))
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
            let all_anonymous =
                (0..node.child_count()).all(|i| node.child(i).is_some_and(|c| !c.is_named()));
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
                    //
                    // A line that does *not* carry the prefix — a string
                    // literal's second line written flush left, say — is
                    // content the prefix is not part of, so it must not gain
                    // one on the way out: its newline is written raw, which
                    // the prefix writer leaves alone. `nl` would re-indent it
                    // to the code around the token and change the program's
                    // string.
                    let indented = line[..c1].starts_with(&pre);
                    let c0 = usize::from(indented) * pre.len();
                    if indented {
                        builder.nl();
                    } else {
                        builder.write("\n");
                    }
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
                hole_points: &mut Peekable<IntoIter<HolePoint>>,
                holes: &mut Vec<Hole>,
                prefix: &[Box<str>],
                row: usize,
                c0: usize,
                c1: usize,
            ) {
                let mut col = c0;
                while let Some(&HolePoint {
                    row: r,
                    start: s,
                    end: e,
                    ..
                }) = hole_points.peek()
                {
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

            // The padding that aligns a splice at `node` to its column (see
            // [`TSProvider::align_splices`]): the text between the line's
            // prefix and the hole, every character but a tab turned into a
            // space — empty when nothing precedes the hole on its line, where
            // the line's indent is already its column. The push still marks
            // the splice as aligned, so host text spliced there is laid out at
            // that column too (#310). `None` when `node` is not a splice (a
            // one-line unquote's hole, [`FlatNode::Splice`]), or when another
            // hole precedes it on its line — that hole's splice has a width
            // only known when the term is built.
            fn splice_pad<P: TSProvider>(
                provider: &P,
                node: tree_sitter::Node,
                lines: &[&str],
                hole_points: &mut Peekable<IntoIter<HolePoint>>,
                prefix: &[Box<str>],
            ) -> Option<String> {
                let (start, end) = (node.start_position(), node.end_position());
                let at = (start.row, start.column, end.column);
                if start.row != end.row
                    || !hole_points.peek().is_some_and(|h| h.splice && h.at() == at)
                {
                    return None;
                }
                let before = &lines[start.row][..start.column];
                let before = before.strip_prefix(&*prefix.concat()).unwrap_or(before);
                if before.contains(provider.hole_str()) {
                    return None;
                }
                Some(
                    before
                        .chars()
                        .map(|c| if c == '\t' { '\t' } else { ' ' })
                        .collect(),
                )
            }

            // otherwise, recurse into children
            #[allow(clippy::too_many_arguments)]
            fn process<'a, P: TSProvider>(
                provider: &P,
                node: tree_sitter::Node,
                builder: &'a mut QTermBuilder, // TODO: use TupleBuilder
                depth: &mut i32,
                lines: &[&str],
                hole_points: &mut Peekable<IntoIter<HolePoint>>,
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
                let child = node.child(i).unwrap();
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
                // A hole that follows text on its line is padded out to its
                // column for the length of the splice, if the language asks.
                let pad = provider
                    .align_splices()
                    .then(|| splice_pad(provider, child, lines, hole_points, prefix))
                    .flatten();
                if let Some(pad) = &pad {
                    builder.push(pad);
                }
                builder.child(&arc(f(
                    provider,
                    child,
                    lines,
                    hole_points,
                    holes,
                    prefix,
                    false,
                )));
                if pad.is_some() {
                    builder.pop();
                }
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
                FlatNode::Hole | FlatNode::Splice => {
                    lines.last_mut().unwrap().push_str(self.provider.hole_str());
                    let new_col = lines.last().unwrap().len();
                    hole_points.push(HolePoint {
                        row,
                        start: col,
                        end: new_col,
                        splice: *c == FlatNode::Splice,
                    });
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

        Ok(TSLanguagePost {
            holes,
            qterm,
            hole_str,
        })
    }

    /// Parse `code` as a *fragment*: inside each of the provider's
    /// [`fragment_wrappers`](TSProvider::fragment_wrappers) in turn, keeping
    /// the first whose fragment writes back as `code` did. `None` when none
    /// does — the caller reports its own error, since the wrappers are an
    /// implementation detail.
    pub fn parse_fragment(&mut self, code: &[FlatNode]) -> Option<TSLanguagePost> {
        let hole_str = self.provider.hole_str();
        let text = flat_text(code, hole_str);
        let want = text.trim();
        if want.is_empty() {
            return None;
        }
        let holes = code.iter().filter(|c| c.is_hole()).count();
        for w in self.provider.fragment_wrappers() {
            let mut wrapped = Vec::with_capacity(code.len() + 4);
            wrapped.push(FlatNode::Str(w.before));
            wrapped.push(FlatNode::NewLine);
            wrapped.extend_from_slice(code);
            wrapped.push(FlatNode::NewLine);
            wrapped.push(FlatNode::Str(w.after));
            let Ok(post) = self.parse_plain(None, &wrapped) else {
                continue;
            };
            let Some(qterm) = w.extract(&post.qterm) else {
                continue;
            };
            // Byte-exact, and every hole inside it: nothing of the wrapper is
            // left, and nothing of the fragment was lost.
            if qterm.coparse() == want && count_holes(&qterm, hole_str) == holes {
                return Some(TSLanguagePost {
                    qterm: (*qterm).clone(),
                    ..post
                });
            }
        }
        None
    }
}

impl<P: TSProvider> Language for TSLanguage<P> {
    type Post = TSLanguagePost;

    /// [`parse_plain`](Self::parse_plain), then — only if that fails — the
    /// [fragment](Self::parse_fragment) retry, reporting the plain parse's
    /// error when that fails too.
    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        match self.parse_plain(ikind, code) {
            Ok(post) => Ok(post),
            Err(err) => self.parse_fragment(code).ok_or(err),
        }
    }

    fn arity(&self, tag: &str) -> Arity {
        self.provider.arity(tag)
    }

    fn separator(&self, tag: &str) -> Option<&'static str> {
        self.provider.separator(tag)
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

    fn machine_spec(&self) -> Option<crate::machine::MachineSpec> {
        self.provider.machine_spec()
    }

    fn repl_spec(&self) -> Option<crate::machine::ReplSpec> {
        self.provider.repl_spec()
    }

    fn native_machine(&self) -> Option<Box<dyn crate::machine::Machine>> {
        self.provider.native_machine()
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
                    repr,
                } => arc(crate::qterm::qquote_repr_at(
                    tag,
                    *index,
                    lang,
                    fill(term, plugs, hole_str),
                    cmds,
                    span.clone(),
                    *repr,
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

    fn separator(&self, tag: &str) -> Option<&'static str> {
        self.0.separator(tag)
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

    fn machine_spec(&self) -> Option<crate::machine::MachineSpec> {
        self.0.machine_spec()
    }

    fn repl_spec(&self) -> Option<crate::machine::ReplSpec> {
        self.0.repl_spec()
    }

    fn native_machine(&self) -> Option<Box<dyn crate::machine::Machine>> {
        self.0.native_machine()
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
