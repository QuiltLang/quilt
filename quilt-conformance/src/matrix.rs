//! The capability matrix: the axes every language is asked about, the status
//! of each cell, and the two rendered artifacts (JSON for the website, Markdown
//! for the wiki).
//!
//! The matrix is the contract described in issue #144. A cell is *declared* in
//! `conformance/spec/<lang>.toml` and, where the harness can, *verified* against
//! the real implementation. Both directions fail:
//!
//! * a declared `supported` cell whose probe fails → the claim is wrong;
//! * an undeclared capability whose probe passes → the spec is stale.
//!
//! The matrix is deliberately **not** schema-versioned: it is regenerated from
//! source on every run and consumed only by this repo and the website, so a
//! version field would be a second thing to keep in sync for no benefit.

use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// Where a capability sits for one language.
///
/// `Partial` is the status the old website table could not express, and it is
/// where Lean and Nix actually live (string-based metas, no emit into a ground
/// loop). A `Partial` or `Unsupported` cell is not an untested gap: the harness
/// requires it to carry a `note`, and the probe asserts the stated limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Works, and the probe proves it.
    Supported,
    /// Works within a stated limit; `note` says what the limit is.
    Partial,
    /// Deliberately not supported; the probe asserts a clean failure.
    Unsupported,
    /// Intended and tracked by an issue; no probe.
    Planned,
}

impl Status {
    /// The glyph the Markdown table and the website key use.
    pub fn emoji(self) -> &'static str {
        match self {
            Status::Supported => "✅",
            Status::Partial => "🟡",
            Status::Unsupported => "⬜",
            Status::Planned => "🔵",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Supported => "supported",
            Status::Partial => "partial",
            Status::Unsupported => "unsupported",
            Status::Planned => "planned",
        }
    }
}

/// One capability axis. Adding a variant here is what forces every existing
/// spec file to answer a new question — the mechanism that would have made
/// Lean declare its `←` glyph collision (#141) on the day it landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Axis {
    // ── As an object (target) language ──────────────────────────────────
    /// `lang↖…↗` parses and round-trips through `coparse`.
    Quotable,
    /// Holes reach the syntactic positions the spec lists.
    HolePositions,
    /// The spec's variadic container tags really report `Arity::Variadic`.
    VariadicContainers,
    /// `typ()` classifies the spec's tags into the right `InnerKind`.
    KindClassification,
    /// Rust values lift into this language via a `LiftTo` marker, and the
    /// lifted literal reparses in this grammar as the declared tag.
    LiftInto,
    /// A `HIGHLIGHTS_QUERY` is vendored for this grammar.
    Highlights,

    // ── As a host (meta) language ───────────────────────────────────────
    /// Has a `MetaLanguage` and can drive generation.
    Host,
    /// Which targets this host's `lift_str` can spell.
    LiftFrom,
    /// `↓` has a backend for this host.
    Reduce,
    /// `←` into a variadic container.
    Emit,
    /// `let ↖…↗ = …` destructuring.
    PatternMatch,
    /// `hashbang()` — runnable through `quilt run`.
    Runnable,
    /// A published runtime package implementing the `QTerm` builder API.
    RuntimeBinding,

    // ── Cross-cutting ───────────────────────────────────────────────────
    /// Glyphs that collide with this language's own syntax.
    GlyphCollisions,
    /// Usable as a non-ground member of a `.a.b.quilt` language chain.
    ChainMember,
    /// Correct comment syntax on the generated-file header.
    HeaderComment,
    /// `quilt-lsp` support.
    Lsp,
}

