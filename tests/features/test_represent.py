"""The ``dumps(represent=...)`` emitter protocol (ADR-021).

A ``represent`` callback describes how a host's own Python objects emit, via the
``YAMLRocksScalar``/``YAMLRocksSequence``/``YAMLRocksMapping`` node descriptors,
returning ``None`` to defer to the built-in rendering. These tests pin the
behavior ESPHome's dumper depends on and the byte-for-byte parity of deferred
values with a plain ``dumps``.
"""

from __future__ import annotations

import pytest

import yamlrocks


def test_scalar_with_custom_tag_forces_single_quotes():
    """A custom-tagged scalar under auto style is force-quoted, matching PyYAML:
    a plain form would reload without the tag."""
    out = yamlrocks.dumps(
        {"key": "my_id"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, tag="!extend") if v == "my_id" else None
        ),
    )
    assert out == b"key: !extend 'my_id'\n"


def test_scalar_explicit_literal_style():
    """An explicit ``style="literal"`` emits a ``|`` block, keeping the tag."""
    out = yamlrocks.dumps(
        {"lam": "return x + 1;"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, tag="!lambda", style="literal")
            if isinstance(v, str) and v.startswith("return")
            else None
        ),
    )
    assert out == b"lam: !lambda |-\n  return x + 1;\n"


def test_implicit_tag_is_elided():
    """A standard tag the plain value already resolves to (``!!bool`` on ``true``,
    ``!!float`` on a float literal) is dropped, rendering the bare value."""

    def rep(v):
        if v is True:
            return yamlrocks.YAMLRocksScalar("true", tag="!!bool")
        if isinstance(v, float):
            return yamlrocks.YAMLRocksScalar("1.0e17", tag="!!float")
        return None

    assert yamlrocks.dumps({"b": True}, represent=rep) == b"b: true\n"
    assert yamlrocks.dumps({"f": 1e17}, represent=rep) == b"f: 1.0e17\n"


def test_str_tag_on_number_looking_value_quotes_to_stay_string():
    """``!!str`` on a value that would plainly resolve to a number quotes it (so
    it stays a string) and drops the now-implied tag."""
    out = yamlrocks.dumps(
        {"s": "x"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("123", tag="!!str") if v == "x" else None
        ),
    )
    assert out == b"s: '123'\n"


def test_tagged_mapping_form():
    """A ``YAMLRocksMapping`` with a tag emits the tag before an indented block
    mapping (the ``!include {file, vars}`` form)."""
    out = yamlrocks.dumps(
        {"key": "inc"},
        represent=lambda v: (
            yamlrocks.YAMLRocksMapping(
                [("file", "f.yaml"), ("vars", {"k": "v"})], tag="!include"
            )
            if v == "inc"
            else None
        ),
    )
    assert out == b"key: !include\n  file: f.yaml\n  vars:\n    k: v\n"


def test_sequence_flow_override():
    """``flow=True`` emits a flow sequence."""
    out = yamlrocks.dumps(
        ["a", "b"],
        represent=lambda v: (
            yamlrocks.YAMLRocksSequence(v, flow=True) if isinstance(v, list) else None
        ),
    )
    assert out == b"[a, b]\n"


def test_children_are_re_dispatched_through_represent():
    """A descriptor's children are the original objects; the emitter re-dispatches
    each through ``represent`` (emitter-driven recursion)."""
    seen: list[object] = []

    def rep(v):
        seen.append(v)
        return None

    yamlrocks.dumps({"a": [1, {"b": 2}]}, represent=rep)
    # The mapping, its list value, the list items, and the nested mapping/values
    # all reach represent.
    assert {"a": [1, {"b": 2}]} in seen
    assert [1, {"b": 2}] in seen
    assert {"b": 2} in seen


def test_block_sequences_are_indented():
    """A deferred block sequence indents under its key (the PyYAML dump style),
    not flush."""
    out = yamlrocks.dumps({"a": 1, "b": ["x", "y"]}, represent=lambda v: None)
    assert out == b"a: 1\nb:\n  - x\n  - y\n"


def test_sort_keys_option_applies():
    """``OPT_SORT_KEYS`` sorts mapping keys on the represent path."""
    doc = {"b": 2, "a": 1, "c": 3}
    assert (
        yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None)
        == b"a: 1\nb: 2\nc: 3\n"
    )
    # Without the flag, insertion order is kept.
    assert yamlrocks.dumps(doc, represent=lambda v: None) == b"b: 2\na: 1\nc: 3\n"


def test_shared_object_emits_anchor_and_alias():
    """A Python object appearing more than once emits ``&id`` once and ``*id``
    after, so the shared reference survives a dump/reload."""
    shared = {"x": 1, "y": 2}
    out = yamlrocks.dumps({"a": shared, "b": shared}, represent=lambda v: None)
    assert out == b"a: &id001\n  x: 1\n  y: 2\nb: *id001\n"
    # And it reloads to the shared shape.
    back = yamlrocks.loads(out)
    assert back["a"] == back["b"] == {"x": 1, "y": 2}


def test_unshared_objects_get_no_anchors():
    """Distinct objects never carry an anchor, even with equal contents."""
    out = yamlrocks.dumps({"a": {"x": 1}, "b": {"x": 1}}, represent=lambda v: None)
    assert out == b"a:\n  x: 1\nb:\n  x: 1\n"


def test_cycle_resolves_to_an_alias():
    """A self-referential container resolves to an alias rather than looping."""
    d: dict[str, object] = {}
    d["self"] = d
    out = yamlrocks.dumps(d, represent=lambda v: None)
    assert out == b"&id001\nself: *id001\n"


def test_multiline_string_defaults_to_literal_block():
    """A deferred multi-line string defaults to a ``|`` literal block, matching a
    plain dump rather than a double-quoted scalar."""
    doc = {"multi": "line1\nline2\n"}
    assert yamlrocks.dumps(doc, represent=lambda v: None) == yamlrocks.dumps(doc)
    assert (
        yamlrocks.dumps(doc, represent=lambda v: None)
        == b"multi: |\n  line1\n  line2\n"
    )


@pytest.mark.parametrize(
    "doc",
    [
        {"a": 1, "b": ["x", "y"], "on": "yes", "n": None, "f": 1.5},
        {"nested": {"k": [1, 2, {"z": True}]}},
        {"s": "hello world", "q": "a: b", "empty": ""},
        [1, "two", None, 3.14, False],
        {"multi": "line1\nline2\n"},
    ],
)
def test_deferred_values_match_plain_dumps(doc):
    """When ``represent`` defers on everything, the output is byte-for-byte a plain
    ``dumps`` (the two emitters agree on deferred content)."""
    assert yamlrocks.dumps(doc, represent=lambda v: None) == yamlrocks.dumps(doc)


def test_invalid_style_raises():
    """An unknown scalar style is a ValueError."""
    with pytest.raises(ValueError, match="invalid scalar style"):
        yamlrocks.YAMLRocksScalar("x", style="fancy")


def test_bad_represent_return_raises():
    """A represent callback returning something other than a node or None errors."""
    with pytest.raises(TypeError, match="represent callback must return"):
        yamlrocks.dumps({"a": 1}, represent=lambda v: "not a node")


def test_represent_does_not_change_plain_dumps():
    """Omitting ``represent`` leaves ``dumps`` on its fast path, unchanged."""
    assert yamlrocks.dumps({"a": [1, 2], "b": "x"}) == b"a:\n  - 1\n  - 2\nb: x\n"
