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
| `⟨M⟩` | machine       | Obtain a machine speaking a language              |

Quilt-level line comments are written `⟨//⟩ ...` and block comments `⟨/*⟩ ... ⟨*/⟩`. They are stripped during parsing and never appear in the output.

The target's own `// …` and `/* … */` comments pass through, and Quilt reads their text as prose: `↑`, `↓` and `←` inside one are just characters. At ground level that holds for everything up to the comment's end. Inside a quote, the brackets still work — the quote's closer ends the comment, and an unquote or a nested quote interrupts it, the comment resuming after — so a generated comment can take a hole, the way a `#` comment always could:

```rust
let header = rs↖
    // generated from ↙name.↑↘, do not edit
    fn f() {}
↗;
```

A literal arrow in a quoted comment needs its `\` escape, like anywhere else in a quote.

### Escaping a glyph

Prefixing any operator glyph with `\` makes it literal text rather than a Quilt
operator: `\↖ \↗ \↙ \↘ \↑ \↓ \← \⟨ \⟩`. This matters when the target language
uses one of these characters itself — Lean spells monadic bind `←` and coercion
`↑`, so a `do` block inside a `lean↖…↗` quote needs `\←` (or Lean's ASCII alias
`<-`, which Quilt does not touch). The set lives in `node::GLYPHS` and must stay
the single source of truth for the parser, which reads it directly.

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

An un-annotated `↖…↗` takes its language from, in order:

1. the last annotated quote before it **at the same level** — after `py↖…↗`, a bare `↖…↗` is Python too, until another annotation takes over;
2. the enclosing quote, when it sits inside that quote's `↙…↘`;
3. the next language in the file's extension chain (e.g. `shaders.wgsl.rs.quilt` → bare quotes default to WGSL);
4. the host language.

```rust
let a = py↖x + 1↗;   // Python
let b = ↖[y]↗;       // Python: inherits from `a`
let c = rs↖1 + 2↗;   // Rust
let d = ↖[z]↗;       // Rust
```

"The same level" is the same list of siblings: a quote's body, an unquote's body, or the file. An annotation inside an unquote stays in it, and does not reach the quotes after the enclosing quote. A bare `⟨M⟩` follows the same rule.

An annotation is a lowercase letter followed by letters or digits — `lean4↖…↗` as well as `lean↖…↗`, since both are registered names for the same language. The leading letter is what keeps a number that happens to abut the glyph as ordinary content: `x = 42↖…↗` is the literal `42` and a *bare* quote, not a quote of some language named `42`. The same shape spells the annotation on `↙…↘` and on `↓`.

### Representations `⟨cst⟩`, `⟨str⟩`, `⟨qt⟩`

A quote is always parsed against its language's grammar, so a malformed fragment is an expansion error wherever it is written. What the expanded code *builds* from it is the quote's **representation**, written between the language and the `↖`:

| Spelling | Builds | Use it for |
|---|---|---|
| `ts↖…↗` = `ts⟨cst⟩↖…↗` | a syntax tree in the object language, node by node (the default) | code that is inspected, matched (`let ↖…↗ = …`) or rewritten |
| `ts⟨str⟩↖…↗` | a host string, with the host's own string splicing — a TypeScript template literal | code that is only printed; as fast as hand-written string codegen |
| `ts⟨qt⟩↖…↗` | a flat term: text, newlines and indentation with holes, no object-language tags | printing with `cst`'s exact layout, at a fraction of its cost |

The choice is made at expansion, by the host's `MetaLanguage`: a `⟨str⟩` quote in a TypeScript host expands to one template literal, `` `T[${e}][ids[o + ${off}]] === 1` ``, with nothing to run at all; a `⟨qt⟩` quote to one builder chain with a write per run of text. Inside a quote's unquotes, `↑` and `←` build values of the quote's representation (`qlift_str` and a native join for `⟨str⟩`). A `⟨str⟩` value is a plain string, and its `coparse()` is itself, so callers written for terms keep working.

Representations are inherited like languages: by the bare quotes after an annotated one at the same level, while they are in its language (a `ts↖…↗` after a `ts⟨str⟩↖…↗` is `⟨str⟩` too; `⟨cst⟩↖` takes the default back, keeping the language), and by the quotes inside its unquotes. So a writer annotates its first quote and the rest follow.

Layout is the same in all three — `⟨str⟩` re-indents what it splices at an indented hole, line by line — with one difference: a host *string* holding a newline, spliced under an indent, is written raw by `⟨cst⟩` and `⟨qt⟩` and indented by `⟨str⟩`. A `⟨str⟩` or `⟨qt⟩` quote is a value, not a later stage: a quote nested directly inside one (rather than inside an unquote) is an error. Only TypeScript implements `⟨str⟩` and `⟨qt⟩` so far; other hosts reject them at the quote.

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
let [args] = qmatch_n(&*/* pattern with mvar("args") markers */, &/* value */);
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

That is emit's *imperative* reading — append one term to an accumulator, once around the loop — and it needs a host with statements and a `QTerm` runtime to accumulate into. A pure host has the *functional* reading instead: build the whole list of fragments, then emit it in one go. In Nix, `←` is that join (see [Concrete languages](concrete-languages.md#as-a-host-string-based-meta)), and TypeScript reads it the same way, written as a call — `↙←(items)↘` (see [TypeScript operators](concrete-languages.md#typescript-operators)):

```nix
nix↖[ ↙← (map (s: nix↖"↙s↘"↗) services)↘ ]↗
```

## Lift `↑` and reduce `↓`

`↑` (lift) converts a value into a `QTerm`. What "lift" means is language-specific:

- **Rust `↑`** — calls `qlift()` on the value, building a term whose code reconstructs it. Integers become `integer_literal` nodes; strings become `string_literal` nodes; `Arc<QTerm>` values lift themselves recursively.
- **Python `↑`** — similar, but written *prefix*: `↑(x)` expands to the `quilt` module's `qlift(x)` function (a method can't hang off builtin ints), or to `qlift_html(x)` when the lift targets an HTML quote.
- **Heterogeneous `↑`** — inside a quote of another language (e.g. `python↖ … ↙x.↑↘ … ↗`), Rust's `↑` expands to `qlift_to::<L>()` for that target language, producing a term in the *target's* syntax. `LiftTo` impls exist for every registered language — Python, TypeScript, HTML, WGSL, Zsh, Bash, Nix, Lean, SQL/MySQL and plain text (see `lift/gen.rs` and `lift/text.rs`; the [support matrix](support-matrix.md) has which value types each takes); e.g. a Rust `Vec<u64>` lifts into Python as a `list` literal, and a Rust string lifts into zsh as a properly escaped double-quoted word. The Python host reaches TypeScript (`qlift_ts`), HTML (`qlift_html`) and text (`qlift_text`) the same way; the TypeScript host reaches HTML and text.

### Lifting into text

Every other target has a grammar, and the grammar decides what a lifted value looks like. Plain text (`txt↖…↗`) has none, which is why it is the target for formats Quilt has no grammar for — Caddyfiles, nginx configs, YAML fragments, `.env` files, in-house DSLs. So `↑` into text is spelled by a **dialect**: a small set of lift rules the host chooses. Quilt knows no config format by name. It ships a default and a verbatim dialect, and a format is a dialect its author writes by overriding only the rules that differ.

The default dialect, `Json`, spells every value as JSON does:

| value | `↑` into `txt↖…↗` |
| --- | --- |
| string | `"…"`, with `"`, `\` and control characters escaped (`\n`, `\t`, `\u0001`) |
| integer | its decimal digits |
| float | the shortest round-trip form, decimal point kept (`1.0`); NaN and infinity fail at the lift |
| bool | `true` / `false` |
| list | `[a, b]`, each element lifted the same way |
| already-built term | the text it stands for |

