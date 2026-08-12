use crate::{
    prelude::*,
    strcmd::{PrefixWriter, StrCmd},
    term::{CmdOrHole, STerm, Term},
    validate::Validate,
};
use serde::{Deserialize, Serialize};
use std::{iter::once, sync::Arc};

/**************************************************************/

pub type Gaps = Box<[CmdOrHole]>;

#[derive(Debug, Clone, Eq, Serialize, Deserialize)]
pub enum QTerm {
    Quote {
        tag: Box<str>,
        index: Index,
        lang: Box<str>,
        term: Arc<QTerm>,
        cmds: Box<[CmdOrHole]>,
        /// Byte range of the `anno↖…↗` in the original source, when this term
        /// came from parsing one (`build_nodes` attaches it); `None` for
        /// constructed terms. Diagnostic metadata only — not part of equality.
        /// No serde skip: `reduce` round-trips terms through postcard, which
        /// is positional and cannot tolerate omitted fields.
        span: Option<Span>,
    },
    Unquote {
        tag: Box<str>,
        index: Index,
        lang: Box<str>,
        term: Arc<QTerm>,
        cmds: Box<[CmdOrHole]>,
        /// Byte range of the `anno↙…↘` in the original source, when this term
        /// came from parsing one; diagnostic metadata only, like a quote's.
        span: Option<Span>,
    },
    Tuple {
        tag: Box<str>,
        terms: Box<[Arc<QTerm>]>,
        cmds: Box<[CmdOrHole]>,
    },
}

/// Spans are diagnostic metadata, not part of a term's identity: a parsed term
/// (which carries spans) compares equal to the equivalent constructed one.
impl PartialEq for QTerm {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                QTerm::Quote {
                    tag,
                    index,
                    lang,
                    term,
                    cmds,
                    span: _,
                },
                QTerm::Quote {
                    tag: tag2,
                    index: index2,
                    lang: lang2,
                    term: term2,
                    cmds: cmds2,
                    span: _,
                },
            )
            | (
                QTerm::Unquote {
                    tag,
                    index,
                    lang,
                    term,
                    cmds,
                    span: _,
                },
                QTerm::Unquote {
                    tag: tag2,
                    index: index2,
                    lang: lang2,
                    term: term2,
                    cmds: cmds2,
                    span: _,
                },
            ) => tag == tag2 && index == index2 && lang == lang2 && term == term2 && cmds == cmds2,
            (
                QTerm::Tuple { tag, terms, cmds },
                QTerm::Tuple {
                    tag: tag2,
                    terms: terms2,
                    cmds: cmds2,
                },
            ) => tag == tag2 && terms == terms2 && cmds == cmds2,
            _ => false,
        }
    }
}

pub fn quote(
    tag: &str,
    index: Index,
    lang: &str,
    term: Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    qquote(tag, index, lang, term, cmds).into()
}
pub fn unquote(
    tag: &str,
    index: Index,
    lang: &str,
    term: Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    qunquote(tag, index, lang, term, cmds).into()
}
pub fn tuple(tag: &str, terms: &[Arc<QTerm>], cmds: &[CmdOrHole]) -> Arc<QTerm> {
    QTerm::Tuple {
        tag: tag.into(),
        terms: terms.into(),
        cmds: cmds.into(),
    }
    .into()
}
pub fn leaf(s: &str, code: &str) -> Arc<QTerm> {
    tuple(s, &[], &[cmd(write(code))])
}
pub fn sym(s: &str) -> Arc<QTerm> {
    leaf(s, s)
}

