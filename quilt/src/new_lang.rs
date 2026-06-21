//! `quilt new-lang` — scaffold a new language module (issue #108, the epic's
//! capstone: dogfooding directory templating to stub out Quilt's own internals).
//!
//! Adding a language today is a manual copy of `langs/<lang>/{lang,meta,ops}.rs`
//! plus registration in `omni.rs` and a feature flag (see
//! `docs/wiki/adding-a-language.md`). That is a *parameterized, conditional*
//! directory scaffold — the first internal user of this epic.
//!
//! This is **development tooling**, not a user-facing `quilt` subcommand: it is
//! driven by the `bin/new-lang` scaffold program (run through `quilt scaffold`,
//! issue #95), which collects a [`LangSpec`], calls [`build_lang_tree`] here, and
//! materializes the result through an [`FsSink`](crate::sink::FsSink) honoring
//! `--on-conflict` (it writes into the live repo). This module holds the testable
//! logic; it is not re-exported from the prelude.
//!
//! **Why raw `{{name}}` substitution, not Tier A.** The directory-templating
//! Tier A path ([`crate::template::instantiate`]) lifts a parameter into the
//! hole's object language — a *string literal* in Rust. That is right for
//! *data*, but here the holes are *identifiers and type names* spliced into Rust
//! source (`{{Lang}}Language`, `pub mod {{lang}}`), which must stay bare. So the
//! fragments are plain-text templates with `{{name}}` holes (the byte-safe
//! spelling from issue #91) filled by [`render`], not Tier A. Conditional files
//! (a host gets `meta.rs` + `ops.rs`; a target-only language gets just `lang.rs`)
//! are exactly the *program-assembles-a-QTree* case (issue #95).
//!
//! The generated module mirrors the existing `langs::text` stub: it compiles as
//! `todo!()` stubs once wired in, and `INTEGRATION.md` (also generated) lists
//! the exact `omni.rs` / `langs/mod.rs` / `Cargo.toml` edits — the registration
//! snippet — that finish the job.

use crate::tree::{raw, QTree};
use miette::{bail, Result};

/// What the author answered in the wizard — the parameters of the scaffold.
#[derive(Debug, Clone)]
pub struct LangSpec {
    /// Canonical language key (a valid lowercase identifier), e.g. `ruby`.
    pub key: String,
    /// Extra names that resolve to the same language, e.g. `rb`.
    pub aliases: Vec<String>,
    /// Human display name for docs/comments, e.g. `Ruby`.
    pub display: String,
    /// A **host** language gets a `MetaLanguage` (`meta.rs`) and runtime ops
    /// (`ops.rs`); a target-only language gets just `lang.rs`.
    pub host: bool,
    /// The hole token the grammar recognizes in expression/statement position,
    /// e.g. `__HOLE__` (Python) or `{}` (Rust).
    pub hole: String,
    /// Whether the language is tree-sitter-backed (adds migration guidance).
    pub tree_sitter: bool,
}

impl LangSpec {
    /// The `PascalCase` type prefix derived from [`key`](Self::key): `ruby` ->
    /// `Ruby`, `objective-c` -> `ObjectiveC`. Drops non-alphanumerics, upper-
    /// casing the start of each run.
    #[must_use]
    pub fn pascal(&self) -> String {
        let mut out = String::with_capacity(self.key.len());
        let mut upper = true;
        for c in self.key.chars() {
            if c.is_alphanumeric() {
                if upper {
                    out.extend(c.to_uppercase());
                    upper = false;
                } else {
                    out.push(c);
                }
            } else {
                upper = true;
            }
        }
        out
    }

    /// Validate the spec: `key` must be a non-empty lowercase Rust identifier and
    /// must not collide with a language that already ships.
    pub fn validate(&self) -> Result<()> {
        if self.key.is_empty() {
            bail!("new-lang: language key must not be empty");
        }
        let ok_first = self
            .key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase());
        let ok_rest = self
            .key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok_first || !ok_rest {
            bail!(
                "new-lang: language key {:?} must be a lowercase identifier \
                 (a-z, 0-9, _; starting with a letter)",
                self.key
            );
        }
        const SHIPPED: &[&str] = &[
            "bash",
            "html",
            "nix",
            "python",
            "rust",
            "text",
            "typescript",
            "wgsl",
            "zsh",
            "bootstrap",
            "omni",
        ];
        if SHIPPED.contains(&self.key.as_str()) {
            bail!(
                "new-lang: a language named {:?} already exists in quilt/src/langs",
                self.key
            );
        }
        Ok(())
    }
}