impl Axis {
    /// Every axis, in table order. `Spec::load` requires a declaration for
    /// each, so a new axis is a compile-then-fill-in-every-spec change rather
    /// than a silently-empty column.
    pub const ALL: &'static [Axis] = &[
        Axis::Quotable,
        Axis::HolePositions,
        Axis::VariadicContainers,
        Axis::KindClassification,
        Axis::LiftInto,
        Axis::Highlights,
        Axis::Host,
        Axis::LiftFrom,
        Axis::Reduce,
        Axis::Emit,
        Axis::PatternMatch,
        Axis::Runnable,
        Axis::RuntimeBinding,
        Axis::GlyphCollisions,
        Axis::ChainMember,
        Axis::HeaderComment,
        Axis::Lsp,
    ];

    /// The kebab-case key used in the TOML spec and the JSON.
    pub fn key(self) -> &'static str {
        match self {
            Axis::Quotable => "quotable",
            Axis::HolePositions => "hole-positions",
            Axis::VariadicContainers => "variadic-containers",
            Axis::KindClassification => "kind-classification",
            Axis::LiftInto => "lift-into",
            Axis::Highlights => "highlights",
            Axis::Host => "host",
            Axis::LiftFrom => "lift-from",
            Axis::Reduce => "reduce",
            Axis::Emit => "emit",
            Axis::PatternMatch => "pattern-match",
            Axis::Runnable => "runnable",
            Axis::RuntimeBinding => "runtime-binding",
            Axis::GlyphCollisions => "glyph-collisions",
            Axis::ChainMember => "chain-member",
            Axis::HeaderComment => "header-comment",
            Axis::Lsp => "lsp",
        }
    }

    /// Short column header for the rendered tables.
    pub fn title(self) -> &'static str {
        match self {
            Axis::Quotable => "Quotable",
            Axis::HolePositions => "Holes",
            Axis::VariadicContainers => "Variadic",
            Axis::KindClassification => "Kinds",
            Axis::LiftInto => "Lift in",
            Axis::Highlights => "Highlights",
            Axis::Host => "Host",
            Axis::LiftFrom => "Lift out",
            Axis::Reduce => "Reduce ↓",
            Axis::Emit => "Emit ←",
            Axis::PatternMatch => "Patterns",
            Axis::Runnable => "Runnable",
            Axis::RuntimeBinding => "Runtime",
            Axis::GlyphCollisions => "Glyphs",
            Axis::ChainMember => "Chain",
            Axis::HeaderComment => "Header",
            Axis::Lsp => "LSP",
        }
    }

    /// One-line explanation, rendered into the key beneath each table.
    pub fn description(self) -> &'static str {
        match self {
            Axis::Quotable => "`lang↖…↗` parses and round-trips back to identical source",
            Axis::HolePositions => "syntactic positions an unquote hole can occupy",
            Axis::VariadicContainers => "node kinds that accept arbitrarily many children",
            Axis::KindClassification => "`typ()` sorts tags into expr / stmt / item / block / file",
            Axis::LiftInto => "Rust values lift into this language's literal syntax via `↑`",
            Axis::Highlights => "a tree-sitter `highlights.scm` is vendored for embedded quotes",
            Axis::Host => {
                "has a `MetaLanguage`, so it can be the ground language of a `.quilt` file"
            }
            Axis::LiftFrom => "targets this host can lift a value into",
            Axis::Reduce => "`↓` evaluates a fragment at generation time",
            Axis::Emit => "`←` appends into the surrounding variadic container",
            Axis::PatternMatch => "`let ↖pattern↗ = value` destructures by matching shape",
            Axis::Runnable => "`quilt run` can execute a file in this language directly",
            Axis::RuntimeBinding => "a published package implements the `QTerm` builder API",
            Axis::GlyphCollisions => "Quilt glyphs that are also this language's own syntax",
            Axis::ChainMember => "usable as a non-ground language in a `.a.b.quilt` chain",
            Axis::HeaderComment => "the generated-file header uses this language's comment syntax",
            Axis::Lsp => "`quilt-lsp` projection and downstream-server support",
        }
    }

    /// Object-language axes render in the first table, host axes in the second,
    /// everything else in the third.
    pub fn group(self) -> Group {
        match self {
            Axis::Quotable
            | Axis::HolePositions
            | Axis::VariadicContainers
            | Axis::KindClassification
            | Axis::LiftInto
            | Axis::Highlights => Group::Object,
            Axis::Host
            | Axis::LiftFrom
            | Axis::Reduce
            | Axis::Emit
            | Axis::PatternMatch
            | Axis::Runnable
            | Axis::RuntimeBinding => Group::Host,
            Axis::GlyphCollisions | Axis::ChainMember | Axis::HeaderComment | Axis::Lsp => {
                Group::CrossCutting
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Object,
    Host,
    CrossCutting,
}

impl Group {
    pub const ALL: &'static [Group] = &[Group::Object, Group::Host, Group::CrossCutting];

    pub fn title(self) -> &'static str {
        match self {
            Group::Object => "As an object (target) language",
            Group::Host => "As a host (meta) language",
            Group::CrossCutting => "Cross-cutting",
        }
    }
}

/// One verified cell of the matrix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cell {
    pub axis: Axis,
    pub status: Status,
    /// What the status means in this language's case. Required for `Partial`
    /// and `Unsupported`, so a limitation is always explained rather than
    /// implied by a blank cell.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Issue tracking the gap, for `Partial` / `Planned` cells.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<u32>,
    /// The probe that proved this cell, e.g. `quotable/rust`. `None` means the
    /// cell is declaration-only: no tier of the harness checks it yet. The
    /// website renders these distinctly so "we assert this" and "we assert
    /// this in prose" never look the same.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
    /// Extra detail the probe discovered, e.g. the concrete hole positions or
    /// the list of targets this host can lift into.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail: Vec<String>,
}

impl Cell {
    pub fn is_verified(&self) -> bool {
        self.verified_by.is_some()
    }
}

