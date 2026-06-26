"""The ``!secret`` tag (``OPT_SECRETS``).

``!secret NAME`` reads a value from a ``secrets.yaml`` file, searching from the
requesting file's directory up to the base directory. It is a configuration
convention shared by tools such as Home Assistant and ESPHome, not specific to
any one of them; resolution matches home-assistant-libs/annotatedyaml.

The tag is opt-in on its own ``OPT_SECRETS`` flag because it crosses a distinct
trust boundary (reading a secrets file). See ``test_env_var_tag`` for the
separate ``!env_var`` feature, and ``test_security`` for the path-confinement
guarantees around ``secrets.yaml``.
"""

from __future__ import annotations

import pytest

import yamlrocks

SECRETS = yamlrocks.OPT_SECRETS
INC = yamlrocks.OPT_INCLUDES


@pytest.fixture
def config(tmp_path):
    (tmp_path / "secrets.yaml").write_text(
        "api_key: SECRET123\ndb_pass: hunter2\nlogger: debug\n"
    )
    return tmp_path


def test_secret_resolves(config):
    """!secret resolves a value from secrets.yaml."""
    data = yamlrocks.loads(
        b"api: !secret api_key\n", option=SECRETS, include_dir=str(config)
    )
    assert data == {"api": "SECRET123"}


def test_secret_logger_key_filtered(config):
    """!secret refuses to expose the special 'logger' key."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError):
        yamlrocks.loads(b"x: !secret logger\n", option=SECRETS, include_dir=str(config))


def test_secret_walks_up_directories(config):
    """!secret in an included file resolves against a parent secrets.yaml."""
    sub = config / "packages"
    sub.mkdir()
    (sub / "p.yaml").write_text("key: !secret db_pass\n")
    data = yamlrocks.loads(
        b"pkg: !include packages/p.yaml\n",
        option=INC | SECRETS,
        include_dir=str(config),
    )
    assert data == {"pkg": {"key": "hunter2"}}


def test_secret_resolves_with_relative_include_dir(config, monkeypatch):
    """!secret resolves when include_dir is relative (e.g. '.').

    The common Home Assistant layout runs the loader from the config directory
    with a relative include_dir; the secret climb must compare paths in canonical
    space, not against the raw relative base.
    """
    monkeypatch.chdir(config)
    (config / "app.yaml").write_text("key: !secret api_key\n")
    assert yamlrocks.load("app.yaml", option=SECRETS, include_dir=".") == {
        "key": "SECRET123"
    }
    # And a secret referenced from an included file, still under a relative base.
    sub = config / "packages"
    sub.mkdir()
    (sub / "p.yaml").write_text("pw: !secret db_pass\n")
    (config / "main.yaml").write_text("pkg: !include packages/p.yaml\n")
    assert yamlrocks.load("main.yaml", option=INC | SECRETS, include_dir=".") == {
        "pkg": {"pw": "hunter2"}
    }


def test_secret_undefined_raises(config):
    """!secret raises when the requested secret is not defined."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="secret 'missing'"):
        yamlrocks.loads(
            b"x: !secret missing\n", option=SECRETS, include_dir=str(config)
        )


def test_secret_requires_its_own_flag(config):
    """Without OPT_SECRETS, !secret is a plain custom tag and is not expanded."""
    # Includes alone must not read secrets.yaml.
    data = yamlrocks.loads(
        b"api: !secret api_key\n", option=INC, include_dir=str(config)
    )
    assert data == {"api": "api_key"}


def test_secret_round_trip_preserves_directive_no_leak(config):
    """Round-trip keeps the !secret directive and never leaks the value."""
    doc = yamlrocks.loads(
        b"api: !secret api_key\n",
        option=yamlrocks.OPT_ROUND_TRIP | SECRETS,
        include_dir=str(config),
    )
    out = doc.to_yaml()
    assert b"!secret api_key" in out
    assert b"SECRET123" not in out  # the secret value must not leak into output


# -- secrets.yaml validation semantics (matching annotatedyaml) ----------------


def test_non_dict_secrets_file_errors(tmp_path):
    """A secrets.yaml that is not a mapping is a configuration error."""
    (tmp_path / "secrets.yaml").write_text("- a\n- b\n")
    with pytest.raises(
        yamlrocks.YAMLRocksIncludeError, match="not contain a dictionary"
    ):
        yamlrocks.loads(b"x: !secret foo", option=SECRETS, include_dir=str(tmp_path))


def test_nested_secret_in_secrets_file_is_rejected(tmp_path):
    """A !secret inside secrets.yaml is rejected; secrets cannot self-reference."""
    (tmp_path / "secrets.yaml").write_text("a: 1\nb: !secret a\n")
    with pytest.raises(
        yamlrocks.YAMLRocksIncludeError, match="not supported in a secrets"
    ):
        yamlrocks.loads(b"x: !secret a", option=SECRETS, include_dir=str(tmp_path))


