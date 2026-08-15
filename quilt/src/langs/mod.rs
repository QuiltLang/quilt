pub mod arity;
#[cfg(feature = "bash")]
pub mod bash;
#[cfg(feature = "bootstrap")]
pub mod bootstrap;
#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "lean")]
pub mod lean;
#[cfg(feature = "nix")]
pub mod nix;
#[cfg(feature = "parse")]
pub mod omni;
#[cfg(feature = "python")]
pub mod python;
#[cfg(any(feature = "rust", feature = "bootstrap"))]
pub mod rust;
#[cfg(any(feature = "bash", feature = "zsh"))]
pub mod shell;
#[cfg(feature = "sql")]
pub mod sql;
#[cfg(feature = "text")]
pub mod text;
#[cfg(feature = "typescript")]
pub mod typescript;
#[cfg(feature = "wgsl")]
pub mod wgsl;
#[cfg(feature = "zsh")]
pub mod zsh;

/// The line-comment introducer for a generated file in `lang`, by canonical
/// name or alias.
///
/// The `DO NOT EDIT` header the CLI prepends has to be a comment *in the
/// language it just generated*. That mapping used to live as a hardcoded match
/// on the file extension inside `bin.rs`, structurally disconnected from the
/// language registry — so a new host language silently inherited Rust's `//!`,
/// which is exactly what issue #136 was. Keeping it here, next to the language
/// modules, means adding a host is one place; the conformance battery (#144)
/// additionally fails if a host has no entry, so it cannot be forgotten.
///
/// Returns `None` for a name that is not a registered language, letting the
/// caller decide the fallback.
#[must_use]
pub fn comment_prefix(lang: &str) -> Option<&'static str> {
    Some(match lang {
        // Rust keeps `//!`, an inner doc comment: it documents the generated
        // module rather than whatever item happens to follow.
        "rust" | "rs" => "//!",
        "lean" | "lean4" | "sql" | "mysql" | "mariadb" => "--",
        // Grouped by spelling rather than by language because clippy's
        // `match_same_arms` (pedantic, `-D warnings` in CI) rejects the
        // one-language-per-line form.
        "python" | "py" | "nix" | "bash" | "zsh" | "sh" => "#",
        "typescript" | "ts" | "wgsl" => "//",
        // Deliberately absent:
        //
        // * `html` — its comments are *delimited* (`<!-- … -->`), and a prefix
        //   alone would emit an unterminated comment. A header for HTML needs a
        //   suffix too, which this API cannot express; it is unreachable today
        //   because HTML is target-only and never the ground language.
        // * `text` — plain text has no comment syntax, so there is no spelling
        //   that would not corrupt the output.
        //
        // Both fall through to the caller's fallback rather than being given a
        // wrong answer here.
        _ => return None,
    })
}
