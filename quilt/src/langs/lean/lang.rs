//! The Lean 4 language.
//!
//! Lean is both a quotable *target* (`lean↖ … ↗` inside Rust) and a *host*
//! that can drive generation itself (see [`super::meta`]). This module is the
//! `Language` half — parsing Lean fragments into a `QTerm` — built on the
//! shared tree-sitter helper.
//!
//! Lean's grammar is layered `module` → command → term, with tactics and
//! do-elements being ordinary terms in a `by` / `do` body. That shape drives
//! the three classification hooks below: a hole's kind comes from its
//! *parent* ([`hole_kind`]), because the hole token itself is spelled the same
//! everywhere it appears.
//!
//! [`hole_kind`]: TSProvider::hole_kind

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

pub struct LeanProvider(tree_sitter::Parser);

impl Default for LeanProvider {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&crate::grammars::lean::LANGUAGE.into())
            .expect("Error loading Lean parser");
        Self(parser)
    }
}

/// Top-level command tags — the things that can sit directly in a `module`.
/// Mirrors the `_command` choice in the forked grammar, plus the declaration
/// forms `declaration` wraps (a quote of a bare `def …` squashes past the
/// `declaration` wrapper, so both levels have to be recognised).
fn is_command_tag(tag: &str) -> bool {
    // The grammar names most of the long tail `*_cmd`; the rest are spelled out.
    tag.ends_with("_cmd")
        || matches!(
            tag,
            "declaration"
                | "import"
                | "namespace"
                | "section"
                | "public_section"
                | "end"
                | "mutual"
                | "variable"
                | "universe"
                | "open"
                | "export"
                | "initialize"
                | "set_option"
                | "check"
                | "eval"
                | "print"
                | "reduce"
                // declaration bodies, reachable once `declaration` is squashed
                | "def"
                | "theorem"
                | "example"
                | "abbrev"
                | "instance"
                | "axiom"
                | "opaque"
                | "constant"
                | "structure"
                | "inductive"
        )
}

impl TSProvider for LeanProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser {
        &mut self.0
    }

    fn hole_str(&self) -> &'static str {
        // `__QUILT_HOLE__` matches Lean's identifier regex
        // (`[a-zA-Z_][a-zA-Z_0-9'!?]*`), so it parses as an `identifier` —
        // and therefore as a `_term_atom` — in every term, tactic, do-element,
        // declaration-name and binder position, with **no grammar patch at
        // all**. The range-based hole detection in `treesitter.rs` then
        // recognises it by its byte range, exactly as for tree-sitter-nix.
        //
        // The one position this does *not* reach is a bare hole as a whole
        // top-level command (`namespace D ↙cmd↘ end D`), which is a parse
        // error: no Lean command starts with a bare identifier. See issue #133
        // — splice the enclosing construct, or emit into a `by`/`do` body.
        "__QUILT_HOLE__"
    }

    fn hashbang(&self) -> Option<&'static str> {
        // Lean has no interpreter shebang: a file is run via `lean file.lean`
        // (or `lake env lean`), and `#!` is not comment syntax in Lean 4.
        None
    }

    fn arity(&self, tag: &str) -> Arity {
        match tag {
            // The sibling-sequence containers: a module holds commands, and a
            // `by` / `do` body holds tactics / do-elements. These are where
            // emit (`←`) splices a generated sequence.
            "module" | "by" | "do_block" => Arity::Variadic,
            _ => Arity::Unknown,
        }
    }

    fn typ(&self, tag: &str) -> InnerKind {
        match tag {
            "module" => InnerKind::File,
            // A `by`/`do` body is a block *body*, not a value-producing block.
            "by" | "do_block" => InnerKind::Block,
            t if is_command_tag(t) => InnerKind::Item,
            // Everything else in Lean is a term.
            _ => InnerKind::Expr,
        }
    }

    /// The kind a hole's *position* demands.
    ///
    /// Lean spells the hole identically wherever it appears, so the tag alone
    /// says nothing — the parent does. A hole directly under `module` is a
    /// whole command; one directly under a `by` / `do` body is a tactic /
    /// do-element (statement-like, so emit can splice a sequence there);
    /// anywhere else it is a term.
    fn hole_kind(&self, node: tree_sitter::Node) -> InnerKind {
        // The hole is an ordinary `identifier` node, so its tag says nothing —
        // the parent does. `_term_atom` is a hidden rule, so the identifier is
        // inlined directly under the construct containing it: `by` / `do_block`
        // for a tactic or do-element, `module` for a command (unreachable today,
        // see `hole_str`), and the term's parent otherwise.
        match node.parent().map(|p| p.kind()) {
            Some("module") => InnerKind::Item,
            Some("by" | "do_block") => InnerKind::Stmt,
            _ => InnerKind::Expr,
        }
    }

    /// Squash the `module` wrapper around a single quoted fragment so the term
    /// is the fragment itself. A fragment with several top-level commands (a
    /// whole file) stays a `module`.
    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> Result<(QTerm, InnerKind)> {
        let QTerm::Tuple { tag, terms, .. } = &qterm else {
            return Ok((qterm, InnerKind::default()));
        };
        if &**tag != "module" {
            return Ok((qterm, InnerKind::default()));
        }
        if terms.len() != 1 {
            // Empty, or several commands: a whole (or partial) module.
            return Ok((qterm, InnerKind::File));
        }
        let kind = self.classify_term(&terms[0]);
        let squashed = qterm.squash();
        // A `declaration` is a pure wrapper around the real declaration form;
        // squash through it too so `lean↖def f := 1↗` yields the `def`.
        if let QTerm::Tuple { tag, terms, .. } = &squashed {
            if &**tag == "declaration" && terms.len() == 1 {
                return Ok((squashed.squash(), InnerKind::Item));
            }
        }
        // Trust an explicitly requested kind over the guess when the caller
        // placed the hole in a known position.
        Ok((squashed, ikind.unwrap_or(kind)))
    }

    /// Classify a fully-parsed Lean term.
    ///
    /// Needed (rather than plain [`typ`](TSProvider::typ)) because a
    /// single-command fragment arrives wrapped in `module`, so the root tag
    /// alone would read `File` for what is really an `Item`.
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, terms, .. } if &**tag == "module" => match terms.len() {
                1 => self.classify_term(&terms[0]),
                _ => InnerKind::File,
            },
            QTerm::Tuple { tag, terms, .. } if &**tag == "declaration" && terms.len() == 1 => {
                self.classify_term(&terms[0])
            }
            QTerm::Tuple { tag, .. } => self.typ(tag),
            _ => InnerKind::default(),
        }
    }
}

