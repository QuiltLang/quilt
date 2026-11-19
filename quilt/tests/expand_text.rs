//! Lifting into plain text (`txt↖…↗`), issue #149.
//!
//! Text is the one target with no grammar, so two things a grammar would
//! decide are decided here instead, and these tests pin both:
//!
//! * **Spelling** comes from a dialect (`quilt::lift::text`). Plain `↑` is the
//!   default, JSON's, from every host that can quote text.
//! * **Position** comes from the characters around the hole. Between two `"`
//!   the author wrote, `↑` is spelled for the inside of a string, so a string
//!   renders as its escaped contents rather than a second pair of quotes.
//!
//! The last test runs the *generated* `examples/config_gen.rs` — the YAML,
//! Caddyfile and DSL generator — and checks what it writes: the end-to-end
//! claim that a config generator needs no escaping helpers of its own.

use quilt::langs::omni::Omni;
use quilt::prelude::*;

/// Parse + expand `code` under `chain`, returning the coparsed host source.
fn expand(chain: &[&str], code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(chain, code)?;
    Ok(omni.expand_lang(chain[0], &q)?.coparse())
}

/**************************************************************/
// Spellings: what `↑` into a text quote becomes in each host.

#[test]
fn rust_lifts_into_text_through_the_text_marker() -> Result<()> {
    let out = expand(&["rs"], "let t = txt↖port: ↙p.↑↘↗;\n")?;
    assert!(out.contains("p.qlift_to::<Text>()"), "{out}");
    // `text` is the same language under its long name.
    let out = expand(&["rs"], "let t = text↖port: ↙p.↑↘↗;\n")?;
    assert!(out.contains("p.qlift_to::<Text>()"), "{out}");
    Ok(())
}

#[test]
fn a_hole_between_quotes_is_spelled_for_the_inside_of_a_string() -> Result<()> {
    let out = expand(&["rs"], "let t = txt↖name: \"↙n.↑↘\"↗;\n")?;
    assert!(out.contains("n.qlift_to::<InQuotes<Text>>()"), "{out}");
    Ok(())
}

/// The position is read off *both* neighbours: a quote on one side only is a
/// quote character in the text, not a string the hole is inside.
#[test]
fn one_quote_is_not_a_quoted_position() -> Result<()> {
    for code in [
        "let t = txt↖a \"↙n.↑↘ b↗;\n",
        "let t = txt↖a ↙n.↑↘\" b↗;\n",
        "let t = txt↖↙n.↑↘↗;\n",
    ] {
        let out = expand(&["rs"], code)?;
        assert!(out.contains("n.qlift_to::<Text>()"), "{code} => {out}");
        assert!(!out.contains("InQuotes"), "{code} => {out}");
    }
    Ok(())
}

/// An escaped quote is a quote character in the text: `\"↙x↘\"` writes the
/// two characters `\"` around the value, which is not a string the value is
/// inside.
#[test]
fn an_escaped_quote_does_not_open_a_quoted_position() -> Result<()> {
    let out = expand(&["rs"], "let t = txt↖say \\\"↙n.↑↘\"↗;\n")?;
    assert!(out.contains("n.qlift_to::<Text>()"), "{out}");
    assert!(!out.contains("InQuotes"), "{out}");
    Ok(())
}

/// Only text reads positions off characters. A grammar already knows what a
/// hole inside one of its own string literals is, so the same source into a
/// Python quote keeps the plain Python lift.
#[test]
fn other_targets_ignore_the_surrounding_quotes() -> Result<()> {
    let out = expand(&["rs"], "let t = py↖x = \"↙n.↑↘\"↗;\n")?;
    assert!(out.contains("n.qlift_to::<Python>()"), "{out}");
    Ok(())
}

#[test]
fn python_and_typescript_spell_the_runtime_text_lifts() -> Result<()> {
    let py = expand(&["py"], "t = txt↖a: ↙↑(v)↘ b: \"↙↑(v)↘\"↗\n")?;
    assert!(py.contains("qlift_text(v)"), "{py}");
    assert!(py.contains("qlift_text_in_quotes(v)"), "{py}");
    let ts = expand(&["ts"], "const t = txt↖a: ↙↑(v)↘ b: \"↙↑(v)↘\"↗;\n")?;
    assert!(ts.contains("qlift_text(v)"), "{ts}");
    assert!(ts.contains("qlift_text_in_quotes(v)"), "{ts}");
    Ok(())
}

/// TypeScript has no glob import, so its prelude names every lift the file can
/// call. The target list names languages, not positions, so the in-quotes
/// spelling has to come along with `text` or a quoted hole fails at run time.
#[test]
fn the_typescript_prelude_imports_both_text_lifts() {
    use quilt::langs::typescript::meta::TypeScriptMetaLanguage;
    use quilt::meta::MetaLanguage as _;

    let prelude = TypeScriptMetaLanguage
        .prelude(&["txt"])
        .expect("TypeScript has a prelude");
    assert!(prelude.text.contains("qlift_text,"), "{}", prelude.text);
    assert!(
        prelude.text.contains("qlift_text_in_quotes"),
        "{}",
        prelude.text
    );
}

#[test]
fn rust_and_python_lift_into_typescript_and_html() -> Result<()> {
    let out = expand(&["rs"], "let t = ts↖f(↙x.↑↘)↗;\n")?;
    assert!(out.contains("x.qlift_to::<TypeScript>()"), "{out}");
    let out = expand(&["rs"], "let t = html↖<p>↙x.↑↘</p>↗;\n")?;
    assert!(out.contains("x.qlift_to::<Html>()"), "{out}");
    let out = expand(&["py"], "t = ts↖f(↙↑(x)↘)↗\n")?;
    assert!(out.contains("qlift_ts(x)"), "{out}");
    Ok(())
}

/**************************************************************/
// The generated example, run.

// The committed expansion itself, so this runs exactly what `bin/check-examples`
// diffs. `rustfmt::skip` because it is generated: formatting it here would make
// it drift from what the expander writes.
#[path = "../../examples/config_gen.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
mod config_gen;

#[test]
fn the_config_generator_writes_valid_yaml_caddy_and_rules() {
    let all = config_gen::services();
    let admin = &all[1];

    // YAML: JSON scalars and flow sequences, so the quote and the newline in
    // the values stay inside their scalars. Checked against a YAML parser
    // when the example was written; pinned as text here.
    assert_eq!(
        config_gen::yaml(admin).coparse(),
        indoc::indoc! {r#"
            - name: "O'Brien's \"admin\""
              port: 9000
              public: false
              upstreams: ["unix//run/admin.sock", "localhost:9001"]
              greeting: "say \"hi\"\n# not a comment""#}
    );

    // Caddyfile: bare tokens where they can be, `"…"` with `\"` otherwise,
    // and a backtick token for the value with a backslash in it.
    assert_eq!(
        config_gen::caddy::site(admin).coparse(),
        indoc::indoc! {r#"
            admin.example.com {
                reverse_proxy unix//run/admin.sock localhost:9001
                header X-Service `{"service": "O'Brien's \"admin\""}`
                respond /hello "say \"hi\"
            # not a comment"
            }"#}
    );

    // The DSL: SQL-style strings, `yes`/`no`, comma-separated lists.
    assert_eq!(
        config_gen::rules::rule(admin).coparse(),
        "service 'O''Brien''s \"admin\"' port 9000 public no via \
         'unix//run/admin.sock', 'localhost:9001'"
    );
}
