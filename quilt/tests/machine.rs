//! Machines at the `Multi` rim (docs/design/machines.md): the park of live
//! per-language defaults, `feed_on` — the end-user text door, where the
//! `Language` that can parse (and so validate) lives — and `eval_on`
//! closing the loop by re-reading the answered literal into a term.
#![cfg(all(feature = "parse", feature = "python"))]

use quilt::lang::{InnerKind, Language as _};
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

/**************************************************************/
// `⟨M⟩`, the machine glyph (issue #273).

/// The point of the bare form: a `.sql.py.quilt` file has *already* said
/// "Python ground, SQL fragments", so its machine needs no annotation. The
/// resolution is the one bare `↖…↗` uses — the chain's next default language,
/// falling back to the host — so the same glyph in a plain `.py.quilt` file is
/// a Python machine.
#[test]
fn the_bare_machine_glyph_follows_the_file_chain() -> Result<()> {
    let mut multi = Omni::default();
    let expand = |multi: &mut Omni, chain: &[&str], src: &str| -> Result<String> {
        let term = multi.parse_chain(chain, src)?;
        Ok(multi
            .expand_lang(chain[0], &term)?
            .coparse()
            .trim()
            .to_string())
    };
    assert_eq!(
        expand(&mut multi, &["py", "sql"], "db = ⟨M⟩\n")?,
        "db = spawn(\"sql\")"
    );
    assert_eq!(
        expand(&mut multi, &["py"], "db = ⟨M⟩\n")?,
        "db = spawn(\"py\")"
    );
    // An annotation says it outright, and overrides the chain.
    assert_eq!(
        expand(&mut multi, &["py", "sql"], "sh = bash⟨M⟩\n")?,
        "sh = spawn(\"bash\")"
    );
    Ok(())
}

/// `⟨T⟩` flush against an argument list is the machine's *typing* judgment,
/// the companion of `db.↓(t)`'s value judgment. In operand position it keeps
/// meaning the term type — the same position-overload `↓` already carries.
#[test]
fn the_type_glyph_is_a_method_only_when_flush() -> Result<()> {
    let mut multi = Omni::default();
    let expand = |multi: &mut Omni, src: &str| -> Result<String> {
        let term = multi.parse_chain(&["py", "sql"], src)?;
        Ok(multi.expand_lang("py", &term)?.coparse().trim().to_string())
    };
    assert_eq!(expand(&mut multi, "x = db.⟨T⟩(t)\n")?, "x = db.type_of(t)");
    // A space is the whole difference, exactly as for `↓`.
    assert_eq!(expand(&mut multi, "t: ⟨T⟩ = None\n")?, "t: QTerm = None");
    Ok(())
}

/// A machine no provider backs is refused at *expansion* time, pointing at
/// the glyph, rather than expanding into generated code that fails later.
#[cfg(feature = "wgsl")]
#[test]
fn a_machine_glyph_for_an_unbacked_language_is_refused() {
    let mut multi = Omni::default();
    // The spellings are substituted while the term is *built*, so this is a
    // parse-time refusal — which is what puts the source span on it.
    let Err(err) = multi.parse_chain(&["py"], "m = wgsl⟨M⟩\n") else {
        panic!("wgsl unexpectedly has a machine")
    };
    let err = err.to_string();
    assert!(err.contains("no machine is registered"), "got: {err}");
    // …and it names what quilt *does* know, so the fix is in the message.
    assert!(err.contains("sql"), "got: {err}");
}

/// Like `↑ ↓ ←`, a `⟨M⟩` at sky depth > 0 belongs to the stage that runs the
/// generated code: it survives expansion as its own glyph, annotation and
/// all, instead of being spelled out one stage too early. The *unresolved*
/// annotation goes back in, so the next stage resolves the bare form against
/// its own chain.
#[test]
fn a_machine_glyph_inside_a_quote_is_deferred() -> Result<()> {
    let mut multi = Omni::default();
    let src = "stage2 = ↖db = sql⟨M⟩↗\n";
    let term = multi.parse_chain(&["py", "py"], src)?;
    let expanded = multi.expand_lang("py", &term)?;
    let code = expanded.coparse();
    assert!(code.contains("sql⟨M⟩"), "deferred glyph is gone: {code}");
    Ok(())
}

/// The spec table is the one description of how a language runs: the
/// `Language` methods delegate into it, so the registry, the conformance
/// battery and `qspawn` cannot disagree. Checked rather than asserted in a
/// doc comment, because a hand-written spec that drifts is exactly the bug
/// moving the data was meant to remove.
#[test]
fn the_registry_and_the_runtime_table_agree() {
    use quilt::multi::Languages as _;
    let multi = Omni::default();
    for lang in quilt::machine::REGISTERED_SPECS {
        let Ok(registered) = multi.langs.get(lang) else {
            continue; // language feature off in this build
        };
        // Compared through `Debug` so *every* field is in the comparison —
        // a drifting `print_wrap` is as wrong as a drifting program name.
        assert_eq!(
            format!("{:?}", registered.repl_spec()),
            format!("{:?}", quilt::machine::repl_spec(lang)),
            "{lang}: repl spec differs between the registry and the table"
        );
        assert_eq!(
            format!("{:?}", registered.machine_spec()),
            format!("{:?}", quilt::machine::script_spec(lang)),
            "{lang}: script spec differs between the registry and the table"
        );
        // And the shebang the script spec is derived from is the one the
        // language declares for `quilt run`.
        if let (Some(spec), Some(hashbang)) = (
            quilt::machine::script_spec(lang),
            registered.hashbang().and_then(quilt::lang::parse_hashbang),
        ) {
            assert_eq!(&*spec.program, hashbang.0, "{lang}: shebang drift");
        }
    }
}
