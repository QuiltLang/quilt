use indoc::indoc;
use pretty_assertions::assert_eq;
use quilt::lang::{one_liner, Language};
use quilt::langs::omni::Omni;
use quilt::langs::python::lang::PythonLanguage;
use quilt::prelude::*;
use quilt::term::STerm;

/**************************************************************/

fn roundtrip(code: &str) -> Result<()> {
    roundtrip_lang("rs", code)
}

fn roundtrip_lang(lang: &str, code: &str) -> Result<()> {
    let code = code.trim();
    let mut omni = Omni::default();
    let term = omni.parse_lang(lang, code)?;
    // dbg!(&term);
    // dbg!(&term[0][5]);
    let code2 = term.coparse();
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
