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

Returned by `Language::arity(tag)`. `Variadic` tags (e.g. Rust's `"block"` and `"source_file"`) tell the expander to use `expand_tuple` in variadic mode, generating an imperative builder block rather than a single `tb(..).c(..)..b()` call.

### `hashbang`

Returns the shebang line used to run the expanded file as a script, e.g.:
- Rust: `"#!/usr/bin/env rust-script"`
- Python: `"#!/usr/bin/env python3"`

`quilt run` uses this to determine which runner to invoke.

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
