"""Anchors and aliases across the load paths."""

from __future__ import annotations

import yamlrocks

_ALIAS_DOC = "base: &b\n  name: app\n  port: 8080\nref: *b\n"


def test_scalar_alias_expands():
    """Expand an alias to an anchored scalar value."""
    assert yamlrocks.loads(b"a: &x 1\nb: *x") == {"a": 1, "b": 1}


def test_string_alias_expands():
    """Expand an alias to an anchored string value."""
    assert yamlrocks.loads(b"a: &name hello\nb: *name") == {"a": "hello", "b": "hello"}


def test_mapping_alias_expands():
    """Expand an alias to an anchored mapping."""
    src = b"base: &b\n  x: 1\n  y: 2\nderived: *b\n"
    result = yamlrocks.loads(src)
    assert result["base"] == {"x": 1, "y": 2}
    assert result["derived"] == {"x": 1, "y": 2}


def test_sequence_alias_expands():
    """Expand an alias to an anchored sequence."""
    src = b"base: &b\n  - 1\n  - 2\ncopy: *b\n"
    result = yamlrocks.loads(src)
    assert result["copy"] == [1, 2]


def test_round_trip_alias_access_resolves():
    """Reading an aliased value through a YAMLRocksDocument resolves it (not None)."""
    doc = yamlrocks.loads(_ALIAS_DOC, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc["ref"].to_dict() == {"name": "app", "port": 8080}
    assert doc.to_dict()["ref"] == {"name": "app", "port": 8080}


def test_round_trip_alias_emission_preserved():
    """Resolving aliases on read does not change *alias on emit."""
    src = b"base: &b\n  x: 1\nref: *b\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    assert yamlrocks.dumps(doc) == src


def test_annotated_alias_access_resolves():
    """Annotated mode resolves an alias to the anchored value."""
    data = yamlrocks.loads(_ALIAS_DOC, option=yamlrocks.OPT_ANNOTATED)
    assert data["ref"] == {"name": "app", "port": 8080}


def test_anchors_do_not_cross_documents():
    """An anchor defined in one document is not visible to the next.

    Per the YAML spec, anchors do not span documents, so an alias in document 2
    referencing an anchor from document 1 is an unknown alias, not a silent reuse.
    """
    import pytest

    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads_all(b"---\n&a x\n---\n*a\n")
    # The same cross-document reference also errors on the annotated path.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads_all(b"---\n&a x\n---\n*a\n", option=yamlrocks.OPT_ANNOTATED)
    # Within a single document, aliases still resolve normally.
    assert yamlrocks.loads_all(b"---\na: &x 1\nb: *x\n") == [{"a": 1, "b": 1}]
