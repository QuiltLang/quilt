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
//! * **…and a named alias is a node kind of its own.** The repeat inside it is
//!   credited to the alias's name, alongside the rule of that name if there is
//!   one. Lean's `binders` has no rule at all — it is only ever
//!   `alias(repeat1($._binder), $.binders)` — so listing rule names alone left
//!   it fixed-arity, and a sequence of binders could not be spliced into it.
//!
//! # Which containers are comma-separated
//!
//! A second table (#293) names the variadic kinds whose repeated children are
//! separated by `,` — what tree-sitter's `commaSep1(x)` spells as
//! `seq(x, repeat(seq(",", x)))`. The marker is a `REPEAT` whose body is a
//! `SEQ` that *opens* with the string `","`; the walk is the one above. A kind
//! qualifies when every such separator-led repeat it holds is led by `,`, so
//! Lean's `by`, whose tactics `;` separates, and Python's `dotted_name` (`.`)
//! stay out: the builder writes the separator followed by a space, which is
//! right for a comma and wrong for most else.
//!
//! Finally the result is intersected with the grammar's real node kinds. A rule
//! that is always aliased away never appears as a tag, so it cannot be looked
//! up: html's `script_start_tag` is spelled `start_tag` in every tree, and rust's
//! `_expression`-only helpers never surface at all.

