//! Quote representations: `ts⟨str⟩↖…↗`, `ts⟨qt⟩↖…↗`, and the default
//! `ts⟨cst⟩↖…↗` (see `quilt::qterm::Repr`). Which representation a quote has,
//! how it is inherited, and what a TypeScript host expands each to. That all
//! three build the same code at run time is checked against the runtime in
//! `quilt-wasm/test/repr.mjs`.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::qterm::Repr;
use quilt::term::STerm;

fn parse(chain: &[&str], code: &str) -> Result<Arc<QTerm>> {
    Omni::default().parse_chain(chain, code)
}

fn expand(chain: &[&str], code: &str) -> Result<String> {
    let mut omni = Omni::default();
    let q = omni.parse_chain(chain, code)?;
    Ok(omni.expand_lang(chain[0], &q)?.coparse())
}

/// Every quote in `term`, outermost first, as (language, representation).
fn quotes(term: &QTerm, out: &mut Vec<(String, Repr)>) {
    match term {
        QTerm::Quote {
            lang, term, repr, ..
        } => {
            out.push((lang.to_string(), *repr));
            quotes(term, out);
        }
        QTerm::Unquote { term, .. } => quotes(term, out),
        QTerm::Tuple { terms, .. } => terms.iter().for_each(|t| quotes(t, out)),
    }
}

fn reprs(chain: &[&str], code: &str) -> Result<Vec<(String, Repr)>> {
    let mut out = Vec::new();
    quotes(&*parse(chain, code)?, &mut out);
    Ok(out)
}

fn q(lang: &str, repr: Repr) -> (String, Repr) {
    (lang.to_string(), repr)
}

/// The annotation is source: it survives parsing as written, so a later stage
/// that re-parses a coparsed quote sees the same representation.
#[test]
fn a_representation_round_trips() -> Result<()> {
    for code in [
        "const a = ts⟨str⟩↖1 + ↙x↘↗;\n",
        "const a = ⟨qt⟩↖[↙x↘]↗;\n",
        "const a = ts⟨cst⟩↖f(↙ts⟨str⟩↖y↗↘)↗;\n",
    ] {
        assert_eq!(code, parse(&["ts"], code)?.coparse_quilt());
    }
    Ok(())
}

/// Inherited like a language: by the quotes after it at the same level, while
/// they are in its language; `⟨cst⟩` takes it back, and `⟨str⟩` alone keeps
/// the inherited language.
#[test]
fn a_representation_is_inherited_by_siblings_of_its_language() -> Result<()> {
    let code = indoc! {r#"
        const a = ts⟨str⟩↖1↗;
        const b = ↖2↗;
        const c = ts↖3↗;
        const d = py↖4↗;
        const e = ↖5↗;
        const f = ts↖6↗;
        const g = ⟨cst⟩↖7↗;
        const h = ↖8↗;
        const i = ⟨qt⟩↖9↗;
    "#};
    assert_eq!(
        reprs(&["ts"], code)?,
        [
            q("ts", Repr::Str),
            q("ts", Repr::Str),
            q("ts", Repr::Str),
            q("py", Repr::Cst),
            q("py", Repr::Cst),
            q("ts", Repr::Cst),
            q("ts", Repr::Cst),
            q("ts", Repr::Cst),
            q("ts", Repr::Qt),
        ]
    );
    Ok(())
}

/// And by the quotes inside its unquotes, in its language.
#[test]
fn a_representation_reaches_into_its_unquotes() -> Result<()> {
    let code = "const a = ts⟨str⟩↖f(↙x ?? ↖true↗↘, ↙py↖y↗↘)↗;\n";
    assert_eq!(
        reprs(&["ts"], code)?,
        [q("ts", Repr::Str), q("ts", Repr::Str), q("py", Repr::Cst)]
    );
    Ok(())
}

#[test]
fn an_unknown_representation_is_an_error() {
    let msg = format!(
        "{:?}",
        parse(&["ts"], "const a = ts⟨tree⟩↖1↗;\n").unwrap_err()
    );
    assert!(
        msg.contains("unknown quote representation `⟨tree⟩`"),
        "{msg}"
    );
}

/// `⟨str⟩` is one template literal: `${…}` per hole, `\n` plus the pushed
/// prefix per layout newline, and a hole under a prefix re-indented with
/// `replaceAll`. `↑` into TypeScript is `qlift_str`, `←` an inline join.
#[test]
fn ts_str_is_a_template_literal() -> Result<()> {
    insta::assert_snapshot!(expand(
        &["ts"],
        indoc! {r#"
            const out = ts⟨str⟩↖
              // ↙title↘: `quoted` and ${not a hole}
              F[↙i↘] = {
                test: (a) => ↙test↘,
                name: ↙↑(name)↘,
              };
              ↙←(rest)↘
            ↗;
        "#}
    )?);
    Ok(())
}

/// `⟨qt⟩` is one flat builder chain: a write per run of text, `.n()`/`.p()`/
/// `.x()` for layout, `.c(x)` per hole. `↑` and `←` build terms as in `⟨cst⟩`.
#[test]
fn ts_qt_is_a_flat_builder_chain() -> Result<()> {
    insta::assert_snapshot!(expand(
        &["ts"],
        indoc! {r#"
            const out = ts⟨qt⟩↖
              F[↙i↘] = {
                name: ↙↑(name)↘,
              };
              ↙←(rest)↘
            ↗;
        "#}
    )?);
    Ok(())
}

/// Only TypeScript implements `⟨str⟩` and `⟨qt⟩` so far; elsewhere they are
/// an error at the quote, not a silent `⟨cst⟩`.
#[test]
fn a_host_without_a_representation_says_so() {
    let msg = format!(
        "{:?}",
        expand(&["rs"], "fn f() { let a = rs⟨str⟩↖1 + 2↗; }\n").unwrap_err()
    );
    assert!(msg.contains("this host has no ⟨str⟩ quotes yet"), "{msg}");
    let msg = format!("{:?}", expand(&["py"], "a = py⟨qt⟩↖1 + 2↗\n").unwrap_err());
    assert!(msg.contains("this host has no ⟨qt⟩ quotes yet"), "{msg}");
}

/// A flat representation is a value, not a later stage: a quote nested
/// directly inside one (rather than inside its unquote) is refused.
#[test]
fn a_flat_quote_does_not_stage() {
    let msg = format!(
        "{:?}",
        expand(&["ts"], "const a = ts⟨str⟩↖f(↖x↗)↗;\n").unwrap_err()
    );
    assert!(msg.contains("can't hold a nested quote yet"), "{msg}");
}
