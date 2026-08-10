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
pub const HOSTS: &[&str] = &["bash", "lean", "nix", "python", "rust", "typescript", "zsh"];

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

/// The vendored tree-sitter grammar for a language, or `None` when it has none.
///
/// Only `text` has none — it implements `Language` directly, with a single `text`
/// tag and no grammar to check against. Everything else is tree-sitter backed,
/// which is what lets a test ask the grammar whether a hand-written tag is a
/// node kind it actually defines.
#[must_use]
pub fn grammar(name: &str) -> Option<tree_sitter::Language> {
    use quilt::grammars;
    Some(match name {
        "bash" => grammars::bash::LANGUAGE.into(),
        "html" => grammars::html::LANGUAGE.into(),
        "lean" => grammars::lean::LANGUAGE.into(),
        "nix" => grammars::nix::LANGUAGE.into(),
        "python" => grammars::python::LANGUAGE.into(),
        "rust" => grammars::rust::LANGUAGE.into(),
        "typescript" => grammars::typescript::LANGUAGE_TYPESCRIPT.into(),
        "wgsl" => grammars::wgsl::LANGUAGE.into(),
        "zsh" => grammars::zsh::LANGUAGE.into(),
        // `text` is not tree-sitter backed.
        _ => return None,
    })
}

/// Every node kind a grammar defines, named and anonymous alike.
///
/// This is the authority a hand-written tag table can be checked against:
/// tree-sitter exposes its whole symbol table, so a tag that is misspelled — or
/// that a grammar bump renamed — is detectable instead of silently falling
/// through to `Arity::Unknown`.
#[must_use]
pub fn node_kinds(lang: &tree_sitter::Language) -> std::collections::BTreeSet<&'static str> {
    (0..lang.node_kind_count())
        .filter_map(|id| u16::try_from(id).ok())
        .filter_map(|id| lang.node_kind_for_id(id))
        .collect()
}

/// Build one meta-language, if the language has one. Cheap: every meta is a
/// unit struct (the shells' is a `PhantomData` newtype), so unlike `language`
/// this needs no batching.
pub fn meta(name: &str) -> Option<Box<dyn MetaLanguage>> {
    use quilt::langs;
    Some(match name {
        "bash" => bx(langs::shell::meta::BashMetaLanguage::default()),
        "zsh" => bx(langs::shell::meta::ZshMetaLanguage::default()),
        "lean" => bx(langs::lean::meta::LeanMetaLanguage),
        "nix" => bx(langs::nix::meta::NixMetaLanguage),
        "python" => bx(langs::python::meta::PythonMetaLanguage),
        "rust" => bx(langs::rust::meta::RustMetaLanguage),
        "typescript" => bx(langs::typescript::meta::TypeScriptMetaLanguage),
        _ => return None,
    })
}
