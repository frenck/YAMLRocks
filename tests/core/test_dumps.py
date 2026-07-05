"""Core dumping behaviour and load/dump round-tripping for ``yamlrocks.dumps``."""

from __future__ import annotations

import pytest

import yamlrocks


def test_dumps_returns_bytes():
    """Dump returns a bytes object."""
    assert isinstance(yamlrocks.dumps({"a": 1}), bytes)


def test_dumps_simple_mapping():
    """Dump a simple mapping to YAML."""
    assert yamlrocks.dumps({"key": "value"}) == b"key: value\n"


def test_dumps_sequence():
    """Dump a sequence to a block-style YAML list."""
    assert yamlrocks.dumps([1, 2, 3]) == b"- 1\n- 2\n- 3\n"


def test_dumps_nested_has_no_trailing_space():
    """Dump nested blocks without leaving a space after the colon."""
    # Regression: block children must not leave a space after the colon.
    out = yamlrocks.dumps({"a": {"b": [1, 2]}})
    assert out == b"a:\n  b:\n    - 1\n    - 2\n"
    assert b": \n" not in out


def test_dumps_scalar_types():
    """Dump int, float, bool, and None scalar values (None is empty by default)."""
    out = yamlrocks.dumps({"i": 1, "f": 1.5, "b": True, "n": None})
    assert out == b"i: 1\nf: 1.5\nb: true\nn:\n"


def test_dumps_utf8_bytes_as_string():
    """Valid UTF-8 ``bytes`` serialize as the decoded string."""
    assert yamlrocks.dumps({"k": "héllo".encode()}) == b"k: h\xc3\xa9llo\n"


def test_dumps_invalid_utf8_bytes_is_rejected():
    """Invalid UTF-8 ``bytes`` raise rather than silently corrupting data."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="not valid UTF-8"):
        yamlrocks.dumps({"k": b"\xff\xfe"})


@pytest.mark.parametrize(
    "obj",
    [
        "\ud800",  # bare lone surrogate
        b"file\xff.txt".decode("utf-8", "surrogateescape"),  # surrogateescape
        {b"k\xff".decode("utf-8", "surrogateescape"): 1},  # as a mapping key
        ["\udcff"],  # inside a sequence
    ],
)
def test_dumps_lone_surrogate_str_is_rejected(obj):
    """A str with an unpaired surrogate raises instead of being replaced by U+FFFD."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="surrogate"):
        yamlrocks.dumps(obj)


def test_dumps_empty_collections_use_flow():
    """Dump empty mappings and sequences using flow style."""
    assert yamlrocks.dumps({"a": {}, "b": []}) == b"a: {}\nb: []\n"


@pytest.mark.parametrize(
    "obj",
    [
        {"a": 1, "b": [1, 2, 3]},
        {"server": {"host": "localhost", "ports": [80, 443]}},
        [{"name": "alice", "age": 30}, {"name": "bob"}],
        [[1, 2], [3, 4]],
        {"nested": {"deeply": {"value": [True, False, None]}}},
        "just a scalar",
        [1, "two", 3.0, True, None],
    ],
)
def test_round_trip_load_dump(obj):
    """Round-trip an object through dump then load unchanged."""
    assert yamlrocks.loads(yamlrocks.dumps(obj)) == obj


def test_dumps_sort_keys():
    """Dump mapping keys in sorted order with OPT_SORT_KEYS."""
    out = yamlrocks.dumps({"c": 1, "a": 2, "b": 3}, option=yamlrocks.OPT_SORT_KEYS)
    assert out == b"a: 2\nb: 3\nc: 1\n"


