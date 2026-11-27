"""Multi-line plain and quoted scalars across a re-emit.

A plain or quoted scalar the author spread over several lines folds into a value
that no longer says where those lines were. One blank line in the source folds
to a ``\\n`` in the value. Once anything in the document changes, the emitter
rebuilds the document from its tree, and that scalar must still reload to the
same value. Where its source lines still fit, it comes back exactly as written;
where they do not, it is rebuilt on one line instead.
"""

from __future__ import annotations

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def edited(src: str, key: str | int = "a", value: object = 2) -> bytes:
    """Load ``src`` for round-trip, replace one top-level value, and re-emit."""
    doc = yamlrocks.loads(src, option=RT)
    doc[key] = value
    return doc.to_yaml()


def expected(src: str, key: str | int = "a", value: object = 2) -> object:
    """The data ``src`` holds once the same edit is applied to a plain load."""
    data = yamlrocks.loads(src)
    data[key] = value
    return data


# Each source carries an edit target `a` (or item 0) plus a scalar elsewhere that
# the edit does not touch. After the edit, that scalar must come back as written.
UNTOUCHED = [
    pytest.param(
        "a: 1\nb: text\n\n  ...\nc: 3\n",
        "a: 2\nb: text\n\n  ...\nc: 3\n",
        id="plain-continuation-looks-like-a-marker",
    ),
    pytest.param(
        "a: 1\nb: text\n\n  more\n",
        "a: 2\nb: text\n\n  more\n",
        id="plain-in-mapping",
    ),
    pytest.param(
        "a: 1\nb: x\n\n\n  y\n",
        "a: 2\nb: x\n\n\n  y\n",
        id="plain-two-blank-lines",
    ),
    pytest.param(
        "a: 1\nb: x\n  y\n",
        "a: 2\nb: x\n  y\n",
        id="plain-folded-without-blank-line",
    ),
    pytest.param(
        "a: 1\nr:\n  - x\n\n    y\n  - z\n",
        "a: 2\nr:\n  - x\n\n    y\n  - z\n",
        id="plain-in-block-sequence",
    ),
    pytest.param(
        "a: 1\nf: [x\n\n  y, z]\n",
        "a: 2\nf: [x\n\n  y, z]\n",
        id="plain-in-flow-sequence",
    ),
    pytest.param(
        "a: 1\nf: {k: x\n\n  y}\n",
        "a: 2\nf: {k: x\n\n  y}\n",
        id="plain-in-flow-mapping",
    ),
    pytest.param(
        "a: 1\nb: 'x\n\n  y'\n",
        "a: 2\nb: 'x\n\n  y'\n",
        id="single-quoted-in-mapping",
    ),
    pytest.param(
        "a: 1\nr:\n  - 'a\n\n    b'\n",
        "a: 2\nr:\n  - 'a\n\n    b'\n",
        id="single-quoted-in-block-sequence",
    ),
    pytest.param(
        'a: 1\nb: "x\n\n  y"\n',
        'a: 2\nb: "x\n\n  y"\n',
        id="double-quoted-in-mapping",
    ),
    pytest.param(
        "a: 1\nb: !t &n x\n\n  y\nc: *n\n",
        "a: 2\nb: !t &n x\n\n  y\nc: *n\n",
        id="plain-with-tag-and-anchor",
    ),
    pytest.param(
        "a: 1\nb: |\n  x\n\n  y\n",
        "a: 2\nb: |\n  x\n\n  y\n",
        id="literal-block-scalar",
    ),
    pytest.param(
        "a: 1\nb: >\n  x\n\n  y\n",
        "a: 2\nb: >\n  x\n\n  y\n",
        id="folded-block-scalar",
    ),
]


@pytest.mark.parametrize(("src", "out"), UNTOUCHED)
def test_untouched_multiline_scalar_survives_an_edit_elsewhere(
    src: str, out: str
) -> None:
    """An edit elsewhere re-emits an untouched multi-line scalar exactly as written."""
    emitted = edited(src)

    assert emitted == out.encode()
    assert yamlrocks.loads(emitted) == expected(src)