/**************************************************************/

/// The synthetic command a bare *term* fragment is parsed inside.
///
/// Lean's `module` holds **commands**, not terms, so `lean↖n + 1↗` is a parse
/// error on its own — unlike Rust or Python, whose `source_file` accepts a bare
/// expression. Since term-level composition is the whole point of quoting
/// (`lean↖↙acc↘ * x↗`), [`LeanLanguage`] retries a failed parse inside
/// `#check …`, the smallest Lean command that takes an arbitrary term, and then
/// strips the wrapper back off. See [`strip_check`].
const CHECK_PREFIX: &str = "#check ";

/// Undo the [`CHECK_PREFIX`] wrapper: given the parsed `check` tuple, return
/// just its term child, dropping the `#check` token and the space after it.
///
/// The `check` node is `seq('#check', field('term', _term))`, so it has exactly
/// two children — the anonymous `#check` token and the term — and no trailing
/// commands. Keeping the term alone therefore reproduces the original fragment
/// text exactly, which is what the round-trip property requires.
fn strip_check(qterm: &QTerm) -> Result<Arc<QTerm>> {
    let QTerm::Tuple { tag, terms, cmds } = qterm else {
        bail!("lean: expected a `check` tuple after wrapping, got a quote/unquote");
    };
    if &**tag != "check" || terms.len() != 2 {
        bail!("lean: expected a 2-child `check` tuple after wrapping, got {tag:?}");
    }
    // Nothing may follow the term, or dropping the wrapper would lose text.
    let last_hole = cmds
        .iter()
        .rposition(|c| matches!(c, CmdOrHole::Hole))
        .expect("check tuple has children");
    if cmds[last_hole + 1..]
        .iter()
        .any(|c| !matches!(c, CmdOrHole::Cmd(StrCmd::Write(s)) if s.is_empty()))
    {
        bail!("lean: unexpected trailing text after the `#check` wrapper's term");
    }
    Ok(terms[1].clone())
}

/// Ordinals (into the fragment's hole sequence) of holes that sit **alone on
/// their own line** — the shape a hole spliced at *command* position takes:
///
/// ```text
/// namespace Demo
/// ↙decl↘
/// end Demo
/// ```
///
/// A bare hole is not a valid Lean command (no command starts with an
/// identifier), so such a fragment fails to parse. Prefixing just these holes
/// with [`CHECK_PREFIX`] makes them commands; [`strip_wrapped_checks`] then
/// removes the wrapper again. See issue #133 for the grammar change that would
/// make this unnecessary.
fn line_hole_ordinals(code: &[FlatNode]) -> Vec<usize> {
    let blank = |n: &FlatNode| match n {
        FlatNode::Str(s) => s.trim().is_empty(),
        _ => false,
    };
    let mut out = Vec::new();
    let mut ordinal = 0usize;
    for (i, node) in code.iter().enumerate() {
        if !matches!(node, FlatNode::Hole) {
            continue;
        }
        // Only whitespace may separate the hole from the line break on each
        // side (or from the start/end of the fragment).
        let alone_before = code[..i]
            .iter()
            .rev()
            .take_while(|n| !matches!(n, FlatNode::NewLine))
            .all(blank);
        let alone_after = code[i + 1..]
            .iter()
            .take_while(|n| !matches!(n, FlatNode::NewLine))
            .all(blank);
        if alone_before && alone_after {
            out.push(ordinal);
        }
        ordinal += 1;
    }
    out
}

