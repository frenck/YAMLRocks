"""Round-trip edge cases, inspired by ruamel.yaml's round-trip test suite.

ruamel.yaml is the reference for comment- and formatting-preserving round-trips.
These cases stress the tricky scenarios its suite covers - comments in unusual
positions, blank lines, anchors and merge keys, flow collections, multiple
documents, and every scalar style - and assert yamlrocks reproduces them
**byte-for-byte** when the document is not modified.
"""

from __future__ import annotations

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def rt(src: str) -> str:
    """Round-trip a string and return the emitted text."""
    return yamlrocks.loads(src.encode(), option=RT).to_yaml().decode()


def assert_identical(src: str) -> None:
    """Assert an unmodified round-trip is byte-for-byte identical."""
    assert yamlrocks.loads(src.encode(), option=RT).to_yaml() == src.encode()


# -- Comments in many positions ----------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "# only a comment\n",
        "# leading\nkey: value\n",
        "key: value  # inline\n",
        "key: value\n# trailing\n",
        "a: 1\n# between a and b\nb: 2\n",
        "# c1\n# c2\n# c3\nkey: value\n",
        "list:\n  - a # first\n  - b # second\n",
        "map:\n  # about x\n  x: 1\n  # about y\n  y: 2\n",
        "outer:\n  inner:\n    # deep comment\n    deepest: 1\n",
        "key: value\n\n\n# comment after blank lines\n",
    ],
)
def test_comment_positions_byte_identical(src):
    """Comments in assorted positions survive a round-trip exactly."""
    assert_identical(src)


# -- Blank line preservation -------------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "a: 1\n\nb: 2\n",
        "a: 1\n\n\n\nb: 2\n",
        "first:\n  x: 1\n\n  y: 2\n",
        "\n\nkey: value\n",
        "section1:\n  a: 1\n\nsection2:\n  b: 2\n",
    ],
)
def test_blank_lines_byte_identical(src):
    """Blank lines between entries are preserved exactly."""
    assert_identical(src)


# -- Anchors, aliases, merge keys --------------------------------------------


def test_anchor_names_preserved():
    """Anchor and alias names are kept verbatim on round-trip."""
    src = "defaults: &defaults\n  a: 1\nprod:\n  <<: *defaults\n  b: 2\n"
    assert_identical(src)


def test_merge_key_round_trips_source():
    """A merge key is preserved literally in round-trip mode."""
    src = "base: &b\n  x: 1\nderived:\n  <<: *b\n  y: 2\n"
    out = rt(src)
    assert "<<: *b" in out
    assert out == src


# -- Flow collections --------------------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "flow: {a: 1, b: 2}\n",
        "list: [1, 2, 3]\n",
        "nested: {x: [1, 2], y: {z: 3}}\n",
        "spaced: {a: 1,  b: 2,   c: 3}\n",
        "empty_map: {}\nempty_list: []\n",
        "flow_comment: [1, 2]  # a list\n",
    ],
)
def test_flow_collections_byte_identical(src):
    """Flow-style collections keep their exact spacing and layout."""
    assert_identical(src)


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        ("[a: 1, b: 2]", [{"a": 1}, {"b": 2}]),
        ("[a: 1]", [{"a": 1}]),
        ("[plain, a: 1, last]", ["plain", {"a": 1}, "last"]),
        ("{a: [x: 1, y: 2]}", {"a": [{"x": 1}, {"y": 2}]}),
        ("[? a : b]", [{"a": "b"}]),
    ],
)
def test_flow_implicit_pairs_preserve_structure(src, expected):
    """A flow single-pair mapping (`[a: 1]` is `[{a: 1}]`) survives round-trip.

    The composer used to drop these entirely (the bare `Key` event looked like a
    terminator), corrupting the annotated and round-trip trees into empty or flat
    lists while the source cache hid it from `to_yaml`.
    """
    assert yamlrocks.loads(src) == expected
    assert yamlrocks.loads(src, option=RT).to_dict() == expected
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED) == expected
    assert_identical(src + "\n")


# -- Scalar styles -----------------------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "single: 'quoted'\n",
        'double: "quoted"\n',
        "plain: unquoted\n",
        "literal: |\n  line one\n  line two\n",
        "folded: >\n  folded\n  text\n",
        "literal_strip: |-\n  no trailing newline\n",
        "literal_keep: |+\n  keep\n\n\n",
        'quoted_special: "a: b # c"\n',
        "single_with_quote: 'it''s'\n",
    ],
)
def test_scalar_styles_byte_identical(src):
    """Every scalar style is reproduced exactly on round-trip."""
    assert_identical(src)


