"""Scalar resolution edge cases for the YAML 1.2 core schema."""

from __future__ import annotations

import math

import pytest

import yamlrocks


def load_value(text: str):
    """Load ``x: <text>`` and return the resolved value of ``x``."""
    return yamlrocks.loads(f"x: {text}".encode())["x"]


@pytest.mark.parametrize("text", ["null", "Null", "NULL", "~", ""])
def test_null_forms(text):
    """Resolve the various null spellings to None."""
    assert load_value(text) is None


@pytest.mark.parametrize("text", ["true", "True", "TRUE"])
def test_true_forms(text):
    """Resolve the various true spellings to True."""
    assert load_value(text) is True


@pytest.mark.parametrize("text", ["false", "False", "FALSE"])
def test_false_forms(text):
    """Resolve the various false spellings to False."""
    assert load_value(text) is False


@pytest.mark.parametrize(
    "text,expected",
    [("0", 0), ("42", 42), ("-17", -17), ("+5", 5), ("1000000", 1000000)],
)
def test_decimal_integers(text, expected):
    """Resolve decimal integer literals, including signs."""
    assert load_value(text) == expected


def test_hex_integer():
    """Resolve a hexadecimal integer literal."""
    assert load_value("0xFF") == 255


def test_octal_integer():
    """Resolve an octal integer literal."""
    assert load_value("0o17") == 15


@pytest.mark.parametrize(
    "text,expected",
    [("3.14", 3.14), ("-0.5", -0.5), ("1e3", 1000.0), ("2.5E-2", 0.025)],
)
def test_floats(text, expected):
    """Resolve float literals, including exponent notation."""
    assert load_value(text) == pytest.approx(expected)


def test_infinity():
    """Resolve positive and negative infinity literals."""
    assert load_value(".inf") == math.inf
    assert load_value("-.inf") == -math.inf


def test_nan():
    """Resolve the not-a-number literal to a NaN float."""
    assert math.isnan(load_value(".nan"))


@pytest.mark.parametrize("text", ["yes", "no", "on", "off", "y", "n"])
def test_yaml_11_booleans_are_strings_in_12(text):
    """Treat YAML 1.1 boolean words as plain strings under 1.2."""
    # In YAML 1.2 these are plain strings, not booleans.
    assert load_value(text) == text


def test_leading_zero_is_string():
    """Keep a decimal with a leading zero as a string under 1.2."""
    # 1.2 forbids leading zeros for decimals, so this stays a string.
    assert load_value("0123") == "0123"


def test_version_like_string():
    """Resolve a version-like dotted token as a string."""
    assert load_value("1.2.3") == "1.2.3"


def test_single_quoted_is_always_string():
    """Keep a single-quoted numeric scalar as a string."""
    assert load_value("'42'") == "42"


def test_double_quoted_is_always_string():
    """Keep a double-quoted boolean word as a string."""
    assert load_value('"true"') == "true"


def test_double_quote_escapes():
    """Decode escape sequences inside a double-quoted scalar."""
    assert yamlrocks.loads(rb'x: "a\tb\nc"')["x"] == "a\tb\nc"


def test_backslash_literal_tab_is_rejected():
    """A backslash followed by a literal tab is not a valid escape (only `\\t` is).

    The spec and the YAML test suite reject it; it used to be silently accepted
    as a tab.
    """
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="escape"):
        yamlrocks.loads(b'x: "a\\\tb"')


def test_unicode_escape():
    """Decode a Unicode character in a double-quoted scalar."""
    assert yamlrocks.loads(r'x: "é"'.encode())["x"] == "é"


def test_single_quote_doubling():
    """Decode a doubled single quote inside a single-quoted scalar."""
    assert yamlrocks.loads(b"x: 'it''s'")["x"] == "it's"


def test_unicode_content():
    """Preserve Unicode content in plain scalar values."""
    assert yamlrocks.loads("name: café ☕".encode())["name"] == "café ☕"


