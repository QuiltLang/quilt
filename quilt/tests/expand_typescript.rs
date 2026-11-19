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
    assert_eq!(code, q.coparse_quilt());
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

/// `←` is functional in a TypeScript host (issue #152): `←(items)` spells the
/// runtime's `qseq`, which splices the whole sequence into the container the
/// unquote sits in — here a function body — and `←(items, sep)` passes the
/// separator through.
#[test]
fn host_emit_spells_qseq() -> Result<()> {
    insta::assert_snapshot!(expand(
        &["ts"],
        indoc! {r#"
            const out = ts↖function f() {
              console.log("start");
              ↙←(names.map((n) => ts↖console.log(↙name(n)↘);↗))↘
            }↗;
            const call = ts↖g(↙←(args, ", ")↘)↗;
        "#}
    )?);
    Ok(())
}

/// The sequence is `←`'s argument, so the glyph must open a call. Rust's
/// postfix habit (`ts↖…↗.←` in a ground loop) and a bare `←` would expand to a
/// reference to `qseq` and fail far from the source; they fail here instead,
/// with the spelling that works.
#[test]
fn host_emit_needs_a_call() {
    for code in [
        indoc! {r#"
            const out = ts↖function f() {
                ↙
                for (const n of names) {
                    ts↖console.log(↙name(n)↘)↗.←
                }
                ↘
            }↗;
        "#},
        "const x = ←;\n",
        "const x = ← (xs);\n",
    ] {
        let msg = expand(&["ts"], code).unwrap_err().to_string();
        assert!(msg.contains("write `←(items)`"), "{msg}");
    }
}

/// A `←` at sky depth belongs to a *later* stage, so it is still deferred as
/// its glyph rather than rejected here.
#[test]
fn host_emit_deferred_in_quote() -> Result<()> {
    insta::assert_snapshot!(expand(
        &["ts"],
        "const x = ts↖const y = ts↖const z = ←↗↗;\n"
    )?);
    Ok(())
}

/// Every child the expansion builds itself, and every lift, is spliced with the
/// runtime's taking `.t(..)`, so a tree built by one quote is owned by its root
/// and freed with it; a spliced host value — which the program may still hold —
/// keeps the borrowing `.c(..)`/`.e(..)`. Borrowing a temporary pinned its subtree in
/// wasm memory until the garbage collector ran its finalizer, so repeated
/// expansions grew without bound in a synchronous loop.
#[test]
fn fresh_children_are_taken_host_values_borrowed() -> Result<()> {
    let out = expand(
        &["ts", "html"],
        "const x = html↖<p>↙title↘ <b>↙f(x)↘</b> ↙↑(y)↘</p>↗;\n",
    )?;
    // Literal structure: taken.
    assert!(out.contains(r#".t(sym("<"))"#), "{out}");
    assert!(out.contains(r#".t(tb("start_tag")"#), "{out}");
    assert!(out.contains(r#".t(tb("element")"#), "{out}");
    // A lift returns a new term on every call.
    assert!(out.contains(".t(qlift_html(y))"), "{out}");
    // Host values, including a call that may return a term it keeps: borrowed.
    for v in ["title", "f(x)"] {
        assert!(!out.contains(&format!(".t({v})")), "{v} was taken: {out}");
        assert!(
            out.contains(&format!(".e({v})")) || out.contains(&format!(".c({v})")),
            "{v} not spliced: {out}"
        );
    }
    Ok(())
}
