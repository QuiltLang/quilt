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

`quilt run` automatically sets `PYTHONPATH` to include the `quilt-python/` directory.

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
name("ident")   # identifier node (⟨N⟩ operator)
```

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
#!/usr/bin/env quilt run
from quilt import *

expr = ↖1 + 2↗
print("expr   =", expr.coparse())

ten = ↖10↗
scaled = ↖↙ten↘ * 100↗
print("scaled =", scaled.coparse())
```

Run with `quilt run examples/hello.py.quilt`.

## Relation to Rust bindings

The Python builder API is intentionally parallel to the Rust `QTermBuilder` API. The main differences are:

| Rust                                              | Python                                       |
|---------------------------------------------------|----------------------------------------------|
| `.c(&child)`                                      | `.c(child)`                                  |
| `Arc<QTerm>`                                      | opaque `QTerm` object                        |
| `qlift()` is a method on values via `QLift` trait | `qlift()` is a method on `QTerm` objects     |
| Variadic block uses imperative `b_`               | Variadic block uses fluent `.e(child)` chain |
