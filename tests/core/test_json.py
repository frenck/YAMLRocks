"""JSON export via ``to_json`` (and its async wrapper).

JSON is the lossy subset of YAML, so ``to_json`` applies a fixed projection:
tags are dropped, non-finite floats become ``null``, non-string scalar keys are
stringified, and a collection used as a key is an error. JSON *import* needs no
new API - JSON is valid YAML 1.2, so ``loads`` already parses it.
"""

from __future__ import annotations

import json
import math

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def test_compact_by_default():
    """to_json emits compact JSON (no spaces) by default."""
    assert yamlrocks.to_json({"a": 1, "b": [True, None, 2.5]}) == (
        b'{"a":1,"b":[true,null,2.5]}'
    )


def test_matches_stdlib_json():
    """Round-tripping through stdlib json yields the original value."""
    obj = {"s": "hi", "n": 3, "f": 1.5, "list": [1, 2, {"x": True}], "z": None}
    assert json.loads(yamlrocks.to_json(obj)) == obj


def test_floats_stay_distinguishable_from_ints():
    """A float keeps a decimal point so it is not confused with an int."""
    assert yamlrocks.to_json({"i": 1, "f": 1.0}) == b'{"i":1,"f":1.0}'


def test_non_finite_floats_become_null():
    """NaN and infinities are not valid JSON; they project to null."""
    assert yamlrocks.to_json({"a": math.nan, "b": math.inf, "c": -math.inf}) == (
        b'{"a":null,"b":null,"c":null}'
    )


@pytest.mark.parametrize(
    "value", [1e16, 1e-7, 1e100, 1.7976931348623157e308, 5e-324, 0.1]
)
def test_large_and_tiny_floats_use_compact_scientific_notation(value):
    """A large- or small-magnitude float emits in compact scientific notation
    (e.g. `1.0e+16`), not a full positional decimal that balloons to hundreds of
    digits, and it round-trips exactly through stdlib json."""
    out = yamlrocks.to_json({"v": value})
    # No grotesque expansion: `1e100` must not become a 100-digit string.
    assert len(out) < 40
    assert json.loads(out)["v"] == value


def test_non_string_scalar_keys_are_stringified():
    """Int/bool/null/float keys stringify to their JSON scalar text."""
    assert yamlrocks.to_json({None: 1}) == b'{"null":1}'
    assert yamlrocks.to_json({True: 1, False: 2}) == b'{"true":1,"false":2}'
    assert yamlrocks.to_json({2.5: 1}) == b'{"2.5":1}'


def test_collection_key_is_rejected():
    """A collection has no JSON-key representation, so it errors."""
    with pytest.raises(yamlrocks.YAMLRocksEncodeError, match="key"):
        yamlrocks.to_json({(1, 2): "x"})


def test_string_escaping():
    """Control characters and quotes are escaped per the JSON grammar."""
    assert yamlrocks.to_json({"s": 'a\nb\t"q"\\'}) == b'{"s":"a\\nb\\t\\"q\\"\\\\"}'
    assert yamlrocks.to_json("\x00") == b'"\\u0000"'


def test_indent_and_sort_keys():
    """OPT_INDENT_2/4 pretty-print and OPT_SORT_KEYS orders object keys."""
    out = yamlrocks.to_json(
        {"b": 2, "a": 1}, option=yamlrocks.OPT_INDENT_2 | yamlrocks.OPT_SORT_KEYS
    )
    assert out == b'{\n  "a": 1,\n  "b": 2\n}'
    assert yamlrocks.to_json([1, 2], option=yamlrocks.OPT_INDENT_4) == (
        b"[\n    1,\n    2\n]"
    )


def test_tags_are_dropped():
    """A custom tag carries no JSON meaning; the inner value is emitted."""
    data = yamlrocks.loads(b"x: !mytag 5\n")
    assert yamlrocks.to_json(data) == b'{"x":"5"}'


def test_yaml_to_json_via_loads():
    """The canonical yaml -> json path is loads() then to_json()."""
    src = b"name: app\nports:\n  - 80\n  - 443\nenabled: true\n"
    assert yamlrocks.to_json(yamlrocks.loads(src)) == (
        b'{"name":"app","ports":[80,443],"enabled":true}'
    )


def test_json_import_is_just_loads():
    """JSON is valid YAML 1.2, so loads() already imports it."""
    js = b'{"a": 1, "b": [true, null, 2.5], "c": {"d": "x"}}'
    assert yamlrocks.loads(js) == {"a": 1, "b": [True, None, 2.5], "c": {"d": "x"}}


# -- YAMLRocksDocument / sub-tree export ----------------------------------------------


def test_export_whole_document():
    """to_json resolves a round-trip YAMLRocksDocument to its plain value."""
    doc = yamlrocks.loads(b"a:\n  b: [1, 2]\n", option=RT)
    assert yamlrocks.to_json(doc) == b'{"a":{"b":[1,2]}}'


def test_export_sub_tree_view():
    """to_json accepts a YAMLRocksDocumentView so a sub-tree exports in one call."""
    doc = yamlrocks.loads(b"a:\n  b:\n    c: [1, 2]\n", option=RT)
    assert yamlrocks.to_json(doc["a"]["b"]) == b'{"c":[1,2]}'


def test_export_resolves_aliases():
    """An aliased value is exported as the value it refers to."""
    doc = yamlrocks.loads(b"base: &b\n  x: 1\nref: *b\n", option=RT)
    assert json.loads(yamlrocks.to_json(doc)) == {
        "base": {"x": 1},
        "ref": {"x": 1},
    }


def test_empty_document_is_null():
    """An empty round-trip document projects to null, like empty input."""
    assert yamlrocks.to_json(yamlrocks.loads(b"", option=RT)) == b"null"
    assert yamlrocks.to_json(None) == b"null"


def test_default_callback_for_unknown_types():
    """The default= callback handles types JSON cannot represent natively."""
    out = yamlrocks.to_json({"x": object()}, default=lambda o: "fallback")
    assert out == b'{"x":"fallback"}'
