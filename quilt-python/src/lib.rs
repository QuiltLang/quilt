//! Python bindings for quilt's core IR.
//!
//! These expose the real Rust `QTerm`, its builder, and the `coparse`
//! serializer to Python — the runtime that expanded `.py.quilt` files target.
//! `PythonMetaLanguage` emits Python source like
//! `tb("binary_operator").c(leaf("integer", "1")).w(" ")..b()`, and these bindings
//! are exactly the `tb`/`leaf`/`sym`/`quote`/`unquote`/`cmd`/`write`/`push`/`name`
//! functions, the `NL`/`POP`/`HOLE` constants, and the fluent `Builder`
//! (`.c`/`.w`/`.n`/`.p`/`.x`/`.e`/`.b`) and `QTerm` (`.coparse()`) classes that
//! source calls into.

use pyo3::prelude::*;
use quilt::prelude::{Arc, QTerm};
use quilt::qterm::{
    leaf as mk_leaf, quote as mk_quote, sym as mk_sym, tb as mk_tb, unquote as mk_unquote,
    QTermBuilder,
};
use quilt::strcmd::{push as mk_push, write as mk_write, StrCmd};
use quilt::term::{cmd as mk_cmd, CmdOrHole, STerm};

/**************************************************************/

/// A quilt term (`Arc<QTerm>`).
#[pyclass(name = "QTerm", from_py_object)]
#[derive(Clone)]
struct PyQTerm(Arc<QTerm>);

#[pymethods]
impl PyQTerm {
    /// Serialize the term back to source code.
    fn coparse(&self) -> String {
        self.0.coparse()
    }

    fn __repr__(&self) -> String {
        format!("QTerm({:?})", self.0.coparse())
    }

    fn __str__(&self) -> String {
        self.0.coparse()
    }

    /// Serialize this term to postcard bytes for the heterogeneous `py↓` protocol.
    fn postcard_bytes(&self) -> PyResult<Vec<u8>> {
        postcard::to_allocvec(&self.0)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
    }

    /// Evaluate this term's source to a Python value (the `↓` operator).
    ///
    /// Delegates to the `quilt` package's reducer, which expands the source
    /// first if it is still Quilt (contains glyphs), then evaluates it as a
    /// *block*: leading statements are run and the value is the trailing
    /// expression (None if it ends in a statement) — the block-value semantics
    /// of Rust/Lisp/Ruby/…. So a generated multi-statement stage reduces to its
    /// result, not just a bare expression.
    fn reduce<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let quilt = py.import("quilt")?;
        quilt.getattr("_reduce_src")?.call1((self.0.coparse(),))
    }
}

/// A single string command (`write`/`NL`/`push`/`POP`). Only passed by
/// reference, so it opts out of the `FromPyObject` derive.
#[pyclass(name = "StrCmd", skip_from_py_object)]
#[derive(Clone)]
struct PyStrCmd(StrCmd);

/// A `StrCmd` or a child placeholder (`HOLE`), used in `quote`/`unquote` cmds.
#[pyclass(name = "CmdOrHole", from_py_object)]
#[derive(Clone)]
struct PyCmdOrHole(CmdOrHole);

/// A fluent term builder, mirroring the Rust `QTermBuilder`.
#[pyclass(name = "Builder")]
struct PyBuilder {
    inner: Option<QTermBuilder>,
}

#[pymethods]
impl PyBuilder {
    /// Splice a child term.
    fn c<'py>(mut slf: PyRefMut<'py, Self>, child: &PyQTerm) -> PyRefMut<'py, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.child(&child.0);
        }
        slf
    }

    /// Write literal source text.
    fn w<'py>(mut slf: PyRefMut<'py, Self>, s: &str) -> PyRefMut<'py, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.write(s);
        }
        slf
    }

    /// Emit a newline (respecting the current prefix).
    fn n(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.nl();
        }
        slf
    }

    /// Push an indentation prefix.
    fn p<'py>(mut slf: PyRefMut<'py, Self>, s: &str) -> PyRefMut<'py, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.push(s);
        }
        slf
    }

    /// Pop an indentation prefix.
    fn x(mut slf: PyRefMut<'_, Self>) -> PyRefMut<'_, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.pop();
        }
        slf
    }

    /// Emit a child term (for `Arc<QTerm>` this is the same as [`c`]).
    fn e<'py>(mut slf: PyRefMut<'py, Self>, child: &PyQTerm) -> PyRefMut<'py, Self> {
        if let Some(b) = slf.inner.as_mut() {
            b.emit(child.0.clone());
        }
        slf
    }

    /// Build the term. Consumes the builder.
    fn b(&mut self) -> PyResult<PyQTerm> {
        let builder = self
            .inner
            .take()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Builder already built"))?;
        Ok(PyQTerm(builder.b()))
    }
}

