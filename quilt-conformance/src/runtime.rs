//! The Rust runner for the shared runtime corpus (issue #159).
//!
//! `conformance/runtime/cases.json` describes builder programs and the text
//! each must `coparse` to. Three published packages implement that API —
//! `quiltlang`, `quilt-python`, `quilt-wasm` — and must agree. Before this,
//! they were tested asymmetrically and separately: 9 pytest cases, one
//! `smoke.cjs` CI never ran, and no Rust API-parity tests at all. Nothing
//! compared them, so the documented divergences (`.c(&x)` vs `.c(x)`, postfix
//! `qlift()` vs prefix `qlift(x)`, `qlift_html`) were exactly where drift would
//! go unnoticed.
//!
//! This module is the interpreter for one runtime. The other two are ~80-line
//! runners in their own languages; adding a fourth runtime means writing one
//! more, and it inherits the whole corpus.

use miette::{bail, IntoDiagnostic as _, Result, WrapErr as _};
use quilt::prelude::*;
use serde::Deserialize;
use std::path::PathBuf;

/// One case from the corpus.
#[derive(Debug, Deserialize)]
pub struct Case {
    pub name: String,
    /// Which runtimes have the API this case needs. Absent means all three.
    #[serde(default)]
    pub runtimes: Option<Vec<String>>,
    /// Why a case is narrowed, or what it is guarding. Documentation only.
    #[serde(default)]
    pub why: Option<String>,
    pub term: Term,
    pub coparse: String,
}

impl Case {
    pub fn applies_to(&self, runtime: &str) -> bool {
        self.runtimes
            .as_ref()
            .is_none_or(|rs| rs.iter().any(|r| r == runtime))
    }
}

#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub cases: Vec<Case>,
}

/// A term constructor, mirroring the JSON tagging.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Term {
    Leaf { tag: String, text: String },
    Sym(String),
    Name(String),
    Qlift(Value),
    QliftHtml(Value),
    Tb(Tb),
    Quote(Bracket),
    Unquote(Bracket),
}

#[derive(Debug, Deserialize)]
pub struct Tb {
    pub tag: String,
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub struct Bracket {
    pub tag: String,
    pub index: u8,
    pub lang: String,
    pub term: Box<Term>,
    pub cmds: Vec<Cmd>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    W(String),
    C(Term),
    E(Term),
    P(String),
    /// Unit steps arrive as bare strings: `"n"`, `"x"`.
    #[serde(rename = "n")]
    N,
    #[serde(rename = "x")]
    X,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmd {
    Write(String),
    Push(String),
    #[serde(rename = "NL")]
    Nl,
    #[serde(rename = "POP")]
    Pop,
    #[serde(rename = "HOLE")]
    Hole,
}

/// A liftable scalar, or a nested term (so `qlift` idempotence is testable).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Value {
    // Order matters: serde's untagged enum tries variants top to bottom, and
    // JSON `true` would deserialize as an integer if `Int` came first.
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Term(Box<Term>),
}

pub fn corpus_path() -> PathBuf {
    crate::repo_root().join("conformance/runtime/cases.json")
}

pub fn load() -> Result<Corpus> {
    let path = corpus_path();
    let text = std::fs::read_to_string(&path)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text)
        .into_diagnostic()
        .wrap_err_with(|| format!("parsing {}", path.display()))
}

fn build_cmds(cmds: &[Cmd]) -> Vec<quilt::term::CmdOrHole> {
    cmds.iter()
        .map(|c| match c {
            Cmd::Write(s) => cmd(write(s)),
            Cmd::Push(s) => cmd(push(s)),
            Cmd::Nl => cmd(NL),
            Cmd::Pop => cmd(POP),
            Cmd::Hole => HOLE,
        })
        .collect()
}

fn build_value(v: &Value) -> Result<Arc<QTerm>> {
    Ok(match v {
        Value::Bool(b) => b.qlift(),
        Value::Int(n) => n.qlift(),
        Value::Float(f) => f.qlift(),
        Value::Str(s) => s.as_str().qlift(),
        // `qlift` on a term is the identity — the property the corpus pins.
        Value::Term(t) => build(t)?.qlift(),
    })
}

/// Interpret one corpus term against the Rust runtime.
pub fn build(t: &Term) -> Result<Arc<QTerm>> {
    Ok(match t {
        Term::Leaf { tag, text } => leaf(tag, text),
        Term::Sym(s) => sym(s),
        Term::Name(s) => name(s),
        Term::Qlift(v) => build_value(v)?,
        Term::QliftHtml(_) => {
            bail!(
                "qlift_html has no Rust implementation (no LiftTo<Html> marker — issue #149); \
                 the case should narrow `runtimes` to python and wasm"
            )
        }
        Term::Tb(Tb { tag, steps }) => {
            let mut b = tb(tag);
            for step in steps {
                b = match step {
                    Step::W(s) => b.w(s),
                    Step::C(child) => b.c(&build(child)?),
                    Step::E(child) => b.e(build(child)?),
                    Step::P(s) => b.p(s),
                    Step::N => b.n(),
                    Step::X => b.x(),
                };
            }
            b.b()
        }
        Term::Quote(br) => quote(
            &br.tag,
            br.index,
            &br.lang,
            build(&br.term)?,
            &build_cmds(&br.cmds),
        ),
        Term::Unquote(br) => unquote(
            &br.tag,
            br.index,
            &br.lang,
            build(&br.term)?,
            &build_cmds(&br.cmds),
        ),
    })
}

/// Run every case that applies to the Rust runtime; return the failures.
pub fn run() -> Result<Vec<String>> {
    let corpus = load()?;
    let mut failures = Vec::new();
    let mut ran = 0;

    for case in &corpus.cases {
        if !case.applies_to("rust") {
            continue;
        }
        ran += 1;
        match build(&case.term) {
            Ok(term) => {
                let got = term.coparse();
                if got != case.coparse {
                    failures.push(format!(
                        "{}: coparse is {got:?}, corpus says {:?}",
                        case.name, case.coparse
                    ));
                }
            }
            Err(e) => failures.push(format!("{}: build failed: {e}", case.name)),
        }
    }

    if ran == 0 {
        bail!("no corpus cases applied to the rust runtime");
    }
    Ok(failures)
}
