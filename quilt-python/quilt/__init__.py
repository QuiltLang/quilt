from ._quilt import *
import ast
import os
import shutil
import subprocess
import tempfile

# Quilt operator/quote glyphs. Their presence in coparse() output means the term
# still holds Quilt source (a generated fragment that itself quotes), not plain
# target code, so it must be expanded before it can run.
_GLYPHS = "↖↗↙↘↑↓←⟨⟩"

def _reduce_src(src):
    """Reduce target/Quilt source to a value — the engine behind QTerm.reduce()/`↓`.

    If the source is still Quilt (contains glyphs, e.g. a generated fragment that
    itself quotes) it is expanded via `quilt` first. The source is then evaluated
    as a *block*: any leading statements run and the value is the trailing
    expression — None if it ends in a statement — matching the block-value
    semantics of Rust, Lisp `begin`, Ruby, etc. A lone expression just yields its
    value. The quilt runtime is in scope so expanded builder calls resolve.
    """
    ns = {}
    if any(g in src for g in _GLYPHS):
        src = expand(src)
        exec("from quilt import *", ns)
    body = ast.parse(src).body
    if body and isinstance(body[-1], ast.Expr):
        tail = body.pop().value
        exec(compile(ast.Module(body, []), "<reduce>", "exec"), ns)
        return eval(compile(ast.Expression(tail), "<reduce>", "eval"), ns)
    exec(compile(ast.Module(body, []), "<reduce>", "exec"), ns)
    return None

def reduce(term):
    """Reduce a term to a value — the `↓` operator (block-aware; see _reduce_src)."""
    return _reduce_src(term.coparse())

def _quilt_bin():
    """Locate the `quilt` expander: $QUILT (set by `quilt run`) or PATH."""
    qbin = os.environ.get("QUILT") or shutil.which("quilt")
    if not qbin:
        raise RuntimeError(
            "expand(): cannot find the `quilt` binary — set $QUILT or put "
            "`quilt` on PATH (it is set automatically when run via `quilt`)"
        )
    return qbin

def expand(src, lang="py"):
    """Expand Quilt source text to plain target source by invoking `quilt expand`.

    Unlike reduce()/`.↓` (which is plain-Python eval of coparse()), this handles
    source that still contains Quilt glyphs — e.g. a generated stage that itself
    quotes. It shells out to the prebuilt `quilt` binary, so nothing is compiled.
    """
    qbin = _quilt_bin()
    with tempfile.TemporaryDirectory() as d:
        inp = os.path.join(d, f"frag.{lang}.quilt")
        with open(inp, "w") as f:
            f.write(src)
        subprocess.run([qbin, "expand", inp], check=True, capture_output=True)
        with open(inp[: -len(".quilt")]) as f:  # quilt expand strips `.quilt`
            out = f.read()
    # Drop the leading `//! DO NOT EDIT…` header line(s) quilt expand prepends.
    lines = out.splitlines()
    while lines and lines[0].startswith("//!"):
        lines.pop(0)
    return "\n".join(lines).lstrip("\n")

def run(term, lang="py"):
    """Run a generated *stage* — Quilt source that may still contain glyphs — by
    expanding it and exec-ing it as a module. Returns the resulting namespace.

    The glyph-aware counterpart to reduce(): use reduce()/`.↓` for a single
    plain-Python expression, run() for a whole generated program (e.g. one that
    defines a generator). The quilt runtime is pre-imported into the namespace.
    """
    ns = {}
    exec("from quilt import *", ns)
    exec(compile(expand(term.coparse(), lang), "<quilt-stage>", "exec"), ns)
    return ns

def reduce_rs(term):
    """Evaluate a QTerm by running it as Rust code via rust-script (the `rs↓` operator)."""
    import importlib.resources as _ir
    quilt_dir = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

    input_code = term.coparse()
    out_fd, out_path = tempfile.mkstemp()
    os.close(out_fd)
    try:
        script = f"""
//! ```cargo
//! [dependencies]
//! quilt = {{ path = "{quilt_dir}", default-features = false, features = ["rust"] }}
//! postcard = {{ version = "1.1", features = ["alloc"] }}
//! ```
#[allow(unused_imports)]
use quilt::prelude::*;
use std::io::Write;
fn main() -> Result<()> {{
    let output: Arc<QTerm> = {input_code};
    let data = postcard::to_allocvec(&output).unwrap();
    let mut file = std::fs::File::create("{out_path}").unwrap();
    file.write_all(&data).unwrap();
    Ok(())
}}
"""
        with tempfile.NamedTemporaryFile(suffix=".rs", delete=False) as sf:
            sf.write(script.encode())
            script_path = sf.name
        try:
            result = subprocess.run(["rust-script", script_path], check=True)
        finally:
            os.unlink(script_path)
        with open(out_path, "rb") as f:
            data = f.read()
        return from_postcard_bytes(data)
    finally:
        try:
            os.unlink(out_path)
        except OSError:
            pass