def test_dumps_sort_keys_orders_non_string_keys():
    """OPT_SORT_KEYS sorts non-string keys too, not only strings: integers sort
    numerically (like PyYAML), and mixed keys group by type (null, bool, number,
    string) rather than leaving non-string keys in insertion order."""
    assert yamlrocks.dumps(
        {3: "a", 1: "b", 10: "c", 2: "d"}, option=yamlrocks.OPT_SORT_KEYS
    ) == (b"1: b\n2: d\n3: a\n10: c\n")
    assert (
        yamlrocks.dumps(
            {"z": 1, 2: 2, True: 3, None: 4, "a": 5}, option=yamlrocks.OPT_SORT_KEYS
        )
        == b"null: 4\ntrue: 3\n2: 2\na: 5\nz: 1\n"
    )
    # Big integers (past i64) sort by magnitude, interleaved with small ints.
    big = {30000000000000000000: "c", 5: "small", 20000000000000000000: "b"}
    assert yamlrocks.dumps(big, option=yamlrocks.OPT_SORT_KEYS) == (
        b"5: small\n20000000000000000000: b\n30000000000000000000: c\n"
    )


def test_dumps_indent_4():
    """Dump nested blocks with four-space indentation via OPT_INDENT_4."""
    out = yamlrocks.dumps({"a": {"b": 1}}, option=yamlrocks.OPT_INDENT_4)
    assert out == b"a:\n    b: 1\n"


def test_dumps_flow_style():
    """Dump collections using flow style via OPT_FLOW_STYLE."""
    out = yamlrocks.dumps({"a": [1, 2], "b": {"c": 3}}, option=yamlrocks.OPT_FLOW_STYLE)
    assert out == b"{a: [1, 2], b: {c: 3}}\n"


@pytest.mark.parametrize(
    "value",
    [
        {"a": ["x]y", "p,q", "m{n", "o}p"]},  # flow-indicator chars mid-string
        {"a": ["plain", "with]bracket"]},
        {("key]with]brackets",): "v"},  # complex (tuple) key emitted in flow
    ],
)
def test_dumps_flow_quotes_strings_with_flow_indicators(value):
    """A string containing a flow indicator (`,[]{}`) inside a flow collection is
    quoted, so the `]`/`,` does not end the entry or collection early and the
    document round-trips. (Complex keys are always emitted inline in flow style.)"""
    out = yamlrocks.dumps(value, option=yamlrocks.OPT_FLOW_STYLE)
    assert yamlrocks.loads(out) == value


def test_dumps_block_does_not_over_quote_flow_indicators():
    """In block context `,`/`]` etc. are ordinary content, so a block value or key
    carrying one stays unquoted (no needless quoting)."""
    assert yamlrocks.dumps({"desc": "foo, bar"}) == b"desc: foo, bar\n"
    assert yamlrocks.dumps({"k": "a]b"}) == b"k: a]b\n"


def test_dumps_explicit_start_and_end():
    """Dump with explicit document start and end markers."""
    out = yamlrocks.dumps(
        {"a": 1}, option=yamlrocks.OPT_EXPLICIT_START | yamlrocks.OPT_EXPLICIT_END
    )
    assert out == b"---\na: 1\n...\n"


def test_dumps_quotes_ambiguous_strings():
    """Quote strings that would otherwise resolve to non-string scalars."""
    # "yes" would be a YAML 1.1 boolean, so it is quoted for stability.
    # Double quotes are the default.
    out = yamlrocks.dumps({"x": "yes", "y": "true", "z": "null"})
    assert b'"yes"' in out
    assert b'"true"' in out
    assert b'"null"' in out


@pytest.mark.parametrize(
    "value",
    [
        "...",  # document-end marker: unquoted, it reparses to null
        "﻿",  # leading BOM: stripped by the scanner if unquoted
        "﻿abc",
        "\n",  # all-newline: a clip block scalar would chomp it away
        "\n\n",
        "a\n",
    ],
)
def test_dumps_injection_prone_strings_round_trip(value):
    """Strings that could break out of their scalar position survive a round-trip.

    Covers the document-end marker ``...``, a leading byte order mark, and an
    all-newline string (which a clip block scalar would silently drop).
    """
    assert yamlrocks.loads(yamlrocks.dumps(value)) == value
    assert yamlrocks.loads(yamlrocks.dumps({"k": value})) == {"k": value}
    assert yamlrocks.loads(yamlrocks.dumps([value])) == [value]


