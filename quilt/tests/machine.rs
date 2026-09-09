//! Machines at the `Multi` rim (docs/design/machines.md): the park of live
//! per-language defaults, `feed_on` — the end-user text door, where the
//! `Language` that can parse (and so validate) lives — and `eval_on`
//! closing the loop by re-reading the answered literal into a term.
#![cfg(all(feature = "parse", feature = "python"))]

use quilt::lang::InnerKind;
use quilt::langs::omni::Omni;
use quilt::prelude::*;

/// The park's default machine persists across calls, so a definition fed by
/// one caller is visible to a later eval — the sharing today's one-shot `↓`
/// paths cannot provide. `feed_on` parses the raw text into a term before
/// the machine ever sees it.
#[test]
fn the_park_shares_definitions_between_evals() -> Result<()> {
    let mut multi = Omni::default();
    multi.feed_on("py", InnerKind::Item, "y = 40")?;
    let term = multi.parse_lang("py", "y + 2")?;
    let answer = multi.eval_on("py", &term)?;
    assert_eq!(answer.coparse(), "42");
    Ok(())
}

/// Aliases share one parked machine: a definition fed under `python` is
/// visible to an eval under `py`.
#[test]
fn park_canonicalizes_aliases() -> Result<()> {
    let mut multi = Omni::default();
    multi.feed_on("python", InnerKind::Item, "w = 6")?;
    let term = multi.parse_lang("py", "w * 7")?;
    assert_eq!(multi.eval_on("py", &term)?.coparse(), "42");
    Ok(())
}

/// `spawn_machine` hands out a fresh machine that shares nothing with the
/// park — the isolation law. Machines take terms, so the query is parsed
/// first.
#[test]
fn spawned_machines_are_isolated_from_the_park() -> Result<()> {
    let mut multi = Omni::default();
    multi.feed_on("py", InnerKind::Item, "z = 1")?;
    let query = multi.parse_lang("py", "z")?;
    let mut fresh = multi.spawn_machine("py")?;
    assert!(
        fresh.eval(&query).is_err(),
        "a fresh machine must not see the park's definitions"
    );
    Ok(())
}

/// `feed_on` validates: text that does not parse never reaches the machine.
#[test]
fn feed_on_rejects_unparseable_text() {
    let mut multi = Omni::default();
    assert!(multi.feed_on("py", InnerKind::Expr, "def oops(:").is_err());
}

/// Method-position `↓` (#268): the glyph flush against an argument list
/// spells the host's machine-eval method, completed by the source's own
/// parentheses — while the operator forms stay what they were.
#[test]
fn method_position_reduce_spells_eval() -> Result<()> {
    let mut multi = Omni::default();

    // Python host: `db.↓(schema)` → `db.eval(schema)`.
    let term = multi.parse_lang("py", "db.↓(schema)")?;
    let expanded = multi.expand_lang("py", &term)?;
    assert_eq!(expanded.coparse().trim(), "db.eval(schema)");

    // Rust host: `db.↓(&schema)` → `db.eval(&schema)`.
    let term = multi.parse_lang("rs", "db.↓(&schema);")?;
    let expanded = multi.expand_lang("rs", &term)?;
    assert_eq!(expanded.coparse().trim(), "db.eval(&schema);");

    // Not flush against parentheses: the ordinary reduce operator, as ever.
    let term = multi.parse_lang("py", "term.↓")?;
    let expanded = multi.expand_lang("py", &term)?;
    assert_eq!(expanded.coparse().trim(), "term.reduce()");
    Ok(())
}

/// A parenthesized SQL query is a *value*, and machine-eval routing rides
/// classification — so it must classify Expr, not fall through to the
/// statement/file kinds (the #191 failure mode, surfaced by `db.↓(…)`).
#[cfg(feature = "sql")]
#[test]
fn parenthesized_sql_queries_classify_expr() -> Result<()> {
    use quilt::lang::{InnerKind, Language as _};
    use quilt::multi::Languages as _;
    let mut multi = Omni::default();
    let term = multi.parse_lang("sql", "(SELECT 1 + 1)")?;
    let root = term; // parse_lang wraps in a tagless root; classify sees through nothing here,
                     // so unwrap the way the repl does: peel empty-tag wrappers.
    let mut inner: &quilt::qterm::QTerm = &root;
    while let quilt::qterm::QTerm::Tuple { tag, terms, .. } = inner {
        if tag.is_empty() && terms.len() == 1 {
            inner = &terms[0];
        } else {
            break;
        }
    }
    let kind = multi.langs.get("sql")?.classify_term(inner);
    assert_eq!(
        kind,
        InnerKind::Expr,
        "got {kind:?} for {:?}\n{}",
        inner.coparse(),
        quilt::qsnap::qsnap(inner)
    );
    Ok(())
}

/// A language with no machine spec fails with a message saying so, rather
/// than leaking a broken runner.
#[cfg(feature = "wgsl")]
#[test]
fn languages_without_a_spec_say_so() {
    let mut multi = Omni::default();
    let Err(err) = multi.machine("wgsl") else {
        panic!("wgsl unexpectedly has a machine")
    };
    assert!(err.to_string().contains("no machine"), "got: {err}");
}
