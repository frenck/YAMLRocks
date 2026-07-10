"""The ``dumps(represent=...)`` emitter protocol (ADR-021).

A ``represent`` callback describes how a host's own Python objects emit, via the
``YAMLRocksScalar``/``YAMLRocksSequence``/``YAMLRocksMapping`` node descriptors,
returning ``None`` to defer to the built-in rendering. These tests pin the
behavior ESPHome's dumper depends on and the byte-for-byte parity of deferred
values with a plain ``dumps``.
"""

from __future__ import annotations

import datetime

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


def test_deferred_tagged_null_keeps_tag_without_null_token():
    """A deferred tagged null keeps its tag as a bare ``!x`` in both mapping and
    sequence position, matching plain ``dumps`` byte-for-byte. Expanding it to
    ``!x null`` would reload as the string ``"null"`` instead of an empty value,
    and dropping the tag would lose it entirely."""
    tag = yamlrocks.YAMLRocksTag("!x", None)
    for doc in ({"k": tag}, [tag], {"a": tag, "b": yamlrocks.YAMLRocksTag("!y", None)}):
        deferred = yamlrocks.dumps(doc, represent=lambda _: None)
        plain = yamlrocks.dumps(doc)
        assert deferred == plain
        assert yamlrocks.loads(deferred) == yamlrocks.loads(plain)
    assert yamlrocks.dumps({"k": tag}, represent=lambda _: None) == b"k: !x\n"
    assert yamlrocks.dumps([tag], represent=lambda _: None) == b"- !x\n"


def test_deferred_non_first_tagged_key_uses_explicit_form():
    """A non-first tagged key is emitted in the explicit ``? key`` form, matching
    plain ``dumps``. An inline ``!tag key:`` after a previous entry binds its tag
    to that entry's value (a reparse error after an empty value), so the ``?``
    indicator opens a fresh key instead. A first/only tagged key stays inline."""
    key = yamlrocks.YAMLRocksTag("!foo", "k")
    for doc in ({"a": None, key: 2}, {"a": 1, key: 2}, {key: 1}):
        deferred = yamlrocks.dumps(doc, represent=lambda _: None)
        plain = yamlrocks.dumps(doc)
        assert deferred == plain
        assert yamlrocks.loads(deferred) == yamlrocks.loads(plain)
    assert yamlrocks.dumps({"a": None, key: 2}, represent=lambda _: None) == (
        b"a:\n? !foo k\n: 2\n"
    )
    assert yamlrocks.dumps({key: 1}, represent=lambda _: None) == b"!foo k: 1\n"


def test_deeply_nested_represent_tree_tears_down_without_overflow():
    """A deeply nested synthetic tree emits and is dismantled iteratively, so its
    recursive drop cannot overflow the native stack (regression for the AST being
    freed by the derived recursive ``Drop`` on return)."""
    doc: dict = {}
    cursor = doc
    for _ in range(900):
        child: dict = {}
        cursor["k"] = child
        cursor = child
    cursor["k"] = "leaf"
    out = yamlrocks.dumps(doc, represent=lambda _: None)
    assert yamlrocks.loads(out) is not None


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
        # Types the deferred path routes through the full encode pipeline.
        {"dt": datetime.datetime(2020, 1, 2, 3, 4, 5), "d": datetime.date(2021, 6, 7)},
        {"nums": [1.5, float("inf"), float("-inf"), -0.0], "big": 10**30},
    ],
)
def test_deferred_values_match_plain_dumps(doc):
    """When ``represent`` defers on everything, the output is byte-for-byte a plain
    ``dumps`` (the two emitters agree on deferred content, through the full encode
    pipeline including datetime and numeric formatting)."""
    assert yamlrocks.dumps(doc, represent=lambda v: None) == yamlrocks.dumps(doc)


def test_deferred_values_compose_with_default():
    """A value ``represent`` defers on still reaches the `default` callback, so
    `represent` and `default` compose rather than the former shadowing the latter."""

    class Money:
        pass

    out = yamlrocks.dumps(
        {"total": Money()},
        default=lambda o: "42 EUR" if isinstance(o, Money) else o,
        represent=lambda v: None,
    )
    assert out == b"total: 42 EUR\n"


