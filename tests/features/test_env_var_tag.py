"""The ``!env_var`` tag (``OPT_ENV_VAR``).

``!env_var NAME [default words]`` reads an environment variable, falling back to
the default text when the variable is unset and erroring on a bare, undefined
variable. It is a configuration convention shared by tools such as Home
Assistant and ESPHome, not specific to any one of them; resolution matches
home-assistant-libs/annotatedyaml.

The tag is opt-in on its own ``OPT_ENV_VAR`` flag because it crosses a distinct
trust boundary (reading the process environment). See ``test_secret_tag`` for
the separate ``!secret`` feature.
"""

from __future__ import annotations

import pytest

import yamlrocks

ENV = yamlrocks.OPT_ENV_VAR
INC = yamlrocks.OPT_INCLUDES


def test_env_var_present(monkeypatch):
    """!env_var resolves to the environment variable's value when set."""
    monkeypatch.setenv("YAMLROCKS_HOST", "example.com")
    data = yamlrocks.loads(b"host: !env_var YAMLROCKS_HOST\n", option=ENV)
    assert data == {"host": "example.com"}


def test_env_var_default_when_missing(monkeypatch):
    """!env_var falls back to its default when the variable is unset."""
    monkeypatch.delenv("YAMLROCKS_PORT", raising=False)
    data = yamlrocks.loads(b"port: !env_var YAMLROCKS_PORT 8080\n", option=ENV)
    assert data == {"port": "8080"}


def test_env_var_multiword_default(monkeypatch):
    """!env_var uses the full multi-word default text when unset."""
    monkeypatch.delenv("YAMLROCKS_X", raising=False)
    data = yamlrocks.loads(b"v: !env_var YAMLROCKS_X some default text\n", option=ENV)
    assert data == {"v": "some default text"}


def test_env_var_undefined_raises(monkeypatch):
    """!env_var without a default raises when the variable is undefined."""
    monkeypatch.delenv("YAMLROCKS_NOPE", raising=False)
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not defined"):
        yamlrocks.loads(b"x: !env_var YAMLROCKS_NOPE\n", option=ENV)


def test_undefined_env_var_error_carries_location(monkeypatch):
    """The default raise points at the offending !env_var (line/column), not just
    the file."""
    monkeypatch.delenv("YAMLROCKS_NOPE", raising=False)
    with pytest.raises(yamlrocks.YAMLRocksEnvVarError) as err:
        yamlrocks.loads(b"a: 1\nx: !env_var YAMLROCKS_NOPE\n", option=ENV)
    assert err.value.line == 2
    assert err.value.column is not None


def test_env_var_present_ignores_default(monkeypatch):
    """!env_var prefers the set variable over its default."""
    monkeypatch.setenv("YAMLROCKS_Y", "real")
    data = yamlrocks.loads(b"y: !env_var YAMLROCKS_Y fallback\n", option=ENV)
    assert data == {"y": "real"}


def test_env_var_requires_its_own_flag(monkeypatch):
    """Without OPT_ENV_VAR, !env_var is a plain custom tag and is not expanded."""
    monkeypatch.setenv("YAMLROCKS_HOST", "example.com")
    # No flag at all.
    assert yamlrocks.loads(b"host: !env_var YAMLROCKS_HOST\n") == {
        "host": "YAMLROCKS_HOST"
    }
    # Enabling includes alone does not enable env-var expansion.
    assert yamlrocks.loads(b"host: !env_var YAMLROCKS_HOST\n", option=INC) == {
        "host": "YAMLROCKS_HOST"
    }


def test_env_var_round_trip_preserves_directive(monkeypatch):
    """Round-trip keeps the !env_var directive rather than inlining the value."""
    monkeypatch.setenv("YAMLROCKS_HOST", "example.com")
    doc = yamlrocks.loads(
        b"host: !env_var YAMLROCKS_HOST\n",
        option=yamlrocks.OPT_ROUND_TRIP | ENV,
    )
    out = doc.to_yaml()
    assert b"!env_var YAMLROCKS_HOST" in out
    assert b"example.com" not in out  # the resolved value is not written back


# -- Collecting missing env vars: callback + OPT_ENV_VAR_NOT_FOUND_WARN --------
# Mirrors the !secret missing-handling. By default a bare undefined !env_var is a
# hard error; the on_missing_env_var callback and the OPT_ENV_VAR_NOT_FOUND_WARN
# flag opt into collecting every miss (resolving each to None and continuing). A
# variable that supplies a default is never a miss.


def test_on_missing_env_var_callback_collects_every_miss(monkeypatch):
    """The callback fires once per undefined variable (no default), in order,
    each node resolves to None, and the load continues."""
    monkeypatch.delenv("YR_A", raising=False)
    monkeypatch.delenv("YR_B", raising=False)
    misses = []
    data = yamlrocks.loads(
        b"a: !env_var YR_A\nb: !env_var YR_B\n",
        option=ENV,
        on_missing_env_var=lambda name, file, line: misses.append((name, line)),
    )
    assert data == {"a": None, "b": None}
    assert misses == [("YR_A", 1), ("YR_B", 2)]


def test_env_var_with_default_is_not_a_miss(monkeypatch):
    """A variable that supplies a default uses it and never counts as missing."""
    monkeypatch.delenv("YR_A", raising=False)
    misses = []
    data = yamlrocks.loads(
        b"a: !env_var YR_A fallback\n",
        option=ENV,
        on_missing_env_var=lambda name, file, line: misses.append(name),
    )
    assert data == {"a": "fallback"}
    assert misses == []


def test_env_var_not_found_warn_flag_logs_and_continues(monkeypatch, caplog):
    """The flag logs a WARNING per miss on the yamlrocks logger and continues."""
    monkeypatch.delenv("YR_A", raising=False)
    monkeypatch.delenv("YR_B", raising=False)
    with caplog.at_level("WARNING", logger="yamlrocks"):
        data = yamlrocks.loads(
            b"a: !env_var YR_A\nb: !env_var YR_B\n",
            option=ENV | yamlrocks.OPT_ENV_VAR_NOT_FOUND_WARN,
        )
    assert data == {"a": None, "b": None}
    messages = [r.message for r in caplog.records]
    assert (
        sum("environment variable" in m and "not defined" in m for m in messages) == 2
    )


def test_defined_env_var_unaffected_by_collecting(monkeypatch):
    """A defined variable still resolves while a sibling miss is collected."""
    monkeypatch.setenv("YR_SET", "yes")
    monkeypatch.delenv("YR_A", raising=False)
    data = yamlrocks.loads(
        b"a: !env_var YR_SET\nb: !env_var YR_A\n",
        option=ENV | yamlrocks.OPT_ENV_VAR_NOT_FOUND_WARN,
    )
    assert data == {"a": "yes", "b": None}


def test_missing_secret_and_env_var_callbacks_are_independent(monkeypatch, tmp_path):
    """The two callbacks fire for their own tag only."""
    monkeypatch.delenv("YR_A", raising=False)
    secrets, envs = [], []
    data = yamlrocks.loads(
        b"s: !secret nope\ne: !env_var YR_A\n",
        option=yamlrocks.OPT_SECRETS | ENV,
        include_dir=str(tmp_path),
        on_missing_secret=lambda name, file, line: secrets.append(name),
        on_missing_env_var=lambda name, file, line: envs.append(name),
    )
    assert data == {"s": None, "e": None}
    assert secrets == ["nope"]
    assert envs == ["YR_A"]
