//! Derive each language's variadic-container table from its tree-sitter grammar
//! (issue #202).
//!
//! `Language::arity` answers one question: can a node of this kind hold a
//! variable number of direct children? The expander uses the answer to decide
//! whether a node is a container an emit (`←`) can splice a sequence into. Until
//! now every language answered it from a hand-written allowlist of node kinds,
//! maintained independently per language — which is how bash and zsh came to
//! classify eleven shared constructs differently (#150), and how a table can
//! silently stop matching after `bin/sync-grammars` pulls a grammar that renamed
//! a kind (#174).
//!
//! The grammar already knows the answer. This module reads it out of the
//! vendored `grammar.json` and [`bin/gen-arity`](../../bin/gen-arity) writes the
//! result to `quilt/src/langs/arity.rs`, so the tables are generated from the
//! same pinned rev as the parsers that produce the nodes.
//!
//! # What counts as variadic
//!
//! A `REPEAT` / `REPEAT1` in the rule body — *not* `node-types.json`'s
//! `children.multiple`, which is the obvious scrape and the wrong one: it is
//! true for any node with several distinct child *slots*, so it makes Rust's
//! `function_item` variadic, which `conformance/spec/rust.toml` explicitly
//! declares it must not be.
//!
//! "Direct children" is what makes the rest of the rules fall out. A repeat only
//! counts when the nodes it repeats land under *this* node:
//!
//! * **Hidden (`_`-prefixed) rules are followed**, because tree-sitter inlines
//!   them — their children become the referring node's children. bash's
//!   `program` gets its repeat through `_statements`; without this the file
//!   root, the single most important variadic container for a shell target,
//!   falls out of the set.
//! * **…but category rules are not.** A hidden rule that is just a `CHOICE` of
//!   single-node alternatives (`_expression`, `_literal`) is a *type*, not
//!   structure: it contributes exactly one child, whichever alternative matched.
//!   Following it would inherit every alternative's repeats and blow the set up
//!   (python 30 → 56 spuriously; bash's `command_name`, whose whole body is
//!   `_literal`, would come out variadic).
//! * **`TOKEN` and `ALIAS` are leaves.** Both collapse to exactly one node, so a
//!   repeat inside belongs to that node rather than to its parent. A token has
//!   no children at all (bash's `raw_string`), and `alias($._simple_statements,
//!   $.block)` puts its repeated statements under the `block` — which is why
//!   python's `function_definition` is *not* variadic even though a repeat is
//!   reachable through its `body` field.
//!
//! Finally the result is intersected with the grammar's real node kinds. A rule
//! that is always aliased away never appears as a tag, so it cannot be looked
//! up: html's `script_start_tag` is spelled `start_tag` in every tree, and rust's
//! `_expression`-only helpers never surface at all.

use crate::registry;
use miette::{IntoDiagnostic as _, Result, WrapErr as _};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Node types that stand for exactly one node in the tree (or none), and so can
/// appear as an alternative of a category rule.
const SINGLE_NODE: &[&str] = &[
    "SYMBOL",
    "BLANK",
    "STRING",
    "PATTERN",
    "TOKEN",
    "IMMEDIATE_TOKEN",
    "ALIAS",
];

/// Wrappers that decorate a rule without changing what it matches.
const TRANSPARENT: &[&str] = &["FIELD", "PREC", "PREC_LEFT", "PREC_RIGHT", "PREC_DYNAMIC"];

/// The vendored `grammar.json` for a language, or `None` when it has none.
///
/// Mirrors [`registry::grammar`]: only `text` is grammar-less. TypeScript is the
/// one irregular path — `bin/sync-grammars` preserves the fork's nested
/// `typescript/src/` layout so its scanner's `../../common/scanner.h` include
/// keeps resolving.
#[must_use]
pub fn grammar_json_path(name: &str) -> Option<PathBuf> {
    let grammars = crate::repo_root().join("quilt/grammars");
    Some(match name {
        "typescript" => grammars.join("typescript/typescript/src/grammar.json"),
        "bash" | "html" | "lean" | "nix" | "python" | "rust" | "sql" | "wgsl" | "zsh" => {
            grammars.join(name).join("grammar.json")
        }
        _ => return None,
    })
}

/// A parsed `grammar.json`, reduced to the rule table the derivation needs.
pub struct Grammar {
    name: String,
    rules: serde_json::Map<String, Value>,
}

