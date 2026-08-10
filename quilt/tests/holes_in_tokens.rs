//! Holes that are not a whole word (issue #221).
//!
//! Hole detection in `treesitter.rs` matches a node whose byte range *equals*
//! the hole's. `__QUILT_HOLE__` is spelled so it lexes as an ordinary
//! identifier or word, so that holds exactly where the hole stands alone —
//! and not inside a string, inside a comment, or glued to neighbouring text,
//! where the token around it swallows it. No node matched, the hole point went
//! unconsumed, and the failure surfaced later and elsewhere as "Ran out of
//! holes for unquote".
//!
//! `write_run` now splits the run instead: the text before the hole, the hole
//! as a child of the enclosing token, the text after. It asks nothing of any
//! grammar — which matters, because the grammar route is currently shut: the
//! forks' `quilt_hole` rule panics `tree-sitter generate` (see
//! `QuiltLang/tree-sitter-zsh#1`), and the concatenation case needs the hole to
//! be a `concatenation` element rather than a token anyway.
//!
//! So these are deliberately *cross-language* tests: the fix lives below every
//! grammar, and a regression in it would be a regression for all of them.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/// Parse + expand `code`, returning the coparsed builder source.
fn expand(code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse(code)?;
    Ok(omni.expand(&q)?.coparse())
}

/// Parse + expand with a shell as the ground language (its string-based host,
/// #151), returning the generated shell metaprogram.
fn host_expand(shell: &str, code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(&[shell], code)?;
    Ok(omni.expand_lang(shell, &q)?.coparse())
}

/// Every tree-sitter-backed language, in the position this issue is about.
///
/// One case per language rather than one language exercised deeply: the split
/// happens under `Language`, so what is worth pinning is that no grammar has to
/// opt in. Each of these was "Ran out of holes for unquote" before #221.
#[test]
fn a_hole_inside_a_string_works_in_every_language() -> Result<()> {
    for code in [
        "const X: T = ↖let s = \"hi ↙x↘ there\";↗;\n",
        "const X: T = py↖print(\"hi ↙x↘\")↗;\n",
        "const X: T = ts↖const s = \"hi ↙x↘\";↗;\n",
        "const X: T = lean↖def f := \"hi ↙x↘\"↗;\n",
        "const X: T = bash↖echo \"hi ↙x↘\"↗;\n",
        "const X: T = zsh↖echo \"hi ↙x↘\"↗;\n",
        "const X: T = nix↖{ a = \"hi ↙x↘\"; }↗;\n",
    ] {
        let out = expand(code).map_err(|e| e.wrap_err(format!("expanding {code:?}")))?;
        assert!(
            out.contains("\"hi \""),
            "the text before the hole should be written verbatim, got:\n{out}"
        );
    }
    Ok(())
}

/// The other two positions from the issue, in the host that made them visible.
///
/// `↙u↘.service` is the one a grammar-level `quilt_hole` token would *not* have
/// fixed: `__QUILT_HOLE__.service` is a single `word`, so the hole has to
/// become an element of it rather than a token of its own.
#[test]
fn glued_text_and_comments_take_holes() -> Result<()> {
    assert_eq!(
        host_expand("bash", "echo bash↖systemctl start ↙$u↘.service↗\n")?,
        "echo \"systemctl start $u.service\"\n"
    );
    assert_eq!(
        host_expand("bash", "echo bash↖# built by ↙$u↘↗\n")?,
        "echo \"# built by $u\"\n"
    );
    Ok(())
}

/// A shell host's escaping still applies to the text *around* the hole: the
/// generated `"` is escaped into the metaprogram's own string while the splice
/// stays live. The split must not bypass `expand_tuple`, and this is what says
/// so.
#[test]
fn splitting_a_token_keeps_the_host_escaping() -> Result<()> {
    assert_eq!(
        host_expand("bash", "echo bash↖echo \"hi ↙$x↘ there\"↗\n")?,
        "echo \"echo \\\"hi $x there\\\"\"\n"
    );
    Ok(())
}

/// A multi-line token takes a hole too. This is the other write path — a
/// multi-line token writes its own rows rather than going through the gap
/// logic — so it needs its own case, and the newline has to survive the split.
#[test]
fn a_hole_inside_a_multiline_token() -> Result<()> {
    let out = expand(indoc! {"
        const X: T = py↖s = \"\"\"line one
        line ↙x↘ two\"\"\"↗;
    "})?;
    assert!(out.contains("b_.write(\"line one\")"), "{out}");
    assert!(out.contains("b_.nl()"), "{out}");
    assert!(out.contains("b_.write(\"line \")"), "{out}");
    assert!(out.contains("x.emit(&mut b_)"), "{out}");
    assert!(out.contains("b_.write(\" two\")"), "{out}");
    Ok(())
}

/// Splitting is a parse-side concern, so the surface source still round-trips:
/// `coparse` has to put the token back together exactly as written.
#[test]
fn a_split_token_round_trips() -> Result<()> {
    for code in [
        "const X: T = bash↖echo \"hi ↙x↘ there\"↗;\n",
        "const X: T = bash↖systemctl start ↙x↘.service↗;\n",
        "const X: T = py↖print(\"hi ↙x↘\")↗;\n",
    ] {
        let mut omni = Omni::default();
        let q = omni.parse(code)?;
        assert_eq!(code, q.coparse(), "{code:?} did not round-trip");
    }
    Ok(())
}

/// The guard on the split: a hole that *does* line up with a node exactly must
/// still be claimed by that node, not by the run around it. Traversal is in
/// document order and an exact match is consumed before any run is written, so
/// these keep the tags they always had — `word` for a bare shell argument,
/// Rust's own node for an expression hole.
#[test]
fn an_exact_hole_still_binds_to_its_own_node() -> Result<()> {
    let out = host_expand("bash", "echo bash↖systemctl start ↙$u↘↗\n")?;
    assert_eq!(out, "echo \"systemctl start $u\"\n");

    // A Rust expression hole builds the spliced term directly as the child,
    // with no surrounding token text to write.
    let out = expand("const X: T = ↖1 + ↙x↘↗;\n")?;
    assert!(out.contains("tb(\"binary_expression\")"), "{out}");
    assert!(out.contains(".c(&x)"), "{out}");
    Ok(())
}
