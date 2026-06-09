"""Tests for the quilt_python bindings.

Run after building the module: `bin/build-py`, then
`PYTHONPATH=. python3 -m pytest tests/` from rust/quilt_python.
"""

from quilt import (
    tb,
    leaf,
    sym,
    quote,
    unquote,
    cmd,
    write,
    name,
    qlift,
    HOLE,
)


def test_builder_coparse():
    expr = (
        tb("binary_operator")
        .c(leaf("integer", "1"))
        .w(" ")
        .c(sym("+"))
        .w(" ")
        .c(leaf("integer", "2"))
        .b()
    )
    assert expr.coparse() == "1 + 2"


def test_leaf_and_sym():
    assert leaf("integer", "42").coparse() == "42"
    assert sym("+").coparse() == "+"
    assert name("foo").coparse() == "foo"


def test_quote_with_cmds_and_hole():
    q = quote("x", 0, "py", leaf("integer", "5"), [cmd(write("[")), HOLE, cmd(write("]"))])
    assert q.coparse() == "[5]"


def test_unquote():
    u = unquote("x", 1, "py", leaf("integer", "5"), [HOLE])
    assert u.coparse() == "5"


def test_qlift():
    assert qlift(42).coparse() == "42"
    assert qlift("hi").coparse() == '"hi"'
    # qlift is idempotent on terms
    t = leaf("integer", "7")
    assert qlift(t).coparse() == "7"


def test_str_and_repr():
    e = leaf("integer", "9")
    assert str(e) == "9"
    assert repr(e) == 'QTerm("9")'
