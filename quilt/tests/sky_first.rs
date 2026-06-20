//! Sky-first parsing (issue #86): a whole file parsed as the body of one
//! `lang↖ … ↗`, with `↙…↘` holes left as free variables.

use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::multi::template_params;
use quilt::prelude::*;
use quilt::term::STerm;

/// The wrapper a sky-first parse returns is an index-1 `Quote` in the target
/// language — the implicit `target↖ … ↗` the file sits inside.
#[test]
fn sky_first_is_a_quote_in_the_target_lang() -> Result<()> {
    let mut omni = Omni::default();
    let src = indoc! {r"
        def greet():
            print(↙greeting↘)
    "};
    let t = omni.parse_sky("py", src)?;
    let QTerm::Quote { lang, index, .. } = &*t else {
        panic!("expected a Quote, got {t:?}");
    };
    assert_eq!(&**lang, "py");
    assert_eq!(*index, 1);
    Ok(())
}

/// Free unquote variables surface as the template's parameters, in order and
/// de-duplicated.
#[test]
fn holes_surface_as_params() -> Result<()> {
    let mut omni = Omni::default();
    let src = "print(↙greeting↘ + ↙name↘ + ↙greeting↘)\n";
    let t = omni.parse_sky("py", src)?;
    assert_eq!(
        template_params(&t),
        vec!["greeting".into(), "name".into()] as Vec<Box<str>>
    );
    Ok(())
}

/// No expansion happens: the wrapper carries no `↖`/`↗` markers, so coparsing
/// it reproduces the body with the holes still spelled `↙name↘`.
#[test]
fn coparse_reproduces_body_with_holes() -> Result<()> {
    let mut omni = Omni::default();
    let src = "x = ↙value↘\n";
    let t = omni.parse_sky("py", src)?;
    let out = t.coparse();
    assert!(out.contains("x ="), "got: {out:?}");
    assert!(out.contains("↙value↘"), "got: {out:?}");
    assert!(
        !out.contains('↖'),
        "wrapper should carry no quote marker: {out:?}"
    );
    Ok(())
}

/// A two-extension chain templates the *embedded* language (the leftmost
/// extension) with the host below it, so a `↙rust_expr↘` hole still pops to the
/// host — i.e. the same file parses sky-first without an "unquote depth" error
/// that a host-less zipper would raise.
#[test]
fn two_lang_chain_targets_embedded_lang() -> Result<()> {
    let mut omni = Omni::default();
    let src = "let x = ↙n↘;\n";
    let t = omni.parse_template(&["rs", "wgsl"], src)?;
    let QTerm::Quote { lang, .. } = &*t else {
        panic!("expected a Quote, got {t:?}");
    };
    assert_eq!(&**lang, "wgsl");
    assert_eq!(template_params(&t), vec!["n".into()] as Vec<Box<str>>);
    Ok(())
}
