//! Regenerate `conformance/support-matrix.json` and `docs/wiki/support-matrix.md`.
//!
//! Run via `bin/gen-matrix`. `bin/check-matrix` runs the same thing and fails if
//! the working tree changed, so a capability claim that stops being true — or a
//! capability that quietly starts working — shows up as a one-line diff in the
//! PR rather than as a stale website table.

use miette::{IntoDiagnostic as _, Result, WrapErr as _};
use quilt_conformance::{matrix_json_path, matrix_md_path, run_all};
use std::fs;

fn main() -> Result<()> {
    let (matrix, failures) = run_all()?;

    // Write the artifacts even when probes failed: a failing run is exactly
    // when you want to see what the matrix *would* say, and `cargo test` is the
    // gate that stops a bad matrix from being committed.
    let json_path = matrix_json_path();
    let md_path = matrix_md_path();

    fs::write(&json_path, matrix.to_json())
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", json_path.display()))?;
    fs::write(&md_path, matrix.to_markdown())
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", md_path.display()))?;

    let cells = matrix.rows.iter().map(|r| r.cells.len()).sum::<usize>();
    let verified = matrix
        .rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| c.is_verified())
        .count();

    println!("wrote {}", json_path.display());
    println!("wrote {}", md_path.display());
    println!(
        "{} language(s), {cells} cell(s), {verified} verified by a probe, {} declaration-only",
        matrix.rows.len(),
        cells - verified,
    );

    if failures.is_empty() {
        println!("all probes passed ✓");
        Ok(())
    } else {
        eprintln!("\n{} probe failure(s):", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
        eprintln!("\nThe matrix above reflects the *claims*, not these failures.");
        eprintln!("Run `cargo test -p quilt-conformance` for the full diagnostics.");
        std::process::exit(1);
    }
}
