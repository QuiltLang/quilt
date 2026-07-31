//! The Rust runner for the shared runtime corpus (issue #159).
//!
//! `conformance/runtime/cases.json` describes builder programs and the text
//! each must `coparse` to. Three published packages implement that API —
//! `quiltlang`, `quilt-python`, `quilt-wasm` — and must agree. Before this,
//! they were tested asymmetrically and separately: 9 pytest cases, one
//! `smoke.cjs` CI never ran, and no Rust API-parity tests at all. Nothing
//! compared them, so the documented divergences (`.c(&x)` vs `.c(x)`, postfix
//! `qlift()` vs prefix `qlift(x)`, `qlift_html`, and wasm's `NL()`/`POP()`/
//! `HOLE()` against Python's constants — issue #167) were exactly where drift
//! would go unnoticed.
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

/// A cross-cutting check: a property every case's term must satisfy, rather
/// than a term and its expected text. Declared once in the corpus and run over
/// every shape, so the properties that are *about* the shapes — surviving a
/// postcard round trip, stringifying to the same text as `coparse` — cover the
/// whole corpus without restating any of it.
#[derive(Debug, Deserialize)]
pub struct Check {
    pub name: String,
    /// Which runtimes have the API this check needs.
    pub runtimes: Vec<String>,
    /// Why it is narrowed to those.
    pub why: String,
    /// What the check asserts, and what it is guarding. Documentation only.
    #[serde(default)]
    pub what: Option<String>,
}

impl Check {
    pub fn applies_to(&self, runtime: &str) -> bool {
        self.runtimes.iter().any(|r| r == runtime)
    }
}

#[derive(Debug, Deserialize)]
pub struct Corpus {
    pub cases: Vec<Case>,
    #[serde(default)]
    pub checks: Vec<Check>,
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

/// `postcard_roundtrip`, for the Rust runtime.
///
/// The Rust side owns the `serde` derives that `quilt-python`'s
/// `postcard_bytes` / `from_postcard_bytes` pair — and the heterogeneous
/// `reduce` protocol either side of it — decode with, so it is worth checking
/// here too, where the fast `cargo test` path reaches it.
///
/// Three assertions — text, structure, bytes — because `postcard` is positional
/// and self-describes nothing: an asymmetry between the two derives decodes as a
/// *different term* rather than as an error, and which of the three notices
/// depends on where the asymmetry lands.
///
/// What this cannot see is a *symmetric* schema change. Every term the corpus
/// builds is constructed, so its `QTerm::span` is always `None`, and a
/// `serde(skip)` on both ends round-trips cleanly here — same text, same tree
/// (`PartialEq` ignores spans), same bytes. That is the case the `span` fields'
/// "no serde skip" comments are actually about, and it needs a *parsed*-shaped
/// term, which only Rust can build: see
/// `quilt::qterm::tests::postcard_round_trip_preserves_spans`.
fn postcard_roundtrip(name: &str, term: &Arc<QTerm>, failures: &mut Vec<String>) {
    let bytes = match postcard::to_allocvec(term) {
        Ok(b) => b,
        Err(e) => {
            failures.push(format!("{name}: postcard serialization failed: {e}"));
            return;
        }
    };
    let back: Arc<QTerm> = match postcard::from_bytes(&bytes) {
        Ok(t) => t,
        Err(e) => {
            failures.push(format!(
                "{name}: postcard deserialization failed: {e} ({} bytes)",
                bytes.len()
            ));
            return;
        }
    };
    let (want, got) = (term.coparse(), back.coparse());
    if got != want {
        failures.push(format!(
            "{name}: postcard round trip coparses to {got:?}, term is {want:?}"
        ));
    }
    if back != *term {
        failures.push(format!(
            "{name}: postcard round trip is structurally different (same text, different tree)"
        ));
    }
    match postcard::to_allocvec(&back) {
        Ok(again) if again != bytes => failures.push(format!(
            "{name}: postcard round trip re-serializes to {} bytes, not the original {} — \
             a field is being lost on decode",
            again.len(),
            bytes.len()
        )),
        Err(e) => failures.push(format!("{name}: re-serializing the round trip failed: {e}")),
        Ok(_) => {}
    }
}

/// Run every case and every check that applies to the Rust runtime; return the
/// failures.
pub fn run() -> Result<Vec<String>> {
    let corpus = load()?;
    let mut failures = Vec::new();
    let mut ran = 0;

    // A check this runner has no implementation for is a failure, not a skip:
    // the corpus names the runtimes a check applies to, so an unrecognized one
    // means the corpus is asking for something and being ignored.
    let known = ["postcard_roundtrip"];
    let wanted: Vec<&Check> = corpus
        .checks
        .iter()
        .filter(|c| c.applies_to("rust"))
        .collect();
    for check in &wanted {
        if !known.contains(&check.name.as_str()) {
            failures.push(format!(
                "check {:?} names the rust runtime, but this runner has no implementation of it",
                check.name
            ));
        }
    }
    let wants = |n: &str| wanted.iter().any(|c| c.name == n);

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
                if wants("postcard_roundtrip") {
                    postcard_roundtrip(&case.name, &term, &mut failures);
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