def test_deferred_values_compose_with_serializers():
    """A deferred value still reaches the `serializers` registry, emitting its
    custom tag."""

    class Input:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"pin": Input("gpio")},
        serializers={Input: lambda o: yamlrocks.YAMLRocksTag("!input", o.name)},
        represent=lambda v: None,
    )
    assert out == b"pin: !input gpio\n"


def test_sort_keys_orders_numeric_keys_like_plain_dumps():
    """`sort_keys` on the represent path orders keys by type and value (numbers
    numerically), matching plain `dumps`, not lexically by text."""
    doc = {10: "a", 2: "b", "z": "s"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


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


def test_every_value_reaches_represent_through_compounds():
    """Descendants of a deferred compound (dataclass, set) still reach
    ``represent``, so a callback that restyles nested values is not silently
    skipped inside those shapes."""
    from dataclasses import dataclass

    @dataclass
    class Point:
        x: int
        y: int

    seen: list[object] = []

    def rep(value):
        seen.append(value)
        return None

    yamlrocks.dumps({"p": Point(1, 2), "s": {7}}, represent=rep)
    assert 1 in seen and 2 in seen  # dataclass field values
    assert 7 in seen  # set element


@pytest.mark.parametrize(
    "doc_factory",
    [
        lambda: {"s": {1, 2, 3}},
        lambda: {"fs": frozenset({1})},
    ],
)
def test_deferred_compounds_match_plain_dumps(doc_factory):
    """Deferred sets/frozensets render byte-for-byte like a plain ``dumps``."""
    doc = doc_factory()
    assert yamlrocks.dumps(doc, represent=lambda v: None) == yamlrocks.dumps(doc)


def test_shared_custom_object_aliases():
    """A custom object the host represents as a mapping is deduped with an anchor
    and alias when it appears more than once, not duplicated."""

    class Box:
        def __init__(self, value):
            self.value = value

    box = Box(1)

    def rep(value):
        if isinstance(value, Box):
            return yamlrocks.YAMLRocksMapping([("value", value.value)])
        return None

    out = yamlrocks.dumps({"a": box, "b": box}, represent=rep)
    assert out == b"a: &id001\n  value: 1\nb: *id001\n"


def test_cycle_through_custom_object_resolves_to_alias():
    """A reference cycle through a custom (represented) object resolves to an
    alias instead of hitting the depth limit."""

    class Node:
        def __init__(self):
            self.next = None

    node = Node()
    node.next = node

    def rep(value):
        if isinstance(value, Node):
            pairs = [("next", value.next)] if value.next is not None else []
            return yamlrocks.YAMLRocksMapping(pairs)
        return None

    assert yamlrocks.dumps(node, represent=rep) == b"&id001\nnext: *id001\n"


def test_flow_sequence_downgrades_block_scalar_child():
    """A block scalar inside a flow collection is invalid YAML, so a
    ``style="literal"`` child of a ``flow=True`` sequence is downgraded to a
    quoted scalar rather than emitted as a block."""

    def rep(value):
        if isinstance(value, list):
            return yamlrocks.YAMLRocksSequence(value, flow=True)
        if value == "x":
            return yamlrocks.YAMLRocksScalar(value, style="literal")
        return None

    out = yamlrocks.dumps(["x"], represent=rep)
    assert out == b'["x"]\n'
    # And it reloads to the same value.
    assert yamlrocks.loads(out) == ["x"]


def test_descriptor_tag_is_validated():
    """A descriptor tag is checked with the emit-side tag rules, so a malformed
    tag raises rather than corrupting the output."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(
            {"k": "v"},
            represent=lambda v: (
                yamlrocks.YAMLRocksScalar(v, tag="bad tag") if v == "v" else None
            ),
        )


@pytest.mark.parametrize(
    "option",
    [
        None,
        "OPT_EXPLICIT_START",
        "OPT_EXPLICIT_END",
        "OPT_FLOW_STYLE",
        "OPT_SORT_KEYS",
    ],
)
def test_emit_options_compose_with_represent(option):
    """Deferring on everything under an emit option matches a plain `dumps` with
    that option: `represent` composes with the emit-shaping flags."""
    doc = {"b": [1, 2], "a": 3}
    opt = 0 if option is None else getattr(yamlrocks, option)
    assert yamlrocks.dumps(
        doc, option=opt, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=opt)


def test_shared_nested_sequence_emits_anchor():
    """A shared list nested inside a sequence emits its anchor on the dash line,
    so the later alias has a definition (previously the anchor was dropped)."""
    shared = [1]
    out = yamlrocks.dumps([shared, shared], represent=lambda v: None)
    assert out == b"- &id001\n  - 1\n- *id001\n"
    # The anchor/alias pair reloads to a shared list.
    back = yamlrocks.loads(out)
    assert back == [[1], [1]]


def test_default_only_catches_unserializable_type_not_encode_errors():
    """`default` is a fallback for an unrecognized type only; a genuine encode
    error (non-UTF-8 bytes) propagates instead of being masked, matching plain
    `dumps`."""
    doc = {"b": b"\xff\xfe"}
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc, default=lambda o: "x", represent=lambda v: None)
    # Same error as a plain dump.
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc)


def test_non_progressing_default_raises_cleanly():
    """A `default` that returns its argument is a value defined only in terms of
    itself: it raises cleanly rather than recursing into a native stack overflow
    or emitting a malformed anchored alias."""

    class Unserializable:
        pass

    with pytest.raises(ValueError, match="refers only to itself"):
        yamlrocks.dumps(Unserializable(), default=lambda o: o, represent=lambda v: None)


def test_canonical_tags_are_normalized():
    """A canonical `tag:yaml.org,2002:*` tag (as PyYAML representers use) is
    accepted and gets the same shorthand handling as `!!*`: an implicit bool
    elides, and a str tag on a number quotes to stay a string."""

    def rep(v):
        if v == "b":
            return yamlrocks.YAMLRocksScalar("true", tag="tag:yaml.org,2002:bool")
        if v == "s":
            return yamlrocks.YAMLRocksScalar("123", tag="tag:yaml.org,2002:str")
        return None

    assert yamlrocks.dumps({"k": "b"}, represent=rep) == b"k: true\n"
    assert yamlrocks.dumps({"k": "s"}, represent=rep) == b"k: '123'\n"


def test_self_referential_serializer_raises():
    """A serializer that tags its own input (a non-progressing result) raises
    rather than recursing without bound."""

    class Thing:
        pass

    with pytest.raises(ValueError):
        yamlrocks.dumps(
            {"k": Thing()},
            serializers={Thing: lambda o: yamlrocks.YAMLRocksTag("!x", o)},
            represent=lambda v: None,
        )


def test_default_result_shared_elsewhere_anchors_correctly():
    """When `default` returns an object that is also present directly, the shared
    object keeps its own anchor (a transparent delegation must not overwrite it),
    so the alias has a definition and the output reloads."""

    class Custom:
        pass

    shared = [1]
    out = yamlrocks.dumps(
        [Custom(), shared], default=lambda o: shared, represent=lambda v: None
    )
    assert out == b"- &id001\n  - 1\n- *id001\n"
    assert yamlrocks.loads(out) == [[1], [1]]


def test_shared_mapping_as_sequence_item_anchors_correctly():
    """A shared mapping nested as a sequence item carries its anchor on the dash
    line (`- &id001` then the indented keys), not inline where it would bind to
    the first key, so it reloads to two equal mappings."""
    shared = {"x": 1}
    out = yamlrocks.dumps([shared, shared], represent=lambda v: None)
    assert out == b"- &id001\n  x: 1\n- *id001\n"
    assert yamlrocks.loads(out) == [{"x": 1}, {"x": 1}]


def test_chained_default_raises_like_plain_dumps():
    """A `default` whose result is itself unsupported raises, matching plain
    `dumps`: `default` is not re-invoked on its own result."""

    class A:
        pass

    class B:
        pass

    default = lambda o: B() if isinstance(o, A) else o  # noqa: E731
    with pytest.raises(yamlrocks.YAMLRocksUnserializableError):
        yamlrocks.dumps(A(), default=default, represent=lambda v: None)
    # Same as a plain dump.
    with pytest.raises(yamlrocks.YAMLRocksUnserializableError):
        yamlrocks.dumps(A(), default=default)


@pytest.mark.parametrize(
    "bad",
    [
        [("k", "val", "extra")],  # wrong arity
        [["a", 1]],  # a list, not a tuple
        ["ab"],  # a 2-char string is a 2-item sequence, but not a pair
    ],
)
def test_mapping_descriptor_pair_must_be_a_two_tuple(bad):
    """A `YAMLRocksMapping` entry must be a 2-item `(key, value)` tuple; a longer
    tuple, a list, or a 2-character string is rejected rather than silently
    misread or dropping items."""
    with pytest.raises(ValueError, match="key, value"):
        yamlrocks.dumps(
            "m",
            represent=lambda v: yamlrocks.YAMLRocksMapping(bad) if v == "m" else None,
        )


def test_default_returning_container_with_original_raises():
    """A `default` that returns a container referencing the original object has no
    node to anchor the back-reference to, so it raises rather than emitting an
    orphan alias (a plain `dumps` raises here too)."""

    class C:
        pass

    with pytest.raises(ValueError):
        yamlrocks.dumps(C(), default=lambda o: {"self": o}, represent=lambda v: None)


def test_primitive_subclass_serializer_matches_plain_dumps():
    """A `str`/`int` subclass registered in `serializers` is emitted as its
    builtin (the serializer is not consulted), matching the fast path's dispatch
    order, so a fully-deferred callback stays byte-identical."""

    class MyStr(str):
        pass

    serializers = {MyStr: lambda o: yamlrocks.YAMLRocksTag("!s", str(o))}
    doc = {"k": MyStr("hi")}
    assert yamlrocks.dumps(
        doc, serializers=serializers, represent=lambda v: None
    ) == yamlrocks.dumps(doc, serializers=serializers)


def test_single_newline_string_keeps_chomping():
    """A one-character `"\\n"` value keeps its trailing newline via a `|+` block,
    matching plain `dumps`, rather than a clip `|` that would reload as empty."""
    doc = {"k": "\n"}
    out = yamlrocks.dumps(doc, represent=lambda v: None)
    assert out == yamlrocks.dumps(doc)
    assert yamlrocks.loads(out) == {"k": "\n"}


def test_control_char_scalar_double_quotes_not_single():
    """A value with a control character cannot be single-quoted (single quotes
    escape nothing), so both a custom-tagged auto scalar and a deferred value
    fall back to double quotes and reload correctly, matching plain `dumps`."""
    tagged = yamlrocks.dumps(
        {"k": "v"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("a\x00b", tag="!x") if v == "v" else None
        ),
    )
    assert tagged == b'k: !x "a\\0b"\n'
    assert yamlrocks.loads(tagged) == {"k": "a\x00b"}

    doc = {"k": "a\tb\x00c"}
    for option in (0, yamlrocks.OPT_SINGLE_QUOTES):
        out = yamlrocks.dumps(doc, option=option, represent=lambda v: None)
        assert out == yamlrocks.dumps(doc, option=option)
        assert yamlrocks.loads(out) == doc


def test_integers_beyond_i64_tie_like_plain_dumps():
    """Integer keys past `i64` compare as `f64` (as the fast path treats a
    BigInt), so two that round to the same float keep insertion order, matching
    plain `dumps` rather than reordering."""
    doc = {10**20: "a", 10**20 + 1: "b"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_integer_keys_past_f64_range_still_sort_numeric():
    """An integer key past `f64`'s range keeps the numeric rank (ahead of string
    keys) instead of overflowing to the 'other' rank, and two that saturate to
    the same infinity keep insertion order, matching plain `dumps`."""
    doc = {10**400: "a", "z": "b", 10**400 + 1: "c"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_large_integer_keys_sort_exactly():
    """Integer keys keep their exact value when sorting, so two large `i64`s do
    not collide (as they would if coerced to `f64`), matching plain `dumps`."""
    doc = {9007199254740993: "a", 9007199254740992: "b"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)
