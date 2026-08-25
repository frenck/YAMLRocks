"""Comment placement across every shape, edit, and cursor.

The introducer-line comment (`key: # note`, `- # note`) is stored with the
node's other head comments, and the two can straddle a sequence dash: some
lines belong above it, some below. That split is easy to get right for one
shape and wrong for the next, so these sweeps assert the invariants over the
whole matrix rather than case by case:

* an edit anywhere must not lose a comment, move it to another line, or make
  the document unparsable, and
* reading ``comment_before`` from any cursor and assigning it straight back
  must leave the document byte-for-byte unchanged.
"""

from __future__ import annotations

import re

import pytest
import yaml

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP
COMMENT = re.compile(rb"#\s*(c\d+)")

# One shape per node kind an introducer comment can precede. `c1` sits above the
# introducer, `c2` on it, `c3` below it.
SHAPES = {
    "map-block-map": b"k: # c2\n  # c3\n  a: 1\nz: 9\n",
    "map-block-seq": b"k: # c2\n  # c3\n  - 1\nz: 9\n",
    "map-scalar": b"k: # c2\n  # c3\n  v\nz: 9\n",
    "map-flow": b"k: # c2\n  # c3\n  [1]\nz: 9\n",
    "map-block-scalar": b"k: # c2\n  # c3\n  |\n  text\nz: 9\n",
    "map-anchored": b"k: &a # c2\n  # c3\n  v\nz: 9\n",
    "map-above-and-on": b"# c1\nk: # c2\n  a: 1\nz: 9\n",
    "seq-block-map": b"- x\n# c1\n- # c2\n  # c3\n  a: 1\n",
    "seq-block-seq": b"- x\n# c1\n- # c2\n  # c3\n  - 1\n",
    "seq-scalar": b"- x\n# c1\n- # c2\n  # c3\n  v\n",
    "seq-flow": b"- x\n# c1\n- # c2\n  # c3\n  [1]\n",
    "seq-anchored": b"- x\n# c1\n- &a # c2\n  # c3\n  a: 1\n",
    "seq-nested": b"- x\n# c1\n- # c2\n  - 1\n",
    "map-trailing": b"k: v # c2\nz: 9\n",
    "seq-trailing": b"- y\n- v # c2\n",
}

# Each operation is applied to the commented entry, except the first, which
# edits an unrelated one to force the AST path.
OPERATIONS = [
    "edit elsewhere",
    "read comment",
    "set same comment",
    "set same comment_before",
    "replace value",
]


def _entries(doc, name):
    """The commented node and an unrelated one, for a shape."""
    if name.startswith("seq"):
        return doc.node[1], doc.node[0]
    return doc.node["k"], doc.node["z"]


def _apply(operation, target, other):
    if operation == "edit elsewhere":
        other.value = "EDITED"
    elif operation == "read comment":
        # A read alone must not disturb anything either.
        assert target.comment is None or isinstance(target.comment, str)
    elif operation == "set same comment":
        target.comment = target.comment
    elif operation == "set same comment_before":
        target.comment_before = target.comment_before
    elif operation == "replace value":
        target.value = {"new": 1}


@pytest.mark.parametrize("name", sorted(SHAPES))
@pytest.mark.parametrize("operation", OPERATIONS)
def test_no_edit_loses_a_comment(name: str, operation: str) -> None:
    """Every comment survives every edit, and the result still parses."""
    source = SHAPES[name]
    doc = yamlrocks.loads(source, option=RT)
    _apply(operation, *_entries(doc, name))
    emitted = doc.to_yaml()

    assert sorted(COMMENT.findall(emitted)) == sorted(COMMENT.findall(source))
    yaml.safe_load(emitted)


@pytest.mark.parametrize("name", sorted(SHAPES))
def test_an_unrelated_edit_keeps_every_comment_on_its_line(name: str) -> None:
    """Editing one value moves nothing else: each comment keeps its own line."""
    source = SHAPES[name]
    doc = yamlrocks.loads(source, option=RT)
    _apply("edit elsewhere", *_entries(doc, name))

    def lines(text: bytes) -> dict[bytes, int]:
        return {
            found: number
            for number, line in enumerate(text.split(b"\n"))
            for found in COMMENT.findall(line)
        }

    assert lines(doc.to_yaml()) == lines(source)


# Every cursor that can address a "before" comment, with what it must report.
# A sequence item owns the block above its dash; the first key inside it owns
# the block below the dash's own comment, which is where those lines sit
# relative to that key.
CURSORS = [
    (b"- # c2\n  # c3\n  a: 1\n- x\n", "item", lambda d: d.node[0], None),
    (
        b"- # c2\n  # c3\n  a: 1\n- x\n",
        "item-first-key",
        lambda d: d.node[0]["a"],
        "c3",
    ),
    (b"- y\n# c1\n- # c2\n  # c3\n  a: 1\n", "later-item", lambda d: d.node[1], "c1"),
    (
        b"- y\n# c1\n- # c2\n  # c3\n  a: 1\n",
        "later-item-first-key",
        lambda d: d.node[1]["a"],
        "c3",
    ),
    (
        b"- y\n# c1\n- # c2\n  a: 1\n  b: 2\n",
        "item-second-key",
        lambda d: d.node[1]["b"],
        None,
    ),
    (b"- y\n# c1\n- a: 1\n", "item-without-dash-comment", lambda d: d.node[1], "c1"),
    (
        b"- y\n- a: 1\n  # c3\n  b: 2\n",
        "second-key-in-plain-item",
        lambda d: d.node[1]["b"],
        "c3",
    ),
    (
        b"parent: # c2\n  # c3\n  first: 1\n",
        "mapping-first-key",
        lambda d: d.node["parent"]["first"],
        "c3",
    ),
    (b"# c1\nparent:\n  first: 1\n", "mapping-value", lambda d: d.node["parent"], "c1"),
    (b"k: v\n# c1\nz: 9\n", "later-key", lambda d: d.node["z"], "c1"),
]


@pytest.mark.parametrize(
    ("source", "label", "cursor", "expected"),
    CURSORS,
    ids=[case[1] for case in CURSORS],
)
def test_comment_before_reports_the_block_that_cursor_owns(
    source: bytes, label: str, cursor, expected: str | None
) -> None:
    """Each cursor reports the comment block written for it, and no other."""
    doc = yamlrocks.loads(source, option=RT)
    assert cursor(doc).comment_before == expected


@pytest.mark.parametrize(
    ("source", "label", "cursor", "expected"),
    CURSORS,
    ids=[case[1] for case in CURSORS],
)
def test_comment_before_writes_back_where_it_was_read(
    source: bytes, label: str, cursor, expected: str | None
) -> None:
    """Assigning ``comment_before`` its own value changes nothing."""
    doc = yamlrocks.loads(source, option=RT)
    node = cursor(doc)
    node.comment_before = node.comment_before
    # Force the AST path, so the source cache cannot hide a move.
    doc.node.comment_after = doc.node.comment_after
    assert doc.to_yaml() == source
