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

    /// Eval this term's source and return the resulting Python value (the `↓` operator).
    fn reduce<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let code = self.0.coparse();
        py.eval(&std::ffi::CString::new(code).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(e.to_string())
        })?, None, None)
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

/// Lift a Python value to a term that reconstructs it (the `↑` operator).
/// Supports `int`, `str`, and existing `QTerm`s.
#[pyfunction]
fn qlift(value: &Bound<'_, PyAny>) -> PyResult<PyQTerm> {
    if let Ok(q) = value.extract::<PyQTerm>() {
        return Ok(q);
    }
    if let Ok(n) = value.extract::<i64>() {
        return Ok(PyQTerm(mk_leaf("integer", &n.to_string())));
    }
    if let Ok(s) = value.extract::<String>() {
        let t = mk_tb("string")
            .c(&mk_leaf("string_start", "\""))
            .c(&mk_leaf("string_content", &s))
            .c(&mk_leaf("string_end", "\""))
            .b();
        return Ok(PyQTerm(t));
    }
    Err(pyo3::exceptions::PyTypeError::new_err(
        "qlift: unsupported type (expected int, str, or QTerm)",
    ))
}

/**************************************************************/

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

    m.add("NL", PyStrCmd(StrCmd::NewLine))?;
    m.add("POP", PyStrCmd(StrCmd::Pop))?;
    m.add("HOLE", PyCmdOrHole(CmdOrHole::Hole))?;
    Ok(())
}
