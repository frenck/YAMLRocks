"""Annotated mode (``OPT_ANNOTATED``) - source-location tracking."""

from __future__ import annotations

import copy
import pickle

import pytest

import yamlrocks

ANN = yamlrocks.OPT_ANNOTATED


def test_returns_real_dict_subclass():
    """Annotated mode returns a real dict subclass with the same data."""
    data = yamlrocks.loads(b"a: 1\nb: 2\n", option=ANN)
    assert isinstance(data, dict)
    assert dict(data) == {"a": 1, "b": 2}


def test_preserves_key_order():
    """Annotated mode preserves the source key order."""
    data = yamlrocks.loads(b"z: 1\na: 2\nm: 3\n", option=ANN)
    assert list(data.keys()) == ["z", "a", "m"]


def test_line_and_column_on_root():
    """The root mapping reports its source line and column."""
    data = yamlrocks.loads(b"name: app\nserver:\n  host: localhost\n", option=ANN)
    assert data.__line__ == 1
    assert data.__column__ == 1


def test_line_on_nested_mapping():
    """A nested mapping reports its own source line and column."""
    data = yamlrocks.loads(b"name: app\nserver:\n  host: localhost\n", option=ANN)
    assert data["server"].__line__ == 3
    assert data["server"].__column__ == 3


def test_list_is_real_list_subclass():
    """A sequence is a real list subclass that reports its source line."""
    data = yamlrocks.loads(b"items:\n  - 1\n  - 2\n", option=ANN)
    items = data["items"]
    assert isinstance(items, list)
    assert list(items) == [1, 2]
    assert items.__line__ == 2


def test_config_file_is_none_for_in_memory_bytes():
    """__file__ is None when there is no source path: in-memory bytes with no
    root_path have nowhere to point."""
    data = yamlrocks.loads(b"a: 1\n", option=ANN)
    assert data.__file__ is None


def test_string_scalars_are_annotated():
    """String scalars become YAMLRocksAnnotatedStr with a source location."""
    data = yamlrocks.loads(b"name: app\nhost: localhost\n", option=ANN)
    name = data["name"]
    assert isinstance(name, yamlrocks.YAMLRocksAnnotatedStr)
    assert isinstance(name, str)
    assert name == "app"
    assert name.__line__ == 1
    assert name.__column__ == 7


def test_non_string_scalars_stay_plain():
    """Non-string scalars are returned as plain Python values."""
    data = yamlrocks.loads(b"port: 8080\nactive: true\nempty: null\n", option=ANN)
    assert type(data["port"]) is int
    assert type(data["active"]) is bool
    assert data["empty"] is None


def test_annotated_string_carries_config_file_through_includes(tmp_path):
    """Annotated strings carry their originating file through includes."""
    (tmp_path / "sub.yaml").write_text("value: hello\n")
    data = yamlrocks.loads(
        b"section: !include sub.yaml\n",
        option=ANN | yamlrocks.OPT_INCLUDES,
        include_dir=str(tmp_path),
    )
    value = data["section"]["value"]
    assert isinstance(value, yamlrocks.YAMLRocksAnnotatedStr)
    assert value == "hello"
    assert value.__file__.endswith("sub.yaml")


def test_annotated_str_behaves_like_str():
    """YAMLRocksAnnotatedStr supports normal string operations."""
    s = yamlrocks.YAMLRocksAnnotatedStr("Hello", line=3, column=5, config_file="c.yaml")
    assert s.upper() == "HELLO"
    assert s + " world" == "Hello world"
    assert s.__line__ == 3
    assert s.__file__ == "c.yaml"


def test_annotated_str_uses_slots_not_instance_dict():
    """YAMLRocksAnnotatedStr stores its metadata in __slots__, not a per-instance
    __dict__: annotating a large document must not pay a dict per string."""
    s = yamlrocks.loads(b"a: hello\n", option=ANN)["a"]
    assert isinstance(s, yamlrocks.YAMLRocksAnnotatedStr)
    assert not hasattr(s, "__dict__")
    # Arbitrary attributes are rejected, proving there is no backing dict.
    with pytest.raises(AttributeError):
        s.__does_not_exist__ = 1


