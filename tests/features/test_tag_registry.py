"""YAMLRocksTag registration: the `tags=` mapping, the YAMLRocksTags registry, and composition.

`tags` maps a tag name to a function that receives only the resolved inner
value (the name is implied by which key matched). A registered tag wins over a
`tag_handler` catch-all, which still handles anything not in the registry.
"""

from __future__ import annotations

import pytest

import yamlrocks

# -- The plain mapping form --------------------------------------------------


def test_mapping_function_receives_value_only():
    """A `tags` entry's function is called with just the inner value."""
    assert yamlrocks.loads(b"point: !vec [1, 2]", tags={"!vec": tuple}) == {
        "point": (1, 2)
    }


def test_mapping_builtin_as_handler():
    """A builtin like str.upper drops straight in as a per-tag function."""
    assert yamlrocks.loads(b"a: !upper hello", tags={"!upper": str.upper}) == {
        "a": "HELLO"
    }


def test_mapping_multiple_tags():
    """Several tags can be registered in one mapping."""
    out = yamlrocks.loads(
        b"a: !upper hi\nb: !double 21",
        tags={"!upper": str.upper, "!double": lambda v: int(v) * 2},
    )
    assert out == {"a": "HI", "b": 42}


def test_mapping_on_collection_value():
    """The function receives a parsed mapping/sequence for non-scalar nodes."""
    out = yamlrocks.loads(
        b"p: !point\n  x: 1\n  y: 2\n",
        tags={"!point": lambda v: (v["x"], v["y"])},
    )
    assert out == {"p": (1, 2)}


def test_unregistered_tag_falls_back_to_default():
    """A tag absent from the registry keeps the default drop-and-keep behavior."""
    assert yamlrocks.loads(b"a: !unknown foo", tags={"!vec": tuple}) == {"a": "foo"}


# -- The YAMLRocksTags registry object ------------------------------------------------


def test_tags_is_a_dict():
    """YAMLRocksTags is a dict subclass, so it can be built and inspected like one."""
    tags = yamlrocks.YAMLRocksTags()
    tags["!vec"] = tuple
    assert isinstance(tags, dict)
    assert tags["!vec"] is tuple


def test_tags_register_direct_call():
    """register(name, func) registers immediately and returns the function."""
    tags = yamlrocks.YAMLRocksTags()
    returned = tags.register("!double", lambda v: int(v) * 2)
    assert tags["!double"] is returned
    assert yamlrocks.loads(b"n: !double 4", tags=tags) == {"n": 8}


def test_tags_register_decorator():
    """register(name) is a decorator that leaves the function bound to its name."""
    tags = yamlrocks.YAMLRocksTags()

    @tags.register("!vec")
    def make_vec(value):
        return tuple(value)

    assert make_vec([1, 2]) == (1, 2)  # the name still refers to the function
    assert yamlrocks.loads(b"p: !vec [1, 2]", tags=tags) == {"p": (1, 2)}


def test_tags_reusable_across_calls():
    """One registry can be reused for many loads calls."""
    tags = yamlrocks.YAMLRocksTags()
    tags.register("!upper", str.upper)
    assert yamlrocks.loads(b"a: !upper x", tags=tags) == {"a": "X"}
    assert yamlrocks.loads(b"b: !upper y", tags=tags) == {"b": "Y"}


# -- Composition with tag_handler --------------------------------------------


def test_registry_takes_precedence_over_handler():
    """A registered tag wins; the handler covers everything else."""
    out = yamlrocks.loads(
        b"a: !vec [1, 2]\nb: !other x",
        tags={"!vec": tuple},
        tag_handler=lambda tag, value: f"handled:{value}",
    )
    assert out == {"a": (1, 2), "b": "handled:x"}


def test_handler_only_still_works():
    """Passing only tag_handler keeps its existing behavior."""
    out = yamlrocks.loads(
        b"x: !greet world", tag_handler=lambda tag, value: f"{tag}={value}"
    )
    assert out == {"x": "!greet=world"}


def test_nested_registered_tags_resolve_inside_out():
    """A registered tag receives an already-resolved inner tagged value."""
    tags = yamlrocks.YAMLRocksTags()
    tags.register("!a", lambda v: ("a", v))
    tags.register("!b", lambda v: ("b", v))
    out = yamlrocks.loads(b"outer: !a\n  inner: !b value\n", tags=tags)
    assert out == {"outer": ("a", {"inner": ("b", "value")})}


# -- Reach and validation ----------------------------------------------------


def test_tags_applied_in_loads_all():
    """loads_all honors the registry for every document."""
    out = yamlrocks.loads_all(
        b"---\na: !double 1\n---\nb: !double 2", tags={"!double": lambda v: int(v) * 2}
    )
    assert out == [{"a": 2}, {"b": 4}]


def test_tags_with_passthrough_registry_wins():
    """A registered tag is resolved even when OPT_PASSTHROUGH_TAG is set."""
    out = yamlrocks.loads(
        b"a: !vec [1, 2]\nb: !other x",
        option=yamlrocks.OPT_PASSTHROUGH_TAG,
        tags={"!vec": tuple},
    )
    assert out["a"] == (1, 2)
    # The unregistered tag still passes through as a YAMLRocksTag object.
    assert isinstance(out["b"], yamlrocks.YAMLRocksTag)
    assert out["b"].tag == "!other"


def test_invalid_tags_type_raises():
    """A non-mapping tags argument is a clear TypeError."""
    with pytest.raises(TypeError, match="tags must be a dict"):
        yamlrocks.loads(b"a: 1", tags=[1, 2, 3])