/// Replace every `{{name}}` hole in `template` with its value (raw text — no
/// lifting, so identifiers stay bare). Later vars don't re-scan earlier
/// substitutions.
#[must_use]
pub fn render(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        let needle = ["{{", k, "}}"].concat();
        out = out.replace(&needle, v);
    }
    out
}

/// Build the language module as a [`QTree`] rooted at `<key>/`. A target-only
/// language gets `mod.rs` + `lang.rs`; a host adds `meta.rs` + `ops.rs`. Every
/// tree also carries an `INTEGRATION.md` with the wiring steps. The leaves are
/// raw assets (no DO-NOT-EDIT header): the author is meant to fill the
/// `todo!()` stubs.
pub fn build_lang_tree(spec: &LangSpec) -> Result<QTree> {
    spec.validate()?;
    let pascal = spec.pascal();
    let aliases = alias_list(spec);
    let ts_guidance = if spec.tree_sitter { TS_GUIDANCE } else { "" };
    let vars: &[(&str, &str)] = &[
        ("lang", &spec.key),
        ("Lang", &pascal),
        ("display", &spec.display),
        ("hole", &spec.hole),
        ("aliases", &aliases),
        ("ts_guidance", ts_guidance),
    ];

    let mut tree = QTree::new();
    let modrs = if spec.host {
        "pub mod lang;\npub mod meta;\npub mod ops;\n"
    } else {
        "pub mod lang;\n"
    };
    tree.emit(format!("{}/mod.rs", spec.key), raw(modrs))?;
    tree.emit(format!("{}/lang.rs", spec.key), raw(render(LANG_RS, vars)))?;
    if spec.host {
        tree.emit(format!("{}/meta.rs", spec.key), raw(render(META_RS, vars)))?;
        tree.emit(format!("{}/ops.rs", spec.key), raw(render(OPS_RS, vars)))?;
    }
    tree.emit(
        format!("{}/INTEGRATION.md", spec.key),
        raw(integration_md(spec, &pascal, &aliases)),
    )?;
    Ok(tree)
}

/// The `["a", "b"]` alias-array literal for the omni table: the canonical key
/// first, then any extra aliases.
fn alias_list(spec: &LangSpec) -> String {
    let mut names = vec![spec.key.clone()];
    names.extend(spec.aliases.iter().cloned());
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    format!("[{}]", quoted.join(", "))
}

/// The `INTEGRATION.md` body: the exact edits that register the generated module
/// (the "omni.rs registration snippet" plus the module declaration and feature).
fn integration_md(spec: &LangSpec, pascal: &str, aliases: &str) -> String {
    let key = &spec.key;
    let feature = if spec.tree_sitter {
        "[\"parse\"]"
    } else {
        "[]"
    };
    let lang_import = format!("use super::{key}::lang::{{{pascal}Language, Dyn{pascal}Language}};");
    let lang_row = format!(
        "{key} if \"{key}\" => {pascal}({pascal}Language, Dyn{pascal}Language): {aliases};"
    );

    let mut meta_section = String::new();
    if spec.host {
        meta_section = format!(
            "\n3. **`quilt/src/langs/omni.rs`** — also import the meta-language:\n\
             ```rust\n\
             use super::{key}::meta::{pascal}MetaLanguage;\n\
             ```\n\
             and add a row to the `metas {{ … }}` block of `define_omni!`:\n\
             ```rust\n\
             {key} if \"{key}\" => {pascal}({pascal}MetaLanguage): {aliases};\n\
             ```\n"
        );
    }

    format!(
        "# Integrating the `{display}` language\n\
         \n\
         `quilt new-lang` scaffolded `quilt/src/langs/{key}/`. Fill in the `todo!()`\n\
         stubs, then wire the module in with the edits below (the `omni.rs`\n\
         registration snippet). See `docs/wiki/adding-a-language.md` for the full guide.\n\
         \n\
         1. **`quilt/Cargo.toml`** — add a feature and include it in `default`:\n\
         ```toml\n\
         {key} = {feature}\n\
         ```\n\
         \n\
         2. **`quilt/src/langs/mod.rs`** — declare the module:\n\
         ```rust\n\
         #[cfg(feature = \"{key}\")]\n\
         pub mod {key};\n\
         ```\n\
         \n\
         3. **`quilt/src/langs/omni.rs`** — import the language types:\n\
         ```rust\n\
         {lang_import}\n\
         ```\n\
         and add a row to the `languages {{ … }}` block of `define_omni!`:\n\
         ```rust\n\
         {lang_row}\n\
         ```\n\
         {meta_section}\
         \n\
         Once wired in, `cargo build` compiles with the stubs in place; replace each\n\
         `todo!()` with real behavior and add a round-trip test under\n\
         `quilt/src/langs/{key}/` (model it on an existing language's tests).\n",
        display = spec.display,
    )
}