JSON is the default because it is the closest thing config formats have to a common literal syntax: the same text is a valid YAML scalar or flow sequence, a TOML value, a JavaScript or HCL literal. A newline in a value is `\n`, so a lifted value cannot start a new line of config.

**Quote-aware.** A text quote does not parse, so `↑` cannot ask the tree where it is — but the expander can look at the characters beside the hole. When the hole sits directly between two `"` the author wrote, `↑` is spelled for the inside of a string, and a string renders as its escaped contents rather than a second pair of quotes:

```rust
txt↖name: ↙name.↑↘↗        // name: "O'Brien \"x\""
txt↖name: "↙name.↑↘"↗      // the same text
```

Numbers and booleans read the same in either position; a list inside quotes is its elements and separators, without brackets. An escaped quote (`\"↙x↘\"`) is a quote character in the text, not a string the value is inside.

**Raw.** `Raw` writes a string as its own characters, unescaped, and a list as words separated by spaces. In a Rust host, splicing a `&str` with `↙s↘` (no `↑`) already does that.

**Writing a dialect.** In Rust, implement `TextDialect`. Every method defaults to JSON's rule:

```rust
use quilt::lift::{TextDialect, json_string};

struct Conf;
impl TextDialect for Conf {
    const LIST_OPEN: &'static str = "";   // list layout: `a b c`
    const LIST_SEP: &'static str = " ";
    const LIST_CLOSE: &'static str = "";
    fn string(s: &str) -> String {        // bare when it can be
        if s.chars().all(|c| c.is_alphanumeric() || "._-/:".contains(c)) && !s.is_empty() {
            s.to_string()
        } else {
            json_string(s)
        }
    }
    // also: `in_quotes(s)`, `boolean(b)`, `float(f)`
}
```

A rule with no safe spelling for a value — a newline where a bare token must go — should panic with a message saying so: `↑` cannot return an error, and a failure at the lift site is one the author sees.

**Choosing a dialect.** Each host decides where the lift is spelled:

