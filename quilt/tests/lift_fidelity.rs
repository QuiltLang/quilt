//! A lifted value must be the term the target's parser produces for the same
//! text — not merely something that coparses to it.
//!
//! `↑` exists so a host value can be spliced into an object-language fragment.
//! If the term it builds has a different *shape* from the one the parser would
//! have built, the two are indistinguishable by `coparse` but not by `smatch` /
//! `sinstantiate` / [`QTerm::rewrite`], which compare tree structure. So a
//! lifted `[1, 2]` that spells its brackets as literal text cannot be matched or
//! rewritten the way a parsed one can.
//!
//! Nothing checked this. `quilt-conformance`'s `probe_lift_into` reparses each
//! lifted literal — the hard part — but compares only the resulting *text* and
//! *root tag*, so every shape difference below slipped through it. See issue
//! #174 (findings A1–A3) for the survey.
//!
//! ## The wrapper problem, and why this compares a subtree
//!
//! Reparsing a bare literal does not always yield the literal alone: a shell
//! parses `"s"` at top level as `(command (command_name (string …)))`, because a
//! bare word in statement position is a command. Those wrappers are an artifact
//! of parsing the value *standalone*, not part of the value — spliced into an
//! argument position it is just the `string`.
//!
//! So each case is checked by finding the parse's first subtree whose tag
//! matches the lifted term's root tag and comparing structurally against that.
//! That still pins the whole shape of the lifted node (tag, children, and
//! layout, since `QTerm`'s `PartialEq` compares `cmds`) without asserting
//! anything about how the target chooses to wrap a lone literal.
//!
//! ## Not covered here
//!
//! Two families still differ from their parse, because fixing them moves a tag
//! declared in `conformance/spec/*.toml` and so needs a decision — see the PR
//! and #174:
//!
//! * WGSL literals parse wrapped in `const_literal`; the lifts produce the inner
//!   `int_literal` / `float_literal` / `bool_literal`.
//! * A negative Python or Nix integer parses as a `unary_operator` /
//!   `unary_expression` over a positive literal; the lifts produce a single
//!   signed literal.
//!
//! Strings containing escapes are also exempt: the parser splits them into
//! `escape_sequence` children, which no runtime reproduces.

use quilt::lang::{flat_nodes, Language};
use quilt::langs::{
    bash::lang::BashLanguage, lean::lang::LeanLanguage, nix::lang::NixLanguage,
    python::lang::PythonLanguage, rust::lang::RustLanguage, zsh::lang::ZshLanguage,
};
use quilt::lift::{Bash, Lean, Nix, Python, QLiftTo as _, Zsh};
use quilt::prelude::*;

/// The first subtree tagged `tag`, breadth-first (so an outer wrapper of the
/// same kind wins over a nested one).
fn find_tag<'a>(term: &'a QTerm, tag: &str) -> Option<&'a QTerm> {
    let mut queue = std::collections::VecDeque::from([term]);
    while let Some(t) = queue.pop_front() {
        match t {
            QTerm::Tuple {
                tag: t_tag, terms, ..
            } => {
                if &**t_tag == tag {
                    return Some(t);
                }
                queue.extend(terms.iter().map(AsRef::as_ref));
            }
            QTerm::Quote { term, .. } | QTerm::Unquote { term, .. } => queue.push_back(term),
        }
    }
    None
}

/// The root tag of a term.
fn root_tag(term: &QTerm) -> &str {
    match term {
        QTerm::Tuple { tag, .. } | QTerm::Quote { tag, .. } | QTerm::Unquote { tag, .. } => tag,
    }
}

/// Assert every lifted term equals the parser's node of the same kind.
fn check(lang: &mut dyn Language<Post = quilt::treesitter::TSLanguagePost>, cases: &[Arc<QTerm>]) {
    for lifted in cases {
        let text = lifted.coparse();
        let tag = root_tag(lifted);
        let parsed = lang
            .parse_as(None, &flat_nodes(&text))
            .unwrap_or_else(|e| panic!("lifted {text:?} does not parse: {e}"));
        let found = find_tag(&parsed, tag).unwrap_or_else(|| {
            panic!(
                "lifted {text:?} has root tag {tag:?}, absent from the parse {}",
                parsed.sexp()
            )
        });
        assert_eq!(
            found,
            &**lifted,
            "lift of {text:?} is {} but the parser builds {}",
            lifted.sexp(),
            found.sexp(),
        );
    }
}