pub fn qquote(tag: &str, index: Index, lang: &str, term: Arc<QTerm>, cmds: &[CmdOrHole]) -> QTerm {
    qquote_at(tag, index, lang, term, cmds, None)
}
pub fn qunquote(
    tag: &str,
    index: Index,
    lang: &str,
    term: Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> QTerm {
    qunquote_at(tag, index, lang, term, cmds, None)
}
pub fn qquote_at(
    tag: &str,
    index: Index,
    lang: &str,
    term: Arc<QTerm>,
    cmds: &[CmdOrHole],
    span: Option<Span>,
) -> QTerm {
    QTerm::Quote {
        tag: tag.into(),
        index,
        lang: lang.into(),
        term,
        cmds: cmds.into(),
        span,
    }
}
pub fn qunquote_at(
    tag: &str,
    index: Index,
    lang: &str,
    term: Arc<QTerm>,
    cmds: &[CmdOrHole],
    span: Option<Span>,
) -> QTerm {
    QTerm::Unquote {
        tag: tag.into(),
        index,
        lang: lang.into(),
        term,
        cmds: cmds.into(),
        span,
    }
}
pub fn qtuple(tag: &str, terms: &[Arc<QTerm>], cmds: &[CmdOrHole]) -> QTerm {
    QTerm::Tuple {
        tag: tag.into(),
        terms: terms.into(),
        cmds: cmds.into(),
    }
}
pub fn qleaf(s: &str, code: &str) -> QTerm {
    qtuple(s, &[], &[cmd(write(code))])
}
pub fn qsym(s: &str) -> QTerm {
    qleaf(s, s)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QTermTag {
    Quote(Box<str>, Index, Box<str>),
    Unquote(Box<str>, Index, Box<str>),
    Tuple(Box<str>),
}

impl QTermTag {
    pub fn tuple(tag: &str) -> Self {
        QTermTag::Tuple(tag.into())
    }
}

impl QTerm {
    fn cmds(&self) -> &[CmdOrHole] {
        match self {
            QTerm::Quote { cmds, .. } | QTerm::Unquote { cmds, .. } | QTerm::Tuple { cmds, .. } => {
                cmds
            }
        }
    }

    /// Gets the first child of this term without losing whitespace.
    pub fn squash(&self) -> QTerm {
        assert!(
            self.len() == 1,
            "squash called on term without exactly one child"
        );
        let child = &self[0];
        let self_cmds = self.cmds();
        let child_cmds = child.cmds();
        let hole_ind = self_cmds
            .iter()
            .position(|cmd| matches!(cmd, CmdOrHole::Hole))
            .unwrap();
        let mut cmds = Vec::with_capacity(self_cmds.len() + child_cmds.len() - 1);
        cmds.extend(self_cmds[..hole_ind].iter().cloned());
        cmds.extend(child_cmds.iter().cloned());
        cmds.extend(self_cmds[hole_ind + 1..].iter().cloned());
        match child {
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                span,
                ..
            } => qquote_at(tag, *index, lang, term.clone(), &cmds, span.clone()),
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                span,
                ..
            } => qunquote_at(tag, *index, lang, term.clone(), &cmds, span.clone()),
            QTerm::Tuple { tag, terms, .. } => qtuple(tag, terms, &cmds),
        }
    }

    pub fn rewrite_naive(&self, find: &Self, replace: &Self) -> Arc<Self> {
        if self == find {
            return arc(replace.clone());
        }
        match self {
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                cmds,
                span,
            } => {
                let term = term.rewrite_naive(find, replace);
                arc(qquote_at(tag, *index, lang, term, cmds, span.clone()))
            }
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                span,
            } => {
                let term = term.rewrite_naive(find, replace);
                arc(qunquote_at(tag, *index, lang, term, cmds, span.clone()))
            }
            QTerm::Tuple { tag, terms, cmds } => {
                let terms = terms
                    .iter()
                    .map(|s| s.rewrite_naive(find, replace))
                    .collect::<Vec<_>>();
                tuple(tag, &terms, cmds)
            }
        }
    }

    pub fn with_content(&self, content: &str) -> Arc<Self> {
        match self {
            QTerm::Tuple { tag, terms, .. } if terms.is_empty() => leaf(tag, content),
            _ => panic!("with_content: not a leaf: {self:?}"),
        }
    }

    pub fn sexp(&self) -> String {
        match self {
            QTerm::Quote { lang, term, .. } => {
                format!("{}↖{}↗", lang, term.sexp())
            }
            QTerm::Unquote { lang, term, .. } => {
                format!("{}↙{}↘", lang, term.sexp())
            }
            QTerm::Tuple { tag, terms, .. } => {
                if terms.is_empty() {
                    return format!("{tag}");
                }
                let mut ret = String::new();
                ret.push_str(tag);
                ret.push('(');
                for (i, term) in terms.iter().enumerate() {
                    if i > 0 {
                        ret.push(',');
                    }
                    ret.push_str(&term.sexp());
                }
                ret.push(')');
                ret
            }
        }
    }
}

