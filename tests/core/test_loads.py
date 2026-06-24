"""Core loading behaviour: ``yamlrocks.loads`` and ``yamlrocks.loads_all``."""

from __future__ import annotations

import pytest

import yamlrocks


def test_simple_mapping():
    """Parse a simple single-key mapping into a dict."""
    assert yamlrocks.loads(b"key: value") == {"key": "value"}


def test_multi_key_mapping():
    """Parse a mapping with multiple keys into a dict."""
    assert yamlrocks.loads(b"a: 1\nb: 2\nc: 3") == {"a": 1, "b": 2, "c": 3}


def test_nested_mapping():
    """Parse nested mappings into nested dicts."""
    assert yamlrocks.loads(b"a:\n  b:\n    c: 1") == {"a": {"b": {"c": 1}}}


def test_block_sequence():
    """Parse a block sequence into a list."""
    assert yamlrocks.loads(b"- 1\n- 2\n- 3") == [1, 2, 3]


def test_mapping_in_sequence():
    """Parse a sequence of mappings into a list of dicts."""
    src = b"- name: alice\n  age: 30\n- name: bob\n  age: 25"
    assert yamlrocks.loads(src) == [
        {"name": "alice", "age": 30},
        {"name": "bob", "age": 25},
    ]


def test_sequence_in_mapping():
    """Parse a mapping whose value is a block sequence."""
    assert yamlrocks.loads(b"items:\n  - 1\n  - 2\n") == {"items": [1, 2]}


def test_scalar_types():
    """Resolve int, float, bool, null, and string scalar types."""
    src = b"i: 42\nf: 3.14\nb: true\nn: null\ns: hello"
    assert yamlrocks.loads(src) == {
        "i": 42,
        "f": 3.14,
        "b": True,
        "n": None,
        "s": "hello",
    }


def test_flow_mapping():
    """Parse a flow-style mapping into a dict."""
    assert yamlrocks.loads(b"{a: 1, b: 2}") == {"a": 1, "b": 2}


def test_flow_mapping_bare_keys_are_null():
    """A flow-mapping entry with a key but no `:` has a null value, and must not
    absorb the next entry as its value (`{a, b}` is two null-valued keys)."""
    assert yamlrocks.loads(b"{a, b}") == {"a": None, "b": None}
    assert yamlrocks.loads(b"{a: 1, b, c: 3}") == {"a": 1, "b": None, "c": 3}
    # Same on the round-trip path.
    assert yamlrocks.loads(b"{a, b}", option=yamlrocks.OPT_ROUND_TRIP).to_dict() == {
        "a": None,
        "b": None,
    }


def test_flow_sequence():
    """Parse a flow-style sequence into a list."""
    assert yamlrocks.loads(b"[1, 2, 3]") == [1, 2, 3]


def test_flow_nested():
    """Parse nested flow-style collections."""
    assert yamlrocks.loads(b"{a: [1, 2], b: {c: 3}}") == {"a": [1, 2], "b": {"c": 3}}


@pytest.mark.parametrize("src", [b"[a, b}", b"[}", b"[a}", b"{a: b]", b"[[[}]]", b"{]"])
def test_mismatched_flow_brackets_are_rejected(src):
    """A `]` must close a `[` and a `}` a `{`; a mismatch is invalid YAML.

    Previously the closing indicator was accepted without checking the opener,
    so `[a, b}` silently parsed as `['a', 'b']`.
    """
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="mismatched"):
        yamlrocks.loads(src)


def test_empty_document_is_none():
    """Parse an empty document as None."""
    assert yamlrocks.loads(b"") is None


def test_bare_scalar_document():
    """Parse a bare scalar document as a string."""
    assert yamlrocks.loads(b"hello") == "hello"


def test_empty_indentless_sequence_entry_before_a_sibling_key():
    """An empty indentless `-` entry keeps its null when a sibling key follows.

    The dash sits at the mapping key's column, so dedenting to the sibling key
    `q` emits a `Key` event with no `BlockEnd` (the sequence shares the mapping's
    block level). The empty-entry detection missed `Key` and dropped the null, so
    the sequence came back empty. Pins it to `[None]`, matching the indented form
    `9:\n  -\nq:`.
    """
    src = b"9:\n-\nq:\n"
    assert yamlrocks.loads(src) == {9: [None], "q": None}
    # The round-trip composer has the same empty-entry rule and must keep the
    # entry too, so an unmodified document still re-emits byte-for-byte.
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


