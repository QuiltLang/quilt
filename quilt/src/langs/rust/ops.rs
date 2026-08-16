//! Direct term-lifting for the Rust meta-language.
//!
//! These helpers build the `Arc<QTerm>` that *reconstructs* a term by writing
//! constructor source (`tb(..).c(&child)..b()`, `quote(..)`, ...) directly and
//! splicing child terms at holes. This is the term-valued analogue of
//! `langs::bootstrap::strlift` — same emitted source, but with no string
//! round-trip (no re-parse).
//!
//! The fold that assembles those chains is shared with the Python and
//! TypeScript metas — see [`crate::langs::chain`], which also carries the
//! generated table of Rust's fragments (`.c(&x)`, `&[..]`). Two things here are
//! Rust's alone and stay hand-written: the [`build_variadic_block`] below,
//! which is an imperative `b_` block rather than a chain, and the escaping.
//!
//! [`QLift`] runs the same fold in [`Lit::Term`] mode, which is the difference
//! between code that only has to serialize and code that has to be
//! *matchable*: `qlift` emits structured `string_literal` subterms so
//! `rewrite_naive` can find them.

use crate::langs::chain::{Chain, Lit, RUST};
use crate::prelude::*;
use crate::term::CmdOrHole;
use miette::{bail, IntoDiagnostic};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::process::Command;

/**************************************************************/

/// Escape the body of a Rust double-quoted string literal.
fn str_body(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render a Rust string literal, escaping `\` and `"` (matches `strlift` for
/// `str`).
fn str_lit(s: &str) -> String {
    format!("\"{}\"", str_body(s))
}

/// The shared builder-call fold, spelled for Rust, emitting literals as source
/// text. The dump-only path the three `build_*_code` helpers take.
const FLAT: Chain = Chain::new(&RUST, Lit::Flat(str_lit));

/// The same fold, emitting literals as structured `string_literal` subterms so
/// the result can be manipulated as Rust AST. The path [`QLift`] takes.
const TERM: Chain = Chain::new(&RUST, Lit::Term(strlit_term));

/**************************************************************/

/// Build code that reconstructs a tuple: `tb(tag).w(..).c(&child)..b()`, using
/// the `sym`/`leaf` shorthands when possible. `children` are the already-built
/// child expressions spliced at hole positions.
pub fn build_tuple_code(tag: &str, cmds: &[CmdOrHole], children: &[Arc<QTerm>]) -> Arc<QTerm> {
    FLAT.tuple_code(tag, cmds, children)
}

/// Build `quote(tag, index, lang, <term>, &[..cmds..])`, splicing `term`.
pub fn build_quote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    FLAT.quote_code(tag, index, lang, term, cmds)
}

/// Build `unquote(tag, index, lang, <term>, &[..cmds..])`, splicing `term`.
pub fn build_unquote_code(
    tag: &str,
    index: Index,
    lang: &str,
    term: &Arc<QTerm>,
    cmds: &[CmdOrHole],
) -> Arc<QTerm> {
    FLAT.unquote_code(tag, index, lang, term, cmds)
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

/// Code for a pattern metavariable splice: `mvar("name")`.
///
/// This and [`pattern_let_code`] used to live in `crate::qmatch`, beside the
/// runtime they call into. But `mvar(..)` and `qmatch_n(&p, &v)` are *Rust*
/// source, so emitting them from the language-agnostic core is what made
/// pattern matching Rust-only while reading as though it were not. A second
/// host would add its own pair next to its own `build_*_code`.
pub fn pattern_var_code(name: &str) -> Arc<QTerm> {
    leaf("_", &format!("mvar(\"{name}\")"))
}

/// The two terms a pattern-let rewrites to: the destructuring binder
/// `[a, b]` that replaces the pattern quote, and the matching call
/// `qmatch_n(&<pattern>, &<value>)` that replaces the initializer.
pub fn pattern_let_code(
    names: &[Box<str>],
    pattern: &Arc<QTerm>,
    value: &Arc<QTerm>,
) -> (Arc<QTerm>, Arc<QTerm>) {
    let binder = leaf("_", &format!("[{}]", names.join(", ")));
    let call = tb("_")
        .w("qmatch_n(&")
        .c(pattern)
        .w(", &")
        .c(value)
        .w(")")
        .b();
    (binder, call)
}

/// A Rust string-literal term, structured exactly as the parser (and `↖"s"↗`)
/// produces it, so lifted code can be matched/rewritten as Rust AST (e.g. by
/// `rewrite_naive`).
///
/// `s` is escaped with the same rules as [`str_lit`]. This used to assume the
/// caller had already ensured `s` needed no escaping, which held for the
/// expander's own tags and language names but not for the two paths that pass
/// arbitrary user data: `QLift for str`/`String` (a lifted Rust string) and the
/// `Write`/`Push` command bodies (arbitrary source text). A `"` in either
/// produced a generated program that did not parse. Escaping here is also the
/// *more* faithful reproduction of the parser's own output, since tree-sitter
/// records `string_content` exactly as it appears in source — backslashes and
/// all.
/// The empty string is `(string_literal "\"" "\"")` — the parser emits no
/// `string_content` child when there is no content — so this mirrors that rather
/// than emitting an empty one.
fn strlit_term(s: &str) -> Arc<QTerm> {
    let mut b = tb("string_literal").c(&sym("\""));
    if !s.is_empty() {
        b = b.c(&leaf("string_content", &str_body(s)));
    }
    b.c(&sym("\"")).b()
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
        "nix" => Ok("qlift_to::<Nix>()"),
        "lean" | "lean4" => Ok("qlift_to::<Lean>()"),
        "sql" => Ok("qlift_to::<Sql>()"),
        // Same grammar, different escaping: MySQL reads a backslash inside
        // `'…'` as an escape and the standard does not (#233).
        "mysql" | "mariadb" => Ok("qlift_to::<MySql>()"),
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
            } => TERM.quote_code(tag, *index, lang, &term.qlift(), cmds),
            QTerm::Unquote {
                tag,
                index,
                lang,
                term,
                cmds,
                ..
            } => TERM.unquote_code(tag, *index, lang, &term.qlift(), cmds),
            QTerm::Tuple { tag, terms, cmds } => {
                let children: Vec<Arc<QTerm>> = terms.iter().map(QLift::qlift).collect();
                TERM.tuple_code(tag, cmds, &children)
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

/// Floats need `{:?}` rather than `{}`: `1.0f64.to_string()` is `"1"`, which
/// would lift a float as an *integer* literal and silently change the type of
/// the generated code. A negative float is a `unary_expression` over a positive
/// literal, matching how the parser sees `-1.5` — the same shape the Lean lift
/// uses for the same reason.
macro_rules! qlift_float {
    ($($t:ty),* $(,)?) => {$(
        impl QLift for $t {
            fn qlift(&self) -> Arc<QTerm> {
                let s = format!("{self:?}");
                if *self < 0.0 {
                    // `-1.5` parses as `(unary_expression "-" (float_literal))`:
                    // the sign is a child token, not literal text.
                    return tb("unary_expression")
                        .c(&sym("-"))
                        .c(&leaf("float_literal", s.trim_start_matches('-')))
                        .b();
                }
                leaf("float_literal", &s)
            }
        }
    )*};
}
qlift_float!(f32, f64);

impl QLift for bool {
    fn qlift(&self) -> Arc<QTerm> {
        // `true` parses as `(boolean_literal "true")` — the keyword is a child
        // token, so a flat leaf is a shape the parser never produces.
        tb("boolean_literal")
            .c(&sym(if *self { "true" } else { "false" }))
            .b()
    }
}

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
