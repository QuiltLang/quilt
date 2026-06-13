//! TypeScript is a *host* language: a `.ts.quilt` file is TypeScript with
//! arrow-bracket quotes, expanded to plain TypeScript that calls the
//! `quilt-wasm` runtime. These tests check that TS fragments parse, round-trip
//! through `coparse`, and that the host expander rewrites quotes into runtime
//! builder calls — including the `.html.ts` chain that drives the browser demo.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse `code` under `chain` (ground language first) and assert `coparse`
/// reproduces it exactly.
fn roundtrip(chain: &[&str], code: &str) -> Result<()> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(chain, code)?;
    assert_eq!(code, q.coparse());
    Ok(())
}

/// Parse + expand `code` under `chain`, returning the coparsed builder source.
fn expand(chain: &[&str], code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(chain, code)?;
    Ok(omni.expand_lang(chain[0], &q)?.coparse())
}

/**************************************************************/
// Round-trips: the parsed surface coparses back to the exact input.

#[test]
fn roundtrip_homogeneous_quote() -> Result<()> {
    // A TypeScript quote inside TypeScript (the meta-meta level).
    roundtrip(&["ts"], "const x = ts↖foo(1)↗;\n")
}

#[test]
fn roundtrip_html_expr_hole() -> Result<()> {
    // `.html.ts` chain: TS ground, bare quotes default to HTML; `↙title↘`
    // splices a TS term into HTML node position.
    roundtrip(&["ts", "html"], "const x = html↖<li>↙title↘</li>↗;\n")
}

#[test]
fn lift_resolves_to_qlift_html_at_parse() -> Result<()> {
    // `↑` is resolved when the surface is built, not at expand time: into an
    // HTML splice it spells the runtime's entity-escaping `qlift_html`. (So a
    // source containing `↑` does not round-trip to the glyph — it carries the
    // resolved spelling.)
    let mut omni = Omni::default();
    let q = omni.parse_chain(&["ts", "html"], "const x = html↖<li>↙↑(title)↘</li>↗;\n")?;
    assert!(q.coparse().contains("qlift_html(title)"));
    Ok(())
}

#[test]
fn roundtrip_html_page() -> Result<()> {
    roundtrip(
        &["ts", "html"],
        indoc! {"
            function page(title: QTerm, body: QTerm): QTerm {
              return html↖
                <!DOCTYPE html>
                <html>
                <head><title>↙title↘</title></head>
                <body>↙body↘</body>
                </html>
              ↗;
            }
        "},
    )
}

/**************************************************************/
// Expansions: quotes become `quilt-wasm` runtime builder calls; the ground
// TypeScript around them is preserved.

#[test]
fn expand_html_element() -> Result<()> {
    let out = expand(&["ts", "html"], "const x = html↖<li>hi</li>↗;\n")?;
    println!("{out}");
    // The ground statement survives; the quote becomes a builder chain.
    assert!(out.contains("const x ="));
    assert!(out.contains("tb(\"element\")"));
    assert!(out.contains(".b()"));
    // No arrow brackets remain after expansion.
    assert!(!out.contains('↖') && !out.contains('↗'));
    Ok(())
}

#[test]
fn expand_html_expr_hole() -> Result<()> {
    let out = expand(&["ts", "html"], "const x = html↖<li>↙title↘</li>↗;\n")?;
    println!("{out}");
    // `<li>` is a variadic HTML `element`, so the spliced TS term `title` is
    // appended with `.e(...)` (emit), not `.c(...)`.
    assert!(out.contains(".e(title)"));
    Ok(())
}

#[test]
fn expand_html_lift_hole() -> Result<()> {
    // `↑` into HTML spells the runtime's entity-escaping `qlift_html`.
    let out = expand(&["ts", "html"], "const x = html↖<li>↙↑(title)↘</li>↗;\n")?;
    println!("{out}");
    assert!(out.contains("qlift_html(title)"));
    Ok(())
}

#[test]
fn expand_homogeneous_quote() -> Result<()> {
    // A bare TS quote at ground level reconstructs itself via the builder.
    let out = expand(&["ts"], "const x = ts↖1 + 2↗;\n")?;
    println!("{out}");
    assert!(out.contains("tb(\"binary_expression\")"));
    assert!(out.contains("leaf(\"number\", \"1\")"));
    Ok(())
}

#[test]
fn expand_backslashes() -> Result<()> {
    // A backslash in HTML raw text must be doubled in the emitted TS string.
    let out = expand(
        &["ts", "html"],
        "const x = html↖<script>let s = 'a\\nb';</script>↗;\n",
    )?;
    println!("{out}");
    assert!(out.contains(r"a\\nb"));
    Ok(())
}
