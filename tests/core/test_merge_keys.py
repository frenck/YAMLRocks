"""YAML merge keys (``<<``), matching PyYAML/ruamel default behavior."""

from __future__ import annotations

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
