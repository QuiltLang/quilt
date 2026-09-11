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
    qlift_html,
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
    # Lifting a term yields the code that *reconstructs* it, not the term. That
    # is what reduce(lift(x)) == x requires: lift maps a value to a term whose
    # code evaluates back to it, so for a term the code has to be a constructor
    # call. This assertion used to read `== "7"` ("qlift is idempotent on
    # terms"), which encoded the bug fixed in issue #166.
    t = leaf("integer", "7")
    assert qlift(t).coparse() == 'leaf("integer", "7")'
    # And the law itself, checked by evaluating the generated code.
    assert eval(qlift(t).coparse()).coparse() == t.coparse()


def test_qlift_html():
    assert qlift_html(42).coparse() == "42"
    # strings are entity-escaped, so they are inert as text or attribute value
    assert qlift_html('a "<b>" & c').coparse() == "a &quot;&lt;b&gt;&quot; &amp; c"
    # qlift_html *is* a pass-through on terms, unlike qlift: an already-built
    # HTML fragment is already escaped, and HTML has no reduce for the
    # lift/reduce law to apply to.
    t = leaf("text", "x")
    assert qlift_html(t).coparse() == "x"


def test_str_and_repr():
    e = leaf("integer", "9")
    assert str(e) == "9"
    assert repr(e) == 'QTerm("9")'


def test_html_machine_is_native():
    """`html⟨M⟩` spawns the in-process HTML machine: a document whose
    definitions are its ids. Terms go in; the element's own markup comes back."""
    from quilt import spawn

    # The parser's shape for `<tag id="y">text</tag>` — what an `html↖…↗`
    # quote expands to — is what the machine reads ids from.
    def element(tag, id_, text):
        attr = (
            tb("attribute")
            .c(leaf("attribute_name", "id"))
            .c(sym("="))
            .c(tb("quoted_attribute_value").c(sym('"')).c(leaf("attribute_value", id_)).c(sym('"')).b())
            .b()
        )
        start = tb("start_tag").c(sym("<")).c(leaf("tag_name", tag)).w(" ").c(attr).c(sym(">")).b()
        end = tb("end_tag").c(sym("</")).c(leaf("tag_name", tag)).c(sym(">")).b()
        return tb("element").c(start).c(leaf("text", text)).c(end).b()

    page = spawn("html")
    assert page.lang == "html"
    assert page.eval(element("p", "y", "40")) is None  # a definition answers nothing
    assert page.eval(leaf("text", "#y")) == '<p id="y">40</p>'
    assert page.type_of(leaf("text", "#y")) == "p"
    # Redefining the id replaces the element; a fresh machine has none of it.
    page.eval(element("b", "y", "41"))
    assert page.eval(leaf("text", "#y")) == '<b id="y">41</b>'
    import pytest

    with pytest.raises(RuntimeError):
        spawn("html").eval(leaf("text", "#y"))
