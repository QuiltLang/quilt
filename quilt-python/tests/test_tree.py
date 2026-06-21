"""Tests for the directory layer bindings (issue #96): QTree + node builders +
Tier A instantiation, mirroring the Rust `quilt::tree`/`quilt::template`.

Run after building the module: `bin/build-py`, then
`PYTHONPATH=. python3 -m pytest tests/` from quilt-python.
"""

import os

import pytest

from quilt import (
    QTree,
    Node,
    file,
    raw,
    link,
    subdir,
    emit_tree,
    scaffold_param,
    instantiate,
    leaf,
    name,
    unquote,
    HOLE,
)


def test_emit_and_len():
    t = QTree()
    t.emit("Cargo.toml", raw("[package]\n"))
    t.emit("src/main.rs", file(leaf("source_file", "fn main() {}")))
    # One "src" dir + one "Cargo.toml" at the top level.
    assert len(t) == 2


def test_node_kinds():
    assert repr(file(leaf("x", "1"))) == "Node(file)"
    assert repr(raw(b"bytes")) == "Node(raw)"
    assert repr(raw("text")) == "Node(raw)"
    assert repr(link("a/b")) == "Node(link)"
    assert repr(subdir(QTree())) == "Node(dir)"


def test_subdir_nesting_and_listing():
    inner = QTree()
    inner.emit("main.rs", file(leaf("source_file", "fn main() {}")))
    t = QTree()
    t.emit("a.txt", raw("hi"))
    t.emit("src", subdir(inner))
    listing = t.listing()
    assert "a.txt (raw, 2 bytes)" in listing
    assert "src/" in listing
    assert "src/main.rs (file)" in listing


def test_emit_rejects_bad_path():
    t = QTree()
    with pytest.raises(ValueError):
        t.emit("a/../b", raw("x"))


def test_overlay_right_wins_and_merges_dirs():
    a = QTree()
    a.emit("shared.txt", raw("a"))
    a.emit("src/main.rs", raw("a-main"))
    b = QTree()
    b.emit("shared.txt", raw("b"))
    b.emit("src/lib.rs", raw("b-lib"))
    merged = a.overlay(b)
    # shared.txt + src dir (holding both main.rs and lib.rs).
    assert len(merged) == 2
    assert "src/main.rs" in merged.listing()
    assert "src/lib.rs" in merged.listing()


def test_merge_errors_on_leaf_collision():
    a = QTree()
    a.emit("x", raw("1"))
    b = QTree()
    b.emit("x", raw("2"))
    with pytest.raises(ValueError):
        a.merge(b)


def test_emit_tree_writes_postcard_sidecar(tmp_path):
    # With $QUILT_TREE_OUT set, emit_tree writes the postcard-encoded tree there.
    out = tmp_path / "tree.postcard"
    t = QTree()
    t.emit("a.txt", raw("hi"))
    os.environ["QUILT_TREE_OUT"] = str(out)
    try:
        emit_tree(t)
    finally:
        del os.environ["QUILT_TREE_OUT"]
    assert out.exists()
    assert out.stat().st_size > 0


def test_scaffold_param_reads_env():
    os.environ["QUILT_PARAM_greeting"] = "hi"
    try:
        assert scaffold_param("greeting") == "hi"
    finally:
        del os.environ["QUILT_PARAM_greeting"]
    assert scaffold_param("definitely_unset_param") is None


def test_tier_a_instantiate_fills_a_hole():
    # A hand-built sky-first template: `msg = ↙greeting↘` with a py-language hole.
    hole = unquote("u", 0, "py", name("greeting"), [HOLE])
    from quilt import tb

    tmpl = tb("expr").w("msg = ").c(hole).b()
    out = instantiate(tmpl, {"greeting": "hi"})
    assert out.coparse() == 'msg = "hi"'


def test_tier_a_instantiate_lifts_scalars():
    hole = unquote("u", 0, "py", name("n"), [HOLE])
    from quilt import tb

    tmpl = tb("expr").w("n = ").c(hole).b()
    assert instantiate(tmpl, {"n": 3}).coparse() == "n = 3"
    assert instantiate(tmpl, {"n": True}).coparse() == "n = True"
    assert instantiate(tmpl, {"n": [1, 2]}).coparse() == "n = [1, 2]"


def test_tier_a_missing_param_errors():
    hole = unquote("u", 0, "py", name("missing"), [HOLE])
    from quilt import tb

    tmpl = tb("expr").w("x = ").c(hole).b()
    with pytest.raises(ValueError):
        instantiate(tmpl, {})
