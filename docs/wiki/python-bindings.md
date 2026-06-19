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
reduce(term)     # eval(term.coparse()) — the `↓` operator. Plain-Python, single
                 # expression; the source must NOT contain Quilt glyphs.
run(term)        # for a whole generated *stage*: expand its glyphs via `quilt`,
                 # then exec it as a module; returns the resulting namespace.
expand(src)      # expand Quilt source text to plain Python by shelling out to
                 # `quilt expand` (no compilation). run() is expand() + exec.
reduce_rs(term)  # the `rs↓` operator: evaluate a term as Rust via rust-script.
```

`reduce`/`.↓` cannot run a *generated* stage that itself quotes — its `coparse()`
still has glyphs, which `eval` rejects. `run()` bridges that: it re-invokes the
prebuilt `quilt` binary (found via `$QUILT`, set automatically when launched by
`quilt`, else `PATH`) to expand the fragment, so multi-stage towers run
in-process. See `examples/staged_pow.py.quilt`.

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
