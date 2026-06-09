# QTerm IR

**File:** `rust/quilt/src/qterm.rs`

`QTerm` is the central intermediate representation. Every parsed `.quilt` file becomes a tree of `Arc<QTerm>` nodes; expansion transforms that tree into a flat `QTerm` (no `Quote`/`Unquote` variants) ready for serialization.

## Variants

```rust
pub enum QTerm {
    Quote   { tag, index, lang, term: Arc<QTerm>, cmds },
    Unquote { tag, index, lang, term: Arc<QTerm>, cmds },
    Tuple   { tag, terms: Box<[Arc<QTerm>]>,      cmds },
}
```

### `Tuple`

The workhorse. Represents any parsed AST node: an expression, statement, block, or leaf token.

- **`tag`** — the tree-sitter node kind: `"block"`, `"binary_expression"`, `"integer_literal"`, `"identifier"`, etc.
- **`terms`** — zero or more child `QTerm`s (zero for leaf nodes).
- **`cmds`** — a `Box<[CmdOrHole]>` that interleaves `StrCmd` printing instructions with `Hole` markers. `Hole` at position *i* means "print the *i*-th child here". This is how whitespace and punctuation are stored.

### `Quote`

Wraps a fragment to be treated as data.

- **`tag`** — the tree-sitter node kind of the *hole* where this quote appears in the outer language (e.g. `"expression_statement"`, `"let_declaration"`).
- **`index`** — quasi-quote nesting depth (always 1 after parsing; can increment if quotes are nested inside quotes).
- **`lang`** — the language of the quoted content (e.g. `"rs"`, `"wgsl"`).
- **`term`** — the quoted `QTerm`.
- **`cmds`** — the surrounding serialization commands (includes the `↖…↗` glyphs).

### `Unquote`

A splice site.

- **`tag`** — the hole kind in the outer language.
- **`index`** — depth at which this unquote escapes (1 = escape one level of quoting).
- **`lang`** — the language being unquoted into.
- Same `term` and `cmds` fields as `Quote`.

## Constructor functions

```rust
// Arc-returning constructors (most common)
leaf(tag, code)               // childless node with a Write cmd
sym(s)                        // leaf where tag == code
tuple(tag, &terms, &cmds)
quote(tag, index, lang, term, &cmds)
unquote(tag, index, lang, term, &cmds)

// QTerm-returning (no Arc) — used internally
qleaf, qsym, qtuple, qquote, qunquote
```

`leaf("integer_literal", "42")` is the typical way to create a literal token. `sym("{")` creates a token where the tag is also the printed text (punctuation, keywords).

## Serialization: `StrCmd` and `CmdOrHole`

`StrCmd` drives output generation:

```rust
pub enum StrCmd {
    Write(Box<str>),   // emit this string (ignores prefix)
    NewLine,           // emit newline then re-emit the current indent prefix
    Push(Box<str>),    // push an extra prefix level
    Pop,               // pop the top prefix level
}
```

`CmdOrHole` is either a `StrCmd` or a `Hole` marker. The `cmds` field of every `QTerm` variant is `Box<[CmdOrHole]>`.

`PrefixWriter` (`strcmd.rs`) maintains the prefix stack and interprets `StrCmd`s when writing to any `std::io::Write`.

The constants `NL`, `POP`, and the functions `write(s)`, `push(s)`, `cmd(c)`, `HOLE` are re-exported from `prelude`.

## The `QTermBuilder` API

Builder for constructing terms with an ergonomic fluent interface.

```rust
// Constructors for builders
tb(tag)                  // Tuple builder
qb(tag, index, lang)     // Quote builder
ub(tag, index, lang)     // Unquote builder

// Fluent methods (each returns Self for chaining)
.w(s)        // Write(s)
.n()         // NewLine
.p(s)        // Push(s)
.x()         // Pop
.c(&child)   // insert child at this Hole position
.e(x)        // emit — calls x.emit(&mut self)
.b()         // build and return Arc<QTerm>
```

The `Emit` trait allows any type that "emits" things into a builder:

- `Arc<QTerm>` emits itself as a child.
- `Vec<T: Emit>` emits each element.
- `()` emits nothing.
- `&str` emits a `Write`.
- `StrCmd` emits itself as a cmd.

### Example

```rust
let expr = tb("binary_expression")
    .c(&leaf("integer_literal", "1"))
    .w(" + ")
    .c(&leaf("integer_literal", "2"))
    .b();
```

## Utility methods

| Method                                                 | Description                                               |
|--------------------------------------------------------|-----------------------------------------------------------|
| `qterm.sexp()`                                         | Debug-friendly s-expression string                        |
| `qterm.squash()`                                       | If exactly one child, absorb its `cmds` into the parent's |
| `qterm.rewrite_naive(find, replace)`                   | Recursive structural substitution                         |
| `qterm.coparse()`                                      | Serialize to a `String` using the embedded `cmds`         |
| `qterm.dump(path)`                                     | Write to a file                                           |
| `qterm.dump_with_cmds(path, prefix_cmds, suffix_cmds)` | Write with extra leading/trailing cmds                    |

## `QTermTag`

A tag value that identifies a `QTerm` variant without its children:

```rust
pub enum QTermTag {
    Quote(Box<str>, Index, Box<str>),
    Unquote(Box<str>, Index, Box<str>),
    Tuple(Box<str>),
}
```

Used by `QTermBuilder::new(tag)` and the `Term` trait.
