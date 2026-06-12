//! Direct term-lifting for the Rust meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(&child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. This is the term-valued analogue of
//! `langs::bootstrap::strlift` — same emitted source, but with no string
//! round-trip (no re-parse).

use crate::prelude::*;
use crate::term::CmdOrHole;
use miette::{bail, IntoDiagnostic};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::Command;

/**************************************************************/

/// Render a Rust string literal, escaping `\` and `"` (matches `strlift` for
/// `str`).
fn str_lit(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Render a `StrCmd` as constructor source.
fn strcmd_lit(c: &StrCmd) -> String {
    match c {
        StrCmd::Write(s) => format!("write({})", str_lit(s)),
        StrCmd::NewLine => "NL".to_string(),
        StrCmd::Push(s) => format!("push({})", str_lit(s)),
        StrCmd::Pop => "POP".to_string(),
    }
}

/// Render a `&[CmdOrHole]` as a `[..]` data literal.
fn cmds_lit(cmds: &[CmdOrHole]) -> String {
    let items: Vec<String> = cmds
        .iter()
        .map(|c| match c {
            CmdOrHole::Hole => "HOLE".to_string(),
            CmdOrHole::Cmd(cmd) => format!("cmd({})", strcmd_lit(cmd)),
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/**************************************************************/

/// Build code that reconstructs a tuple: `tb(tag).w(..).c(&child)..b()`, using
/// the `sym`/`leaf` shorthands when possible. `children` are the already-built
/// child expressions spliced at hole positions.
pub fn build_tuple_code(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    // shorthands: a childless node with a single write
    if children.is_empty() && cmds.len() == 1 {
        if let CmdOrHole::Cmd(StrCmd::Write(code)) = &cmds[0] {
            return if tag == &**code {
                leaf("_", &format!("sym({})", str_lit(tag)))
            } else {
                leaf("_", &format!("leaf({}, {})", str_lit(tag), str_lit(code)))
            };
        }
    }
    // full builder chain
    let mut b = tb("_");
    b.write(&format!("tb({})", str_lit(tag)));
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&format!(".w({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.write(".n()");
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.write(&format!(".p({})", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.write(".x()");
            }
            CmdOrHole::Hole => {
                b.write(".c(&");
                b.child(it.next().expect("build_tuple_code: not enough children"));
                b.write(")");
            }
        }
    }
    b.write(".b()");
    b.b()
}

/// Build `quote(tag, index, lang, <term>, &[..cmds..])`, splicing `term`.
pub fn build_quote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write(&format!(
        "quote({}, {}, {}, ",
        str_lit(tag),
        index,
        str_lit(lang)
    ));
    b.child(term);
    b.write(&format!(", &{})", cmds_lit(cmds)));
    b.b()
}

/// Build `unquote(tag, index, lang, <term>, &[..cmds..])`, splicing `term`.
pub fn build_unquote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write(&format!(
        "unquote({}, {}, {}, ",
        str_lit(tag),
        index,
        str_lit(lang)
    ));
    b.child(term);
    b.write(&format!(", &{})", cmds_lit(cmds)));
    b.b()
}

/// Build a variadic node as an imperative block:
/// `{ let mut b_ = tb("tag"); b_.write(..); <child>; ..; b_.b() }`.
/// `children` are already wrapped (see [`wrap_emit`]/[`wrap_splice`]).
pub fn build_variadic_block(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    let mut b = tb("block");
    b.child(&sym("{")).push("    ").nl();
    b.write(&format!("let mut b_ = tb({});", str_lit(tag))).nl();
    let mut it = children.iter();
    for c in cmds {
        match c {
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write(&format!("b_.write({});", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.write("b_.nl();");
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.write(&format!("b_.push({});", str_lit(s)));
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.write("b_.pop();");
            }
            CmdOrHole::Hole => {
                b.child(
                    it.next()
                        .expect("build_variadic_block: not enough children"),
                );
            }
        }
        b.nl();
    }
    b.write("b_.b()");
    b.pop().nl().child(&sym("}"));
    b.b()
}

/// Wrap an expanded child for emission into a variadic block: `<child>.emit(&mut b_);`.
pub fn wrap_emit(child: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb("_");
    b.child(child).write(".emit(&mut b_);");
    b.b()
}

/// Wrap an expanded child spliced as a statement: `<child>;`.
pub fn wrap_splice(child: &Arc<QTerm>) -> Arc<QTerm> {
    let mut b = tb("_");
    b.child(child).write(";");
    b.b()
}

/**************************************************************/

/// Make an identifier term (the `⟨N⟩` operator).
pub fn name(s: &str) -> Arc<QTerm> {
    leaf("identifier", s)
}

/// A Rust string-literal term, structured exactly as the parser (and `↖"s"↗`)
/// produces it, so lifted code can be matched/rewritten as Rust AST (e.g. by
/// `rewrite_naive`). Assumes `s` needs no escaping.
fn strlit_term(s: &str) -> Arc<QTerm> {
    tb("string_literal")
        .c(&sym("\""))
        .c(&leaf("string_content", s))
        .c(&sym("\""))
        .b()
}

/// A `[..]` cmds data literal whose string args are structured `string_literal`s.
fn cmds_lit_term(cmds: &[CmdOrHole]) -> Arc<QTerm> {
    let mut b = tb("_");
    b.write("[");
    for (i, c) in cmds.iter().enumerate() {
        if i > 0 {
            b.write(", ");
        }
        match c {
            CmdOrHole::Hole => {
                b.write("HOLE");
            }
            CmdOrHole::Cmd(StrCmd::Write(s)) => {
                b.write("cmd(write(").child(&strlit_term(s)).write("))");
            }
            CmdOrHole::Cmd(StrCmd::NewLine) => {
                b.write("cmd(NL)");
            }
            CmdOrHole::Cmd(StrCmd::Push(s)) => {
                b.write("cmd(push(").child(&strlit_term(s)).write("))");
            }
            CmdOrHole::Cmd(StrCmd::Pop) => {
                b.write("cmd(POP)");
            }
        }
    }
    b.write("]");
    b.b()
}

/// The Rust spelling of `↑` lifting into the object language `target` (used
/// by the generated `RustMetaLanguage::lift_str`). Homogeneous lifts keep the
/// `qlift()` spelling; heterogeneous ones go through `LiftTo` with the
/// target's marker type (see `crate::lift`).
pub fn lift_spelling(target: &str) -> Result<&'static str> {
    match target {
        "rust" | "rs" => Ok("qlift()"),
        "python" | "py" => Ok("qlift_to::<Python>()"),
        "wgsl" => Ok("qlift_to::<Wgsl>()"),
        "zsh" => Ok("qlift_to::<Zsh>()"),
        "bash" => Ok("qlift_to::<Bash>()"),
        _ => bail!("rust can't lift into {target:?}: no spelling/LiftTo impls registered"),
    }
}

/// The Rust spelling of `↓` reducing with meta-language `target`. The
/// homogeneous case (`target` == `""` or `"rust"`/`"rs"`) keeps `reduce()`;
/// heterogeneous targets invoke the corresponding cross-language reducer.
pub fn reduce_spelling(target: &str) -> Result<&'static str> {
    match target {
        "" | "rust" | "rs" => Ok("reduce()"),
        "python" | "py" => Ok("reduce_py()"),
        _ => bail!("rust can't reduce via {target:?}: no reduce_spelling registered"),
    }
}

/// Evaluate a `QTerm` by running it as Python code, then deserialize the
/// result (the `py↓` operator from a Rust meta-program). The term's code is
/// run via `python3` with the `quilt` Python bindings on `PYTHONPATH`; the
/// result `QTerm` is shuttled back via its postcard serialization.
pub fn reduce_py(x: &QTerm) -> Result<Arc<QTerm>> {
    let input = x.coparse();
    let mut out_file = tempfile::NamedTempFile::new().into_diagnostic()?;
    let out_path = out_file.path().to_str().unwrap();

    let quilt_dir = env!("CARGO_MANIFEST_DIR");
    // The quilt Python package lives next to the quilt crate.
    let py_pkg = format!("{quilt_dir}/../quilt-python");
    let script = indoc::formatdoc! {r#"
        import sys
        sys.path.insert(0, "{py_pkg}")
        from quilt import *
        result = {input}
        data = result.postcard_bytes()
        with open("{out_path}", "wb") as f:
            f.write(data)
    "#};

    let script_file = tempfile::Builder::new()
        .suffix(".py")
        .tempfile()
        .into_diagnostic()?;
    std::fs::write(script_file.path(), script).into_diagnostic()?;
    let status = Command::new("python3")
        .arg(script_file.path())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .into_diagnostic()?;

    if !status.success() {
        bail!("reduce_py: script failed with status {status}");
    }

    let mut data = Vec::new();
    out_file.read_to_end(&mut data).into_diagnostic()?;
    postcard::from_bytes(&data).into_diagnostic()
}

impl QTerm {
    pub fn reduce_py(&self) -> Result<Arc<QTerm>> {
        reduce_py(self)
    }
}

/// Lift a value to a `QTerm` whose code reconstructs it (the `↑` operator).
///
/// Unlike the `build_*_code` helpers (flat, dump-only), the strings here are
/// emitted as structured `string_literal` subterms so the lifted code can be
/// manipulated as Rust AST — the part the bootstrap's parse-based `bs_lift`
/// gets for free and that `rewrite_naive` relies on.
pub trait QLift {
    fn qlift(&self) -> Arc<QTerm>;
}

pub fn qlift<T: QLift + ?Sized>(x: &T) -> Arc<QTerm> {
    x.qlift()
}

/// Homogeneous lifting is the `L = Rust` instance of [`LiftTo`]: anything
/// `QLift` lifts into Rust.
impl<T: QLift + ?Sized> crate::lift::LiftTo<crate::lift::Rust> for T {
    fn lift_to(&self) -> Arc<QTerm> {
        self.qlift()
    }
}

impl QLift for Arc<QTerm> {
    fn qlift(&self) -> Arc<QTerm> {
        match &**self {
            // span is dropped: lifted code rebuilds the term without one
            QTerm::Quote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => tb("_")
                .w("quote(")
                .c(&strlit_term(tag))
                .w(&format!(", {index}, "))
                .c(&strlit_term(lang))
                .w(", ")
                .c(&term.qlift())
                .w(", &")
                .c(&cmds_lit_term(cmds))
                .w(")")
                .b(),
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => tb("_")
                .w("unquote(")
                .c(&strlit_term(tag))
                .w(&format!(", {index}, "))
                .c(&strlit_term(lang))
                .w(", ")
                .c(&term.qlift())
                .w(", &")
                .c(&cmds_lit_term(cmds))
                .w(")")
                .b(),
            QTerm::Tuple { tag, terms, cmds } => {
                // shorthands: a childless node with a single write
                if terms.is_empty() && cmds.len() == 1 {
                    if let CmdOrHole::Cmd(StrCmd::Write(code)) = &cmds[0] {
                        return if **tag == **code {
                            tb("_").w("sym(").c(&strlit_term(tag)).w(")").b()
                        } else {
                            tb("_")
                                .w("leaf(")
                                .c(&strlit_term(tag))
                                .w(", ")
                                .c(&strlit_term(code))
                                .w(")")
                                .b()
                        };
                    }
                }
                // full builder chain
                let mut b = tb("_");
                b.write("tb(").child(&strlit_term(tag)).write(")");
                let mut it = terms.iter();
                for c in cmds {
                    match c {
                        CmdOrHole::Cmd(StrCmd::Write(s)) => {
                            b.write(".w(").child(&strlit_term(s)).write(")");
                        }
                        CmdOrHole::Cmd(StrCmd::NewLine) => {
                            b.write(".n()");
                        }
                        CmdOrHole::Cmd(StrCmd::Push(s)) => {
                            b.write(".p(").child(&strlit_term(s)).write(")");
                        }
                        CmdOrHole::Cmd(StrCmd::Pop) => {
                            b.write(".x()");
                        }
                        CmdOrHole::Hole => {
                            b.write(".c(&")
                                .child(&it.next().expect("qlift: not enough children").qlift())
                                .write(")");
                        }
                    }
                }
                b.write(".b()");
                b.b()
            }
        }
    }
}

impl QLift for str {
    fn qlift(&self) -> Arc<QTerm> {
        strlit_term(self)
    }
}

impl QLift for String {
    fn qlift(&self) -> Arc<QTerm> {
        strlit_term(self)
    }
}

impl QLift for char {
    fn qlift(&self) -> Arc<QTerm> {
        leaf("char_literal", &format!("'{self}'"))
    }
}

macro_rules! qlift_display {
    ($($t:ty),* $(,)?) => {$(
        impl QLift for $t {
            fn qlift(&self) -> Arc<QTerm> {
                leaf("integer_literal", &self.to_string())
            }
        }
    )*};
}
qlift_display!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

/**************************************************************/

/// Evaluate a `QTerm` by compiling and running it, then deserialize the result
/// (the `↓` operator). The term's code is run as a `rust-script` linked against
/// quilt's `rust` feature; the value is shuttled back via `postcard`.
pub fn reduce<T: Serialize + for<'de> Deserialize<'de>>(x: &QTerm) -> Result<T> {
    // get x as a string
    let input = x.coparse();
    // create the file the result is shuttled back through (postcard bytes)
    let mut out_file = tempfile::NamedTempFile::new().into_diagnostic()?;
    let out_path = out_file.path().to_str().unwrap();

    // build a full rust-script program with its dependencies in the frontmatter.
    // `CARGO_MANIFEST_DIR` is an absolute path to *this* quilt crate, so the
    // manifest works regardless of cwd (embedded-manifest relative paths would
    // resolve against rust-script's cache dir, not cwd).
    let quilt_dir = env!("CARGO_MANIFEST_DIR");
    let script = indoc::formatdoc! {r#"
        //! ```cargo
        //! [dependencies]
        //! quilt = {{ path = "{quilt_dir}", package = "quiltlang", default-features = false, features = ["rust"] }}
        //! postcard = {{ version = "1.1", features = ["alloc"] }}
        //! ```
        #[allow(unused_imports)]
        use quilt::prelude::*;
        use std::io::Write;
        fn main() -> Result<()> {{
            let output = {input};
            let data = postcard::to_allocvec(&output).unwrap();
            let mut file = std::fs::File::create("{out_path}").unwrap();
            file.write_all(&data).unwrap();
            Ok(())
        }}
    "#};

    // write the script to a temp file and run it
    let script_file = tempfile::Builder::new()
        .suffix(".rs")
        .tempfile()
        .into_diagnostic()?;
    std::fs::write(script_file.path(), script).into_diagnostic()?;
    let status = Command::new("rust-script")
        .arg(script_file.path())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .into_diagnostic()?;

    if !status.success() {
        bail!("reduce: script failed with status {status}");
    }

    let mut data = Vec::new();
    out_file.read_to_end(&mut data).into_diagnostic()?;
    postcard::from_bytes(&data).into_diagnostic()
}

impl QTerm {
    pub fn reduce<T: Serialize + for<'de> Deserialize<'de>>(&self) -> Result<T> {
        reduce(self)
    }
}
