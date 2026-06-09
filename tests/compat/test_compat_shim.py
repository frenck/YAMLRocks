"""The PyYAML-compatible shim, ``yamlrocks.compat``."""

from __future__ import annotations

import io

import pytest

import yamlrocks.compat as yaml


def test_safe_load_str():
    """safe_load parses a str input."""
    assert yaml.safe_load("a: 1\nb: 2") == {"a": 1, "b": 2}


def test_safe_load_bytes():
    """safe_load parses a bytes input."""
    assert yaml.safe_load(b"key: value") == {"key": "value"}


def test_safe_load_file_like():
    """safe_load reads from a file-like object."""
    assert yaml.safe_load(io.StringIO("x: hello")) == {"x": "hello"}


def test_safe_load_all():
    """safe_load_all returns a list of all documents in the stream."""
    assert yaml.safe_load_all("---\na: 1\n---\nb: 2") == [{"a": 1}, {"b": 2}]


def test_load_is_safe_load():
    """load ignores the Loader argument and behaves like safe_load."""
    assert yaml.load("a: 1", Loader=object()) == {"a": 1}


def test_safe_dump_returns_str():
    """safe_dump returns a str when no stream is given."""
    out = yaml.safe_dump({"a": 1, "b": 2})
    assert isinstance(out, str)
    assert out == "a: 1\nb: 2\n"


def test_safe_dump_sorts_keys_by_default():
    """safe_dump sorts mapping keys by default."""
    # PyYAML sorts keys by default; the shim matches that.
    assert yaml.safe_dump({"c": 3, "a": 1, "b": 2}) == "a: 1\nb: 2\nc: 3\n"


def test_safe_dump_unsorted():
    """safe_dump preserves key order when sort_keys is false."""
    assert yaml.safe_dump({"c": 3, "a": 1}, sort_keys=False) == "c: 3\na: 1\n"


def test_safe_dump_to_stream():
    """safe_dump writes to a stream and returns None."""
    buf = io.StringIO()
    assert yaml.safe_dump({"k": "v"}, buf) is None
    assert buf.getvalue() == "k: v\n"


def test_safe_dump_all():
    """safe_dump_all emits multiple documents separated by '---'."""
    assert yaml.safe_dump_all([{"a": 1}, {"b": 2}]) == "a: 1\n---\nb: 2\n"


def test_yaml_error_is_catchable():
    """A parse failure raises the shim's YAMLError."""
    with pytest.raises(yaml.YAMLError):
        yaml.safe_load(b"x: 'unterminated")


def test_round_trip_through_shim():
    """A nested object survives a safe_dump then safe_load round-trip."""
    obj = {"name": "app", "ports": [80, 443], "nested": {"k": "v"}}
    assert yaml.safe_load(yaml.safe_dump(obj)) == obj
