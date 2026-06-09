//! WGSL is a *target* language: `wgsl↖ … ↗` fragments embedded in a Rust host,
//! expanded by the Rust `MetaLanguage`. These tests check that WGSL fragments
//! parse, round-trip through `coparse`, and expand to builder code.

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
    roundtrip("const X: T = wgsl↖bitcast<u32>(↙y↘)↗;\n")
}

#[test]
fn roundtrip_stmt() -> Result<()> {
    roundtrip("const X: T = wgsl↖agents[idx].reg[1] = move_ok[idx];↗;\n")
}

#[test]
fn roundtrip_shader() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = wgsl↖
            @compute @workgroup_size(64)
            fn run(@builtin(global_invocation_id) gid: vec3<u32>) {
                let idx = gid.x;
                agents[idx].reg[0] = ↙rhs↘;
            }
        ↗;
    "#})
}

#[test]
fn expand_expr() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(y: &Arc<QTerm>) -> Arc<QTerm> {
            wgsl↖bitcast<u32>(↙y↘)↗
        }
    "#})?;
    println!("{out}");
    // The expansion reconstructs the WGSL expression via the builder, splicing `y`.
    assert!(out.contains("bitcast"));
    assert!(out.contains(".c(&y)"));
    Ok(())
}

#[test]
fn expand_stmt() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(rhs: &Arc<QTerm>) -> Arc<QTerm> {
            wgsl↖agents[idx].reg[0] = ↙rhs↘;↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains(".c(&rhs)"));
    Ok(())
}

/// With the chain `["rs", "wgsl"]` (from a filename like `shaders.wgsl.rs.quilt`),
/// un-annotated quotes default to WGSL: bare `↖…↗` parses and expands exactly
/// like the explicit `wgsl↖…↗` spelling.
#[test]
fn chain_default_quote_lang() -> Result<()> {
    let code = indoc! {r#"
        fn shader(y: &Arc<QTerm>) -> Arc<QTerm> {
            ↖bitcast<u32>(↙y↘)↗
        }
    "#};
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["rs", "wgsl"], code)?;
    assert_eq!(code, q.coparse());
    let out = omni.expand_lang("rs", &q)?.coparse();
    println!("{out}");

    let explicit = expand(&code.replace('↖', "wgsl↖"))?;
    assert_eq!(out, explicit);
    Ok(())
}

/// A single-language chain behaves exactly like the plain `parse`: un-annotated
/// quotes default to the host language.
#[test]
fn chain_single_lang_back_compat() -> Result<()> {
    let code = indoc! {r#"
        fn pair(x: &Arc<QTerm>) -> Arc<QTerm> {
            ↖(↙x↘, ↙x↘)↗
        }
    "#};
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["rs"], code)?;
    assert_eq!(code, q.coparse());
    let plain = omni.parse(code)?;
    assert_eq!(q.coparse(), plain.coparse());
    Ok(())
}

/// A `↑` inside an unquote in a `wgsl↖…↗` quote lifts *into WGSL*: it expands
/// to the heterogeneous `qlift_to::<Wgsl>()`, not the homogeneous `qlift()`.
#[test]
fn expand_heterogeneous_lift() -> Result<()> {
    let out = expand(indoc! {r#"
        fn shader(width: u32) -> Arc<QTerm> {
            wgsl↖const G_WIDTH: u32 = ↙width.↑↘;↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains("width.qlift_to::<Wgsl>()"));
    Ok(())
}

/// A `↑` outside any quote stays homogeneous (`qlift()`).
#[test]
fn expand_homogeneous_lift_unaffected() -> Result<()> {
    let out = expand(indoc! {r#"
        fn lifted() -> Arc<QTerm> {
            ↖0↗.↑
        }
    "#})?;
    println!("{out}");
    assert!(out.contains(".qlift()"));
    assert!(!out.contains("qlift_to"));
    Ok(())
}
