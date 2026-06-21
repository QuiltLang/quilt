//! End-to-end tests for the `quilt instantiate` CLI (issue #88), driving the
//! built binary the way a user would.

use std::io::Write;
use std::process::Command;
use tempfile::Builder;

/// A `*.tmpl.quilt` file holding `content`, kept alive for the test's duration.
fn tmpl(content: &str) -> tempfile::NamedTempFile {
    let mut f = Builder::new().suffix(".py.tmpl.quilt").tempfile().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f
}

fn quilt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quilt"))
}

#[test]
fn set_scalars_to_stdout() {
    let t = tmpl("x = ↙n↘\ny = ↙name↘\n");
    let out = quilt()
        .args(["instantiate"])
        .arg(t.path())
        .args(["--set", "n=5", "--set", "name=bob"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("x = 5"), "got: {stdout:?}");
    assert!(stdout.contains(r#"y = "bob""#), "got: {stdout:?}");
    assert!(!stdout.contains('↙'), "holes should be filled: {stdout:?}");
}

#[test]
fn values_toml_supplies_a_list() {
    let t = tmpl("xs = ↙items↘\n");
    let mut vals = Builder::new().suffix(".toml").tempfile().unwrap();
    vals.write_all(b"items = [1, 2, 3]\n").unwrap();
    let out = quilt()
        .args(["instantiate"])
        .arg(t.path())
        .arg("--values")
        .arg(vals.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("xs = [1, 2, 3]"), "got: {stdout:?}");
}

#[test]
fn set_overrides_values() {
    let t = tmpl("g = ↙greeting↘\n");
    let mut vals = Builder::new().suffix(".toml").tempfile().unwrap();
    vals.write_all(b"greeting = \"from-file\"\n").unwrap();
    let out = quilt()
        .args(["instantiate"])
        .arg(t.path())
        .arg("--values")
        .arg(vals.path())
        .args(["--set", "greeting=from-cli"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains(r#"g = "from-cli""#), "got: {stdout:?}");
}

#[test]
fn missing_param_fails_listing_all() {
    let t = tmpl("a = ↙x↘\nb = ↙y↘\n");
    let out = quilt()
        .args(["instantiate"])
        .arg(t.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("missing template parameter(s)"),
        "got: {stderr:?}"
    );
    assert!(
        stderr.contains('x') && stderr.contains('y'),
        "got: {stderr:?}"
    );
}

#[test]
fn out_writes_a_file() {
    let t = tmpl("v = ↙val↘\n");
    let dir = tempfile::tempdir().unwrap();
    let out_path = dir.path().join("out.py");
    let status = quilt()
        .args(["instantiate"])
        .arg(t.path())
        .arg("--out")
        .arg(&out_path)
        .args(["--set", "val=7"])
        .status()
        .unwrap();
    assert!(status.success());
    let written = std::fs::read_to_string(&out_path).unwrap();
    assert!(written.contains("v = 7"), "got: {written:?}");
}

#[test]
fn non_template_file_is_rejected() {
    // A plain .quilt (not .tmpl.quilt) is not a template.
    let mut f = Builder::new().suffix(".py.quilt").tempfile().unwrap();
    f.write_all(b"x = 1\n").unwrap();
    let out = quilt()
        .args(["instantiate"])
        .arg(f.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("tmpl.quilt"), "got: {stderr:?}");
}