def test_dumps_single_quotes_opt_in():
    """``OPT_SINGLE_QUOTES`` switches the quote character to single quotes."""
    out = yamlrocks.dumps({"x": "yes", "y": "true"}, option=yamlrocks.OPT_SINGLE_QUOTES)
    assert b"'yes'" in out
    assert b"'true'" in out
    assert b'"' not in out


def test_dumps_quotes_numeric_strings():
    """Quote numeric-looking strings so they round-trip as strings."""
    out = yamlrocks.dumps({"version": "1.0"})
    assert yamlrocks.loads(out) == {"version": "1.0"}


def test_dumps_multiline_strings_default_to_literal_block():
    """A multi-line string is emitted as a literal block scalar by default."""
    out = yamlrocks.dumps({"text": "line1\nline2\n"})
    assert out == b"text: |\n  line1\n  line2\n"


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        ("a\nb\n", b"k: |\n  a\n  b\n"),  # clip (one trailing newline)
        ("a\nb", b"k: |-\n  a\n  b\n"),  # strip (no trailing newline)
        ("a\nb\n\n", b"k: |+\n  a\n  b\n\n"),  # keep (extra blank line)
    ],
)
def test_dumps_literal_block_chomping(value, expected):
    """The chomping indicator is chosen so the block round-trips exactly."""
    out = yamlrocks.dumps({"k": value})
    assert out == expected
    assert yamlrocks.loads(out)["k"] == value


@pytest.mark.parametrize(
    "value",
    [
        "has\r\ncrlf\n",  # carriage return cannot be in a block scalar
        "  indented\n  first line\n",  # leading whitespace ambiguity
        "ctrl\x01char\nhere\n",  # control character
    ],
)
def test_dumps_unblockable_multiline_falls_back_to_double_quotes(value):
    """A multi-line string a literal block cannot represent uses double quotes,
    and still round-trips exactly."""
    out = yamlrocks.dumps({"k": value})
    assert out.startswith(b'k: "')
    assert yamlrocks.loads(out)["k"] == value


def test_dumps_default_callback():
    """Use the default callback to serialize an unsupported object."""

    class Custom:
        pass

    out = yamlrocks.dumps({"v": Custom()}, default=lambda o: "custom")
    assert yamlrocks.loads(out) == {"v": "custom"}


def test_dumps_unserializable_raises():
    """Raise TypeError when dumping an unserializable object without a default."""
    with pytest.raises(TypeError):
        yamlrocks.dumps({"v": object()})


def test_dumps_null_style_default_is_empty():
    """By default a None is emitted as an empty node in block positions."""
    assert yamlrocks.dumps({"a": None, "list": [1, None]}) == b"a:\nlist:\n  - 1\n  -\n"


def test_dumps_opt_null_as_keyword_flag():
    """``OPT_NULL_AS_KEYWORD`` selects the explicit ``null`` keyword."""
    out = yamlrocks.dumps({"a": None}, option=yamlrocks.OPT_NULL_AS_KEYWORD)
    assert out == b"a: null\n"


@pytest.mark.parametrize(
    ("obj", "expected"),
    [
        (None, b"null\n"),  # top-level scalar: an empty document would be ambiguous
        ({None: 1}, b"null: 1\n"),  # mapping key
    ],
)
def test_dumps_null_style_empty_falls_back_where_ambiguous(obj, expected):
    """The default empty null is only used where it reads back unambiguously;
    elsewhere (top-level scalar, flow, a key) it falls back to the ``null``
    keyword."""
    assert yamlrocks.dumps(obj) == expected


def test_dumps_null_style_empty_flow_falls_back():
    """In a flow collection an empty entry is invalid, so ``null`` is used."""
    out = yamlrocks.dumps({"a": [None, 1]}, option=yamlrocks.OPT_FLOW_STYLE)
    assert out == b"{a: [null, 1]}\n"