# -- Multiple documents ------------------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "---\na: 1\n---\nb: 2\n",
        "# doc 1\n---\na: 1\n# doc 2\n---\nb: 2\n",
        "---\nfirst: 1\n...\n---\nsecond: 2\n",
    ],
)
def test_multi_document_byte_identical(src):
    """Multi-document streams (and their markers) round-trip exactly."""
    assert_identical(src)


# -- Indentation variations --------------------------------------------------


@pytest.mark.parametrize(
    "src",
    [
        "a:\n    b:\n        c: 1\n",  # 4-space indent
        "list:\n- a\n- b\n",  # sequence at key indent
        "list:\n  - a\n  - b\n",  # sequence indented
        "mixed:\n  - name: x\n    value: 1\n  - name: y\n    value: 2\n",
    ],
)
def test_indentation_byte_identical(src):
    """The document's own indentation is preserved, not normalized."""
    assert_identical(src)


# -- Editing keeps the rest intact -------------------------------------------


def test_edit_one_value_keeps_comments_and_layout():
    """Editing a single value preserves surrounding comments and structure."""
    src = (
        "# Application configuration\n"
        "name: my-app  # the app name\n"
        "\n"
        "database:\n"
        "  host: localhost  # db host\n"
        "  port: 5432\n"
    )
    doc = yamlrocks.loads(src.encode(), option=RT)
    doc["database"]["port"] = 5433
    out = doc.to_yaml().decode()
    assert "# Application configuration" in out
    assert "# the app name" in out
    assert "# db host" in out
    assert "port: 5433" in out


def test_edit_preserves_foot_comment():
    """Replacing a mapping value keeps a following (foot) comment."""
    doc = yamlrocks.loads(b"a: 1\n\n# foot\nb: 2\n", option=RT)
    doc["a"] = 99
    assert b"# foot" in yamlrocks.dumps(doc)


def test_edit_preserves_sequence_item_comment():
    """Replacing a sequence item keeps the comment attached to it."""
    doc = yamlrocks.loads(b"- one  # first\n- two\n", option=RT)
    doc[0] = "ONE"
    assert b"# first" in yamlrocks.dumps(doc)


def test_edit_preserves_inline_comment_spacing_and_padding():
    """Editing a value keeps the alignment around it: the gap after the key's
    colon and the run of spaces before the inline `#` are both carried over, so
    only the value itself changes."""
    doc = yamlrocks.loads(b"port:    8080    # the listen port\nname: app\n", option=RT)
    doc["port"] = 9090
    assert doc.to_yaml() == b"port:    9090    # the listen port\nname: app\n"


def test_set_comment_via_api_uses_single_space():
    """A comment written through the API has no original spacing to keep, so it
    emits with a single space before the `#` (the one normalization)."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    doc.node["a"].comment = "added"
    assert doc.to_yaml() == b"a: 1 # added\n"


# -- The round-trip composer rejects malformed block structure --------------
# Mis-indented input that puts a block collection in mapping-key position, or a
# bare key with no `:`, is invalid YAML. The fast decoder (and PyYAML) reject it;
# the composer must too, rather than silently composing a nonsense complex key.


@pytest.mark.parametrize(
    "src",
    [
        b"deps:\n  - a: 1\n  b: 2\n",  # `b:` dedents to the dash column
        b"x:\n  - 1\n  k: 2\n",
    ],
)
def test_round_trip_rejects_block_collection_as_key(src):
    """A block collection reaching mapping-key position is rejected in round-trip
    mode, matching the fast path."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="cannot be a mapping key"):
        yamlrocks.loads(src, option=RT)


def test_round_trip_rejects_missing_colon():
    """A bare scalar key with no `:` in a block mapping is rejected in round-trip
    mode, matching the fast path."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="expected ':'"):
        yamlrocks.loads(b"a: 1\nbare\n", option=RT)


@pytest.mark.parametrize("src", [b"? [a, b]\n: c\n", b"{a, b}\n", b"[a: b]\n"])
def test_round_trip_keeps_valid_complex_and_flow_keys(src):
    """The validity check must not reject legitimate explicit `?` complex keys or
    flow mappings with bare keys; those still compose and round-trip."""
    assert yamlrocks.loads(src, option=RT).to_yaml() == src