def test_mapping_keys_are_annotated():
    """Mapping keys, not just values, carry their own source location."""
    data = yamlrocks.loads(b"name: app\nserver:\n  host: localhost\n", option=ANN)
    key = next(iter(data))
    assert isinstance(key, yamlrocks.YAMLRocksAnnotatedStr)
    assert key == "name"
    assert key.__line__ == 1
    assert key.__column__ == 1
    # A nested key reports its own line.
    nested_key = next(iter(data["server"]))
    assert isinstance(nested_key, yamlrocks.YAMLRocksAnnotatedStr)
    assert nested_key == "host"
    assert nested_key.__line__ == 3


def test_annotated_keys_lookup_with_plain_str():
    """Annotated keys stay hashable and equal to the plain string, so plain-str
    lookups and dict conversion keep working."""
    data = yamlrocks.loads(b"key: value\n", option=ANN)
    assert data["key"] == "value"
    assert "key" in data
    assert dict(data) == {"key": "value"}


def test_non_string_keys_stay_plain():
    """Non-string mapping keys are returned plain, mirroring value behavior."""
    data = yamlrocks.loads(b"1: one\n2: two\n", option=ANN)
    for key in data:
        assert type(key) is int


def test_root_reports_real_path_via_load(tmp_path):
    """Nodes from the top-level file report the real path passed to ``load()``,
    not a synthetic placeholder, including the annotated keys."""
    cfg = tmp_path / "configuration.yaml"
    cfg.write_text("name: home\nserver:\n  host: localhost\n")
    data = yamlrocks.load(str(cfg), option=ANN | yamlrocks.OPT_INCLUDES)
    assert data.__file__ == str(cfg)
    assert data["server"].__file__ == str(cfg)
    assert next(iter(data)).__file__ == str(cfg)


def test_root_reports_real_path_via_load_without_includes(tmp_path):
    """The top-level file's path reaches annotated nodes even without
    ``OPT_INCLUDES``: ``load(path)`` alone is enough for ``__file__``."""
    cfg = tmp_path / "configuration.yaml"
    cfg.write_text("name: home\nserver:\n  host: localhost\n")
    data = yamlrocks.load(str(cfg), option=ANN)
    assert data["name"].__file__ == str(cfg)
    assert next(iter(data["server"])).__file__ == str(cfg)


# -- Annotated mode runs the same structural validation as the default path -----


def test_annotated_rejects_bare_scalar_after_mapping():
    """A bare scalar where a key is expected is rejected in annotated mode too."""
    with pytest.raises(yamlrocks.YAMLRocksParseError, match="expected ':'"):
        yamlrocks.loads(b"a: a\nnokeyhere", option=ANN)


def test_annotated_rejects_over_indented_block():
    """An over-indented block (a block collection in key position) is rejected."""
    import pytest

    src = b"iot_domain:\n  - platform: x\n      option1: abc\n"
    with pytest.raises(yamlrocks.YAMLRocksParseError, match="block collection"):
        yamlrocks.loads(src, option=ANN)


def test_annotated_matches_default_error_location():
    """The annotated path reports the identical message and line/column."""
    src = b"a: a\nnokeyhere"
    with pytest.raises(yamlrocks.YAMLRocksParseError) as default:
        yamlrocks.loads(src)
    with pytest.raises(yamlrocks.YAMLRocksParseError) as annotated:
        yamlrocks.loads(src, option=ANN)
    assert str(default.value) == str(annotated.value)
    assert (default.value.line, default.value.column) == (
        annotated.value.line,
        annotated.value.column,
    )


# -- Annotated containers honor the dict/list subclass contract ----------------
# voluptuous, copy, and pickle all assume a dict/list subclass is constructible,
# copyable, and pickleable; the native containers must satisfy that to be usable
# directly (no conversion pass).


def test_annotated_containers_are_constructible():
    """The native containers construct with no arguments (voluptuous's
    data.__class__() pattern) and from an existing mapping/iterable."""
    assert yamlrocks.YAMLRocksAnnotatedDict() == {}
    assert yamlrocks.YAMLRocksAnnotatedList() == []
    assert yamlrocks.YAMLRocksAnnotatedDict({"a": 1}) == {"a": 1}
    assert yamlrocks.YAMLRocksAnnotatedList([1, 2]) == [1, 2]


def test_voluptuous_style_reconstruction():
    """Rebuilding via type(data)() then filling (what voluptuous does) works."""
    data = yamlrocks.loads(b"a:\n  b: c\n", option=ANN)
    out = type(data)()
    for key, value in data.items():
        out[key] = value
    assert out == {"a": {"b": "c"}}
    assert isinstance(out, dict)


