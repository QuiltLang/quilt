use super::lang::BootstrapLanguage;
use super::strlift::StrLift;
use crate::lang::{one_liner, Arity, FlatNode, Language};
use crate::meta::OuterKind;
use crate::prelude::*;
use crate::qterm::tb;
use crate::term::STerm;
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};
use miette::{bail, IntoDiagnostic};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::io::Read;
use std::process::Command;
use std::sync::LazyLock;

/**************************************************************/

// NOTE: see bs_hole test below
pub const BS_HOLE: &str = "__BS_HOLE__";
pub static BS_HOLE_PARSE: LazyLock<Arc<QTerm>> = LazyLock::new(|| {
    BootstrapLanguage::default()
        .parse_expr(&one_liner(BS_HOLE))
        .unwrap()
});
pub static BS_HOLE_EXPAND: LazyLock<Arc<QTerm>> = LazyLock::new(|| {
    BootstrapMetaLanguage::default()
        .str_expand(&(*BS_HOLE_PARSE))
        .unwrap()
});
pub static BS_HOLE_CODE: LazyLock<String> = LazyLock::new(|| BS_HOLE_EXPAND.coparse().clone());

/**************************************************************/

/// Fill holes in a qterm with plugs.
fn fill<'a, I: Iterator<Item = &'a Arc<QTerm>>>(
    qterm: &QTerm,
    plugs: &mut I,
) -> Result<Arc<QTerm>> {
    if *qterm == **BS_HOLE_PARSE {
        plugs
            .next()
            .cloned()
            .ok_or_else(|| miette!("fill: not enough plugs for {qterm:?}"))
    } else if let QTerm::Tuple { tag, terms, cmds } = qterm {
        let terms = terms
            .iter()
            .map(|x| fill(x, plugs))
            .collect::<Result<Vec<_>>>()?;
        Ok(tuple(tag, &terms, cmds))
    } else {
        bail!("fill: unexpected tuple: {qterm:?}")
    }
}

/// Fill holes in an expanded qterm with plugs.
fn fill_expanded<'a, I: Iterator<Item = &'a Arc<QTerm>>>(
    qterm: &QTerm,
    plugs: &mut I,
) -> Result<Arc<QTerm>> {
    if *qterm == **BS_HOLE_EXPAND {
        plugs
            .next()
            .cloned()
            .ok_or_else(|| miette!("fill: not enough plugs for {qterm:?}"))
    } else if let QTerm::Tuple { tag, terms, cmds } = qterm {
        let terms: Vec<Arc<QTerm>> = terms
            .iter()
            .map(|x| fill_expanded(x, plugs))
            .collect::<Result<Vec<_>>>()?;
        Ok(tuple(tag, &terms, cmds))
    } else {
        bail!("fill: unexpected tuple: {qterm:?}")
    }
}

/**************************************************************/

// NOTE: it's not normal for the a MetaLanguage to use a Language, but bootstrapping does.
#[derive(Default)]
pub struct BootstrapMetaLanguage(RefCell<BootstrapLanguage>);

impl BootstrapMetaLanguage {
    /// Expand using strings.
    fn str_expand<T: StrLift>(&self, t: &T) -> Result<Arc<QTerm>> {
        let lifted = t.strlift();
        self.parse_expr(&one_liner(&lifted))
    }

    /// Expand using strings and fill holes with plugs.
    fn str_expand_and_fill<T: StrLift>(
        &self,
        qterm: &T,
        plugs: &[Arc<QTerm>],
    ) -> Result<Arc<QTerm>> {
        // lift to qterm
        let qterm = self.str_expand(qterm)?;
        // Fill the holes with plugs.
        fill_expanded(&qterm, &mut plugs.iter())
    }

    fn parse_expr(&self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.0.borrow_mut().parse_expr(code)
    }
    fn parse_stmt(&self, code: &[FlatNode]) -> Result<Arc<QTerm>> {
        self.0.borrow_mut().parse_stmt(code)
    }
}

