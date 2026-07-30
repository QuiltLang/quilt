# Python Bindings

**Crate:** `quilt-python/` — `quilt_python` (Cargo), `quilt` (Python import name)

The `quilt_python` crate exposes the core Quilt IR to Python via PyO3. It is the *runtime* that expanded `.py.quilt` files import:

```python
from quilt import *
```

## Building

```sh
build-py   # maturin build --release, then installs the module into the package dir
```

This builds a wheel with maturin and extracts the native extension module into `quilt-python/quilt/` as `_quilt.abi3.so`. The module name is `quilt._quilt`; the `quilt/__init__.py` re-exports everything for `from quilt import *`.

The crate targets ABI3 (`abi3-py38`) so one build works for CPython ≥ 3.8.

`quilt` automatically sets `PYTHONPATH` to include the `quilt-python/` directory.

## API

The Python API mirrors the Rust `QTermBuilder` fluent interface.

### Term constructors

```python
leaf(tag, code)    # childless node with Write(code) command
sym(s)             # leaf where tag == code
tb(tag)            # begin a Tuple builder
qb(tag, i, lang)   # begin a Quote builder
ub(tag, i, lang)   # begin an Unquote builder
quote(tag, i, lang, term, cmds)   # construct a Quote QTerm directly
unquote(tag, i, lang, term, cmds) # construct an Unquote QTerm directly
```

### Builder fluent methods

Called on a builder object returned by `tb/qb/ub`:

```python
builder.w("text")      # Write
builder.n()            # NewLine
builder.p("  ")        # Push indent prefix
builder.x()            # Pop prefix
builder.c(child)       # insert child (no & — Python doesn't have borrows)
builder.e(child)       # emit (like .c but semantically "emit into variadic")
builder.b()            # build → QTerm
```

### Command constructors

```python
write("text")   # StrCmd::Write
push("  ")      # StrCmd::Push
NL              # StrCmd::NewLine constant
POP             # StrCmd::Pop constant
HOLE            # CmdOrHole::Hole constant
cmd(strcmd)     # CmdOrHole::Cmd(strcmd)
```

> These three are **constants** here and **functions** (`NL()`, `POP()`,
> `HOLE()`) in the `quilt-wasm` runtime that `.ts.quilt` files target — the one
> shape change to watch for when porting a metaprogram between the two hosts.
> See [Relation to the wasm runtime](#relation-to-the-wasm-runtime).

### QTerm methods

```python
term.coparse()      # serialize to a string
term.qlift()        # lift to builder code (like Rust's QLift trait)
```

### Other functions

```python
name("ident")       # identifier node (⟨N⟩ operator)
qlift(value)        # lift int/str/QTerm to a Python term (↑ into a py quote)
qlift_html(value)   # lift int/str/QTerm to HTML text, entity-escaped (↑ into an html quote)
```

### Running generated code

Helpers (in `quilt/__init__.py`) for evaluating a term's `coparse()` output:

```python
reduce(term)     # the `↓` operator: run the term's code and return the value.
                 # Glyph-aware (expands a still-Quilt fragment via `quilt` first)
                 # and block-aware (runs leading statements, returns the trailing
                 # expression — the block value).
run(term)        # run a generated stage and return its whole namespace (dict),
                 # e.g. when you want several bindings, not one value.
expand(src)      # expand Quilt source text to plain Python by shelling out to
                 # `quilt expand` (no compilation). run() is expand() + exec.
reduce_rs(term)  # the `rs↓` operator: evaluate a term as Rust via rust-script.
```

`reduce`/`.↓` evaluates a term to a value, and does so across stages:

- **Glyph-aware** — if the source is still Quilt (it contains glyphs, e.g. a
  generated fragment that itself quotes) it is expanded via the prebuilt `quilt`
  binary first (found via `$QUILT`, set automatically when launched by `quilt`,
  else `PATH`).
- **Block-aware** — a generated *stage* is usually a statement sequence, not a
  bare expression. `↓` runs the leading statements and returns the value of the
  trailing expression — None if it ends in a statement — the block-value
  semantics of Rust (`{ …; expr }`), Lisp `begin`, Ruby, etc. So a stage ending
  in its result expression reduces straight to that result. `examples/staged_pow.py.quilt`
  ends Stage 2 with its `make_scaled` generator and reduces it with `stage2.↓`.

`run()` remains for when you want the stage's whole namespace rather than a
single value.

## How expanded `.py.quilt` code looks

When the Quilt engine expands a Python `.quilt` file, each `↖…↗` quote becomes a call that constructs a `QTerm`:

```python
# Source:
expr = ↖1 + 2↗

# Expanded (approximately):
expr = (tb("binary_expression")
    .c(leaf("integer_literal", "1"))
    .w(" + ")
    .c(leaf("integer_literal", "2"))
    .b())
```

And an unquote `↙x↘` becomes a `.c(x)` splice at the corresponding hole position.

## Example: `hello.py.quilt`

```python
#!/usr/bin/env quilt
from quilt import *

expr = ↖1 + 2↗
print("expr   =", expr.coparse())

ten = ↖10↗
scaled = ↖↙ten↘ * 100↗
print("scaled =", scaled.coparse())
```

Run with `quilt examples/hello.py.quilt`.

## Relation to Rust bindings

The Python builder API is intentionally parallel to the Rust `QTermBuilder` API. The main differences are:

| Rust                                              | Python                                       |
|---------------------------------------------------|----------------------------------------------|
| `.c(&child)`                                      | `.c(child)`                                  |
| `Arc<QTerm>`                                      | opaque `QTerm` object                        |
| `↑` is postfix: `x.↑` → `x.qlift()` (`QLift` trait) | `↑` is prefix: `↑(x)` → free `qlift(x)` function |
| Variadic block uses imperative `b_`               | Variadic block uses fluent `.e(child)` chain |

## Relation to the wasm runtime

`quilt-wasm` is the third published runtime — the one expanded `.ts.quilt`
files target — and is a closer match to this one: same fluent builder, same
by-value `.c(child)`, same prefix `qlift(x)`, same `.e(child)` variadic chain.
All three are held together by a shared corpus
(`conformance/runtime/cases.json`), so a drift between them is a test failure.

One difference remains:

| Python                            | quilt-wasm                          |
|-----------------------------------|-------------------------------------|
| `NL`, `POP`, `HOLE` are constants | `NL()`, `POP()`, `HOLE()` are functions |

```python
cmd(NL)                                         # quilt-python
[cmd(write("[")), HOLE, cmd(write("]"))]
```

```js
cmd(NL())                                       // quilt-wasm
[cmd(write("[")), HOLE(), cmd(write("]"))]
```

Expanded code is always right for its own target, since each meta-language
emits its own runtime's spelling; the cost falls on a human porting a
metaprogram across hosts. `wasm-bindgen` cannot export a module-scope constant
at all — `#[wasm_bindgen]` on a `const` is a compile error, and only functions,
structs, enums and impls reach JS — and a shared singleton would in any case be
consumed by its first use, because wasm-bindgen *moves* struct values passed in
arrays. Issue #167 weighed the alternative (a hand-maintained JS entry point
wrapping wasm-pack's output) and kept the divergence; `quilt-wasm/README.md`
records the full reasoning.