/**************************************************************/

/// Start building a tuple node with the given tag.
#[pyfunction]
fn tb(tag: &str) -> PyBuilder {
    PyBuilder {
        inner: Some(mk_tb(tag)),
    }
}

/// A leaf node: a tag whose only content is `code`.
#[pyfunction]
fn leaf(tag: &str, code: &str) -> PyQTerm {
    PyQTerm(mk_leaf(tag, code))
}

/// A symbol: a leaf whose tag and code are the same.
#[pyfunction]
fn sym(s: &str) -> PyQTerm {
    PyQTerm(mk_sym(s))
}

/// A quoted fragment.
#[pyfunction]
fn quote(tag: &str, index: u8, lang: &str, term: &PyQTerm, cmds: Vec<PyCmdOrHole>) -> PyQTerm {
    let cmds: Vec<CmdOrHole> = cmds.into_iter().map(|c| c.0).collect();
    PyQTerm(mk_quote(tag, index, lang, term.0.clone(), &cmds))
}

/// An unquoted splice.
#[pyfunction]
fn unquote(tag: &str, index: u8, lang: &str, term: &PyQTerm, cmds: Vec<PyCmdOrHole>) -> PyQTerm {
    let cmds: Vec<CmdOrHole> = cmds.into_iter().map(|c| c.0).collect();
    PyQTerm(mk_unquote(tag, index, lang, term.0.clone(), &cmds))
}

/// Wrap a `StrCmd` as a `CmdOrHole`.
#[pyfunction]
fn cmd(c: &PyStrCmd) -> PyCmdOrHole {
    PyCmdOrHole(mk_cmd(c.0.clone()))
}

/// A `Write` command.
#[pyfunction]
fn write(s: &str) -> PyStrCmd {
    PyStrCmd(mk_write(s))
}

/// A `Push` command.
#[pyfunction]
fn push(s: &str) -> PyStrCmd {
    PyStrCmd(mk_push(s))
}

/// An identifier term (the `⟨N⟩` operator).
#[pyfunction]
fn name(s: &str) -> PyQTerm {
    PyQTerm(mk_leaf("identifier", s))
}

/// Build the Python source that reconstructs `term`, recursively.
///
/// This is what `↑` on an already-built `QTerm` must produce, because `↑` is
/// governed by
///
/// ```text
/// ↓(↑(x)) == x
/// ```
///
/// `↑` maps a value to a term *whose code evaluates back to that value*, and `↓`
/// evaluates a term's code. For a plain value like `42` the code is `42`. For a
/// term, the code has to be a constructor call — `leaf("integer", "7")` — so
/// that evaluating it yields the term again.
///
/// It used to return the term unchanged, which reads as a sensible no-op but
/// breaks the law: `↓` then evaluated the term's *own* code (`7`) and produced
/// the integer 7 rather than the term. `test_main.py` asserted that
/// ("qlift is idempotent on terms"), so the test encoded the bug. Rust's
/// `QLift for Arc<QTerm>` has always done it correctly; this brings the Python
/// runtime into line. See issue #166.
fn lift_term(term: &Arc<QTerm>) -> Arc<QTerm> {
    use quilt::langs::python::ops;

    match &**term {
        QTerm::Tuple { tag, terms, cmds } => {
            let children: Vec<Arc<QTerm>> = terms.iter().map(lift_term).collect();
            ops::build_tuple_code(tag, cmds, &children)
        }
        QTerm::Quote {
            tag,
            index,
            lang,
            term,
            cmds,
            ..
        } => ops::build_quote_code(tag, *index, lang, &lift_term(term), cmds),
        QTerm::Unquote {
            tag,
            index,
            lang,
            term,
            cmds,
            ..
        } => ops::build_unquote_code(tag, *index, lang, &lift_term(term), cmds),
    }
}

