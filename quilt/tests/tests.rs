use indoc::indoc;
use pretty_assertions::assert_eq;
use quilt::lang::{one_liner, Language};
use quilt::langs::omni::Omni;
use quilt::langs::python::lang::PythonLanguage;
use quilt::prelude::*;
use quilt::term::STerm;

fn expand(lang: &str, code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let term = omni.parse_lang(lang, code.trim())?;
    let expanded = omni.expand_lang(lang, &term)?;
    Ok(expanded.coparse())
}

/**************************************************************/

fn roundtrip(code: &str) -> Result<()> {
    roundtrip_lang("rs", code)
}

/// Note `coparse_quilt`, not `coparse`: this asserts the term renders back as
/// the `.quilt` *source* it was parsed from, which is the reading in which a
/// glyph in content carries a `\` (#223). `coparse` is the other reading — the
/// code the term generates — and the two differ only on escapes.
fn roundtrip_lang(lang: &str, code: &str) -> Result<()> {
    let code = code.trim();
    let mut omni = Omni::default();
    let term = omni.parse_lang(lang, code)?;
    // dbg!(&term);
    // dbg!(&term[0][5]);
    let code2 = term.coparse_quilt();
    // println!("'{code}'");
    // println!("'{code2}'");
    assert_eq!(code, code2);
    Ok(())
}

#[test]
fn rust_hello() -> Result<()> {
    roundtrip(indoc! {r#"
        fn hello() {
            println!("Hello, world!");
        }
    "#})
}

#[test]
fn python_hello() -> Result<()> {
    roundtrip_lang(
        "py",
        indoc! {r#"
            def hello():
                print("Hello, world!")
        "#},
    )
}

#[test]
fn rs_py() -> Result<()> {
    roundtrip(indoc! {r#"
        fn hello() {
            let code = py↖
                def hello():
                    print("Hello, world!")
            ↗;
            println!(code);
        }
    "#})
}

#[test]
fn rs_py_minimal() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖1↗;
    "#})
}

#[test]
fn squash() -> Result<()> {
    let rs = "def f(): pass";
    let mut py = PythonLanguage::default();
    let qterm = py.parse_file(&one_liner(rs))?;

    let block = qterm[4].clone();
    dbg!(&block);
    assert_eq!(block.coparse(), "pass");
    let squashed = block.squash();
    assert_eq!(squashed.coparse(), "pass");
    Ok(())
}

#[test]
fn whitespace_ownership() -> Result<()> {
    let rs = indoc! {r#"
        const X: T = py↖ 1 ↗;
    "#};
    let mut omni = Omni::default();
    let term = omni.parse(rs)?;
    assert_eq!(rs, term.coparse());
    assert_eq!("py↖ 1 ↗", term[5].coparse());
    assert_eq!(" 1 ", term[5][0].coparse());
    Ok(())
}

#[test]
fn rs_py_multiline() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
            123
        ↗;
    "#})
}

#[test]
fn rs_py_multiline_no_indent() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖
        123
        ↗;
    "#})
}

#[test]
fn expr() -> Result<()> {
    roundtrip_lang("py", "1 ")
}

#[test]
fn rs_empty() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = ↖↗;
    "#})
}

#[test]
fn rs_empty_nl() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = ↖
        ↗;
    "#})
}

#[test]
fn rs_empty_nl_2() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = ↖
        
        ↗;
    "#})
}

