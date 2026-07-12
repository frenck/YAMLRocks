"""The ``dumps(represent=...)`` emitter protocol (ADR-021).

A ``represent`` callback describes how a host's own Python objects emit, via the
``YAMLRocksScalar``/``YAMLRocksSequence``/``YAMLRocksMapping`` node descriptors,
returning ``None`` to defer to the built-in rendering. These tests pin the
behavior ESPHome's dumper depends on and the byte-for-byte parity of deferred
values with a plain ``dumps``.
"""

from __future__ import annotations

import dataclasses
import datetime
import decimal
import enum

import pytest

import yamlrocks


def _deep_dict(depth: int) -> dict:
    """A ``depth``-level nested ``{"k": {"k": ...}}`` chain ending in a leaf."""
    doc: dict = {}
    cursor = doc
    for _ in range(depth):
        child: dict = {}
        cursor["k"] = child
        cursor = child
    cursor["k"] = "leaf"
    return doc


def test_scalar_with_custom_tag_forces_single_quotes():
    """A custom-tagged scalar under auto style is force-quoted."""
    out = yamlrocks.dumps(
        {"key": "my_id"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, tag="!extend") if v == "my_id" else None
        ),
    )
    assert out == b"key: !extend 'my_id'\n"


def test_scalar_explicit_literal_style():
    """An explicit ``style="literal"`` emits a ``|`` block."""
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
    """A standard tag the plain value already resolves to is dropped."""

    def rep(v):
        if v is True:
            return yamlrocks.YAMLRocksScalar("true", tag="!!bool")
        if isinstance(v, float):
            return yamlrocks.YAMLRocksScalar("1.0e17", tag="!!float")
        return None

    assert yamlrocks.dumps({"b": True}, represent=rep) == b"b: true\n"
    assert yamlrocks.dumps({"f": 1e17}, represent=rep) == b"f: 1.0e17\n"


def test_str_tag_on_number_looking_value_quotes_to_stay_string():
    """``!!str`` on a value that would plainly resolve to a number quotes it."""
    out = yamlrocks.dumps(
        {"s": "x"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("123", tag="!!str") if v == "x" else None
        ),
    )
    assert out == b"s: '123'\n"


def test_tagged_mapping_form():
    """A ``YAMLRocksMapping`` with a tag emits the tag before an indented block mapping."""
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
    """A deferred tagged null keeps its tag as a bare ``!x`` in both mapping and sequence position."""
    tag = yamlrocks.YAMLRocksTag("!x", None)
    for doc in ({"k": tag}, [tag], {"a": tag, "b": yamlrocks.YAMLRocksTag("!y", None)}):
        deferred = yamlrocks.dumps(doc, represent=lambda _: None)
        plain = yamlrocks.dumps(doc)
        assert deferred == plain
        assert yamlrocks.loads(deferred) == yamlrocks.loads(plain)
    assert yamlrocks.dumps({"k": tag}, represent=lambda _: None) == b"k: !x\n"
    assert yamlrocks.dumps([tag], represent=lambda _: None) == b"- !x\n"


def test_shared_tagged_empty_value_keeps_tag_and_anchor_losslessly():
    """A shared tagged empty value anchors the first occurrence and aliases the rest."""
    tag = yamlrocks.YAMLRocksTag("!x", None)
    mapping = yamlrocks.dumps({"a": tag, "b": tag}, represent=lambda _: None)
    assert mapping == b"a: !x &id001\nb: *id001\n"
    assert yamlrocks.loads(mapping) == {"a": "", "b": ""}
    sequence = yamlrocks.dumps([tag, tag], represent=lambda _: None)
    assert sequence == b"- !x &id001\n- *id001\n"
    assert yamlrocks.loads(sequence) == ["", ""]


def test_key_that_lowers_to_a_collection_matches_plain_dumps():
    """A key that only decomposes into a collection (a ``default`` result) emits inline in flow, matching plain dumps."""

    class Key:
        pass

    key = Key()
    default = lambda o: {"x": {"y": 1}}  # noqa: E731
    deferred = yamlrocks.dumps({key: 1}, default=default, represent=lambda _: None)
    assert deferred == yamlrocks.dumps({key: 1}, default=default)
    assert deferred == b"{x: {y: 1}}: 1\n"


def test_indent_4_applies_to_an_explicit_block_collection_key():
    """A descriptor key with ``flow=False`` opts into the explicit ``?`` form, and `OPT_INDENT_4` indents it."""

    class Key:
        pass

    key = Key()
    out = yamlrocks.dumps(
        {key: 1},
        option=yamlrocks.OPT_INDENT_4,
        represent=lambda v: (
            yamlrocks.YAMLRocksMapping([("x", {"y": 1})], flow=False)
            if v is key
            else None
        ),
    )
    assert out == b"?\n    x:\n        y: 1\n: 1\n"
    # A mapping key reloads as its hashable tuple form.
    assert yamlrocks.loads(out) == {(("x", (("y", 1),)),): 1}