def test_str_input_accepted():
    """Accept a str input and parse it."""
    assert yamlrocks.loads("key: value") == {"key": "value"}


def test_bytearray_input_accepted():
    """Accept a bytearray input and parse it."""
    assert yamlrocks.loads(bytearray(b"key: value")) == {"key": "value"}


def test_invalid_type_raises():
    """Raise TypeError for an unsupported input type."""
    with pytest.raises(TypeError):
        yamlrocks.loads(123)


def test_loads_all_multiple_documents():
    """Parse a multi-document stream into a list of documents."""
    docs = yamlrocks.loads_all(b"---\na: 1\n---\nb: 2")
    assert docs == [{"a": 1}, {"b": 2}]


def test_loads_all_single_document():
    """Parse a single-document stream into a one-element list."""
    assert yamlrocks.loads_all(b"a: 1\nb: 2") == [{"a": 1, "b": 2}]


def test_deeply_nested_does_not_crash():
    """Parse a deeply nested structure without crashing."""
    # Regression guard: nested structures must not trigger unbounded allocation.
    src = "a:\n" + "".join(f"{' ' * (i * 2)}n{i}:\n" for i in range(1, 20))
    src += f"{' ' * 40}leaf: 1\n"
    result = yamlrocks.loads(src.encode())
    assert isinstance(result, dict)


# -- Block sequence at the same indent as its parent mapping key ---------------


def test_sequence_value_at_mapping_key_indent():
    """A block sequence value indented level with the mapping key parses, and a
    following sibling entry is not mistaken for a key (regression: the implicit
    sequence must leave the mapping's BlockEnd for the mapping to consume)."""
    src = (
        b"tasks:\n"
        b"  - name: files\n"
        b"    value:\n"
        b"    - a\n"
        b"    - b\n"
        b"  - name: expr\n"
        b"    value: x\n"
    )
    assert yamlrocks.loads(src) == {
        "tasks": [
            {"name": "files", "value": ["a", "b"]},
            {"name": "expr", "value": "x"},
        ]
    }


def test_nested_same_indent_sequence_round_trips():
    """The same structure round-trips byte-for-byte through OPT_ROUND_TRIP."""
    src = b"a:\n  k:\n  - 1\n  - 2\nb: 3\n"
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.to_yaml() == src
    assert yamlrocks.loads(src) == {"a": {"k": [1, 2]}, "b": 3}


# -- Reserved indicators (@ and `) cannot start a plain scalar -----------------


@pytest.mark.parametrize("ch", ["@", "`"])
def test_reserved_indicator_cannot_start_plain_scalar(ch):
    """`@` and backtick are reserved; a leading one is a scan error (like PyYAML)."""
    with pytest.raises(yamlrocks.YAMLRocksParseError, match="reserved indicator"):
        yamlrocks.loads(f"{ch}: value\n".encode())


@pytest.mark.parametrize("ch", ["@", "`"])
def test_reserved_indicator_in_flow_context(ch):
    """The rejection applies in flow context too."""
    with pytest.raises(yamlrocks.YAMLRocksParseError, match="reserved indicator"):
        yamlrocks.loads(f"[{ch}foo]\n".encode())


def test_reserved_indicator_allowed_mid_scalar_and_quoted():
    """`@`/backtick are fine inside a scalar or when quoted."""
    assert yamlrocks.loads(b"email: foo@bar.com") == {"email": "foo@bar.com"}
    assert yamlrocks.loads(b'k: "@v"') == {"k": "@v"}
    assert yamlrocks.loads(b"k: a`b") == {"k": "a`b"}


# -- Collection (complex) mapping keys convert to a hashable Python key --------

# A mapping or sequence used as a mapping key cannot be a Python dict key as-is.
# Rather than rejecting such valid YAML, YAMLRocks renders the key as its hashable
# counterpart: a sequence becomes a tuple, a mapping a tuple of (key, value)
# pairs (order-preserving, so it survives a dumps/loads round-trip). Annotated
# mode does the same, so the two paths agree.


@pytest.mark.parametrize(
    "option",
    [None, yamlrocks.OPT_ANNOTATED],
    ids=["fast", "annotated"],
)
def test_sequence_key_becomes_a_tuple(option):
    """A sequence used as a mapping key resolves to a tuple key on both paths."""
    data = yamlrocks.loads(b"[1, 2]: value\n", option=option)
    assert data == {(1, 2): "value"}


