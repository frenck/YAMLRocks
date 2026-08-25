"""Custom tag handling: tag_handler callback, passthrough, and defaults."""

from __future__ import annotations

import yamlrocks


def test_default_strips_custom_tag():
    """By default a custom tag is dropped, keeping its scalar value."""
    assert yamlrocks.loads(b"x: !custom foo") == {"x": "foo"}


def test_passthrough_returns_tag_object():
    """OPT_PASSTHROUGH_TAG returns a YAMLRocksTag object with its tag and value."""
    result = yamlrocks.loads(b"x: !custom foo", option=yamlrocks.OPT_PASSTHROUGH_TAG)
    tag = result["x"]
    assert isinstance(tag, yamlrocks.YAMLRocksTag)
    assert tag.tag == "!custom"
    assert tag.value == "foo"


def test_tag_handler_called_with_tag_and_value():
    """tag_handler is invoked with the tag and value and its result is used."""
    seen = []

    def handler(tag, value):
        seen.append((tag, value))
        return f"{tag}={value}"

    result = yamlrocks.loads(b"x: !greet world", tag_handler=handler)
    assert result == {"x": "!greet=world"}
    assert seen == [("!greet", "world")]


def test_tag_handler_on_mapping():
    """tag_handler receives the parsed mapping as the tagged value."""
    result = yamlrocks.loads(
        b"point: !vec\n  x: 1\n  y: 2\n",
        tag_handler=lambda tag, value: (tag, value),
    )
    assert result == {"point": ("!vec", {"x": 1, "y": 2})}


def test_tag_handler_on_sequence():
    """tag_handler receives the parsed sequence as the tagged value."""
    result = yamlrocks.loads(
        b"data: !set\n  - a\n  - b\n",
        tag_handler=lambda tag, value: {"_tag": tag, "_items": value},
    )
    assert result == {"data": {"_tag": "!set", "_items": ["a", "b"]}}


def test_standard_tags_not_treated_as_custom():
    """Core-schema tags like !!str are applied, not treated as custom."""
    # !!str is a core-schema tag, not a custom one.
    assert yamlrocks.loads(b"x: !!str 42") == {"x": "42"}


def test_explicit_core_tag_with_nonconforming_content_stays_a_string():
    """A core tag whose content does not match its type is kept as a string."""
    # Not coerced to a wrong-but-valid value (`!!int nope` used to become 0).
    assert yamlrocks.loads(b"x: !!int nope") == {"x": "nope"}
    assert yamlrocks.loads(b"x: !!float nope") == {"x": "nope"}
    assert yamlrocks.loads(b"x: !!bool maybe") == {"x": "maybe"}
    assert yamlrocks.loads(b"x: !!null text") == {"x": "text"}
    # A conforming value still resolves to its type (including a big int).
    assert yamlrocks.loads(b"x: !!int 42") == {"x": 42}
    assert yamlrocks.loads(b"x: !!null") == {"x": None}
    assert yamlrocks.loads(b"x: !!int 99999999999999999999") == {
        "x": 99999999999999999999
    }
    # An integer-form value under `!!float` is a conforming float, not a string.
    assert yamlrocks.loads(b"x: !!float 42") == {"x": 42.0}


def test_nested_custom_tags():
    """tag_handler is applied to custom tags nested within other custom tags."""
    result = yamlrocks.loads(
        b"outer: !a\n  inner: !b value\n",
        tag_handler=lambda tag, value: {tag: value},
    )
    assert result == {"outer": {"!a": {"inner": {"!b": "value"}}}}


def test_tag_constructor_and_attributes():
    """YAMLRocksTag can be constructed directly and exposes its tag and value."""
    tag = yamlrocks.YAMLRocksTag("!x", [1, 2, 3])
    assert tag.tag == "!x"
    assert tag.value == [1, 2, 3]


def test_tag_repr():
    """repr of a YAMLRocksTag shows the tag name with an elided value."""
    tag = yamlrocks.YAMLRocksTag("!custom", "payload")
    assert repr(tag) == "YAMLRocksTag('!custom', ...)"


# -- Emitting custom tags (the write side) -----------------------------------


def test_dumps_tag_object_scalar():
    """A YAMLRocksTag with a scalar value emits `!tag value`."""
    assert yamlrocks.dumps({"x": yamlrocks.YAMLRocksTag("!input", "foo")}) == (
        b"x: !input foo\n"
    )


def test_dumps_tag_object_at_root():
    """A YAMLRocksTag may be the whole document."""
    assert yamlrocks.dumps(yamlrocks.YAMLRocksTag("!input", "foo")) == b"!input foo\n"


