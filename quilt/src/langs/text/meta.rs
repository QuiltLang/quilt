//! Text is a target, not a host — but say so, rather than panicking.
//!
//! `text` has no grammar and no runtime, so it cannot drive expansion: there is
//! nothing for a `.txt.quilt` metaprogram to be written *in*. `langs/omni.rs`
//! reflects that by leaving `text` out of the `metas` section, so this type is
//! unreachable through `Omni`.
//!
//! It is still `pub`, though, so a consumer wiring it into a `Single` or a
//! `DictMulti` by hand reaches it — and every method used to be a `todo!()`,
//! which panicked. That is the class of panic issue #11 set out to remove;
//! `langs/text/lang.rs` was converted at the time ("the last of the panics issue
//! #11 set out to remove"), and this file beside it was missed (#174, finding J).
//! Each method now returns an error saying why text cannot host and what to do
//! instead.

use miette::Result;

use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

/// The diagnostic every method returns: text cannot act as a host language.
fn not_a_host(what: &str) -> miette::Report {
    miette!(
        "text can't act as a host language (expanding {what}): it has no grammar to \
         write a metaprogram in and no runtime to build terms with. Quote text from \
         a real host instead — `txt↖ … ↗` inside a `.rs.quilt` or `.py.quilt` file — \
         and that host's meta-language will drive the expansion."
    )
}

#[derive(Default)]
pub struct TextMetaLanguage;

impl MetaLanguage for TextMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Err(not_a_host("a quote"))
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        Err(not_a_host("an unquote"))
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        _tag: &str,
        _qterms: &[Arc<QTerm>],
        _cmds: &[CmdOrHole],
        _arity: Arity,
    ) -> Result<Arc<QTerm>> {
        Err(not_a_host("a tuple"))
    }
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    /// Every method is an `Err`, never a panic. The conformance battery (#144)
    /// treats a panic as a hard failure for every language including the
    /// unsupported cases, which is what surfaced the same bug in
    /// `langs/text/lang.rs`.
    #[test]
    fn every_method_errors_instead_of_panicking() {
        let meta = TextMetaLanguage;
        let term = leaf("text", "hi");

        let results = [
            meta.expand_quote("text", "text", 1, "text", &term, &[]),
            meta.expand_unquote("text", "text", 1, "text", &term, &[]),
            meta.expand_tuple("text", "text", &[], &[], Arity::Unknown),
        ];
        for result in results {
            let err = result.expect_err("text cannot host, so this must be an Err");
            assert!(
                err.to_string().contains("can't act as a host language"),
                "diagnostic should explain why, got: {err}"
            );
        }
    }
}
