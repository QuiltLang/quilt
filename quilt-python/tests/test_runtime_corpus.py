"""The Python runner for the shared runtime corpus (issue #159).

`conformance/runtime/cases.json` describes builder programs and the text each
must coparse to. The Rust and Node runners execute the same file, so a
divergence between the three published runtimes — quiltlang, quilt-python,
quilt-wasm — is a test failure rather than something a user discovers.

This file is the interpreter for one of them; it is deliberately thin, because
the point is that adding a runtime costs one small runner and inherits the
whole corpus.
"""

import json
import pathlib

import pytest
from quilt import HOLE, NL, POP, cmd, leaf, name, push, qlift, qlift_html
from quilt import quote as mk_quote
from quilt import sym, tb
from quilt import unquote as mk_unquote
from quilt import write

RUNTIME = "python"
CORPUS = (
    pathlib.Path(__file__).resolve().parents[2] / "conformance" / "runtime" / "cases.json"
)


def load_cases():
    data = json.loads(CORPUS.read_text())
    return [c for c in data["cases"] if RUNTIME in c.get("runtimes", [RUNTIME])]


def build_cmds(cmds):
    out = []
    for c in cmds:
        if c == "HOLE":
            out.append(HOLE)
        elif c == "NL":
            out.append(cmd(NL))
        elif c == "POP":
            out.append(cmd(POP))
        elif "write" in c:
            out.append(cmd(write(c["write"])))
        elif "push" in c:
            out.append(cmd(push(c["push"])))
        else:
            raise AssertionError(f"unknown cmd {c!r}")
    return out


def build_value(v):
    # A nested term means "lift an already-built term", which is how the corpus
    # pins qlift's behaviour on terms — see issue #166, where the three
    # runtimes disagree.
    return build(v) if isinstance(v, dict) else v


def build(t):
    """Interpret one corpus term against the Python runtime."""
    if "leaf" in t:
        return leaf(t["leaf"]["tag"], t["leaf"]["text"])
    if "sym" in t:
        return sym(t["sym"])
    if "name" in t:
        return name(t["name"])
    if "qlift" in t:
        return qlift(build_value(t["qlift"]))
    if "qlift_html" in t:
        return qlift_html(build_value(t["qlift_html"]))
    if "tb" in t:
        b = tb(t["tb"]["tag"])
        for step in t["tb"]["steps"]:
            if step == "n":
                b = b.n()
            elif step == "x":
                b = b.x()
            elif "w" in step:
                b = b.w(step["w"])
            elif "c" in step:
                b = b.c(build(step["c"]))
            elif "e" in step:
                b = b.e(build(step["e"]))
            elif "p" in step:
                b = b.p(step["p"])
            else:
                raise AssertionError(f"unknown step {step!r}")
        return b.b()
    for kind, ctor in (("quote", mk_quote), ("unquote", mk_unquote)):
        if kind in t:
            q = t[kind]
            return ctor(
                q["tag"],
                q["index"],
                q["lang"],
                build(q["term"]),
                build_cmds(q["cmds"]),
            )
    raise AssertionError(f"unknown term {t!r}")


CASES = load_cases()


def test_corpus_is_reachable():
    assert CASES, f"no corpus cases applied to the {RUNTIME} runtime ({CORPUS})"


@pytest.mark.parametrize("case", CASES, ids=[c["name"] for c in CASES])
def test_case(case):
    got = build(case["term"]).coparse()
    assert got == case["coparse"], (
        f"{case['name']}: coparse is {got!r}, corpus says {case['coparse']!r}"
    )


# ── the lift law ────────────────────────────────────────────────────────────
#
# reduce(lift(x)) == x. The corpus can only compare coparsed text; proving the
# law needs the generated code to actually be *evaluated*, which this runner can
# do and a JSON corpus cannot. Issue #166.
LAW_TERMS = [
    ("leaf", lambda: leaf("integer", "7")),
    ("sym", lambda: sym("+")),
    ("name", lambda: name("f")),
    ("binary", lambda: tb("binary_operator")
        .c(leaf("integer", "1")).w(" ").c(sym("+")).w(" ").c(leaf("integer", "2")).b()),
    ("newline", lambda: tb("block").w("a").n().w("b").b()),
    ("prefix", lambda: tb("block").w("{").p("    ").n().w("body").x().n().w("}").b()),
    ("quote", lambda: mk_quote("x", 0, "py", leaf("integer", "5"),
                               [cmd(write("[")), HOLE, cmd(write("]"))])),
    ("unquote", lambda: mk_unquote("x", 1, "py", leaf("integer", "5"), [HOLE])),
]


# The namespace the generated code is evaluated in: exactly the runtime's public
# names, as an expanded `.py.quilt` module would have them via `from quilt import
# *`. Passed explicitly rather than relying on this module's scope, which aliases
# `quote`/`unquote` to avoid colliding with the corpus builders.
import quilt as _quilt_module

LAW_NS = {n: getattr(_quilt_module, n) for n in dir(_quilt_module) if not n.startswith("_")}


@pytest.mark.parametrize("label,build_term", LAW_TERMS, ids=[n for n, _ in LAW_TERMS])
def test_lift_law(label, build_term):
    x = build_term()
    code = qlift(x).coparse()
    # eval is the point: this *is* the reduce step.
    back = eval(code, dict(LAW_NS))  # noqa: S307
    assert back.coparse() == x.coparse(), (
        f"{label}: reduce(lift(x)) != x\n  x    = {x.coparse()!r}\n"
        f"  lift = {code}\n  back = {back.coparse()!r}"
    )
