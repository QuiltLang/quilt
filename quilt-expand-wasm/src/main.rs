//! The Quilt expander as a WASI command, for in-browser expansion (issue #47).
//!
//! Reads a `.quilt` source from **stdin**, takes the language chain (ground
//! language first, e.g. `ts html` for the `.html.ts` chain) from **argv**, and
//! writes the expanded source to **stdout**. Errors go to stderr with a
//! non-zero exit. This is the full pipeline — parse → `QTerm` → expand →
//! `coparse` — so the browser playground can expand edited source live with no
//! server and no offline `quilt expand` step.
//!
//! A WASI command (not wasm-bindgen) because expansion pulls in tree-sitter and
//! the C grammars, which need a libc; the browser runs it through a small WASI
//! shim that wires argv / stdin / stdout to in-memory buffers.

use std::io::{Read, Write};

use quilt::langs::omni::Omni;
use quilt::term::STerm;

fn main() {
    // Chain, ground language first. Defaults to the demo's `.html.ts` chain.
    let chain: Vec<String> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            vec!["ts".to_string(), "html".to_string()]
        } else {
            args
        }
    };
    let chain: Vec<&str> = chain.iter().map(String::as_str).collect();

    let mut source = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut source) {
        eprintln!("quilt-expand-wasm: failed to read stdin: {e}");
        std::process::exit(2);
    }

    match expand(&chain, &source) {
        Ok(out) => {
            let mut stdout = std::io::stdout();
            let _ = stdout.write_all(out.as_bytes());
            let _ = stdout.flush();
        }
        Err(report) => {
            // Render the miette diagnostic to stderr and fail.
            eprintln!("{report:?}");
            std::process::exit(1);
        }
    }
}

/// Parse `source` under `chain` and expand it to flat source — the same two
/// steps `quilt expand` runs (`parse_chain` then `expand_lang` on the ground
/// language), then `coparse`.
fn expand(chain: &[&str], source: &str) -> miette::Result<String> {
    let mut omni = Omni::default();
    let sterm = omni.parse_chain(chain, source)?;
    let expanded = omni.expand_lang(chain[0], &sterm)?;
    Ok(expanded.coparse())
}