impl Term for QTerm {
    type Tag = QTermTag;

    fn tag(&self) -> Self::Tag {
        match self {
            QTerm::Quote {
                tag, index, lang, ..
            } => QTermTag::Quote(tag.clone(), *index, lang.clone()),
            QTerm::Unquote {
                tag, index, lang, ..
            } => QTermTag::Unquote(tag.clone(), *index, lang.clone()),
            QTerm::Tuple { tag, .. } => QTermTag::Tuple(tag.clone()),
        }
    }

    fn children(&self) -> impl Iterator<Item = &Self> {
        let ret: Box<dyn Iterator<Item = _>> = match self {
            QTerm::Quote { term, .. } | QTerm::Unquote { term, .. } => bx(once(term.as_ref())),
            QTerm::Tuple { terms, .. } => bx(terms.iter().map(|x| x.as_ref())),
        };
        ret
    }

    fn len(&self) -> usize {
        match self {
            QTerm::Quote { .. } | QTerm::Unquote { .. } => 1,
            QTerm::Tuple { terms, .. } => terms.len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            QTerm::Quote { .. } | QTerm::Unquote { .. } => false,
            QTerm::Tuple { terms, .. } => terms.is_empty(),
        }
    }
}

impl QTerm {
    /// Whether this term is a Quilt operator that `Multi::build_nodes` deferred
    /// to a later stage — `↑`, `←` or `<anno>↓` inside an unresolved quote,
    /// plugged back in as `leaf(ident_tag, glyph)`.
    ///
    /// It matters only when rendering `.quilt` source (see
    /// [`STerm::coparse_quilt`]): such a glyph is Quilt syntax and must stay
    /// bare, while the same glyph arriving as escaped content must be written
    /// `\`-escaped. The two are told apart by the tag. A language's own glyph
    /// token comes back from its grammar as an anonymous node, which `sym`
    /// builds with `tag == text` (Lean's bind is `sym("←")`); a deferred
    /// operator is tagged with the language's *identifier* kind, which never
    /// spells itself `←`. Anything with children is a fragment, not a leaf.
    fn is_deferred_operator(&self) -> bool {
        let QTerm::Tuple { tag, terms, cmds } = self else {
            return false;
        };
        if !terms.is_empty() {
            return false;
        }
        match cmds.as_ref() {
            [CmdOrHole::Cmd(StrCmd::Write(text))] => {
                **text != **tag && crate::glyphs::is_deferred_operator(text)
            }
            _ => false,
        }
    }
}

impl STerm for QTerm {
    fn write<W: std::io::Write>(&self, writer: &mut PrefixWriter<'_, W>) {
        match self {
            QTerm::Quote { term, cmds, .. } | QTerm::Unquote { term, cmds, .. } => {
                // A quote's own `cmds` are the arrows and the language
                // annotation — Quilt syntax, not content — so they are never
                // escaped, however the body is written.
                let outer = writer.set_escaping(false);
                let outer_bracketed = writer.bracketed();
                for cmd in cmds {
                    match cmd {
                        CmdOrHole::Cmd(cmd) => writer.interpret(cmd),
                        CmdOrHole::Hole => {
                            writer.set_escaping(outer);
                            writer.set_bracketed(true);
                            term.write(writer);
                            writer.set_bracketed(outer_bracketed);
                            writer.set_escaping(false);
                        }
                    }
                }
                writer.set_escaping(outer);
            }
            QTerm::Tuple { terms, cmds, .. } => {
                let outer = if writer.bracketed() && self.is_deferred_operator() {
                    writer.set_escaping(false)
                } else {
                    writer.escaping()
                };
                let mut children = terms.iter();
                for cmd in cmds {
                    match cmd {
                        CmdOrHole::Cmd(cmd) => writer.interpret(cmd),
                        CmdOrHole::Hole => children.next().unwrap().write(writer),
                    }
                }
                writer.set_escaping(outer);
            }
        }
    }
}

