//! End-to-end tests for `quilt instantiate <dir> --out <dir>` (issue #90):
//! instantiate a whole template directory and materialize it, driving the built
//! binary the way a user would.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn quilt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quilt"))
}

/// A template directory laid out from `(relative path, contents)` pairs.
fn template_dir(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, contents) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    dir
}

#[test]
fn instantiates_a_directory_to_out() {
    let tmpl = template_dir(&[
        ("app.py.tmpl.quilt", "name = ↙who↘\nn = ↙count↘\n"),
        ("src/util.py.tmpl.quilt", "tag = ↙who↘\n"),
        ("README.md", "fixed asset\n"),
    ]);
    let out = TempDir::new().unwrap();

    let result = quilt()
        .args(["instantiate"])
        .arg(tmpl.path())
        .arg("--out")
        .arg(out.path())
        .args(["--set", "who=bob", "--set", "count=3"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    // Template files: marker dropped, holes filled.
    assert_eq!(
        fs::read_to_string(out.path().join("app.py")).unwrap(),
        "name = \"bob\"\nn = 3\n"
    );
    assert_eq!(
        fs::read_to_string(out.path().join("src/util.py")).unwrap(),
        "tag = \"bob\"\n"
    );
    // Asset copied verbatim.
    assert_eq!(
        fs::read_to_string(out.path().join("README.md")).unwrap(),
        "fixed asset\n"
    );
}

#[test]
fn directory_without_out_is_rejected() {
    let tmpl = template_dir(&[("x.py.tmpl.quilt", "v = ↙v↘\n")]);
    let result = quilt()
        .args(["instantiate"])
        .arg(tmpl.path())
        .args(["--set", "v=1"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(stderr.contains("--out"), "got: {stderr}");
}

#[test]
fn missing_param_fails_listing_all() {
    let tmpl = template_dir(&[("t.py.tmpl.quilt", "a = ↙p↘\nb = ↙q↘\n")]);
    let out = TempDir::new().unwrap();
    let result = quilt()
        .args(["instantiate"])
        .arg(tmpl.path())
        .arg("--out")
        .arg(out.path())
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert!(
        stderr.contains("missing template parameter"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains('p') && stderr.contains('q'),
        "got: {stderr}"
    );
}

#[test]
fn existing_conflict_is_refused_by_default() {
    let tmpl = template_dir(&[("f.py.tmpl.quilt", "v = ↙v↘\n")]);
    let out = TempDir::new().unwrap();
    // Pre-existing file at the target path: the default policy refuses.
    fs::write(out.path().join("f.py"), "mine\n").unwrap();
    let result = quilt()
        .args(["instantiate"])
        .arg(tmpl.path())
        .arg("--out")
        .arg(out.path())
        .args(["--set", "v=1"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    // The user's file is left untouched.
    assert_eq!(
        fs::read_to_string(out.path().join("f.py")).unwrap(),
        "mine\n"
    );
}
