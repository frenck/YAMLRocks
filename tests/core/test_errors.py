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
