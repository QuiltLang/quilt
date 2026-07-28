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
    insta::assert_snapshot!(out);
    Ok(())
}

/// A fully literal fragment flattens to a single flat string — no tower of
/// `${\"…\"}` from the nested tuple structure.
#[test]
fn host_literal_flattens() -> Result<()> {
    insta::assert_snapshot!(host_expand("nix↖{ x = 1; y = 2; }↗")?);
    insta::assert_snapshot!(host_expand("nix↖[ 1 2 ↙x↘ ]↗")?);
    Ok(())
}

/// `↑` in a host unquote spells Nix's `toString`, rendering a value as text for
/// interpolation: `↙↑ n↘` becomes `${toString n}`.
#[test]
fn host_lift_to_string() -> Result<()> {
    let out = host_expand("let n = 3; in nix↖x + ↙↑ n↘↗")?;
    insta::assert_snapshot!(out);
    Ok(())
}

/// The string model is language-agnostic: a Nix host can generate *any* target
/// (here Bash), reconstructing it the same way.
#[test]
fn host_generates_other_language() -> Result<()> {
    insta::assert_snapshot!(host_expand("bash↖echo ↙msg↘↗")?);
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

/// Nix-as-host has no `b_` accumulator to emit into (see `wrap_child` in
/// `langs::nix::meta`), so a *ground* `←` fails loudly instead of leaking the
/// `__EMIT__` placeholder into the generated Nix, and the message points at the
/// functional alternative.
#[test]
fn host_emit_unsupported() {
    for code in [
        // Inside a host unquote — the case that used to expand to
        // `"a ${v.__EMIT__} b"` with no error at all.
        r#"let v = "x"; in nix↖a ↙v.←↘ b↗"#,
        // …and at plain ground position.
        "let gen = ←; in gen",
    ] {
        let msg = host_expand(code).unwrap_err().to_string();
        assert!(msg.contains("nix can't emit"), "{msg}");
        assert!(msg.contains("concatStringsSep"), "{msg}");
    }
}

/// A `←` at sky depth belongs to a *later* stage, so it is still deferred as
/// its glyph — rejecting emit for this host must not over-fire on quoted code
/// the host merely passes through.
#[test]
fn host_emit_deferred_in_quote() -> Result<()> {
    insta::assert_snapshot!(host_expand("let gen = nix↖{ a = ←; }↗; in gen")?);
    Ok(())
}

/// Nix is untyped and has no annotation syntax, so `⟨T⟩` has nowhere to go and
/// fails loudly rather than leaking `__TYPE__`. Unlike `↑↓←` it is not staged,
/// so this holds inside a quote too. (Lean, the other string host, answers
/// `String`.)
#[test]
fn host_type_unsupported() {
    for code in [
        "let gen = ⟨T⟩; in gen",
        "let gen = nix↖{ a = ⟨T⟩; }↗; in gen",
    ] {
        let msg = host_expand(code).unwrap_err().to_string();
        assert!(msg.contains("nix has no type for"), "{msg}");
    }
}

/// `⟨N⟩` builds an identifier from a string. A fragment *is* a string here, so
/// the spelling is the identity — which for a Nix string is `toString`.
#[test]
fn host_name_is_identity() -> Result<()> {
    insta::assert_snapshot!(host_expand(
        r#"let pkg = "hello"; in nix↖{ ↙⟨N⟩ pkg↘ = true; }↗"#
    )?);
    Ok(())
}

/// `↓` compiles a term and deserializes the result back, which needs the
/// `QTerm` runtime this host doesn't have — so a *ground* `↓` fails loudly
/// instead of leaking `__REDUCE__`, pointing at ordinary Nix evaluation.
#[test]
fn host_reduce_unsupported() {
    let msg = host_expand("let gen = ↓nix↖1↗; in gen")
        .unwrap_err()
        .to_string();
    assert!(msg.contains("nix can't reduce"), "{msg}");
    assert!(msg.contains("↙…↘"), "{msg}");
}

/// Like `←`, a `↓` at sky depth belongs to a later stage and is still deferred
/// as its glyph.
#[test]
fn host_reduce_deferred_in_quote() -> Result<()> {
    insta::assert_snapshot!(host_expand("let gen = nix↖{ a = ↓; }↗; in gen")?);
    Ok(())
}
