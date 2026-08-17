//! Plain text: quote an arbitrary fragment with no language-specific parsing.
//!
//! Text has no grammar, so "parsing" is just recording the literal runs and the
//! holes between them. The resulting term is a single `text` tuple whose `cmds`
//! interleave the writes and the holes, which is exactly what serialization
//! needs and nothing more.
//!
//! This used to be four `todo!()`s — so `text↖…↗` panicked rather than erroring,
//! the last of the panics issue #11 set out to remove. The conformance battery
//! (#144) treats a panic as a hard failure for every language including the
//! unsupported cases, which is what surfaced it.

use crate::lang::{Arity, Comments, FlatNode, Hole, InnerKind, Language, LanguagePost};
use crate::prelude::*;
use crate::qterm::QTerm;
use crate::term::CmdOrHole;
use miette::bail;

/**************************************************************/

/// The tag every text fragment carries. Text has no node kinds, so one tag is
/// the whole vocabulary.
pub const TEXT_TAG: &str = "text";

/// Flatten `code` into the `cmds` sequence of a `text` tuple, counting holes.
fn build(code: &[FlatNode]) -> (Vec<CmdOrHole>, usize) {
    let mut cmds = Vec::with_capacity(code.len());
    let mut holes = 0;
    for node in code {
        match node {
            FlatNode::Str(s) => cmds.push(cmd(write(s))),
            FlatNode::NewLine => cmds.push(cmd(NL)),
            FlatNode::Hole => {
                cmds.push(HOLE);
                holes += 1;
            }
        }
    }
    (cmds, holes)
}

#[derive(Default)]
pub struct TextLanguage;

impl Language for TextLanguage {
    type Post = TextLanguagePost;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        Ok(TextLanguagePost::new(code))
    }

    /// A text fragment is an opaque run of characters — never a statement or an
    /// item, so hosts splice it as a value.
    fn typ(&self, _tag: &str) -> InnerKind {
        InnerKind::Expr
    }

    /// Text has no nesting, so it accepts as many spliced children as the
    /// author writes holes for.
    fn arity(&self, _tag: &str) -> Arity {
        Arity::Variadic
    }
}

#[derive(Debug)]
pub struct TextLanguagePost {
    cmds: Box<[CmdOrHole]>,
    holes: Box<[Hole]>,
}

impl TextLanguagePost {
    fn new(code: &[FlatNode]) -> Self {
        let (cmds, n) = build(code);
        let holes = (0..n)
            .map(|_| Hole {
                otag: TEXT_TAG.into(),
                // Anything can be spliced into text; the host decides how the
                // child renders, so we impose no kind.
                ikind: None,
                prefix: Box::default(),
            })
            .collect();
        Self {
            cmds: cmds.into(),
            holes,
        }
    }
}

impl LanguagePost for TextLanguagePost {
    fn holes(&self) -> &[Hole] {
        &self.holes
    }

    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        if plugs.len() != self.holes.len() {
            bail!(
                "text: expected {} spliced term(s), got {}",
                self.holes.len(),
                plugs.len()
            );
        }
        Ok(crate::qterm::tuple(TEXT_TAG, plugs, &self.cmds))
    }
}

impl Comments for TextLanguage {
    /// Plain text has no comment syntax, so every spelling would corrupt the
    /// output rather than annotate it.
    const LINE: Option<&'static str> = None;
}

/**************************************************************/

#[derive(Default)]
pub struct DynTextLanguage;

impl Language for DynTextLanguage {
    type Post = Box<dyn LanguagePost>;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post> {
        Ok(bx(TextLanguagePost::new(code)))
    }

    fn typ(&self, _tag: &str) -> InnerKind {
        InnerKind::Expr
    }

    fn arity(&self, _tag: &str) -> Arity {
        Arity::Variadic
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::flat_nodes;
    use crate::term::{STerm as _, Term as _};

    #[test]
    fn roundtrips_literal_text() -> Result<()> {
        let mut lang = TextLanguage;
        let term = lang.parse(&flat_nodes("hello, world"))?;
        assert_eq!(term.coparse(), "hello, world");
        assert_eq!(term.tag(), crate::qterm::QTermTag::tuple(TEXT_TAG));
        Ok(())
    }

    #[test]
    fn roundtrips_multiline_text() -> Result<()> {
        let mut lang = TextLanguage;
        let src = "line one\nline two\n\nline four";
        assert_eq!(lang.parse(&flat_nodes(src))?.coparse(), src);
        Ok(())
    }

    #[test]
    fn splices_holes_in_order() -> Result<()> {
        let mut lang = TextLanguage;
        let code = [
            FlatNode::Str("a="),
            FlatNode::Hole,
            FlatNode::Str(" b="),
            FlatNode::Hole,
        ];
        let term = lang.parse_with(&code, &[leaf(TEXT_TAG, "1"), leaf(TEXT_TAG, "2")])?;
        assert_eq!(term.coparse(), "a=1 b=2");
        Ok(())
    }

    /// The stub used to `todo!()`; a wrong plug count must be an error.
    #[test]
    fn wrong_plug_count_is_an_error() -> Result<()> {
        let mut lang = TextLanguage;
        let post = lang.parse_pre(None, &[FlatNode::Hole])?;
        let err = post.parse_post(&[]).unwrap_err();
        assert!(err.to_string().contains("expected 1"), "{err}");
        Ok(())
    }
}
