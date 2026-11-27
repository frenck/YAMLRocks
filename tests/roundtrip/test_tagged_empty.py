"""Tagged empty nodes across a re-emit.

A node can be written as its tag alone, with no content (``key: !t``). It loads
as the empty value under that tag: ``!!str`` alone is the empty string, ``!!null``
alone is null, and a custom tag hands its handler an empty string. Once anything
in the document changes, the emitter rebuilds the document from its tree, and
that node must come back as its tag alone. A ``null`` token after the tag would
reload as the string "null" instead.
"""

from __future__ import annotations

from typing import Any

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP

NULL_STYLES = [
    pytest.param(0, id="null-as-empty"),
    pytest.param(yamlrocks.OPT_NULL_AS_TILDE, id="null-as-tilde"),
    pytest.param(yamlrocks.OPT_NULL_AS_KEYWORD, id="null-as-keyword"),
]


def keep_tag(tag: str, value: Any) -> tuple[str, Any]:
    """Keep a custom tag visible in the loaded data."""
    return (tag, value)


def edited(src: str, option: int = 0) -> bytes:
    """Load ``src`` for round-trip, change its top-level `a`, and re-emit."""
    doc = yamlrocks.loads(src, option=RT | option)
    doc["a"] = 2
    return doc.to_yaml()


def assert_same_data(src: str, out: bytes) -> None:
    """``out`` loads to the same data as ``src``, with and without a tag handler."""
    assert yamlrocks.loads_all(out) == yamlrocks.loads_all(src)
    assert yamlrocks.loads_all(out, tag_handler=keep_tag) == yamlrocks.loads_all(
        src, tag_handler=keep_tag
    )


# Each source starts with an edit target `a: 2` that the edit leaves as is, so
# the whole document must come back byte for byte.
SHAPES = [
    pytest.param("a: 2\nk: !t\n", id="mapping-value"),
    pytest.param("a: 2\nk: !t # note\n", id="mapping-value-comment"),
    pytest.param("a: 2\nk: !t &x # note\n", id="mapping-value-anchor-comment"),
    pytest.param("a: 2\nk: &x # note\n", id="mapping-value-anchor-only-comment"),
    pytest.param("a: 2\nk: !!str # note\n", id="mapping-value-str-comment"),
    pytest.param("a: 2\nk: !!null # note\n", id="mapping-value-null-comment"),
    pytest.param("a: 2\ns:\n- !t\n- 2\n", id="sequence-item"),
    pytest.param("a: 2\ns:\n- !t # note\n- 2\n", id="sequence-item-comment"),
    pytest.param("a: 2\ns:\n- !t &x # note\n- 2\n", id="sequence-item-anchor-comment"),
    pytest.param("a: 2\ns:\n- !!str # note\n", id="sequence-item-str-comment"),
    pytest.param("a: 2\n!t : v\n", id="mapping-key"),
    pytest.param("a: 2\n!!str : v\n", id="mapping-key-str"),
    pytest.param("a: 2\n!!null : v\n", id="mapping-key-null"),
    pytest.param("a: 2\n!t &x : v\n", id="mapping-key-anchor"),
    pytest.param("a: 2\nf: [!t , b]\n", id="flow-sequence-entry"),
    pytest.param("a: 2\nf: [b, !!str ]\n", id="flow-sequence-last-entry"),
    pytest.param("a: 2\nf: {b: !t , c: 1}\n", id="flow-mapping-value"),
    pytest.param("a: 2\nf: {!!str : c}\n", id="flow-mapping-key"),
]


@pytest.mark.parametrize("option", NULL_STYLES)
@pytest.mark.parametrize("src", SHAPES)
def test_tagged_empty_node_reemits_as_written(src: str, option: int) -> None:
    """A tagged empty node comes back as its tag alone under every null style."""
    out = edited(src, option)

    assert out == src.encode()
    assert_same_data(src, out)


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param(
            "a: 2\nk: !t # note\n  # below\nz: 1\n",
            "a: 2\nk: !t # note\n  # below\nz: 1\n",
            id="mapping-value-comment-below",
        ),
        pytest.param(
            "a: 2\ns:\n- !t # note\n  # below\n- 2\n",
            "a: 2\ns:\n- !t # note\n  # below\n- 2\n",
            id="sequence-item-comment-below",
        ),
        pytest.param(
            "a: 2\nf: [\n  !t , # note\n  b\n  ]\n",
            "a: 2\nf: [\n    !t ,  # note\n    b\n  ]\n",
            id="commented-flow-sequence",
        ),
        pytest.param(
            "a: 2\nw: {\n  foo : !!str,\n  !!str : bar,\n  }\n",
            "a: 2\nw: {foo: !!str , !!str : bar}\n",
            id="yaml-test-suite-WZ62",
        ),
    ],
)
def test_tagged_empty_node_keeps_its_data(src: str, expected: str) -> None:
    """A tagged empty node rebuilt in a new layout still loads as empty."""
    out = edited(src)

    assert out == expected.encode()
    assert_same_data(src, out)


def test_core_tags_on_empty_nodes_keep_their_meaning() -> None:
    """Empty ``!!str`` reloads as the empty string and empty ``!!null`` as null."""
    out = edited("a: 1\ns: !!str\nn: !!null\nl: [!!str , !!null ]\n")

    assert yamlrocks.loads(out) == {"a": 2, "s": "", "n": None, "l": ["", None]}


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param("---\n!t\n", "---\n!t\n", id="tag"),
        pytest.param("---\n!t &x\n", "---\n!t &x\n", id="tag-anchor"),
        pytest.param("---\n!!str\n", "---\n!!str\n", id="str"),
        pytest.param("--- !t\n", "---\n!t\n", id="tag-on-document-start"),
        pytest.param("---\n!\n", "---\n!\n", id="yaml-test-suite-UKK6-02"),
        pytest.param(
            "---\n- !!str\n-\n  !!null : a\n  b: !!str\n- !!str : !!null\n",
            "---\n- !!str\n- !!null : a\n  b: !!str\n- !!str : !!null\n",
            id="yaml-test-suite-FH7J",
        ),
    ],
)
def test_tagged_empty_document_survives_upgrade(src: str, expected: str) -> None:
    """A tagged empty node in a document ``upgrade()`` rebuilds still loads as empty."""
    out = yamlrocks.upgrade("x: yes\n" + src)

    assert out == f"%YAML 1.2\n---\nx: true\n{expected}".encode()
    assert_same_data("x: true\n" + src, out)


@pytest.mark.parametrize(
    ("option", "token"),
    [
        pytest.param(0, "", id="null-as-empty"),
        pytest.param(yamlrocks.OPT_NULL_AS_TILDE, " ~", id="null-as-tilde"),
        pytest.param(yamlrocks.OPT_NULL_AS_KEYWORD, " null", id="null-as-keyword"),
    ],
)
def test_assigned_none_still_follows_the_null_style(option: int, token: str) -> None:
    """A ``None`` assigned next to a tagged empty node follows the null style."""
    doc = yamlrocks.loads("a: 1\nk: !t\n", option=RT | option)
    doc["a"] = None

    assert doc.to_yaml() == f"a:{token}\nk: !t\n".encode()
