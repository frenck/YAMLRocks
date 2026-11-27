"""Block scalars (``|`` literal and ``>`` folded) across a re-emit.

A block scalar is written one of two ways. An unmodified one replays its source,
which keeps the author's layout, but only where that source still reads back as
its value: its body lines stay at their source columns, and what they mean
depends on how deep the scalar sits. Everywhere else, and for a modified one,
the emitter writes it from its value, with the indentation indicator, chomping
indicator and folded line breaks that value needs.
"""

from __future__ import annotations

import random
from pathlib import Path

import pytest

import yamlrocks

RT = yamlrocks.OPT_ROUND_TRIP
SUITE = Path(__file__).parent.parent / "data" / "yaml_test_suite" / "cases"


def test_modified_folded_scalar_keeps_its_line_breaks():
    """A folded scalar rewritten after a comment edit keeps its line breaks."""
    doc = yamlrocks.loads("a: 1\nb: >\n  ab\n\n  cd\n", option=RT)
    doc.node["b"].comment = "t"
    out = doc.to_yaml()
    assert out == b"a: 1\nb: > # t\n  ab\n\n  cd\n"
    assert yamlrocks.loads(out) == {"a": 1, "b": "ab\ncd\n"}


def test_anchored_folded_scalar_keeps_its_line_breaks():
    """A folded scalar rewritten after an anchor edit keeps its line breaks."""
    doc = yamlrocks.loads("a: 1\nb: >\n  ab\n\n  cd\n", option=RT)
    doc.node["b"].anchor = "x"
    out = doc.to_yaml()
    assert out == b"a: 1\nb: &x >\n  ab\n\n  cd\n"
    assert yamlrocks.loads(out) == {"a": 1, "b": "ab\ncd\n"}


def test_modified_scalar_keeps_its_indentation_indicator():
    """A rewritten block scalar whose first line starts with a space keeps ``|2``."""
    doc = yamlrocks.loads("a: 1\nb: |2\n   x\n  y\n", option=RT)
    doc.node["b"].comment = "t"
    out = doc.to_yaml()
    assert out == b"a: 1\nb: |2 # t\n   x\n  y\n"
    assert yamlrocks.loads(out) == {"a": 1, "b": " x\ny\n"}


def test_reindented_block_scalar_is_rewritten():
    """A block scalar whose source no longer sits past its moved key is rewritten."""
    doc = yamlrocks.loads("a:\n b: |\n  x\n", option=RT)
    doc["z"] = 1
    out = doc.to_yaml()
    assert out == b"a:\n  b: |\n    x\nz: 1\n"
    assert yamlrocks.loads(out) == {"a": {"b": "x\n"}, "z": 1}


def test_reindented_indentation_indicator_is_recounted():
    """A moved block scalar with ``|1`` gets the indicator its new parent needs."""
    doc = yamlrocks.loads("a:\n b: |1\n   x\n", option=RT)
    doc["z"] = 1
    out = doc.to_yaml()
    assert out == b"a:\n  b: |2\n     x\nz: 1\n"
    assert yamlrocks.loads(out) == {"a": {"b": " x\n"}, "z": 1}


def test_unmodified_block_scalar_replays_where_it_still_fits():
    """A block scalar the emitter moves keeps its source if it still reads back."""
    src = "a:\n b: >\n   folded\n   text\n"
    doc = yamlrocks.loads(src, option=RT)
    doc["z"] = 1
    assert doc.to_yaml() == b"a:\n  b: >\n   folded\n   text\nz: 1\n"


@pytest.mark.parametrize("case", ["K527", "93WF", "TS54", "F6MC"])
def test_suite_block_scalars_survive_a_modification(case: str):
    """Marking each block scalar of a suite case modified keeps the case's data."""
    path = SUITE / case / "in.yaml"
    if not path.exists():
        pytest.skip("YAML test suite submodule not checked out")

    src = path.read_text()
    expected = yamlrocks.loads(src)
    loaded = yamlrocks.loads(src, option=RT)
    paths = [
        node_path
        for node_path, _ in loaded.walk()
        if loaded.locate(node_path).style in ("literal", "folded")
    ]
    assert paths

    for node_path in paths:
        doc = yamlrocks.loads(src, option=RT)
        doc.locate(node_path).comment = "t"
        assert yamlrocks.loads(doc.to_yaml()) == expected


# -- The seeded property test --------------------------------------------------

LINE_KINDS = [
    lambda rng: "",
    lambda rng: rng.choice(["a", "word", "x y", "#hash", "---", "...", "--- x", "-"]),
    lambda rng: " " * rng.randint(1, 4) + rng.choice(["a", "b c", "#c", "- d"]),
    lambda rng: " " * rng.randint(1, 5),
    lambda rng: "\t" + rng.choice(["", "tab", " x"]),
    lambda rng: rng.choice(["end", "a b"]) + rng.choice([" ", "  ", "\t", " \t"]),
    lambda rng: " ".join(rng.choice(["lorem", "ipsum", "dolor"]) for _ in range(60)),
    lambda rng: "y" * rng.randint(100, 300),
]

EDGE_VALUES = [
    "",
    "\n",
    "\n\n\n",
    "x",
    " ",
    "  lead\nrest\n",
    "\n\n  indented after blanks\n",
    "text\n\n\n",
    "\n\nleading blanks\n",
    "a\n  more\nb\n",
    "trailing \nspaces  \n",
    "#a\n---\n...\n",
]


