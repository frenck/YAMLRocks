"""Snapshot tests over the YAML corpus.

For every file in ``tests/data/corpus/`` these tests pin down:

* the Python object produced by ``loads`` (``loads/<name>.txt``),
* the bytes produced by ``dumps`` of that object (``dumps/<name>.yaml``),
* the bytes produced by round-trip emission (``roundtrip/<name>.yaml``).

They also assert two semantic invariants that must hold regardless of the exact
serialization:

* fast-path dump stability: ``loads(dumps(x)) == x``
* round-trip semantic stability: re-parsing a round-tripped document yields the
  same Python object as the original.
"""

from __future__ import annotations

import math
import pathlib
import pprint

import pytest

import yamlrocks

CORPUS = sorted(
    (pathlib.Path(__file__).resolve().parents[1] / "data" / "corpus").glob("*.yaml")
)
CORPUS_IDS = [p.name for p in CORPUS]


def yaml_equal(a, b) -> bool:
    """Deep equality that treats two NaN floats as equal (unlike ``==``)."""
    if isinstance(a, float) and isinstance(b, float):
        return a == b or (math.isnan(a) and math.isnan(b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(yaml_equal(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(
            yaml_equal(x, y) for x, y in zip(a, b, strict=True)
        )
    return a == b


@pytest.mark.parametrize("path", CORPUS, ids=CORPUS_IDS)
def test_loads_snapshot(path, assert_snapshot):
    """The object produced by loads matches its stored snapshot."""
    data = yamlrocks.loads(path.read_bytes())
    rendered = pprint.pformat(data, width=88, sort_dicts=False)
    assert_snapshot(f"loads/{path.stem}.txt", rendered)


@pytest.mark.parametrize("path", CORPUS, ids=CORPUS_IDS)
def test_dumps_snapshot(path, assert_snapshot):
    """The bytes produced by dumps match the stored snapshot."""
    data = yamlrocks.loads(path.read_bytes())
    out = yamlrocks.dumps(data)
    assert_snapshot(f"dumps/{path.stem}.yaml", out.decode())


@pytest.mark.parametrize("path", CORPUS, ids=CORPUS_IDS)
def test_roundtrip_snapshot(path, assert_snapshot):
    """The bytes produced by round-trip emission match the stored snapshot."""
    doc = yamlrocks.loads(path.read_bytes(), option=yamlrocks.OPT_ROUND_TRIP)
    out = doc.to_yaml()
    assert_snapshot(f"roundtrip/{path.stem}.yaml", out.decode())


@pytest.mark.parametrize("path", CORPUS, ids=CORPUS_IDS)
def test_dump_stability(path):
    """Re-parsing dumped output yields the original object."""
    data = yamlrocks.loads(path.read_bytes())
    assert yaml_equal(yamlrocks.loads(yamlrocks.dumps(data)), data)


@pytest.mark.parametrize("path", CORPUS, ids=CORPUS_IDS)
def test_roundtrip_byte_identical(path):
    """An unmodified round-trip reproduces the source bytes exactly."""
    # An unmodified round-trip must reproduce the source exactly.
    raw = path.read_bytes()
    doc = yamlrocks.loads(raw, option=yamlrocks.OPT_ROUND_TRIP)
    assert doc.to_yaml() == raw
