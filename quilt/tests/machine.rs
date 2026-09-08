//! Machines at the `Multi` rim (docs/design/machines.md, phase 1): the park
//! of live per-language defaults, and `eval_on` closing the text→term loop —
//! machines speak text, and the `Language` re-reads the answered literal
//! back into a term.
#![cfg(all(feature = "parse", feature = "python"))]

use quilt::lang::InnerKind;
use quilt::langs::omni::Omni;
use quilt::prelude::*;

/// The park's default machine persists across calls, so a definition fed by
/// one caller is visible to a later eval — the sharing today's one-shot `↓`
/// paths cannot provide.
#[test]
fn the_park_shares_definitions_between_evals() -> Result<()> {
    let mut multi = Omni::default();
    multi.machine("py")?.feed_str(InnerKind::Item, "y = 40")?;
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
    multi
        .machine("python")?
        .feed_str(InnerKind::Item, "w = 6")?;
    let term = multi.parse_lang("py", "w * 7")?;
    assert_eq!(multi.eval_on("py", &term)?.coparse(), "42");
    Ok(())
}

/// `spawn_machine` hands out a fresh machine that shares nothing with the
/// park — the isolation law.
#[test]
fn spawned_machines_are_isolated_from_the_park() -> Result<()> {
    let mut multi = Omni::default();
    multi.machine("py")?.feed_str(InnerKind::Item, "z = 1")?;
    let mut fresh = multi.spawn_machine("py")?;
    assert!(
        fresh.feed_str(InnerKind::Expr, "z").is_err(),
        "a fresh machine must not see the park's definitions"
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