def test_annotated_dict_copy_and_deepcopy_preserve_annotations():
    """copy/deepcopy round-trip the data and keep source locations."""
    data = yamlrocks.loads(b"a:\n  b: c\n", option=ANN)
    assert copy.copy(data) == {"a": {"b": "c"}}
    deep = copy.deepcopy(data)
    assert deep == {"a": {"b": "c"}}
    assert isinstance(deep, yamlrocks.YAMLRocksAnnotatedDict)
    assert deep["a"].__line__ == 2  # annotation survives deepcopy


def test_annotated_list_copy_and_deepcopy_preserve_annotations():
    """Lists copy/deepcopy with their annotations too."""
    items = yamlrocks.loads(b"items:\n  - 1\n  - 2\n", option=ANN)["items"]
    assert copy.deepcopy(items) == [1, 2]
    assert copy.deepcopy(items).__line__ == items.__line__
    assert isinstance(copy.deepcopy(items), yamlrocks.YAMLRocksAnnotatedList)


def test_annotated_containers_pickle_round_trip():
    """pickle round-trips both containers, preserving type and annotations."""
    data = yamlrocks.loads(b"a:\n  b: c\nitems:\n  - 1\n", option=ANN)
    restored = pickle.loads(pickle.dumps(data))
    assert restored == {"a": {"b": "c"}, "items": [1]}
    assert isinstance(restored, yamlrocks.YAMLRocksAnnotatedDict)
    assert isinstance(restored["items"], yamlrocks.YAMLRocksAnnotatedList)


def test_annotated_container_module_is_yamlrocks():
    """__module__ is set so pickle can locate the class by reference."""
    assert yamlrocks.YAMLRocksAnnotatedDict.__module__ == "yamlrocks"
    assert yamlrocks.YAMLRocksAnnotatedList.__module__ == "yamlrocks"


# -- End positions (__end_line__/__end_column__), mirroring PyYAML's end_mark ---
# The end is 1-based and points just past the node's last character, so a tool
# can underline the full span (start..end) of a key, value, or block.


def test_scalar_key_has_end_position():
    """A string key carries an end mark just past its last character."""
    data = yamlrocks.loads(b"key: value\nbroad: x\n", option=ANN)
    key = list(data)[1]  # the 'broad' key
    assert (key.__line__, key.__column__) == (2, 1)
    assert (key.__end_line__, key.__end_column__) == (2, 6)  # 'broad' spans cols 1..5


def test_scalar_value_has_end_position():
    """A string value reports its own end mark."""
    data = yamlrocks.loads(b"key: value\n", option=ANN)
    value = data["key"]
    assert (value.__line__, value.__column__) == (1, 6)
    assert (value.__end_line__, value.__end_column__) == (1, 11)  # 'value' is 5 chars


def test_mapping_end_position_spans_the_block():
    """A mapping's end mark reaches the end of its furthest child (block end)."""
    data = yamlrocks.loads(b"a: 1\nb:\n  - x\n  - yy\n", option=ANN)
    assert data.__line__ == 1
    # The block ends at 'yy' on line 4 (cols 5..6), so end is line 4, column 7.
    assert (data.__end_line__, data.__end_column__) == (4, 7)


def test_sequence_end_position_spans_the_block():
    """A sequence's end mark reaches the end of its last element."""
    items = yamlrocks.loads(b"b:\n  - x\n  - yy\n", option=ANN)["b"]
    assert items.__line__ == 2
    assert (items.__end_line__, items.__end_column__) == (3, 7)


def test_end_position_on_multiline_scalar():
    """A multi-line (folded/literal) scalar ends on its last content line."""
    data = yamlrocks.loads(b"text: |\n  one\n  two\n", option=ANN)
    text = data["text"]
    # The literal block content is "one\ntwo\n"; the end lands past "two".
    assert text.__end_line__ >= text.__line__


def test_default_end_position_when_constructed_bare():
    """A bare-constructed container defaults its end marks to zero (like the
    other source fields), so the subclass contract is unchanged."""
    assert yamlrocks.YAMLRocksAnnotatedDict().__end_line__ == 0
    assert yamlrocks.YAMLRocksAnnotatedList().__end_column__ == 0
    assert yamlrocks.YAMLRocksAnnotatedStr("x").__end_line__ == 0


