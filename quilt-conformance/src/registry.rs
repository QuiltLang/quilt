//! Per-language construction, kept deliberately lazy.
//!
//! `Omni::default()` and `dict_omni_language()` build *every* language's
//! tree-sitter parser eagerly. That is the right default for the CLI, which
//! needs the whole registry, but it is the wrong default for a test: the
//! existing suite calls `Omni::default()` inside nearly every `#[test]` fn, so
//! a run pays for ten parser constructions per test — including Lean's, whose
//! vendored `parser.c` is ~44 MB (#134).
//!
//! The battery instead builds exactly the one `Language` a probe needs, once
//! per language, and reuses it across that language's whole fragment corpus.
//! With one `#[test]` per language the work also fans out across libtest's
//! threads, so the wall-clock cost is one parser construction, not ten.

use miette::{miette, Result};
use quilt::lang::{Language, LanguagePost};
use quilt::meta::MetaLanguage;
use quilt::prelude::*;

/// The object-safe `Language` the registry hands out.
pub type BoxLang = Box<dyn Language<Post = Box<dyn LanguagePost>>>;

/// Canonical names of every registered language, in matrix order. Kept in step
/// with `langs/omni.rs` by `tests/conformance.rs::registry_matches_omni`.
pub const LANGUAGES: &[&str] = &[
    "bash",
    "html",
    "lean",
    "nix",
    "python",
    "rust",
    "text",
    "typescript",
    "wgsl",
    "zsh",
];

/// Canonical names of every registered *host* (a language with a `MetaLanguage`).
pub const HOSTS: &[&str] = &["lean", "nix", "python", "rust", "typescript"];

/// Build one language. Expensive — call once per language, not once per probe.
pub fn language(name: &str) -> Result<BoxLang> {
    use quilt::langs;
    Ok(match name {
        "bash" => bx(langs::bash::lang::DynBashLanguage::default()),
        "html" => bx(langs::html::lang::DynHtmlLanguage::default()),
        "lean" => bx(langs::lean::lang::DynLeanLanguage::default()),
        "nix" => bx(langs::nix::lang::DynNixLanguage::default()),
        "python" => bx(langs::python::lang::DynPythonLanguage::default()),
        "rust" => bx(langs::rust::lang::DynRustLanguage::default()),
        "text" => bx(langs::text::lang::DynTextLanguage),
        "typescript" => bx(langs::typescript::lang::DynTypeScriptLanguage::default()),
        "wgsl" => bx(langs::wgsl::lang::DynWgslLanguage::default()),
        "zsh" => bx(langs::zsh::lang::DynZshLanguage::default()),
        _ => return Err(miette!("no Language registered for {name:?}")),
    })
}

/// Build one meta-language, if the language has one. Cheap: every meta is a
/// unit struct, so unlike `language` this needs no batching.
pub fn meta(name: &str) -> Option<Box<dyn MetaLanguage>> {
    use quilt::langs;
    Some(match name {
        "lean" => bx(langs::lean::meta::LeanMetaLanguage),
        "nix" => bx(langs::nix::meta::NixMetaLanguage),
        "python" => bx(langs::python::meta::PythonMetaLanguage),
        "rust" => bx(langs::rust::meta::RustMetaLanguage),
        "typescript" => bx(langs::typescript::meta::TypeScriptMetaLanguage),
        _ => return None,
    })
}
