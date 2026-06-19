//! Nix as a *target* language: `nix↖ … ↗` fragments embedded in a Rust host,
//! expanded by the Rust `MetaLanguage` (the `roundtrip_*` / `expand_*` tests),
//! and Nix as a *host* (meta) language driving generation with its string-based
//! meta (the `host_*` tests — see `langs::nix::meta`).
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

/// Parse + expand `code` with **Nix as the host** (ground language), returning
/// the coparsed Nix metaprogram (string-based meta — see `langs::nix::meta`).
fn host_expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["nix"], code)?;
    Ok(omni.expand_lang("nix", &q)?.coparse())
}

/// Nix-as-host, homogeneous: a `nix↖…↗` quote becomes a Nix string literal and
/// a host unquote `↙key↘` becomes Nix's own `${key}` antiquotation.
#[test]
fn host_attrset_splice() -> Result<()> {
    let out = host_expand("let key = \"enabled\"; in nix↖{ ↙key↘ = true; }↗")?;
    assert_eq!(out, "let key = \"enabled\"; in \"{ ${key} = true; }\"");
    Ok(())
}

/// A fully literal fragment flattens to a single flat string — no tower of
/// `${\"…\"}` from the nested tuple structure.
#[test]
fn host_literal_flattens() -> Result<()> {
    assert_eq!(
        host_expand("nix↖{ x = 1; y = 2; }↗")?,
        "\"{ x = 1; y = 2; }\""
    );
    assert_eq!(host_expand("nix↖[ 1 2 ↙x↘ ]↗")?, "\"[ 1 2 ${x} ]\"");
    Ok(())
}

/// `↑` in a host unquote spells Nix's `toString`, rendering a value as text for
/// interpolation: `↙↑ n↘` becomes `${toString n}`.
#[test]
fn host_lift_to_string() -> Result<()> {
    let out = host_expand("let n = 3; in nix↖x + ↙↑ n↘↗")?;
    assert_eq!(out, "let n = 3; in \"x + ${toString n}\"");
    Ok(())
}

/// The string model is language-agnostic: a Nix host can generate *any* target
/// (here Bash), reconstructing it the same way.
#[test]
fn host_generates_other_language() -> Result<()> {
    assert_eq!(host_expand("bash↖echo ↙msg↘↗")?, "\"echo ${msg}\"");
    Ok(())
}

/// Multi-line fragments keep their newlines and indentation inside the Nix
/// string literal.
#[test]
fn host_multiline() -> Result<()> {
    let out = host_expand(indoc! {r#"
        nix↖{
          name = ↙name↘;
          deps = [ ↙dep↘ ];
        }↗"#})?;
    assert_eq!(
        out,
        indoc! {r#"
            "{
              name = ${name};
              deps = [ ${dep} ];
            }""#}
    );
    Ok(())
}

/// Ground (host) Nix code without lift round-trips through parse + coparse.
#[test]
fn host_roundtrips() -> Result<()> {
    let code = "let key = \"enabled\"; in nix↖{ ↙key↘ = true; }↗";
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["nix"], code)?;
    assert_eq!(code, q.coparse());
    Ok(())
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