/// Tree-sitter migration guidance, spliced into `lang.rs` only when the language
/// is tree-sitter-backed. A doc comment, so it never affects compilation.
const TS_GUIDANCE: &str = "//!\n\
//! This language is tree-sitter-backed: instead of implementing `Language`\n\
//! directly (below), most languages wrap `TSLanguage<YourProvider>` where\n\
//! `YourProvider: TSProvider` sets `hole_str()` to the hole token and supplies\n\
//! the parser. See `langs::rust::lang` / `langs::python::lang` for the pattern,\n\
//! and vendor the grammar under `grammars/` (see issue #32).\n";

/// The `lang.rs` template — a compiling `todo!()` stub modeled on
/// `langs::text::lang`, with the language's names and hole token filled in.
const LANG_RS: &str = r#"//! `{{display}}` language support for Quilt — scaffolded by `quilt new-lang`.
//!
//! Replace the `todo!()` stubs with real parse/serialize behavior. This file
//! compiles as-is (the stubs panic at runtime), so the crate builds the moment
//! the module is registered (see `INTEGRATION.md`).
{{ts_guidance}}
use crate::lang::{FlatNode, Hole, InnerKind, Language, LanguagePost};
use crate::prelude::*;
use crate::qterm::QTerm;

/// The hole token recognized in `{{display}}` source.
pub const HOLE_TOKEN: &str = "{{hole}}";

/**************************************************************/

#[derive(Default)]
pub struct {{Lang}}Language;

impl Language for {{Lang}}Language {
    type Post = {{Lang}}LanguagePost;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, _code: &[FlatNode]) -> Result<Self::Post> {
        todo!("parse {{display}} source into a {{Lang}}LanguagePost")
    }
}

#[derive(Debug)]
pub struct {{Lang}}LanguagePost;

impl LanguagePost for {{Lang}}LanguagePost {
    fn holes(&self) -> &[Hole] {
        todo!("report this fragment's holes")
    }

    fn parse_post(&self, _plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>> {
        todo!("plug the children back in and build the QTerm")
    }
}

/**************************************************************/

/// The boxed-post variant `omni` registers (its `Languages` dict is dynamic).
#[derive(Default)]
pub struct Dyn{{Lang}}Language;

impl Language for Dyn{{Lang}}Language {
    type Post = Box<dyn LanguagePost>;

    fn parse_pre(&mut self, _ikind: Option<InnerKind>, _code: &[FlatNode]) -> Result<Self::Post> {
        todo!("parse {{display}} source (boxed-post variant)")
    }
}
"#;

/// The `meta.rs` template (host languages only) — a `MetaLanguage` stub modeled
/// on `langs::text::meta`.
const META_RS: &str = r#"//! `{{display}}` meta-language (host) — scaffolded by `quilt new-lang`.
//!
//! Drives expansion of `{{lang}}↖ … ↗` quotes into builder calls against the
//! runtime ops in `ops.rs`. Replace the `todo!()` stubs. Model it on
//! `langs::python::meta` / `langs::rust::meta`.

use miette::Result;

use crate::lang::Arity;
use crate::prelude::{Index, *};
use crate::{meta::MetaLanguage, qterm::QTerm, term::CmdOrHole};

/**************************************************************/

#[derive(Default)]
pub struct {{Lang}}MetaLanguage;

impl MetaLanguage for {{Lang}}MetaLanguage {
    fn expand_quote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        todo!("emit {{display}} builder code for a quote")
    }

    fn expand_unquote(
        &self,
        _lang1: &str,
        _tag: &str,
        _i: Index,
        _lang2: &str,
        _qterm: &Arc<QTerm>,
        _cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>> {
        todo!("emit {{display}} builder code for an unquote")
    }

    fn expand_tuple(
        &self,
        _lang1: &str,
        _tag: &str,
        _qterms: &[Arc<QTerm>],
        _cmds: &[CmdOrHole],
        _arity: Arity,
    ) -> Result<Arc<QTerm>> {
        todo!("emit {{display}} builder code for a tuple")
    }
}
"#;

/// The `ops.rs` template (host languages only) — a documented placeholder for
/// the runtime helpers expanded `.{{lang}}.quilt` code calls.
const OPS_RS: &str = r#"//! Runtime ops for the `{{display}}` host language — scaffolded by `quilt new-lang`.
//!
//! These are the helpers that expanded `.{{lang}}.quilt` code calls — the analog
//! of `langs::rust::ops` (`qlift`, `name`, `reduce`, …) and `langs::python::ops`.
//! `meta.rs` emits calls to them. Add them here as your meta-language grows.
"#;

/**************************************************************/

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{Node, Segment};

