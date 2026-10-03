"""Bulk snapshots for transformations that need values and presentation."""

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def test_snapshot_values_and_metadata():
    """The snapshot includes typed leaves, key comments, styles, and tags."""
    doc = yamlrocks.loads(
        b"# header\na: !custom 'yes' # inline\nb: [true, 42, null]\n", option=RT
    )
    tree = doc.to_tree()
    assert isinstance(tree, yamlrocks.YAMLRocksTreeNode)
    assert tree.kind == "mapping"
    (key, value), (_, sequence) = tree.value
    assert "header" in (tree.comment_before or key.comment_before or "")
    assert key.value == "a"
    assert (value.value, value.style, value.tag) == ("yes", "single", "!custom")
    assert "inline" in value.comment
    assert sequence.kind == "sequence"
    assert sequence.style == "flow"
    assert [child.value for child in sequence.value] == [True, 42, None]


def test_snapshot_retains_aliases_and_merge_keys():
    """A host can choose expansion without reverse engineering YAML syntax."""
    tree = yamlrocks.loads(
        b"base: &b {a: 1}\ncopy: *b\nmerged: {<<: *b}\n", option=RT
    ).to_tree()
    (_, base), (_, alias), (_, merged) = tree.value
    assert base.anchor == "b"
    assert (alias.kind, alias.value) == ("alias", "b")
    merge_key, merge_value = merged.value[0]
    assert merge_key.is_merge_key
    assert (merge_value.kind, merge_value.value) == ("alias", "b")
    literal = yamlrocks.loads(b"'<<': 1\n", option=RT).to_tree().value[0][0]
    assert not literal.is_merge_key


def test_snapshot_is_owned_and_immutable():
    """Editing or releasing the document cannot change an existing snapshot."""
    doc = yamlrocks.loads(b"a: 1\n", option=RT)
    tree = doc.to_tree()
    doc["a"] = 2
    del doc
    assert tree.value[0][1].value == 1
    with pytest.raises(AttributeError):
        tree.style = "flow"
    assert isinstance(tree.value, tuple)


def test_empty_and_multiple_documents():
    """No root is None; multiple roots are never silently truncated."""
    assert yamlrocks.loads(b"", option=RT).to_tree() is None
    with pytest.raises(ValueError, match="one document"):
        yamlrocks.loads(b"a: 1\n---\nb: 2\n", option=RT).to_tree()


@pytest.mark.parametrize(
    "option, expected", [(0, "yes"), (yamlrocks.OPT_YAML_1_1, True)]
)
def test_snapshot_uses_document_schema(option, expected):
    """Scalar resolution stays in YAMLRocks and follows the selected schema."""
    tree = yamlrocks.loads(b"yes", option=RT | option).to_tree()
    assert tree.value == expected
