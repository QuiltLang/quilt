//! The SQL target language.
//!
//! SQL is only ever a *quoted* language (e.g. `sql↖ … ↗` inside Rust): Quilt
//! parses SQL fragments into a `QTerm` and the host language's `MetaLanguage`
//! (Rust's) drives expansion. There is therefore no `SqlMetaLanguage` — only
//! this `Language` impl, built on the shared tree-sitter helper.
//!
//! The point of quoting SQL rather than concatenating strings is that a value
//! spliced with `↑` becomes a *literal node*, not text: `LiftTo<Sql> for str`
//! (see [`crate::lift`]) builds a `literal` whose spelling is a standard SQL
//! string with every `'` doubled, so a value can never close the literal and
//! continue the statement. The conformance battery reparses every lifted
//! literal in this grammar, which is what keeps that claim honest (#219).
//!
//! Two shapes of fragment are accepted, mirroring how the grammar is layered:
//!
//! * A **statement** — `sql↖SELECT * FROM t WHERE id = ↙id.↑↘↗` — parses on its
//!   own, since `program` is a repeat of statements.
//! * A bare **expression** — `sql↖id = ↙id.↑↘↗`, the shape a composable
//!   predicate takes — does not: `program` holds statements, so a lone
//!   expression is a parse error. [`SqlLanguage`] retries such a fragment
//!   inside [`SELECT_PREFIX`] and strips the wrapper back off, the same
//!   technique `langs::lean::lang` uses with `#check …`.
//!
//! The same wrapper serves a hole standing where a whole *statement* goes
//! (`↙stmt↘;` on its own line), which a bare identifier cannot occupy either —
//! see [`statement_hole_ordinals`] (#234).

use crate::{
    lang::{Arity, FlatNode, InnerKind, Language, LanguagePost},
    prelude::*,
    qterm::QTerm,
    term::CmdOrHole,
    treesitter::{TSLanguage, TSLanguagePost, TSProvider},
};
use miette::{bail, Result};
use tree_sitter::Parser;

/**************************************************************/

pub struct SqlProvider(tree_sitter::Parser);

impl Default for SqlProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::sql::LANGUAGE.into())
            .expect("Error loading SQL parser");
        Self(parser)
    }
}