    fn target_spec() -> LangSpec {
        LangSpec {
            key: "ruby".into(),
            aliases: vec!["rb".into()],
            display: "Ruby".into(),
            host: false,
            hole: "__HOLE__".into(),
            tree_sitter: true,
        }
    }

    fn host_spec() -> LangSpec {
        LangSpec {
            host: true,
            ..target_spec()
        }
    }

    fn leaf_text(tree: &QTree, path: &str) -> String {
        let mut cur = tree;
        let segs: Vec<_> = path.split('/').collect();
        for seg in &segs[..segs.len() - 1] {
            let Some(Node::Dir(sub)) = cur.get(&Segment::new(*seg).unwrap()) else {
                panic!("missing dir {seg}");
            };
            cur = sub;
        }
        match cur.get(&Segment::new(segs[segs.len() - 1]).unwrap()) {
            Some(Node::Raw { bytes, .. }) => String::from_utf8(bytes.clone()).unwrap(),
            other => panic!("expected a raw leaf at {path}, got {other:?}"),
        }
    }

    #[test]
    fn pascal_case() {
        assert_eq!(target_spec().pascal(), "Ruby");
        let mut s = target_spec();
        s.key = "objective_c".into();
        assert_eq!(s.pascal(), "ObjectiveC");
    }

    #[test]
    fn target_only_omits_meta_and_ops() {
        let tree = build_lang_tree(&target_spec()).unwrap();
        let Some(Node::Dir(d)) = tree.get(&Segment::new("ruby").unwrap()) else {
            panic!("ruby dir");
        };
        // mod.rs, lang.rs, INTEGRATION.md — no meta.rs / ops.rs.
        assert_eq!(d.len(), 3);
        assert!(d.get(&Segment::new("meta.rs").unwrap()).is_none());
        assert_eq!(leaf_text(&tree, "ruby/mod.rs"), "pub mod lang;\n");
    }

    #[test]
    fn host_includes_meta_and_ops() {
        let tree = build_lang_tree(&host_spec()).unwrap();
        let Some(Node::Dir(d)) = tree.get(&Segment::new("ruby").unwrap()) else {
            panic!("ruby dir");
        };
        // mod.rs, lang.rs, meta.rs, ops.rs, INTEGRATION.md.
        assert_eq!(d.len(), 5);
        assert!(leaf_text(&tree, "ruby/mod.rs").contains("pub mod meta;"));
        assert!(leaf_text(&tree, "ruby/meta.rs").contains("RubyMetaLanguage"));
        assert!(leaf_text(&tree, "ruby/ops.rs").contains("Runtime ops"));
    }

    #[test]
    fn lang_rs_is_substituted() {
        let src = leaf_text(&build_lang_tree(&target_spec()).unwrap(), "ruby/lang.rs");
        assert!(src.contains("pub struct RubyLanguage;"));
        assert!(src.contains("pub struct DynRubyLanguage;"));
        assert!(src.contains(r#"pub const HOLE_TOKEN: &str = "__HOLE__";"#));
        // No unsubstituted holes remain.
        assert!(!src.contains("{{"), "unsubstituted hole in lang.rs:\n{src}");
        // tree-sitter guidance present for a TS-backed language.
        assert!(src.contains("tree-sitter-backed"));
    }

    #[test]
    fn integration_md_lists_omni_rows() {
        let md = leaf_text(
            &build_lang_tree(&host_spec()).unwrap(),
            "ruby/INTEGRATION.md",
        );
        assert!(md
            .contains(r#"ruby if "ruby" => Ruby(RubyLanguage, DynRubyLanguage): ["ruby", "rb"];"#));
        assert!(md.contains(r#"ruby if "ruby" => Ruby(RubyMetaLanguage): ["ruby", "rb"];"#));
        assert!(md.contains("pub mod ruby;"));
        assert!(!md.contains("{{"), "unsubstituted hole in INTEGRATION.md");
    }

    #[test]
    fn target_md_has_no_meta_row() {
        let md = leaf_text(
            &build_lang_tree(&target_spec()).unwrap(),
            "ruby/INTEGRATION.md",
        );
        assert!(!md.contains("MetaLanguage"));
        // a target-only language is feature = ["parse"] when tree-sitter-backed.
        assert!(md.contains(r#"ruby = ["parse"]"#));
    }

    #[test]
    fn rejects_shipped_and_bad_keys() {
        let mut s = target_spec();
        s.key = "rust".into();
        assert!(build_lang_tree(&s).is_err());
        s.key = "Bad".into();
        assert!(build_lang_tree(&s).is_err());
        s.key = String::new();
        assert!(build_lang_tree(&s).is_err());
    }
}