def test_deferred_non_first_tagged_key_uses_explicit_form():
    """A non-first tagged key is emitted in the explicit ``? key`` form."""
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
    """A deeply nested synthetic tree emits and is dismantled iteratively."""
    doc = _deep_dict(500)
    out = yamlrocks.dumps(doc, represent=lambda _: None)
    assert yamlrocks.loads(out) is not None


def test_represent_deep_nesting_raises_cleanly():
    """Nesting beyond the represent path's stack headroom raises instead of aborting the interpreter."""
    # The lowering re-enters Python per level, so it cannot ride the segmented
    # stack the pure-Rust descents use (CPython's C-stack check would abort);
    # past its headroom it must raise the clean depth error.
    deep: list = []
    cursor = deep
    for _ in range(1500):
        child: list = []
        cursor.append(child)
        cursor = child
    with pytest.raises(ValueError, match="too deeply nested"):
        yamlrocks.dumps(deep, represent=lambda _: None)


def test_mapping_error_after_deep_value_tears_down_iteratively():
    """A mapping lowering error dismantles the already-built pairs iteratively."""
    # `object()` has no scalar rendering and no `default`, so lowering "bad" fails
    # after "deep" is already built.
    doc = {"deep": _deep_dict(300), "bad": object()}
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc, represent=lambda _: None)


def test_sequence_error_after_deep_item_tears_down_iteratively():
    """The sequence path likewise dismantles already-built items iteratively when a later item fails to lower."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps([_deep_dict(300), object()], represent=lambda _: None)


def test_explicit_block_style_rejects_lossy_values():
    """An explicit literal/folded style that cannot round-trip the value is rejected."""
    scalar = yamlrocks.YAMLRocksScalar
    cases = [
        ("a\nb", "folded"),
        ("  indented\nrest", "literal"),
        ("a\rb", "literal"),
        ("a\x00b", "folded"),
    ]
    for value, style in cases:
        with pytest.raises(ValueError, match="cannot represent this value"):
            yamlrocks.dumps(
                {"k": value},
                represent=lambda v, _val=value, _st=style: (
                    scalar(_val, style=_st) if v == _val else None
                ),
            )
    # A lossless literal (single or multi-line, no leading whitespace) is fine and
    # reloads unchanged.
    out = yamlrocks.dumps(
        {"k": "line1\nline2"},
        represent=lambda v: scalar(v, style="literal") if v == "line1\nline2" else None,
    )
    assert out == b"k: |-\n  line1\n  line2\n"
    assert yamlrocks.loads(out) == {"k": "line1\nline2"}
    # An explicit double-quoted style is never rejected: it escapes a carriage
    # return and control characters, so it round-trips losslessly.
    for value in ("a\rb", "a\x00b"):
        quoted = yamlrocks.dumps(
            {"k": value},
            represent=lambda v, _val=value: (
                scalar(_val, style="double") if v == _val else None
            ),
        )
        assert yamlrocks.loads(quoted) == {"k": value}


def test_default_self_reference_after_deep_subtree_raises_cleanly():
    """A ``default`` result that both nests deeply and refers back to the original object is unrepresentable."""

    class Box:
        pass

    box = Box()

    def default(obj):
        return {"deep": _deep_dict(300), "self": obj}

    with pytest.raises(ValueError, match="refers only to itself"):
        yamlrocks.dumps(box, default=default, represent=lambda _: None)


def test_dump_forwards_represent_to_a_stream():
    """The file-oriented ``dump`` forwards ``represent`` to ``dumps``."""
    import io

    buffer = io.BytesIO()
    yamlrocks.dump(
        {"key": "my_id"},
        buffer,
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, tag="!extend") if v == "my_id" else None
        ),
    )
    assert buffer.getvalue() == b"key: !extend 'my_id'\n"


def test_async_dump_forwards_represent_to_a_stream():
    """The async file wrapper ``async_dump`` forwards ``represent`` to ``dump``."""
    import asyncio
    import io

    async def run() -> bytes:
        buffer = io.BytesIO()
        await yamlrocks.async_dump(
            {"key": "my_id"},
            buffer,
            represent=lambda v: (
                yamlrocks.YAMLRocksScalar(v, tag="!extend") if v == "my_id" else None
            ),
        )
        return buffer.getvalue()

    assert asyncio.run(run()) == b"key: !extend 'my_id'\n"


def test_empty_tuple_is_not_aliased():
    """An empty tuple is the CPython `()` singleton."""
    out = yamlrocks.dumps([(), ()], represent=lambda _: None)
    assert out == yamlrocks.dumps([(), ()]) == b"- []\n- []\n"


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
    """A descriptor's children are the original objects; the emitter re-dispatches each through ``represent``."""
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
    """A deferred block sequence indents under its key."""
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
    """A Python object appearing more than once emits ``&id`` once and ``*id`` after."""
    shared = {"x": 1, "y": 2}
    out = yamlrocks.dumps({"a": shared, "b": shared}, represent=lambda v: None)
    assert out == b"a: &id001\n  x: 1\n  y: 2\nb: *id001\n"
    # And it reloads to the shared shape.
    back = yamlrocks.loads(out)
    assert back["a"] == back["b"] == {"x": 1, "y": 2}


