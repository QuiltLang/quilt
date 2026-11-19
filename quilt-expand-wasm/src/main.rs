//! The Quilt expander as a WASI command, for in-browser expansion (issue #47).
//!
//! Reads a `.quilt` source from **stdin**, takes the language chain (ground
//! language first, e.g. `ts html` for the `.html.ts` chain) from **argv**, and
//! writes the expanded source to **stdout**. Errors go to stderr with a
//! non-zero exit. This is the full pipeline — parse → `QTerm` → expand →
//! `coparse` — so the browser playground can expand edited source live with no
//! server and no offline `quilt expand` step.
//!
//! Like `quilt expand`, the output opens with the host's runtime import when
//! the source uses a Quilt construct and did not write one (issue #274): an
//! expanded TypeScript module calls `tb(..)`, `qlift(..)` and the rest, and
//! without the import a browser that loads it as a module fails on the first
//! call. `--no-prelude` leaves it out, as `quilt expand --no-prelude` does, for
//! a caller that evaluates the result in a scope already holding the runtime
//! (the `↓` reduce in `examples/web/quilt-rt.js`, where an ESM `import` inside
//! `new Function` is a syntax error).
//!
//! A WASI command (not wasm-bindgen) because expansion pulls in tree-sitter and
//! the C grammars, which need a libc; the browser runs it through a small WASI
//! shim that wires argv / stdin / stdout to in-memory buffers.

use std::io::{Read, Write};

use quilt::langs::omni::Omni;
use quilt::meta::with_prelude;
use quilt::term::STerm;

fn main() {
    // Chain, ground language first. Defaults to the demo's `.html.ts` chain.
    let mut prelude = true;
    let chain: Vec<String> = {
        let args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| {
                let flag = a == "--no-prelude";
                prelude &= !flag;
                !flag
            })
            .collect();
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

    match expand(&chain, &source, prelude) {
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

/// Parse `source` under `chain` and expand it to flat source — the same steps
/// `quilt expand` runs (`parse_chain` then `expand_lang` on the ground
/// language, then `coparse`), opened with the runtime import it would write
/// unless `prelude` is off.
fn expand(chain: &[&str], source: &str, prelude: bool) -> miette::Result<String> {
    let mut omni = Omni::default();
    let sterm = omni.parse_chain(chain, source)?;
    let body = omni.expand_lang(chain[0], &sterm)?.coparse();
    Ok(match omni.prelude_for(chain, source).filter(|_| prelude) {
        Some(p) => with_prelude(&body, &p.text, chain[0]),
        None => body,
    })
}

#[cfg(test)]
mod tests {
    use super::expand;

    /// The expander opens its output with the runtime import, as `quilt
    /// expand` does (issue #274): the browser loads a Stage-1 module straight
    /// from it, and with no import its first `tb(…)` is a `ReferenceError`.
    /// `--no-prelude` leaves it out, for reduce (`↓`), which evaluates the
    /// result with the runtime already in scope.
    #[test]
    fn the_runtime_import_is_written_unless_turned_off() {
        let src = "// a module\nexport const f = () => html↖<p>hi</p>↗;\n";
        let out = expand(&["ts", "html"], src, true).unwrap();
        assert!(
            out.starts_with("// a module\nimport { tb,") && out.contains(" } from \"quilt\";\n"),
            "{out}"
        );
        let bare = expand(&["ts", "html"], src, false).unwrap();
        assert!(!bare.contains("from \"quilt\""), "{bare}");
        // A source that imports the runtime itself keeps its own import alone.
        let own = format!("import {{ tb, leaf }} from \"quilt\";\n{src}");
        let out = expand(&["ts", "html"], &own, true).unwrap();
        assert_eq!(out.matches("from \"quilt\"").count(), 1, "{out}");
    }
}
