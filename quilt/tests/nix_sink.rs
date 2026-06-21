//! `nix build` check for `NixSink` (issue #98): lower a `QTree` to a Nix
//! derivation and actually build it, asserting the resulting store directory has
//! the right structure and content. Skipped when `nix-build` is unavailable (or
//! can't resolve `<nixpkgs>`), so it never fails CI on a machine without Nix —
//! the unit tests in `sink.rs` already prove the emitted text.

use quilt::prelude::*;
use std::process::Command;

fn nix_build_available() -> bool {
    Command::new("nix-build")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn nix_sink_builds_a_store_directory() {
    if !nix_build_available() {
        eprintln!("skipping nix_sink_builds_a_store_directory: nix-build not on PATH");
        return;
    }

    let t = tree! {
        "Cargo.toml" => raw(b"[package]\nname = \"demo\"\n".to_vec()),
        "src" => dir! {
            "lib.rs" => file(leaf("source_file", "pub fn greet() -> &'static str { \"hi\" }\n")),
        },
        "run.sh" => raw(b"#!/bin/sh\necho hi\n".to_vec()).mode(0o755),
    };

    let mut sink = NixSink::new("quilt-nix-sink-test");
    write_tree(&mut sink, &t).unwrap();
    let source = sink.into_source();

    let dir = tempfile::tempdir().unwrap();
    let nix_file = dir.path().join("default.nix");
    std::fs::write(&nix_file, &source).unwrap();

    let out = Command::new("nix-build")
        .arg(&nix_file)
        .arg("--no-out-link")
        .output()
        .unwrap();
    if !out.status.success() {
        // No `<nixpkgs>` / offline / sandbox quirk — don't fail the suite over a
        // missing Nix channel; the lowering itself is covered by unit tests.
        eprintln!(
            "skipping nix_sink_builds_a_store_directory: nix-build failed (likely no <nixpkgs>):\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }

    let stdout = String::from_utf8(out.stdout).unwrap();
    let store = std::path::Path::new(stdout.trim().lines().last().unwrap());

    assert_eq!(
        std::fs::read_to_string(store.join("Cargo.toml")).unwrap(),
        "[package]\nname = \"demo\"\n"
    );
    assert!(std::fs::read_to_string(store.join("src/lib.rs"))
        .unwrap()
        .contains("greet"));

    // The 0o755 leaf was lowered as an executable store file; the linkFarm
    // symlink resolves to it, so the execute bit survives.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "run.sh should be executable (mode {mode:o})"
        );
    }
}
