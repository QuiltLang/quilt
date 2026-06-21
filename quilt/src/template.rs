//! Tier A template instantiation (issue #87): fill the `↙name↘` holes of a
//! sky-first [`QTerm`](crate::qterm::QTerm) (see
//! [`Multi::parse_sky`](crate::multi::Multi::parse_sky)) with concrete values,
//! producing finished output source. No host code is compiled or run — each
//! hole is replaced by its parameter value *lifted* into the hole's object
//! language, reusing the existing [`LiftTo`] machinery rather than inventing a
//! per-language substitution.
//!
//! Because lifting is pure and `lift.rs` is always compiled, this module is too:
//! a wasm consumer can instantiate templates without the tree-sitter parser.
//!
//! The richer **Tier B** path (issue #89) handles holes that are real host
//! expressions — loops, conditionals, computation — instead of bare parameter
//! names. [`tier_b_program`] source-wraps the template into a host metaprogram
//! that the normal parse → expand → run pipeline executes; the running program
//! prints the instantiated output. That run is the CLI's job (`bin.rs`); this
//! module only generates the wrapper, so it stays parser- and runtime-free.

use crate::multi::ident_name;
use crate::prelude::*;
use crate::qterm::qquote_at;
use miette::LabeledSpan;
use std::collections::BTreeMap;

/**************************************************************/

/// A value supplied for a template parameter. Scalars cover the common literal
/// kinds; [`List`](ParamValue::List) is a (possibly nested) sequence that lifts
/// to the target language's list literal.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<ParamValue>),
}

impl ParamValue {
    /// A human name for this value's kind, for error messages.
    fn kind(&self) -> &'static str {
        match self {
            ParamValue::Str(_) => "string",
            ParamValue::Int(_) => "integer",
            ParamValue::Float(_) => "float",
            ParamValue::Bool(_) => "bool",
            ParamValue::List(_) => "list",
        }
    }
}

impl From<&str> for ParamValue {
    fn from(s: &str) -> Self {
        ParamValue::Str(s.to_owned())
    }
}
impl From<String> for ParamValue {
    fn from(s: String) -> Self {
        ParamValue::Str(s)
    }
}
impl From<i64> for ParamValue {
    fn from(i: i64) -> Self {
        ParamValue::Int(i)
    }
}
impl From<i32> for ParamValue {
    fn from(i: i32) -> Self {
        ParamValue::Int(i.into())
    }
}
impl From<f64> for ParamValue {
    fn from(f: f64) -> Self {
        ParamValue::Float(f)
    }
}
impl From<bool> for ParamValue {
    fn from(b: bool) -> Self {
        ParamValue::Bool(b)
    }
}
impl<T: Into<ParamValue>> From<Vec<T>> for ParamValue {
    fn from(xs: Vec<T>) -> Self {
        ParamValue::List(xs.into_iter().map(Into::into).collect())
    }
}

/// The parameter environment an instantiation is run against: name → value.
pub type ParamEnv = BTreeMap<Box<str>, ParamValue>;

/**************************************************************/
// `LiftTo<L> for ParamValue` for the targets whose `lift.rs` impls already
// cover every scalar kind *and* a list literal. Implementing the trait lets the
// blanket `[T]`/`Vec<T>` list impls light up for free, so a nested `List` lifts
// recursively with the right container spelling.

impl LiftTo<Python> for ParamValue {
    fn lift_to(&self) -> Arc<QTerm> {
        match self {
            ParamValue::Str(s) => LiftTo::<Python>::lift_to(s.as_str()),
            ParamValue::Int(i) => LiftTo::<Python>::lift_to(i),
            ParamValue::Float(f) => LiftTo::<Python>::lift_to(f),
            ParamValue::Bool(b) => LiftTo::<Python>::lift_to(b),
            ParamValue::List(xs) => LiftTo::<Python>::lift_to(xs.as_slice()),
        }
    }
}

