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
    assert_eq!(code, q.coparse_quilt());
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
    assert_eq!(code, q.coparse_quilt());
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
    assert_eq!(code, q.coparse_quilt());
    let out = omni.expand_lang("rs", &q)?.coparse();
    println!("{out}");

    let explicit = expand(&code.replace('↖', "nix↖"))?;
    assert_eq!(out, explicit);
    Ok(())
}

/// Nix-as-host has no `b_` accumulator, but it does have emit (issue #155):
/// `←` is the *functional* reading of "append these into the surrounding
/// container" — hand it the whole list of fragments and it joins them. It is
/// applied prefix, by juxtaposition, exactly like `↑`/`toString`.
#[test]
fn host_emit_joins_list() -> Result<()> {
    let out = host_expand(r#"let ns = [ "a" "b" ]; in nix↖[ ↙← (map (n: nix↖"↙n↘"↗) ns)↘ ]↗"#)?;
    insta::assert_snapshot!(out);
    // No `__EMIT__` placeholder may reach the generated Nix.
    assert!(!out.contains("__EMIT__"), "{out}");
    Ok(())
}

/// The separator is a newline because that is the only one correct for *both*
/// container kinds: whitespace-insensitive Nix (above) and a line-oriented
/// target like Bash, where the emitted fragments must land one per line.
#[test]
fn host_emit_into_target_language() -> Result<()> {
    let out = host_expand(indoc! {r#"
        let fs = [ "a" "b" ]; in bash↖
        set -e
        ↙← (map (f: bash↖cp ↙f↘ /tmp/↗) fs)↘
        ↗"#})?;
    insta::assert_snapshot!(out);
    Ok(())
}

/// The spelling is `builtins`-only — this host ships no runtime library, so a
/// generated Nix file must stay evaluable with nothing imported.
#[test]
fn host_emit_spelling_needs_no_runtime() -> Result<()> {
    let out = host_expand("let gen = ←; in gen")?;
    assert_eq!(out, "let gen = (builtins.concatStringsSep \"\\n\"); in gen");
    Ok(())
}

/// The spelling has to be *evaluable* Nix, not merely well-formed text, and it
/// has to mean what the docs say: fragments joined one per line. Pinning the
/// spelling in a snapshot cannot catch a typo inside it, so evaluate it for
/// real when a `nix` is on `PATH` (CI runs inside `nix develop`; a contributor
/// without Nix still gets every other test in this file).
#[test]
fn host_emit_evaluates() -> Result<()> {
    if std::process::Command::new("nix")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skipping host_emit_evaluates: no `nix` on PATH");
        return Ok(());
    }
    let expanded =
        host_expand(r#"let ns = [ "a" "b" ]; in nix↖[ ↙← (map (n: nix↖"↙n↘"↗) ns)↘ ]↗"#)?;
    let out = std::process::Command::new("nix")
        .args(["eval", "--raw", "--expr", &expanded])
        .output()
        .expect("running `nix eval`");
    assert!(
        out.status.success(),
        "nix eval failed on {expanded}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[ \"a\"\n\"b\" ]");
    Ok(())
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
