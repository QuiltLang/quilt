# Concepts

## The `.quilt` file format

A `.quilt` file is ordinary source code in some *ground language* plus Quilt bracket syntax layered on top. The ground language is inferred from the file's inner extension:

| Filename            | Ground language                               |
|---------------------|-----------------------------------------------|
| `foo.rs.quilt`      | Rust                                          |
| `foo.py.quilt`      | Python                                        |
| `foo.wgsl.rs.quilt` | Rust (ground), WGSL (default for bare quotes) |

When expanded, the `.quilt` suffix is stripped to produce the output file name.

## Operator glyphs

All Quilt operators are Unicode characters. The VS Code extension provides chord keybindings so they are easy to type (see [Editor Setup](editor-setup.md)).

| Glyph | Name          | Meaning                                           |
|-------|---------------|---------------------------------------------------|
| `↖`   | left-quote    | Open a quote bracket                              |
| `↗`   | right-quote   | Close a quote bracket                             |
| `↙`   | left-unquote  | Open an unquote bracket                           |
| `↘`   | right-unquote | Close an unquote bracket                          |
| `↑`   | lift          | Convert a runtime value into a `QTerm`            |
| `↓`   | reduce        | Evaluate a `QTerm` by compiling and running it    |
| `←`   | emit          | Append a term into the surrounding variadic block |
| `⟨T⟩` | type          | Placeholder for `Arc<QTerm>` in bootstrap source  |
| `⟨N⟩` | name          | Create an identifier node                         |

Quilt-level line comments are written `⟨//⟩ ...` and block comments `⟨/*⟩ ... ⟨*/⟩`. They are stripped during parsing and never appear in the output.

### Escaping a glyph

Prefixing any operator glyph with `\` makes it literal text rather than a Quilt
operator: `\↖ \↗ \↙ \↘ \↑ \↓ \← \⟨ \⟩`. This matters when the target language
uses one of these characters itself — Lean spells monadic bind `←` and coercion
`↑`, so a `do` block inside a `lean↖…↗` quote needs `\←` (or Lean's ASCII alias
`<-`, which Quilt does not touch). The set lives in `node::GLYPHS` and must stay
in sync with the character classes in `tree-sitter-quilt/grammar.js`.

A backslash before anything else is *not* an escape and passes through verbatim,
so a target-language `"\n"` needs no special handling.

The escape belongs to Quilt's surface syntax and is consumed when the file is
parsed: `\←` produces a real `←` in the `QTerm`, so the expanded output contains
the bare glyph — which is exactly what the target language wants. Re-escaping on
output happens only on the `Node` (Quilt-source) path, never on the `QTerm`
(target-source) one. See `node::escape`.

## Quote and unquote

### Quote `↖…↗`

A quote bracket wraps a code fragment and turns it into a *value* — a `QTerm` that can be inspected, transformed, and emitted later:

```rust
// In a .rs.quilt file:
let expr: Arc<QTerm> = ↖1 + 2↗;
```

This expands to Rust code that *constructs* the `1 + 2` AST at runtime using the `QTermBuilder` API:

```rust
let expr: Arc<QTerm> = tb("binary_expression")
    .c(&leaf("integer_literal", "1"))
    .w(" + ")
    .c(&leaf("integer_literal", "2"))
    .b();
```

An annotation before `↖` specifies the language of the quoted fragment:

```rust
let html_frag = html↖<p>Hello</p>↗;
let wgsl_frag = wgsl↖vec4<f32>(1.0, 0.0, 0.0, 1.0)↗;
```

An un-annotated `↖…↗` defaults to the host language, or to the second language in the file's extension chain (e.g. `shaders.wgsl.rs.quilt` → bare quotes default to WGSL).

An annotation is a lowercase letter followed by letters or digits — `lean4↖…↗` as well as `lean↖…↗`, since both are registered names for the same language. The leading letter is what keeps a number that happens to abut the glyph as ordinary content: `x = 42↖…↗` is the literal `42` and a *bare* quote, not a quote of some language named `42`. The same shape spells the annotation on `↙…↘` and on `↓`.

### Unquote `↙…↘`

An unquote bracket splices an already-built term *into* a quote:

```rust
let one = ↖1↗;
let expr = ↖↙one↘ + 2↗;  // splices `one` at the hole position
```

The inner content of an unquote is code in the *ground* language evaluated at code-generation time.

### Pattern matching `let ↖…↗ = …`

A quote in the *binding position* of a Rust ground `let` is a **pattern**: instead of building a term, it destructures the value term by matching its shape. Each ground unquote inside the pattern must be a plain identifier and becomes a *metavariable* that binds the matching part of the value:

```rust
let py↖def f(↙args↘): pass↗ = py↖def f(x: int): pass↗;
// args now holds a term that coparses to "x: int"
```

The statement expands to a destructuring of `qmatch_n` (see `qmatch.rs`):

```rust
let [args] = qmatch_n(&/* pattern with mvar("args") markers */, &/* value */);
```

Matching is *syntactic*: both sides are compared as the source text they coparse to, anchored at both ends and whitespace-sensitive. Each metavariable binds the text between the surrounding literals (leftmost-shortest when ambiguous; two adjacent metavariables are rejected) as a leaf term that splices back verbatim. A failed match panics at generation time — the expanded `let` is irrefutable.

This is distinct from `rewrite_naive` (term-equality rewriting): pattern lets match during expansion, like macro patterns in Rust or Racket.

## Quasi-quoting and staging

Quilt is a *two-level* language. The ground level runs at code-generation time; the sky level (inside `↖…↗`) is data that will be serialized to the output file.

`Stage` in the engine tracks the current depth:
- `Stage::Ground` — executing code. Quotes capture their content and reduce become evaluations.
- `Stage::Sky(lang, depth)` — inside `depth` levels of nested quotes.

Nested quotes increment the depth; unquotes decrement it. An unquote whose depth would reach zero is *escaped* back to ground and its content is evaluated.

## Indentation and whitespace

Quilt automatically dedents the content of a quote block. Leading whitespace common to all lines inside `↖\n…\n↗` is stripped, so deeply-nested quoted code stays readable:

```rust
let block = ↖
    for i in 0..10 {
        println!("{i}");
    }
