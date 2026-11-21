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
 coparse() + header     Arc<QTerm> → foo.rs      (serialize via StrCmds)
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

The file name's inner extensions form a *chain*. `shaders.wgsl.rs.quilt` → chain `["rs", "wgsl"]` meaning: Rust is the ground language and bare `↖…↗` inside Rust defaults to WGSL. A bare quote after an annotated one at the same level takes that annotation instead (see [Concepts](concepts.md#quote-)). A plain `foo.rs.quilt` → chain `["rs"]`.

A `Zipper<Box<str>>` tracks the current default language as the parser descends into nested quotes. `zipper.back()` moves to the next default; an explicit annotation (`py↖…↗`) pushes the named language to the front.

### Indentation stripping (two passes)

Before calling the language parser, `build_nodes` performs two pre-processing passes:

1. **Strip outer prefix.** The enclosing indent (the hole's `prefix` field — the indentation the quote sits at) is removed from every line after a newline, but only as much of it as *every* non-blank line carries (`shared_prefix`). A body written left of the host code therefore keeps its relative indentation instead of failing with "Failed to strip prefix" (#292). A blank line loses as much of the full prefix as it has.
2. **Dedent body.** The common leading whitespace of the continuation lines is removed. Blank lines — the closer's own line among them — do not bound it, so a `↗` left of its body leaves no indent behind. An *inline* opener's content (`↖def f():…`) bounds it too, so such a body keeps its indentation — unless the last line starts with `}`, `)` or `]`, which marks the column the body is written relative to, and the continuation lines' common indent is stripped as in a block quote (#316).

One boundary newline on each side is then trimmed (and re-emitted as builder cmds), so `↖\n…\n↗` parses the same body as `↖…↗`. At top level the boundary newlines are real content and stay.

The mirror image happens on the way out: when the expanded term is written back, a multi-line token (a string, a block comment) whose continuation lines never carried the prefix keeps them verbatim rather than gaining it (#296), so expansion does not change a string's contents.

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

A quote whose [representation](concepts.md#representations-cst-str-qt) is not the default `⟨cst⟩` (`ts⟨str⟩↖…↗`, `ts⟨qt⟩↖…↗`, #322) is not rebuilt node by node: the expander flattens it to `Piece`s (text, layout commands, holes) and hands them to `MetaLanguage::expand_flat`, which builds a host string or a flat term.

## Stage 4: serialization

The fully-expanded `Arc<QTerm>` contains only `Tuple` nodes. `coparse()` writes it back to text by walking the `cmds` sequences and interpreting them with `PrefixWriter` (`QTerm::dump(path)` does the same straight to a file).

The CLI (`bin.rs`'s `generate`) then assembles the file, outside the term — which is why `quilt check` and the expander snapshots never see these:

1. a generated header comment, naming the input by its file name only, so the output does not depend on the working directory (#299), and spelled with the *output* language's comment marker (`langs::header_comment`, from each language's `Comments::HEADER` — `//!` for Rust, `#` for Python, `--` for Lean):

   ```
   //! DO NOT EDIT. GENERATED BY `quilt expand foo.rs.quilt`.
   ```

2. the source's leading `#!` shebang, **dropped** (a Rust `#![…]` inner attribute is kept);
3. the host's runtime import (`use quilt::prelude::*;`, `from quilt import *`, a named TypeScript import), prepended by `meta::with_prelude` when the source uses a glyph and does not already import the runtime (#275; `--prelude` / `--no-prelude` override it). See [the runtime import](cli.md#the-runtime-import-issue-274).