impl TSProvider for SqlProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // `__QUILT_HOLE__` matches this grammar's `_identifier` regex
        // (`/[A-Za-z_À-ſ][0-9A-Za-z_À-ſ]*/`), so it parses
        // as an `identifier` wherever one may appear — predicate operand,
        // select expression, relation name, `IN` list element — with **no
        // grammar patch at all**, the same free ride nix and lean get. The
        // range-based hole detection in `treesitter.rs` then recognises it by
        // its byte range. `test/corpus/quilt.txt` in the fork pins those four
        // positions.
        //
        // The one position this does *not* reach unaided is a bare hole as a
        // whole statement (`SELECT 1; ↙stmt↘; SELECT 2;`): no SQL statement
        // starts with a bare identifier, so that is a parse error. That case is
        // handled a level up, by the `SELECT …` wrapper retry in
        // `SqlLanguage::parse_pre` — see `statement_hole_ordinals` (#234).
        "__QUILT_HOLE__"
    }

    fn hashbang(&self) -> Option<&'static str> {
        // SQL is executed by a database, not by an interpreter reading a
        // shebang, and `#!` is not comment syntax in SQL.
        None
    }

    /// Derived from the grammar's `REPEAT` rules by `bin/gen-arity`, not
    /// hand-curated — see `quilt/src/langs/arity.rs` (#202).
    fn arity(&self, tag: &str) -> Arity {
        Arity::from_table(crate::langs::arity::SQL, tag)
    }

    /// SQL is layered `program` → statement → clause → expression, so the tag
    /// alone answers this: the three things `program`'s repeat holds are
    /// statement-like, everything else is a value or a part of one.
    fn typ(&self, tag: &str) -> InnerKind {
        match tag {
            "program" => InnerKind::File,
            // The `choice` inside `program`'s repeat: a plain statement, a
            // `BEGIN … COMMIT` transaction, or a T-SQL `BEGIN … END` block.
            "statement" | "transaction" | "block" => InnerKind::Stmt,
            // Clauses (`select`, `from`, `where`, …) and operands alike. SQL
            // has no kind between "statement" and "value", and clauses are
            // spliced the same way operands are.
            _ => InnerKind::Expr,
        }
    }

    /// Squash the `program` wrapper around a single quoted fragment so the term
    /// is the fragment itself. A fragment with several statements (or one with
    /// its terminating `;`) stays a `program`, since dropping the wrapper would
    /// lose the separators.
    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "program" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            // Empty, several statements, or a statement plus its trailing `;`.
            // `classify_term` recognises the last of those as a `Stmt`.
            let kind = self.classify_term(&qterm);
            return Ok((qterm, kind));
        }
        let kind = self.classify_term(&terms[0]);
        Ok((qterm.squash(), ikind.unwrap_or(kind)))
    }

    /// Classify a fully-parsed SQL term as expression / statement / file.
    ///
    /// Needed (rather than plain [`typ`](TSProvider::typ)) because a
    /// single-statement fragment arrives wrapped in `program` — and, when it
    /// keeps its terminating `;`, wrapped in a *two*-child `program` whose root
    /// tag alone would read `File` for what is really one statement.
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, terms, .. } if &**tag == "program" => match terms.len() {
                1 => self.classify_term(&terms[0]),
                // A single statement plus its trailing `;`: still a `Stmt`.
                2 if is_semi(&terms[1]) => self.classify_term(&terms[0]),
                // Empty (0) or several statements (3+): a whole script.
                _ => InnerKind::File,
            },
            QTerm::Tuple { tag, .. } => self.typ(tag),
            _ => InnerKind::default(),
        }
    }
}

/// Is this term the anonymous `;` statement terminator?
fn is_semi(term: &QTerm) -> bool {
    matches!(term, QTerm::Tuple { tag, terms, .. } if &**tag == ";" && terms.is_empty())
}

/**************************************************************/

/// The synthetic statement a bare *expression* fragment is parsed inside.
///
/// SQL's `program` holds **statements**, so `sql↖id = 1↗` is a parse error on
/// its own — unlike Rust or Python, whose `source_file` accepts a bare
/// expression. Since expression-level composition is much of the point of
/// quoting (`sql↖SELECT * FROM t WHERE ↙pred↘↗`), [`SqlLanguage`] retries a
/// failed parse inside `SELECT …`, the smallest SQL statement that takes an
/// arbitrary expression, and then strips the wrapper back off. See
/// [`strip_select`].
const SELECT_PREFIX: &str = "SELECT ";

