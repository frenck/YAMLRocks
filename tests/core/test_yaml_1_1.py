"""YAML 1.1 compatibility mode (``OPT_YAML_1_1``)."""

from __future__ import annotations

import pytest

import yamlrocks


def load11(text: str):
    return yamlrocks.loads(f"x: {text}".encode(), option=yamlrocks.OPT_YAML_1_1)["x"]


@pytest.mark.parametrize("text", ["yes", "Yes", "YES", "on", "On", "true", "True"])
def test_truthy_words(text):
    """Resolve YAML 1.1 truthy words to True under OPT_YAML_1_1."""
    assert load11(text) is True


@pytest.mark.parametrize("text", ["no", "No", "NO", "off", "Off", "false", "False"])
def test_falsy_words(text):
    """Resolve YAML 1.1 falsy words to False under OPT_YAML_1_1."""
    assert load11(text) is False


def test_yaml_12_keeps_yes_as_string():
    """Keep "yes" as a string in default YAML 1.2 mode."""
    assert yamlrocks.loads(b"x: yes")["x"] == "yes"


def test_1_1_does_not_affect_plain_strings():
    """Leave ordinary plain strings unchanged in YAML 1.1 mode."""
    assert load11("hello") == "hello"


def test_1_1_integers_still_parse():
    """Resolve integer literals in YAML 1.1 mode."""
    assert load11("42") == 42


@pytest.mark.parametrize("text", ["y", "Y"])
def test_single_letter_truthy(text):
    """Resolve single-letter y/Y to True under OPT_YAML_1_1."""
    assert load11(text) is True


@pytest.mark.parametrize("text", ["n", "N"])
def test_single_letter_falsy(text):
    """Resolve single-letter n/N to False under OPT_YAML_1_1."""
    assert load11(text) is False


@pytest.mark.parametrize(
    "text",
    ["~", "null", "Null", "NULL", ""],
)
def test_null_forms(text):
    """Resolve the YAML 1.1 null spellings (and empty) to None."""
    assert load11(text) is None


def test_octal_c_style():
    """Resolve C-style octal 0777 to its decimal value under OPT_YAML_1_1."""
    assert load11("0777") == 511


def test_hex_integer():
    """Resolve hexadecimal integers under OPT_YAML_1_1."""
    assert load11("0xFF") == 255


def test_binary_integer():
    """Resolve binary integers under OPT_YAML_1_1."""
    assert load11("0b1010") == 10


def test_underscores_in_integer():
    """Strip underscores from integers under OPT_YAML_1_1."""
    assert load11("1_000_000") == 1000000


def test_negative_integer():
    """Resolve a signed negative integer under OPT_YAML_1_1."""
    assert load11("-42") == -42


def test_positive_signed_integer():
    """Resolve an explicitly positive-signed integer under OPT_YAML_1_1."""
    assert load11("+7") == 7


def test_sexagesimal_integer():
    """Resolve base-60 integers (190:20:30) under OPT_YAML_1_1."""
    assert load11("190:20:30") == 190 * 3600 + 20 * 60 + 30


def test_sexagesimal_simple():
    """Resolve a simple two-part sexagesimal integer (1:30 == 90)."""
    assert load11("1:30") == 90


def test_negative_sexagesimal():
    """Resolve a negative sexagesimal integer under OPT_YAML_1_1."""
    assert load11("-1:30") == -90


def test_underscores_in_float():
    """Strip underscores from floats under OPT_YAML_1_1."""
    assert load11("1_000.5") == 1000.5


@pytest.mark.parametrize("text", [".inf", ".Inf", ".INF", "+.inf"])
def test_positive_infinity(text):
    """Resolve positive-infinity spellings under OPT_YAML_1_1."""
    import math

    assert math.isinf(load11(text)) and load11(text) > 0


@pytest.mark.parametrize("text", ["-.inf", "-.Inf", "-.INF"])
def test_negative_infinity(text):
    """Resolve negative-infinity spellings under OPT_YAML_1_1."""
    import math

    assert math.isinf(load11(text)) and load11(text) < 0


@pytest.mark.parametrize("text", [".nan", ".NaN", ".NAN"])
def test_not_a_number(text):
    """Resolve not-a-number spellings under OPT_YAML_1_1."""
    import math

    assert math.isnan(load11(text))


def test_sexagesimal_float():
    """Resolve a base-60 float (1:30.5) under OPT_YAML_1_1."""
    assert load11("1:30.5") == 90.5


def test_scientific_float():
    """Resolve scientific-notation floats under OPT_YAML_1_1."""
    assert load11("1.2e3") == 1200.0


def test_invalid_sexagesimal_stays_string():
    """A sexagesimal with an out-of-range later segment stays a string."""
    assert load11("1:70:30") == "1:70:30"