def generated_values(count: int, seed: int) -> list[str]:
    """Edge values plus ``count`` random multi-line strings from a fixed seed."""
    rng = random.Random(seed)
    values = list(EDGE_VALUES)
    for _ in range(count):
        lines = [rng.choice(LINE_KINDS)(rng) for _ in range(rng.randint(1, 6))]
        values.append(
            "\n" * rng.randint(0, 2) + "\n".join(lines) + "\n" * rng.randint(0, 3)
        )
    return values


VALUES = generated_values(500, seed=20261127)


class Block:
    """A placeholder the ``represent`` hook turns into a block scalar."""

    def __init__(self, value: str) -> None:
        self.value = value


def block_dumps(data: object, style: str, option: int = 0, **properties) -> bytes:
    """Dump ``data`` with every ``Block`` written in the block ``style``."""

    def represent(obj: object) -> object:
        if isinstance(obj, Block):
            return yamlrocks.YAMLRocksScalar(obj.value, style=style, **properties)
        return None

    return yamlrocks.dumps(data, option=option, represent=represent)


def unblock(data: object) -> object:
    """``data`` with every ``Block`` replaced by its value."""
    if isinstance(data, Block):
        return data.value
    if isinstance(data, dict):
        return {key: unblock(value) for key, value in data.items()}
    if isinstance(data, list):
        return [unblock(item) for item in data]
    return data


def positions(value: str) -> list[object]:
    """``value`` as a mapping value and a sequence item at several depths, as an
    item of a nested sequence, at the document root, and anchored and aliased.
    """
    shared = Block(value)
    return [
        {"k": Block(value)},
        {"a": {"b": {"c": Block(value)}}},
        [Block(value)],
        {"a": [{"b": [Block(value)]}]},
        [[Block(value)]],
        Block(value),
        {"k": shared, "j": shared},
    ]


@pytest.mark.parametrize("style", ["literal", "folded"])
@pytest.mark.parametrize(
    "option", [0, yamlrocks.OPT_INDENT_4], ids=["indent-2", "indent-4"]
)
@pytest.mark.parametrize(
    "properties",
    [{}, {"tag": "!t"}, {"comment": "note"}, {"tag": "!t", "comment": "note"}],
    ids=["bare", "tag", "comment", "tag-and-comment"],
)
def test_written_block_scalars_read_back(style: str, option: int, properties: dict):
    """Every generated string written as a block scalar reads back unchanged."""
    for value in VALUES:
        for data in positions(value):
            out = block_dumps(data, style, option, **properties)
            assert yamlrocks.loads(out) == unblock(data), (value, out)


def block_text(value: str, style: str, parent: int) -> str:
    """The header and body of ``value`` as a block scalar whose parent node sits
    at column ``parent`` (body lines start ``parent + 2`` columns in).
    """
    text = block_dumps({"k": Block(value)}, style).decode()
    assert text.startswith("k: ")
    header, _, body = text[3:].partition("\n")
    lines = [" " * parent + line if line else line for line in body.split("\n")]
    return header + "\n" + "\n".join(lines)


# `(template, path to the scalar, parent column)`: a source with a block scalar
# placed where `{}` is, the path that reaches it (`None` for a key, which only
# takes the unrelated edit), and the column of the node that holds it.
TEMPLATES = [
    ("a: 1\nk: {}", ["k"], 0),
    ("a:\n  b:\n    k: {}", ["a", "b", "k"], 4),
    ("- {}- x\n", [0], 0),
    ("- - {}  - x\n", [0, 0], 2),
    ("a:\n- - {}", ["a", 0, 0], 2),
    ("- k: {}", [0, "k"], 2),
    ("{}", [], 0),
    ("a: 1\nk: !t {}", ["k"], 0),
    ("k: &a {}j: *a\n", ["k"], 0),
    ("k: # note\n  {}", ["k"], 0),
    ("? {}: v\n", None, 0),
    # One-space indentation: the emitter moves these keys right on a re-emit.
    ("a:\n b: {}", ["a", "b"], 1),
    ("a:\n - {}", ["a", 0], 1),
    ("a:\n - - {}", ["a", 0, 0], 3),
]


def unrelated_edit(doc: object) -> None:
    """Change something the block scalar does not hold: add a top-level key, or
    replace the last item of a top-level sequence with more than one.
    """
    data = doc.to_dict() if hasattr(doc, "to_dict") else doc
    if isinstance(data, dict):
        doc["zz"] = 1
    elif isinstance(data, list) and len(data) > 1:
        doc[len(data) - 1] = "y"


@pytest.mark.parametrize("style", ["literal", "folded"])
def test_reemitted_block_scalars_read_back(style: str):
    """Every generated block scalar survives a re-emit in every position."""
    for value in VALUES:
        for template, path, parent in TEMPLATES:
            src = template.replace("{}", block_text(value, style, parent))
            expected = yamlrocks.loads(src)

            doc = yamlrocks.loads(src, option=RT)
            unrelated_edit(doc)
            edited = yamlrocks.loads(src)
            unrelated_edit(edited)
            out = doc.to_yaml()
            assert yamlrocks.loads(out) == edited, (value, src, out)

            if path is None:
                continue

            # The anchor is `a`, the name the aliased template already uses, so
            # its alias still resolves.
            for attribute, setting in (("comment", "t"), ("anchor", "a")):
                doc = yamlrocks.loads(src, option=RT)
                setattr(doc.locate(path), attribute, setting)
                out = doc.to_yaml()
                assert yamlrocks.loads(out) == expected, (value, src, out)
