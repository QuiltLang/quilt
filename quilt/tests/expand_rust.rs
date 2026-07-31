//! The production `RustMetaLanguage` (via `Omni`) must expand to exactly the same
//! code as the reference `BootstrapMetaLanguage` (via `Bootstrap`). These tests
//! expand the same inputs through both engines and assert identical output, so we
//! get a fast feedback loop without invoking `rust-script`.
//!
//! Two oracles, doing different jobs (#157):
//!
//! * **The differential check** in `expand_both` — Omni against Bootstrap — is a
//!   real assertion. It validates *semantics* while being indifferent to
//!   spelling, so it keeps working across a refactor that invalidates every
//!   snapshot below.
//! * **The snapshots** pin the exact generated code. They are what makes a
//!   deliberate change to builder spelling a bulk `cargo insta review` instead
//!   of N hand-edited string literals — and they gave the four tests that
//!   previously only `println!`ed their output a real assertion for free.
//!
//! Semantic results (a reduced value, a `qmatch` binding) and negative
//! invariants ("this must *not* emit") stay as ordinary assertions: they state
//! something that must be true, not something that merely is true today.

use indoc::indoc;
use miette::ensure;
use quilt::lang::Arity;
use quilt::langs::bootstrap::Bootstrap;
use quilt::langs::omni::Omni;
use quilt::langs::rust::lang::DynRustLanguage;
use quilt::langs::rust::meta::RustMetaLanguage;
use quilt::meta::MetaLanguage;
use quilt::multi::DictMulti;
use quilt::prelude::*;
use quilt::term::{CmdOrHole, STerm};
use std::ops::Range;

/// Expand `code` with both engines, assert identical output, and return it.
fn expand_both(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let oq = omni.parse(code)?;
    let omni_out = omni.expand(&oq)?.coparse();

    let mut bootstrap = Bootstrap::default();
    let bq = bootstrap.parse(code)?;
    let boot_out = bootstrap.expand(&bq)?.coparse();

    assert_eq!(
        omni_out, boot_out,
        "RustMetaLanguage output differs from BootstrapMetaLanguage"
    );
    Ok(omni_out)
}

#[test]
fn simple() -> Result<()> {
    insta::assert_snapshot!(expand_both("let expr = ↖1 + 2↗;")?);
    Ok(())
}

#[test]
fn quote_expr() -> Result<()> {
    insta::assert_snapshot!(expand_both("↖1 + 2↗")?);
    Ok(())
}

