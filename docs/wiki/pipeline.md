# Parse → Expand Pipeline

This page describes how `quilt expand foo.rs.quilt` turns a `.quilt` file into `foo.rs`.

## Overview

```
foo.rs.quilt
    │
    ▼
 Node::parse()          source → Box<[Node]>     (hand-written scanner)
    │
    ▼
 Multi::parse_chain()   Node tree → Arc<QTerm>   (recursive, dispatches to Language impls)
    │
    ▼
 Multi::expand_lang()   Arc<QTerm> → Arc<QTerm>  (flat, no Quote/Unquote)
    │
    ▼
 QTerm::dump()          Arc<QTerm> → foo.rs      (serialize via StrCmds)
```

## Stage 1: `Node::parse` — surface AST

**File:** `quilt/src/node.rs`, `quilt/src/node/parse.rs`

`Node::parse(source)` scans the raw source string and produces a `Box<[Node]>`. `Node` is a simple enum:

```rust
pub enum Node {
    Content(Box<str>),          // any text that isn't a Quilt glyph
    NewLine,
    Quote { anno, nodes, span },   // lang↖ … ↗ (span = byte range in the source)
    Unquote { anno, nodes, span }, // lang↙ … ↘
    Lift,                       // ↑
    Reduce,                     // ↓
    Emit,                       // ←
    Type,                       // ⟨T⟩
    Name,                       // ⟨N⟩
}
```

`anno` is the language annotation before `↖`/`↙`; it is empty for un-annotated brackets. Quilt comments (`⟨//⟩` / `⟨/*⟩…⟨*/⟩`) are consumed by the parser and never appear in the `Node` list — the line form takes the newline and indentation in front of it too, so removing one leaves no blank line behind.

### Why the parser is hand-written

It was a tree-sitter grammar (`tree-sitter-quilt`) until issue #254. Quilt's outer syntax is a *lexical* language — glyph-delimited brackets over otherwise-opaque text — so a generalised parser was building a CST only for a conversion pass to walk it and throw it away. Scanning straight to `Node` measured **24.5× faster** on the repo's own corpus, and drops tree-sitter from this stage entirely: `Node::parse` is available in the runtime-only build.

The grammar has since been deleted, and with it the second description of Quilt's surface syntax. While both existed a differential test held them to producing identical `Node` trees over every `.quilt` file in the repo, the grammar's own corpus, and ~2M generated inputs; `quilt/tests/parser_corpus.rs` keeps that corpus and snapshots the answers, so what the two agreed on is still pinned.

Nesting is tracked on an explicit stack rather than by recursion, so a pathological input is a diagnostic rather than a stack overflow.

### `scan`: the same parser, recovering

`Node::parse` is strict — the first diagnostic wins and nothing comes back. [`quilt-lsp`](lsp.md) cannot use that: an editor buffer is malformed most of the time someone is typing in it, and blanking every region in the file between typing a `↖` and its `↗` would make the server useless.

So `quilt::node::scan` is the recovering half of the same parser. It returns every token with its byte range plus every diagnostic, and never fails. Two properties make it what the server builds on:

- the tokens **tile the source** — concatenating them reproduces it exactly — so a consumer copying spans sees every byte;
- Quilt's own comments (`⟨//⟩`, `⟨/*⟩…⟨*/⟩`) get tokens, where the `Node` tree drops them. That is what lets the server translate a `⟨//⟩` into the host language's `//` without re-scanning for it.

Both halves drive the same scanner, so the server and the compiler cannot disagree about where a bracket is.

## Stage 2: `Multi::parse_chain` — building the QTerm tree

**File:** `quilt/src/multi.rs` — `Multi::build_nodes`

This stage recursively descends through the `Node` list and dispatches each language fragment to its `Language` implementation via a two-phase parse.

### Language chain and zipper

The file name's inner extensions form a *chain*. `shaders.wgsl.rs.quilt` → chain `["rs", "wgsl"]` meaning: Rust is the ground language and bare `↖…↗` inside Rust defaults to WGSL. A plain `foo.rs.quilt` → chain `["rs"]`.