/// One language's row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub name: String,
    pub display: String,
    pub aliases: Vec<String>,
    pub feature: String,
    pub blurb: String,
    /// `runtime` (emits builder calls into a `QTerm` library), `string`
    /// (reconstructs fragments as host string literals), or `none`.
    pub meta_kind: String,
    /// Repo-relative path to the `Language` impl, for the website's links.
    pub lang_src: String,
    /// Repo-relative path to the `MetaLanguage` impl, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_src: Option<String>,
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn cell(&self, axis: Axis) -> Option<&Cell> {
        self.cells.iter().find(|c| c.axis == axis)
    }
}

/// The whole matrix — what lands in `conformance/support-matrix.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Matrix {
    /// Human-readable provenance so a reader of the JSON knows not to hand-edit
    /// it and knows which command rebuilds it.
    pub generated_by: String,
    pub rows: Vec<Row>,
}

impl Matrix {
    pub fn new(mut rows: Vec<Row>) -> Self {
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            generated_by: "bin/gen-matrix (quilt-conformance) — DO NOT EDIT".into(),
            rows,
        }
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("matrix serializes");
        s.push('\n');
        s
    }

    /// Render the wiki page. Three tables (object / host / cross-cutting), each
    /// with an emoji key, plus a notes section listing every `Partial` and
    /// `Unsupported` cell with its explanation and issue link.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();

        out.push_str("# Language Support Matrix\n\n");
        out.push_str(
            "<!-- DO NOT EDIT. Generated by `bin/gen-matrix`; \
             `bin/check-matrix` fails if this file drifts. -->\n\n",
        );
        out.push_str(
            "Every cell below is declared in `conformance/spec/<lang>.toml` and, \
             where the harness can reach it, verified against the implementation by \
             `cargo test -p quilt-conformance`. A claim that stops being true fails \
             CI. See [issue #144](https://github.com/QuiltLang/quilt/issues/144).\n\n",
        );

        out.push_str("## Key\n\n");
        out.push_str("| | Meaning |\n|---|---|\n");
        for status in [
            Status::Supported,
            Status::Partial,
            Status::Unsupported,
            Status::Planned,
        ] {
            let meaning = match status {
                Status::Supported => "Works, and a probe proves it",
                Status::Partial => "Works within a stated limit — see the notes below the table",
                Status::Unsupported => "Deliberately unsupported; the probe asserts a clean error",
                Status::Planned => "Intended and tracked by an issue; not yet implemented",
            };
            let _ = writeln!(
                out,
                "| {} `{}` | {meaning} |",
                status.emoji(),
                status.label()
            );
        }
        out.push_str(
            "\nA cell marked with a trailing `*` is **declaration-only**: it is \
             recorded in the spec but no tier of the harness verifies it yet.\n\n",
        );

        for group in Group::ALL {
            let axes: Vec<Axis> = Axis::ALL
                .iter()
                .copied()
                .filter(|a| a.group() == *group)
                .collect();

            let _ = writeln!(out, "## {}\n", group.title());

            out.push_str("| Language |");
            for a in &axes {
                let _ = write!(out, " {} |", a.title());
            }
            out.push_str("\n|---|");
            for _ in &axes {
                out.push_str("---|");
            }
            out.push('\n');

            for row in &self.rows {
                let _ = write!(out, "| **{}** |", row.display);
                for a in &axes {
                    match row.cell(*a) {
                        Some(c) => {
                            let mark = if c.is_verified() { "" } else { "*" };
                            let _ = write!(out, " {}{mark} |", c.status.emoji());
                        }
                        None => out.push_str("  |"),
                    }
                }
                out.push('\n');
            }

            out.push_str("\nColumns:\n\n");
            for a in &axes {
                let _ = writeln!(out, "- **{}** — {}", a.title(), a.description());
            }
            out.push('\n');
        }

        out.push_str("## Notes and limitations\n\n");
        let mut any = false;
        for row in &self.rows {
            let notable: Vec<&Cell> = row
                .cells
                .iter()
                .filter(|c| {
                    matches!(
                        c.status,
                        Status::Partial | Status::Unsupported | Status::Planned
                    ) && c.note.is_some()
                })
                .collect();
            if notable.is_empty() {
                continue;
            }
            any = true;
            let _ = writeln!(out, "### {}\n", row.display);
            for c in notable {
                let issue = c.issue.map_or(String::new(), |n| {
                    format!(" ([#{n}](https://github.com/QuiltLang/quilt/issues/{n}))")
                });
                let _ = writeln!(
                    out,
                    "- {} **{}** — {}{issue}",
                    c.status.emoji(),
                    c.axis.title(),
                    c.note.as_deref().unwrap_or_default(),
                );
            }
            out.push('\n');
        }
        if !any {
            out.push_str("_None._\n");
        }

        out
    }
}