impl LiftTo<Nix> for ParamValue {
    fn lift_to(&self) -> Arc<QTerm> {
        match self {
            ParamValue::Str(s) => LiftTo::<Nix>::lift_to(s.as_str()),
            ParamValue::Int(i) => LiftTo::<Nix>::lift_to(i),
            ParamValue::Float(f) => LiftTo::<Nix>::lift_to(f),
            ParamValue::Bool(b) => LiftTo::<Nix>::lift_to(b),
            ParamValue::List(xs) => LiftTo::<Nix>::lift_to(xs.as_slice()),
        }
    }
}

/**************************************************************/

/// Lift a parameter value into a `QTerm` of object language `target` — the
/// runtime, dynamically-dispatched analog of `value.qlift_to::<Target>()`.
/// Scalars reuse the `lift.rs` / Rust `qlift` spellings; lists use the target's
/// list literal. Targets whose lift would be ambiguous or undefined for a given
/// value kind return a clear error rather than guessing.
pub fn lift_param(target: &str, value: &ParamValue) -> Result<Arc<QTerm>> {
    match target {
        "python" | "py" => Ok(LiftTo::<Python>::lift_to(value)),
        "nix" => Ok(LiftTo::<Nix>::lift_to(value)),
        #[cfg(feature = "rust")]
        "rust" | "rs" => Ok(lift_rust(value)),
        "html" => Ok(leaf("text", &html_text(value))),
        "zsh" => lift_shell::<Zsh>(value),
        "bash" => lift_shell::<Bash>(value),
        other => Err(miette!(
            "instantiate: no template lift defined for target language {other:?} \
             (supported: python, rust, nix, html, zsh, bash)"
        )),
    }
}

/// Lift into Rust, reusing `qlift` (= `LiftTo<Rust>`) for the kinds it covers
/// and spelling the rest (float, bool, list) directly.
#[cfg(feature = "rust")]
fn lift_rust(value: &ParamValue) -> Arc<QTerm> {
    match value {
        ParamValue::Str(s) => qlift(s.as_str()),
        ParamValue::Int(i) => qlift(i),
        ParamValue::Float(f) => leaf("float_literal", &format!("{f:?}")),
        ParamValue::Bool(b) => leaf("boolean_literal", if *b { "true" } else { "false" }),
        ParamValue::List(xs) => {
            // A Rust array literal: `[a, b, c]`.
            let mut b = tb("array_expression").w("[");
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    b = b.w(", ");
                }
                b = b.c(&lift_rust(x));
            }
            b.w("]").b()
        }
    }
}

/// Lift into a POSIX shell (Zsh/Bash). Both have `lift.rs` impls for strings and
/// integers; other kinds have no inert spelling, so they error.
fn lift_shell<L>(value: &ParamValue) -> Result<Arc<QTerm>>
where
    str: LiftTo<L>,
    i64: LiftTo<L>,
{
    match value {
        ParamValue::Str(s) => Ok(LiftTo::<L>::lift_to(s.as_str())),
        ParamValue::Int(i) => Ok(LiftTo::<L>::lift_to(i)),
        other => Err(miette!(
            "shell template holes support only strings and integers, got a {}",
            other.kind()
        )),
    }
}

