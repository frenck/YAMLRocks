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


def test_dumps_tags_registry_by_type():
    """tags={type: func} emits a `!tag value` for a registered type."""

    class Marker:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"hello": Marker("world")},
        tags={Marker: lambda o: yamlrocks.YAMLRocksTag("!marker", o.name)},
    )
    assert out == b"hello: !marker world\n"


def test_dumps_tags_registry_tuple_return():
    """A tags callback may return a (tag, value) tuple instead of a Tag."""

    class Marker:
        def __init__(self, name):
            self.name = name

    out = yamlrocks.dumps(
        {"x": Marker("z")},
        tags={Marker: lambda o: ("!marker", o.name)},
    )
    assert out == b"x: !marker z\n"


def test_dumps_tags_registry_before_dataclass():
    """A registered dataclass type emits a tag rather than auto-mapping."""
    from dataclasses import dataclass

    @dataclass
    class Input:
        name: str

    out = yamlrocks.dumps(
        {"hello": Input("test_name")},
        tags={Input: lambda o: yamlrocks.YAMLRocksTag("!input", o.name)},
    )
    assert out == b"hello: !input test_name\n"
    # Round-trip closes with the load-side tags registry.
    back = yamlrocks.loads(out, tags={"!input": lambda v: Input(str(v))})
    assert back == {"hello": Input("test_name")}


def test_dumps_tags_registry_is_exact_type():
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
            tags={Base: lambda o: yamlrocks.YAMLRocksTag("!b", "x")},
        )


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


def test_dumps_tags_callback_bad_return_raises():
    """A tags callback that returns a non-tag, non-tuple value errors clearly."""
    import pytest

    class Marker:
        pass

    with pytest.raises(yamlrocks.YAMLRocksError):
        yamlrocks.dumps({"x": Marker()}, tags={Marker: lambda o: 123})


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
