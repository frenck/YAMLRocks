"""Comment-bearing descriptors for freshly constructed Python output."""

import pytest

import yamlrocks


def represent(value):
    """Let descriptors pass through and ordinary values use native defaults."""
    if isinstance(
        value,
        (
            yamlrocks.YAMLRocksScalar,
            yamlrocks.YAMLRocksSequence,
            yamlrocks.YAMLRocksMapping,
        ),
    ):
        return value
    return None


def test_scalar_style_tag_and_comments():
    """All scalar presentation survives without reparsing generated YAML."""
    scalar = yamlrocks.YAMLRocksScalar(
        "value",
        style="single",
        tag="!custom",
        comment="inline",
        comment_before="before",
    )
    output = yamlrocks.dumps({"key": scalar}, represent=represent)
    assert output == b"# before\nkey: !custom 'value' # inline\n"
    assert (scalar.comment, scalar.comment_before, scalar.comment_after) == (
        "inline",
        "before",
        None,
    )


def test_collection_comments():
    """Collection and sequence item comments use the existing emitter."""
    item = yamlrocks.YAMLRocksScalar(
        "text", style="double", comment="item", comment_before="item heading"
    )
    sequence = yamlrocks.YAMLRocksSequence(
        [item],
        comment="list",
        comment_before="list heading",
        comment_after="list footer",
    )
    output = yamlrocks.dumps({"items": sequence}, represent=represent)
    assert (
        output
        == b'# list heading\nitems: # list\n  # item heading\n  - "text" # item\n  # list footer\n'
    )
    assert yamlrocks.loads(output) == {"items": ["text"]}


def test_root_mapping_comments():
    """A fresh root mapping can carry document header and footer comments."""
    value = yamlrocks.YAMLRocksMapping(
        [("a", 1)], comment_before="header\nsecond line", comment_after="footer"
    )
    output = yamlrocks.dumps(value, represent=represent)
    assert output == b"# header\n# second line\na: 1\n# footer\n"


def test_multiple_outputs_do_not_consume_comments():
    """Descriptors can be reused for any number of jq-style output documents."""
    value = yamlrocks.YAMLRocksScalar("text", comment="note")
    assert yamlrocks.dumps(value, represent=represent) == yamlrocks.dumps(
        value, represent=represent
    )


def test_flow_collection_comments():
    """Comments inside flow containers are emitted on valid separate lines."""
    value = yamlrocks.YAMLRocksScalar(
        "text", comment="inline", comment_before="heading"
    )
    for descriptor, expected in (
        (yamlrocks.YAMLRocksSequence([value, "other"], flow=True), ["text", "other"]),
        (
            yamlrocks.YAMLRocksMapping([("key", value), ("other", 1)], flow=True),
            {"key": "text", "other": 1},
        ),
    ):
        output = yamlrocks.dumps(descriptor, represent=represent)
        assert b"# heading" in output and b"# inline" in output
        assert yamlrocks.loads(output) == expected


@pytest.mark.parametrize(
    "descriptor",
    [
        yamlrocks.YAMLRocksScalar,
        yamlrocks.YAMLRocksSequence,
        yamlrocks.YAMLRocksMapping,
    ],
)
def test_inline_comment_rejects_line_breaks(descriptor):
    """A line break in an inline comment is rejected when the descriptor is built, before it can inject content."""
    with pytest.raises(ValueError, match="cannot contain a line break"):
        descriptor(
            "v" if descriptor is yamlrocks.YAMLRocksScalar else [], comment="a\nb: 1"
        )


def test_multi_line_comment_normalizes_line_breaks():
    """Every YAML line break starts a new comment line, as with the node cursor."""
    scalar = yamlrocks.YAMLRocksScalar("v", comment_before="one\rtwo\r\nthree")
    assert scalar.comment_before == "one\ntwo\nthree"
    output = yamlrocks.dumps({"k": scalar}, represent=represent)
    assert output == b"# one\n# two\n# three\nk: v\n"


def test_comment_after_needs_a_place_to_render():
    """comment_after follows the cursor's rule: a block collection or the document root only."""
    after = yamlrocks.YAMLRocksScalar("v", comment_after="foot")
    with pytest.raises(ValueError, match="comment_after is only supported"):
        yamlrocks.dumps({"k": after, "z": 1}, represent=represent)
    assert yamlrocks.dumps(after, represent=represent) == b"v\n# foot\n"


def test_key_comments_must_keep_the_key_on_one_line():
    """A key may have a comment above it, but nothing that would spread it over several lines."""
    above = yamlrocks.YAMLRocksScalar("k", comment_before="above")
    output = yamlrocks.dumps(
        yamlrocks.YAMLRocksMapping([("a", 1), (above, 2)]), represent=represent
    )
    assert output == b"a: 1\n# above\nk: 2\n"

    inline_key = yamlrocks.YAMLRocksScalar("k", comment="on key")
    with pytest.raises(ValueError, match="mapping key cannot carry"):
        yamlrocks.dumps(
            yamlrocks.YAMLRocksMapping([(inline_key, 1)]), represent=represent
        )

    commented_item = yamlrocks.YAMLRocksScalar("a", comment="inside")
    flow_key = yamlrocks.YAMLRocksSequence([commented_item], flow=True)
    with pytest.raises(ValueError, match="inside a mapping key"):
        yamlrocks.dumps(
            yamlrocks.YAMLRocksMapping([(flow_key, 1)]), represent=represent
        )


def test_width_never_folds_a_key_in_a_commented_flow_mapping():
    """A flow mapping laid out one entry per line for its comments still keeps every key on one line under width."""
    yaml = pytest.importorskip("yaml")
    value = yamlrocks.YAMLRocksScalar("v", comment_before="above")
    mapping = yamlrocks.YAMLRocksMapping([("key with several words", value)], flow=True)
    output = yamlrocks.dumps(mapping, represent=represent, width=8)
    assert b"key with several words" in output
    assert yamlrocks.loads(output) == {"key with several words": "v"}
    assert yaml.safe_load(output) == {"key with several words": "v"}