def test_unshared_objects_get_no_anchors():
    """Distinct objects never carry an anchor."""
    out = yamlrocks.dumps({"a": {"x": 1}, "b": {"x": 1}}, represent=lambda v: None)
    assert out == b"a:\n  x: 1\nb:\n  x: 1\n"


def test_cycle_resolves_to_an_alias():
    """A self-referential container resolves to an alias rather than looping."""
    d: dict[str, object] = {}
    d["self"] = d
    out = yamlrocks.dumps(d, represent=lambda v: None)
    assert out == b"&id001\nself: *id001\n"


def test_multiline_string_defaults_to_literal_block():
    """A deferred multi-line string defaults to a ``|`` literal block."""
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
    """When ``represent`` defers on everything."""
    assert yamlrocks.dumps(doc, represent=lambda v: None) == yamlrocks.dumps(doc)


def test_deferred_values_compose_with_default():
    """A value ``represent`` defers on still reaches the `default` callback."""

    class Money:
        pass

    out = yamlrocks.dumps(
        {"total": Money()},
        default=lambda o: "42 EUR" if isinstance(o, Money) else o,
        represent=lambda v: None,
    )
    assert out == b"total: 42 EUR\n"


def test_deferred_values_compose_with_serializers():
    """A deferred value still reaches the `serializers` registry."""

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
    """`sort_keys` on the represent path orders keys by type and value."""
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
    """Omitting ``represent`` leaves ``dumps`` on its fast path."""
    assert yamlrocks.dumps({"a": [1, 2], "b": "x"}) == b"a:\n  - 1\n  - 2\nb: x\n"


def test_every_value_reaches_represent_through_compounds():
    """Descendants of a deferred compound."""
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
    """A custom object represented as a mapping is deduped with an anchor/alias when repeated."""

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
    """A reference cycle through a custom object resolves to an alias."""

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
    """A block scalar inside a flow collection is invalid YAML."""

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
    """A descriptor tag is checked with the emit-side tag rules."""
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
    """Deferring under an emit option matches a plain dumps with that option."""
    doc = {"b": [1, 2], "a": 3}
    opt = 0 if option is None else getattr(yamlrocks, option)
    assert yamlrocks.dumps(
        doc, option=opt, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=opt)


def test_shared_nested_sequence_emits_anchor():
    """A shared list nested inside a sequence emits its anchor on the dash line."""
    shared = [1]
    out = yamlrocks.dumps([shared, shared], represent=lambda v: None)
    assert out == b"- &id001\n  - 1\n- *id001\n"
    # The anchor/alias pair reloads to a shared list.
    back = yamlrocks.loads(out)
    assert back == [[1], [1]]


def test_default_only_catches_unserializable_type_not_encode_errors():
    """`default` is a fallback for an unrecognized type only; a genuine encode error."""
    doc = {"b": b"\xff\xfe"}
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc, default=lambda o: "x", represent=lambda v: None)
    # Same error as a plain dump.
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc)


def test_non_progressing_default_raises_cleanly():
    """A default that returns its own argument raises cleanly instead of overflowing."""

    class Unserializable:
        pass

    with pytest.raises(ValueError, match="refers only to itself"):
        yamlrocks.dumps(Unserializable(), default=lambda o: o, represent=lambda v: None)


def test_canonical_tags_are_normalized():
    """A canonical `tag:yaml.org,2002:*` tag."""

    def rep(v):
        if v == "b":
            return yamlrocks.YAMLRocksScalar("true", tag="tag:yaml.org,2002:bool")
        if v == "s":
            return yamlrocks.YAMLRocksScalar("123", tag="tag:yaml.org,2002:str")
        return None

    assert yamlrocks.dumps({"k": "b"}, represent=rep) == b"k: true\n"
    assert yamlrocks.dumps({"k": "s"}, represent=rep) == b"k: '123'\n"


def test_self_referential_serializer_raises():
    """A serializer that tags its own input."""

    class Thing:
        pass

    with pytest.raises(ValueError):
        yamlrocks.dumps(
            {"k": Thing()},
            serializers={Thing: lambda o: yamlrocks.YAMLRocksTag("!x", o)},
            represent=lambda v: None,
        )


def test_default_result_shared_elsewhere_anchors_correctly():
    """When `default` returns an object that is also present directly."""

    class Custom:
        pass

    shared = [1]
    out = yamlrocks.dumps(
        [Custom(), shared], default=lambda o: shared, represent=lambda v: None
    )
    assert out == b"- &id001\n  - 1\n- *id001\n"
    assert yamlrocks.loads(out) == [[1], [1]]


