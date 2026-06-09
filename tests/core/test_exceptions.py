"""The exception hierarchy: base class, subtree, builtin compatibility, and the
structured ``file``/``line``/``column`` (and ``include_stack``/``schema_path``)
attributes."""

from __future__ import annotations

import os

import pytest

import yamlrocks
from yamlrocks import exceptions as ex


def test_everything_derives_from_the_base():
    """Every concrete error is a YAMLRocksError."""
    for name in ex.__all__:
        cls = getattr(ex, name)
        assert issubclass(cls, ex.YAMLRocksError)


def test_category_builtin_compatibility():
    """Decode errors are ValueErrors; encode errors are TypeErrors."""
    assert issubclass(ex.YAMLRocksDecodeError, ValueError)
    assert issubclass(ex.YAMLRocksEncodeError, TypeError)


def test_parse_error_is_specific_and_located():
    """A syntax error is a YAMLRocksParseError carrying 1-based line/column."""
    with pytest.raises(ex.YAMLRocksParseError) as info:
        yamlrocks.loads(b'key: "unterminated')
    exc = info.value
    assert isinstance(exc, ex.YAMLRocksDecodeError)
    assert isinstance(exc, ValueError)
    assert exc.line == 1
    assert exc.column == 6
    assert exc.file is None  # in-memory input has no path
    assert exc.message


def test_duplicate_key_error(tmp_path):
    """A duplicate key under OPT_DUPLICATE_KEYS_ERROR is its own class."""
    with pytest.raises(ex.YAMLRocksDuplicateKeyError):
        yamlrocks.loads(b"a: 1\na: 2\n", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)


def test_schema_error_carries_schema_path():
    """A schema failure is a YAMLRocksSchemaError with the failing path."""
    schema = {"type": "object", "properties": {"n": {"type": "integer"}}}
    with pytest.raises(ex.YAMLRocksSchemaError) as info:
        yamlrocks.loads(b"n: not-an-int\n", schema=schema)
    assert info.value.schema_path is not None


def test_include_not_found_is_specific(tmp_path):
    """A missing include is a YAMLRocksIncludeNotFoundError with file + chain."""
    (tmp_path / "main.yaml").write_text("x: !include nope.yaml\n")
    with pytest.raises(ex.YAMLRocksIncludeNotFoundError) as info:
        yamlrocks.load(str(tmp_path / "main.yaml"), option=yamlrocks.OPT_INCLUDES)
    exc = info.value
    assert isinstance(exc, ex.YAMLRocksIncludeError)
    assert isinstance(exc, ex.YAMLRocksDecodeError)
    assert exc.file is not None
    assert isinstance(exc.include_stack, list)


def test_circular_include_is_specific(tmp_path):
    """A cycle raises YAMLRocksCircularIncludeError."""
    (tmp_path / "a.yaml").write_text("x: !include b.yaml\n")
    (tmp_path / "b.yaml").write_text("y: !include a.yaml\n")
    with pytest.raises(ex.YAMLRocksCircularIncludeError):
        yamlrocks.load(str(tmp_path / "a.yaml"), option=yamlrocks.OPT_INCLUDES)


def test_include_depth_is_specific(tmp_path):
    """An over-deep include chain raises YAMLRocksIncludeDepthError."""
    for i in range(60):
        target = f"!include f{i + 1}.yaml" if i < 59 else "done"
        (tmp_path / f"f{i}.yaml").write_text(f"v: {target}\n")
    with pytest.raises(ex.YAMLRocksIncludeDepthError):
        yamlrocks.load(str(tmp_path / "f0.yaml"), option=yamlrocks.OPT_INCLUDES)


def test_include_confinement_is_specific(tmp_path):
    """An escape attempt raises YAMLRocksIncludeConfinementError."""
    (tmp_path / "outside.yaml").write_text("x: 1\n")
    cfg = tmp_path / "config"
    cfg.mkdir()
    (cfg / "main.yaml").write_text("x: !include ../outside.yaml\n")
    with pytest.raises(ex.YAMLRocksIncludeConfinementError):
        yamlrocks.load(str(cfg / "main.yaml"), option=yamlrocks.OPT_INCLUDES)


def test_secret_not_found_is_specific(tmp_path):
    """A missing secret raises YAMLRocksSecretNotFoundError (a SecretError)."""
    (tmp_path / "secrets.yaml").write_text("api_key: x\n")
    (tmp_path / "main.yaml").write_text("v: !secret missing\n")
    with pytest.raises(ex.YAMLRocksSecretNotFoundError) as info:
        yamlrocks.load(str(tmp_path / "main.yaml"), option=yamlrocks.OPT_SECRETS)
    assert isinstance(info.value, ex.YAMLRocksSecretError)


def test_env_var_error_is_specific(monkeypatch):
    """An undefined !env_var raises YAMLRocksEnvVarError."""
    monkeypatch.delenv("YAMLROCKS_UNSET_XYZ", raising=False)
    with pytest.raises(ex.YAMLRocksEnvVarError):
        yamlrocks.loads(
            b"v: !env_var YAMLROCKS_UNSET_XYZ\n", option=yamlrocks.OPT_ENV_VAR
        )


def test_unserializable_error_is_specific():
    """A value with no representation raises YAMLRocksUnserializableError."""
    with pytest.raises(ex.YAMLRocksUnserializableError) as info:
        yamlrocks.dumps(object())
    assert isinstance(info.value, ex.YAMLRocksEncodeError)
    assert isinstance(info.value, TypeError)


def test_load_enriches_file_attribute(tmp_path):
    """Errors from load() know the path they came from."""
    bad = tmp_path / "bad.yaml"
    bad.write_text('key: "unterminated')
    with pytest.raises(ex.YAMLRocksParseError) as info:
        yamlrocks.load(str(bad))
    assert info.value.file == os.fspath(bad)
