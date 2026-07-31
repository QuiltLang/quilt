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
from quilt import HOLE, NL, POP, cmd, from_postcard_bytes, leaf, name, push, qlift, qlift_html
from quilt import quote as mk_quote
from quilt import sym, tb
from quilt import unquote as mk_unquote
from quilt import write

RUNTIME = "python"
CORPUS = (
    pathlib.Path(__file__).resolve().parents[2] / "conformance" / "runtime" / "cases.json"
)

# The cross-cutting checks this runner knows how to run. The corpus says which
# runtimes each of its checks applies to; anything it names us for that is not
# in here is a failure, not a skip — see test_every_declared_check_is_implemented.
IMPLEMENTED_CHECKS = {"postcard_roundtrip", "stringify_agrees_with_coparse"}


def load_cases():
    data = json.loads(CORPUS.read_text())
    return [c for c in data["cases"] if RUNTIME in c.get("runtimes", [RUNTIME])]


def load_checks():
    data = json.loads(CORPUS.read_text())
    return [c for c in data.get("checks", []) if RUNTIME in c["runtimes"]]


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
CHECKS = load_checks()
CHECK_NAMES = {c["name"] for c in CHECKS}


def test_corpus_is_reachable():
    assert CASES, f"no corpus cases applied to the {RUNTIME} runtime ({CORPUS})"


@pytest.mark.parametrize("case", CASES, ids=[c["name"] for c in CASES])
def test_case(case):
    got = build(case["term"]).coparse()
    assert got == case["coparse"], (
        f"{case['name']}: coparse is {got!r}, corpus says {case['coparse']!r}"
    )


# ── the cross-cutting checks ────────────────────────────────────────────────
#
# A check is a property every corpus shape must have, rather than a term and its
# expected text, so it covers the whole corpus without restating any of it.


def test_every_declared_check_is_implemented():
    """A check naming this runtime that it cannot run is a failure, not a skip.

    Otherwise adding one to the corpus would quietly do nothing here while
    passing everywhere it *is* implemented — the silent-gap failure mode the
    conformance epic (#144) exists to close.
    """
    unknown = sorted(CHECK_NAMES - IMPLEMENTED_CHECKS)
    assert not unknown, (
        f"corpus check(s) {unknown} name the {RUNTIME} runtime, but this runner has no "
        f"implementation of them ({CORPUS})"
    )


@pytest.mark.skipif(
    "postcard_roundtrip" not in CHECK_NAMES,
    reason="the corpus does not declare postcard_roundtrip for this runtime",
)
@pytest.mark.parametrize("case", CASES, ids=[c["name"] for c in CASES])
def test_postcard_roundtrip(case):
    """`from_postcard_bytes(x.postcard_bytes())` must give back `x` (issue #192).

    This pair is the wire format of the heterogeneous reduce protocol: `rs↓` in
    Python decodes bytes the Rust expander wrote, and `py↓` in Rust decodes
    bytes written here. Nothing tested either direction.

    The bytes assertion is not redundant with the text one: `postcard` is
    positional and self-describes nothing, so an asymmetry between encode and
    decode does not fail — it produces a *different term*, which may still
    coparse the same.

    What no corpus case can see is a *symmetric* schema change, because these
    terms are all constructed and so carry no spans. That one is a Rust test
    (`qterm::tests::postcard_round_trip_preserves_spans`), where a spanned term
    can actually be built.
    """
    x = build(case["term"])
    data = x.postcard_bytes()
    back = from_postcard_bytes(data)
    assert back.coparse() == x.coparse(), (
        f"{case['name']}: postcard round trip coparses to {back.coparse()!r}, "
        f"term is {x.coparse()!r}"
    )
    assert back.postcard_bytes() == data, (
        f"{case['name']}: postcard round trip re-serializes to "
        f"{len(back.postcard_bytes())} bytes, not the original {len(data)} — "
        "a field is being lost on decode"
    )


@pytest.mark.skipif(
    "stringify_agrees_with_coparse" not in CHECK_NAMES,
    reason="the corpus does not declare stringify_agrees_with_coparse for this runtime",
)
@pytest.mark.parametrize("case", CASES, ids=[c["name"] for c in CASES])
def test_stringify_agrees_with_coparse(case):
    """`str(x)` is `x.coparse()` (issue #192).

    Both are exported next to `coparse`, and an f-string reaches for `__str__`
    without the author choosing it, so the two must not drift apart.

    `repr` is only checked for shape: it wraps the *quoted* source, and the
    quoting is Rust's `Debug` for `str`, whose escaping is not Python's `!r`.
    """
    x = build(case["term"])
    assert str(x) == x.coparse(), f"{case['name']}: str(x) is {str(x)!r}"
    r = repr(x)
    assert r.startswith('QTerm("') and r.endswith('")'), (
        f"{case['name']}: repr(x) is {r!r}, expected QTerm(\"…\")"
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