def test_shared_mapping_as_sequence_item_anchors_correctly():
    """A shared mapping nested as a sequence item carries its anchor on the dash line."""
    shared = {"x": 1}
    out = yamlrocks.dumps([shared, shared], represent=lambda v: None)
    assert out == b"- &id001\n  x: 1\n- *id001\n"
    assert yamlrocks.loads(out) == [{"x": 1}, {"x": 1}]


def test_chained_default_raises_like_plain_dumps():
    """A `default` whose result is itself unsupported raises."""

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
    """A `YAMLRocksMapping` entry must be a `(key, value)` tuple of exactly two items."""
    with pytest.raises(ValueError, match="key, value"):
        yamlrocks.dumps(
            "m",
            represent=lambda v: yamlrocks.YAMLRocksMapping(bad) if v == "m" else None,
        )


def test_default_returning_container_with_original_raises():
    """A default returning a container that references the original object raises."""

    class C:
        pass

    with pytest.raises(ValueError):
        yamlrocks.dumps(C(), default=lambda o: {"self": o}, represent=lambda v: None)


def test_primitive_subclass_serializer_matches_plain_dumps():
    """A `str`/`int` subclass registered in `serializers` is emitted as its builtin."""

    class MyStr(str):
        pass

    serializers = {MyStr: lambda o: yamlrocks.YAMLRocksTag("!s", str(o))}
    doc = {"k": MyStr("hi")}
    assert yamlrocks.dumps(
        doc, serializers=serializers, represent=lambda v: None
    ) == yamlrocks.dumps(doc, serializers=serializers)


def test_single_newline_string_keeps_chomping():
    """A bare `"\\n"` value keeps its trailing newline via a `|+` block."""
    doc = {"k": "\n"}
    out = yamlrocks.dumps(doc, represent=lambda v: None)
    assert out == yamlrocks.dumps(doc)
    assert yamlrocks.loads(out) == {"k": "\n"}


def test_control_char_scalar_double_quotes_not_single():
    """A value with a control character cannot be single-quoted."""
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
    """Integer keys past `i64` compare as `f64`."""
    doc = {10**20: "a", 10**20 + 1: "b"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_integer_keys_past_f64_range_still_sort_numeric():
    """An integer key past `f64`'s range keeps the numeric rank."""
    doc = {10**400: "a", "z": "b", 10**400 + 1: "c"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_large_integer_keys_sort_exactly():
    """Integer keys keep their exact value when sorting."""
    doc = {9007199254740993: "a", 9007199254740992: "b"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda v: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_sort_keys_does_not_double_invoke_serializers():
    """Sorting a mapping with serialized keys must not invoke the serializer twice."""

    class Custom:
        def __init__(self, name: str) -> None:
            self.name = name

    calls: list[str] = []

    def serialize(obj: Custom):
        calls.append(obj.name)
        return ("!c", obj.name)

    doc = {Custom("b"): 1, Custom("a"): 2}
    serializers = {Custom: serialize}
    deferred = yamlrocks.dumps(
        doc,
        option=yamlrocks.OPT_SORT_KEYS,
        serializers=serializers,
        represent=lambda _: None,
    )
    deferred_calls = list(calls)
    calls.clear()
    plain = yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, serializers=serializers
    )
    assert deferred == plain
    assert deferred_calls == calls == ["b", "a"]


def test_sort_keys_deferred_bad_key_still_raises():
    """A deferred key whose conversion genuinely errors."""
    doc = {b"\xff\xfe": 1, "a": 2}
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda _: None)
    with pytest.raises(yamlrocks.YAMLRocksEncodeError):
        yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_sort_keys_lets_represent_rescue_an_unconvertible_key():
    """`represent` runs first."""

    def rescue(v):
        return yamlrocks.YAMLRocksScalar("safe") if isinstance(v, bytes) else None

    out = yamlrocks.dumps(
        {b"\xff": 1, "a": 2}, option=yamlrocks.OPT_SORT_KEYS, represent=rescue
    )
    assert out == b"a: 2\nsafe: 1\n"


def test_sort_keys_int_subclass_key_sorts_by_numeric_value():
    """A large `int` subclass key that overrides `__str__` sorts by its real numeric value."""

    class Weird(int):
        def __str__(self) -> str:
            return "zzz"

    doc = {Weird(10**30): "big", 5: "small"}
    assert yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda _: None
    ) == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)


def test_deferred_root_tagged_collection_indents_under_tag():
    """A tagged block collection at the document root indents its body one step under the tag."""
    for doc in (
        yamlrocks.YAMLRocksTag("!foo", [1, 2, 3]),
        yamlrocks.YAMLRocksTag("!foo", {"k": "v"}),
    ):
        assert yamlrocks.dumps(doc, represent=lambda _: None) == yamlrocks.dumps(doc)
    assert (
        yamlrocks.dumps(
            yamlrocks.YAMLRocksTag("!foo", [1, 2]), represent=lambda _: None
        )
        == b"!foo\n  - 1\n  - 2\n"
    )


