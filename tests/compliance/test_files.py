"""File-based ``load``/``load_all``/``dump`` APIs."""

from __future__ import annotations

import io

import pytest

import yamlrocks


def test_load_from_path(tmp_path):
    """load() reads YAML from a Path object."""
    p = tmp_path / "config.yaml"
    p.write_text("name: app\nport: 8080\n")
    assert yamlrocks.load(p) == {"name": "app", "port": 8080}


def test_load_from_str_path(tmp_path):
    """load() reads YAML from a string file path."""
    p = tmp_path / "config.yaml"
    p.write_text("a: 1\n")
    assert yamlrocks.load(str(p)) == {"a": 1}


def test_load_from_file_object(tmp_path):
    """load() reads YAML from an open binary file object."""
    p = tmp_path / "config.yaml"
    p.write_bytes(b"x: hello\n")
    with open(p, "rb") as f:
        assert yamlrocks.load(f) == {"x": "hello"}


def test_load_text_file_object():
    """load() reads YAML from a text stream."""
    assert yamlrocks.load(io.StringIO("k: v\n")) == {"k": "v"}


def test_load_all_from_path(tmp_path):
    """load_all() reads every document from a multi-document file."""
    p = tmp_path / "multi.yaml"
    p.write_text("---\na: 1\n---\nb: 2\n")
    assert yamlrocks.load_all(p) == [{"a": 1}, {"b": 2}]


def test_load_includes_default_to_file_dir(tmp_path):
    """load() resolves !include against the loaded file's directory by default."""
    (tmp_path / "automations.yaml").write_text("- alias: morning\n")
    root = tmp_path / "configuration.yaml"
    root.write_text("automation: !include automations.yaml\n")
    # No include_dir given - it defaults to the file's directory.
    data = yamlrocks.load(root, option=yamlrocks.OPT_INCLUDES)
    assert data == {"automation": [{"alias": "morning"}]}


def test_dump_to_path(tmp_path):
    """dump() writes YAML to a Path and reloads to the same data."""
    p = tmp_path / "out.yaml"
    yamlrocks.dump({"name": "app", "ports": [80, 443]}, p)
    assert p.read_bytes() == b"name: app\nports:\n  - 80\n  - 443\n"
    assert yamlrocks.load(p) == {"name": "app", "ports": [80, 443]}


def test_dump_to_str_path(tmp_path):
    """dump() writes YAML to a string file path."""
    p = tmp_path / "out.yaml"
    yamlrocks.dump({"a": 1}, str(p))
    assert p.read_text() == "a: 1\n"


def test_dump_to_binary_stream():
    """dump() writes YAML bytes to a binary stream."""
    buf = io.BytesIO()
    yamlrocks.dump({"a": 1}, buf)
    assert buf.getvalue() == b"a: 1\n"


def test_dump_to_text_stream():
    """dump() writes YAML text to a text stream."""
    buf = io.StringIO()
    yamlrocks.dump({"a": 1}, buf)
    assert buf.getvalue() == "a: 1\n"


def test_dump_with_options(tmp_path):
    """dump() honors options such as OPT_SORT_KEYS when writing to a file."""
    p = tmp_path / "out.yaml"
    yamlrocks.dump({"b": 2, "a": 1}, p, option=yamlrocks.OPT_SORT_KEYS)
    assert p.read_text() == "a: 1\nb: 2\n"


def test_load_dump_round_trip_via_files(tmp_path):
    """A load then dump then load cycle through files preserves the data."""
    src = tmp_path / "src.yaml"
    src.write_text("server:\n  host: localhost\n  ports:\n    - 80\n")
    obj = yamlrocks.load(src)
    out = tmp_path / "out.yaml"
    yamlrocks.dump(obj, out)
    assert yamlrocks.load(out) == obj


def test_missing_file_raises(tmp_path):
    """load() raises FileNotFoundError for a nonexistent path."""
    with pytest.raises(FileNotFoundError):
        yamlrocks.load(tmp_path / "does-not-exist.yaml")
