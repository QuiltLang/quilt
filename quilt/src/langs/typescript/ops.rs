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
use crate::meta::Piece;
use crate::prelude::*;
use crate::strcmd::write;
use crate::term::{cmd, CmdOrHole, HOLE};

/**************************************************************/

/// Render a TypeScript double-quoted string literal, escaping `\` and `"`
/// (and the control characters a lexer would choke on). The table is
/// [`crate::lift::ts_dquote_escape`], shared with `LiftTo<TypeScript>`, so
/// generated code and lifted values agree.
fn str_lit(s: &str) -> String {
    format!("\"{}\"", crate::lift::ts_dquote_escape(s))
}

/// The shared builder-call fold, spelled for TypeScript.
///
/// The lifts are named as fresh: `qlift`/`qlift_html`/`qlift_text…` return a
/// new term on every call, so `↙↑(x)↘` can hand its result to the builder
/// outright.
const CHAIN: Chain = Chain::new(&TYPESCRIPT, Lit::Flat(str_lit)).fresh_calls(&[
    "qlift",
    "qlift_html",
    "qlift_text",
    "qlift_text_in_quotes",
]);

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

/**************************************************************/
// `⟨str⟩` and `⟨qt⟩` quotes (see `crate::qterm::Repr`).

/// Escape `s` for the inside of a template literal: `\`, `` ` `` and `${`,
/// and a newline as `\n` so the literal stays on one line of the output.
fn template_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push_str("\\\\"),
            '`' => out.push_str("\\`"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// A `⟨str⟩` quote as one template literal: its text inline, each hole as
/// `${…}`, each layout newline as `\n` followed by the prefixes pushed at
/// that point. A hole under a prefix is re-indented the way hand-written
/// string codegen does it, `String(x).replaceAll("\n", "\n<prefix>")`; a hole
/// with no prefix is interpolated as is.
pub fn build_str_code(pieces: &[Piece]) -> Arc<QTerm> {
    let mut cmds = vec![cmd(write("`"))];
    let mut children = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => cmds.push(cmd(write(&template_escape(t)))),
            Piece::NewLine => {
                cmds.push(cmd(write(&format!(
                    "\\n{}",
                    template_escape(&stack.concat())
                ))));
            }
            Piece::Push(s) => stack.push(s),
            Piece::Pop => {
                stack.pop();
            }
            Piece::Hole(e) => {
                let prefix = stack.concat();
                if prefix.is_empty() {
                    cmds.extend([cmd(write("${")), HOLE, cmd(write("}"))]);
                } else {
                    cmds.extend([
                        cmd(write("${String(")),
                        HOLE,
                        cmd(write(&format!(
                            ").replaceAll(\"\\n\", {})}}",
                            str_lit(&format!("\n{prefix}"))
                        ))),
                    ]);
                }
                children.push(e.clone());
            }
        }
    }
    cmds.push(cmd(write("`")));
    tuple("template_string", &children, &cmds)
}

/// A `⟨qt⟩` quote as one flat builder chain: `tb("").w(…).n().p(…).c(x)…b()`,
/// a write per run of text rather than a node per token.
pub fn build_qt_code(pieces: &[Piece]) -> Arc<QTerm> {
    let mut cmds = vec![cmd(write("tb(\"\")"))];
    let mut children = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Text(t) => cmds.push(cmd(write(&format!(".w({})", str_lit(t))))),
            Piece::NewLine => cmds.push(cmd(write(".n()"))),
            Piece::Push(s) => cmds.push(cmd(write(&format!(".p({})", str_lit(s))))),
            Piece::Pop => cmds.push(cmd(write(".x()"))),
            Piece::Hole(e) => {
                cmds.extend([cmd(write(".c(")), HOLE, cmd(write(")"))]);
                children.push(e.clone());
            }
        }
    }
    cmds.push(cmd(write(".b()")));
    tuple("call_expression", &children, &cmds)
}
