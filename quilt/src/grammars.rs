//! Vendored tree-sitter grammars for the languages Quilt can parse.
//!
//! The generated `parser.c` / `scanner.c` for each language live under
//! `quilt/grammars/<lang>/` and are compiled by `build.rs` (gated on the same
//! Cargo features as the bindings below). They are **generated** — never edit
//! them by hand. Regenerate from the pinned `QuiltLang/tree-sitter-*` forks with
//! `bin/sync-grammars`; the pins (git url + rev) live in the workspace
//! `Cargo.toml` `[workspace.dependencies]`, and CI's `bin/check-grammars` fails
//! if the vendored copies drift from them.
//!
//! Each `LANGUAGE` is the same [`LanguageFn`] the upstream `tree-sitter-<lang>`
//! crate exposes, so callers use it identically:
//! `parser.set_language(&grammars::rust::LANGUAGE.into())`.
//!
//! [`LanguageFn`]: tree_sitter_language::LanguageFn

/// Declare a vendored grammar: a module exposing `$konst: LanguageFn` backed by
/// the generated parser's `extern "C"` entry point (`$func`), gated on `$feat`.
macro_rules! grammar {
    ($feat:literal, $module:ident, $konst:ident, $func:ident) => {
        #[cfg(feature = $feat)]
        pub mod $module {
            use tree_sitter_language::LanguageFn;

            extern "C" {
                fn $func() -> *const ();
            }

            /// The tree-sitter [`LanguageFn`](tree_sitter_language::LanguageFn)
            /// for this vendored grammar.
            pub const $konst: LanguageFn = unsafe { LanguageFn::from_raw($func) };
        }
    };
}

// Host languages: their parsers are tied to the `parse` umbrella feature (see
// `build.rs` and `Cargo.toml`), so they are always present whenever this module
// is compiled.
grammar!("parse", rust, LANGUAGE, tree_sitter_rust);
grammar!("parse", python, LANGUAGE, tree_sitter_python);

// Target languages: each behind its own feature (which implies `parse`).
grammar!(
    "typescript",
    typescript,
    LANGUAGE_TYPESCRIPT,
    tree_sitter_typescript
);
grammar!("wgsl", wgsl, LANGUAGE, tree_sitter_wgsl);
grammar!("bash", bash, LANGUAGE, tree_sitter_bash);
grammar!("html", html, LANGUAGE, tree_sitter_html);
grammar!("zsh", zsh, LANGUAGE, tree_sitter_zsh);