def test_bad_logger_value_logs_but_does_not_fail(tmp_path, caplog):
    """A non-debug logger value in secrets.yaml logs a warning and continues,
    without echoing the offending value (secrets.yaml content) into the log."""
    (tmp_path / "secrets.yaml").write_text("logger: s3cr3t-leak\nfoo: bar\n")
    with caplog.at_level("WARNING", logger="yamlrocks"):
        data = yamlrocks.loads(
            b"x: !secret foo", option=SECRETS, include_dir=str(tmp_path)
        )
    assert data == {"x": "bar"}
    assert any("logger: debug" in r.message for r in caplog.records)
    # The bad value lives in secrets.yaml, so it must never reach the log.
    assert not any("s3cr3t-leak" in r.message for r in caplog.records)


def test_good_logger_value_is_silent(tmp_path, caplog):
    """logger: debug is the expected value and emits no diagnostic."""
    (tmp_path / "secrets.yaml").write_text("logger: debug\nfoo: bar\n")
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(b"x: !secret foo", option=SECRETS, include_dir=str(tmp_path))
    assert not caplog.records


# -- Collecting missing secrets: callback + OPT_SECRET_NOT_FOUND_WARN ----------
# By default an undefined !secret is a hard error (fail-fast: never boot with a
# hole where a secret belongs). The on_missing_secret callback and the
# OPT_SECRET_NOT_FOUND_WARN flag opt into collecting every miss in one pass
# instead, resolving each to None. Structural secrets.yaml faults still raise.


def test_missing_secret_raises_by_default(tmp_path):
    """The default is fail-fast: the first undefined secret raises."""
    with pytest.raises(yamlrocks.YAMLRocksSecretNotFoundError):
        yamlrocks.loads(
            b"a: !secret x\nb: !secret y\n", option=SECRETS, include_dir=str(tmp_path)
        )


def test_missing_secret_error_carries_location(tmp_path):
    """The default raise points at the offending !secret (line/column/file), not
    just the file, so every consumer's default error is located."""
    with pytest.raises(yamlrocks.YAMLRocksSecretNotFoundError) as err:
        yamlrocks.loads(
            b"a: 1\nb: !secret missing\n", option=SECRETS, include_dir=str(tmp_path)
        )
    assert err.value.line == 2
    assert err.value.column is not None
    assert err.value.file is not None


def test_on_missing_secret_callback_collects_every_miss(tmp_path):
    """The callback fires once per undefined secret (in document order), each
    node resolves to None, and the load continues."""
    misses = []
    data = yamlrocks.loads(
        b"a: !secret x\nb: !secret yy\n",
        option=SECRETS,
        include_dir=str(tmp_path),
        on_missing_secret=lambda name, file, line: misses.append((name, line)),
    )
    assert data == {"a": None, "b": None}
    assert misses == [("x", 1), ("yy", 2)]


def test_on_missing_secret_receives_the_requesting_file(tmp_path):
    """The callback's file argument is the file containing the !secret."""
    (tmp_path / "configuration.yaml").write_bytes(b"a: !secret x\n")
    seen = []
    yamlrocks.load(
        str(tmp_path / "configuration.yaml"),
        option=SECRETS | yamlrocks.OPT_INCLUDES,
        on_missing_secret=lambda name, file, line: seen.append((name, file, line)),
    )
    assert len(seen) == 1
    name, file, line = seen[0]
    assert name == "x" and line == 1 and file.endswith("configuration.yaml")


def test_secret_not_found_warn_flag_logs_and_continues(tmp_path, caplog):
    """The flag logs a WARNING per miss on the yamlrocks logger and continues."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        data = yamlrocks.loads(
            b"a: !secret x\nb: !secret yy\n",
            option=SECRETS | yamlrocks.OPT_SECRET_NOT_FOUND_WARN,
            include_dir=str(tmp_path),
        )
    assert data == {"a": None, "b": None}
    messages = [r.message for r in caplog.records]
    assert sum("is not defined in any secrets.yaml" in m for m in messages) == 2


def test_found_secrets_are_unaffected_by_collecting(tmp_path):
    """A defined secret still resolves normally while the flag is set; only the
    undefined one is collected as None."""
    (tmp_path / "secrets.yaml").write_text("real: value\n")
    data = yamlrocks.loads(
        b"a: !secret real\nb: !secret missing\n",
        option=SECRETS | yamlrocks.OPT_SECRET_NOT_FOUND_WARN,
        include_dir=str(tmp_path),
    )
    assert data == {"a": "value", "b": None}


def test_structural_secrets_fault_still_raises_under_the_flag(tmp_path):
    """Only 'name not defined' downgrades; a non-mapping secrets.yaml still
    raises even with the flag set."""
    (tmp_path / "secrets.yaml").write_text("- not\n- a\n- mapping\n")
    with pytest.raises(yamlrocks.YAMLRocksIncludeError):
        yamlrocks.loads(
            b"a: !secret x\n",
            option=SECRETS | yamlrocks.OPT_SECRET_NOT_FOUND_WARN,
            include_dir=str(tmp_path),
        )


def test_callback_and_flag_both_active(tmp_path, caplog):
    """The callback and the flag compose: both fire for the same miss."""
    misses = []
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(
            b"a: !secret x\n",
            option=SECRETS | yamlrocks.OPT_SECRET_NOT_FOUND_WARN,
            include_dir=str(tmp_path),
            on_missing_secret=lambda name, file, line: misses.append(name),
        )
    assert misses == ["x"]
    assert any("is not defined" in r.message for r in caplog.records)
