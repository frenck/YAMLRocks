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


# Every combination of a sequence item with or without a comment on its dash,
# with or without comments above and below it, addressed from every cursor that
# can reach one. Generated rather than listed: the cell this missed the first
# time (the first key of an item whose dash carries no comment) is exactly the
# kind a hand-written list of examples leaves out.
def _item_sources() -> dict[str, bytes]:
    sources = {}
    for above in ("", "# above\n"):
        for dash in ("", " # dash"):
            for below in ("", "  # below\n"):
                # A comment below the dash needs one on the dash to sit under.
                if below and not dash:
                    continue
                name = f"above={bool(above)},dash={bool(dash)},below={bool(below)}"
                sources[name] = (
                    b"- first: 0\n"
                    + above.encode()
                    + b"-"
                    + dash.encode()
                    + b"\n"
                    + below.encode()
                    + b"  a: 1\n  b: 2\n"
                ).replace(b"-\n  a: 1", b"- a: 1")
    return sources


ITEM_SOURCES = _item_sources()

# Which node each cursor addresses, and how to find the line its "before"
# comments precede. The item's line is its own `-`, which is the *second* dash
# in these sources; the keys are found by their text. When the dash carries no
# comment the item and its first key share that one line, so the two cursors
# address the same block by design.
CURSORS = {
    "item": (lambda doc: doc.node[1], lambda lines: _second_dash(lines)),
    "first-key": (
        lambda doc: doc.node[1]["a"],
        lambda lines: _line_with(lines, b"a: 1"),
    ),
    "second-key": (
        lambda doc: doc.node[1]["b"],
        lambda lines: _line_with(lines, b"b: 2"),
    ),
}


def _second_dash(lines: list[bytes]) -> int:
    """The line of the second item's `-`."""
    dashes = [i for i, line in enumerate(lines) if line.lstrip().startswith(b"-")]
    return dashes[1]


def _line_with(lines: list[bytes], text: bytes) -> int:
    return next(i for i, line in enumerate(lines) if text in line)


def _comments_directly_above(source: bytes, line_of) -> str | None:
    """The comment lines immediately above a line, the oracle for the getter.

    Whatever a reader sees written above that line, and nothing further up.
    """
    lines = source.split(b"\n")
    index = line_of(lines)
    collected = []
    while index > 0 and lines[index - 1].strip().startswith(b"#"):
        index -= 1
        collected.insert(0, lines[index].strip().lstrip(b"#").strip().decode())
    return "\n".join(collected) if collected else None


@pytest.mark.parametrize("name", sorted(ITEM_SOURCES))
@pytest.mark.parametrize("cursor", sorted(CURSORS))
def test_comment_before_matches_what_is_written_above_that_line(
    name: str, cursor: str
) -> None:
    """Each cursor reports exactly the comment lines written above its own line."""
    source = ITEM_SOURCES[name]
    resolve, line_of = CURSORS[cursor]
    doc = yamlrocks.loads(source, option=RT)

    assert resolve(doc).comment_before == _comments_directly_above(source, line_of)


@pytest.mark.parametrize("name", sorted(ITEM_SOURCES))
@pytest.mark.parametrize("cursor", sorted(CURSORS))
def test_comment_before_writes_back_where_it_was_read(name: str, cursor: str) -> None:
    """Assigning ``comment_before`` its own value changes nothing, from any cursor."""
    source = ITEM_SOURCES[name]
    resolve, _ = CURSORS[cursor]
    doc = yamlrocks.loads(source, option=RT)
    node = resolve(doc)
    node.comment_before = node.comment_before
    # Force the AST path, so the source cache cannot hide a move.
    doc.node.comment_after = doc.node.comment_after
    assert doc.to_yaml() == source


@pytest.mark.parametrize(
    "name", sorted(name for name in ITEM_SOURCES if "dash=True" in name)
)
def test_writing_one_side_of_a_dash_leaves_the_other_alone(name: str) -> None:
    """With a comment on the dash, the item and its first key own separate blocks.

    Without one they share a line, and so address the same block on purpose;
    only a dash comment splits them.
    """
    source = ITEM_SOURCES[name]
    doc = yamlrocks.loads(source, option=RT)
    below = doc.node[1]["a"].comment_before
    doc.node[1].comment_before = "written above"
    assert doc.node[1]["a"].comment_before == below

    doc = yamlrocks.loads(source, option=RT)
    above = doc.node[1].comment_before
    doc.node[1]["a"].comment_before = "written below"
    assert doc.node[1].comment_before == above


# The comment on a mapping's own key line, reached from the first key inside it.
def test_mapping_first_key_reports_the_leading_block() -> None:
    """A mapping's leading comments stay readable from its first key."""
    doc = yamlrocks.loads(b"parent: # note\n  # leading\n  first: 1\n", option=RT)
    assert doc.node["parent"].comment == "note"
    assert doc.node["parent"]["first"].comment_before == "leading"