def test_double_quoted_scalar_end_is_past_the_closing_quote():
    """A double-quoted value's end mark lands past the closing quote, not at the
    unescaped value's length (the quotes are part of the source span)."""
    data = yamlrocks.loads(b'k: "hello"\n', option=ANN)
    value = data["k"]
    # '"hello"' occupies columns 4..10; the end is just past it, at column 11.
    assert (value.__end_line__, value.__end_column__) == (1, 11)


def test_single_quoted_scalar_end_is_past_the_closing_quote():
    """A single-quoted value's end mark lands past the closing quote."""
    data = yamlrocks.loads(b"k: 'sq'\n", option=ANN)
    value = data["k"]
    # "'sq'" occupies columns 4..7; the end is just past it, at column 8.
    assert (value.__end_line__, value.__end_column__) == (1, 8)


def test_escaped_scalar_end_counts_source_not_value_length():
    """An escaped double-quoted value (shorter than its source) ends at the
    source span's end, so an escape is not mistaken for a single character."""
    data = yamlrocks.loads(b'k: "a\\tb"\n', option=ANN)
    value = data["k"]
    assert value == "a\tb"  # three characters after unescaping
    # '"a\tb"' is six source columns (4..9); the end is at column 10, not the
    # column 8 the three-character unescaped value would imply.
    assert (value.__end_line__, value.__end_column__) == (1, 10)


def test_quoted_scalar_byte_range_slices_the_verbatim_token():
    """src[__offset__:__end_offset__] is the verbatim source token, quotes and
    all, which is what makes the byte range exact where line/column round."""
    src = b'k: "hello"\n'
    value = yamlrocks.loads(src, option=ANN)["k"]
    assert src[value.__offset__ : value.__end_offset__] == b'"hello"'


def test_multiline_double_quoted_scalar_ends_on_the_closing_quote_line():
    """A double-quoted value spanning lines ends on the line of its closing
    quote, derived from the true source end rather than the folded value."""
    src = b'k: "line one\n     line two"\nnext: 1\n'
    value = yamlrocks.loads(src, option=ANN)["k"]
    assert value.__line__ == 1
    assert value.__end_line__ == 2  # the closing quote is on the second line


def test_quoted_scalar_end_column_survives_a_leading_bom():
    """A leading byte order mark is not content, so columns past it match the
    scanner's basis (the first real character is column 1)."""
    src = '﻿x: "q"\n'.encode()
    value = yamlrocks.loads(src, option=ANN)["x"]
    assert (value.__line__, value.__column__) == (1, 4)
    assert (value.__end_line__, value.__end_column__) == (1, 7)


# -- Byte offsets (__offset__/__end_offset__): exact source byte ranges ---------


def test_annotated_byte_offsets_slice_exact_source():
    """`__offset__`/`__end_offset__` give each node's exact source byte range.

    The range is precise even for quoted scalars (the closing quote is included),
    so slicing `source[node.__offset__:node.__end_offset__]` reproduces the
    verbatim source token across containers, strings, and (annotated) numbers.
    """
    src = b'name: hello\nquoted: "two words"\nnum: 42\nitems:\n- a\n- bb\n'
    data = yamlrocks.loads(src, option=ANN | yamlrocks.OPT_ANNOTATE_NUMBERS)
    assert src[data.__offset__ : data.__end_offset__].startswith(b"name: hello")
    name = data["name"]
    assert src[name.__offset__ : name.__end_offset__] == b"hello"
    quoted = data["quoted"]
    assert src[quoted.__offset__ : quoted.__end_offset__] == b'"two words"'
    num = data["num"]
    assert src[num.__offset__ : num.__end_offset__] == b"42"
    items = data["items"]
    assert src[items.__offset__ : items.__end_offset__] == b"- a\n- bb"


def test_byte_offsets_survive_pickle_and_deepcopy():
    """The byte offsets are preserved through pickle and deepcopy."""
    data = yamlrocks.loads(b"a:\n  b: c\n", option=ANN)
    for restored in (pickle.loads(pickle.dumps(data)), copy.deepcopy(data)):
        assert restored.__offset__ == data.__offset__
        assert restored.__end_offset__ == data.__end_offset__


