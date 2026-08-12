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
        // The position this does *not* reach is a bare hole as a whole
        // statement (`SELECT 1; ↙stmt↘; SELECT 2;`): no SQL statement starts
        // with a bare identifier, so that is a parse error. Splice the
        // enclosing statement instead. See issue #219.
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

/// The SQL `Language`: [`TSLanguage<SqlProvider>`] plus the bare-expression
/// retry described on [`SELECT_PREFIX`].
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
