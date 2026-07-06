"""YAML merge keys (``<<``), matching PyYAML/ruamel default behavior."""

from __future__ import annotations

import pytest

import yamlrocks


def test_simple_merge():
    """A merge key pulls all keys from the referenced anchor mapping."""
    src = b"base: &b\n  a: 1\n  b: 2\nderived:\n  <<: *b\n  c: 3\n"
    result = yamlrocks.loads(src)
    assert result["derived"] == {"a": 1, "b": 2, "c": 3}


def test_explicit_key_overrides_merge():
    """An explicit key takes precedence over a merged value."""
    src = b"base: &b\n  a: 1\n  b: 2\nderived:\n  <<: *b\n  b: 99\n"
    result = yamlrocks.loads(src)
    assert result["derived"]["b"] == 99
    assert result["derived"]["a"] == 1


def test_multiple_merge_sources():
    """A list of merge sources combines keys from every anchor."""
    src = b"a: &a {x: 1}\nb: &b {y: 2}\nc:\n  <<: [*a, *b]\n  z: 3\n"
    result = yamlrocks.loads(src)
    assert result["c"] == {"x": 1, "y": 2, "z": 3}


def test_earlier_merge_wins():
    """In a merge list, the earlier source wins on conflicting keys."""
    # In `<<: [*a, *b]`, *a is applied before *b, so *a wins on conflicts.
    src = b"a: &a {k: 1}\nb: &b {k: 2}\nc:\n  <<: [*a, *b]\n"
    result = yamlrocks.loads(src)
    assert result["c"]["k"] == 1


def test_nested_merge():
    """A merge key inside a nested mapping merges and keeps local keys."""
    src = (
        b"defaults: &d\n  timeout: 30\n  retries: 3\n"
        b"services:\n  web:\n    <<: *d\n    port: 80\n"
    )
    result = yamlrocks.loads(src)
    assert result["services"]["web"] == {"timeout": 30, "retries": 3, "port": 80}


def test_no_merge_key_unaffected():
    """A mapping without merge keys loads unchanged."""
    assert yamlrocks.loads(b"a: 1\nb: 2\n") == {"a": 1, "b": 2}


# -- Merge value that cannot be merged is preserved, not dropped ---------------


def test_merge_with_custom_tag_value_is_preserved():
    """A `<<` whose value is a custom tag is kept under `<<`, not dropped, so a
    host application can resolve it (e.g. ESPHome's deferred `!include`)."""
    out = yamlrocks.loads(
        b"base:\n  existing: value\n  <<: !t foo\n",
        option=yamlrocks.OPT_YAML_1_1,
        tags={"!t": lambda v: {"merged": "yes"}},
    )["base"]
    assert out == {"existing": "value", "<<": {"merged": "yes"}}


def test_merge_with_tag_preserved_in_loads_all():
    """The same preservation holds on the loads_all (multi-doc) path."""
    out = yamlrocks.loads_all(
        b"base:\n  e: v\n  <<: !t x\n",
        option=yamlrocks.OPT_YAML_1_1,
        tags={"!t": lambda v: {"m": 1}},
    )[0]["base"]
    assert out == {"e": "v", "<<": {"m": 1}}


def test_plain_mapping_merge_still_works():
    """A normal mapping merge is unaffected by the preservation change."""
    out = yamlrocks.loads(
        b"base:\n  e: v\n  <<:\n    a: 1\n", option=yamlrocks.OPT_YAML_1_1
    )
    assert out["base"] == {"e": "v", "a": 1}


def test_empty_merge_value_is_dropped():
    """An empty `<<:` contributes nothing and leaves no `<<` key."""
    out = yamlrocks.loads(b"base:\n  a: 1\n  <<:\n", option=yamlrocks.OPT_YAML_1_1)
    assert out["base"] == {"a": 1}


def test_quoted_merge_key_is_a_literal_string_not_a_merge():
    """A *quoted* ``"<<"`` is an ordinary string key, never a merge directive, so
    its mapping value is kept rather than being merged into the parent."""
    out = yamlrocks.loads(b'obj:\n  "<<":\n    x: 1\n  y: 2\n')["obj"]
    assert out == {"<<": {"x": 1}, "y": 2}


def test_single_quoted_merge_key_is_a_literal_string():
    """The same holds for a single-quoted ``'<<'`` key."""
    out = yamlrocks.loads(b"obj:\n  '<<':\n    x: 1\n  y: 2\n")["obj"]
    assert out == {"<<": {"x": 1}, "y": 2}


def test_plain_merge_still_merges_after_the_quoted_fix():
    """A plain ``<<`` still merges, so the quoted-vs-plain distinction is exact."""
    out = yamlrocks.loads(b"base: &b\n  x: 1\nuse:\n  <<: *b\n  zz: 2\n")["use"]
    assert out == {"x": 1, "zz": 2}


