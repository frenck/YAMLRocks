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
    # An *implicit* first document (no leading `---`) must clear its anchors too,
    # or its `&x` leaks into the next document instead of raising.
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads_all(b"a: &x 1\n---\nb: *x\n")
    # Within a single document, aliases still resolve normally.
    assert yamlrocks.loads_all(b"---\na: &x 1\nb: *x\n") == [{"a": 1, "b": 1}]
    assert yamlrocks.loads_all(b"a: &x 1\nb: *x\n") == [{"a": 1, "b": 1}]


def test_deleting_an_anchor_target_still_referenced_raises():
    """Deleting an anchor target an alias still references raises, leaving the doc intact."""
    import pytest

    doc = yamlrocks.loads(
        b"base: &b\n  x: 1\nref: *b\nother: 2\n", option=yamlrocks.OPT_ROUND_TRIP
    )
    with pytest.raises(ValueError, match="alias"):
        del doc["base"]
    assert doc.to_yaml() == b"base: &b\n  x: 1\nref: *b\nother: 2\n"
    assert yamlrocks.loads(doc.to_yaml())["ref"] == {"x": 1}


def test_deleting_the_alias_itself_is_allowed():
    """Removing the alias (not its anchor target) is safe and leaves valid YAML."""
    doc = yamlrocks.loads(
        b"base: &b\n  x: 1\nref: *b\nother: 2\n", option=yamlrocks.OPT_ROUND_TRIP
    )
    del doc["ref"]
    assert yamlrocks.loads(doc.to_yaml()) == {"base": {"x": 1}, "other": 2}


def test_deleting_a_nested_anchor_target_still_referenced_raises():
    """The guard covers a nested-view delete, not only the top-level path."""
    import pytest

    src = b"outer:\n  inner: &b 1\nref: *b\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    with pytest.raises(ValueError, match="alias"):
        del doc["outer"]["inner"]
    assert doc.to_yaml() == src
    assert yamlrocks.loads(doc.to_yaml())["ref"] == 1


def test_unknown_alias_is_rejected_on_every_load_path():
    """An `*alias` that names no anchor is an error on every path, not a silent
    null. The round-trip and includes paths run through the composer, which used
    to skip this check and quietly resolve an undefined (or forward) reference to
    `None`; they now reject it like the fast and annotated paths (and PyYAML)."""
    import pytest

    for opt in (
        0,
        yamlrocks.OPT_ANNOTATED,
        yamlrocks.OPT_ROUND_TRIP,
        yamlrocks.OPT_INCLUDES,
    ):
        with pytest.raises(yamlrocks.YAMLRocksError, match="unknown alias"):
            yamlrocks.loads(b"a: 1\nb: *ghost\n", option=opt)
    # A forward reference (`*x` before `&x`) is likewise rejected, and an anchor
    # does not leak across documents.
    with pytest.raises(yamlrocks.YAMLRocksError, match="unknown alias"):
        yamlrocks.loads(b"---\na: &x 1\n---\nb: *x\n", option=yamlrocks.OPT_ROUND_TRIP)


def test_malformed_anchor_or_tag_is_rejected_on_every_load_path():
    """An alias cannot carry its own anchor/tag, and a node cannot have two
    anchors. The composer paths enforce this like the fast decoder (and PyYAML),
    instead of silently accepting the malformed node."""
    import pytest

    for opt in (
        0,
        yamlrocks.OPT_ANNOTATED,
        yamlrocks.OPT_ROUND_TRIP,
        yamlrocks.OPT_INCLUDES,
    ):
        with pytest.raises(yamlrocks.YAMLRocksError, match="anchor or tag"):
            yamlrocks.loads(b"a: &a 1\nb: &b *a\n", option=opt)
        with pytest.raises(yamlrocks.YAMLRocksError, match="anchor or tag"):
            yamlrocks.loads(b"a: &a 1\nb: !!str *a\n", option=opt)
        with pytest.raises(yamlrocks.YAMLRocksError, match="two anchors"):
            yamlrocks.loads(b"a: &a\n  &b 1\n", option=opt)


def test_recursive_self_referential_alias_is_rejected():
    """A self-referential anchor (`&a` then `- *a`, a list containing itself) is
    rejected rather than building a cyclic structure. yamlrocks requires an anchor
    to be defined before it is referenced, which also bounds the
    infinite-structure/DoS surface; PyYAML/ruamel build the cycle, we do not."""
    import pytest

    with pytest.raises(yamlrocks.YAMLRocksError, match="alias"):
        yamlrocks.loads(b"&a\n- *a\n")
    # A mapping that references its own anchor as a value is rejected too.
    with pytest.raises(yamlrocks.YAMLRocksError, match="alias"):
        yamlrocks.loads(b"a: &x\n  b: *x\n")