/// Rebuild `code` with [`CHECK_PREFIX`] inserted before each hole whose ordinal
/// is in `targets`.
fn wrap_line_holes<'a>(code: &[FlatNode<'a>], targets: &[usize]) -> Vec<FlatNode<'a>> {
    let mut out = Vec::with_capacity(code.len() + targets.len());
    let mut ordinal = 0usize;
    for node in code {
        if matches!(node, FlatNode::Hole) {
            if targets.contains(&ordinal) {
                out.push(FlatNode::Str(CHECK_PREFIX));
            }
            ordinal += 1;
        }
        out.push(node.clone());
    }
    out
}

/// Undo [`wrap_line_holes`] in the parsed tree: replace each `check` tuple that
/// wraps one of the holes we wrapped with that hole itself.
///
/// Only the wrappers *we* introduced are removed — holes are counted in tree
/// order and matched against `targets` — so a genuine `#check ↙x↘` written by
/// the author survives untouched.
fn strip_wrapped_checks(term: &Arc<QTerm>, hole_str: &str, targets: &[usize]) -> Arc<QTerm> {
    fn walk(
        term: &Arc<QTerm>,
        hole_str: &str,
        targets: &[usize],
        ordinal: &mut usize,
    ) -> Arc<QTerm> {
        let QTerm::Tuple { tag, terms, cmds } = &**term else {
            // Quotes/unquotes cannot appear inside a freshly parsed fragment.
            return term.clone();
        };

        // A `check` we introduced: `seq('#check', <hole>)`, two children whose
        // second is the hole at a targeted ordinal.
        if &**tag == "check" && terms.len() == 2 {
            if let QTerm::Tuple { tag: inner, .. } = &*terms[1] {
                if &**inner == hole_str && targets.contains(ordinal) {
                    *ordinal += 1;
                    return terms[1].clone();
                }
            }
        }

        if &**tag == hole_str {
            *ordinal += 1;
            return term.clone();
        }

        let children: Vec<Arc<QTerm>> = terms
            .iter()
            .map(|t| walk(t, hole_str, targets, ordinal))
            .collect();
        tuple(tag, &children, cmds)
    }

    let mut ordinal = 0usize;
    walk(term, hole_str, targets, &mut ordinal)
}

/// The Lean `Language`: [`TSLanguage<LeanProvider>`] plus the bare-term retry
/// described on [`CHECK_PREFIX`].
#[derive(Default)]
pub struct LeanLanguage(TSLanguage<LeanProvider>);

impl Language for LeanLanguage {
    type Post = TSLanguagePost;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        // A command fragment (`def …`, `theorem …`, a whole module) parses on
        // its own; only a bare term needs the wrapper, so try plain first and
        // keep this a no-op for everything else.
        let err = match self.0.parse_pre(ikind, code) {
            Ok(post) => return Ok(post),
            Err(e) => e,
        };

        let mut wrapped = Vec::with_capacity(code.len() + 1);
        wrapped.push(FlatNode::Str(CHECK_PREFIX));
        wrapped.extend_from_slice(code);

        // Report the *original* error whenever the retry doesn't yield a clean
        // single-term `check` — the wrapper is an implementation detail, and
        // naming it would only mislead. Note the retry can *succeed* on input
        // that is not a term at all (`#check` followed by several commands
        // still parses as a module), which is why the shape is re-checked here
        // rather than trusted.
        if let Some(post) = self
            .0
            .parse_pre(Some(InnerKind::Expr), &wrapped)
            .ok()
            .and_then(|post| {
                let qterm = strip_check(&post.qterm).ok()?;
                Some(TSLanguagePost {
                    qterm: (*qterm).clone(),
                    ..post
                })
            })
        {
            return Ok(post);
        }

        // Last resort: holes at *command* position, which a bare identifier
        // cannot occupy. Wrap only the holes that sit alone on their own line
        // and strip the wrappers back out of the tree.
        let targets = line_hole_ordinals(code);
        if !targets.is_empty() {
            let wrapped = wrap_line_holes(code, &targets);
            if let Some(post) = self.0.parse_pre(ikind, &wrapped).ok().map(|post| {
                let hole_str = post.hole_str;
                let qterm = strip_wrapped_checks(&arc(post.qterm.clone()), hole_str, &targets);
                TSLanguagePost {
                    qterm: (*qterm).clone(),
                    ..post
                }
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

/// Boxed-`Post` form of [`LeanLanguage`], for the dynamic registry.
#[derive(Default)]
pub struct DynLeanLanguage(LeanLanguage);

impl Language for DynLeanLanguage {
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