def test_dumps_tag_mapping_value_is_block():
    """A tagged mapping drops to an indented block under the tag."""
    tag = yamlrocks.YAMLRocksTag("!extend", {"a": 1, "b": 2})
    assert yamlrocks.dumps({"e": tag}) == b"e: !extend\n  a: 1\n  b: 2\n"


def test_dumps_tag_sequence_item():
    """A tagged value works as a block-sequence item."""
    tag = yamlrocks.YAMLRocksTag("!input", "foo")
    assert yamlrocks.dumps({"items": [tag, "plain"]}) == (
        b"items:\n  - !input foo\n  - plain\n"
    )


def test_dumps_tag_multiline_scalar_is_block_literal():
    """A tagged multi-line string emits as a tagged block scalar."""
    tag = yamlrocks.YAMLRocksTag("!lambda", "return id(s).state > 5;\n")
    assert yamlrocks.dumps({"lambda": tag}) == (
        b"lambda: !lambda |\n  return id(s).state > 5;\n"
    )


def test_dumps_passthrough_tag_round_trips():
    """A passthrough-loaded document re-emits its tags byte-for-byte."""
    data = yamlrocks.loads(b"x: !input foo\n", option=yamlrocks.OPT_PASSTHROUGH_TAG)
    assert yamlrocks.dumps(data) == b"x: !input foo\n"


def test_dumps_serializers_registry_by_type():
    """serializers={type: func} emits a `!tag value` for a registered type."""

    class Marker:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"hello": Marker("world")},
        serializers={Marker: lambda o: yamlrocks.YAMLRocksTag("!marker", o.name)},
    )
    assert out == b"hello: !marker world\n"


def test_dumps_serializers_registry_tuple_return():
    """A serializers callback may return a (tag, value) tuple instead of a Tag."""

    class Marker:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"x": Marker("z")},
        serializers={Marker: lambda o: ("!marker", o.name)},
    )
    assert out == b"x: !marker z\n"


def test_dumps_serializers_registry_before_dataclass():
    """A registered dataclass type emits a tag rather than auto-mapping."""
    from dataclasses import dataclass

    @dataclass
    class Input:
        name: str

    out = yamlrocks.dumps(
        {"hello": Input("test_name")},
        serializers={Input: lambda o: yamlrocks.YAMLRocksTag("!input", o.name)},
    )
    assert out == b"hello: !input test_name\n"
    # Round-trip closes with the load-side tags registry.
    back = yamlrocks.loads(out, tags={"!input": lambda v: Input(str(v))})
    assert back == {"hello": Input("test_name")}


def test_dumps_serializers_registry_is_exact_type():
    """Matching is by exact type; a subclass is not caught by a base entry."""

    class Base:
        pass

    class Derived(Base):
        pass

    # Only Base is registered, so a Derived instance is not matched and falls
    # through to the unserializable error.
    import pytest

    with pytest.raises(yamlrocks.YAMLRocksUnserializableError):
        yamlrocks.dumps(
            {"x": Derived()},
            serializers={Base: lambda o: yamlrocks.YAMLRocksTag("!b", "x")},
        )


def test_dump_side_tags_keyword_is_gone():
    """The dump-side registry is `serializers=`, not `tags=`; the old name was
    renamed with no deprecation alias, so it must raise (guards against a future
    accidental re-introduction)."""
    import pytest

    class Marker:
        pass

    reg = {Marker: lambda o: yamlrocks.YAMLRocksTag("!m", "x")}
    for fn in (yamlrocks.dumps, yamlrocks.dump, yamlrocks.async_dump):
        with pytest.raises(TypeError):
            fn({"x": Marker()}, tags=reg)


def test_dumps_default_may_return_tag():
    """A `default` callback returning a YAMLRocksTag is emitted as a tag."""

    class Marker:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"x": Marker("foo")},
        default=lambda o: yamlrocks.YAMLRocksTag("!marker", o.name),
    )
    assert out == b"x: !marker foo\n"


def test_dumps_serializers_callback_bad_return_raises():
    """A serializers callback that returns a non-tag, non-tuple value errors clearly."""
    import pytest

    class Marker:
        pass

    with pytest.raises(yamlrocks.YAMLRocksError):
        yamlrocks.dumps({"x": Marker()}, serializers={Marker: lambda o: 123})


def test_dumps_tag_with_whitespace_is_rejected():
    """A tag containing whitespace would split on reload, so it is rejected.

    `YAMLRocksTag("!bad tag", "v")` would emit `!bad tag v`, reloading as the tag
    `!bad` on the value `"tag v"`; the emitter refuses rather than corrupt.
    """
    import pytest

    for bad in ["!bad tag", "!x\ny: 1", "!t\tx"]:
        with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="invalid tag"):
            yamlrocks.dumps(yamlrocks.YAMLRocksTag(bad, "v"))