impl Grammar {
    /// Load a language's vendored grammar, or `None` when it has none.
    pub fn load(name: &str) -> Result<Option<Self>> {
        let Some(path) = grammar_json_path(name) else {
            return Ok(None);
        };
        let src = std::fs::read_to_string(&path)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "reading {} — run `bin/sync-grammars` to vendor it",
                    path.display()
                )
            })?;
        let json: Value = serde_json::from_str(&src)
            .into_diagnostic()
            .wrap_err_with(|| format!("parsing {}", path.display()))?;
        let rules = json
            .get("rules")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Ok(Some(Self {
            name: name.to_string(),
            rules,
        }))
    }

    /// Every node kind this grammar gives a variable number of direct children,
    /// sorted.
    ///
    /// Restricted to kinds the compiled parser actually reports, so the result
    /// is exactly the set that can affect expansion — a tag that never appears
    /// in a tree could never be looked up anyway.
    #[must_use]
    pub fn variadic_tags(&self, kinds: &BTreeSet<&str>) -> Vec<String> {
        let mut tags: Vec<String> = self
            .rules
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .filter(|(name, _)| kinds.contains(name.as_str()))
            .filter(|(name, rule)| self.has_repeat(rule, &mut vec![name.as_str()]))
            .map(|(name, _)| name.clone())
            .collect();
        tags.sort();
        tags
    }

    /// Does `rule` give the node it belongs to a variable number of *direct*
    /// children? See the module docs for why each case is what it is.
    fn has_repeat<'a>(&'a self, rule: &'a Value, seen: &mut Vec<&'a str>) -> bool {
        let Some(kind) = rule.get("type").and_then(Value::as_str) else {
            return false;
        };
        match kind {
            "REPEAT" | "REPEAT1" => true,
            "SEQ" | "CHOICE" => members(rule).any(|m| self.has_repeat(m, seen)),
            _ if TRANSPARENT.contains(&kind) => self.has_repeat(content(rule), seen),
            "SYMBOL" => {
                let Some(target) = rule.get("name").and_then(Value::as_str) else {
                    return false;
                };
                // A visible symbol is one child, whatever its own shape.
                if !target.starts_with('_') || seen.contains(&target) {
                    return false;
                }
                let Some(body) = self.rules.get(target) else {
                    return false; // an external token: a leaf
                };
                if is_category(body) {
                    return false;
                }
                seen.push(target);
                let found = self.has_repeat(body, seen);
                seen.pop();
                found
            }
            // Everything else is a leaf as far as *this* node's children go.
            // `ALIAS`, `TOKEN` and `IMMEDIATE_TOKEN` are the ones worth naming:
            // each collapses to exactly one node, so a repeat inside is that
            // node's structure rather than this one's. The rest (`STRING`,
            // `PATTERN`, `BLANK`) have no inner structure at all.
            _ => false,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Is this hidden rule a *category* — a choice between single-node alternatives,
/// i.e. a type rather than structure?
///
/// The test is deliberately about the alternatives' *shape*, not about naming
/// conventions or the grammar's `supertypes` list: bash's `_literal` is a choice
/// of two symbols and one `alias(repeat1(…), $.word)`, and that alias is still a
/// single node, so `_literal` is a category even though `supertypes` never
/// mentions it.
fn is_category(rule: &Value) -> bool {
    rule.get("type").and_then(Value::as_str) == Some("CHOICE") && members(rule).all(is_single_node)
}

/// Does this rule stand for at most one node in the tree?
fn is_single_node(rule: &Value) -> bool {
    let Some(kind) = rule.get("type").and_then(Value::as_str) else {
        return false;
    };
    if SINGLE_NODE.contains(&kind) {
        return true;
    }
    if TRANSPARENT.contains(&kind) {
        return is_single_node(content(rule));
    }
    false
}

fn members(rule: &Value) -> impl Iterator<Item = &Value> {
    rule.get("members")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
}

fn content(rule: &Value) -> &Value {
    rule.get("content").unwrap_or(&Value::Null)
}

/// The derived table for every registered language, in [`registry::LANGUAGES`]
/// order. Languages without a grammar are absent.
pub fn derive_all() -> Result<Vec<(&'static str, Vec<String>)>> {
    let mut out = Vec::new();
    for &name in registry::LANGUAGES {
        let Some(grammar) = Grammar::load(name)? else {
            continue;
        };
        let Some(compiled) = registry::grammar(name) else {
            continue;
        };
        let kinds = registry::node_kinds(&compiled);
        out.push((name, grammar.variadic_tags(&kinds)));
    }
    Ok(out)
}