@pytest.mark.parametrize(
    "option",
    [None, yamlrocks.OPT_ANNOTATED],
    ids=["fast", "annotated"],
)
def test_mapping_key_becomes_a_tuple(option):
    """A mapping used as a mapping key resolves to a tuple-of-pairs key on both
    paths (a tuple, not a frozenset, so it round-trips through dumps)."""
    data = yamlrocks.loads(b"? {a: 1}\n: value\n", option=option)
    assert data == {(("a", 1),): "value"}


def test_collection_key_agrees_between_fast_and_annotated():
    """The hashable key is identical whether or not OPT_ANNOTATED is set."""
    src = b"[1, 2]: x\n? {a: 1}\n: y\n"
    assert yamlrocks.loads(src) == yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)


@pytest.mark.parametrize(
    "src",
    [
        b"{a: 1}: v\n",  # mapping key
        b"[1, 2]: v\n",  # sequence key
        b"{a: 1, b: 2}: v\n",  # multi-entry mapping key
        b"? {x: {y: 1}}\n: v\n",  # nested mapping key
    ],
)
def test_complex_key_survives_a_dumps_loads_round_trip(src):
    """A collection mapping key round-trips through dumps: the tuple-of-pairs
    representation re-serializes and reloads to an equal value (a frozenset would
    re-serialize as a sequence and reload as a different type)."""
    data = yamlrocks.loads(src)
    assert yamlrocks.loads(yamlrocks.dumps(data)) == data


# -- OPT_REJECT_COMPLEX_KEYS: opt into rejecting a complex key -----------------
# Off by default (accept-and-convert is the spec-compliant default). With the
# flag, a collection used as a mapping key raises YAMLRocksComplexKeyError with a
# location, so a scalar-keyed consumer can catch the common unquoted-template typo
# early. Scope A: both sequence and mapping keys are rejected.

REJECT = yamlrocks.OPT_REJECT_COMPLEX_KEYS


@pytest.mark.parametrize(
    "src",
    [
        b"{a: 1}: b\n",  # flow mapping key
        b"[1, 2]: b\n",  # flow sequence key (scope A: rejected too)
        b"? {a: 1}\n: v\n",  # explicit mapping key
        b"? [1, 2]\n: v\n",  # explicit sequence key
        b"[{a: 1}: v]\n",  # single-pair flow entry
        b"v: {{ x }}\n",  # the unquoted-template typo (a mapping key)
    ],
)
def test_complex_key_rejected_with_the_flag(src):
    """A complex key raises YAMLRocksComplexKeyError under the flag."""
    with pytest.raises(yamlrocks.YAMLRocksComplexKeyError):
        yamlrocks.loads(src, option=REJECT)


def test_complex_key_error_is_located_and_a_decode_error():
    """The error carries line/column and is a YAMLRocksDecodeError/ValueError."""
    with pytest.raises(yamlrocks.YAMLRocksComplexKeyError) as err:
        yamlrocks.loads(b"a: 1\n{x: 1}: 2\n", option=REJECT)
    assert isinstance(err.value, yamlrocks.YAMLRocksDecodeError)
    assert isinstance(err.value, ValueError)
    assert err.value.line == 2 and err.value.column == 1


def test_reject_flag_works_on_annotated_and_tag_paths():
    """The rejection fires on the annotated and tag-resolving paths too."""
    for option in (
        REJECT | yamlrocks.OPT_ANNOTATED,
        REJECT | yamlrocks.OPT_PASSTHROUGH_TAG,
    ):
        with pytest.raises(yamlrocks.YAMLRocksComplexKeyError):
            yamlrocks.loads(b"{a: 1}: b\n", option=option)


def test_reject_flag_leaves_scalar_keys_alone():
    """Scalar keys (incl. an embedded template) are unaffected by the flag."""
    assert yamlrocks.loads(b"k: v\n", option=REJECT) == {"k": "v"}
    # An embedded template starts with a normal char, so it is a plain scalar key.
    assert yamlrocks.loads(b"a_{{ x }}_b: v\n", option=REJECT) == {"a_{{ x }}_b": "v"}