/// Undo the [`SELECT_PREFIX`] wrapper: given the parsed statement, return just
/// the expression it wrapped.
///
/// The input is what [`SqlProvider::unwrap`] hands back, so the `program` root
/// has already been squashed away and this starts at the `statement`. The rest
/// of the chain is fixed by the grammar — `statement` → `select` →
/// `select_expression` → `term` → the expression — and every link is checked
/// here rather than assumed, because the retry can succeed on input that is not
/// a lone expression at all (`SELECT a, b` gives a two-`term`
/// `select_expression`; `SELECT 1; DROP TABLE t` leaves the multi-statement
/// `program` unsquashed). Anything but the exact single-expression shape is
/// refused, and the caller then reports the *original* parse error.
fn strip_select(qterm: &QTerm) -> Result<Arc<QTerm>> {
    /// The single child of a tuple with the expected tag, or an error.
    fn only_child<'a>(term: &'a QTerm, want: &str) -> Result<&'a Arc<QTerm>> {
        let QTerm::Tuple { tag, terms, .. } = term else {
            bail!("sql: expected a {want:?} tuple after wrapping, got a quote/unquote");
        };
        if &**tag != want || terms.len() != 1 {
            bail!("sql: expected a 1-child {want:?} tuple after wrapping, got {tag:?}");
        }
        Ok(&terms[0])
    }

    let select = only_child(qterm, "statement")?;
    // `select` is `seq(keyword_select, optional(keyword_distinct),
    // select_expression)`, so an un-aliased one has exactly two children.
    let QTerm::Tuple { tag, terms, cmds } = &**select else {
        bail!("sql: expected a `select` tuple after wrapping, got a quote/unquote");
    };
    if &**tag != "select" || terms.len() != 2 {
        bail!("sql: expected a 2-child `select` tuple after wrapping, got {tag:?}");
    }
    // Nothing may follow the select expression, or dropping the wrapper would
    // lose text.
    let last_hole = cmds
        .iter()
        .rposition(|c| matches!(c, CmdOrHole::Hole))
        .expect("select tuple has children");
    if cmds[last_hole + 1..]
        .iter()
        .any(|c| !matches!(c, CmdOrHole::Cmd(StrCmd::Write(s)) if s.is_empty()))
    {
        bail!("sql: unexpected trailing text after the `SELECT` wrapper's expression");
    }
    // Peel the two wrapper layers, `select_expression` then `term`.
    //
    // Either can be *absent*, and that is not a failure. `select_expression`,
    // `term` and the expression beneath them span exactly the same bytes when
    // the fragment is a lone hole (`sql↖↙v↘↗`), and `build_nodes` replaces the
    // outermost node whose range matches the hole — so the layers are simply
    // not in the tree. A layer that *is* present must hold exactly one child:
    // `SELECT a, b` gives `select_expression` two `term`s and `SELECT x AS y`
    // gives `term` an alias sibling, and neither is a lone expression.
    let mut inner = terms[1].clone();
    for layer in ["select_expression", "term"] {
        let QTerm::Tuple { tag, terms, .. } = &*inner else {
            bail!("sql: expected a tuple inside the `SELECT` wrapper, got a quote/unquote");
        };
        if &**tag != layer {
            break;
        }
        if terms.len() != 1 {
            bail!("sql: the `SELECT` wrapper holds a {layer:?} with {} children, not a lone expression", terms.len());
        }
        let next = terms[0].clone();
        inner = next;
    }
    Ok(inner)
}

/// Ordinals (into the fragment's hole sequence) of holes that stand where a
/// whole **statement** goes:
///
/// ```text
/// SELECT 1;
/// ↙stmt↘;
/// SELECT 2;
/// ```
///
/// `program` is `repeat(seq(choice(statement, transaction, block), ';'))` and no
/// SQL statement begins with a bare identifier, so such a hole is a parse error
/// on its own. Wrapping just these holes in [`SELECT_PREFIX`] makes them
/// statements; [`strip_wrapped_selects`] removes the wrapper again.
///
/// A hole qualifies when nothing but whitespace precedes it on its line and
/// nothing but whitespace and **at most one `;`** follows it. The `;` is the
/// difference from the Lean version this is modelled on
/// (`langs::lean::lang::line_hole_ordinals`): SQL statements are terminated, so
/// the natural spelling puts the separator right after the hole. Allowing the
/// bare form too covers the trailing statement `program` lets go unterminated.
///
/// Deliberately line-based rather than `;`-scanning: a `;` inside a string
/// literal is ordinary text in the flat node stream, and a scanner that split on
/// it would wrap holes that are not in statement position at all. Anything this
/// declines simply falls back to the original parse error.
fn statement_hole_ordinals(code: &[FlatNode]) -> Vec<usize> {
    /// The text on one side of a hole, up to the nearest line break — or `None`
    /// if another hole intervenes, which means this one is not alone.
    fn text_run<'a>(nodes: impl Iterator<Item = &'a FlatNode<'a>>) -> Option<String> {
        let mut out = String::new();
        for node in nodes {
            match node {
                FlatNode::NewLine => break,
                FlatNode::Str(s) => out.push_str(s),
                FlatNode::Hole => return None,
            }
        }
        Some(out)
    }

    let mut out = Vec::new();
    for (i, node) in code.iter().enumerate() {
        if !matches!(node, FlatNode::Hole) {
            continue;
        }
        let ordinal = code[..i]
            .iter()
            .filter(|n| matches!(n, FlatNode::Hole))
            .count();
        let before = text_run(code[..i].iter().rev());
        let after = text_run(code[i + 1..].iter());
        let alone = matches!(before, Some(ref b) if b.trim().is_empty())
            && matches!(after, Some(ref a) if matches!(a.trim(), "" | ";"));
        if alone {
            out.push(ordinal);
        }
    }
    out
}