def test_untouched_multiline_scalar_in_a_sequence_of_mappings() -> None:
    """A multi-line plain inside a sequence item mapping survives an item edit."""
    src = "- a: 1\n  b: x\n\n    y\n- q\n"
    emitted = edited(src, key=1, value="r")

    assert emitted == b"- a: 1\n  b: x\n\n    y\n- r\n"
    assert yamlrocks.loads(emitted) == expected(src, key=1, value="r")


def test_view_of_an_unmodified_document_keeps_a_multiline_scalar() -> None:
    """A view's re-emit keeps a nested multi-line plain scalar loadable."""
    src = "m:\n  b: text\n\n    more\n"
    doc = yamlrocks.loads(src, option=RT)
    emitted = doc["m"].to_yaml()

    assert emitted == b"b: text\n\n    more\n"
    assert yamlrocks.loads(emitted) == {"b": "text\nmore"}


def test_root_multiline_plain_scalar_survives_a_forced_re_emit() -> None:
    """A root multi-line plain scalar at column 0 replays as written."""
    doc = yamlrocks.loads("text\n\nmore\n", option=RT)
    emitted = yamlrocks.loads(doc.to_yaml(), option=RT).to_yaml()

    assert emitted == b"text\n\nmore\n"


@pytest.mark.parametrize(
    "src",
    [
        pytest.param("z: 1\na:\n b: x\n\n  y\n", id="plain"),
        pytest.param("z: 1\na:\n b: 'x\n\n  y'\n", id="single-quoted"),
    ],
)
def test_multiline_scalar_too_shallow_for_its_new_indent_is_rebuilt(src: str) -> None:
    """A scalar whose source lines no longer clear its key is double-quoted."""
    # The emitter writes nested mappings two columns in, so the source's
    # continuation line at column 2 no longer sits past the re-indented key.
    emitted = edited(src, key="z")

    assert emitted == b'z: 2\na:\n  b: "x\\ny"\n'
    assert yamlrocks.loads(emitted) == expected(src, key="z")


@pytest.mark.parametrize(
    ("src", "out"),
    [
        pytest.param(
            "a: 1\n? x\n\n  y\n: v\n",
            'a: 2\n? "x\\ny"\n: v\n',
            id="explicit-block-key",
        ),
        pytest.param(
            "a: 1\nf: {x\n\n  y: v}\n",
            'a: 2\nf: {"x\\ny": v}\n',
            id="flow-mapping-key",
        ),
    ],
)
def test_multiline_key_is_rebuilt_on_one_line(src: str, out: str) -> None:
    """A multi-line scalar key is double-quoted on re-emit, never written raw."""
    emitted = edited(src)

    assert emitted == out.encode()
    assert yamlrocks.loads(emitted) == expected(src)


def test_edited_plain_value_with_a_line_break_is_double_quoted() -> None:
    """A plain scalar edited to hold a line break re-emits double-quoted."""
    doc = yamlrocks.loads("a: 1\nb: text\n\n  more\n", option=RT)
    doc.node["b"].value = "new\nvalue"
    emitted = doc.to_yaml()

    assert emitted == b'a: 1\nb: "new\\nvalue"\n'
    assert yamlrocks.loads(emitted) == {"a": 1, "b": "new\nvalue"}


@pytest.mark.parametrize(
    "src",
    [
        pytest.param("a: 1\nb: text\n\n  more\n", id="plain"),
        pytest.param("a: 1\nb: 'text\n\n  more'\n", id="single-quoted"),
    ],
)
def test_commenting_a_multiline_scalar_keeps_its_value(src: str) -> None:
    """Setting a comment on a multi-line scalar re-emits it double-quoted."""
    doc = yamlrocks.loads(src, option=RT)
    doc.node["b"].comment = "note"
    emitted = doc.to_yaml()

    assert emitted == b'a: 1\nb: "text\\nmore" # note\n'
    assert yamlrocks.loads(emitted) == yamlrocks.loads(src)