def test_tagged_bool():
    """An explicit !!bool tag resolves a YAML 1.1 bool word."""
    assert yamlrocks.loads(b"x: !!bool yes", option=yamlrocks.OPT_YAML_1_1)["x"] is True


def test_tagged_int():
    """An explicit !!int tag resolves a YAML 1.1 integer."""
    assert yamlrocks.loads(b"x: !!int 0777", option=yamlrocks.OPT_YAML_1_1)["x"] == 511


def test_tagged_float():
    """An explicit !!float tag resolves a YAML 1.1 float."""
    assert yamlrocks.loads(b"x: !!float 1.5", option=yamlrocks.OPT_YAML_1_1)["x"] == 1.5


def test_tagged_null():
    """An explicit !!null tag resolves to None under OPT_YAML_1_1."""
    assert yamlrocks.loads(b"x: !!null ~", option=yamlrocks.OPT_YAML_1_1)["x"] is None


def test_quoted_yes_stays_string():
    """A quoted yes stays a string even under OPT_YAML_1_1."""
    assert load11('"yes"') == "yes"


# -- OPT_PYYAML_COMPAT: PyYAML's off-spec boolean set --------------------------

PYYAML = yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_PYYAML_COMPAT


def loadpy(text: str):
    return yamlrocks.loads(f"x: {text}".encode(), option=PYYAML)["x"]


def test_pyyaml_compat_drops_single_letter_booleans():
    """Bare y/Y/n/N stay strings under PyYAML-compat (PyYAML omits them)."""
    for value in ["y", "Y", "n", "N"]:
        assert loadpy(value) == value
        assert isinstance(loadpy(value), str)


def test_pyyaml_compat_keeps_word_booleans():
    """yes/no/on/off/true/false still resolve as booleans."""
    assert loadpy("yes") is True
    assert loadpy("on") is True
    assert loadpy("off") is False
    assert loadpy("no") is False


def test_yaml_1_1_alone_is_spec_correct():
    """Plain OPT_YAML_1_1 keeps bare y/n as booleans, per the real 1.1 spec."""
    assert load11("y") is True
    assert load11("n") is False


def test_pyyaml_compat_implies_1_1():
    """OPT_PYYAML_COMPAT works standalone: it implies the 1.1 schema."""
    out = yamlrocks.loads(b"a: yes\nb: 0777\n", option=yamlrocks.OPT_PYYAML_COMPAT)
    assert out == {"a": True, "b": 511}


def test_pyyaml_compat_carries_into_upgrade():
    """OPT_UPGRADE_1_1 under PyYAML-compat rewrites yes->true but leaves y."""
    doc = yamlrocks.loads(
        b"a: yes\nb: y\nc: 0777\n",
        option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_UPGRADE_1_1 | PYYAML,
    )
    # OPT_UPGRADE_1_1 stamps a %YAML 1.2 directive on the re-emitted document.
    assert doc.to_yaml() == b"%YAML 1.2\n---\na: true\nb: y\nc: 511\n"


def test_pyyaml_compat_carries_into_migration_warning(caplog):
    """The 1.1->1.2 warning under PyYAML-compat flags yes but not y."""
    with caplog.at_level("WARNING", logger="yamlrocks"):
        yamlrocks.loads(b"a: y\nb: yes\n", option=PYYAML | yamlrocks.OPT_YAML_1_1_WARN)
    messages = [r.message for r in caplog.records]
    assert any("'yes'" in m for m in messages)
    assert not any("'y'" in m for m in messages)


def test_merge_tag_marker_resolves_under_the_merge_skip_optimization():
    """An empty `!!merge`-tagged node still triggers the merge post-pass (1.1).

    The decoder skips the merge post-pass when no merge marker was produced; a
    marker created from the `!!merge` tag (not just a plain `<<`) must set that
    flag, or it would leak unresolved. Regression guard for that path.
    """
    assert yamlrocks.loads(b"x: !!merge\n", option=yamlrocks.OPT_YAML_1_1) == {
        "x": "<<"
    }


def test_leading_underscore_numbers_keep_1_1_behavior():
    """A leading-underscore scalar still resolves as a 1.1 number.

    The 1.1 parsers strip every underscore before parsing, so `_5` resolves to
    an int, `_1:30` to a sexagesimal int, and `_1.5` to a float (unusual, but the
    long-standing behavior). The fast-path first-byte gate must not reclassify
    them as strings. `_hello` is not a number and stays a string.
    """
    opt = yamlrocks.OPT_YAML_1_1
    assert yamlrocks.loads(b"a: _5", option=opt) == {"a": 5}
    assert yamlrocks.loads(b"a: _1:30", option=opt) == {"a": 90}
    assert yamlrocks.loads(b"a: _1.5", option=opt) == {"a": 1.5}
    assert yamlrocks.loads(b"a: _hello", option=opt) == {"a": "_hello"}
