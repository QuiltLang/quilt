//! Direct term-lifting for the Python meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. The emitted source is **Python**, targeting a
//! `quilt` Python runtime whose builder mirrors the Rust `QTermBuilder` fluent
//! API (`tb`/`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`, `quote`/`unquote`, `leaf`/`sym`,
//! `cmd`/`write`/`push`/`NL`/`POP`/`HOLE`).
//!
//! The fold that assembles those chains is shared with the Rust and TypeScript
//! metas — see [`crate::langs::chain`]. All that is Python-specific is which
//! fragments it writes, which is a generated table, and the escaping below.
//!
//! One difference is not a spelling and so is not in the table: a variadic node
//! is a fluent `.e()` emit chain rather than Rust's imperative
//! `{ let mut b_ = ..; ..; b_.b() }` block, which has no Python
//! expression-context equivalent. (A consequence: statement-context splicing —
//! control flow inside `↙..↘` that emits into a named `b_` — is not expressible
//! in Python and is unsupported.)

use crate::langs::chain::{Chain, Lit, PYTHON};
use crate::prelude::*;
use crate::term::CmdOrHole;

/**************************************************************/

/// Render a Python string literal, escaping `"` (mirrors `rust::ops::str_lit`).
fn str_lit(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// The shared builder-call fold, spelled for Python.
const CHAIN: Chain = Chain::new(&PYTHON, Lit::Flat(str_lit));

/**************************************************************/

/// Build code that reconstructs a tuple: `tb(tag).w(..).c(child)..b()`, using
/// the `sym`/`leaf` shorthands when possible. `children` are the already-built
/// child expressions spliced at hole positions.
pub fn build_tuple_code(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    CHAIN.tuple_code(tag, cmds, children)
}

/// Build `quote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
pub fn build_quote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    CHAIN.quote_code(tag, index, lang, term, cmds)
}

/// Build `unquote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
pub fn build_unquote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    CHAIN.unquote_code(tag, index, lang, term, cmds)
}

/// Build a variadic node as a fluent emit chain:
/// `tb(tag).e(child).p("..").n()..b()`. `children` are the already-expanded
/// terms; each is emitted with `.e(..)` (which, like Rust's `.emit(&mut b_)`,
/// appends one-or-more children). Unlike Rust's variadic, there is no named
/// `b_`, so statement-context splicing is unsupported (see module docs).
pub fn build_variadic_block(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    CHAIN.variadic_code(tag, cmds, children)
}

/**************************************************************/

/// Make an identifier term (the `⟨N⟩` operator).
pub fn name(s: &str) -> Arc<QTerm> {
    leaf("identifier", s)
}
