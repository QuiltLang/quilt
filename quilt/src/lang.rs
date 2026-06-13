use crate::prelude::*;
use crate::qterm::QTerm;
use std::fmt::Debug;
use std::sync::Arc;

/**************************************************************/

/// Kinds of terms. These are sorts of messages for communicating between parsers.
///
/// The variants name the grammatical roles a fragment can play. Languages map
/// their tree-sitter node tags onto these via [`Language::typ`] /
/// [`Language::classify_term`], and holes record the kind their position
/// demands via [`TSProvider::hole_kind`](crate::treesitter::TSProvider::hole_kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InnerKind {
    /// An expression — produces a value (e.g. `1 + 2`, a function call).
    Expr,
    /// A statement — runs for its effect and lives among siblings in a block
    /// (e.g. `let x = 1;`, an expression statement).
    Stmt,
    /// An item / top-level declaration (e.g. `fn`, `struct`, `impl`, `use`).
    /// Like [`Stmt`](InnerKind::Stmt) it sits among siblings rather than
    /// producing a value, but it is a definition rather than a runtime
    /// statement. [`InnerKind::is_stmt_like`] groups the two.
    Item,
    /// A brace-delimited block *body* — the `{ … }` of a function, loop, or
    /// branch. Distinct from a block used as a value (which is an
    /// [`Expr`](InnerKind::Expr)); see
    /// [`TSProvider::hole_kind`](crate::treesitter::TSProvider::hole_kind),
    /// which reads the surrounding tree to tell the two apart.
    Block,
    #[default]
    /// A whole file / module — a sequence of top-level items.
    File,
    // TODO: add more, language specific types, number, function, etc.
}

impl InnerKind {
    /// Whether a fragment of this kind lives in a "variadic" sibling position
    /// (a statement or an item) rather than producing a value. The expander's
    /// emit/splice heuristics treat statements and items alike, so they ask
    /// this instead of comparing against a single variant.
    #[must_use]
    pub fn is_stmt_like(self) -> bool {
        matches!(self, InnerKind::Stmt | InnerKind::Item)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Arity {
    #[default]
    Unknown,
    Const(u8),
    Variadic,
}

#[derive(Debug, Clone, Default)]
pub struct Hole {
    pub otag: Box<str>, // outer tag: where this hole appears in the outer language
    /// Inner kind: what kind of thing can fill this hole, derived from `otag`
    /// via the outer language's [`Language::typ`]. Threaded into the nested
    /// `parse_pre` so an unquote body is parsed with the kind its position
    /// demands (e.g. a `{ }` in statement position) instead of a guess.
    pub ikind: Option<InnerKind>,
    pub prefix: Box<[Box<str>]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatNode<'a> {
    Hole,
    Str(&'a str),
    NewLine,
}

// WARN: assumers no newlines
pub fn one_liner(s: &str) -> [FlatNode<'_>; 1] {
    [FlatNode::Str(s)]
}

pub fn flat_nodes(s: &str) -> Vec<FlatNode<'_>> {
    let lines = s.split('\n').collect::<Vec<_>>();
    let mut nodes = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            nodes.push(FlatNode::NewLine);
        }
        if !line.is_empty() {
            nodes.push(FlatNode::Str(line));
        }
    }
    nodes
}

impl std::ops::Add for &Hole {
    type Output = Hole;

    fn add(self, rhs: Self) -> Self::Output {
        let otag = rhs.otag.clone();
        let ikind = rhs.ikind;
        let mut prefix = self.prefix.to_vec();
        prefix.extend_from_slice(&rhs.prefix);
        let prefix = prefix.into();
        Hole {
            otag,
            ikind,
            prefix,
        }
    }
}

pub trait Language {
    type Post: LanguagePost;

    /// Parse a string into something that can be filled with plugs and a list of hole types.
    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post>;

    /// Convenience function to parse and fill with plugs.
    fn parse_with(&mut self, code: &[FlatNode], plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        self.parse_pre(Default::default(), code)?.parse_post(plugs)
    }

    /// Convenience function to parse without plugs.
    fn parse(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_with(code, &[])
    }

    /// Convenience function to parse as a given `TermKind` and fill with plugs.
    fn parse_as(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_pre(ikind, code)?.parse_post(&[])
    }

    fn parse_expr(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_as(Some(InnerKind::Expr), code)
    }

    fn parse_stmt(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_as(Some(InnerKind::Stmt), code)
    }

    fn parse_file(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_as(Some(InnerKind::File), code)
    }

    fn parse_auto(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.parse_as(None, code)
    }

    fn arity(&self, _tag: &str) -> Arity {
        Default::default()
    }

    fn typ(&self, _tag: &str) -> InnerKind {
        Default::default()
    }

    /// Classify a fully-parsed `QTerm` to determine its grammatical kind.
    ///
    /// This is the *accurate* version of [`typ`]: unlike `typ`, which receives
    /// only the root tag name, `classify_term` can inspect the full term tree.
    /// This matters for languages (e.g. WGSL) where a single statement is
    /// wrapped in a `source_file` node with a trailing `;` sibling, so the
    /// root tag alone gives `File` even though the fragment is really `Stmt`.
    ///
    /// The default implementation falls back to `typ` on the root tag, which
    /// is correct for languages whose `unwrap` always squashes the wrapper.
    fn classify_term(&self, term: &QTerm) -> InnerKind {
        match term {
            QTerm::Tuple { tag, .. } => self.typ(tag),
            _ => InnerKind::default(),
        }
    }

    /// Shebang line used to run an expanded file of this language, if supported.
    /// e.g. `"#!/usr/bin/env rust-script"` or `"#!/usr/bin/env python3"`.
    fn hashbang(&self) -> Option<&'static str> {
        None
    }
}

pub trait LanguagePost: Debug {
    /// Get the hole types that this language supports.
    fn holes(&self) -> &[Hole];
    /// Fill with plugs.
    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>>;
}

/**************************************************************/

impl Language for Box<dyn Language<Post = Box<dyn LanguagePost>>> {
    type Post = Box<dyn LanguagePost>;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        self.as_mut().parse_pre(ikind, code)
    }

    fn arity(&self, tag: &str) -> Arity {
        self.as_ref().arity(tag)
    }

    fn typ(&self, tag: &str) -> InnerKind {
        self.as_ref().typ(tag)
    }

    fn classify_term(&self, term: &QTerm) -> InnerKind {
        self.as_ref().classify_term(term)
    }

    fn hashbang(&self) -> Option<&'static str> {
        self.as_ref().hashbang()
    }
}

impl LanguagePost for Box<dyn LanguagePost> {
    fn holes(&self) -> &[Hole] {
        self.as_ref().holes()
    }

    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        self.as_ref().parse_post(plugs)
    }
}
