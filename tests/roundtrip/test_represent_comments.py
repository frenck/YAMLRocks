"""Comment-bearing descriptors for freshly constructed Python output."""

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