def test_width_with_represent_raises():
    """`width` line-wrapping is not implemented on the represent path."""
    with pytest.raises(ValueError, match="width is not supported"):
        yamlrocks.dumps({"k": "x" * 200}, width=80, represent=lambda _: None)


# --- Regressions from the pre-merge audit rounds ---


def test_alias_as_mapping_key_reloads():
    """An aliased mapping key keeps a space before the colon so the output reloads (anchor names may contain ':')."""
    shared = ("a", "b")
    doc = {"first": shared, shared: "second"}
    block = yamlrocks.dumps(doc, represent=lambda _: None)
    assert block == b"first: &id001\n  - a\n  - b\n*id001 : second\n"
    assert yamlrocks.loads(block) == {"first": ["a", "b"], ("a", "b"): "second"}
    flow = yamlrocks.dumps(
        doc, option=yamlrocks.OPT_FLOW_STYLE, represent=lambda _: None
    )
    assert flow == b"{first: &id001 [a, b], *id001 : second}\n"
    assert yamlrocks.loads(flow) == {"first": ["a", "b"], ("a", "b"): "second"}


def test_flow_unsafe_string_honors_single_quote_preference():
    """A flow-quoted string follows `OPT_SINGLE_QUOTES`, matching plain dumps."""
    option = yamlrocks.OPT_FLOW_STYLE | yamlrocks.OPT_SINGLE_QUOTES
    deferred = yamlrocks.dumps(["a,b"], option=option, represent=lambda _: None)
    assert deferred == yamlrocks.dumps(["a,b"], option=option)
    assert deferred == b"['a,b']\n"


def test_bytes_keys_sort_with_strings():
    """`OPT_SORT_KEYS` ranks a bytes key with the strings, matching plain dumps."""
    doc = {"z": 1, b"a": 2, b"m": 3}
    deferred = yamlrocks.dumps(
        doc, option=yamlrocks.OPT_SORT_KEYS, represent=lambda _: None
    )
    assert deferred == yamlrocks.dumps(doc, option=yamlrocks.OPT_SORT_KEYS)
    assert deferred == b"a: 2\nm: 3\nz: 1\n"


def test_shared_tagged_collection_tag_first_matches_plain_dumps():
    """A tag wrapping a not-yet-shared collection emits a fresh tagged copy; later bare occurrences stay untagged."""
    d = {"k": "v"}
    doc = [yamlrocks.YAMLRocksTag("!x", d), d]
    deferred = yamlrocks.dumps(doc, represent=lambda _: None)
    # Byte-for-byte the plain output: the tag belongs to the wrapper occurrence
    # only, so no alias may inherit it.
    assert deferred == yamlrocks.dumps(doc)
    assert deferred == b"- !x\n  k: v\n- k: v\n"


def test_shared_tagged_collection_alias_first_still_raises():
    """A tag wrapping an already-anchored value raises: an alias cannot carry the tag."""
    d = {"k": "v"}
    with pytest.raises(ValueError, match="shared value"):
        yamlrocks.dumps([d, yamlrocks.YAMLRocksTag("!x", d)], represent=lambda _: None)


def test_nested_tag_raises_on_both_paths():
    """A tag wrapping an already-tagged value raises on the plain and represent paths alike."""
    nested = yamlrocks.YAMLRocksTag("!outer", yamlrocks.YAMLRocksTag("!inner", "v"))
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="already carries a tag"):
        yamlrocks.dumps(nested)
    with pytest.raises(ValueError, match="already carries a tag"):
        yamlrocks.dumps(nested, represent=lambda _: None)


def test_canonical_tag_via_serializers_rejected_like_plain_dumps():
    """A canonical-URI tag from the serializers channel is rejected on both paths (normalization is a descriptor-only affordance)."""

    class Marker:
        pass

    serializers = {Marker: lambda v: ("tag:yaml.org,2002:str", "x")}
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="must start with '!'"):
        yamlrocks.dumps(Marker(), serializers=serializers)
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="must start with '!'"):
        yamlrocks.dumps(Marker(), serializers=serializers, represent=lambda _: None)
    # The descriptor channel keeps normalizing, so PyYAML representers port.
    out = yamlrocks.dumps(
        {"k": "s"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("x", tag="tag:yaml.org,2002:str")
            if v == "s"
            else None
        ),
    )
    assert out == b"k: x\n"


def test_none_type_serializer_is_ignored_like_plain_dumps():
    """`None` renders as a null before the serializers registry is consulted, matching plain dumps."""
    serializers = {type(None): lambda v: ("!n", "x")}
    doc = {"k": None}
    deferred = yamlrocks.dumps(doc, serializers=serializers, represent=lambda _: None)
    assert deferred == yamlrocks.dumps(doc, serializers=serializers)


