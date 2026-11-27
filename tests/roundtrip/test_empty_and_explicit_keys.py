"""Empty and unusual mapping keys across a re-emit.

Once anything in a document changes, the emitter rebuilds it from its tree.
A key the author left empty (``: value``, a bare ``?``) must come back empty, an
explicit ``?`` key must stay explicit, and a key that cannot be written on one
line must switch to the explicit form instead of turning into YAML that no
longer loads. Each source here starts with an edit target ``a: 2`` that the edit
leaves as is, so the rest of the document is written from the tree.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def keep_tag(tag: str, value: Any) -> tuple[str, Any]:
    """Keep a custom tag visible in the loaded data."""
    return (tag, value)


def edited(src: str, option: int = 0) -> bytes:
    """Load ``src`` for round-trip, set its top-level `a` to 2, and re-emit."""
    doc = yamlrocks.loads(src, option=RT | option)
    doc["a"] = 2
    return doc.to_yaml()


def assert_same_data(src: str, out: bytes) -> None:
    """``out`` loads to the same data as ``src``, with and without a tag handler."""
    assert yamlrocks.loads_all(out) == yamlrocks.loads_all(src)
    assert yamlrocks.loads_all(out, tag_handler=keep_tag) == yamlrocks.loads_all(
        src, tag_handler=keep_tag
    )


# Shapes whose source form is kept as written.
KEPT = [
    pytest.param("a: 2\n: v\n", id="block-empty-key"),
    pytest.param("a: 2\n:\n", id="block-empty-key-and-value"),
    pytest.param("a: 2\n: # note\n  child: v\n", id="block-empty-key-comment"),
    pytest.param("a: 2\ns:\n- : v\n", id="sequence-item-empty-key"),
    pytest.param("a: 2\n?\n: v\n", id="explicit-empty-key"),
    pytest.param("a: 2\n? !t\n: v\n", id="explicit-tagged-empty-key"),
    pytest.param("a: 2\n? &x\n: v\n", id="explicit-anchored-empty-key"),
    pytest.param("a: 2\n? !!str\n: v\n", id="explicit-str-empty-key"),
    pytest.param("a: 2\n? [b, c]\n: v\n", id="explicit-flow-key"),
    pytest.param("a: 2\nf: {: v}\n", id="flow-mapping-empty-key"),
    pytest.param("a: 2\nf: {b: 1, : v}\n", id="flow-mapping-later-empty-key"),
    pytest.param("a: 2\nf: {? b: c}\n", id="flow-mapping-explicit-key"),
    pytest.param("a: 2\nf: {? : c}\n", id="flow-mapping-explicit-empty-key"),
    pytest.param("a: 2\n-: v\n", id="dash-key"),
    pytest.param("a: 2\n?: v\n", id="question-mark-key"),
    pytest.param("a: 2\nb:: v\n", id="key-ending-in-colon"),
    pytest.param("a: 2\nk:\n&x c: 3\n", id="anchored-key-after-empty-value"),
    pytest.param("a: 2\nk:\n!!str c: 3\n", id="tagged-key-after-empty-value"),
]


@pytest.mark.parametrize("src", KEPT)
def test_key_reemits_as_written(src: str) -> None:
    """An empty, explicit, or indicator-like key comes back as the author wrote it."""
    out = edited(src)

    assert out == src.encode()
    assert_same_data(src, out)


# Shapes whose source form cannot be kept as is, with what is written instead.
REWRITTEN = [
    pytest.param(
        "a: 2\n?\n&x : 1\n",
        "a: 2\n?\n:\n&x : 1\n",
        id="explicit-empty-key-then-anchored-empty-key",
    ),
    pytest.param(
        "a: 2\n? b\n&x c: 3\n",
        "a: 2\n? b\n:\n&x c: 3\n",
        id="explicit-key-then-anchored-key",
    ),
    pytest.param(
        "a: 2\nf: [: v]\n",
        "a: 2\nf: [{: v}]\n",
        id="flow-sequence-pair-empty-key",
    ),
    pytest.param(
        "a: 2\nf: {i: -:}\n",
        'a: 2\nf: {i: "-", : null}\n',
        id="flow-dash-before-empty-pair",
    ),
    pytest.param(
        "a: 2\nf: {i: ?:}\n",
        'a: 2\nf: {i: "?", : null}\n',
        id="flow-question-mark-before-empty-pair",
    ),
    pytest.param(
        "a: 2\n[b]:\t#\n",
        "a: 2\n? [\n    b   #\n  ]\n:\n",
        id="comment-after-flow-sequence-key",
    ),
    pytest.param(
        "a: 2\nk:\n  [b]: #\n",
        "a: 2\nk:\n  ? [\n      b   #\n    ]\n  :\n",
        id="comment-after-nested-flow-key",
    ),
    pytest.param(
        "a: 2\n? |+\n x",
        "a: 2\n? |-\n  x\n:\n",
        id="explicit-keep-block-key-at-end",
    ),
    pytest.param(
        "a: 2\n? !t |+\n x",
        "a: 2\n? !t |-\n  x\n:\n",
        id="explicit-tagged-keep-block-key-at-end",
    ),
]


@pytest.mark.parametrize(("src", "out"), REWRITTEN)
def test_key_rewritten_where_its_source_form_does_not_reload(
    src: str, out: str
) -> None:
    """A key whose source form would not reload is rewritten to one that does."""
    reemitted = edited(src)

    assert reemitted == out.encode()
    assert_same_data(src, reemitted)


@pytest.mark.parametrize(
    "option",
    [
        pytest.param(0, id="null-as-empty"),
        pytest.param(yamlrocks.OPT_NULL_AS_TILDE, id="null-as-tilde"),
        pytest.param(yamlrocks.OPT_NULL_AS_KEYWORD, id="null-as-keyword"),
    ],
)
def test_loaded_empty_key_ignores_the_null_style(option: int) -> None:
    """A key empty in its source stays empty whatever the document's null style."""
    src = "a: 2\n: v\nf: {: w}\n"

    assert edited(src, option) == src.encode()