@pytest.mark.parametrize(
    "option",
    [None, yamlrocks.OPT_NULL_AS_KEYWORD, yamlrocks.OPT_NULL_AS_TILDE],
)
def test_dumps_null_style_always_round_trips_to_none(option):
    """Every null style parses back to None, so the choice is purely cosmetic."""
    data = {"a": None, "b": [None, {"c": None}]}
    assert yamlrocks.loads(yamlrocks.dumps(data, option=option)) == data


def test_dumps_opt_null_as_tilde_flag():
    """``OPT_NULL_AS_TILDE`` selects the ``~`` indicator as the default."""
    out = yamlrocks.dumps({"a": None, "l": [None]}, option=yamlrocks.OPT_NULL_AS_TILDE)
    assert out == b"a: ~\nl:\n  - ~\n"


def test_dumps_null_style_flags_are_mutually_exclusive():
    """Setting both null-style flags is a ValueError."""
    with pytest.raises(ValueError, match="mutually exclusive"):
        yamlrocks.dumps(
            {"a": None},
            option=yamlrocks.OPT_NULL_AS_KEYWORD | yamlrocks.OPT_NULL_AS_TILDE,
        )


def test_option_mask_uses_more_than_32_bits():
    """At least one flag lives above bit 31, exercising the 64-bit option mask."""
    flags = [
        value
        for name, value in vars(yamlrocks).items()
        if name.startswith("OPT_") and isinstance(value, int)
    ]
    assert max(flags) >= 1 << 32
    # A high-bit flag still composes with a low-bit one.
    out = yamlrocks.dumps(
        {"b": 1, "a": None},
        option=yamlrocks.OPT_NULL_AS_TILDE | yamlrocks.OPT_SORT_KEYS,
    )
    assert out == b"a: ~\nb: 1\n"


def test_dumps_indentless_sequences():
    """OPT_INDENTLESS_SEQUENCES aligns sequence dashes with their key."""
    data = {"key": [1, 2], "nested": {"inner": ["a"]}}
    assert yamlrocks.dumps(data) == b"key:\n  - 1\n  - 2\nnested:\n  inner:\n    - a\n"
    out = yamlrocks.dumps(data, option=yamlrocks.OPT_INDENTLESS_SEQUENCES)
    assert out == b"key:\n- 1\n- 2\nnested:\n  inner:\n  - a\n"
    # Either way parses back to the same value.
    assert yamlrocks.loads(out) == data


def test_dumps_indentless_sequence_of_mappings():
    """An indentless sequence of mappings keeps the dash at the key column."""
    out = yamlrocks.dumps(
        {"items": [{"a": 1, "b": 2}]}, option=yamlrocks.OPT_INDENTLESS_SEQUENCES
    )
    assert out == b"items:\n- a: 1\n  b: 2\n"


def test_dumps_cyclic_structure_raises_not_crashes():
    """A self-referential object raises instead of overflowing the stack."""
    d: dict = {}
    d["self"] = d
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="deeply nested"):
        yamlrocks.dumps(d)
    items: list = []
    items.append(items)
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="deeply nested"):
        yamlrocks.dumps(items)


def test_dumps_excessively_deep_structure_raises():
    """A structure nested past the recursion limit raises a clean error."""
    deep: dict = {}
    cur = deep
    for _ in range(5000):
        nxt: dict = {}
        cur["k"] = nxt
        cur = nxt
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="deeply nested"):
        yamlrocks.dumps(deep)


@pytest.mark.parametrize(
    "value",
    [
        "a\nb\n",
        "a\nb",
        "a\nb\n\n",
        "a\n\nb\n",
        "trailing \nspace\n",
        "has\r\ncrlf\n",
        "  indented first\n  line\n",
        "tab\there\nand newline\n",
        "\nleading blank\n",
        "ends no newline\nlast",
        "many\ntrailing\n\n\n",
        "ctrl\x01char\nhere\n",
        "café\nrésumé\n",
        "#!/bin/sh\necho hi\n",
        "embedded: yaml\nkey: value\n",
        "just\ttabs no newline",
        "single line",
        "",
    ],
)
def test_dumps_multiline_strings_round_trip_exactly(value):
    """Every multi-line string round-trips byte-for-value, whether it emits as a
    literal block or falls back to double quotes."""
    assert yamlrocks.loads(yamlrocks.dumps({"k": value}))["k"] == value


