//! HTML is a *target* language: `html↖ … ↗` fragments embedded in a Rust host,
//! expanded by the Rust `MetaLanguage`. These tests check that HTML fragments
//! parse, round-trip through `coparse`, and expand to builder code — including
//! holes inside `<script>` raw text and inside attribute values.

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
fn roundtrip_element() -> Result<()> {
    roundtrip("const X: T = html↖<div class=\"box\">hi</div>↗;\n")
}

#[test]
fn roundtrip_attr_hole() -> Result<()> {
    roundtrip("const X: T = html↖<input value=\"↙w↘\">↗;\n")
}

#[test]
fn roundtrip_page() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = html↖
            <!DOCTYPE html>
            <html>
            <body>
            <canvas id="c"></canvas>
            <script type="module">
            const WIDTH = ↙w↘;
            canvas.width = ↙cw↘;
            </script>
            </body>
            </html>
        ↗;
    "#})
}

#[test]
fn roundtrip_style() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = html↖
            <style>
              body { margin: 0; background: #111; }
            </style>
        ↗;
    "#})
}

#[test]
fn roundtrip_script_only_hole() -> Result<()> {
    // A hole directly between the tags, with no surrounding raw text.
    roundtrip("const X: T = html↖<script type=\"text/plain\">↙b64↘</script>↗;\n")
}

#[test]
fn roundtrip_backslashes() -> Result<()> {
    // `\n` inside the raw text must survive as backslash-n (the generated
    // builder code must escape `\` in its string literals).
    roundtrip(indoc! {r#"
        const X: T = html↖
            <script>
            const blob = new Blob([src, '\n', handler]);
            </script>
        ↗;
    "#})
}

#[test]
fn expand_backslashes() -> Result<()> {
    let out = expand(indoc! {r#"
        fn page() -> Arc<QTerm> {
            html↖<script>let s = 'a\nb';</script>↗
        }
    "#})?;
    println!("{out}");
    // The builder string literal must double the backslash.
    assert!(out.contains(r"a\\nb"));
    Ok(())
}

#[test]
fn expand_attr_hole() -> Result<()> {
    let out = expand(indoc! {r#"
        fn page(w: &Arc<QTerm>) -> Arc<QTerm> {
            html↖<input value="↙w↘">↗
        }
    "#})?;
    println!("{out}");
    // The expansion reconstructs the element via the builder, splicing `w`.
    assert!(out.contains("input"));
    assert!(out.contains("quoted_attribute_value"));
    assert!(out.contains(".c(&w)") || out.contains("w.emit("));
    Ok(())
}

#[test]
fn expand_script_hole() -> Result<()> {
    let out = expand(indoc! {r#"
        fn page(shader: &Arc<QTerm>) -> Arc<QTerm> {
            html↖
                <script type="x-wgsl">
                ↙shader↘
                </script>
            ↗
        }
    "#})?;
    println!("{out}");
    assert!(out.contains("script_element"));
    assert!(out.contains(".c(&shader)") || out.contains("shader.emit("));
    Ok(())
}
