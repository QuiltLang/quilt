//! Direct term-lifting for the TypeScript meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. The emitted source is **TypeScript**,
//! targeting the `quilt-wasm` runtime whose builder mirrors the Rust
//! `QTermBuilder` fluent API (`tb`/`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`,
//! `quote`/`unquote`, `leaf`/`sym`, `cmd`/`write`/`push`/`NL`/`POP`/`HOLE`).
//!
//! It is a near-clone of `langs::python::ops` — TypeScript and Python share the
//! method-chain shape and `[..]` array literals — differing only in:
//! * `NL`/`POP`/`HOLE` are emitted as **calls** (`NL()`, `POP()`, `HOLE()`):
//!   wasm-bindgen can't export struct-valued constants, so the runtime exposes
//!   them as functions (see `quilt-wasm/src/lib.rs`).
//! * string literals escape `\` as well as `"`.
//! * a variadic node is a fluent `.e()` emit chain (as in Python), so
//!   statement-context splicing into a named `b_` is unsupported.

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

/// Render a `StrCmd` as constructor source. `NL`/`POP` are runtime functions.
fn strcmd_lit(c: &StrCmd) -> String {
    match c {
        StrCmd::Write(s) => format!("write({})", str_lit(s)),
        StrCmd::NewLine => "NL()".to_string(),
        StrCmd::Push(s) => format!("push({})", str_lit(s)),
        StrCmd::Pop => "POP()".to_string(),
    }
}

/// Render a `&[CmdOrHole]` as a TypeScript `[..]` array literal. A hole is the
/// runtime's `HOLE()` function.
fn cmds_lit(cmds: &[CmdOrHole]) -> String {
    let items: Vec<String> = cmds
        .iter()
        .map(|c| match c {
            CmdOrHole::Hole => "HOLE()".to_string(),
            CmdOrHole::Cmd(cmd) => format!("cmd({})", strcmd_lit(cmd)),
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/**************************************************************/

/// Build code that reconstructs a tuple: `tb(tag).w(..).c(child)..b()`, using
/// the `sym`/`leaf` shorthands when possible. `children` are the already-built
/// child expressions spliced at hole positions.
pub fn build_tuple_code(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    // shorthands: a childless node with a single write
    if children.is_empty() && cmds.len() == 1 {
        if let CmdOrHole::Cmd(StrCmd::Write(code)) = &cmds[0] {
            return if tag == &**code {
                leaf("_", &format!("sym({})", str_lit(tag)))
            } else {
                leaf("_", &format!("leaf({}, {})", str_lit(tag), str_lit(code)))
            };
        }
    }
    // full builder chain
    let mut b = tb("_");
    b.write(&format!("tb({})", str_lit(tag)));
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&format!(".w({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.write(".n()");
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.write(&format!(".p({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.write(".x()");
            }
            CmdOrHole::Hole => {
                b.write(".c(");
                b.child(it.next().expect("build_tuple_code: not enough children"));
                b.write(")");
            }
        }
    }
    b.write(".b()");
    b.b()
}

/// Build `quote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
pub fn build_quote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write(&format!(
        "quote({}, {}, {}, ",
        str_lit(tag),
        index,
        str_lit(lang)
    ));
    b.child(term);
    b.write(&format!(", {})", cmds_lit(cmds)));
    b.b()
}

/// Build `unquote(tag, index, lang, <term>, [..cmds..])`, splicing `term`.
pub fn build_unquote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write(&format!(
        "unquote({}, {}, {}, ",
        str_lit(tag),
        index,
        str_lit(lang)
    ));
    b.child(term);
    b.write(&format!(", {})", cmds_lit(cmds)));
    b.b()
}

/// Build a variadic node as a fluent emit chain:
/// `tb(tag).e(child).p("..").n()..b()`. `children` are the already-expanded
/// terms; each is emitted with `.e(..)`. As in Python's runtime there is no
/// named `b_`, so statement-context splicing is unsupported (see module docs).
pub fn build_variadic_block(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write(&format!("tb({})", str_lit(tag)));
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&format!(".w({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.write(".n()");
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.write(&format!(".p({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.write(".x()");
            }
            CmdOrHole::Hole => {
                b.write(".e(");
                b.child(
                    it.next()
                        .expect("build_variadic_block: not enough children"),
                );
                b.write(")");
            }
        }
    }
    b.write(".b()");
    b.b()
}

/**************************************************************/

/// Make an identifier term (the `⟨N⟩` operator).
pub fn name(s: &str) -> Arc<QTerm> {
    leaf("identifier", s)
}