/// Rebuild `code` with [`SELECT_PREFIX`] inserted before each hole whose ordinal
/// is in `targets`.
fn wrap_statement_holes<'a>(code: &[FlatNode<'a>], targets: &[usize]) -> Vec<FlatNode<'a>> {
    let mut out = Vec::with_capacity(code.len() + targets.len());
    let mut ordinal = 0usize;
    for node in code {
        if matches!(node, FlatNode::Hole) {
            if targets.contains(&ordinal) {
                out.push(FlatNode::Str(SELECT_PREFIX));
            }
            ordinal += 1;
        }
        out.push(node.clone());
    }
    out
}

/// Undo [`wrap_statement_holes`] in the parsed tree: replace each `statement`
/// that wraps one of the holes we wrapped with that hole itself.
///
/// Only the wrappers *we* introduced are removed — holes are counted in tree
/// order and matched against `targets` — so a genuine `SELECT ↙col↘` written by
/// the author survives untouched.
///
/// The shape searched for is `statement(select(keyword_select, <hole>))`: the
/// hole spans the same bytes as the `select_expression`, `term` and `field` that
/// would otherwise sit between, and `build_nodes` replaces the outermost node
/// whose range matches, so those layers are not in the tree. That is the same
/// collapse [`strip_select`] peels through.
///
/// Returns the rewritten term and **how many wrappers were removed**. The caller
/// requires that to equal `targets.len()` and discards the retry otherwise: a
/// target that did not come back in the expected shape means the prefix landed
/// somewhere other than statement position — inside a string literal, say — and
/// keeping such a tree would silently splice a stray `SELECT` into the output.
/// Failing there costs only the original parse error, which is what the user
/// would have seen anyway.
fn strip_wrapped_selects(
    term: &Arc<QTerm>,
    hole_str: &str,
    targets: &[usize],
) -> (Arc<QTerm>, usize) {
    /// Is this the `statement(select(keyword_select, <hole>))` we introduced?
    fn wrapped_hole<'a>(terms: &'a [Arc<QTerm>], hole_str: &str) -> Option<&'a Arc<QTerm>> {
        let [only] = terms else { return None };
        let QTerm::Tuple {
            tag: sel,
            terms: sel_terms,
            ..
        } = &**only
        else {
            return None;
        };
        if &**sel != "select" {
            return None;
        }
        let [_keyword, hole] = &sel_terms[..] else {
            return None;
        };
        matches!(&**hole, QTerm::Tuple { tag, .. } if &**tag == hole_str).then_some(hole)
    }

    fn walk(
        term: &Arc<QTerm>,
        hole_str: &str,
        targets: &[usize],
        ordinal: &mut usize,
        stripped: &mut usize,
    ) -> Arc<QTerm> {
        let QTerm::Tuple { tag, terms, cmds } = &**term else {
            // Quotes/unquotes cannot appear inside a freshly parsed fragment.
            return term.clone();
        };

        if &**tag == "statement" {
            if let Some(hole) = wrapped_hole(terms, hole_str) {
                if targets.contains(ordinal) {
                    *ordinal += 1;
                    *stripped += 1;
                    return hole.clone();
                }
            }
        }

        if &**tag == hole_str {
            *ordinal += 1;
            return term.clone();
        }

        let children: Vec<Arc<QTerm>> = terms
            .iter()
            .map(|t| walk(t, hole_str, targets, ordinal, stripped))
            .collect();
        tuple(tag, &children, cmds)
    }

    let mut ordinal = 0usize;
    let mut stripped = 0usize;
    let out = walk(term, hole_str, targets, &mut ordinal, &mut stripped);
    (out, stripped)
}

