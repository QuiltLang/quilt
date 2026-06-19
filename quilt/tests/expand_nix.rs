//! Nix is a *target* language: `nix↖ … ↗` fragments embedded in a Rust host,
//! expanded by the Rust `MetaLanguage`. These tests check that Nix fragments
//! parse, round-trip through `coparse`, and expand to builder code.
//!
//! Nix is purely expression-oriented (no statements), so every fragment is a
//! value and unquotes splice into expression positions.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse `code` and assert `coparse` reproduces it exactly.
fn roundtrip(code: &str) -> Result<()> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    assert_eq!(code, q.coparse());
    Ok(())
}

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

#[test]
fn roundtrip_expr() -> Result<()> {
    roundtrip("const X: T = nix↖a + ↙b↘ * 2↗;\n")
}

#[test]
fn roundtrip_attrset() -> Result<()> {
    roundtrip("const X: T = nix↖{ pname = ↙name↘; version = \"1.0\"; }↗;\n")
}

#[test]
fn roundtrip_list() -> Result<()> {
    roundtrip("const X: T = nix↖[ 1 2 ↙x↘ ]↗;\n")
}

#[test]
fn roundtrip_select() -> Result<()> {
    roundtrip("const X: T = nix↖pkgs.hello.${↙attr↘}↗;\n")
}

#[test]
fn roundtrip_derivation() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = nix↖
            { pkgs ? import <nixpkgs> { }, ... }:
            pkgs.stdenv.mkDerivation {
              pname = ↙name↘;
              version = "1.0";
              buildInputs = [ pkgs.hello ];
            }
        ↗;
    "#})
}

#[test]
fn expand_expr() -> Result<()> {
    let out = expand(indoc! {r#"
        fn nix_expr(b: &Arc<QTerm>) -> Arc<QTerm> {
            nix↖1 + ↙b↘↗
        }
    "#})?;
    println!("{out}");
    // The expansion reconstructs the Nix expression via the builder, splicing `b`.
    assert!(out.contains(".c(&b)"));
    Ok(())
}

#[test]
fn expand_attrset() -> Result<()> {
    let out = expand(indoc! {r#"
        fn drv(name: &Arc<QTerm>) -> Arc<QTerm> {
            nix↖{ pname = ↙name↘; }↗
        }
    "#})?;
    println!("{out}");
    // The binding's value position is non-variadic, so the splice is positional.
    assert!(out.contains(".c(&name)"));
    Ok(())
}

/// A `↑` inside an unquote in a `nix↖…↗` quote lifts *into Nix*: it expands to
/// the heterogeneous `qlift_to::<Nix>()`, not the homogeneous `qlift()`.
#[test]
fn expand_heterogeneous_lift() -> Result<()> {
    let out = expand(indoc! {r#"
        fn drv(version: &str) -> Arc<QTerm> {
            nix↖{ version = ↙version.↑↘; }↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains("version.qlift_to::<Nix>()"));
    Ok(())
}

/// A generation-time loop can emit lifted values into a variadic Nix list
/// (`[ … ]`): each `↑.←` lifts a Rust value to a Nix term and appends it.
#[test]
fn expand_list_emit() -> Result<()> {
    let out = expand(indoc! {r#"
        fn names(items: &[String]) -> Arc<QTerm> {
            nix↖[ ↙{ for s in items { s.↑.←; } }↘ ]↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains("qlift_to::<Nix>()"));
    assert!(out.contains(".emit(&mut b_)"));
    Ok(())
}

/// With the chain `["rs", "nix"]` (from a filename like `flake.nix.rs.quilt`),
/// un-annotated quotes default to Nix: bare `↖…↗` parses and expands exactly
/// like the explicit `nix↖…↗` spelling.
#[test]
fn chain_default_quote_lang() -> Result<()> {
    let code = indoc! {r#"
        fn drv(name: &Arc<QTerm>) -> Arc<QTerm> {
            ↖{ pname = ↙name↘; }↗
        }
    "#};
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["rs", "nix"], code)?;
    assert_eq!(code, q.coparse());
    let out = omni.expand_lang("rs", &q)?.coparse();
    println!("{out}");

    let explicit = expand(&code.replace('↖', "nix↖"))?;
    assert_eq!(out, explicit);
    Ok(())
}
