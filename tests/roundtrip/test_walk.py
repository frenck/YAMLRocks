"""YAMLRocksDocument traversal: walk() and to_dict() (yamlium-inspired)."""

from __future__ import annotations

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

SOURCE = b"""\
server:
  host: localhost
  ports:
    - 80
    - 443
name: app
"""


def test_to_dict():
    """to_dict() returns the whole document as plain nested Python data."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert doc.to_dict() == {
        "server": {"host": "localhost", "ports": [80, 443]},
        "name": "app",
    }


def test_walk_yields_paths_and_values():
    """walk() yields a path and value for every leaf scalar."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    walked = {tuple(p): v for p, v in doc.walk()}
    assert walked == {
        ("server", "host"): "localhost",
        ("server", "ports", 0): 80,
        ("server", "ports", 1): 443,
        ("name",): "app",
    }


def test_walk_enables_bulk_edits():
    """walk() paths can be used to locate and mutate leaves in place."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    for path, value in doc.walk():
        if value == "localhost":
            node = doc
            for key in path[:-1]:
                node = node[key]
            node[path[-1]] = "0.0.0.0"
    assert doc["server"]["host"] == "0.0.0.0"
    assert b"0.0.0.0" in doc.to_yaml()


def test_view_walk_is_relative():
    """walk() on a view yields paths relative to that view."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    walked = {tuple(p): v for p, v in doc["server"].walk()}
    assert walked == {
        ("host",): "localhost",
        ("ports", 0): 80,
        ("ports", 1): 443,
    }


def test_view_to_dict():
    """to_dict() on a view returns only that subtree as plain data."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert doc["server"].to_dict() == {"host": "localhost", "ports": [80, 443]}


def test_document_range():
    """range() returns the document's 1-based source span."""
    doc = yamlrocks.loads(b"name: hello\nport: 8080\n", option=RT)
    start_line, start_col, end_line, end_col = doc.range()
    assert (start_line, start_col) == (1, 1)
    assert end_line == 2
    assert end_col >= 1


def test_view_range_is_for_the_subtree():
    """A view's range() spans just its nested node."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    start_line, start_col, end_line, end_col = doc["server"].range()
    assert start_line == 2
    assert start_col >= 1
    assert end_line >= start_line
    assert end_col >= 1


def test_quoted_scalar_end_carries_into_range():
    """range() reaches a trailing quoted value's true end (past the closing
    quote), not the unescaped value's length: the document ends where its
    furthest child does."""
    doc = yamlrocks.loads(b'a: 1\nlast: "xy"\n', option=RT)
    start_line, start_col, end_line, end_col = doc.range()
    assert (start_line, start_col) == (1, 1)
    # '"xy"' on line 2 occupies columns 7..10; the end is just past it, column 11
    # (not column 9, which the two-character value alone would give).
    assert (end_line, end_col) == (2, 11)


def test_view_len_counts_children():
    """len() on a view reports its container's child count."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert len(doc["server"]) == 2


def test_view_repr_shows_kind():
    """repr() of a view shows its node kind."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert repr(doc["server"]) == "YAMLRocksDocumentView(mapping)"


def test_view_contains():
    """Membership tests on a view reflect its mapping keys."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    view = doc["server"]
    assert "host" in view
    assert "missing" not in view


def test_view_get_returns_value_or_default():
    """get() on a view returns the value, or the default when absent."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    view = doc["server"]
    assert view.get("host", None) == "localhost"
    assert view.get("missing", "fallback") == "fallback"


def test_view_keys_lists_mapping_keys():
    """keys() on a view lists its mapping keys in order."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert list(doc["server"].keys()) == ["host", "ports"]


def test_view_to_yaml_emits_subtree():
    """to_yaml() on a view serializes only that nested subtree."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert doc["server"].to_yaml() == b"host: localhost\nports:\n  - 80\n  - 443\n"


def test_view_node_bridges_to_node():
    """A view's node property returns a YAMLRocksNode at the same path."""
    doc = yamlrocks.loads(SOURCE, option=RT)
    assert repr(doc["server"].node) == "YAMLRocksNode(mapping)"
