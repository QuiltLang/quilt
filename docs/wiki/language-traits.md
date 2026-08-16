# Language Traits

**Files:** `quilt/src/lang.rs`, `quilt/src/meta.rs`

Two trait families are the extension points for adding language support to Quilt.

## `Language` / `LanguagePost` — parsing

```rust
pub trait Language {
    type Post: LanguagePost;

    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post>;
    // Convenience wrappers:
    fn parse(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>>;
    fn parse_expr/stmt/file/auto(&mut self, code: &[FlatNode]) -> Result<Arc<QTerm>>;
    fn arity(&self, tag: &str) -> Arity;
    fn typ(&self, tag: &str) -> InnerKind;
    fn hashbang(&self) -> Option<&'static str>;
}

pub trait LanguagePost: Debug {
    fn holes(&self) -> &[Hole];
    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>>;
}

// Comment syntax rides alongside, as consts rather than `Language` methods —
// see `Comments` below.
pub trait Comments {
    const LINE: Option<&'static str> = Some("//");
    const HEADER: Option<&'static str> = Self::LINE;
}
```

### `FlatNode`

```rust
pub enum FlatNode<'a> {
    Hole,          // a placeholder for a child term
    Str(&'a str),  // ordinary source text
    NewLine,
}
```

The multi-language engine collects the `Node` list for a given fragment and builds a `Vec<FlatNode>` where each `Node::Quote`/`Node::Unquote` becomes a `FlatNode::Hole`. This flat list is passed to `parse_pre`.

### `Hole`

```rust
pub struct Hole {
    pub otag: Box<str>,            // the tree-sitter tag of the hole in the outer language
    pub prefix: Box<[Box<str>]>,   // accumulated indentation prefixes
}
```

`parse_pre` must return one `Hole` per `FlatNode::Hole` in its input (in order). The `Multi` engine uses `hole.otag` to label the `Quote`/`Unquote` node in the output tree, and `hole.prefix` to strip indentation from nested content.

### `InnerKind`

```rust
pub enum InnerKind { Expr, Stmt, File }
```

Passed as a hint to `parse_pre`. Some parsers use it to try specific grammar entry points instead of guessing. `None` means auto-detect.

### `Arity`

```rust
pub enum Arity { Unknown, Const(u8), Variadic }
```