#[test]
fn python_lifts_match_the_parser() {
    let empty: Vec<u8> = Vec::new();
    check(
        &mut PythonLanguage::default(),
        &[
            3u32.qlift_to::<Python>(),
            1.5f64.qlift_to::<Python>(),
            true.qlift_to::<Python>(),
            false.qlift_to::<Python>(),
            "".qlift_to::<Python>(),
            "hi there".qlift_to::<Python>(),
            vec![1u8, 4].qlift_to::<Python>(),
            vec![vec![1u8], vec![2, 3]].qlift_to::<Python>(),
            empty.qlift_to::<Python>(),
        ],
    );
}

#[test]
fn nix_lifts_match_the_parser() {
    let empty: Vec<u8> = Vec::new();
    check(
        &mut NixLanguage::default(),
        &[
            42u64.qlift_to::<Nix>(),
            1.5f64.qlift_to::<Nix>(),
            true.qlift_to::<Nix>(),
            false.qlift_to::<Nix>(),
            "".qlift_to::<Nix>(),
            "/etc/nixos".qlift_to::<Nix>(),
            vec![1u8, 4].qlift_to::<Nix>(),
            empty.qlift_to::<Nix>(),
        ],
    );
}

#[test]
fn lean_lifts_match_the_parser() {
    let empty: Vec<u8> = Vec::new();
    check(
        &mut LeanLanguage::default(),
        &[
            3u32.qlift_to::<Lean>(),
            (-2i32).qlift_to::<Lean>(),
            1.5f64.qlift_to::<Lean>(),
            (-0.5f64).qlift_to::<Lean>(),
            "".qlift_to::<Lean>(),
            "Nat.succ".qlift_to::<Lean>(),
            vec![1u8, 2].qlift_to::<Lean>(),
            empty.qlift_to::<Lean>(),
        ],
    );
}

/// The shell strings below are the arithmetic-opener cases from issue #212.
///
/// `sh_dquote_escape` escapes `"`, `\`, `$` and backtick and nothing else,
/// which is right for a real shell — `(` is ordinary text inside `"…"`. The zsh
/// grammar disagreed: it offered the bare `((…))` arithmetic *command* opener as
/// an alternative inside `string`, so `((` was in the lexer's valid-token set at
/// every position in a string and won the same-length tie against
/// `string_content` (`prec(-1)`). Every lifted string containing `((` therefore
/// produced zsh that our own parser rejected — silently, at lift time. Fixed in
/// the fork by restricting `string` to the `$`-sigil forms.
///
/// Both shells get the cases: bash was never affected, which is what made the
/// divergence a grammar bug rather than an escaping one, and pinning it here
/// keeps that true.
#[test]
fn shell_lifts_match_the_parser() {
    check(
        &mut BashLanguage::default(),
        &[
            42u32.qlift_to::<Bash>(),
            "".qlift_to::<Bash>(),
            "/var/log".qlift_to::<Bash>(),
            "((".qlift_to::<Bash>(),
            "(())".qlift_to::<Bash>(),
            "x = ((a+b))".qlift_to::<Bash>(),
        ],
    );
    check(
        &mut ZshLanguage::default(),
        &[
            42u32.qlift_to::<Zsh>(),
            "".qlift_to::<Zsh>(),
            "/var/log".qlift_to::<Zsh>(),
            "((".qlift_to::<Zsh>(),
            "(())".qlift_to::<Zsh>(),
            "x = ((a+b))".qlift_to::<Zsh>(),
        ],
    );
}

/// Rust's homogeneous `↑` (`QLift`), which is what a `.rs.quilt` metaprogram
/// splices into a Rust quote.
#[test]
fn rust_lifts_match_the_parser() {
    check(
        &mut RustLanguage::default(),
        &[
            7u32.qlift(),
            1.5f64.qlift(),
            (-1.5f64).qlift(),
            true.qlift(),
            "".qlift(),
            "ab".qlift(),
        ],
    );
}