def test_plain_merge_outside_key_position_is_the_literal_string():
    """A plain ``<<`` that is not a mapping key is just the string ``<<``."""
    assert yamlrocks.loads(b"k: <<\n") == {"k": "<<"}
    assert yamlrocks.loads(b"- <<\n") == ["<<"]


def test_merge_key_inside_a_complex_key_does_not_leak_to_tag_handler():
    """A ``<<`` nested in a complex (mapping) key is literal data: it stays the
    string ``<<`` and never surfaces the internal merge tag to a tag handler."""
    seen = []

    def handler(tag, value):
        seen.append((tag, value))
        return value

    out = yamlrocks.loads(b"{<<: 1}: v\n", tag_handler=handler)
    assert out == {(("<<", 1),): "v"}
    assert seen == []


def test_merge_applied_on_all_data_paths_not_just_fast():
    """`<<` merges must be applied on every path that returns DATA (annotated,
    includes, round-trip to_dict), matching the fast `loads()`. Round-trip
    `to_yaml()` still keeps `<<` literal for byte-for-byte fidelity."""
    src = b"d: &d\n  x: 1\no:\n  <<: *d\n  y: 9\n"
    expected = {"x": 1, "y": 9}
    assert yamlrocks.loads(src)["o"] == expected
    assert dict(yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)["o"]) == expected
    assert yamlrocks.loads(src, option=yamlrocks.OPT_INCLUDES)["o"] == expected
    doc = yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.to_dict()["o"] == expected
    # Fidelity: the emitted source still carries the literal merge key.
    assert doc.to_yaml() == src


def test_merge_via_include_is_applied_after_resolution(tmp_path):
    """`<<: !include base.yaml` merges the included mapping in, the Home Assistant
    packages pattern, instead of leaving a literal `<<` key."""
    (tmp_path / "base.yaml").write_bytes(b"x: 1\ny: 2\n")
    root = b"obj:\n  <<: !include base.yaml\n  y: 9\n"
    out = yamlrocks.loads(
        root, option=yamlrocks.OPT_INCLUDES, include_dir=str(tmp_path)
    )
    assert out == {"obj": {"x": 1, "y": 9}}


def test_quoted_merge_key_stays_literal_on_data_paths():
    """A quoted `"<<"` is a literal key, never a merge, on the AST paths too."""
    src = b'o:\n  "<<": 1\n'
    assert yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)["o"] == {"<<": 1}
    assert yamlrocks.loads(src, option=yamlrocks.OPT_INCLUDES)["o"] == {"<<": 1}


def test_merge_sequence_with_non_mergeable_element_keeps_only_leftover():
    """A `<<` sequence mixing a mapping and a non-mergeable element merges the
    mapping and keeps only the leftover under the literal `<<`, not the whole
    list, so the merged keys are not also duplicated. All data paths agree with
    the fast path (mirrors the fast `merge_into` leftover semantics)."""
    src = b"base: &b\n  x: 1\no:\n  <<: [*b, plain]\n"
    expected = {"x": 1, "<<": ["plain"]}
    assert yamlrocks.loads(src)["o"] == expected
    assert dict(yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)["o"]) == expected
    assert yamlrocks.loads(src, option=yamlrocks.OPT_INCLUDES)["o"] == expected
    assert (
        yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_dict()["o"] == expected
    )


@pytest.mark.parametrize(
    ("src", "order"),
    [
        # A `<<` expands at its own position; explicit keys keep their spot.
        (b"base: &b {x: 1}\nuse:\n  <<: *b\n  y: 2\n", ["x", "y"]),
        (b"base: &b {x: 1}\nuse:\n  y: 2\n  <<: *b\n", ["y", "x"]),
        # An explicit key overrides a merged one in place (keeps its position).
        (b"base: &b {x: 1, z: 9}\nuse:\n  z: 0\n  <<: *b\n  y: 2\n", ["z", "x", "y"]),
        # A list merge expands each source in order, at the `<<` position.
        (
            b"a: &a {p: 1}\nb: &b {q: 2}\nuse:\n  <<: [*a, *b]\n  r: 3\n",
            ["p", "q", "r"],
        ),
    ],
)
def test_merge_key_order_is_source_order_on_every_path(src, order):
    """Merged keys land at the `<<` position and explicit keys keep their written
    spot, and the fast, annotated, and round-trip paths all agree on that order
    (previously the fast path put explicit keys first, diverging from the others)."""
    fast = list(yamlrocks.loads(src)["use"].keys())
    annotated = list(dict(yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)["use"]))
    round_trip = list(
        yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_dict()["use"].keys()
    )
    assert fast == annotated == round_trip == order


