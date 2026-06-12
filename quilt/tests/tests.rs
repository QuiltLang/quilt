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