#[test]
fn rs_py_empty() -> Result<()> {
    roundtrip(indoc! {r#"
        const X: T = py↖123
        ↗;
    "#})
}

#[test]
fn rs_stmt() -> Result<()> {
    roundtrip(indoc! {r#"
        let code = ↖println!("Hello, world!");↗;
    "#})
}

#[test]
fn rs_var() -> Result<()> {
    roundtrip(indoc! {r#"
        let code = ↖let ↙foo↘ = "bar";↗;
    "#})
}

#[test]
fn reduce_expands_homogeneous() -> Result<()> {
    // ↓ at ground level in Rust expands to `.reduce()` in the coparse output
    let out = expand(
        "rs",
        indoc! {r#"
        fn main() {
            let result = program.↓;
        }
    "#},
    )?;
    assert_eq!(
        out.trim(),
        "fn main() {\n    let result = program.reduce();\n}"
    );
    Ok(())
}

#[test]
fn hetero_reduce_py_coparse() -> Result<()> {
    // py↓ at ground level in Rust expands to `.reduce_py()` in the coparse output
    let out = expand(
        "rs",
        indoc! {r#"
        fn main() {
            let result = program.py↓;
        }
    "#},
    )?;
    assert_eq!(
        out.trim(),
        "fn main() {\n    let result = program.reduce_py();\n}"
    );
    Ok(())
}

#[test]
fn hetero_reduce_py_expands_to_reduce_py() -> Result<()> {
    let out = expand(
        "rs",
        indoc! {r#"
        fn main() {
            let result = program.py↓;
        }
    "#},
    )?;
    assert!(
        out.contains("reduce_py()"),
        "expected `reduce_py()` in output, got: {out}"
    );
    Ok(())
}

#[test]
fn homo_reduce_expands_to_reduce() -> Result<()> {
    let out = expand(
        "rs",
        indoc! {r#"
        fn main() {
            let result = program.↓;
        }
    "#},
    )?;
    assert!(
        out.contains("reduce()"),
        "expected `reduce()` in output, got: {out}"
    );
    assert!(
        !out.contains("reduce_py()"),
        "should not contain `reduce_py()`, got: {out}"
    );
    Ok(())
}

#[test]
fn py_homo_reduce_expands_to_reduce() -> Result<()> {
    let out = expand("py", "result = program.↓")?;
    assert!(
        out.contains("reduce()"),
        "expected `reduce()` in py output, got: {out}"
    );
    Ok(())
}

#[test]
fn py_hetero_reduce_rs_expands_to_reduce_rs() -> Result<()> {
    let out = expand("py", "result = program.rs↓")?;
    assert!(
        out.contains("reduce_rs()"),
        "expected `reduce_rs()` in py output, got: {out}"
    );
    Ok(())
}

/// A multi-line `/* … */` plain comment reaches the language parser as a single
/// `FlatNode::Str` with raw newlines in it. The line table that `Point`s index
/// used to be built by appending that whole string to the current line, so a
/// comment spanning N lines left the table N-1 rows short of what tree-sitter
/// saw — which made an ordinary file fail to parse, and, when a later node's
/// row ran off the end, panic with an index out of bounds.
///
/// Found by `bin/fuzz` (issue #161); the crashing input is kept as
/// `fuzz/seeds/multiline_block_comment`.
#[test]
fn multiline_plain_block_comment_round_trips() -> Result<()> {
    roundtrip(indoc! {r#"
        /* a
           b */
        fn f() {
            println!("hi");
        }
    "#})
}

/// The same, with the comment indented inside a block: the continuation line
/// carries a prefix, which is the case the row bookkeeping feeds into.
#[test]
fn indented_multiline_plain_block_comment_round_trips() -> Result<()> {
    roundtrip(indoc! {r#"
        fn f() {
            /* one
               two
               three */
            let x = 1;
        }
    "#})
}

/* --------------------- escaped glyphs round-trip (#223) ------------------- */

/// Every glyph, escaped, at ground level: a `.quilt` file that spells a glyph
/// `\↑` must come back spelling it `\↑`. `coparse` — the *output* reading —
/// deliberately writes it bare, which is why the round trip needs its own
/// renderer rather than a fix to the one the expander uses.
#[test]
fn every_escaped_glyph_round_trips_at_ground_level() -> Result<()> {
    for g in quilt::glyphs::GLYPHS {
        roundtrip(&format!("let s = \"\\{g}\";"))?;
    }
    Ok(())
}

/// …and inside a quote, where the fragment is a different language's code. The
/// brackets (`↖ ↗ ↙ ↘ ⟨ ⟩`) are unambiguous anywhere: none of them is ever
/// deferred to a later stage, so a bare one in a term can only have come from
/// an escape.
#[test]
fn escaped_brackets_round_trip_inside_a_quote() -> Result<()> {
    for g in ['↖', '↗', '↙', '↘', '⟨', '⟩'] {
        roundtrip(&format!("let q = rs↖let s = \"\\{g}\";↗;"))?;
    }
    Ok(())
}

/// The other direction, and the reason the renderer cannot simply escape every
/// glyph it sees: `↑ ↓ ←` inside an unresolved quote are *deferred operators*,
/// left for the next stage, and must reach the output bare. Escaping one would
/// silently turn a staged program into a literal.
#[test]
fn deferred_operators_are_not_escaped() -> Result<()> {
    roundtrip("let q = rs↖let y = ↑;↗;")?;
    roundtrip("let q = rs↖let y = ↓;↗;")?;
    roundtrip("let q = rs↖let y = py↓;↗;")?;
    roundtrip("let q = rs↖fn f() { ←; }↗;")?;
    roundtrip("let q = lean↖def n := ↑↗;")?;
    // Two stages deep, where the inner glyph is deferred twice over.
    roundtrip("let q = rs↖let z = rs↖let y = ↑;↗;↗;")
}

/// The residual, pinned so it is a known shape rather than a surprise: an
/// *operator* glyph escaped inside a quote, in a position where the target
/// grammar makes it a leaf of its own — here a Rust string's contents. Such a
/// leaf is indistinguishable from the deferred-operator placeholder the
/// expander plugs in (both are a single-glyph leaf with a named tag), and the
/// renderer resolves the ambiguity in favour of the operator, because escaping
/// a real deferred operator would break staging outright while this loses an
/// escape that was already lost before #223.
///
/// Telling the two apart needs the placeholder to be marked in the term, which
/// is a representation change — see the discussion on #223.
#[test]
fn an_escaped_operator_inside_a_quoted_string_is_the_known_residual() -> Result<()> {
    let mut omni = Omni::default();
    let src = "let q = rs↖let s = \"\\↑\";↗;";
    let back = omni.parse(src)?.coparse_quilt();
    assert_eq!(
        back, "let q = rs↖let s = \"↑\";↗;",
        "if this now round-trips, the residual on #223 is closed — replace this \
         test with `roundtrip({src:?})`"
    );
    Ok(())
}

/// A `//` line comment must not swallow the bracket that closes the quote it
/// sits in (issue #226). The comment token runs to end of line, so before the
/// fix the `↗` was comment text and the quote was never closed — a `malformed
/// Quilt syntax` error on source that reads as obviously valid, with the
/// workaround (a newline before the closer) discoverable only by accident.
///
/// The same applies to `↘`, and to Quilt's own `⟨//⟩` comment.
#[test]
fn a_line_comment_does_not_eat_the_closing_bracket() -> Result<()> {
    roundtrip("let b = rs↖let x = 1; // hi↗;")?;
    roundtrip("let u = rs↖let y = ↙f()↘; // hi↗;")?;
    // The closer of an *unquote* is bounded too.
    roundtrip("let n = rs↖let y = ↙f() // why↘;↗;")?;

    // Quilt's own comment is meta-level, so it is stripped rather than passed
    // through — `roundtrip` is the wrong assertion for it. What it shares with
    // `//` is that it must not eat the closer.
    let mut omni = Omni::default();
    let out = omni
        .parse_lang("rs", "let q = rs↖let x = 1; ⟨//⟩ note↗;")?
        .coparse();
    assert_eq!(out, "let q = rs↖let x = 1; ↗;");
    Ok(())
}

/// …while a comment at *ground* level still runs to end of line and carries
/// every glyph, closing arrows included. That is the half the #226 fix must not
/// break, and it is not hypothetical: four files under `examples/` explain
/// Quilt in a `//` comment and name the `↙…↘` brackets while doing it, so
/// bounding the token everywhere turned them all into parse errors.
#[test]
fn a_ground_level_line_comment_still_reaches_end_of_line() -> Result<()> {
    let mut omni = Omni::default();
    roundtrip(indoc! {r"
        // prose about the ↙…↘ hole and the ↖…↗ brackets, with ↑ ↓ ← for good measure
        let x = 1;"})?;

    let out = omni
        .parse_lang("rs", "⟨//⟩ a quilt comment about ↙…↘\nlet x = 1;")?
        .coparse();
    assert!(
        !out.contains("quilt comment"),
        "a `⟨//⟩` comment is stripped, not passed through: {out:?}"
    );
    assert!(
        out.contains("let x = 1;"),
        "the code after the comment survived: {out:?}"
    );
    Ok(())
}

/// A glyph inside a `//` or `/* … */` comment is raw text to Quilt — the
/// comment token consumes it, so a `\` there is a literal backslash and not an
/// escape — and the source renderer has to leave it alone (#237). Getting this
/// wrong was invisible until #229 put a comment full of brackets in a test:
/// `coparse_quilt` escaped them, and re-parsing gave a comment with stray
/// backslashes in it.
#[test]
fn a_c_style_comment_is_raw_and_is_not_escaped() -> Result<()> {
    roundtrip("// prose about ↙…↘ and ↑ ↓ ←\nlet x = 1;")?;
    roundtrip("/* block ↙…↘ with ⟨ ⟩ */\nlet x = 1;")?;
    roundtrip("let q = rs↖let x = 1; // hi ↑↗;")?;
    roundtrip_lang("ts", "// ts comment ↙…↘\nconst x = 1;")
}

/// The other half, and the reason the test is on the comment's *spelling* and
/// not only on its tag: Quilt's grammar has `//` and `/* … */` and nothing
/// else, so a `#` or `--` comment is ordinary content to it. A `↑` written in
/// a Python comment had to be spelled `\↑` in the source, and rendering it
/// back bare would turn it into a lift operator. Suppressing escapes for every
/// node a language calls a comment would fix the C-style languages and break
/// these.
#[test]
fn a_hash_or_dash_comment_is_not_raw_and_keeps_its_escapes() -> Result<()> {
    roundtrip_lang("py", "# python comment with \\↙…\\↘ and \\↑\nx = 1")?;
    roundtrip_lang("nix", "{ }  # nix comment \\↑")?;
    roundtrip_lang("bash", "# shell comment \\↓\necho hi")
}