@pytest.mark.parametrize(
    "value", [2**63, 10**30, -(10**25), 2**100, 9223372036854775808]
)
def test_dumps_big_integers_beyond_i64(value):
    """A Python int too large for i64 dumps as an exact integer, not a crash or
    a float, and round-trips."""
    out = yamlrocks.dumps({"x": value})
    assert out == f"x: {value}\n".encode()
    assert yamlrocks.loads(out)["x"] == value


def test_dumps_big_integer_key_and_to_json():
    """Big integers work as mapping keys and in JSON output."""
    assert yamlrocks.dumps({10**30: "v"}) == f"{10**30}: v\n".encode()
    assert yamlrocks.to_json({"x": 10**30}) == f'{{"x":{10**30}}}'.encode()


def test_dumps_big_int_subclass_emits_true_digits():
    """An int subclass whose value exceeds i64 serializes its real digits.

    The big-int path must not trust `str(obj)` (a subclass can override
    `__str__`, and an IntEnum's repr leaks through it on some Pythons); it reduces
    to a true base int first, so the exact value is emitted as valid YAML/JSON.
    """
    from enum import IntEnum, IntFlag

    class Wrapped(int):
        def __str__(self):  # would corrupt a str()-based encoder
            return "HACKED"

    class Big(IntEnum):
        X = 10**25

    class Flags(IntFlag):
        HIGH = 1 << 70

    for obj in (Wrapped(10**30), Big.X, Flags.HIGH):
        digits = str(int(obj)).encode()
        assert yamlrocks.dumps(obj) == digits + b"\n"
        assert yamlrocks.to_json(obj) == digits
        assert yamlrocks.loads(yamlrocks.dumps(obj)) == int(obj)


@pytest.mark.parametrize(
    ("value", "expected"),
    [
        (1e308, b"x: 1.0e+308\n"),
        (6.022e23, b"x: 6.022e+23\n"),
        (1e-10, b"x: 1.0e-10\n"),
        (1e-5, b"x: 1.0e-05\n"),
        (1e15, b"x: 1000000000000000.0\n"),  # below the threshold: decimal
        (1e16, b"x: 1.0e+16\n"),  # at the threshold: scientific
        (3.14, b"x: 3.14\n"),
    ],
)
def test_dumps_float_scientific_notation(value, expected):
    """Large/small floats use scientific notation (matching PyYAML); all the
    forms round-trip back to the same float."""
    out = yamlrocks.dumps({"x": value})
    assert out == expected
    assert yamlrocks.loads(out)["x"] == value


def test_dumps_quotes_merge_key_string():
    """A literal '<<' key is quoted so it does not reparse as a merge key."""
    out = yamlrocks.dumps({"<<": "x", "k": 1})
    assert b'"<<"' in out or b"'<<'" in out
    assert yamlrocks.loads(out) == {"<<": "x", "k": 1}


def test_dumps_escapes_control_characters():
    """Control characters are quoted and escaped, not emitted raw (a raw control
    byte makes YAML a spec-compliant reader rejects)."""
    for value in ["a\x01b", "tab\x07bell", "del\x7fhere"]:
        out = yamlrocks.dumps(value)
        assert b"\\x" in out
        assert yamlrocks.loads(out) == value


@pytest.mark.parametrize(
    "value",
    [
        "0777",  # YAML 1.1 C-style octal
        "0x1f",  # hex
        "0o17",  # YAML 1.2 octal
        "0b101",  # binary
        "1_000",  # underscored int
        ".inf",  # special float
        "12",  # plain int
    ],
)
def test_dumps_keeps_number_looking_strings_as_strings(value):
    """A string either schema would read as a number is quoted, so it stays a
    string when re-loaded under YAML 1.2 or 1.1. (Sexagesimal `1:30` is a 1.1-only
    form left unquoted by design, so a `datetime.time` stays an unquoted timestamp
    literal; see needs_quoting.)"""
    out = yamlrocks.dumps(value)
    assert yamlrocks.loads(out) == value
    assert yamlrocks.loads(out, option=yamlrocks.OPT_YAML_1_1) == value


