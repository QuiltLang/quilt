//! String-style term reconstruction for the shell meta-languages (issue #151).
//!
//! Unlike `langs::rust::ops` / `langs::python::ops` (which emit *builder* calls
//! into a `QTerm` runtime), a shell host has no runtime library: it represents
//! generated code as plain **double-quoted shell words**. Each helper builds an
//! `Arc<QTerm>` whose `.coparse()` is a shell word that, when the script runs,
//! expands to the fragment's text.
//!
//! The mapping is direct because the shell already has interpolation: a Quilt
//! quote `↖ … ↗` becomes `" … "`, and a host unquote `↙x↘` is spliced
//! **verbatim** into that string. Verbatim is the whole difference from
//! [`langs::nix::ops`], which wraps its splices in `${…}`: a Nix expression
//! carries no sigil of its own and must be marked, whereas every shell
//! expression that produces a value already carries one — `$name`,
//! `${arr[0]}`, `$(cmd)`, `$((1 + 2))` — and each of those interpolates as
//! written inside `"…"`, without word-splitting. Wrapping would be actively
//! wrong: `${$(cmd)}` is a syntax error.
//!
//! Static sub-structure is flattened inline (children tagged [`SHSTR`] are
//! spliced verbatim) so a fully literal fragment is a single flat string rather
//! than a tower of concatenations.
//!
//! [`langs::nix::ops`]: crate::langs::nix::ops

use crate::lift::sh_dquote_escape;
use crate::prelude::*;
use crate::qterm::QTermBuilder;
use crate::term::CmdOrHole;

/**************************************************************/

/// Tag marking a `QTerm` these helpers built as a shell *string fragment*
/// (`"…"`). [`append_child`] flattens such a child — splicing its content
/// inline — instead of concatenating another quoted word onto it.
pub const SHSTR: &str = "_shstr";

/// Replay a fragment's `cmds`/`terms` into `b` verbatim (no re-escaping — the
/// writes were already escaped when the fragment was built). Used to splice a
/// [`SHSTR`] child's *inner* content (its surrounding quotes stripped).
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
/// (`SHSTR`) is inlined verbatim with its wrapping quotes stripped; any other
/// expression is a dynamic splice — a ground unquote's value — written as-is,
/// because a shell expansion already carries its own `$`.
fn append_child(b: &mut QTermBuilder, child: &Arc<QTerm>) {
    if let QTerm::Tuple { tag, terms, cmds } = &**child {
        if &**tag == SHSTR {
            // Strip the wrapping quote writes (first and last cmd).
            replay(b, &cmds[1..cmds.len() - 1], terms);
            return;
        }
    }
    b.child(child);
}

/// Walk a tuple's `cmds`, writing literal text (escaped) and splicing children
/// at holes, into `b` (already mid-string).
fn append_content(b: &mut QTermBuilder, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) {
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&sh_dquote_escape(s));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.nl();
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.push(&sh_dquote_escape(s));
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

/// Build the shell string fragment for a tuple: `"<reconstructed text>"`, with
/// children spliced at holes. This is the core of [`expand_tuple`].
///
/// [`expand_tuple`]: super::meta::ShellMetaLanguage
pub fn build_str_code(cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    let mut b = tb(SHSTR);
    b.write("\"");
    append_content(&mut b, cmds, children);
    b.write("\"");
    b.b()
}

/// Reconstruct a *nested* quote `lang↖<body>↗` as a shell string fragment (the
/// quasi-quotation case, quote depth > 0; the outermost quote never reaches
/// here). Best-effort: the glyphs are preserved literally around the expanded
/// body.
pub fn build_quote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    build_glyph_str(lang, '↖', body, '↗')
}

/// Reconstruct a *nested* unquote `lang↙<body>↘` as a shell string fragment (an
/// unquote that does not reach ground; depth > 0). Best-effort, as
/// [`build_quote_str`].
pub fn build_unquote_str(lang: &str, body: &Arc<QTerm>) -> Arc<QTerm> {
    build_glyph_str(lang, '↙', body, '↘')
}

fn build_glyph_str(lang: &str, open: char, body: &Arc<QTerm>, close: char) -> Arc<QTerm> {
    let mut b = tb(SHSTR);
    b.write("\"");
    b.write(&sh_dquote_escape(lang));
    b.write(&open.to_string());
    append_child(&mut b, body);
    b.write(&close.to_string());
    b.write("\"");
    b.b()
}