/// Lift a Python value to a term that reconstructs it (the `↑` operator).
/// Supports `int`, `str`, and existing `QTerm`s.
#[pyfunction]
fn qlift(value: &Bound<'_, PyAny>) -> PyResult<PyQTerm> {
    if let Ok(q) = value.extract::<PyQTerm>() {
        return Ok(PyQTerm(lift_term(&q.0)));
    }
    // Bool *before* int: `bool` is a subclass of `int` in Python, so
    // `extract::<i64>()` succeeds on `True` and would lift it as `1` — silently
    // generating an integer where the author wrote a boolean. Found by the
    // shared runtime corpus (#159); the core library's `LiftTo<Python> for bool`
    // is the reference, and it produces `True`/`False`.
    if let Ok(b) = value.extract::<bool>() {
        let s = if b { "True" } else { "False" };
        return Ok(PyQTerm(mk_leaf(if b { "true" } else { "false" }, s)));
    }
    if let Ok(n) = value.extract::<i64>() {
        return Ok(PyQTerm(mk_leaf("integer", &n.to_string())));
    }
    // `{:?}` keeps the decimal point, so 1.0 lifts as the float `1.0` rather
    // than the integer `1` — matching `LiftTo<Python> for f64`.
    if let Ok(f) = value.extract::<f64>() {
        return Ok(PyQTerm(mk_leaf("float", &format!("{f:?}"))));
    }
    if let Ok(s) = value.extract::<String>() {
        // Shares the core's `py_string_term`, which owns both the escaping rule
        // and the tree shape. Writing `s` raw here produced literals that do not
        // parse (`a"b`) or that silently changed value (`a\\b`, where `\\b` is a
        // backspace escape) — found by the shared runtime corpus (#159) — and
        // reproducing the *shape* by hand let the two drift on the empty string,
        // which the parser spells with no `string_content` child at all.
        return Ok(PyQTerm(quilt::lift::py_string_term(&s)));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "qlift: unsupported type (expected bool, int, float, str, or QTerm)",
    ))
}

/// Lift a Python value to an HTML term (the `↑` operator with an `html`
/// splice target). Strings become entity-escaped `text` leaves — inert as
/// text content or as a double-quoted attribute value — and terms pass
/// through unchanged, so already-built fragments can be lifted too.
#[pyfunction]
fn qlift_html(value: &Bound<'_, PyAny>) -> PyResult<PyQTerm> {
    if let Ok(q) = value.extract::<PyQTerm>() {
        return Ok(q);
    }
    // Bool before int, for the same reason as `qlift`: `True` must render as
    // `True`, not `1`.
    if let Ok(b) = value.extract::<bool>() {
        return Ok(PyQTerm(mk_leaf("text", if b { "True" } else { "False" })));
    }
    if let Ok(n) = value.extract::<i64>() {
        return Ok(PyQTerm(mk_leaf("text", &n.to_string())));
    }
    if let Ok(f) = value.extract::<f64>() {
        return Ok(PyQTerm(mk_leaf("text", &format!("{f:?}"))));
    }
    if let Ok(s) = value.extract::<String>() {
        return Ok(PyQTerm(mk_leaf("text", &escape_html(&s))));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "qlift_html: unsupported type (expected bool, int, float, str, or QTerm)",
    ))
}

/// Escape `& < > " '` so the result is inert HTML wherever a hole can sit.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/**************************************************************/

/// Deserialize a `QTerm` from postcard bytes (the `rs↓` protocol in Python).
#[pyfunction]
fn from_postcard_bytes(data: &[u8]) -> PyResult<PyQTerm> {
    postcard::from_bytes::<Arc<QTerm>>(data)
        .map(PyQTerm)
        .map_err(|e| pyo3::exceptions::PyValueError::new_err(e.to_string()))
}

/// quilt's core IR, exposed to Python as the native `quilt._quilt` module
/// (re-exported by the `quilt` package's `__init__.py`).
#[pymodule]
fn _quilt(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyQTerm>()?;
    m.add_class::<PyStrCmd>()?;
    m.add_class::<PyCmdOrHole>()?;
    m.add_class::<PyBuilder>()?;

    m.add_function(wrap_pyfunction!(tb, m)?)?;
    m.add_function(wrap_pyfunction!(leaf, m)?)?;
    m.add_function(wrap_pyfunction!(sym, m)?)?;
    m.add_function(wrap_pyfunction!(quote, m)?)?;
    m.add_function(wrap_pyfunction!(unquote, m)?)?;
    m.add_function(wrap_pyfunction!(cmd, m)?)?;
    m.add_function(wrap_pyfunction!(write, m)?)?;
    m.add_function(wrap_pyfunction!(push, m)?)?;
    m.add_function(wrap_pyfunction!(name, m)?)?;
    m.add_function(wrap_pyfunction!(qlift, m)?)?;
    m.add_function(wrap_pyfunction!(qlift_html, m)?)?;
    m.add_function(wrap_pyfunction!(from_postcard_bytes, m)?)?;

    m.add("NL", PyStrCmd(StrCmd::NewLine))?;
    m.add("POP", PyStrCmd(StrCmd::Pop))?;
    m.add("HOLE", PyCmdOrHole(CmdOrHole::Hole))?;
    Ok(())
}