@pytest.mark.parametrize(
    "encoding",
    ["utf-8", "utf-16", "utf-16-le", "utf-16-be", "utf-32", "utf-32-le", "utf-32-be"],
)
def test_utf16_and_utf32_input_is_transcoded(encoding):
    """UTF-16 and UTF-32 input (with or without a BOM) is accepted, per spec.

    Previously only UTF-8 was accepted: a BOM-less UTF-16 stream silently
    mis-parsed into a garbage string and a BOM-prefixed one raised a confusing
    "invalid UTF-8". The encoding is now detected and transcoded.
    """
    src = "k: héllo ☃\nlist:\n- 1\n- 2\n"
    assert yamlrocks.loads(src.encode(encoding)) == {"k": "héllo ☃", "list": [1, 2]}


def test_odd_length_utf16_raises_clear_error():
    """A truncated UTF-16 stream raises a clear, encoding-specific error."""
    with pytest.raises(ValueError, match="UTF-16"):
        yamlrocks.loads(b"\xff\xfe\x61\x00\x3a")


def test_literal_block_scalar():
    """Parse a literal block scalar preserving newlines."""
    assert yamlrocks.loads(b"x: |\n  line1\n  line2\n")["x"] == "line1\nline2\n"


def test_folded_block_scalar():
    """Parse a folded block scalar joining lines with spaces."""
    assert yamlrocks.loads(b"x: >\n  line1\n  line2\n")["x"] == "line1 line2\n"


def test_literal_strip_chomping():
    """Parse a literal block scalar with strip chomping."""
    assert yamlrocks.loads(b"x: |-\n  line1\n  line2\n")["x"] == "line1\nline2"


def test_multiline_plain_scalar_folds():
    """A plain scalar folds its continuation lines with single spaces."""
    assert yamlrocks.loads(b"d: foo bar\n  baz qux\n")["d"] == "foo bar baz qux"


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        (b"k: a b c\n", "a b c"),  # internal spaces are content
        (b"k: word1 word2  word3\n", "word1 word2  word3"),  # runs of spaces kept
        (b"k: trailing spaces here   \n", "trailing spaces here"),  # trailing stripped
        (b"k: value   # comment\n", "value"),  # ` #` starts a comment
        (b"k: a#b c\n", "a#b c"),  # `#` not after a blank is content
        (b"k: a:b c\n", "a:b c"),  # `:` not before a blank is content
        (b"k: http://example.com/p\n", "http://example.com/p"),  # URL colons
    ],
)
def test_single_line_plain_scalar_with_internal_spaces(src, expected):
    """A multi-word plain scalar keeps internal spaces, strips trailing ones, and
    still honors the ` #` comment and `: ` key boundaries.

    Regression guard for the single-line fast path that consumes a whole run of
    content-and-blanks in one pass rather than restarting at every space.
    """
    assert yamlrocks.loads(src)["k"] == expected
    # The same holds inside a flow collection.
    assert yamlrocks.loads(b"[a b, c d]") == ["a b", "c d"]


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        (b"d: foo bar\n  'quoted' tail\n", "foo bar 'quoted' tail"),
        (b'd: foo bar\n  "quoted" tail\n', 'foo bar "quoted" tail'),
        (b"d: only\n  'quoted'\n", "only 'quoted'"),
    ],
)
def test_multiline_plain_scalar_continuation_starting_with_quote(text, expected):
    """A plain scalar continuation line may start with a quote.

    In block context ``'`` and ``"`` are ordinary plain-scalar characters - the
    first-character restriction only applies to the scalar's start, not its
    continuation lines - so the scalar keeps folding instead of being misread as
    a new quoted node. Regression for a real Home Assistant blueprint.
    """
    assert yamlrocks.loads(text)["d"] == expected


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        (b'[aaa\n  "bbb", ccc]\n', ['aaa "bbb"', "ccc"]),
        (b"[aaa\n  'bbb']\n", ["aaa 'bbb'"]),
        (b"[aaa\n  |bbb]\n", ["aaa |bbb"]),
        (b"[aaa\n  >bbb]\n", ["aaa >bbb"]),
    ],
)
def test_flow_plain_scalar_continuation_starting_with_quote_or_block_indicator(
    text, expected
):
    """In a flow collection a plain scalar continuation may start with `'`, `"`,
    `|`, or `>`: none are flow indicators, so they fold into the scalar (only the
    real flow indicators `{`/`[` open a new node). Matches PyYAML and the spec;
    regression for a real Ansible (dev-sec) config.
    """
    assert yamlrocks.loads(text) == expected


