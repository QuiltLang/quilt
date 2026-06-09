//! Direct term-lifting for the Python meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. The emitted source is **Python**, targeting a
//! `quilt` Python runtime whose builder mirrors the Rust `QTermBuilder` fluent
//! API (`tb`/`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`, `quote`/`unquote`, `leaf`/`sym`,
//! `cmd`/`write`/`push`/`NL`/`POP`/`HOLE`).
//!
//! It differs from `langs::rust::ops` in three Python-specific ways:
//! * `.c(child)` / list literals carry no Rust `&` borrow.
//! * cmd sequences are Python lists `[..]`, not `&[..]`.
//! * a variadic node is a fluent `.e()` emit chain rather than Rust's imperative
//!   `{ let mut b_ = ..; ..; b_.b() }` block, which has no Python
//!   expression-context equivalent. (A consequence: statement-context splicing
//!   — control flow inside `↙..↘` that emits into a named `b_` — is not
//!   expressible in Python and is unsupported.)

use crate::prelude::*;
use crate::term::CmdOrHole;

/**************************************************************/

/// Render a Python string literal, escaping `"` (mirrors `rust::ops::str_lit`).
fn str_lit(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Render a `StrCmd` as constructor source.
fn strcmd_lit(c: &StrCmd) -> String {
    match c {
        StrCmd::Write(s) => format!("write({})", str_lit(s)),
        StrCmd::NewLine => "NL".to_string(),
        StrCmd::Push(s) => format!("push({})", str_lit(s)),
        StrCmd::Pop => "POP".to_string(),
    }
}

/// Render a `&[CmdOrHole]` as a Python `[..]` list literal.
fn cmds_lit(cmds: &[CmdOrHole]) -> String {
    let items: Vec<String> = cmds
        .iter()
        .map(|c| match c {
            CmdOrHole::Hole => "HOLE".to_string(),
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
/// terms; each is emitted with `.e(..)` (which, like Rust's `.emit(&mut b_)`,
/// appends one-or-more children). Unlike Rust's variadic, there is no named
/// `b_`, so statement-context splicing is unsupported (see module docs).
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