def test_default_byte_offsets_when_constructed_bare():
    """A bare-constructed annotated object defaults its byte offsets to zero."""
    assert yamlrocks.YAMLRocksAnnotatedDict().__offset__ == 0
    assert yamlrocks.YAMLRocksAnnotatedList().__end_offset__ == 0
    assert yamlrocks.YAMLRocksAnnotatedStr("x").__offset__ == 0


# -- Scalar style on annotated strings (__style__) -----------------------------
# Exposes how a string scalar was written, so a consumer can tell a block scalar
# (| or >) from a plain/quoted one. ESPHome uses this to offset !lambda #line
# directives to the block body. The vocabulary matches round-trip YAMLRocksNode.style.


@pytest.mark.parametrize(
    ("src", "style"),
    [
        (b"x: hello\n", "plain"),
        (b"x: 'hello'\n", "single"),
        (b'x: "hello"\n', "double"),
        (b"x: |\n  body\n", "literal"),
        (b"x: >\n  body\n", "folded"),
    ],
)
def test_annotated_str_exposes_scalar_style(src, style):
    """A string scalar reports the source style it was written in."""
    value = yamlrocks.loads(src, option=ANN)["x"]
    assert value.__style__ == style


def test_block_scalar_style_enables_content_offset():
    """A block scalar (| or >) is distinguishable, which is what lets a consumer
    point a #line directive at the body rather than the indicator line."""
    lit = yamlrocks.loads(b"lambda: |-\n  return x;\n", option=ANN)["lambda"]
    # The annotation starts at the indicator line (like PyYAML's start_mark), so a
    # consumer adds 1 for block styles to reach the body.
    assert lit.__style__ in ("literal", "folded")
    plain = yamlrocks.loads(b"x: hello\n", option=ANN)["x"]
    assert plain.__style__ not in ("literal", "folded")


def test_bare_constructed_str_defaults_to_plain_style():
    """A bare-constructed annotated string defaults its style to ``plain``."""
    assert yamlrocks.YAMLRocksAnnotatedStr("x").__style__ == "plain"


# -- OPT_ANNOTATE_NUMBERS: opt-in source locations on int/float ----------------
# By default numeric scalars stay plain int/float (see test_non_string_scalars_
# stay_plain). With OPT_ANNOTATE_NUMBERS they become int/float subclasses
# carrying the same metadata as annotated strings. bool/None always stay plain.

NUM = ANN | yamlrocks.OPT_ANNOTATE_NUMBERS


def test_numbers_stay_plain_without_the_flag():
    """Without OPT_ANNOTATE_NUMBERS the documented contract holds: plain int/float."""
    data = yamlrocks.loads(b"port: 8080\nratio: 1.5\n", option=ANN)
    assert type(data["port"]) is int
    assert type(data["ratio"]) is float


def test_int_is_annotated_with_the_flag():
    """An integer carries its source location and stays int-compatible."""
    data = yamlrocks.loads(b"key: value\nport: 8080\n", option=NUM)
    port = data["port"]
    assert isinstance(port, int) and port == 8080
    assert type(port) is not int  # it is a subclass now
    assert isinstance(port, yamlrocks.YAMLRocksAnnotatedInt)
    assert (port.__line__, port.__column__) == (2, 7)
    assert (port.__end_line__, port.__end_column__) == (2, 11)  # '8080' is 4 chars
    assert port.__style__ == "plain"


def test_float_is_annotated_with_the_flag():
    """A float carries its source location and stays float-compatible."""
    data = yamlrocks.loads(b"ratio: 1.5\n", option=NUM)
    ratio = data["ratio"]
    assert isinstance(ratio, float) and ratio == 1.5
    assert isinstance(ratio, yamlrocks.YAMLRocksAnnotatedFloat)
    assert ratio.__line__ == 1


def test_annotated_numbers_behave_like_numbers():
    """Arithmetic, equality, and hashing are unaffected by the subclass."""
    data = yamlrocks.loads(b"a: 8080\nb: 8080\n", option=NUM)
    a = data["a"]
    assert a + 1 == 8081
    assert a * 2 == 16160
    assert a == 8080 and hash(a) == hash(8080)
    assert {a: "x"}[8080] == "x"  # usable and equal as a dict key