#[test]
fn variadic() -> Result<()> {
    let out = expand_both(indoc! {r#"
        ↖fn foo() {
            println!("Hello");
            println!("World");
        }↗
    "#})?;
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn splicing() -> Result<()> {
    let out = expand_both(indoc! {r#"{
        fn mk(i: usize) -> Result<⟨T⟩> {
            Ok(↖{
                ↙{
                    for c in 0..i {
                        if c != 0 {NL.←;}
                        ↖println!("hi");↗.←;
                    }
                }↘
            }↗)
        }
        mk(3).unwrap()
    }"#})?;
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn ground_stmt_quote_emits() -> Result<()> {
    // A quoted statement in statement position of ground code would previously
    // build a term and silently drop it; it is now emitted into the enclosing
    // builder, same as writing `.←;` explicitly.
    let out = expand_both(indoc! {r#"
        let p = ↖{
            ↙{
                ↖println!("hi");↗
            }↘
        }↗;
    "#})?;
    assert!(out.contains(".b().emit(&mut b_);"), "{out}");
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn ground_tail_quote_stays_value() -> Result<()> {
    // A tail-expression quote parses with the same outer tag as a
    // statement-position one (`expression_statement`), but its body is an
    // expression: it must remain the block's value, not get emitted.
    let out = expand_both("fn f() -> Arc<QTerm> {\n    ↖1 + 2↗\n}")?;
    assert!(!out.contains(".emit("), "{out}");
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn ground_unit_unquote_spliced() -> Result<()> {
    // An unquote whose ground body is a statement-shaped block (imperative
    // emits, unit value) is spliced as a plain statement instead of the
    // `{...}.emit(&mut b_);` unit-emit workaround.
    let out = expand_both(indoc! {r#"
        let p = ↖{
            ↙{
                for i in 0..3 {
                    ↖println!("hi");↗.←;
                }
            }↘
        }↗;
    "#})?;
    assert!(!out.contains("}.emit(&mut b_);"), "{out}");
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn pattern_let() -> Result<()> {
    // A quote in the binding position of a `let` is a pattern (issue #18):
    // its ground unquotes become metavariables and the statement destructures
    // the value by matching its shape.
    insta::assert_snapshot!(expand_both("let ↖1 + ↙x↘↗ = rhs;")?);
    Ok(())
}

/// `let mut ↖p↗ = v;` is a pattern-let too.
///
/// The expander used to take the pattern from a fixed `terms[1]`, which is the
/// `mutable_specifier` here, not the pattern. The quote then expanded as an
/// ordinary term-builder *in binding position*, so `quilt expand` happily wrote
/// Rust that does not compile:
///
/// ```text
/// let mut tb("binary_expression").c(&a) … .b() = v;
/// ```
///
/// The meta-language now reports the pattern and value positions relative to its
/// own separator token, so an extra child cannot shift them (#174, finding E3).
#[test]
fn pattern_let_with_mut() -> Result<()> {
    let out = expand_both("let mut ↖1 + ↙x↘↗ = rhs;")?;
    assert!(
        out.contains("qmatch_n"),
        "`let mut` should still be a pattern-let: {out}"
    );
    assert!(
        !out.contains("tb(\"binary_expression\").c(&x)"),
        "the pattern must not expand as a term builder in binding position: {out}"
    );
    insta::assert_snapshot!(out);
    Ok(())
}

#[test]
fn pattern_let_value_quote_untouched() -> Result<()> {
    // Only the binding position triggers pattern matching: a value quote and
    // a type-position quote expand as before.
    let out = expand_both("let x = ↖1 + 2↗;")?;
    assert!(!out.contains("qmatch_n"), "{out}");
    let out = expand_both("let x: ↖T↗ = rhs;")?;
    assert!(!out.contains("qmatch_n"), "{out}");
    Ok(())
}

#[test]
fn pattern_let_duplicate_var_rejected() {
    let mut omni = Omni::default();
    let qterm = omni.parse("let ↖↙x↘ + ↙x↘↗ = rhs;").unwrap();
    let err = omni.expand(&qterm).unwrap_err();
    assert!(err.to_string().contains("more than once"), "{err}");
}

#[test]
fn pattern_let_non_ident_var_rejected() {
    let mut omni = Omni::default();
    let qterm = omni.parse("let ↖1 + ↙f(x)↘↗ = rhs;").unwrap();
    let err = omni.expand(&qterm).unwrap_err();
    assert!(err.to_string().contains("plain identifier"), "{err}");
}

/// A host that spells metavariable binders `$name`, to check that
/// [`MetaLanguage::pattern_var_name`] is what decides.
///
/// Every answer is `RustMetaLanguage`'s except that one. The rule used to be a
/// free function in `multi.rs` applied to every host, so this spelling was not
/// expressible at all — the `$` would have been rejected as "not a plain
/// identifier" before the meta-language was ever consulted (#174, finding E4).
struct DollarMeta;

impl MetaLanguage for DollarMeta {
    /// The point of the test: `$a` binds the name `a`.
    fn pattern_var_name(&self, term: &QTerm) -> Result<Box<str>> {
        let text = term.coparse();
        let name = text.trim();
        let rest = name
            .strip_prefix('$')
            .ok_or_else(|| miette!("pattern metavariable must be $name, got {name:?}"))?;
        ensure!(!rest.is_empty(), "empty metavariable name");
        Ok(rest.into())
    }

    // Everything below is Rust's own answer, unchanged.
    fn expand_quote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        RustMetaLanguage.expand_quote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_unquote(
        &self,
        lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        RustMetaLanguage.expand_unquote(lang1, tag, i, lang2, qterm, cmds)
    }

    fn expand_tuple(
        &self,
        lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity,
    ) -> Result<Arc<QTerm>> {
        RustMetaLanguage.expand_tuple(lang1, tag, qterms, cmds, arity)
    }

    fn pattern_tag(&self) -> Option<&'static str> {
        RustMetaLanguage.pattern_tag()
    }

    fn pattern_binding(&self, terms: &[Arc<QTerm>]) -> Option<(usize, usize)> {
        RustMetaLanguage.pattern_binding(terms)
    }

    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        RustMetaLanguage.pattern_var(name)
    }

    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        RustMetaLanguage.pattern_let(names, pattern, value)
    }
}

fn dollar_multi() -> DictMulti {
    let mut multi = DictMulti::default();
    multi.add_lang("rs", Box::new(DynRustLanguage::default()));
    multi.add_meta("rs", Box::new(DollarMeta));
    multi
}

/// A host can choose its own metavariable spelling, because the rule is now a
/// trait method rather than the core's (#174, finding E4).
#[test]
fn pattern_var_name_is_the_metas_choice() -> Result<()> {
    let mut multi = dollar_multi();
    let qterm = multi.parse_lang("rs", "let ↖↙$a↘ + ↙$b↘↗ = rhs;")?;
    let out = multi.expand_lang("rs", &qterm)?.coparse();

    // `$a` bound the name `a`: it is the binder and the metavariable both.
    assert!(out.contains("let [a, b]"), "{out}");
    assert!(out.contains("mvar(\"a\")"), "{out}");
    assert!(out.contains("mvar(\"b\")"), "{out}");
    // The `$` is a binder sigil, not part of the name.
    assert!(!out.contains("$a"), "{out}");
    Ok(())
}

/// The same override rejects what it does not recognise — a bare identifier,
/// which is exactly what the old hardcoded rule *only* accepted.
#[test]
fn pattern_var_name_override_rejects_bare_ident() {
    let mut multi = dollar_multi();
    let qterm = multi.parse_lang("rs", "let ↖↙a↘ + ↙b↘↗ = rhs;").unwrap();
    let err = multi.expand_lang("rs", &qterm).unwrap_err();
    assert!(err.to_string().contains("must be $name"), "{err}");
}

#[test]
fn pattern_let_runtime() -> Result<()> {
    // End-to-end: expand and run the issue #18 example shape. The Rust
    // pattern destructures a Rust quote, the Python pattern a Python quote;
    // each metavariable binds the matched source text.
    let mut omni = Omni::default();
    let code = indoc! {r#"{
        let ↖1 + ↙x↘↗ = ↖1 + 2↗;
        let py↖def f(↙args↘): pass↗ = py↖def f(y: int): pass↗;
        format!("{} | {}", x.coparse(), args.coparse())
    }"#};
    let qterm = omni.parse(code)?;
    let out: String = omni.expand(&qterm)?.reduce()?;
    assert_eq!(out, "2 | y: int");
    Ok(())
}

#[test]
fn reduce() -> Result<()> {
    let mut omni = Omni::default();
    let code = "3..5";
    let qterm = omni.parse(code)?;
    let reduced: Range<i32> = qterm.reduce()?;
    assert_eq!(reduced, 3..5);
    Ok(())
}

#[test]
fn splicing_nested() -> Result<()> {
    let out = expand_both(indoc! {r#"{
        fn mk(i: usize) -> Result<⟨T⟩> {
            Ok(↖{
                ↙{
                    for c in 0..i {
                        {
                            if c != 0 {NL.←;}
                            ↖println!("hi");↗
                        }.←;
                    }
                }↘
            }↗)
        }
        mk(3).unwrap()
    }"#})?;
    insta::assert_snapshot!(out);
    Ok(())
}