↗;
// The common four-space indent is stripped before parsing.
```

## Variadic nodes and emit `←`

Some language nodes accept an arbitrary number of children — `block` and `source_file` in Rust, for example. Inside a variadic quote, `←` (emit) appends a term to the growing list:

```rust
let program = ↖{
    ↙{
        for i in 0..n {
            ↖println!("{i}");↗.←;
        }
    }↘
}↗;
```

The unquote `↙{…}↘` runs a Rust loop at generation time; each iteration emits one `println!` statement into the outer block.

That is emit's *imperative* reading — append one term to an accumulator, once around the loop — and it needs a host with statements and a `QTerm` runtime to accumulate into. A pure host has the *functional* reading instead: build the whole list of fragments, then emit it in one go. In Nix, `←` is that join (see [Concrete languages](concrete-languages.md#as-a-host-string-based-meta)):

```nix
nix↖[ ↙← (map (s: nix↖"↙s↘"↗) services)↘ ]↗
```

## Lift `↑` and reduce `↓`

`↑` (lift) converts a value into a `QTerm`. What "lift" means is language-specific:

- **Rust `↑`** — calls `qlift()` on the value, building a term whose code reconstructs it. Integers become `integer_literal` nodes; strings become `string_literal` nodes; `Arc<QTerm>` values lift themselves recursively.
- **Python `↑`** — similar, but written *prefix*: `↑(x)` expands to the `quilt` module's `qlift(x)` function (a method can't hang off builtin ints), or to `qlift_html(x)` when the lift targets an HTML quote.
- **Heterogeneous `↑`** — inside a quote of another language (e.g. `python↖ … ↙x.↑↘ … ↗`), Rust's `↑` expands to `qlift_to::<L>()` for that target language, producing a term in the *target's* syntax. `LiftTo` impls exist for Python, WGSL, Zsh, and Bash (see `lift.rs`); e.g. a Rust `Vec<u64>` lifts into Python as a `list` literal, and a Rust string lifts into zsh as a properly escaped double-quoted word.

The operators `↑` (lift), `↓` (reduce), and `←` (emit) are **staged** like the brackets around them. An operator only spells itself out (`qlift(…)`, `reduce()`, …) once it reaches ground — when its enclosing unquotes bring it back to the running stage. An operator still nested inside an unresolved quote belongs to a *later* stage, so it is deferred: it survives expansion as its literal glyph, which the generated code spells out when it runs. So a nested generator emits a real operator for its output to expand — e.g. `py↖ … py↖ … ↙↑(a)↘ … ↗ … ↗` coparses with a literal `↙↑(a)↘`, not a prematurely-spelled `↙qlift(a)↘`.

`↓` (reduce) evaluates a `QTerm` by compiling it (via `rust-script` for Rust) and deserializing the result using `postcard`:

```rust
let n: i32 = ↓↖21 + 21↗;  // evaluates to 42 at generation time
```

## The `⟨T⟩` and `⟨N⟩` operators

These are used internally in bootstrap source:

- `⟨T⟩` expands to `Arc<QTerm>` — the canonical type of a quilt term in Rust meta-code.
- `⟨N⟩` creates an `identifier` node from a string — useful when building code that references a named variable.

Each host answers for itself: a string-based host (nix, lean) represents a
fragment as a plain string, so lean's `⟨T⟩` is `String` and `⟨N⟩` is the
identity. Where a host has no answer at all — `⟨T⟩` in untyped Nix — expansion
fails with a message saying so rather than emitting a placeholder.

## Why the arrows?

The quote glyphs `↖↗` and unquote glyphs `↙↘` are a synthesis of the two historical notations for quasi-quotation:

- **Quine corners `⌜⌝`** — introduced by W.V.O. Quine in *Mathematical Logic* (1940) as the standard logical notation for quasi-quotation. The corner bracket signals "treat the contents as a name of an expression, with holes where values can be substituted."
- **Lisp backtick `` ` ``** — the quasi-quote operator used in Lisp since the 1970s, where `` `(foo ,bar) `` quotes the list with `,bar` as an unquote splice. Almost every modern language's macro system (Scheme, Clojure, Rust's `quote!`, Julia's `:(...)`) descends from this convention.

Both traditions use distinct paired delimiters to mark the boundary between "code that runs" and "code that is data." Quilt's arrows extend this to *two* pairs — `↖↗` for quote and `↙↘` for unquote — so both levels are visually present at once. The direction mirrors the lift/reduce operators: the upward-pointing quote arrows (`↖↗`) lift code up into a term, just as `↑` does to a value; the downward-pointing unquote arrows (`↙↘`) bring a term back down into running code, just as `↓` evaluates one.
