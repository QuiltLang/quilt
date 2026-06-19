//! String-style term reconstruction for the Nix meta-language.
//!
//! Unlike `langs::rust::ops` / `langs::python::ops` (which emit *builder* calls
//! into a `QTerm` runtime), the Nix host has no runtime library: it represents
//! generated code as plain **Nix strings**. Each helper builds an `Arc<QTerm>`
//! whose `.coparse()` is a Nix string expression that, when evaluated (e.g. with
//! `nix eval`), produces the fragment's text.
//!
//! The mapping is direct because Nix already has string antiquotation: a Quilt
//! quote `↖ … ↗` becomes a Nix string literal `" … "`, and a host unquote
//! `↙x↘` becomes Nix's own `${x}`, splicing the runtime value of `x`. Static
//! sub-structure is flattened inline (children tagged [`NIXSTR`] are spliced
//! verbatim) so a fully literal fragment is a single flat string rather than a
//! tower of `${"…"}`.

use crate::prelude::*;
use crate::qterm::QTermBuilder;
use crate::term::CmdOrHole;

/**************************************************************/

/// Tag marking a `QTerm` these helpers built as a Nix *string fragment*
/// (`"…"`). [`append_child`] flattens such a child — splicing its content
/// inline — instead of wrapping it in another `${…}` antiquotation.
pub const NIXSTR: &str = "_nixstr";

/// Escape literal fragment text for a Nix double-quoted string. Besides `"` and
/// `\`, the antiquotation opener `${` is escaped (to `\${`) so interpolation
/// *in the generated code* stays literal; Quilt's own unquotes are emitted as
/// unescaped `${…}` by [`append_child`].
fn nix_str_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' | '"' => {
                out.push('\\');
                out.push(c);
            }
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out
}

/// Replay a fragment's `cmds`/`terms` into `b` verbatim (no re-escaping — the
/// writes were already escaped when the fragment was built). Used to splice a
/// [`NIXSTR`] child's *inner* content (its surrounding quotes stripped).
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
/// (`NIXSTR`) is inlined verbatim with its wrapping quotes stripped; any other
/// expression is a dynamic splice — a ground unquote's value — emitted as Nix
/// antiquotation `${expr}`.
fn append_child(b: &mut QTermBuilder, child: &Arc<QTerm>) {
    if let QTerm::Tuple { tag, terms, cmds } = &**child {
        if &**tag == NIXSTR {
            // Strip the wrapping quote writes (first and last cmd).
            replay(b, &cmds[1..cmds.len() - 1], terms);
            return;
        }
    }
    b.write("${");
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
                b.write(&nix_str_escape(s));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.nl();
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.push(&nix_str_escape(s));
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

/// Build the Nix string fragment for a tuple: `"<reconstructed text>"`, with
/// children spliced at holes. This is the core of [`expand_tuple`].
///
/// [`expand_tuple`]: super::meta::NixMetaLanguage
pub fn build_str_code(cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    let mut b = tb(NIXSTR);
    b.write("\"");
    append_content(&mut b, cmds, children);
    b.write("\"");
    b.b()
}

/// Reconstruct a *nested* quote `lang↖<body>↗` as a Nix string fragment (the
/// quasi-quotation case, quote depth > 0; the outermost quote never reaches
/// here). Best-effort: the glyphs are preserved literally around the expanded
/// body.
pub fn build_quote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb(NIXSTR);
    b.write("\"");
    b.write(&nix_str_escape(lang));
    b.write("↖");
    append_child(&mut b, body);
    b.write("↗");
    b.write("\"");
    b.b()
}

/// Reconstruct a *nested* unquote `lang↙<body>↘` as a Nix string fragment (an
/// unquote that does not reach ground; depth > 0). Best-effort, as
/// [`build_quote_str`].
pub fn build_unquote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb(NIXSTR);
    b.write("\"");
    b.write(&nix_str_escape(lang));
    b.write("↙");
    append_child(&mut b, body);
    b.write("↘");
    b.write("\"");
    b.b()
}
