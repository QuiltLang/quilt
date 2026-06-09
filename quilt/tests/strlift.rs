use indoc::indoc;
use quilt::{
    lang::{one_liner, Language},
    langs::{bootstrap::strlift::StrLift, omni::Omni, python::lang::PythonLanguage},
    prelude::*,
    term::STerm,
};
// use quilt::langs::bootstrap::strlift

/**************************************************************/

#[test]
fn strlift() -> Result<()> {
    let mut py = PythonLanguage::default();
    let code = "1 + 2";
    // println!("{code}");
    let qterm = py.parse(&one_liner(code))?;
    // dbg!(&qterm);
    let lifted = qterm.strlift();
    println!("{lifted}");
    assert_eq!(
        lifted,
        r#"tb("binary_operator").c(&leaf("integer", "1")).w(" ").c(&sym("+")).w(" ").c(&leaf("integer", "2")).b()"#
    );
    let reduced = tuple(
        "binary_operator",
        &[leaf("integer", "1"), sym("+"), leaf("integer", "2")],
        &[HOLE, cmd(write(" ")), HOLE, cmd(write(" ")), HOLE],
    );
    // dbg!(&reduced);
    assert_eq!(reduced, qterm);
    let coparsed = reduced.coparse();
    // println!("{coparsed}");
    assert_eq!(coparsed, code);
    Ok(())
}

#[test]
fn strlift_2() -> Result<()> {
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
    // println!("{code}");
    let qterm = omni.parse(code)?;
    // dbg!(&qterm);
    let lifted = qterm.strlift();
    println!("{lifted}");
    assert_eq!(
        lifted,
        r#"tb("function_item").c(&sym("fn")).w(" ").c(&leaf("identifier", "hello")).c(&tb("parameters").c(&sym("(")).c(&tb("parameter").c(&leaf("identifier", "s")).c(&sym(":")).w(" ").c(&tb("reference_type").c(&sym("&")).c(&tb("generic_type").c(&leaf("type_identifier", "Arc")).c(&tb("type_arguments").c(&sym("<")).c(&leaf("type_identifier", "QTerm")).c(&sym(">")).b()).b()).b()).b()).c(&sym(")")).b()).w(" ").c(&tb("block").c(&sym("{")).p("    ").n().c(&tb("let_declaration").c(&sym("let")).w(" ").c(&leaf("identifier", "code")).w(" ").c(&sym("=")).w(" ").c(&quote("block", 1, "py", tb("function_definition").c(&sym("def")).w(" ").c(&leaf("identifier", "hello")).c(&tb("parameters").c(&sym("(")).c(&sym(")")).b()).c(&sym(":")).p("    ").n().c(&tb("block").c(&tb("expression_statement").c(&tb("call").c(&leaf("identifier", "print")).c(&tb("argument_list").c(&sym("(")).c(&unquote("identifier", 1, "py", leaf("identifier", "s"), &[cmd(write("↙")), HOLE, cmd(write("↘"))])).c(&sym(")")).b()).b()).b()).b()).x().b(), &[cmd(write("py")), cmd(write("↖")), cmd(push("    ")), cmd(NL), HOLE, cmd(POP), cmd(NL), cmd(write("↗"))])).c(&sym(";")).b()).n().c(&tb("expression_statement").c(&tb("macro_invocation").c(&leaf("identifier", "dbg")).c(&sym("!")).c(&tb("token_tree").c(&sym("(")).c(&leaf("identifier", "code")).c(&sym(")")).b()).b()).c(&sym(";")).b()).x().n().c(&sym("}")).b()).n().b()"#
    );
    // this is the contents of the string above:
    let reduced = tb("function_item")
        .c(&sym("fn"))
        .w(" ")
        .c(&leaf("identifier", "hello"))
        .c(&tb("parameters")
            .c(&sym("("))
            .c(&tb("parameter")
                .c(&leaf("identifier", "s"))
                .c(&sym(":"))
                .w(" ")
                .c(&tb("reference_type")
                    .c(&sym("&"))
                    .c(&tb("generic_type")
                        .c(&leaf("type_identifier", "Arc"))
                        .c(&tb("type_arguments")
                            .c(&sym("<"))
                            .c(&leaf("type_identifier", "QTerm"))
                            .c(&sym(">"))
                            .b())
                        .b())
                    .b())
                .b())
            .c(&sym(")"))
            .b())
        .w(" ")
        .c(&tb("block")
            .c(&sym("{"))
            .p("    ")
            .n()
            .c(&tb("let_declaration")
                .c(&sym("let"))
                .w(" ")
                .c(&leaf("identifier", "code"))
                .w(" ")
                .c(&sym("="))
                .w(" ")
                .c(&quote(
                    "block",
                    1,
                    "py",
                    tb("function_definition")
                        .c(&sym("def"))
                        .w(" ")
                        .c(&leaf("identifier", "hello"))
                        .c(&tb("parameters").c(&sym("(")).c(&sym(")")).b())
                        .c(&sym(":"))
                        .p("    ")
                        .n()
                        .c(&tb("block")
                            .c(&tb("expression_statement")
                                .c(&tb("call")
                                    .c(&leaf("identifier", "print"))
                                    .c(&tb("argument_list")
                                        .c(&sym("("))
                                        .c(&unquote(
                                            "identifier",
                                            1,
                                            "py",
                                            leaf("identifier", "s"),
                                            &[cmd(write("↙")), HOLE, cmd(write("↘"))],
                                        ))
                                        .c(&sym(")"))
                                        .b())
                                    .b())
                                .b())
                            .b())
                        .x()
                        .b(),
                    &[
                        cmd(write("py")),
                        cmd(write("↖")),
                        cmd(push("    ")),
                        cmd(NL),
                        HOLE,
                        cmd(POP),
                        cmd(NL),
                        cmd(write("↗")),
                    ],
                ))
                .c(&sym(";"))
                .b())
            .n()
            .c(&tb("expression_statement")
                .c(&tb("macro_invocation")
                    .c(&leaf("identifier", "dbg"))
                    .c(&sym("!"))
                    .c(&tb("token_tree")
                        .c(&sym("("))
                        .c(&leaf("identifier", "code"))
                        .c(&sym(")"))
                        .b())
                    .b())
                .c(&sym(";"))
                .b())
            .x()
            .n()
            .c(&sym("}"))
            .b())
        .n()
        .b();
    // dbg!(&reduced);
    assert_eq!(reduced, qterm);
    let coparsed = reduced.coparse();
    // println!("{coparsed}");
    assert_eq!(coparsed, code);
    Ok(())
}
