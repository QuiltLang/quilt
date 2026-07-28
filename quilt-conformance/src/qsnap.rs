//! Canonical structural rendering of a `QTerm`.
//!
//! `coparse()` answers "what text does this produce", which is the *output* of
//! the IR, not the IR. Two structurally different terms — a flat `leaf` and a
//! nested tuple whose children happen to concatenate the same way — coparse
//! identically, so a suite that only compares coparsed text cannot see a tree
//! being reshaped underneath it. That matters most exactly when we are doing
//! the cross-cutting refactors this harness exists to protect.
//!
//! `qsnap` renders the tree, the tags, and the `cmds` interleaving in a form
//! that is stable, diffable, and readable in a failure message:
//!
//! ```text
//! Tuple binary_expression ⟨·, " ", ·, " ", ·⟩
//! ├─ Tuple integer_literal ⟨"1"⟩
//! ├─ Tuple identifier ⟨"+"⟩
//! ╰─ Tuple integer_literal ⟨"2"⟩
//! ```
//!
//! Cmd glyphs: `·` hole, `"…"` write, `⏎` newline, `»"…"` push prefix, `«` pop.
//! Spans are deliberately omitted — they are diagnostic metadata excluded from
//! `PartialEq`, so including them would make snapshots churn on unrelated edits.

use quilt::{
    prelude::*,
    strcmd::StrCmd,
    term::{CmdOrHole, Term as _},
};
use std::fmt::Write as _;

/// Render `term` as a structural snapshot.
pub fn qsnap(term: &QTerm) -> String {
    let mut out = String::new();
    render(term, &mut out, "", true, true);
    out
}

/// Render just the head line of a term — its variant, tag and cmds — without
/// children. Useful in assertion messages where the whole tree is too much.
pub fn qhead(term: &QTerm) -> String {
    let mut out = String::new();
    head(term, &mut out);
    out
}

fn head(term: &QTerm, out: &mut String) {
    match term {
        QTerm::Tuple { tag, cmds, .. } => {
            let _ = write!(out, "Tuple {tag} {}", cmds_str(cmds));
        }
        QTerm::Quote {
            tag,
            index,
            lang,
            cmds,
            ..
        } => {
            let _ = write!(out, "Quote[{index}] {lang}:{tag} {}", cmds_str(cmds));
        }
        QTerm::Unquote {
            tag,
            index,
            lang,
            cmds,
            ..
        } => {
            let _ = write!(out, "Unquote[{index}] {lang}:{tag} {}", cmds_str(cmds));
        }
    }
}

fn render(term: &QTerm, out: &mut String, prefix: &str, last: bool, root: bool) {
    if !root {
        out.push_str(prefix);
        out.push_str(if last { "╰─ " } else { "├─ " });
    }
    head(term, out);
    out.push('\n');

    let child_prefix = if root {
        String::new()
    } else {
        format!("{prefix}{}", if last { "   " } else { "│  " })
    };

    let children: Vec<&QTerm> = term.children().collect();
    let n = children.len();
    for (i, child) in children.into_iter().enumerate() {
        render(child, out, &child_prefix, i + 1 == n, false);
    }
}

fn cmds_str(cmds: &[CmdOrHole]) -> String {
    let mut s = String::from("⟨");
    for (i, c) in cmds.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        match c {
            CmdOrHole::Hole => s.push('·'),
            CmdOrHole::Cmd(StrCmd::Write(w)) => {
                let _ = write!(s, "{:?}", w.as_ref());
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => s.push('⏎'),
            CmdOrHole::Cmd(StrCmd::Push(p)) => {
                let _ = write!(s, "»{:?}", p.as_ref());
            }
            CmdOrHole::Cmd(StrCmd::Pop) => s.push('«'),
        }
    }
    s.push('⟩');
    s
}

/// Structural invariants every parsed term must satisfy, independent of
/// language. Returns the list of violations (empty means the term is sound).
///
/// These are the properties that hold by construction *if* a `Language` impl is
/// well-behaved, which is precisely why they are worth asserting for a new
/// language whose impl is not yet trusted.
pub fn structural_violations(term: &QTerm) -> Vec<String> {
    let mut bad = Vec::new();
    check(term, &mut bad, "root");
    bad
}

fn check(term: &QTerm, bad: &mut Vec<String>, path: &str) {
    let cmds = match term {
        QTerm::Tuple { cmds, .. } | QTerm::Quote { cmds, .. } | QTerm::Unquote { cmds, .. } => cmds,
    };
    let holes = cmds.iter().filter(|c| matches!(c, CmdOrHole::Hole)).count();
    let children: Vec<&QTerm> = term.children().collect();

    // Every child must have exactly one hole to be written into, or it can
    // never appear in the output — a silent truncation at serialization time.
    if holes != children.len() {
        bad.push(format!(
            "{path}: {} child(ren) but {holes} hole(s) in cmds — {}",
            children.len(),
            qhead(term),
        ));
    }

    for (i, child) in children.into_iter().enumerate() {
        check(child, bad, &format!("{path}.{i}"));
    }
}