def test_assigned_none_key_follows_the_null_style() -> None:
    """A key assigned as ``None`` through the API is written as a null token."""
    doc = yamlrocks.loads("a: 1\n", option=RT)
    doc[None] = 2

    assert doc.to_yaml() == b"a: 1\nnull: 2\n"

    doc = yamlrocks.loads("a: 1\n", option=RT | yamlrocks.OPT_NULL_AS_TILDE)
    doc[None] = 2

    assert doc.to_yaml() == b"a: 1\n~: 2\n"


@pytest.mark.parametrize(
    ("src", "out"),
    [
        pytest.param(
            "-  e\n ::\n", '-  "e :"\n- 3\n', id="folds-to-trailing-space-colon"
        ),
        pytest.param(
            "-   e\n a::\n", '-   "e a:"\n- 3\n', id="folds-to-trailing-colon"
        ),
    ],
)
def test_folded_value_ending_in_a_colon_is_quoted(src: str, out: str) -> None:
    """A plain value folded to end in `:` is quoted, not replayed as a key."""
    doc = yamlrocks.loads(src, option=RT)
    first = yamlrocks.loads(src)[0]
    doc[1] = 3
    reemitted = doc.to_yaml()

    assert reemitted == out.encode()
    assert yamlrocks.loads(reemitted) == [first, 3]


def test_keep_block_scalar_at_end_survives_a_key_added_after_it() -> None:
    """A ``|+`` scalar that ended the input keeps its value when a key follows."""
    src = "a: 1\nk: |+\n  x"
    doc = yamlrocks.loads(src, option=RT)
    doc["b"] = 3
    out = doc.to_yaml()

    assert out == b"a: 1\nk: |-\n  x\nb: 3\n"
    assert yamlrocks.loads(out) == {"a": 1, "k": "x", "b": 3}


def test_keep_block_scalar_at_end_still_reemits_as_written() -> None:
    """A ``|+`` scalar that still ends the output keeps its source form."""
    src = "a: 2\nk: |+\n  x"

    assert edited(src) == src.encode()


def test_empty_keys_reemit_from_a_file(tmp_path: Path) -> None:
    """A file with empty keys written back after an edit keeps them empty."""
    path = tmp_path / "config.yaml"
    path.write_bytes(b"a: 1\n: v\n?\n: w\n")
    doc = yamlrocks.loads(path.read_bytes(), option=RT)
    doc["a"] = 2

    assert doc.to_yaml() == b"a: 2\n: v\n?\n: w\n"


def test_detached_copy_of_an_unterminated_keep_scalar_keeps_both_values():
    """A keep scalar that ended the input reads back unchanged when a detached copy repeats it."""
    doc = yamlrocks.loads(b"a: &anchor\n  k: |+\n    x", option=RT)
    doc["b"] = {}
    doc.node["b"].make_alias("anchor")
    doc.node["b"].detach()

    assert yamlrocks.loads(doc.to_yaml()) == {"a": {"k": "x"}, "b": {"k": "x"}}