def test_explicit_plain_style_rejects_structural_content():
    """An explicit plain style that would re-read as structure or lose content raises."""
    cases = [
        "a\nb",
        "x: y",
        "a # c",
        "*alias",
        " padded",
        "padded ",
        "",
        "---",
        "\ufeffx",
    ]
    for value in cases:
        with pytest.raises(ValueError, match="cannot represent this value"):
            yamlrocks.dumps(
                {"k": "s"},
                represent=lambda v, _val=value: (
                    yamlrocks.YAMLRocksScalar(_val, style="plain") if v == "s" else None
                ),
            )


def test_explicit_plain_style_allows_type_changing_content():
    """Forcing plain on content that merely re-reads as another type is the documented use."""
    for value, expected in [
        ("true", b"k: true\n"),
        ("1.5", b"k: 1.5\n"),
        ("-x86", b"k: -x86\n"),
    ]:
        out = yamlrocks.dumps(
            {"k": "s"},
            represent=lambda v, _val=value: (
                yamlrocks.YAMLRocksScalar(_val, style="plain") if v == "s" else None
            ),
        )
        assert out == expected


def test_explicit_single_style_rejects_unrepresentable_content():
    """An explicit single-quoted style raises for line breaks and control characters it cannot escape."""
    for value in ("a\nb", "a\rb", "a\x01b"):
        with pytest.raises(ValueError, match="cannot represent this value"):
            yamlrocks.dumps(
                {"k": "s"},
                represent=lambda v, _val=value: (
                    yamlrocks.YAMLRocksScalar(_val, style="single")
                    if v == "s"
                    else None
                ),
            )
    # A tab is representable in single quotes and honored.
    out = yamlrocks.dumps(
        {"k": "s"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("a\tb", style="single") if v == "s" else None
        ),
    )
    assert out == b"k: 'a\tb'\n"
    assert yamlrocks.loads(out) == {"k": "a\tb"}


def test_explicit_block_style_on_a_key_downgrades_to_double_quotes():
    """A block scalar cannot be an inline key: an explicit literal key emits double-quoted, without a spurious lossiness error."""
    out = yamlrocks.dumps(
        {"a\nb": 1},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, style="literal") if v == "a\nb" else None
        ),
    )
    assert out == b'"a\\nb": 1\n'
    # Content a literal could not hold is fine on a key: the downgrade rescues it.
    out = yamlrocks.dumps(
        {" x\ny": 1},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar(v, style="literal") if v == " x\ny" else None
        ),
    )
    assert out == b'" x\\ny": 1\n'


def test_descriptor_attributes_are_readable():
    """The descriptor fields documented in the API reference are readable attributes."""
    scalar = yamlrocks.YAMLRocksScalar("x", tag="!t", style="literal")
    assert (scalar.value, scalar.tag, scalar.style) == ("x", "!t", "literal")
    assert yamlrocks.YAMLRocksScalar("x").style == "auto"
    seq = yamlrocks.YAMLRocksSequence((1, 2), tag="!s")
    assert (seq.items, seq.tag, seq.flow) == ((1, 2), "!s", None)
    mapping = yamlrocks.YAMLRocksMapping([("k", 1)], flow=True)
    assert (mapping.pairs, mapping.tag, mapping.flow) == ([("k", 1)], None, True)


def test_descriptor_snapshots_one_shot_iterables():
    """A generator-backed descriptor is drained at construction, so reusing it emits the same items every time."""
    descriptor = yamlrocks.YAMLRocksSequence(x for x in [1, 2, 3])

    def rep(v):
        if isinstance(v, list) and v and v[0] == "L":
            return descriptor
        return None

    out = yamlrocks.dumps({"a": ["L", 1], "b": ["L", 2]}, represent=rep)
    assert out == b"a:\n  - 1\n  - 2\n  - 3\nb:\n  - 1\n  - 2\n  - 3\n"


def test_descriptor_reference_cycle_is_collectable():
    """A reference cycle through a descriptor is garbage-collectable, not a permanent leak."""
    import gc
    import weakref

    class Canary:
        pass

    canary = Canary()
    ref = weakref.ref(canary)
    items = [canary]
    descriptor = yamlrocks.YAMLRocksSequence(items)
    items.append(descriptor)
    del items, descriptor, canary
    gc.collect()
    assert ref() is None


