//! Compile the vendored tree-sitter parsers under `grammars/<lang>/` into the
//! crate. Each grammar is gated on a Cargo feature, mirroring `src/grammars.rs`:
//!
//! * `rust` / `python` — the host parsers, tied to the `parse` umbrella feature.
//! * `typescript` / `wgsl` / `bash` / `html` / `zsh` / `nix` — each behind its
//!   own feature (all of which imply `parse`).
//!
//! A `default-features = false` build (e.g. nanobots on `wasm32`) enables none
//! of these, so no C is compiled and the crate stays tree-sitter-free.

use std::path::Path;

/// Is the Cargo feature `name` enabled for this build?
fn feature(name: &str) -> bool {
    std::env::var_os(format!("CARGO_FEATURE_{}", name.to_uppercase())).is_some()
}

/// The directory holding a grammar's generated `parser.c`/`scanner.c`. Most
/// grammars vendor it flat at `grammars/<lang>/`; typescript keeps the fork's
/// nested layout (`typescript/src/` beside a shared `common/`) so its scanner's
/// `#include "../../common/scanner.h"` resolves without patching generated code.
fn src_dir(lang: &str) -> std::path::PathBuf {
    match lang {
        "typescript" => Path::new("grammars")
            .join("typescript")
            .join("typescript")
            .join("src"),
        _ => Path::new("grammars").join(lang),
    }
}

/// Compile a grammar's `parser.c` (+ `scanner.c` if present) into a static lib.
/// The grammar's own directory is on the include path so its generated
/// `#include "tree_sitter/parser.h"` (and sibling headers like html's `tag.h`)
/// resolve.
fn build(lang: &str) {
    let dir = src_dir(lang);

    let mut cc = cc::Build::new();
    cc.std("c11").include(&dir);

    #[cfg(target_env = "msvc")]
    cc.flag("-utf-8");

    cc.file(dir.join("parser.c"));

    let scanner = dir.join("scanner.c");
    if scanner.exists() {
        cc.file(&scanner);
    }

    cc.compile(&format!("tree_sitter_{lang}"));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Rebuild if any vendored parser/scanner/header changes (e.g. after
    // `bin/sync-grammars`).
    println!("cargo:rerun-if-changed=grammars");

    if !feature("parse") {
        return;
    }

    build("rust");
    build("python");

    for lang in ["typescript", "wgsl", "bash", "html", "zsh", "nix"] {
        if feature(lang) {
            build(lang);
        }
    }
}
