//! String-style term reconstruction for the Lean meta-language.
//!
//! Unlike `langs::rust::ops` / `langs::python::ops` (which emit *builder* calls
//! into a `QTerm` runtime), the Lean host has no runtime library: it represents
//! generated code as plain **Lean strings**. Each helper builds an `Arc<QTerm>`
//! whose `.coparse()` is a Lean `String` expression that, when evaluated,
//! produces the fragment's text. See issue #132 for why.
//!
//! The mapping is direct because Lean already has string interpolation: a Quilt
//! quote `↖ … ↗` becomes an interpolated string literal `s!" … "`, and a host
//! unquote `↙x↘` becomes Lean's own `{x}`, splicing the runtime value of `x`
//! (Lean applies `ToString` at the splice, so no explicit conversion is
//! needed). Static sub-structure is flattened inline (children tagged
//! [`LEANSTR`] are spliced verbatim) so a fully literal fragment is a single
//! flat string rather than a tower of `{s!"…"}`.
//!
//! Verified against Lean 4.32.1: `s!"…"` is valid with no interpolation at all,
//! raw newlines are legal inside it, `\{` escapes a literal brace, and a bare
//! `}` needs no escape.

use crate::prelude::*;
use crate::qterm::QTermBuilder;
use crate::term::CmdOrHole;

/**************************************************************/

/// Tag marking a `QTerm` these helpers built as a Lean *string fragment*
/// (`s!"…"`). [`append_child`] flattens such a child — splicing its content
/// inline — instead of wrapping it in another `{…}` interpolation.
pub const LEANSTR: &str = "_leanstr";

/// Escape literal fragment text for a Lean interpolated string literal.
///
/// Besides `"` and `\`, the interpolation opener `{` is escaped (to `\{`) so
/// braces *in the generated code* — structure instances, implicit binders
/// `{α : Type}`, set-builders — stay literal. Quilt's own unquotes are emitted
/// as unescaped `{…}` by [`append_child`]. A closing `}` is not special to
/// Lean outside an interpolation, so it is left alone.
fn lean_str_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '"' | '{' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// Replay a fragment's `cmds`/`terms` into `b` verbatim (no re-escaping — the
/// writes were already escaped when the fragment was built). Used to splice a
/// [`LEANSTR`] child's *inner* content (its surrounding `s!"` / `"` stripped).
fn replay(b: &mut QTermBuilder, cmds: &[CmdOrHole], terms: &[Arc<QTerm>]) {
    let mut it = terms.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(s);
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.nl();
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.push(s);
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.pop();
            }
            CmdOrHole::Hole => {
                b.child(it.next().expect("replay: term underflow"));
            }
        }
    }
}

/// Append one child fragment into `b` (already mid-string). A child we built
/// (`LEANSTR`) is inlined verbatim with its wrapping `s!"` / `"` stripped; any
/// other expression is a dynamic splice — a ground unquote's value — emitted as
/// Lean interpolation `{expr}`.
fn append_child(b: &mut QTermBuilder, child: &Arc<QTerm>) {
    if let QTerm::Tuple { tag, terms, cmds } = &**child {
        if &**tag == LEANSTR {
            // Strip the wrapping `s!"` and `"` writes (first and last cmd).
            replay(b, &cmds[1..cmds.len() - 1], terms);
            return;
        }
    }
    b.write("{");
    b.child(child);
    b.write("}");
}

/// Walk a tuple's `cmds`, writing literal text (escaped) and splicing children
/// at holes, into `b` (already mid-string).
fn append_content(b: &mut QTermBuilder, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) {
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&lean_str_escape(s));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.nl();
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.push(&lean_str_escape(s));
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.pop();
            }
            CmdOrHole::Hole => {
                append_child(b, it.next().expect("build_str_code: not enough children"));
            }
        }
    }
}

/// Build the Lean string fragment for a tuple: `s!"<reconstructed text>"`, with
/// children spliced at holes. This is the core of `expand_tuple`.
pub fn build_str_code(cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    let mut b = tb(LEANSTR);
    b.write("s!\"");
    append_content(&mut b, cmds, children);
    b.write("\"");
    b.b()
}

/// Reconstruct a *nested* quote `lang↖<body>↗` as a Lean string fragment (the
/// quasi-quotation case, quote depth > 0; the outermost quote never reaches
/// here). Best-effort: the glyphs are preserved literally around the expanded
/// body.
pub fn build_quote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb(LEANSTR);
    b.write("s!\"");
    b.write(&lean_str_escape(lang));
    b.write("↖");
    append_child(&mut b, body);
    b.write("↗");
    b.write("\"");
    b.b()
}

/// Reconstruct a *nested* unquote `lang↙<body>↘` as a Lean string fragment (an
/// unquote that does not reach ground; depth > 0). Best-effort, as
/// [`build_quote_str`].
pub fn build_unquote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb(LEANSTR);
    b.write("s!\"");
    b.write(&lean_str_escape(lang));
    b.write("↙");
    append_child(&mut b, body);
    b.write("↘");
    b.write("\"");
    b.b()
}

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_braces_quotes_and_backslashes() {
        // Braces matter most: Lean code is full of them (implicit binders,
        // structure instances), and an unescaped `{` would open an
        // interpolation in the generated string.
        assert_eq!(lean_str_escape("{α : Type}"), r"\{α : Type}");
        assert_eq!(lean_str_escape(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(lean_str_escape(r"back\slash"), r"back\\slash");
        // A bare `}` is not special outside an interpolation.
        assert_eq!(lean_str_escape("}"), "}");
    }
}