def test_bool_and_none_stay_plain_even_with_the_flag():
    """bool/None can't be subclassed, so they stay plain (matching PyYAML)."""
    data = yamlrocks.loads(b"active: true\nempty: null\n", option=NUM)
    assert type(data["active"]) is bool
    assert data["empty"] is None


def test_annotated_number_classes_default_to_plain_style():
    """Bare-constructed annotated numbers default their metadata sensibly."""
    assert yamlrocks.YAMLRocksAnnotatedInt(5).__style__ == "plain"
    assert yamlrocks.YAMLRocksAnnotatedInt(5) == 5
    assert yamlrocks.YAMLRocksAnnotatedFloat(2.5).__line__ == 0
    assert yamlrocks.YAMLRocksAnnotatedFloat(2.5) == 2.5


# -- Aliases share object identity with their anchor (PyYAML's behavior) -------
# An alias (*a) resolves to the *same* Python object as its anchor (&a), not an
# independent copy, so mutating one is visible through every reference. ESPHome's
# package/substitution passes mutate aliased objects in place and rely on this.


def test_alias_mapping_is_the_same_object_as_anchor():
    """A mapping alias yields the same object as its anchor, like PyYAML."""
    data = yamlrocks.loads(b"base: &a\n  k: 1\nref: *a\n", option=ANN)
    assert data["base"] is data["ref"]


def test_mutating_through_one_alias_is_visible_through_all():
    """Because aliases share identity, an in-place change is seen everywhere."""
    data = yamlrocks.loads(b"base: &a\n  k: 1\nref: *a\n", option=ANN)
    data["base"]["k"] = 99
    assert data["ref"]["k"] == 99


def test_alias_sequence_is_the_same_object_as_anchor():
    """A sequence alias shares identity with its anchor too."""
    data = yamlrocks.loads(b"base: &a [1, 2]\nref: *a\n", option=ANN)
    assert data["base"] is data["ref"]
    data["base"].append(3)
    assert list(data["ref"]) == [1, 2, 3]


def test_alias_scalar_string_is_the_same_object_as_anchor():
    """A string-scalar alias shares the single annotated-string instance."""
    data = yamlrocks.loads(b"a: &x hello\nb: *x\n", option=ANN)
    assert data["a"] is data["b"]


def test_alias_to_anchor_that_itself_holds_an_alias():
    """Nested anchors resolve to a shared graph: c (-> b) shares b's object,
    and b's inner *a shares a's object."""
    data = yamlrocks.loads(b"a: &a {v: 1}\nb: &b {inner: *a}\nc: *b\n", option=ANN)
    assert data["b"] is data["c"]
    assert data["b"]["inner"] is data["a"]


def test_aliases_share_across_documents_only_within_a_document():
    """Each document resolves its own anchors; identity is shared within a
    document (a fresh anchor scope per document)."""
    docs = yamlrocks.loads_all(
        b"base: &a {k: 1}\nref: *a\n---\nbase: &a {k: 2}\nref: *a\n",
        option=ANN,
    )
    assert docs[0]["base"] is docs[0]["ref"]
    assert docs[1]["base"] is docs[1]["ref"]
    assert docs[0]["base"] is not docs[1]["base"]


# -- __source_tag__ provenance + is_secret/is_env_var/is_include predicates -----
# Every annotated node exposes which tag produced it: a config directive
# (!secret/!env_var/!include*) or a custom application tag, or None for an inline
# scalar. The three booleans are sugar over the built-in config-tag subset.


PROV = ANN | yamlrocks.OPT_SECRETS | yamlrocks.OPT_ENV_VAR | yamlrocks.OPT_INCLUDES


def _provenance_dir(tmp_path):
    (tmp_path / "secrets.yaml").write_text("pw: hunter2\nport: 8080\n")
    (tmp_path / "sub.yaml").write_text("nested: 1\n")
    return tmp_path


def test_source_tag_reports_each_directive(tmp_path, monkeypatch):
    monkeypatch.setenv("YR_HOST", "localhost")
    _provenance_dir(tmp_path)
    src = (
        b"pw: !secret pw\n"
        b"host: !env_var YR_HOST\n"
        b"sub: !include sub.yaml\n"
        b"plain: hello\n"
        b"tagged: !mytag custom\n"
    )
    data = yamlrocks.loads(src, option=PROV, include_dir=str(tmp_path))
    assert data["pw"].__source_tag__ == "!secret"
    assert data["host"].__source_tag__ == "!env_var"
    assert data["sub"].__source_tag__ == "!include"
    assert data["plain"].__source_tag__ is None
    assert data["tagged"].__source_tag__ == "!mytag"  # custom tag surfaces too


