"""Option flag wiring and exception types."""

from __future__ import annotations

import pytest

import yamlrocks


def test_all_option_flags_exist():
    """All documented option flags are exposed as integers."""
    for name in [
        "OPT_YAML_1_1",
        "OPT_ROUND_TRIP",
        "OPT_ANNOTATED",
        "OPT_INCLUDES",
        "OPT_INDENT_2",
        "OPT_INDENT_4",
        "OPT_SORT_KEYS",
        "OPT_FLOW_STYLE",
        "OPT_SINGLE_QUOTES",
        "OPT_INDENTLESS_SEQUENCES",
        "OPT_NULL_AS_KEYWORD",
        "OPT_NULL_AS_TILDE",
        "OPT_EXPLICIT_START",
        "OPT_EXPLICIT_END",
        "OPT_DUPLICATE_KEYS_ERROR",
        "OPT_DUPLICATE_KEYS_WARN",
        "OPT_YAML_1_1_WARN",
        "OPT_PYYAML_COMPAT",
        "OPT_ANNOTATE_NUMBERS",
        "OPT_REJECT_COMPLEX_KEYS",
        "OPT_SECRET_NOT_FOUND_WARN",
        "OPT_ENV_VAR_NOT_FOUND_WARN",
        "OPT_INCLUDE_DIR_RECURSIVE",
        "OPT_PASSTHROUGH_TAG",
        "OPT_SERIALIZE_NUMPY",
        "OPT_UPGRADE_1_1",
        "OPT_OMIT_MICROSECONDS",
        "OPT_NAIVE_UTC",
        "OPT_UTC_Z",
        "OPT_PASSTHROUGH_DATETIME",
        "OPT_PASSTHROUGH_DATACLASS",
        "OPT_SECRETS",
        "OPT_ENV_VAR",
    ]:
        assert isinstance(getattr(yamlrocks, name), int)


def test_removed_non_str_keys_flag():
    """OPT_NON_STR_KEYS was removed; YAML supports non-string keys natively."""
    assert not hasattr(yamlrocks, "OPT_NON_STR_KEYS")
    # Non-string keys round-trip without any flag.
    assert yamlrocks.loads(b"1: a\n2: b") == {1: "a", 2: "b"}
    assert yamlrocks.dumps({1: "a"}) == b"1: a\n"


def test_duplicate_keys_last_wins_by_default():
    """Without the flag, a repeated key keeps the last value (PyYAML behavior)."""
    assert yamlrocks.loads(b"a: 1\na: 2") == {"a": 2}


def test_duplicate_keys_error_flag():
    """OPT_DUPLICATE_KEYS_ERROR rejects a mapping that repeats a key."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="duplicate mapping key: a"
    ):
        yamlrocks.loads(b"a: 1\nb: 2\na: 3", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)


def test_duplicate_keys_error_reports_location():
    """The duplicate-key error carries the line of the offending key."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="line 3"):
        yamlrocks.loads(b"a: 1\nb: 2\na: 3", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)


def test_duplicate_keys_error_nested():
    """Duplicate detection applies to nested mappings too."""
    with pytest.raises(
        yamlrocks.YAMLRocksDecodeError, match="duplicate mapping key: k"
    ):
        yamlrocks.loads(
            b"x:\n  k: 1\n  k: 2", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR
        )


def test_duplicate_keys_error_allows_unique():
    """A document with unique keys passes with the flag set."""
    out = yamlrocks.loads(b"a: 1\nb: 2", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)
    assert out == {"a": 1, "b": 2}


def test_duplicate_keys_error_exempts_merge_key():
    """Repeating the merge key `<<` is allowed; it is how merges compose."""
    out = yamlrocks.loads(
        b"a: &a {x: 1}\nb: &b {y: 2}\nc:\n  <<: [*a, *b]\n  z: 3",
        option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR,
    )
    assert out["c"] == {"z": 3, "x": 1, "y": 2}