def test_dumps_tag_without_leading_bang_is_rejected():
    """A tag must start with `!`; otherwise it is not a tag at all."""
    import pytest

    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="must start with"):
        yamlrocks.dumps(yamlrocks.YAMLRocksTag("noexcl", "v"))


def test_dumps_tuple_tag_with_whitespace_is_rejected():
    """A `(tag, value)` callback return is validated the same as a YAMLRocksTag."""
    import pytest

    class Marker:
        pass

    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="invalid tag"):
        yamlrocks.dumps(
            {"x": Marker()}, serializers={Marker: lambda o: ("!bad tag", "v")}
        )


def test_dumps_verbatim_tag_with_commas_is_allowed():
    """A verbatim tag carrying URI commas has no whitespace and is allowed."""
    out = yamlrocks.dumps(yamlrocks.YAMLRocksTag("!<tag:example.com,2024:foo>", "v"))
    assert out == b"!<tag:example.com,2024:foo> v\n"


def test_dumps_non_first_tagged_key_uses_explicit_key_form():
    """A tagged key after another entry is emitted in explicit `? key` form.

    An inline tagged key (`!t k:`) after a previous entry would have its `!t`
    read as that entry's value's node property (a reparse error); the `?`
    indicator starts a fresh key that the preceding value cannot absorb.
    """
    data = {"a": None, yamlrocks.YAMLRocksTag("!foo", "k"): 2}
    out = yamlrocks.dumps(data)
    assert out == b"a:\n? !foo k\n: 2\n"
    # It reloads cleanly (the tagged key is dropped by default, leaving its value).
    assert yamlrocks.loads(out) == {"a": None, "k": 2}
    # The same after a non-null value (which inline form would also corrupt).
    assert yamlrocks.dumps({"a": 1, yamlrocks.YAMLRocksTag("!foo", "k"): 2}) == (
        b"a: 1\n? !foo k\n: 2\n"
    )


def test_dumps_first_tagged_key_stays_inline():
    """A first/only tagged key has no preceding entry, so it stays inline."""
    assert yamlrocks.dumps({yamlrocks.YAMLRocksTag("!foo", "k"): 1}) == b"!foo k: 1\n"
    # A null value before an untagged key still uses the compact empty form.
    assert yamlrocks.dumps({"a": None, "b": 2}) == b"a:\nb: 2\n"


def test_dumps_shorthand_tag_with_flow_indicator_is_rejected():
    """A flow indicator terminates a shorthand tag scan, so it would corrupt.

    `YAMLRocksTag("!foo,bar", "v")` would emit `!foo,bar v`, reloading as the
    tag `!foo` on the value `,bar v` (which is itself malformed).
    """
    import pytest

    for bad in ["!foo,bar", "!foo]bar", "!foo}bar", "!!str,x"]:
        with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="flow indicator"):
            yamlrocks.dumps(yamlrocks.YAMLRocksTag(bad, "v"))


def test_dumps_shorthand_tag_with_bracket_or_brace_is_allowed():
    """`[`/`{` do not terminate a tag scan, so they stay part of the tag."""
    assert yamlrocks.dumps(yamlrocks.YAMLRocksTag("!foo[bar", "v")) == b"!foo[bar v\n"


def test_dumps_named_tag_handle_is_rejected():
    """A named handle (`!h!x`) needs a `%TAG` directive dumps never writes, so emitting it would produce YAML `loads` rejects."""
    import pytest

    for bad in ["!h!x", "!e!foo", "!!str!x"]:
        with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="named tag handle"):
            yamlrocks.dumps(yamlrocks.YAMLRocksTag(bad, "v"))
        with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="named tag handle"):
            yamlrocks.dumps(object(), serializers={object: lambda o, _t=bad: (_t, "v")})


def test_dumps_malformed_verbatim_tag_is_rejected():
    """A verbatim tag must be `!<...>` with non-empty content and a closing `>`."""
    import pytest

    # `!<tag:a>b>` closes at the first `>`, so emitting it would truncate the tag
    # and leave `b>` in the document as content.
    for bad in ["!<>", "!<unterminated", "!<tag:foo", "!<tag:a>b>"]:
        with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="verbatim tag"):
            yamlrocks.dumps(yamlrocks.YAMLRocksTag(bad, "v"))


def test_to_json_drops_tag():
    """to_json emits the inner value and drops the tag."""
    data = yamlrocks.loads(b"x: !input foo\n", option=yamlrocks.OPT_PASSTHROUGH_TAG)
    assert yamlrocks.to_json(data) == b'{"x":"foo"}'