def test_dumps_accepts_document_view():
    """dumps serializes a YAMLRocksDocumentView (a sub-tree), like to_json does."""
    doc = yamlrocks.loads(
        b"a:\n  b:\n    c: 1\n    d: 2\n", option=yamlrocks.OPT_ROUND_TRIP
    )
    assert yamlrocks.loads(yamlrocks.dumps(doc["a"]["b"])) == {"c": 1, "d": 2}


# -- YAML 1.1 dump mode (OPT_YAML_1_1 / OPT_PYYAML_COMPAT on dumps) -----------
# Targeting a 1.1 schema quotes the scalars only that schema reads as non-strings
# (bare `y`/`n` booleans under strict 1.1; sexagesimal `1:30` and leading-
# underscore `_5` under both 1.1 variants), so the output re-reads identically
# under it. The 1.2 default leaves them bare.

Y11 = yamlrocks.OPT_YAML_1_1
PYC = yamlrocks.OPT_PYYAML_COMPAT


@pytest.mark.parametrize("value", ["y", "Y", "n", "N", "1:30", "10:20:30", "_5"])
def test_dumps_yaml_1_1_quotes_its_ambiguities_but_default_does_not(value):
    """Strict 1.1 quotes a string it reads as a non-string; the 1.2 default does not."""
    assert b'"' not in yamlrocks.dumps({"k": value})
    assert b'"' in yamlrocks.dumps({"k": value}, option=Y11)


@pytest.mark.parametrize("schema", [Y11, PYC])
@pytest.mark.parametrize("value", ["1:30", "10:20:30", "_5", "yes", "0777", "on"])
def test_dumps_1_1_output_round_trips_under_its_schema(schema, value):
    """1.1 and PyYAML-compat dump output re-reads identically under the same schema."""
    obj = {"k": value}
    assert yamlrocks.loads(yamlrocks.dumps(obj, option=schema), option=schema) == obj


def test_dumps_pyyaml_compat_keeps_bare_y_n_but_quotes_sexagesimal():
    """PyYAML-compat leaves bare `y`/`n` (not bools for it) but quotes sexagesimal (it reads it)."""
    assert yamlrocks.dumps({"k": "y"}, option=PYC) == b"k: y\n"
    assert yamlrocks.dumps({"k": "1:30"}, option=PYC) == b'k: "1:30"\n'


@pytest.mark.parametrize("schema", [Y11, PYC])
@pytest.mark.parametrize(
    "value",
    [
        "2020-01-02",
        "2020-01-02T10:00:00",
        "2020-01-02 10:00:00.5Z",
        "2001-12-15 2:59:43.10",
    ],
)
def test_dumps_1_1_quotes_timestamp_strings(schema, value):
    """A 1.1 reader reads a timestamp as a date, so a 1.1-targeted dump quotes it; 1.2 leaves it bare."""
    assert b'"' in yamlrocks.dumps({"k": value}, option=schema)
    assert b'"' not in yamlrocks.dumps({"k": value})


def test_dumps_1_1_does_not_over_quote_non_timestamps():
    """A single-digit-field date (`2020-1-2`) is a plain string to a 1.1 reader too, so it is not quoted."""
    assert yamlrocks.dumps({"k": "2020-1-2"}, option=PYC) == b"k: 2020-1-2\n"


def test_dumps_default_is_unchanged_by_the_1_1_dump_work():
    """The 1.2 default output is untouched: a 1.1-only ambiguity stays bare."""
    assert yamlrocks.dumps({"mac": "01:02:03"}) == b"mac: 01:02:03\n"
    assert yamlrocks.dumps({"k": "_5"}) == b"k: _5\n"