def test_provenance_booleans(tmp_path, monkeypatch):
    monkeypatch.setenv("YR_HOST", "localhost")
    _provenance_dir(tmp_path)
    src = b"pw: !secret pw\nhost: !env_var YR_HOST\nsub: !include sub.yaml\nplain: x\n"
    data = yamlrocks.loads(src, option=PROV, include_dir=str(tmp_path))
    assert data["pw"].is_secret and not data["pw"].is_env_var
    assert data["host"].is_env_var and not data["host"].is_secret
    assert data["sub"].is_include
    assert not (
        data["plain"].is_secret or data["plain"].is_env_var or data["plain"].is_include
    )


def test_include_dir_variants_are_is_include(tmp_path):
    (tmp_path / "d").mkdir()
    (tmp_path / "d" / "a.yaml").write_text("x: 1\n")
    src = b"items: !include_dir_merge_named d\n"
    data = yamlrocks.loads(src, option=PROV, include_dir=str(tmp_path))
    assert data["items"].is_include
    assert data["items"].__source_tag__ == "!include_dir_merge_named"


def test_numeric_secret_carries_provenance(tmp_path):
    _provenance_dir(tmp_path)
    data = yamlrocks.loads(
        b"port: !secret port\n",
        option=PROV | yamlrocks.OPT_ANNOTATE_NUMBERS,
        include_dir=str(tmp_path),
    )
    assert data["port"] == 8080
    assert isinstance(data["port"], yamlrocks.YAMLRocksAnnotatedInt)
    assert data["port"].is_secret and data["port"].__source_tag__ == "!secret"


def test_provenance_survives_pickle_and_deepcopy(tmp_path):
    _provenance_dir(tmp_path)
    data = yamlrocks.loads(
        b"sub: !include sub.yaml\n", option=PROV, include_dir=str(tmp_path)
    )
    assert pickle.loads(pickle.dumps(data))["sub"].__source_tag__ == "!include"
    assert copy.deepcopy(data)["sub"].is_include


def test_bare_constructed_has_no_source_tag():
    assert yamlrocks.YAMLRocksAnnotatedStr("x").__source_tag__ is None
    assert yamlrocks.YAMLRocksAnnotatedDict().is_secret is False
    assert yamlrocks.YAMLRocksAnnotatedList().is_include is False
    assert yamlrocks.YAMLRocksAnnotatedInt(5).is_env_var is False


def test_source_target_carries_the_directive_argument(tmp_path, monkeypatch):
    """__source_target__ holds the directive's argument (secret name, include
    path, env-var spec), so it pairs with __source_tag__ to rebuild the directive."""
    monkeypatch.setenv("YR_HOST", "localhost")
    (tmp_path / "secrets.yaml").write_text("db_password: hunter2\n")
    (tmp_path / "sub.yaml").write_text("x: 1\n")
    src = (
        b"pw: !secret db_password\n"
        b"host: !env_var YR_HOST fallback\n"
        b"sub: !include sub.yaml\n"
        b"plain: hi\n"
    )
    data = yamlrocks.loads(src, option=PROV, include_dir=str(tmp_path))
    assert data["pw"].__source_target__ == "db_password"
    assert data["host"].__source_target__ == "YR_HOST fallback"
    assert data["sub"].__source_target__ == "sub.yaml"
    assert data["plain"].__source_target__ is None
    # tag + target reconstruct the directive a redactor would write back.
    pw = data["pw"]
    assert f"{pw.__source_tag__} {pw.__source_target__}" == "!secret db_password"


def test_source_target_survives_pickle(tmp_path):
    (tmp_path / "secrets.yaml").write_text("k: v\n")
    data = yamlrocks.loads(b"s: !secret k\n", option=PROV, include_dir=str(tmp_path))
    assert pickle.loads(pickle.dumps(data))["s"].__source_target__ == "k"


def test_bare_constructed_has_no_source_target():
    assert yamlrocks.YAMLRocksAnnotatedStr("x").__source_target__ is None
    assert yamlrocks.YAMLRocksAnnotatedDict().__source_target__ is None