Returned by `Language::arity(tag)`. `Variadic` tags (e.g. Rust's `"block"` and `"source_file"`) tell the expander to use `expand_tuple` in variadic mode, generating an imperative builder block rather than a single `tb(..).c(..)..b()` call — so that one child can contribute zero-or-many terms.

The tables are **generated, not hand-written** (issue #202): `bin/gen-arity` reads the `REPEAT` rules out of each vendored `quilt/grammars/<lang>/grammar.json` and writes `quilt/src/langs/arity.rs`, and every provider's `arity` is one line:

```rust
fn arity(&self, tag: &str) -> Arity {
    Arity::from_table(crate::langs::arity::RUST, tag)
}
```

`bin/check-arity` regenerates and fails on drift, so a grammar bump that adds, drops or renames a repeat container shows up as a reviewable diff rather than as emit quietly changing where it may splice. The derivation itself — which repeats count as a node's *own* children, and why hidden rules are followed but category rules, aliases and tokens are not — is documented in `quilt-conformance/src/arity.rs`.

Declaring a tag variadic costs nothing in generated output: a variadic node with no unquote among its direct children is built fluently anyway, since only an unquote can contribute a variable number of terms.

### `hashbang`

Returns the shebang line used to run the expanded file as a script, e.g.:
- Rust: `"#!/usr/bin/env rust-script"`
- Python: `"#!/usr/bin/env python3"`

`quilt` uses this to determine which runner to invoke.

### `Comments`

How the language spells comments, implemented beside its `Language` impl:

```rust
impl Comments for RustLanguage {
    const LINE: Option<&'static str> = Some("//");
    const HEADER: Option<&'static str> = Some("//!");   // inner doc comment
}
```

`LINE` is the plain line-comment introducer; `HEADER` is the one the CLI puts on
a generated file's `DO NOT EDIT` banner, and defaults to `LINE` — Rust is the
only language where the two differ. `None` means no *prefix* can express one
(HTML's comments are delimited; plain text has none), and the caller decides
what to do about it.

Look them up by language name with `langs::line_comment` / `langs::header_comment`,
which `define_omni!` generates from the registry table, so aliases agree by
construction. quilt-lsp reads the same functions for its adapters' comment
syntax rather than declaring the spellings a second time
([#194](https://github.com/QuiltLang/quilt/issues/194)).

These are associated consts rather than `Language` methods for two reasons: an
associated const would make `Language` non-dyn-compatible, and `DynOmniLanguages`
dispatches through `dyn Language`; and a const lookup is a `match` returning a
`&'static str`, where a `&self` method would mean constructing a language — a
tree-sitter `Parser` — to read one.

---

## `MetaLanguage` — expansion

```rust
pub trait MetaLanguage {
    fn expand_quote(
        &self, lang1: &str, tag: &str, i: Index,
        lang2: &str, qterm: &Arc<QTerm>, cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;

    fn expand_unquote(
        &self, lang1: &str, tag: &str, i: Index,
        lang2: &str, qterm: &Arc<QTerm>, cmds: &[CmdOrHole],
    ) -> Result<Arc<QTerm>>;

    fn expand_tuple(
        &self, lang1: &str, tag: &str,
        qterms: &[Arc<QTerm>], cmds: &[CmdOrHole], arity: Arity,
    ) -> Result<Arc<QTerm>>;

    fn wrap_child(&self, qterm: Arc<QTerm>, okind: OuterKind) -> Result<Arc<QTerm>>;

    // Operator spellings — the string that ↑ / ↓ / ← / ⟨T⟩ / ⟨N⟩ expand to:
    fn lift_str(&self)   -> &'static str { LIFT   }
    fn reduce_str(&self) -> &'static str { REDUCE }
    fn emit_str(&self)   -> &'static str { EMIT   }
    fn type_str(&self)   -> &'static str { TYPE   }
    fn name_str(&self)   -> &'static str { NAME   }
}
```

### How expansion works

When the expander encounters a `Quote { lang2, … }` at Sky depth, it calls:

```
meta.expand_quote(lang1, tag, index, lang2, expanded_term, cmds)
```

This should return a `QTerm` whose code, when executed, constructs the quoted term at runtime. For the **Rust** meta-language this means returning code like:

```
quote("expression_statement", 1, "rs", <term>, &[...cmds...])
```

Similarly for `expand_unquote` and `expand_tuple`.

### `OuterKind`

```rust
pub enum OuterKind { None, Emit, Splice }
```

Passed to `wrap_child`:
- `None` — no wrapping needed.
- `Emit` — the child is a quote inside a variadic context; wrap as `.emit(&mut b_)` (Rust) or `.e(child)` (Python).
- `Splice` — the child is a statement-valued unquote that should be inlined; wrap as a bare statement (Rust: add `;`).

### Operator spelling constants

The five special glyphs in `.quilt` source are translated to language-specific strings:

| Glyph | Default constant | Rust override                  |
|-------|------------------|--------------------------------|
| `↑`   | `"__LIFT__"`     | `"qlift()"`                    |
| `↓`   | `"__REDUCE__"`   | `"reduce()"`                   |
| `←`   | `"__EMIT__"`     | `(same)"`                      |
| `⟨T⟩` | `"__TYPE__"`     | `"Arc<QTerm>"` (via bootstrap) |
| `⟨N⟩` | `"__NAME__"`     | `"name()"`                     |

The Bootstrap meta-language has its own spellings for bootstrapping `meta.rs`.

All five accessors return `Result`, because not every meta-language has a
spelling for every glyph: a **string-based** meta (nix, lean) has no `QTerm`
runtime, so `↓` has none, and untyped Nix has none for `⟨T⟩`. Returning an error
there is what keeps the default `__REDUCE__`/`__TYPE__` placeholders from
leaking into generated code. A meta that *does* have a meaning for a glyph
should spell it rather than fail — in the string model a fragment is a string,
so lean's `⟨T⟩` is `String` and `⟨N⟩` is the host's identity (`id`, and
`toString` for nix). The same applies to `←`: having no `b_` accumulator is not
the same as having no emit, and nix spells it
`builtins.concatStringsSep "\n"` — the functional reading, joining a list of
fragments into the surrounding container (issue #155). Lean's still fails
(issue #133).

Having a `QTerm` runtime is not sufficient either. Python and TypeScript build a
variadic container as a fluent `tb(..).e(child).b()` chain rather than Rust's
imperative block, so there is no name for a ground `←` to append to — and their
runtimes expose no `emit` method on a term. Both therefore fail on `←` too
(issue #152). The rule is the same in every case: spell the glyph where the host
has a meaning for it, fail with actionable guidance where it does not, and never
let the placeholder reach the output.

---

## `TSLanguage` — the tree-sitter helper

**File:** `quilt/src/treesitter.rs`

Most concrete language implementations use `TSLanguage<P: TSProvider>` rather than implementing `Language` directly. A `TSProvider` supplies:

```rust
pub trait TSProvider {
    fn parser(&mut self) -> &mut tree_sitter::Parser;
    fn hole_str(&self) -> &'static str;   // placeholder: "{}" or "__HOLE__"
    fn unwrap(&self, qterm: QTerm, ikind: Option<InnerKind>) -> (QTerm, InnerKind);
    fn arity(&self, tag: &str) -> Arity   { Arity::Unknown }
    fn typ(&self, tag: &str) -> InnerKind { InnerKind::File }
    fn hashbang(&self) -> Option<&'static str> { None }
}
```

`TSLanguage::parse_pre` substitutes each `FlatNode::Hole` with `hole_str()`, feeds the resulting string to the tree-sitter parser, finds the placeholder tokens by their text content, and records the hole positions. `parse_post` then replaces those positions with real `Arc<QTerm>` children.

`unwrap` is called on the top-level tree-sitter node to strip the outer `source_file` wrapper and infer whether the fragment is an expression or statement.

`DynTSLanguage<P>` is a newtype that wraps `TSLanguage<P>` and boxes its `Post` type so it can be stored as `Box<dyn Language<Post = Box<dyn LanguagePost>>>` in a `DictMulti`.
