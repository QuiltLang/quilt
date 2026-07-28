//! The declared half of the matrix: `conformance/spec/<lang>.toml`.
//!
//! A spec is a *claim*, hand-written and reviewed. The battery turns each claim
//! into a probe. The point of keeping this as data rather than as Rust test
//! code is that adding a language becomes adding one file — the ~100 assertions
//! that follow are the harness's job, not the author's.

use crate::matrix::{Axis, Status};
use miette::{bail, miette, Context as _, IntoDiagnostic as _, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};

/// A hole's expected syntactic position, mirroring `quilt::lang::InnerKind`.
///
/// `Any` is the spec spelling of `Hole { ikind: None }` — the language imposes
/// no kind on what may be spliced there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Expr,
    Stmt,
    Item,
    Block,
    File,
    Any,
}

impl Kind {
    pub fn to_inner(self) -> Option<quilt::lang::InnerKind> {
        use quilt::lang::InnerKind;
        Some(match self {
            Kind::Expr => InnerKind::Expr,
            Kind::Stmt => InnerKind::Stmt,
            Kind::Item => InnerKind::Item,
            Kind::Block => InnerKind::Block,
            Kind::File => InnerKind::File,
            Kind::Any => return None,
        })
    }
}

/// One declared capability cell.
#[derive(Debug, Clone, Deserialize)]
pub struct Claim {
    pub status: Status,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub issue: Option<u32>,
}

/// A source fragment that must parse and round-trip, with the root tag it must
/// produce. The tag assertion is what makes this a structural check rather than
/// a "the text came back" check.
#[derive(Debug, Clone, Deserialize)]
pub struct Fragment {
    pub name: String,
    /// The `InnerKind` to parse at; omit to exercise `parse_auto`.
    #[serde(default)]
    pub kind: Option<Kind>,
    pub code: String,
    /// Expected root tuple tag.
    pub tag: String,
}

/// A hole-position probe. `code` is split on the `@` marker: each `@` becomes a
/// hole, and `ikinds` lists the `InnerKind` each hole must be assigned.
#[derive(Debug, Clone, Deserialize)]
pub struct HoleProbe {
    pub name: String,
    pub code: String,
    pub ikinds: Vec<Kind>,
}

/// A value lifted into this language via its `LiftTo` marker, and the tag +
/// text the lifted literal must produce. Reparsing that text in this grammar is
/// what catches escaping bugs (Nix `${`, Lean `{`, shell `$`).
#[derive(Debug, Clone, Deserialize)]
pub struct LiftProbe {
    /// One of the value keys the battery knows how to build; see
    /// `battery::lift_values`.
    pub value: String,
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub name: String,
    pub display: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub feature: String,
    pub blurb: String,
    /// `runtime` | `string` | `none`
    pub meta_kind: String,
    pub lang_src: String,
    #[serde(default)]
    pub meta_src: Option<String>,

    /// Every axis in `Axis::ALL` must appear. A missing key is a hard error, so
    /// adding an axis forces every language to answer it.
    pub capabilities: BTreeMap<String, Claim>,

    #[serde(default)]
    pub fragments: Vec<Fragment>,
    #[serde(default)]
    pub holes: Vec<HoleProbe>,
    /// tag → expected `InnerKind` from `Language::typ`.
    #[serde(default)]
    pub kinds: BTreeMap<String, Kind>,
    /// Tags that must report `Arity::Variadic`.
    #[serde(default)]
    pub variadic: Vec<String>,
    /// Tags that must *not* report `Arity::Variadic`. Guards against a language
    /// over-declaring variadicity, which silently changes emit behaviour.
    #[serde(default)]
    pub not_variadic: Vec<String>,
    /// The `LiftTo` marker for this language, when one exists (`Wgsl`, `Nix`, …).
    #[serde(default)]
    pub lift_marker: Option<String>,
    #[serde(default)]
    pub lift: Vec<LiftProbe>,
    /// Targets this language's `MetaLanguage::lift_str` must be able to spell.
    #[serde(default)]
    pub lift_from: Vec<String>,
    /// Targets it must explicitly refuse, so a missing spelling is a decision
    /// rather than an oversight.
    #[serde(default)]
    pub lift_from_unsupported: Vec<String>,
}

impl Spec {
    pub fn claim(&self, axis: Axis) -> Result<&Claim> {
        self.capabilities
            .get(axis.key())
            .ok_or_else(|| miette!("{}: spec is missing capability {:?}", self.name, axis.key()))
    }

    /// Load and validate every spec in `dir`.
    pub fn load_all(dir: &Path) -> Result<Vec<Spec>> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .into_diagnostic()
            .wrap_err_with(|| format!("reading spec dir {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        entries.sort();

        if entries.is_empty() {
            bail!("no spec files in {}", dir.display());
        }

        let mut specs = Vec::new();
        for path in entries {
            let text = fs::read_to_string(&path)
                .into_diagnostic()
                .wrap_err_with(|| format!("reading {}", path.display()))?;
            let spec: Spec = toml::from_str(&text)
                .into_diagnostic()
                .wrap_err_with(|| format!("parsing {}", path.display()))?;
            spec.validate()
                .wrap_err_with(|| format!("validating {}", path.display()))?;
            specs.push(spec);
        }
        specs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(specs)
    }

    /// Structural checks that do not need the engine: every axis answered, every
    /// non-trivial status explained, every `Partial`/`Planned` cell tracked.
    fn validate(&self) -> Result<()> {
        for axis in Axis::ALL {
            let claim = self.claim(*axis)?;
            match claim.status {
                Status::Partial | Status::Unsupported => {
                    if claim.note.is_none() {
                        bail!(
                            "{}: capability {:?} is {:?} but has no `note` — every limitation \
                             must say what the limit is",
                            self.name,
                            axis.key(),
                            claim.status.label(),
                        );
                    }
                }
                Status::Planned => {
                    if claim.issue.is_none() {
                        bail!(
                            "{}: capability {:?} is `planned` but has no `issue` — planned work \
                             must be tracked",
                            self.name,
                            axis.key(),
                        );
                    }
                }
                Status::Supported => {}
            }
            if claim.status == Status::Partial && claim.issue.is_none() {
                bail!(
                    "{}: capability {:?} is `partial` but has no `issue` — a stated limitation \
                     needs a tracking issue",
                    self.name,
                    axis.key(),
                );
            }
        }

        let unknown: Vec<&String> = self
            .capabilities
            .keys()
            .filter(|k| !Axis::ALL.iter().any(|a| a.key() == k.as_str()))
            .collect();
        if !unknown.is_empty() {
            bail!("{}: unknown capability keys {unknown:?}", self.name);
        }

        if !matches!(self.meta_kind.as_str(), "runtime" | "string" | "none") {
            bail!(
                "{}: meta_kind must be `runtime`, `string` or `none`, got {:?}",
                self.name,
                self.meta_kind
            );
        }
        Ok(())
    }
}