impl std::ops::Index<usize> for QTerm {
    type Output = QTerm;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index).unwrap()
    }
}

impl Validate for QTerm {
    type Error = miette::Error;

    fn validate(self) -> Result<Self, Self::Error> {
        let mut depth: u32 = 0;
        let mut holes = 0;
        for cmd in self.cmds() {
            match cmd {
                CmdOrHole::Cmd(cmd) => match cmd {
                    StrCmd::Push(_) => depth += 1,
                    StrCmd::Pop => {
                        if depth == 0 {
                            miette::bail!("running stack depth below 0");
                        }
                        depth -= 1;
                    }
                    _ => (),
                },
                CmdOrHole::Hole => {
                    holes += 1;
                }
            }
        }
        if depth != 0 {
            miette::bail!("total stack depth not 0");
        }
        if holes != self.len() {
            miette::bail!("number of holes does not match number of children");
        }
        Ok(self)
    }
}

/**************************************************************/

pub struct QTermBuilder {
    tag: QTermTag,
    children: Vec<Arc<QTerm>>,
    cmds: Vec<CmdOrHole>,
    /// Source span for the built Quote/Unquote (ignored for Tuple).
    span: Option<Span>,
}

impl QTermBuilder {
    pub fn new(tag: QTermTag) -> Self {
        Self {
            tag,
            children: Vec::new(),
            cmds: Vec::new(),
            span: None,
        }
    }

    pub fn span(&mut self, span: Span) -> &mut Self {
        self.span = Some(span);
        self
    }

    pub fn child(&mut self, child: &Arc<QTerm>) -> &mut Self {
        self.cmds.push(CmdOrHole::Hole);
        self.children.push(child.clone());
        self
    }

    pub fn cmd(&mut self, cmd: &StrCmd) -> &mut Self {
        self.cmds.push(CmdOrHole::Cmd(cmd.clone()));
        self
    }

    pub fn build(self) -> QTerm {
        match self.tag {
            QTermTag::Quote(tag, index, lang) => {
                assert_eq!(self.children.len(), 1);
                qquote_at(
                    &tag,
                    index,
                    &lang,
                    self.children[0].clone(),
                    &self.cmds,
                    self.span,
                )
            }
            QTermTag::Unquote(tag, index, lang) => {
                assert_eq!(self.children.len(), 1);
                qunquote_at(
                    &tag,
                    index,
                    &lang,
                    self.children[0].clone(),
                    &self.cmds,
                    self.span,
                )
            }
            QTermTag::Tuple(tag) => qtuple(&tag, &self.children, &self.cmds),
        }
    }

    pub fn first(self) -> Option<Arc<QTerm>> {
        self.children.first().cloned()
    }

    // convenience methods

    pub fn write(&mut self, s: &str) -> &mut Self {
        if s.is_empty() {
            self
        } else {
            self.cmd(&StrCmd::Write(s.into()))
        }
    }

    pub fn nl(&mut self) -> &mut Self {
        self.cmd(&StrCmd::NewLine)
    }

    pub fn push(&mut self, s: &str) -> &mut Self {
        // unlike write, we must push an empty string to avoid unmatched pops
        // assert!(!s.is_empty());
        self.cmd(&StrCmd::Push(s.into()))
    }

    pub fn pop(&mut self) -> &mut Self {
        self.cmd(&StrCmd::Pop)
    }

    pub fn emit(&mut self, x: impl Emit) -> &mut Self {
        x.emit(self);
        self
    }

    pub fn write_if(&mut self, s: &str, b: bool) -> &mut Self {
        if b && !s.is_empty() {
            self.cmd(&StrCmd::Write(s.into()))
        } else {
            self
        }
    }

    pub fn nl_if(&mut self, b: bool) -> &mut Self {
        if b {
            self.cmd(&StrCmd::NewLine)
        } else {
            self
        }
    }

    pub fn push_if(&mut self, s: &str, b: bool) -> &mut Self {
        if b {
            self.cmd(&StrCmd::Push(s.into()))
        } else {
            self
        }
    }