impl MetaLanguage for BootstrapMetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        self.str_expand_and_fill(
            &quote(tag, i, lang2, BS_HOLE_PARSE.clone(), cmds),
            std::slice::from_ref(qterm),
        )
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        tag: &str,
        i: Index,
        lang2: &str,
        qterm: &Arc<QTerm>,
        cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        self.str_expand_and_fill(
            &unquote(tag, i, lang2, BS_HOLE_PARSE.clone(), cmds),
            std::slice::from_ref(qterm),
        )
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        tag: &str,
        qterms: &[Arc<QTerm>],
        cmds: &[CmdOrHole],
        arity: Arity,
    ) -> Result<Arc<QTerm>> {
        if arity == Arity::Variadic {
            let mut b_ = tb("block");
            b_.child(&sym("{")).push("    ").nl();

            b_.child(&self.parse_stmt(&one_liner(&format!("let mut b_ = tb(\"{tag}\");")))?)
                .nl();
            for cmd in cmds {
                let child = match cmd {
                    CmdOrHole::Cmd(cmd) => match cmd {
                        StrCmd::Write(s) => &self.parse_stmt(&one_liner(&format!(
                            "b_.write({});",
                            s.as_ref().strlift()
                        )))?,
                        StrCmd::NewLine => &self.parse_stmt(&one_liner("b_.nl();"))?,
                        StrCmd::Push(s) => &self.parse_stmt(&one_liner(&format!(
                            "b_.push({});",
                            s.as_ref().strlift()
                        )))?,
                        StrCmd::Pop => &self.parse_stmt(&one_liner("b_.pop();"))?,
                    },
                    CmdOrHole::Hole => &BS_HOLE_PARSE,
                };
                b_.child(child).nl();
            }
            b_.child(&self.parse_expr(&one_liner("b_.b()"))?);

            b_.pop().nl().child(&sym("}"));
            return fill(&b_.build(), &mut qterms.iter());
        }

        self.str_expand_and_fill(
            &tuple(tag, &(vec![BS_HOLE_PARSE.clone(); qterms.len()]), cmds),
            qterms,
        )
    }

    fn wrap_child(&self, sterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>> {
        match okind {
            OuterKind::None => Ok(sterm),
            OuterKind::Emit => fill(
                &*self
                    .0
                    .borrow_mut()
                    // the `let` avoids having multiple mutable borrows of b_ for nested emits
                    .parse_stmt(&one_liner(&format!("{BS_HOLE}.emit(&mut b_);")))?,
                &mut std::iter::once(&sterm),
            ),
            OuterKind::Splice => fill(
                &*self
                    .0
                    .borrow_mut()
                    .parse_stmt(&one_liner(&format!("{BS_HOLE};")))?,
                &mut std::iter::once(&sterm),
            ),
        }
    }

    fn pattern_tag(&self) -> Option<&'static str> {
        Some("let_declaration")
    }

    fn pattern_var(&self, name: &str) -> Result<Arc<QTerm>> {
        Ok(crate::qmatch::pattern_var_code(name))
    }

    fn pattern_let(
        &self,
        names: &[Box<str>],
        pattern: &Arc<QTerm>,
        value: &Arc<QTerm>,
    ) -> Result<(Arc<QTerm>, Arc<QTerm>)> {
        Ok(crate::qmatch::pattern_let_code(names, pattern, value))
    }

    // No heterogeneous lifting from the bootstrap meta: `target` is ignored.
    fn lift_str(&self, _target: &str) -> Result<&'static str> {
        Ok("bs_lift()")
    }

    // No heterogeneous reduction from the bootstrap meta: `target` is ignored.
    fn reduce_str(&self, _target: &str) -> Result<&'static str> {
        Ok("bs_reduce()")
    }

    fn type_str(&self) -> &'static str {
        "Arc<QTerm>"
    }

    fn name_str(&self) -> &'static str {
        "bs_name"
    }

    fn emit_str(&self) -> &'static str {
        // "b_.emit"
        "emit(&mut b_)"
    }
}

pub fn bs_lift<T: StrLift>(x: &T) -> Arc<QTerm> {
    let lifted = x.strlift();
    BootstrapLanguage::default()
        .parse_expr(&one_liner(&lifted))
        .expect("bs_lift: failed to parse lifted code")
}

pub trait BsLift {
    fn bs_lift(&self) -> Arc<QTerm>;
}

impl<T: StrLift> BsLift for T {
    fn bs_lift(&self) -> Arc<QTerm> {
        bs_lift(self)
    }
}

pub fn bs_reduce<T: Serialize + for<'de> Deserialize<'de>>(x: &QTerm) -> Result<T> {
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
        //! quilt = {{ path = "{quilt_dir}", package = "quiltlang", default-features = false, features = ["bootstrap"] }}
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
        bail!("bs_reduce: script failed with status {status}");
    }

    let mut data = Vec::new();
    out_file.read_to_end(&mut data).into_diagnostic()?;
    postcard::from_bytes(&data).into_diagnostic()
}

impl QTerm {
    pub fn bs_reduce<T: Serialize + for<'de> Deserialize<'de>>(&self) -> Result<T> {
        bs_reduce(self)
    }
}

pub fn bs_name(s: &str) -> Arc<QTerm> {
    leaf("identifier", s)
}