def test_indented_dashes_continuation_folds_not_a_document_marker():
    """An indented `--- ` (or `...`) on a plain-scalar continuation line is
    ordinary content, not a document marker: a marker is only `---`/`...` at
    column 0. Regression for a real flux2 CRD description that wraps onto a line
    starting `--- Many .condition.type values ...`.
    """
    src = (
        b"k:\n"
        b"  description: type of condition.\n"
        b"    --- Many values are consistent\n"
        b"    like Available\n"
        b"  maxLength: 316\n"
    )
    data = yamlrocks.loads(src)
    assert data["k"]["description"] == (
        "type of condition. --- Many values are consistent like Available"
    )
    assert data["k"]["maxLength"] == 316
    # And round-trip preserves it byte-for-byte.
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


def test_column_zero_dashes_still_end_the_document():
    """A `---` at column 0 is still a real document marker (the fix above must not
    swallow it), so a multi-line plain scalar ends and a new document begins."""
    docs = yamlrocks.loads_all(b"foo\nbar\n---\nbaz\n")
    assert docs == ["foo bar", "baz"]


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        (b"\xef\xbb\xbfname: value\n", {"name": "value"}),
        (b"\xef\xbb\xbf# comment\nname: value\n", {"name": "value"}),
        (b"\xef\xbb\xbf- a\n- b\n", ["a", "b"]),
        (b"\xef\xbb\xbf---\na: 1\n", {"a": 1}),
        (b"\xef\xbb\xbf", None),
    ],
)
def test_leading_byte_order_mark_is_stripped(src, expected):
    """A UTF-8 BOM at the start of the stream is encoding metadata, not content:
    it is skipped so the first scalar/key does not carry a stray ``﻿`` and a
    ``#`` right after it still opens a comment. Matches PyYAML; regression for real
    Helm `frobnitz_with_bom` test charts.
    """
    assert yamlrocks.loads(src) == expected


@pytest.mark.parametrize(
    "src",
    [
        b"\xef\xbb\xbfname: value\n",
        b"\xef\xbb\xbf# comment\nname: value\n",
        b"\xef\xbb\xbf- a\n- b\n",
        b"\xef\xbb\xbf",
    ],
)
def test_leading_byte_order_mark_round_trips(src):
    """Round-trip preserves a leading BOM byte-for-byte: it is restored at the head
    of the stream on re-emission so the document stays identical on disk."""
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


@pytest.mark.parametrize(
    "text",
    [
        "100000000000000000000000000000000",
        "-99999999999999999999999999",
        "9223372036854775808",  # i64::MAX + 1
        "-9223372036854775809",  # i64::MIN - 1
    ],
)
def test_big_integer_literals_load_as_int(text):
    """An integer literal too large for i64 loads as a Python int, not a string."""
    value = yamlrocks.loads(f"x: {text}".encode())["x"]
    assert isinstance(value, int)
    assert value == int(text)


def test_big_integer_round_trip_assignment():
    """Assigning a big int to a round-trip document keeps it an int."""
    doc = yamlrocks.loads(b"x: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
    doc["x"] = 10**40
    assert doc.to_yaml() == f"x: {10**40}\n".encode()
    assert (
        yamlrocks.loads(doc.to_yaml(), option=yamlrocks.OPT_ROUND_TRIP)["x"] == 10**40
    )


@pytest.mark.parametrize(
    ("text", "expected"),
    [
        ("10_000_000_000_000_000_000", 10_000_000_000_000_000_000),  # i64 overflow
        ("1_000_000_000_000_000_000_000_000", 10**24),
        ("123_456", 123_456),  # fits i64, still an int
        ("-9_999_999_999_999_999_999", -9_999_999_999_999_999_999),
    ],
)
def test_yaml_11_underscored_big_integer_loads_as_int(text, expected):
    """A YAML 1.1 underscored integer past i64 stays an int, like its plain twin.

    The big-int fallback used to ignore underscore separators, so an overflowing
    value such as `10_000_000_000_000_000_000` silently became a string while the
    same number without underscores correctly loaded as an int.
    """
    value = yamlrocks.loads(text.encode(), option=yamlrocks.OPT_YAML_1_1)
    assert isinstance(value, int) and not isinstance(value, bool)
    assert value == expected
