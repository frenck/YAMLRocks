"""Plain text the emitter writes at column 0, across a re-emit.

A root scalar and the keys of a root block mapping are written at column 0,
whatever indentation the source gave them. There a leading ``%`` opens a
directive, and ``---`` or ``...`` followed by white space is a document marker,
so plain text that starts that way is quoted. Anywhere else on a line it stays
plain.
"""

from __future__ import annotations

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP


def commented_root(src: str) -> bytes:
    """Load ``src`` for round-trip, comment its root, and re-emit it."""
    doc = yamlrocks.loads(src, option=RT)
    doc.locate([]).comment = "note"
    return doc.to_yaml()


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param(" %x\n", b'"%x" # note\n', id="directive-start"),
        pytest.param("\t%\t\n", b'"%" # note\n', id="lone-percent"),
        pytest.param(" --- a\n", b'"--- a" # note\n', id="start-marker"),
        pytest.param("\t...\tx\n", b'"...\\tx" # note\n', id="end-marker-and-tab"),
        pytest.param(" ---\nb\n", b'"--- b" # note\n', id="marker-folded-onto-a-line"),
        pytest.param("﻿ %x\n", b'\xef\xbb\xbf"%x" # note\n', id="after-bom"),
        pytest.param(" ---x\n", b"---x # note\n", id="no-marker-without-space"),
        pytest.param("!t %x\n", b"!t %x # note\n", id="after-a-tag"),
        pytest.param("&a ... x\n", b"&a ... x # note\n", id="after-an-anchor"),
    ],
)
def test_root_scalar_at_column_zero(src: str, expected: bytes):
    """A root scalar that would open a directive or marker at column 0 is quoted."""
    out = commented_root(src)
    assert out == expected
    assert yamlrocks.loads(out) == yamlrocks.loads(src)


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param(" %x: 1\n %y: 2\n", b'"%x": 2\n"%y": 2\n', id="directive-start"),
        pytest.param(" ... a: 1\n b: 2\n", b'"... a": 2\nb: 2\n', id="end-marker"),
        pytest.param(
            " ---: 1\n b: 2\n", b"---: 2\nb: 2\n", id="bare-marker-before-colon"
        ),
    ],
)
def test_root_mapping_key_at_column_zero(src: str, expected: bytes):
    """A root mapping key that would open a directive or marker is quoted."""
    doc = yamlrocks.loads(src, option=RT)
    first_key = next(iter(doc.to_dict()))
    doc[first_key] = 2
    out = doc.to_yaml()
    assert out == expected
    assert yamlrocks.loads(out) == {**yamlrocks.loads(src), first_key: 2}


@pytest.mark.parametrize(
    ("src", "expected"),
    [
        pytest.param("a: 1\n--- %x\n", b'a: 2\n---\n"%x"\n', id="on-the-marker-line"),
        pytest.param("a: 1\n--- ... x\n", b'a: 2\n---\n"... x"\n', id="end-marker"),
        pytest.param("a: 1\n---\n %x\n", b'a: 2\n---\n"%x"\n', id="below-the-marker"),
    ],
)
def test_root_scalar_of_a_later_document(src: str, expected: bytes):
    """A root scalar written below a ``---`` line follows the same rule."""
    doc = yamlrocks.loads(src, option=RT)
    doc[0] = {"a": 2}
    out = doc.to_yaml()
    assert out == expected
    assert yamlrocks.loads_all(out) == [{"a": 2}, yamlrocks.loads_all(src)[1]]


def test_value_keeps_its_plain_style():
    """Text that is only unsafe at column 0 stays plain as a mapping value."""
    doc = yamlrocks.loads("a: 1\nb: %x\nc: --- y\n", option=RT)
    doc["a"] = 2
    assert doc.to_yaml() == b"a: 2\nb: %x\nc: --- y\n"


def test_multiline_root_scalar_keeps_its_lines():
    """A multi-line root scalar whose lines are safe at column 0 keeps them."""
    doc = yamlrocks.loads("a\n%b\n---\nk: 1\n", option=RT)
    doc[1] = {"k": 2}
    out = doc.to_yaml()
    assert out == b"a\n%b\n---\nk: 2\n"
    assert yamlrocks.loads_all(out) == ["a %b", {"k": 2}]