A `Zipper<Box<str>>` tracks the current default language as the parser descends into nested quotes. `zipper.back()` moves to the next default; an explicit annotation (`py↖…↗`) pushes the named language to the front.

### Indentation stripping (two passes)

Before calling the language parser, `build_nodes` performs two pre-processing passes:

1. **Strip outer prefix.** Each line's common prefix (accumulated from the hole's `prefix` field, which represents the current indentation level) is removed.
2. **Dedent body.** The common leading whitespace of all non-empty lines in the fragment is removed.

First and last newlines are also stripped so that multi-line quotes look natural without extra blank lines.

### Two-phase language parse

Each language implements:

```rust
trait Language {
    fn parse_pre(&mut self, ikind: Option<InnerKind>, code: &[FlatNode]) -> Result<Self::Post>;
}
trait LanguagePost {
    fn holes(&self) -> &[Hole];
    fn parse_post(&self, plugs: &[Arc<QTerm>]) -> Result<Arc<QTerm>>;
}
```

`FlatNode` is a flat sequence of `Str` / `Hole` / `NewLine` — holes stand in for nested quote/unquote brackets. The language parser (tree-sitter for Rust and Python) parses this with placeholder tokens (`{}` for Rust, `__HOLE__` for Python) and records where each hole ends up in the AST. `parse_post` then substitutes the real `Arc<QTerm>` children into those positions.

### Recursion

For each `Node::Quote { anno, nodes }` encountered:
- A `qb(hole_tag, 1, lang)` builder is created, carrying the node's source span.
- `build_nodes` is called recursively on `nodes` with the new zipper.
- The resulting builder is closed and pushed as a plug.

For each `Node::Unquote { anno, nodes }`:
- A `ub(hole_tag, 1, outer_lang)` builder is created, carrying the node's source span.
- `build_nodes` is called recursively on `nodes` with the zipper unwound one step (the inner content is in the outer language). An unquote with no enclosing quote is a spanned "unquote depth too high" error.

Special nodes (`Lift`, `Reduce`, `Emit`, `Type`, `Name`) are translated to their language-specific string spellings by querying the `MetaLanguages` registry.

## Stage 3: `Multi::expand_lang` — expansion

**File:** `quilt/src/multi.rs` — `Expander`

The `Expander` walks the `QTerm` tree and calls the ground language's `MetaLanguage` to transform each node. It tracks the current `Stage` (Ground vs Sky).

### Ground stage

- `Tuple { tag, terms, cmds }` — recurse into each child at Ground; call `meta.wrap_child` on the result.
- `Quote { … }` — switch to Sky, recurse.
- `Unquote { … }` — error (unquote at depth 0 is invalid); the diagnostic points at the unquote's source span when the term carries one.

### Sky stage (inside quotes)

- `Tuple { tag, terms, cmds }` — check `Language::arity(tag)` to determine if variadic. Recurse into each child; if variadic, set `OuterKind::Emit` or `OuterKind::Splice` on the `wrap_child` call. Call `meta.expand_tuple`.
- `Quote { index, … }` — depth increases by `index`; call `meta.expand_quote`.
- `Unquote { index, … }` — depth decreases. If new depth == 0, escape to Ground; otherwise call `meta.expand_unquote`.

### MetaLanguage calls

The three required `MetaLanguage` methods build the *code* that will reconstruct the term at runtime:

| Method           | What it builds                                                   |
|------------------|------------------------------------------------------------------|
| `expand_quote`   | `quote(tag, i, lang, <term>, &cmds)` constructor call            |
| `expand_unquote` | `unquote(tag, i, lang, <term>, &cmds)` constructor call          |
| `expand_tuple`   | `tb(tag).w(..).c(&child)..b()` builder chain (or variadic block) |

## Stage 4: serialization

The fully-expanded `Arc<QTerm>` contains only `Tuple` nodes. `QTerm::dump(path)` creates the output file by walking the `cmds` sequences and interpreting them with `PrefixWriter`.

The CLI (`bin.rs`) prepends a generated header comment:

```
//! DO NOT EDIT. GENERATED BY `quilt expand foo.rs.quilt`.
```
