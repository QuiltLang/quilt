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
    # qlift is idempotent on terms
    t = leaf("integer", "7")
    assert qlift(t).coparse() == "7"


def test_qlift_html():
    assert qlift_html(42).coparse() == "42"
    # strings are entity-escaped, so they are inert as text or attribute value
    assert qlift_html('a "<b>" & c').coparse() == "a &quot;&lt;b&gt;&quot; &amp; c"
    # qlift_html is idempotent on terms
    t = leaf("text", "x")
    assert qlift_html(t).coparse() == "x"


def test_str_and_repr():
    e = leaf("integer", "9")
    assert str(e) == "9"
    assert repr(e) == 'QTerm("9")'


# --- #96: the directory layer (QTree + sinks) ------------------------------

import os
import shutil

import pytest
from quilt import QTree, file, raw, link, subdir, write_tree


def test_qtree_emit_and_len():
    t = QTree()
    t.emit("Cargo.toml", raw(b"[package]\n"))
    t.emit("src/main.rs", file(leaf("source_file", "fn main() {}")))
    # "src" is created once; the tree has two top-level entries.
    assert len(t) == 2


def test_qtree_rejects_bad_path():
    t = QTree()
    with pytest.raises(ValueError):
        t.emit("a/../b", raw("x"))


def test_raw_accepts_bytes_or_str():
    # Both spellings round-trip to the same bytes on disk.
    assert repr(raw(b"x")) == "Node(raw)"
    assert repr(raw("x")) == "Node(raw)"
    assert repr(file(leaf("module", "x = 1"))) == "Node(file)"
    assert repr(subdir(QTree())) == "Node(dir)"
    assert repr(link("other.txt")) == "Node(link)"


def test_write_tree(tmp_path):
    t = QTree()
    t.emit("README.md", raw("# hi\n"))
    t.emit("src/app.py", file(leaf("module", "x = 1\n")))
    t.emit("empty", subdir(QTree()))  # empty dir is kept
    out = str(tmp_path / "proj")
    report = dict(write_tree(t, out))
    assert report["README.md"] == "create"
    assert report["src/app.py"] == "create"
    assert open(os.path.join(out, "README.md")).read() == "# hi\n"
    assert open(os.path.join(out, "src/app.py")).read() == "x = 1\n"
    assert os.path.isdir(os.path.join(out, "empty"))


def test_write_tree_conflict_policy(tmp_path):
    out = str(tmp_path / "proj")
    os.makedirs(out)
    with open(os.path.join(out, "a.txt"), "w") as f:
        f.write("old")
    t = QTree()
    t.emit("a.txt", raw("new"))
    # The default policy refuses to clobber an existing file.
    with pytest.raises(Exception):
        write_tree(t, out)
    assert open(os.path.join(out, "a.txt")).read() == "old"
    # ... overwrite replaces it.
    report = dict(write_tree(t, out, on_conflict="overwrite"))
    assert report["a.txt"] == "overwrite"
    assert open(os.path.join(out, "a.txt")).read() == "new"


def test_write_tree_dry_run_writes_nothing(tmp_path):
    out = str(tmp_path / "proj")
    t = QTree()
    t.emit("a.txt", raw("hi"))
    report = dict(write_tree(t, out, dry_run=True))
    assert report["a.txt"] == "create"
    assert not os.path.exists(os.path.join(out, "a.txt"))


def test_tier_a_instantiate():
    # The Tier A helper shells out to the `quilt` binary; skip when it is absent.
    if not (os.environ.get("QUILT") or shutil.which("quilt")):
        pytest.skip("no quilt binary on PATH (set $QUILT)")
    from quilt import instantiate

    out = instantiate("greeting = ↙who↘\nn = ↙count↘\n", lang="py", who="bob", count=3)
    assert 'greeting = "bob"' in out
    assert "n = 3" in out
