//! Direct term-lifting for the TypeScript meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. The emitted source is **TypeScript**,
//! targeting the `quilt-wasm` runtime whose builder mirrors the Rust
//! `QTermBuilder` fluent API (`tb`/`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`,
//! `quote`/`unquote`, `leaf`/`sym`, `cmd`/`write`/`push`/`NL`/`POP`/`HOLE`).
//!
//! This used to be a near-clone of `langs::python::ops`, and the fold both were
//! copies of now lives once in [`crate::langs::chain`]. What is left here is
//! what is genuinely TypeScript's:
//!
//! * string literals escape `\` as well as `"` — escaping is runtime logic, not
//!   a shape a sample can carry, so it stays hand-written;
//! * a variadic node is a fluent `.e()` emit chain (as in Python), so
//!   statement-context splicing into a named `b_` is unsupported.
//!
//! The third divergence — `NL`/`POP`/`HOLE` emitted as **calls** (`NL()`) where
//! Python emits bare constants — is in the generated table, not here.
//! wasm-bindgen cannot export a module-scope constant at all, and a shared
//! singleton would be consumed by its first use anyway; it is a deliberate one
//! (issue #167), and the one row of that table no parser can check, since both
//! spellings are valid TypeScript. See `quilt-wasm/src/lib.rs` and the README's
//! "Divergences from the Python runtime".

use crate::langs::chain::{Chain, Lit, TYPESCRIPT};
use crate::prelude::*;
use crate::term::CmdOrHole;

/**************************************************************/

/// Render a TypeScript double-quoted string literal, escaping `\` and `"`
/// (and the control characters a lexer would choke on).
fn str_lit(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The shared builder-call fold, spelled for TypeScript.
const CHAIN: Chain = Chain::new(&TYPESCRIPT, Lit::Flat(str_lit));

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
/// terms; each is emitted with `.e(..)`. As in Python's runtime there is no
/// named `b_`, so statement-context splicing is unsupported (see module docs).
pub fn build_variadic_block(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    CHAIN.variadic_code(tag, cmds, children)
}

/**************************************************************/

/// Make an identifier term (the `⟨N⟩` operator).
pub fn name(s: &str) -> Arc<QTerm> {
    leaf("identifier", s)
}