| host | per module | per site |
| --- | --- | --- |
| Rust | `type Text = quilt::lift::Text<Conf>;` — `↑` names `Text` unqualified, so this shadows the prelude's `Text<Json>` | `↙x.qlift_to::<Text<Conf>>()↘` |
| Python | `qlift_text = Conf().lift` and `qlift_text_in_quotes = Conf().lift_in_quotes` (subclass `TextDialect`) | `↙qlift_text(x, Conf)↘` |
| TypeScript | — (the runtime import is fixed) | `↙qlift_text(x, conf)↘`, where `conf` is an object with any of `string`, `in_quotes`, `boolean`, `float`, `list_open`, `list_sep`, `list_close` |

`examples/config_gen.rs.quilt` generates a YAML fragment, a Caddyfile and a small DSL this way, with no escaping written at any splice site, and `quilt/tests/expand_text.rs` runs what it generates.

The operators `↑` (lift), `↓` (reduce), and `←` (emit) are **staged** like the brackets around them. An operator only spells itself out (`qlift(…)`, `reduce()`, …) once it reaches ground — when its enclosing unquotes bring it back to the running stage. An operator still nested inside an unresolved quote belongs to a *later* stage, so it is deferred: it survives expansion as its literal glyph, which the generated code spells out when it runs. So a nested generator emits a real operator for its output to expand — e.g. `py↖ … py↖ … ↙↑(a)↘ … ↗ … ↗` coparses with a literal `↙↑(a)↘`, not a prematurely-spelled `↙qlift(a)↘`.

`↓` (reduce) evaluates a `QTerm` by compiling it (via `rust-script` for Rust) and deserializing the result using `postcard`:

```rust
let n: i32 = ↓↖21 + 21↗;  // evaluates to 42 at generation time
```

## The `⟨T⟩`, `⟨N⟩` and `⟨M⟩` operators

The first two are used internally in bootstrap source:

- `⟨T⟩` expands to `Arc<QTerm>` — the canonical type of a quilt term in Rust meta-code.
- `⟨N⟩` creates an `identifier` node from a string — useful when building code that references a named variable.

`⟨M⟩` obtains a **machine** — a stateful evaluator of a language
([machines](../design/machines.md), issue #273). Like `↖…↗` it takes an
optional language annotation, and like `↖…↗` the bare form resolves through
the file's extension chain, so a `.sql.py.quilt` file needs no annotation at
all:

```python
# report.sql.py.quilt — Python ground, SQL fragments, a SQL machine
db = ⟨M⟩                       # = spawn("sql")
sh = bash⟨M⟩                   # = spawn("bash"), a persistent shell
db.↓(↖CREATE TEMP TABLE menu(item TEXT, price REAL);↗)
print("SUM(price) is a", db.⟨T⟩(↖(SELECT SUM(price) FROM menu)↗))
```

Each occurrence obtains one machine; sharing is ordinary binding, so two
`⟨M⟩`s are two isolated machines. The syntax names the *language* — which
provider answers (a live `sqlite3`, a persistent shell, a replay script) is
the registry's question, which is what keeps provider details out of the
program. In Rust ground it spells `qspawn("sql")`, which returns a
`Result`, so it is usually written `sql⟨M⟩?`.

The last two lines above show the **method-position** rule that `↓` and `⟨T⟩`
share: a glyph flush against an argument list is the machine judgment it
names, spelled as a method. `db.↓(t)` asks for a value, `db.⟨T⟩(t)` asks for
a type; with a space, or anywhere else, both keep their operator meanings.

Each host answers for itself: a string-based host (nix, lean) represents a
fragment as a plain string, so lean's `⟨T⟩` is `String` and `⟨N⟩` is the
identity. Where a host has no answer at all — `⟨T⟩` in untyped Nix — expansion
fails with a message saying so rather than emitting a placeholder.

## Why the arrows?

The quote glyphs `↖↗` and unquote glyphs `↙↘` are a synthesis of the two historical notations for quasi-quotation:

- **Quine corners `⌜⌝`** — introduced by W.V.O. Quine in *Mathematical Logic* (1940) as the standard logical notation for quasi-quotation. The corner bracket signals "treat the contents as a name of an expression, with holes where values can be substituted."
- **Lisp backtick `` ` ``** — the quasi-quote operator used in Lisp since the 1970s, where `` `(foo ,bar) `` quotes the list with `,bar` as an unquote splice. Almost every modern language's macro system (Scheme, Clojure, Rust's `quote!`, Julia's `:(...)`) descends from this convention.

Both traditions use distinct paired delimiters to mark the boundary between "code that runs" and "code that is data." Quilt's arrows extend this to *two* pairs — `↖↗` for quote and `↙↘` for unquote — so both levels are visually present at once. The direction mirrors the lift/reduce operators: the upward-pointing quote arrows (`↖↗`) lift code up into a term, just as `↑` does to a value; the downward-pointing unquote arrows (`↙↘`) bring a term back down into running code, just as `↓` evaluates one.