def test_duplicate_key_in_merge_source_uses_last_value():
    """A `<<` source mapping that repeats a key contributes that key's LAST value
    (the value it has as data), not the first, matching how the same anchor
    materializes on its own and every other path (and PyYAML). An explicit key in
    the target still wins over the merged one."""
    # `&a {x: 1, x: 2}` is `{x: 2}` as data, so merging it gives x = 2, not 1.
    assert yamlrocks.loads(b"a: &a {x: 1, x: 2}\nc:\n  <<: *a\n")["c"] == {"x": 2}
    # Block-style anchor with a repeated key behaves the same.
    assert yamlrocks.loads(b"a: &a\n  x: 1\n  x: 2\nc:\n  <<: *a\n")["c"] == {"x": 2}
    # An explicit target key still overrides the (deduped) merged value.
    assert yamlrocks.loads(b"a: &a {x: 1, x: 2}\nc:\n  x: 9\n  <<: *a\n")["c"] == {
        "x": 9
    }


def test_null_merge_value_is_dropped_on_every_path():
    """A null `<<` value (`<<: ~`, an empty `<<:`, or a null element of a merge
    list) contributes nothing and leaves no `<<` key, on every data path. The
    composer paths used to keep a literal `<<: None`, diverging from the fast
    path; they now agree."""
    for src, expected in [
        (b"base:\n  a: 1\n  <<: ~\n", {"a": 1}),
        (b"base:\n  a: 1\n  <<:\n", {"a": 1}),
        (b"a: &a {x: 1}\nbase:\n  <<: [*a, ~]\n", {"x": 1}),
    ]:
        assert yamlrocks.loads(src)["base"] == expected
        assert (
            dict(yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED)["base"])
            == expected
        )
        assert yamlrocks.loads(src, option=yamlrocks.OPT_INCLUDES)["base"] == expected
        assert (
            yamlrocks.loads(src, option=yamlrocks.OPT_ROUND_TRIP).to_dict()["base"]
            == expected
        )


def test_repeated_unmergeable_merge_keys_collect_into_a_list():
    """A mapping may carry `<<` more than once (`<<: !include a` / `<<: !include
    b`). When the values are deferred (resolved by the host, not mergeable here),
    a Python dict cannot hold two `<<` slots, so both are kept under one `<<` as
    a list in document order, matching the `<<: [a, b]` sequence form. This lets
    a host like ESPHome run its own repeated-merge pass instead of silently
    losing all but one `<<`."""
    tags = {"!x": lambda v: f"<{v}>"}
    src = b"<<: !x a\n<<: !x b\n"
    expected = {"<<": ["<a>", "<b>"]}
    # Fast path and annotated path resolve the tag and collect identically.
    assert yamlrocks.loads(src, tags=tags) == expected
    assert (
        dict(yamlrocks.loads(src, option=yamlrocks.OPT_ANNOTATED, tags=tags))
        == expected
    )
    # The sequence form already produced this shape and still does.
    assert yamlrocks.loads(b"<<: [!x a, !x b]\n", tags=tags) == expected
    # ESPHome's exact option combination (the reported case).
    esphome = (
        yamlrocks.OPT_ANNOTATED
        | yamlrocks.OPT_PYYAML_COMPAT
        | yamlrocks.OPT_ANNOTATE_NUMBERS
        | yamlrocks.OPT_DUPLICATE_KEYS_ERROR
    )
    assert dict(yamlrocks.loads(src, option=esphome, tags=tags)) == expected


def test_repeated_merge_flattens_a_preserved_sequence_one_level():
    """A `<<` sequence value meeting a later `<<` flattens into one list rather
    than nesting, so either order yields the same flat sequence."""
    tags = {"!x": lambda v: f"<{v}>"}
    flat = {"<<": ["<a>", "<b>", "<c>"]}
    assert yamlrocks.loads(b"<<: [!x a, !x b]\n<<: !x c\n", tags=tags) == flat
    assert yamlrocks.loads(b"<<: !x a\n<<: [!x b, !x c]\n", tags=tags) == flat


def test_repeated_merge_mixes_mergeable_and_deferred():
    """A mergeable `<<` (a plain mapping) still folds into the dict; only the
    deferred `<<` values are preserved, keeping document order."""
    tags = {"!x": lambda v: f"<{v}>"}
    assert yamlrocks.loads(b"<<: {p: 1}\n<<: !x b\n", tags=tags) == {
        "p": 1,
        "<<": "<b>",
    }
    assert yamlrocks.loads(b"<<: !x a\n<<: {p: 1}\n", tags=tags) == {
        "<<": "<a>",
        "p": 1,
    }