/// Render a value as inert HTML text content (the lift target for `html`
/// templates). Strings are entity-escaped so the value stays data, not markup;
/// a list is rendered comma-separated.
fn html_text(value: &ParamValue) -> String {
    match value {
        ParamValue::Str(s) => html_escape(s),
        ParamValue::Int(i) => i.to_string(),
        ParamValue::Float(f) => format!("{f:?}"),
        ParamValue::Bool(b) => (if *b { "true" } else { "false" }).to_owned(),
        ParamValue::List(xs) => xs.iter().map(html_text).collect::<Vec<_>>().join(", "),
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/**************************************************************/

/// Instantiate a sky-first template (Tier A): walk `template`, replacing every
/// `↙name↘` hole with its parameter value from `env` lifted into the hole's
/// object language. The result is a finished `QTerm` whose `coparse()` is the
/// output source. Errors if a hole names a missing parameter, or if a hole is a
/// host expression rather than a bare name (that needs Tier B, issue #89).
pub fn instantiate(template: &QTerm, env: &ParamEnv) -> Result<Arc<QTerm>> {
    match template {
        QTerm::Unquote {
            lang, term, span, ..
        } => {
            let raw = term.coparse();
            let name = ident_name(&raw).ok_or_else(|| {
                hole_error(
                    span.as_ref(),
                    format!(
                        "Tier A template holes must be a bare parameter name; `{}` is a \
                         host expression — that needs Tier B (issue #89)",
                        raw.trim()
                    ),
                )
            })?;
            let value = env.get(&*name).ok_or_else(|| {
                hole_error(
                    span.as_ref(),
                    format!("missing template parameter `{name}`"),
                )
            })?;
            lift_param(lang, value).map_err(|e| hole_error(span.as_ref(), e.to_string()))
        }
        QTerm::Quote {
            tag,
            index,
            lang,
            term,
            cmds,
            span,
        } => {
            let term = instantiate(term, env)?;
            Ok(arc(qquote_at(tag, *index, lang, term, cmds, span.clone())))
        }
        QTerm::Tuple { tag, terms, cmds } => {
            let terms = terms
                .iter()
                .map(|t| instantiate(t, env))
                .collect::<Result<Vec<_>>>()?;
            Ok(tuple(tag, &terms, cmds))
        }
    }
}

/// An instantiation error pointing at the offending `↙…↘` hole when its source
/// span is known (parsed terms carry spans; constructed ones don't).
fn hole_error(span: Option<&Span>, msg: impl Into<String>) -> miette::Report {
    let msg = msg.into();
    match span {
        Some(s) => miette!(
            labels = vec![LabeledSpan::at(s.clone(), "this hole")],
            "{msg}"
        ),
        None => miette!("{msg}"),
    }
}

/**************************************************************/
// Tier B (issue #89): host-backed holes.

/// Source-wrap a sky-first template `body` into a host-language metaprogram for
/// Tier B. The body becomes the inside of a `target↖ … ↗` quote with each
/// declared parameter bound (in scope) to its value, and the program prints the
/// quote's `coparse()`. Expanding and running the result (the normal
/// parse → expand → run pipeline) yields the instantiated output — so a hole may
/// be any host expression over the parameters (`↙↑(greeting if formal else hi)↘`,
/// `↙↑(", ".join(names))↘`, …), not just a bare name.
///
/// Only a **Python host** is supported for now (the simplest: Python allows the
/// quote at module scope, so the body's own indentation is preserved verbatim).
/// A Rust host would need the body re-indented into a `fn main` and is left for
/// later.
pub fn tier_b_program(
    host: &str,
    target: &str,
    body: &str,
    params: &[(Box<str>, ParamValue)],
) -> Result<String> {
    match host {
        "python" | "py" => Ok(python_tier_b(target, body, params)),
        other => Err(miette!(
            "Tier B currently supports only a Python host; got {other:?} (issue #89)"
        )),
    }
}

/// The Python-host Tier B wrapper. Parameters bind to their Python literals at
/// module scope; the body sits inside `target↖ … ↗` at column 0 (so no
/// indentation is added to strip back off); the result is written to stdout.
fn python_tier_b(target: &str, body: &str, params: &[(Box<str>, ParamValue)]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("from quilt import *\n\n");
    for (name, value) in params {
        // Python literal == the value lifted into Python, coparsed.
        let lit = LiftTo::<Python>::lift_to(value).coparse();
        let _ = writeln!(s, "{name} = {lit}");
    }
    let _ = write!(s, "\n__quilt_template__ = {target}↖\n{body}");
    if !body.ends_with('\n') {
        s.push('\n');
    }
    s.push_str("↗\n\nimport sys\nsys.stdout.write(__quilt_template__.coparse())\n");
    s
}