def test_document_dump_ignores_represent():
    """Dumping a round-trip document re-emits its preserved layout; `represent` (like the other emit-shaping arguments) is ignored."""
    doc = yamlrocks.loads(b"a: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
    calls: list = []
    out = yamlrocks.dumps(doc, represent=lambda v: calls.append(v))
    assert out == b"a: 1\n"
    assert calls == []


def test_apostrophe_in_custom_tagged_scalar_keeps_single_quotes():
    """A custom-tagged scalar with an apostrophe single-quotes by doubling it, as PyYAML does."""
    out = yamlrocks.dumps(
        {"k": "s"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("it's", tag="!x") if v == "s" else None
        ),
    )
    assert out == b"k: !x 'it''s'\n"
    assert yamlrocks.loads(out) == {"k": "it's"}


def test_wrapper_tag_on_self_referential_value_raises():
    """A wrapper tag on a self-referential value raises: its self-aliases (and any later bare occurrence) would inherit the tag."""
    cyclic: dict = {}
    cyclic["self"] = cyclic
    tag = yamlrocks.YAMLRocksTag("!x", cyclic)
    with pytest.raises(ValueError, match="self-referential or shared"):
        yamlrocks.dumps(tag, represent=lambda _: None)
    with pytest.raises(ValueError, match="self-referential or shared"):
        yamlrocks.dumps([tag, cyclic], represent=lambda _: None)


def test_stateful_scalar_default_runs_per_occurrence_like_plain_dumps():
    """A repeated object whose `default` returns fresh scalars re-invokes the callback per occurrence, matching plain dumps."""

    class Box:
        pass

    box = Box()

    def make_default():
        counter = {"n": 0}

        def default(obj):
            counter["n"] += 1
            return str(counter["n"])

        return default

    deferred = yamlrocks.dumps(
        [box, box], default=make_default(), represent=lambda _: None
    )
    assert deferred == yamlrocks.dumps([box, box], default=make_default())
    assert deferred == b'- "1"\n- "2"\n'


def test_explicit_plain_style_in_flow_downgrades_to_quotes():
    """An explicit plain style whose value cannot stand plain in flow is emitted quoted there, keeping the value intact."""

    def rep(v):
        if v == "s":
            return yamlrocks.YAMLRocksSequence(["m"], flow=True)
        if v == "m":
            return yamlrocks.YAMLRocksScalar("a,b", style="plain")
        return None

    out = yamlrocks.dumps({"k": "s"}, represent=rep)
    assert out == b'k: ["a,b"]\n'
    assert yamlrocks.loads(out) == {"k": ["a,b"]}
    # In block context the same explicit plain is honored verbatim.
    block = yamlrocks.dumps(
        {"k": "m"},
        represent=lambda v: (
            yamlrocks.YAMLRocksScalar("a,b", style="plain") if v == "m" else None
        ),
    )
    assert block == b"k: a,b\n"


def test_shared_object_behind_fresh_default_results_matches_plain_dumps():
    """A repeated object whose `default` mints a fresh result per occurrence emits independent copies, byte-identical to plain dumps."""

    class Box:
        pass

    box = Box()
    doc = [box, box]
    default = lambda o: [1]  # noqa: E731
    deferred = yamlrocks.dumps(doc, default=default, represent=lambda _: None)
    assert deferred == yamlrocks.dumps(doc, default=default)
    # A default that returns the *same* object still aliases: identity follows
    # the node the delegation produced, not the wrapper.
    cached = {"x": 1}
    shared = yamlrocks.dumps(doc, default=lambda o: cached, represent=lambda _: None)
    assert shared == b"- &id001\n  x: 1\n- *id001\n"
    assert yamlrocks.loads(shared) == [{"x": 1}, {"x": 1}]


# --- Byte-for-byte parity sweep: dumps(x, represent=lambda _: None) == dumps(x) ---
#
# A callback that defers on every value must reproduce a plain `dumps` exactly.
# The two paths are separate emitters (the deferred path lowers to a synthetic
# node tree and emits it through the round-trip emitter), so this sweep is the
# guard against them drifting on any type, structural position, or option. The
# one accepted divergence is aliasing: `represent` keeps PyYAML-style anchors for
# shared objects while plain `dumps` never aliases, so a case whose deferred
# output introduces an anchor the plain output lacks is skipped (see ADR-021).


class _ParityEnum(enum.Enum):
    """Enum members whose values exercise the container and numeric key paths."""

    TUPLE = (1, 2)
    NUMBER = 9


@dataclasses.dataclass
class _ParityDataclass:
    """A dataclass base, decomposed into its fields on both paths."""

    x: int = 1
    name: str = "n"


def _parity_bases():
    """A value of every type and edge the emitter special-cases."""
    return [
        None,
        True,
        False,
        0,
        -1,
        2**63,
        2**63 - 1,
        10**20,
        10**400,
        0.0,
        1.5,
        1e16,
        1e-5,
        1e308,
        float("inf"),
        float("nan"),
        "",
        "plain",
        "it's",
        'has "quote"',
        "true",
        "null",
        "123",
        "~",
        ": leading",
        "trailing ",
        " leading",
        "a\nb",
        "a\nb\n",
        "  indented\nfirst\n",
        "a\tb",
        "a\x00b",
        "a\x7fb",
        "café",
        # Flow-indicator and BOM-leading strings: quoted only in flow context
        # (and by quote preference), the seam between the two emitters' quoting.
        "a,b",
        "[x]",
        "{k: v}",
        "\ufeffbom",
        b"bytes",
        b"with\nnewline",
        datetime.date(2020, 1, 2),
        datetime.datetime(2020, 1, 2, 3, 4, 5),
        datetime.datetime(2020, 1, 2, 3, 4, 5, 123456),
        datetime.time(3, 4, 5),
        _ParityEnum.TUPLE,
        _ParityEnum.NUMBER,
        _ParityDataclass(),
        decimal.Decimal("2.5"),
        [],
        {},
        [1, 2, 3],
        {"a": 1, "b": 2},
        (1, 2),
        (),
        frozenset([1]),
        # Tagged values, including collections that at the document root must
        # indent their body under the tag rather than emit it flush.
        yamlrocks.YAMLRocksTag("!t", "scalar"),
        yamlrocks.YAMLRocksTag("!t", None),
        yamlrocks.YAMLRocksTag("!t", [1, 2, 3]),
        yamlrocks.YAMLRocksTag("!t", {"k": "v"}),
        yamlrocks.YAMLRocksTag("!t", {"a": [1, 2]}),
    ]


def _parity_positions(value, hashable):
    """The same value embedded in every structural position."""
    yield value
    yield [value]
    yield [value, value]
    yield [1, value]
    yield {"k": value}
    yield {"a": 1, "k": value}
    yield {"k": value, "z": 2}
    yield {"a": {"b": value}}
    yield [[value]]
    yield {"list": [value]}
    yield [{"k": value}]
    # Two accepted, non-byte-identical renderings are skipped as keys:
    #  - A tagged value: the fast path emits it inline (`!t [1, 2]: x`) while the
    #    represent path emits a valid explicit `? !t ...` block key.
    #  - A datetime/date/time, Decimal, or Enum: under `OPT_SORT_KEYS` the
    #    represent path keeps such a converting key in input order rather than
    #    ranking it by its converted form (ranking would double-run the
    #    conversion; see `sorted_pairs`), while the fast path orders it by that
    #    form.
    # Both reload identically; they are documented limitations, not bugs.
    keyable = not isinstance(
        value,
        (
            yamlrocks.YAMLRocksTag,
            datetime.date,
            datetime.time,
            decimal.Decimal,
            enum.Enum,
        ),
    )
    if hashable and keyable:
        yield {value: "x"}
        yield {"a": 1, value: "x"}


_PARITY_OPTIONS = {
    "default": 0,
    "sort_keys": yamlrocks.OPT_SORT_KEYS,
    "flow": yamlrocks.OPT_FLOW_STYLE,
    "single_quotes": yamlrocks.OPT_SINGLE_QUOTES,
    "explicit_start": yamlrocks.OPT_EXPLICIT_START,
    "explicit_end": yamlrocks.OPT_EXPLICIT_END,
    "null_keyword": yamlrocks.OPT_NULL_AS_KEYWORD,
    "null_tilde": yamlrocks.OPT_NULL_AS_TILDE,
    "indent_4": yamlrocks.OPT_INDENT_4,
    "indentless": yamlrocks.OPT_INDENTLESS_SEQUENCES,
    "sort_flow": yamlrocks.OPT_SORT_KEYS | yamlrocks.OPT_FLOW_STYLE,
    # Flow forces the emitters' own quoting fallbacks; combined with the quote
    # preference it pins the seam where they must pick the same quote character.
    "flow_single": yamlrocks.OPT_FLOW_STYLE | yamlrocks.OPT_SINGLE_QUOTES,
}


@pytest.mark.parametrize("option_name", _PARITY_OPTIONS)
def test_deferred_output_matches_plain_dumps(option_name):
    """A fully deferred callback reproduces plain dumps byte-for-byte across a broad corpus."""
    option = _PARITY_OPTIONS[option_name]
    for base in _parity_bases():
        try:
            hash(base)
            hashable = True
        except TypeError:
            hashable = False
        for doc in _parity_positions(base, hashable):
            plain = yamlrocks.dumps(doc, option=option)
            try:
                deferred = yamlrocks.dumps(doc, option=option, represent=lambda _: None)
            except ValueError as err:
                # Accepted aliasing divergence: a shared tagged value cannot carry
                # its tag onto an alias, so the represent path raises where plain
                # (which never aliases) emits it twice. Only that specific edge is
                # tolerated; any other error is a real failure.
                if "shared value" in str(err):
                    continue
                raise
            # Accepted aliasing divergence: the represent path anchors a shared
            # object where plain `dumps` duplicates it, so the bytes differ. The
            # reloaded value must still match, though: this is what catches a lossy
            # anchor (e.g. a shared tagged null that emitted `!x &id001 null` and
            # reloaded as the string "null" instead of an empty value).
            if b"&id" in deferred and b"&id" not in plain:
                assert yamlrocks.loads(deferred) == yamlrocks.loads(plain), (
                    f"aliasing reload mismatch: {base!r} in {doc!r} [{option_name}]"
                )
                continue
            assert deferred == plain, f"{base!r} in {doc!r} [{option_name}]"