    pub fn pop_if(&mut self, b: bool) -> &mut Self {
        if b {
            self.cmd(&StrCmd::Pop)
        } else {
            self
        }
    }

    pub fn c(mut self, child: &Arc<QTerm>) -> Self {
        self.child(child);
        self
    }
    pub fn w(mut self, s: &str) -> Self {
        self.write(s);
        self
    }
    pub fn n(mut self) -> Self {
        self.nl();
        self
    }
    pub fn p(mut self, s: &str) -> Self {
        self.push(s);
        self
    }
    pub fn x(mut self) -> Self {
        self.pop();
        self
    }
    pub fn e(mut self, x: impl Emit) -> Self {
        self.emit(x);
        self
    }
    pub fn b(self) -> Arc<QTerm> {
        arc(self.build())
    }
}

pub fn qb(tag: &str, index: Index, lang: &str) -> QTermBuilder {
    QTermBuilder::new(QTermTag::Quote(tag.into(), index, lang.into()))
}
pub fn ub(tag: &str, index: Index, lang: &str) -> QTermBuilder {
    QTermBuilder::new(QTermTag::Unquote(tag.into(), index, lang.into()))
}
pub fn tb(tag: &str) -> QTermBuilder {
    QTermBuilder::new(QTermTag::Tuple(tag.into()))
}

/**************************************************************/

/// What a child of a variadic node may contribute to its parent's builder:
/// exactly one term, several, or none.
///
/// # Why the impls are all on references
///
/// Expanded code calls this as `x.emit(&mut b_);`, where `x` is whatever
/// expression the author unquoted. Implementing it for `Arc<QTerm>` made that
/// call **move** `x` — fine when `x` is a freshly built term, fatal when the
/// author named it and used it twice:
///
/// ```text
/// let name = ident(&format!("pow{n}"));
/// decls.push(lean↖def ↙name↘ (x : Nat) : Nat := ↙body(n)↘↗);
/// decls.push(lean↖theorem ↙lemma↘ : ↙name↘ x = x ^ ↑n := by simp [↙name↘]↗);
/// //                                  ^^^^ E0382: use of moved value
/// ```
///
/// Nothing here needs ownership — every impl only reads — so the impls are on
/// `&T` instead, and method-call auto-ref makes `x.emit(&mut b_)` borrow. The
/// generated spelling is unchanged; it just stopped consuming its argument.
///
/// This only became reachable when the variadic tables started following the
/// grammars (#202): with two tags per language, an unquote almost never sat
/// directly under a variadic node, so the accumulator form was rare.
pub trait Emit {
    fn emit(self, builder: &mut QTermBuilder);
}

impl Emit for &Arc<QTerm> {
    fn emit(self, builder: &mut QTermBuilder) {
        builder.child(self);
    }
}

impl Emit for &() {
    fn emit(self, _builder: &mut QTermBuilder) {}
}

/// A child that contributes several terms — what `←` produces.
///
/// The bound is `&T: Emit` rather than `T: Emit` so the elements are borrowed
/// out of the vector too, keeping the whole chain non-consuming.
impl<'a, T> Emit for &'a Vec<T>
where
    &'a T: Emit,
{
    fn emit(self, builder: &mut QTermBuilder) {
        for item in self {
            item.emit(builder);
        }
    }
}

impl Emit for &str {
    fn emit(self, builder: &mut QTermBuilder) {
        builder.write(self);
    }
}

impl Emit for &StrCmd {
    fn emit(self, builder: &mut QTermBuilder) {
        builder.cmd(self);
    }
}

