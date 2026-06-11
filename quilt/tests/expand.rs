use indoc::indoc;
use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::term::STerm;

/**************************************************************/

#[test]
fn expand() -> Result<()> {
    let mut omni = Omni::default();
    let code = indoc! {r#"
        fn hello(s: &Arc<QTerm>) {
            let code = py↖
                def hello():
                    print(↙s↘)
            ↗;
            dbg!(code);
        }
    "#};
    let qterm = omni.parse(code)?;
    let expanded = omni.expand(&qterm)?;
    // dbg!(&expanded);
    println!("{}", expanded.coparse());
    // TODO: test the expanded code
    Ok(())
}

/// Assert that `err` is an "unquote depth too high" diagnostic with a single
/// label pointing at `code[start..end]`.
fn assert_depth_error_at(err: &miette::Report, start: usize, end: usize) {
    assert!(err.to_string().contains("unquote depth too high"));
    assert!(err.to_string().contains(&format!("bytes {start}..{end}")));
    let labels: Vec<_> = err.labels().expect("error should carry a label").collect();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].offset(), start);
    assert_eq!(labels[0].len(), end - start);
}

/// An unquote with no enclosing quote used to panic deep in `build_nodes`
/// (empty zipper); now it's a depth error pointing at the offending `↙…↘`.
#[test]
fn unquote_outside_quote_error_has_span() {
    let mut omni = Omni::default();
    let code = indoc! {r"
        fn hello() {
            let y = ↙y↘;
        }
    "};
    let err = omni.parse(code).unwrap_err();

    let start = code.find('↙').unwrap();
    let end = code.find('↘').unwrap() + "↘".len();
    assert_depth_error_at(&err, start, end);
}

/// The expander's depth check reports the span carried by the unquote term.
/// Surface syntax can't produce an over-deep unquote past the parser (it's
/// caught there), so build one directly: an index-2 unquote inside an
/// index-1 quote.
#[test]
fn expander_depth_error_has_span() {
    let mut omni = Omni::default();
    let span = 10..17;
    let mut inner = ub("expression_statement", 2, "rs");
    inner.span(span.clone());
    let inner = inner.c(&sym("y")).b();
    let qterm = qb("expression_statement", 1, "rs").c(&inner).b();

    let err = omni.expand(&qterm).unwrap_err();
    assert_depth_error_at(&err, span.start, span.end);
}
