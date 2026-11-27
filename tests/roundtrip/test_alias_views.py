"""Views reached through an alias work on the anchored container."""

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

MAPPING = b"base: &x {v: 2, s: [1, 2]}\nref: *x\n"
SEQUENCE = b"base: &x [1, 2]\nref: *x\n"
CHAIN = b"a: &a {v: 1}\nb: &b {inner: *a}\nc: *b\n"


def load(source):
    """Load a round-trip document."""
    return yamlrocks.loads(source, option=RT)


def test_reads_through_an_alias_to_a_mapping():
    """Every read on a view of an aliased mapping sees the anchor's entries."""
    ref = load(MAPPING)["ref"]

    assert ref["v"] == 2
    assert ref["s"][1] == 2
    assert len(ref) == 2
    assert "v" in ref
    assert "missing" not in ref
    assert ref.keys() == ["v", "s"]
    assert ref.get("v", None) == 2
    assert ref.to_dict() == ref.unwrap() == {"v": 2, "s": [1, 2]}
    assert ref.walk() == [(("v",), 2), (("s", 0), 1), (("s", 1), 2)]
    assert repr(ref) == "YAMLRocksDocumentView(mapping)"


def test_reads_through_an_alias_to_a_sequence():
    """A view of an aliased sequence indexes, counts, and iterates the anchor's items."""
    ref = load(SEQUENCE)["ref"]

    assert ref[1] == 2
    assert len(ref) == 2
    assert list(ref) == [1, 2]


def test_reads_through_an_alias_inside_an_aliased_container():
    """An alias reached through another alias is followed too."""
    assert load(CHAIN)["c"]["inner"]["v"] == 1


def test_a_scalar_alias_still_reads_as_its_plain_value():
    """Indexing an alias to a scalar returns the plain value, not a view."""
    assert load(b"a: &s hi\nb: *s\n")["b"] == "hi"


def test_the_node_and_range_of_an_alias_view_are_the_alias():
    """An alias view's node cursor and range describe the alias, and indexing the cursor follows it."""
    ref = load(MAPPING)["ref"]

    assert ref.node.style == "alias"
    assert ref.node.is_alias
    assert ref.range() == (2, 6, 2, 6)
    assert ref.node["v"].value == 2


def test_to_yaml_of_an_alias_view_emits_the_anchored_node():
    """to_yaml on an alias view serializes the container the alias names."""
    assert load(MAPPING)["ref"].to_yaml() == b"&x {v: 2, s: [1, 2]}\n"


@pytest.mark.parametrize(
    ("edit", "expected"),
    [
        (
            lambda doc: doc["ref"].__setitem__("v", 3),
            b"base: &x {v: 3, s: [1, 2]}\nref: *x\n",
        ),
        (
            lambda doc: doc["ref"]["s"].__setitem__(0, 9),
            b"base: &x {v: 2, s: [9, 2]}\nref: *x\n",
        ),
        (lambda doc: doc["ref"].__delitem__("v"), b"base: &x {s: [1, 2]}\nref: *x\n"),
        (
            lambda doc: setattr(doc.node["ref"]["v"], "value", 3),
            b"base: &x {v: 3, s: [1, 2]}\nref: *x\n",
        ),
    ],
)
def test_a_write_through_an_alias_changes_the_anchor_once(edit, expected):
    """A write through an alias view changes the anchor's value and keeps the alias, as the cursor does."""
    doc = load(MAPPING)
    edit(doc)

    assert doc.to_yaml() == expected
    assert doc["base"].to_dict() == doc["ref"].to_dict()


def test_a_write_through_an_alias_chain_reaches_the_innermost_anchor():
    """A write two aliases deep changes the anchor the inner alias names."""
    doc = load(CHAIN)
    doc["c"]["inner"]["v"] = 5

    assert doc.to_yaml() == b"a: &a {v: 5}\nb: &b {inner: *a}\nc: *b\n"


def test_assigning_the_alias_itself_replaces_only_the_alias():
    """Assigning to the alias's own key replaces the alias and leaves the anchor alone."""
    doc = load(MAPPING)
    doc["ref"] = {"v": 7}

    assert doc.to_yaml() == b"base: &x {v: 2, s: [1, 2]}\nref:\n  v: 7\n"


def test_an_alias_view_resolves_its_anchor_in_its_own_document():
    """In a stream, an alias view reads and writes the anchor of its own document."""
    doc = load(b"base: &x {v: 1}\n---\nbase: &x {v: 2}\nref: *x\n")

    assert doc[1]["ref"]["v"] == 2
    doc[1]["ref"]["v"] = 3
    assert doc.to_yaml() == b"base: &x {v: 1}\n---\nbase: &x {v: 3}\nref: *x\n"


def test_ordinary_paths_are_unchanged():
    """Views of plain mappings and sequences address the same nodes as before."""
    doc = load(b"m: {a: [1, {b: 2}]}\n")

    assert doc["m"]["a"][1]["b"] == 2
    doc["m"]["a"][1]["b"] = 3
    assert doc.to_yaml() == b"m: {a: [1, {b: 3}]}\n"