def test_duplicate_keys_error_in_loads_all():
    """loads_all honors OPT_DUPLICATE_KEYS_ERROR for each document."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="duplicate mapping key"):
        yamlrocks.loads_all(
            b"---\na: 1\na: 2", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR
        )


@pytest.mark.parametrize(
    "extra_option",
    [0, yamlrocks.OPT_ROUND_TRIP, yamlrocks.OPT_ANNOTATED],
    ids=["fast", "round_trip", "annotated"],
)
def test_duplicate_keys_error_on_every_load_path(extra_option):
    """OPT_DUPLICATE_KEYS_ERROR applies on the AST-backed paths too, not only
    the fast path."""
    option = yamlrocks.OPT_DUPLICATE_KEYS_ERROR | extra_option
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="duplicate mapping key"):
        yamlrocks.loads(b"a: 1\na: 2\n", option=option)


def test_duplicate_keys_error_round_trip_exempts_merge_key():
    """The merge key << may repeat on the round-trip path too."""
    doc = "base1: &a {x: 1}\nbase2: &b {y: 2}\nm:\n  <<: *a\n  <<: *b\n"
    option = yamlrocks.OPT_DUPLICATE_KEYS_ERROR | yamlrocks.OPT_ROUND_TRIP
    assert yamlrocks.loads(doc, option=option) is not None


def test_duplicate_keys_distinguish_value_types():
    """An int key and a string key are not duplicates."""
    out = yamlrocks.loads(b"1: a\n'1': b\n", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)
    assert out == {1: "a", "1": "b"}


@pytest.mark.parametrize(
    "src",
    [
        b"1: a\n1.0: b\n",  # int vs equal float
        b"true: a\n1: b\n",  # bool True vs 1
        b"0: a\nfalse: b\n",  # 0 vs False
        b"2.0: a\n2: b\n",  # integral float vs int
    ],
)
@pytest.mark.parametrize(
    "extra", [0, yamlrocks.OPT_ROUND_TRIP, yamlrocks.OPT_ANNOTATED]
)
def test_duplicate_keys_error_catches_python_equal_numeric_keys(src, extra):
    """Keys distinct in YAML but equal as Python dict keys (1, 1.0, True) are flagged."""
    with pytest.raises(yamlrocks.YAMLRocksDuplicateKeyError):
        yamlrocks.loads(src, option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR | extra)


def test_duplicate_keys_numeric_non_equal_are_allowed():
    """Numerically distinct keys (1 and 2, 1 and 1.5) are not duplicates."""
    assert yamlrocks.loads(
        b"1: a\n2: b\n", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR
    ) == {1: "a", 2: "b"}
    assert yamlrocks.loads(
        b"1: a\n1.5: b\n", option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR
    ) == {1: "a", 1.5: "b"}


def test_option_flags_are_distinct_bits():
    """Each option flag is a single distinct bit."""
    flags = [
        yamlrocks.OPT_YAML_1_1,
        yamlrocks.OPT_ROUND_TRIP,
        yamlrocks.OPT_ANNOTATED,
        yamlrocks.OPT_INCLUDES,
        yamlrocks.OPT_INDENT_2,
        yamlrocks.OPT_INDENT_4,
        yamlrocks.OPT_SORT_KEYS,
        yamlrocks.OPT_FLOW_STYLE,
        yamlrocks.OPT_PYYAML_COMPAT,
        yamlrocks.OPT_ANNOTATE_NUMBERS,
    ]
    # Each flag is a single distinct bit.
    for flag in flags:
        assert flag and (flag & (flag - 1)) == 0
    assert len(set(flags)) == len(flags)


def test_combined_options():
    """Combining sort-keys and indent options affects the output."""
    out = yamlrocks.dumps(
        {"b": 1, "a": 2}, option=yamlrocks.OPT_SORT_KEYS | yamlrocks.OPT_INDENT_4
    )
    assert out == b"a: 2\nb: 1\n"


def test_decode_error_is_value_error():
    """YAMLRocksDecodeError is a subclass of ValueError."""
    assert issubclass(yamlrocks.YAMLRocksDecodeError, ValueError)


def test_encode_error_is_type_error():
    """YAMLRocksEncodeError is a subclass of TypeError."""
    assert issubclass(yamlrocks.YAMLRocksEncodeError, TypeError)


def test_passthrough_tag_returns_tag_object():
    """Passthrough mode surfaces custom tags as YAMLRocksTag objects."""
    result = yamlrocks.loads(b"x: !custom foo", option=yamlrocks.OPT_PASSTHROUGH_TAG)
    # With passthrough, custom tags surface as YAMLRocksTag objects.
    assert isinstance(result["x"], yamlrocks.YAMLRocksTag)
    assert result["x"].tag == "!custom"
    assert result["x"].value == "foo"


def test_duplicate_keys_warn_logs_and_keeps_last(caplog):
    """OPT_DUPLICATE_KEYS_WARN logs a diagnostic but keeps last-wins (fast path)."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        out = yamlrocks.loads(
            b"a: 1\nb: 2\na: 3", option=yamlrocks.OPT_DUPLICATE_KEYS_WARN
        )
    assert out == {"a": 3, "b": 2}
    assert any("duplicate mapping key 'a'" in r.message for r in caplog.records)