def test_complex_key_converts_without_the_flag():
    """Without the flag the default convert-to-hashable behavior is unchanged."""
    assert yamlrocks.loads(b"{a: 1}: b\n") == {(("a", 1),): "b"}
    assert yamlrocks.loads(b"v: {{ x }}\n") == {"v": {(("x", None),): None}}


# -- loads_all: trailing/empty documents and annotated multi-doc ---------------


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        (b"only: x\n---\n", [{"only": "x"}, None]),
        (b"---\n", [None]),
        (b"a: 1\n---\nb: 2\n", [{"a": 1}, {"b": 2}]),
        (b"---\na: 1\n---\n", [{"a": 1}, None]),
        (b"---\n---\n", [None, None]),
        (b"", []),
        (b"a: 1\n", [{"a": 1}]),
    ],
)
def test_loads_all_document_count_matches_pyyaml(src, expected):
    """An explicit `---` yields a document even when empty (PyYAML semantics),
    so a trailing `---` is a distinct null document, not dropped."""
    assert list(yamlrocks.loads_all(src)) == expected


def test_loads_all_honors_annotated():
    """loads_all annotates each document when OPT_ANNOTATED is set."""
    docs = yamlrocks.loads_all(b"a: 1\n---\nb: 2\n", option=yamlrocks.OPT_ANNOTATED)
    assert [dict(d) for d in docs] == [{"a": 1}, {"b": 2}]
    assert all(isinstance(d, yamlrocks.YAMLRocksAnnotatedDict) for d in docs)
    assert docs[0].__line__ == 1
    assert docs[1].__line__ == 3


def test_loads_all_annotated_keeps_trailing_empty_doc():
    """The trailing empty document survives as None in annotated mode too."""
    docs = yamlrocks.loads_all(b"only: x\n---\n", option=yamlrocks.OPT_ANNOTATED)
    assert len(docs) == 2
    assert isinstance(docs[0], yamlrocks.YAMLRocksAnnotatedDict)
    assert docs[1] is None


@pytest.mark.parametrize(
    "option",
    [
        yamlrocks.OPT_INCLUDES,
        yamlrocks.OPT_SECRETS,
        yamlrocks.OPT_ENV_VAR,
        yamlrocks.OPT_ROUND_TRIP,
    ],
)
def test_loads_all_rejects_unsupported_options(option):
    """loads_all rejects options it cannot honor rather than silently ignoring.

    It has no include_dir parameter and returns a list, so include/secret/env-var
    resolution and the single-document round-trip option are errors, not no-ops.
    """
    with pytest.raises(ValueError, match="loads_all"):
        yamlrocks.loads_all(b"a: 1\n", option=option)


# -- `---`/`...` are document markers only at column 0 -------------------------
# An indented `---`/`...` is ordinary plain-scalar content (a mapping value or a
# sequence item), not a document marker. Treating it as a marker silently turned
# the value into None and could swallow following keys; the spec and PyYAML both
# read it as the literal string. Regression test for that fast-path bug.


@pytest.mark.parametrize("marker", ["...", "---"])
def test_indented_marker_is_a_plain_scalar_value(marker):
    """An indented `---`/`...` as a mapping value is the literal string.

    The scanner is shared with the round-trip path, so pin both: the fast path
    decodes the string, and round-trip re-emits byte-for-byte.
    """
    src = f"top: {marker}\n".encode()
    assert yamlrocks.loads(src) == {"top": marker}
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


@pytest.mark.parametrize("marker", ["...", "---"])
def test_indented_marker_does_not_swallow_following_keys(marker):
    """A `...`/`---` value must not end the document and drop later keys."""
    src = f"m:\n  n: {marker}\n  o: 2\n".encode()
    assert yamlrocks.loads(src) == {"m": {"n": marker, "o": 2}}
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


@pytest.mark.parametrize("marker", ["...", "---"])
def test_indented_marker_as_sequence_item(marker):
    """An indented `---`/`...` sequence item is the literal string, not a marker."""
    src = f"a:\n  - {marker}\n  - b\n".encode()
    assert yamlrocks.loads(src) == {"a": [marker, "b"]}
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_yaml() == src


def test_column_zero_markers_still_delimit_documents():
    """The fix must not stop real column-0 markers from delimiting documents."""
    assert yamlrocks.loads_all(b"---\nx\n...\n---\ny\n") == ["x", "y"]