use crate::registry;
use miette::{IntoDiagnostic as _, Result, WrapErr as _};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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
    /// A kind is defined by its own rule and by every named `ALIAS` that spells
    /// it (see [`Self::definitions`]); it is variadic when any of them is.
    ///
    /// Restricted to kinds the compiled parser actually reports, so the result
    /// is exactly the set that can affect expansion — a tag that never appears
    /// in a tree could never be looked up anyway.
    #[must_use]
    pub fn variadic_tags(&self, kinds: &BTreeSet<&str>) -> Vec<String> {
        self.definitions()
            .into_iter()
            .filter(|(name, _)| kinds.contains(name.as_str()))
            .filter(|(_, defs)| defs.iter().any(|def| def.has_repeat(self)))
            .map(|(name, _)| name)
            .collect()
    }

    /// Every variadic node kind whose repeated children are separated by `,`,
    /// sorted. See the module docs.
    #[must_use]
    pub fn comma_separated_tags(&self, kinds: &BTreeSet<&str>) -> Vec<String> {
        let defs = self.definitions();
        self.variadic_tags(kinds)
            .into_iter()
            .filter(|name| {
                let mut seps = BTreeSet::new();
                for def in &defs[name] {
                    self.separators(def.body, &mut def.seen.clone(), &mut seps);
                }
                seps.len() == 1 && seps.contains(",")
            })
            .collect()
    }

    /// Every visible node kind, with the rule bodies that give it its direct
    /// children: its own rule, if it has one, and the content of every named
    /// `ALIAS` that produces it.
    ///
    /// An alias is a leaf to the rule that *contains* it, but it is a node in
    /// its own right, and its content is that node's structure. Lean's `binders`
    /// exists only as `_binders: alias(repeat1($._binder), $.binders)`, so
    /// without this a hole in binder position could never take a sequence.
    /// When the content is a single symbol the node takes that rule's children
    /// (`alias($._simple_statements, $.block)`), so the symbol is followed even
    /// when it is visible — unless it is a category, which is still one child.
    ///
    /// An alias's repeat must also repeat something named: bash spells `word`
    /// as `alias(repeat1($._special_character), $.word)` in places, and a
    /// `_special_character` is a token, so that is a lexical run rather than a
    /// list of children (likewise SQL's `identifier`, zsh's `_expansion_word`).
    /// Rules keep the plain test, so this adds kinds without removing any.
    fn definitions(&self) -> BTreeMap<String, Vec<Def<'_>>> {
        let mut defs: BTreeMap<String, Vec<Def<'_>>> = BTreeMap::new();
        for (name, rule) in &self.rules {
            if !name.starts_with('_') {
                defs.entry(name.clone()).or_default().push(Def {
                    body: rule,
                    seen: vec![name.as_str()],
                    alias: false,
                });
            }
            let mut aliases = Vec::new();
            collect_aliases(rule, &mut aliases);
            for (value, content) in aliases {
                let inner = strip(content);
                let (body, seen) = match inner.get("type").and_then(Value::as_str) {
                    Some("SYMBOL") => {
                        let Some((target, body)) = inner
                            .get("name")
                            .and_then(Value::as_str)
                            .and_then(|t| self.rules.get_key_value(t))
                        else {
                            continue; // an external token: a leaf
                        };
                        if is_category(body) {
                            continue;
                        }
                        (body, vec![target.as_str()])
                    }
                    _ => (content, Vec::new()),
                };
                defs.entry(value.to_string()).or_default().push(Def {
                    body,
                    seen,
                    alias: true,
                });
            }
        }
        defs
    }

    /// Collect the tokens that lead a separator repeat (`repeat(seq(",", x))`)
    /// among the direct children of the node `rule` belongs to. Walks the rule
    /// the way [`Self::has_repeat`] does.
    fn separators<'a>(
        &'a self,
        rule: &'a Value,
        seen: &mut Vec<&'a str>,
        out: &mut BTreeSet<String>,
    ) {
        let Some(kind) = rule.get("type").and_then(Value::as_str) else {
            return;
        };
        match kind {
            "REPEAT" | "REPEAT1" => {
                let body = strip(content(rule));
                let lead = (body.get("type").and_then(Value::as_str) == Some("SEQ"))
                    .then(|| members(body).next().map(strip))
                    .flatten()
                    .filter(|m| m.get("type").and_then(Value::as_str) == Some("STRING"))
                    .and_then(|m| m.get("value").and_then(Value::as_str));
                match lead {
                    // The repeat's own body is one element and its separator;
                    // whatever repeats inside the element is not this list's.
                    Some(token) => {
                        out.insert(token.to_string());
                    }
                    None => self.separators(content(rule), seen, out),
                }
            }
            "SEQ" | "CHOICE" => {
                for m in members(rule) {
                    self.separators(m, seen, out);
                }
            }
            _ if TRANSPARENT.contains(&kind) => self.separators(content(rule), seen, out),
            "SYMBOL" => {
                let Some(target) = rule.get("name").and_then(Value::as_str) else {
                    return;
                };
                if !target.starts_with('_') || seen.contains(&target) {
                    return;
                }
                let Some(body) = self.rules.get(target) else {
                    return;
                };
                if is_category(body) {
                    return;
                }
                seen.push(target);
                self.separators(body, seen, out);
                seen.pop();
            }
            _ => {}
        }
    }

    /// Does `rule` give the node it belongs to a variable number of *direct*
    /// children? See the module docs for why each case is what it is.
    ///
    /// With `named_only`, a repeat counts only when it repeats something
    /// named ([`Self::yields_named`]); see [`Self::definitions`].
    fn has_repeat<'a>(
        &'a self,
        rule: &'a Value,
        seen: &mut Vec<&'a str>,
        named_only: bool,
    ) -> bool {
        let Some(kind) = rule.get("type").and_then(Value::as_str) else {
            return false;
        };
        match kind {
            "REPEAT" | "REPEAT1" => {
                !named_only || self.yields_named(content(rule), &mut Vec::new())
            }
            "SEQ" | "CHOICE" => members(rule).any(|m| self.has_repeat(m, seen, named_only)),
            _ if TRANSPARENT.contains(&kind) => self.has_repeat(content(rule), seen, named_only),
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
                let found = self.has_repeat(body, seen, named_only);
                seen.pop();
                found
            }
            // Everything else is a leaf as far as *this* node's children go.
            // `ALIAS`, `TOKEN` and `IMMEDIATE_TOKEN` are the ones worth naming:
            // each collapses to exactly one node, so a repeat inside is that
            // node's structure rather than this one's (a named alias's own
            // structure is credited to it by `definitions`). The rest (`STRING`,
            // `PATTERN`, `BLANK`) have no inner structure at all.
            _ => false,
        }
    }

    /// Can `rule` produce a *named* node? External tokens (symbols without a
    /// rule) are assumed to.
    fn yields_named<'a>(&'a self, rule: &'a Value, seen: &mut Vec<&'a str>) -> bool {
        let Some(kind) = rule.get("type").and_then(Value::as_str) else {
            return false;
        };
        match kind {
            "ALIAS" => rule.get("named").and_then(Value::as_bool) == Some(true),
            "SEQ" | "CHOICE" => members(rule).any(|m| self.yields_named(m, seen)),
            "REPEAT" | "REPEAT1" => self.yields_named(content(rule), seen),
            _ if TRANSPARENT.contains(&kind) => self.yields_named(content(rule), seen),
            "SYMBOL" => {
                let Some(target) = rule.get("name").and_then(Value::as_str) else {
                    return false;
                };
                if !target.starts_with('_') {
                    return true;
                }
                if seen.contains(&target) {
                    return false;
                }
                let Some(body) = self.rules.get(target) else {
                    return true; // an external token
                };
                seen.push(target);
                let found = self.yields_named(body, seen);
                seen.pop();
                found
            }
            // `STRING`, `PATTERN`, `BLANK`, `TOKEN`, `IMMEDIATE_TOKEN`.
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

/// Every named `ALIAS` inside `rule`, as `(value, content)`. Anonymous aliases
/// (`alias($.x, "kw")`) produce no node kind, so they are skipped; an alias
/// nested in another alias's content is still collected.
fn collect_aliases<'a>(rule: &'a Value, out: &mut Vec<(&'a str, &'a Value)>) {
    let Some(kind) = rule.get("type").and_then(Value::as_str) else {
        return;
    };
    match kind {
        "ALIAS" => {
            if rule.get("named").and_then(Value::as_bool) == Some(true) {
                if let Some(value) = rule.get("value").and_then(Value::as_str) {
                    out.push((value, content(rule)));
                }
            }
            collect_aliases(content(rule), out);
        }
        "SEQ" | "CHOICE" => {
            for m in members(rule) {
                collect_aliases(m, out);
            }
        }
        _ => collect_aliases(content(rule), out),
    }
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

/// `rule` with its [`TRANSPARENT`] wrappers peeled off.
fn strip(mut rule: &Value) -> &Value {
    while rule
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|k| TRANSPARENT.contains(&k))
    {
        rule = content(rule);
    }
    rule
}

/// One rule body that defines a node kind's direct children: the kind's own
/// rule, or the content of a named `ALIAS` that produces it.
struct Def<'a> {
    body: &'a Value,
    /// The hidden rules already being followed, for the cycle check.
    seen: Vec<&'a str>,
    alias: bool,
}

impl Def<'_> {
    fn has_repeat(&self, grammar: &Grammar) -> bool {
        grammar.has_repeat(self.body, &mut self.seen.clone(), self.alias)
    }
}

/// One language's derived tables.
pub struct Tables {
    /// The variadic node kinds ([`Grammar::variadic_tags`]).
    pub variadic: Vec<String>,
    /// The comma-separated ones among them ([`Grammar::comma_separated_tags`]).
    pub comma_separated: Vec<String>,
}

/// The derived tables for every registered language, in
/// [`registry::LANGUAGES`] order. Languages without a grammar are absent.
pub fn derive_all() -> Result<Vec<(&'static str, Tables)>> {
    let mut out = Vec::new();
    for &name in registry::LANGUAGES {
        let Some(grammar) = Grammar::load(name)? else {
            continue;
        };
        let Some(compiled) = registry::grammar(name) else {
            continue;
        };
        let kinds = registry::node_kinds(&compiled);
        out.push((
            name,
            Tables {
                variadic: grammar.variadic_tags(&kinds),
                comma_separated: grammar.comma_separated_tags(&kinds),
            },
        ));
    }
    Ok(out)
}