#[allow(unused_macros)]
macro_rules! emit {
    // WARN: doing `$b.emit(&$e);` will cause double-borrow errors in nested cases
    ($b:expr, $e:expr) => {
        $e.emit(&mut $b);
    };
    ($b:expr, $e:expr, $($rest:expr),+) => {
        emit!($b, $e);
        emit!($b, $($rest),+);
    };
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit() {
        let mut b_ = tb("block");
        sym("a").emit(&mut b_);
        ().emit(&mut b_);
        vec![sym("b"), sym("c")].emit(&mut b_);
        let qterm = b_.build();

        let expected = tb("block").c(&sym("a")).c(&sym("b")).c(&sym("c")).build();
        assert_eq!(qterm, expected);
    }

    /// Emitting a named term must not consume it.
    ///
    /// This is the shape expanded code produces for a variadic node with an
    /// unquoted child, and authors reuse such a binding constantly — `let name
    /// = ident(…)` used in both a `def` and the theorem about it. While `Emit`
    /// was implemented for `Arc<QTerm>` the emit *moved* it, so the second use
    /// was E0382 and the generated program did not compile.
    ///
    /// It is a compile-time property, so the assertion is that this function
    /// builds at all; the equality below just keeps it honest about emitting
    /// the same term twice rather than dropping one.
    #[test]
    fn emit_borrows_rather_than_consuming() {
        let name = sym("a");
        let mut b_ = tb("block");
        name.emit(&mut b_);
        // Still usable: the emit above borrowed it.
        name.emit(&mut b_);
        let terms = vec![sym("b")];
        terms.emit(&mut b_);
        terms.emit(&mut b_);
        assert_eq!(name, sym("a"));

        let expected = tb("block")
            .c(&sym("a"))
            .c(&sym("a"))
            .c(&sym("b"))
            .c(&sym("b"))
            .build();
        assert_eq!(b_.build(), expected);
    }

    #[test]
    fn emit_macro() {
        let mut b_ = tb("block");
        emit!(b_, sym("a"), (), vec![sym("b"), sym("c")]);
        let qterm = b_.build();

        let expected = tb("block").c(&sym("a")).c(&sym("b")).c(&sym("c")).build();
        assert_eq!(qterm, expected);
    }

    #[test]
    fn emit_macro_nested() {
        let mut b_ = tb("block");
        emit!(b_, sym("a"), {
            emit!(b_, sym("b"));
            sym("c")
        });
        let qterm = b_.build();

        let expected = tb("block").c(&sym("a")).c(&sym("b")).c(&sym("c")).build();
        assert_eq!(qterm, expected);
    }

    /// A term with `span`s set, nested inside a tuple, survives postcard.
    ///
    /// This is the guard the `span` fields' "no serde skip" comments ask for.
    /// The shared runtime corpus round-trips every shape it builds
    /// (`conformance/runtime/cases.json`, issue #192), but those terms are all
    /// *constructed*, so their spans are all `None` — and a `None` that is
    /// dropped on both ends re-encodes to the same bytes and coparses to the
    /// same text, so the corpus cannot see it go. Only a parsed term carries a
    /// span, and only Rust can build one, so the check lives here.
    ///
    /// It has to assert on the spans directly: [`PartialEq`] ignores them by
    /// design and `coparse` never renders them, so the two obvious assertions
    /// both pass on a term that lost them.
    #[test]
    fn postcard_round_trip_preserves_spans() {
        let inner = qunquote_at("x", 1, "rs", sym("n"), &[HOLE], Some(7..12)).into();
        let quoted = qquote_at("q", 0, "py", inner, &[HOLE], Some(3..20)).into();
        let term: Arc<QTerm> = tb("block").w("[").c(&quoted).w("]").b();

        let bytes = postcard::to_allocvec(&term).expect("serializes");
        let back: Arc<QTerm> = postcard::from_bytes(&bytes).expect("deserializes");

        assert_eq!(back, term, "round trip changed the tree");
        assert_eq!(
            back.coparse(),
            term.coparse(),
            "round trip changed the text"
        );
        assert_eq!(
            postcard::to_allocvec(&back).expect("re-serializes"),
            bytes,
            "round trip re-serializes to different bytes"
        );

        // The part neither equality above can see.
        let QTerm::Tuple { terms, .. } = &*back else {
            panic!("expected a tuple");
        };
        let QTerm::Quote { span, term, .. } = &*terms[0] else {
            panic!("expected a quote");
        };
        assert_eq!(*span, Some(3..20), "the quote's span was lost");
        let QTerm::Unquote { span, .. } = &**term else {
            panic!("expected an unquote");
        };
        assert_eq!(*span, Some(7..12), "the nested unquote's span was lost");
    }
}
