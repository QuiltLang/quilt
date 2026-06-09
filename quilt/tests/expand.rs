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