def test_tag_directive_resolves_named_handle():
    """A `%TAG` directive's handle expands in the document it introduces."""
    src = b"%TAG !e! tag:example.com,2024:\n---\nx: !e!foo bar\n"
    # The named handle resolves; the custom tag is stripped by default, leaving
    # the inner value.
    assert yamlrocks.loads(src) == {"x": "bar"}


def test_roundtrip_edit_keeps_tag_directive():
    """Editing a round-trip document re-emits its `%TAG` directive.

    Dropping the directive would strand the `!e!foo` node on an undefined handle
    that no longer reloads; the edited document must stay self-contained.
    """
    src = b"%TAG !e! tag:example.com,2020:\n---\nv: !e!foo old\nw: keep\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    doc["w"] = "changed"
    out = yamlrocks.dumps(doc)
    assert out == (b"%TAG !e! tag:example.com,2020:\n---\nv: !e!foo old\nw: changed\n")
    assert yamlrocks.loads(out) == {"v": "old", "w": "changed"}


def test_roundtrip_unmodified_tag_directive_is_byte_identical():
    """An unmodified round-trip document with a `%TAG` re-emits verbatim."""
    src = b"%TAG !e! tag:example.com,2020:\n---\nv: !e!foo old\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    assert yamlrocks.dumps(doc) == src


def test_roundtrip_dropped_empty_document_does_not_leak_its_directive():
    """A directive scoped to a skipped empty document does not reach the next."""
    # The `%TAG` belongs to the empty first document; once that document is
    # dropped, re-emitting the second (modified) one must not carry the handle.
    src = b"%TAG !e! tag:x:\n---\n---\nb: 2\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    doc["b"] = 9
    out = yamlrocks.dumps(doc)
    assert b"%TAG" not in out
    assert out == b"---\nb: 9\n"


def test_roundtrip_edit_keeps_yaml_version_directive():
    """Editing a round-trip document re-emits its `%YAML` version directive."""
    src = b"%YAML 1.2\n---\nv: 1\nw: 2\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    doc["w"] = 9
    assert yamlrocks.dumps(doc) == b"%YAML 1.2\n---\nv: 1\nw: 9\n"


def test_duplicate_tag_directive_is_rejected():
    """A `%TAG` handle declared twice in one document is an error.

    The spec allows each handle at most once per document; we previously accepted
    a repeat with last-wins, inconsistent with the duplicate-`%YAML` rejection.
    """
    import pytest

    src = b"%TAG !e! tag:a:\n%TAG !e! tag:b:\n---\nx: 1\n"
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="duplicate %TAG"):
        yamlrocks.loads(src)
    # A different handle on the same line is fine.
    assert yamlrocks.loads(b"%TAG !e! tag:a:\n%TAG !f! tag:b:\n---\nx: 1\n") == {"x": 1}


def test_verbatim_tag_canonicalized_for_handler():
    """A verbatim `!<uri>` reaches a handler as the bare URI, like `%TAG` expansion."""
    src = b"x: !<tag:example.com,2020:app/foo> bar\n"
    seen = []
    yamlrocks.loads(src, tag_handler=lambda tag, val: seen.append(tag) or val)
    assert seen == ["tag:example.com,2020:app/foo"]


def test_verbatim_tag_passthrough_keeps_verbatim_spelling():
    """A passthrough `YAMLRocksTag` keeps the `!<...>` spelling so `dumps` accepts it.

    Canonicalizing the passthrough tag to a bare URI would make `dumps` reject it
    (`validate_tag` requires a leading `!`), breaking the load/dump round-trip.
    """
    src = b"x: !<tag:example.com,2020:app/foo> bar\n"
    node = yamlrocks.loads(src, option=yamlrocks.OPT_PASSTHROUGH_TAG)["x"]
    assert node.tag == "!<tag:example.com,2020:app/foo>"
    # The verbatim tag survives a dump/load round-trip through the passthrough object.
    reloaded = yamlrocks.loads(
        yamlrocks.dumps(node), option=yamlrocks.OPT_PASSTHROUGH_TAG
    )
    assert reloaded.tag == node.tag
    assert reloaded.value == node.value


def test_verbatim_tag_value_round_trips():
    """A verbatim-tagged value survives a dump/load round-trip via passthrough.

    The default `loads` strips custom tags, so this exercises `OPT_PASSTHROUGH_TAG`,
    where the `!<...>` spelling is preserved and `dumps` can re-emit it.
    """
    for src in [b"!<tag:example.com,2020:foo> bar", b"!<a> y"]:
        tag = yamlrocks.loads(src, option=yamlrocks.OPT_PASSTHROUGH_TAG)
        reloaded = yamlrocks.loads(
            yamlrocks.dumps(tag), option=yamlrocks.OPT_PASSTHROUGH_TAG
        )
        assert (reloaded.tag, reloaded.value) == (tag.tag, tag.value), src
