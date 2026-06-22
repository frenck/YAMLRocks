"""Error reporting quality."""

from __future__ import annotations

import pytest

import yamlrocks


def test_unterminated_double_quote():
    """Raise YAMLRocksDecodeError for an unterminated double-quoted scalar."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(b'key: "unterminated')


def test_unterminated_single_quote():
    """Raise YAMLRocksDecodeError for an unterminated single-quoted scalar."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(b"key: 'unterminated")


def test_invalid_utf8_raises_value_error():
    """Raise ValueError for input that is not valid UTF-8.

    The bytes start with ASCII (so they are not a UTF-16/32 BOM) but contain an
    invalid UTF-8 sequence, exercising the UTF-8 decode error path.
    """
    with pytest.raises(ValueError):
        yamlrocks.loads(b"key: \xc3\x28")


def test_error_message_includes_line_and_column():
    """Include line and column information in the error message."""
    try:
        yamlrocks.loads(b'key: "unterminated')
    except yamlrocks.YAMLRocksDecodeError as exc:
        message = str(exc)
        assert "line" in message
        assert "column" in message
    else:  # pragma: no cover - the call must raise
        pytest.fail("expected YAMLRocksDecodeError")


def test_decode_error_subclasses_value_error():
    """Verify YAMLRocksDecodeError is a subclass of ValueError."""
    with pytest.raises(ValueError):
        yamlrocks.loads(b'x: "open')


# -- Tabs and indentation --
#
# A tab is never indentation (YAML indents with spaces) but is valid separation
# before a plain or flow scalar. So a tab is rejected only when it would indent a
# block collection node, and accepted when it merely separates a scalar.


@pytest.mark.parametrize(
    "src",
    [
        pytest.param(b"-\t-\n", id="dash-tab-dash"),
        pytest.param(b"- \t-\n", id="dash-space-tab-dash"),
        pytest.param(b"?\t-\n", id="explicit-key-tab-dash"),
        pytest.param(b"?\tkey:\n", id="explicit-key-tab-mapping"),
        pytest.param(b"a:\n\tb: c\n", id="tab-before-mapping-key"),
        pytest.param(b'a:\n\t"b": c\n', id="tab-before-double-quoted-key"),
        pytest.param(b"a:\n\t'b': c\n", id="tab-before-single-quoted-key"),
        pytest.param(b"a:\n\t&x b: c\n", id="tab-before-anchored-key"),
        pytest.param(b"- [\n\tfoo,\n foo\n ]\n", id="tab-indent-in-flow"),
    ],
)
def test_tab_cannot_indent_a_block_collection(src):
    """Reject a tab that indents a block sequence entry or mapping key."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="tab"):
        yamlrocks.loads(src)


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param(b"key:\tvalue\n", {"key": "value"}, id="tab-after-colon"),
        pytest.param(b"-\tvalue\n", ["value"], id="tab-after-dash"),
        pytest.param(b"foo:\n \tbar\n", {"foo": "bar"}, id="tab-before-plain-value"),
        pytest.param(b"[ a,\tb ]\n", ["a", "b"], id="tab-as-flow-separation"),
    ],
)
def test_tab_is_valid_separation_before_a_scalar(src, expected):
    """Accept a tab that separates a scalar rather than indenting a collection."""
    assert yamlrocks.loads(src) == expected