/// The SQL `Language`: [`TSLanguage<SqlProvider>`] plus the bare-expression
/// retry described on [`SELECT_PREFIX`], and the statement-position hole
/// wrapper described on [`statement_hole_ordinals`].
#[derive(Default)]
pub struct SqlLanguage(TSLanguage<SqlProvider>);

impl Language for SqlLanguage {
    type Post = TSLanguagePost;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        // A statement fragment (and a whole script) parses on its own; only a
        // bare expression needs the wrapper, so try plain first and keep this a
        // no-op for everything else.
        let err = match self.0.parse_pre(ikind, code) {
            Ok(post) => return Ok(post),
            Err(e) => e,
        };

        let mut wrapped = Vec::with_capacity(code.len() + 1);
        wrapped.push(FlatNode::Str(SELECT_PREFIX));
        wrapped.extend_from_slice(code);

        // Report the *original* error whenever the retry doesn't yield a clean
        // single-expression `SELECT` — the wrapper is an implementation detail,
        // and naming it would only mislead.
        if let Some(post) = self
            .0
            .parse_pre(Some(InnerKind::Expr), &wrapped)
            .ok()
            .and_then(|post| {
                let qterm = strip_select(&post.qterm).ok()?;
                Some(TSLanguagePost {
                    qterm: (*qterm).clone(),
                    ..post
                })
            })
        {
            return Ok(post);
        }

        // Last resort: holes at *statement* position, which a bare identifier
        // cannot occupy. Wrap only those holes and strip the wrappers back out
        // of the tree, leaving every other hole alone.
        let targets = statement_hole_ordinals(code);
        if !targets.is_empty() {
            let wrapped = wrap_statement_holes(code, &targets);
            if let Some(post) = self.0.parse_pre(ikind, &wrapped).ok().and_then(|post| {
                let hole_str = post.hole_str;
                let (qterm, stripped) =
                    strip_wrapped_selects(&arc(post.qterm.clone()), hole_str, &targets);
                // Every wrapper we added must have come back out; see
                // `strip_wrapped_selects`.
                if stripped != targets.len() {
                    return None;
                }
                // The wrapped holes were measured against the *wrapper*'s
                // position, so they came back `Expr` (a select expression).
                // What can actually fill them is a statement — the same answer
                // `langs::lean::lang::hole_kind` gives a hole in a `by`/`do`
                // body, and what `multi.rs` threads into the host parse of the
                // unquote body.
                let mut holes = post.holes.into_vec();
                for &t in &targets {
                    if let Some(hole) = holes.get_mut(t) {
                        hole.ikind = Some(InnerKind::Stmt);
                    }
                }
                Some(TSLanguagePost {
                    qterm: (*qterm).clone(),
                    holes: holes.into_boxed_slice(),
                    hole_str,
                })
            }) {
                return Ok(post);
            }
        }

        Err(err)
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

    fn hashbang(&self) -> Option<&'static str> {
        self.0.hashbang()
    }
}

/// Boxed-`Post` form of [`SqlLanguage`], for the dynamic registry.
#[derive(Default)]
pub struct DynSqlLanguage(SqlLanguage);

impl Language for DynSqlLanguage {
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

    fn hashbang(&self) -> Option<&'static str> {
        self.0.hashbang()
    }
}
