//! Tier B: host-backed holes via a generated render wrapper (issue #89).

use quilt::langs::omni::Omni;
use quilt::prelude::*;
use quilt::template::{tier_b_program, ParamValue};
use quilt::term::STerm;
use std::path::PathBuf;
use std::process::Command;

fn params(pairs: &[(&str, ParamValue)]) -> Vec<(Box<str>, ParamValue)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), v.clone()))
        .collect()
}

/// The generated wrapper binds each param, wraps the body in a `target↖ … ↗`
/// quote, and prints its coparse.
#[test]
fn wrapper_structure() {
    let prog = tier_b_program(
        "py",
        "py",
        "X = ↙↑(greeting)↘\n",
        &params(&[("greeting", "Hi".into())]),
    )
    .unwrap();
    assert!(prog.contains("from quilt import *"), "{prog}");
    assert!(prog.contains(r#"greeting = "Hi""#), "{prog}");
    assert!(prog.contains("py↖"), "{prog}");
    assert!(prog.contains("X = ↙↑(greeting)↘"), "{prog}");
    assert!(prog.contains("coparse()"), "{prog}");
}

#[test]
fn non_python_host_errors() {
    let err = tier_b_program("rs", "py", "let x = 1;\n", &[]).unwrap_err();
    assert!(err.to_string().contains("Python host"), "{err}");
}

/// The wrapped loop/conditional template is a valid quilt metaprogram: it parses
/// and expands to host (Python) code with no errors. Exercises the whole Tier B
/// wrap + expand path deterministically, without running anything.
#[test]
fn loop_conditional_wrapper_expands() -> Result<()> {
    let body =
        "ROSTER = ↙↑(\", \".join(names))↘\nTONE = ↙↑(\"formal\" if formal else \"casual\")↘\n";
    let prog = tier_b_program(
        "py",
        "py",
        body,
        &params(&[
            (
                "names",
                ParamValue::List(vec!["Ada".into(), "Linus".into()]),
            ),
            ("formal", ParamValue::Bool(false)),
        ]),
    )?;
    let mut omni = Omni::default();
    let st = omni.parse_chain(&["py"], &prog)?;
    let py = omni.expand_lang("py", &st)?.coparse();
    // Each hole expands to a `qlift(...)` runtime call; the program ends in a
    // `coparse()`.
    assert!(py.contains("qlift"), "expanded: {py}");
    assert!(py.contains("coparse"), "expanded: {py}");
    Ok(())
}

/// End-to-end: wrap, expand, and actually run the Python metaprogram, asserting
/// the instantiated output. Skipped unless a built `quilt` Python package is
/// present (a fresh `cargo test` without `build-py` won't have one).
#[test]
fn end_to_end_run() -> Result<()> {
    let Some(pythonpath) = quilt_pythonpath() else {
        eprintln!("skipping end_to_end_run: no built quilt-python found");
        return Ok(());
    };
    let body = "GREETING = ↙↑(greeting)↘\nROSTER = ↙↑(\", \".join(names))↘\nTONE = ↙↑(\"formal\" if formal else \"casual\")↘\n";
    let prog = tier_b_program(
        "py",
        "py",
        body,
        &params(&[
            ("greeting", "Hello".into()),
            (
                "names",
                ParamValue::List(vec!["Ada".into(), "Grace".into()]),
            ),
            ("formal", ParamValue::Bool(true)),
        ]),
    )?;
    let mut omni = Omni::default();
    let st = omni.parse_chain(&["py"], &prog)?;
    let py = omni.expand_lang("py", &st)?.coparse();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("render.py");
    std::fs::write(&file, &*py).unwrap();
    let out = Command::new("python3")
        .env("PYTHONPATH", &pythonpath)
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(r#"GREETING = "Hello""#), "{stdout}");
    assert!(stdout.contains(r#"ROSTER = "Ada, Grace""#), "{stdout}"); // host loop (join)
    assert!(stdout.contains(r#"TONE = "formal""#), "{stdout}"); // host conditional
    Ok(())
}

/// The dir to put on `PYTHONPATH` for a built `quilt` package next to this crate
/// (`<dir>/quilt/_quilt.abi3.so`), or `None` to skip the run test.
fn quilt_pythonpath() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../quilt-python")
        .canonicalize()
        .ok()?;
    dir.join("quilt/_quilt.abi3.so").exists().then_some(dir)
}