def test_duplicate_keys_warn_in_annotated(caplog):
    """The warning is emitted on the annotated (AST) path too."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        out = yamlrocks.loads(
            b"a: 1\na: 2",
            option=yamlrocks.OPT_DUPLICATE_KEYS_WARN | yamlrocks.OPT_ANNOTATED,
        )
    assert dict(out) == {"a": 2}
    assert any("duplicate mapping key 'a'" in r.message for r in caplog.records)


def test_duplicate_keys_no_warn_by_default(caplog):
    """Without the flag, duplicates are silent (no log record)."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(b"a: 1\na: 2")
    assert not caplog.records


def test_duplicate_keys_error_takes_precedence_over_warn():
    """The fatal flag wins when both error and warn are set."""
    with pytest.raises(yamlrocks.YAMLRocksDuplicateKeyError):
        yamlrocks.loads(
            b"a: 1\na: 2",
            option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR
            | yamlrocks.OPT_DUPLICATE_KEYS_WARN,
        )


def test_yaml_11_warn_logs_one_one_only_syntax(caplog):
    """OPT_YAML_1_1_WARN logs each scalar that 1.2 would type differently."""
    src = b"enabled: yes\nmask: 0777\nport: 8080\nname: app\n"
    with caplog.at_level("WARNING", logger="yamlrocks"):
        out = yamlrocks.loads(
            src, option=yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_YAML_1_1_WARN
        )
    assert out == {"enabled": True, "mask": 511, "port": 8080, "name": "app"}
    messages = [r.message for r in caplog.records]
    assert any("'yes'" in m and "bool in 1.1" in m for m in messages)
    assert any("'0777'" in m and "int in 1.1" in m for m in messages)
    # Values that resolve the same in both schemas are not flagged.
    assert not any("8080" in m or "'app'" in m for m in messages)


def test_yaml_11_warn_fires_on_upgrade(caplog):
    """The migration flag OPT_UPGRADE_1_1 activates the diagnostic too, reported
    against the original 1.1 spelling before the upgrade rewrites it."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(
            b"a: yes\n", option=yamlrocks.OPT_UPGRADE_1_1 | yamlrocks.OPT_YAML_1_1_WARN
        )
    assert any("'yes'" in r.message for r in caplog.records)


def test_yaml_11_warn_is_noop_without_1_1_mode(caplog):
    """Without 1.1/upgrade mode the flag does nothing (1.2 has no 1.1 syntax)."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        out = yamlrocks.loads(b"a: yes\n", option=yamlrocks.OPT_YAML_1_1_WARN)
    assert out == {"a": "yes"}
    assert not caplog.records


def test_yaml_11_mode_silent_without_warn_flag(caplog):
    """1.1 mode alone emits nothing; the diagnostic is opt-in."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(b"a: yes\n", option=yamlrocks.OPT_YAML_1_1)
    assert not caplog.records
