"""Compliance and round-trip tests against the official YAML test suite.

The suite (https://github.com/yaml/yaml-test-suite) is a git submodule at
``tests/data/yaml_test_suite/cases/``, tracking its ``data`` branch. Every case
is exercised, including the variants a case stores in numbered subdirectories
(``DE56/00``, ``KH5V/01``, ...); the case id is the path relative to the suite
root. The category auto-skips when the submodule is not checked out, so a plain
``pytest`` stays green without it.

yamlrocks passes the suite in full, so each case is asserted directly against its
filesystem markers, with no recorded baseline of known failures:

* A case marked invalid (it carries an ``error`` file) must be rejected on load.
* Every other (valid) case must load without error, re-emit byte-for-byte in
  round-trip mode, and, when the suite ships a canonical ``in.json``, resolve to
  that value.

A multi-document input is checked both ways: ``loads`` resolves the first
document (matched against the first canonical value), and ``loads_all`` resolves
the whole stream (matched value-for-value against every canonical value), so a
regression in document 2 or later cannot hide. A regression simply fails the
relevant assertion, which is the point; a spec-valid construct yamlrocks ever
chooses not to support would be marked with an explicit ``pytest.mark.xfail``
here, not hidden in a baseline file.
"""

from __future__ import annotations

import json
import math
import pathlib

import pytest

import yamlrocks

SUITE_DIR = pathlib.Path(__file__).resolve().parents[1] / "data" / "yaml_test_suite"
CASES_DIR = SUITE_DIR / "cases"
# A case is any directory holding an ``in.yaml``. Most sit at the top level, but
# a case with several variants stores each under a numbered subdirectory, so the
# search recurses and the case id is the path relative to the suite root.
CASES = (
    sorted((p.parent for p in CASES_DIR.rglob("in.yaml")), key=lambda p: p.as_posix())
    if CASES_DIR.is_dir()
    else []
)
CASE_IDS = [c.relative_to(CASES_DIR).as_posix() for c in CASES]

if not CASES:
    # The suite lives in a git submodule (see `.gitmodules`). Without it the
    # corpus is absent, so skip cleanly rather than failing, mirroring the
    # real-world config category.
    pytest.skip(
        "YAML test suite submodule not checked out; run "
        "`git submodule update --init tests/data/yaml_test_suite/cases`",
        allow_module_level=True,
    )

# The suite marks an invalid input with an ``error`` file; everything else is a
# valid document. A canonical ``in.json`` gives the value a valid case must
# resolve to (absent for comment-only or otherwise value-less documents).
_ERROR_IDS = {
    cid for c, cid in zip(CASES, CASE_IDS, strict=True) if (c / "error").exists()
}
ERROR_IDS = sorted(_ERROR_IDS)
VALID_IDS = [cid for cid in CASE_IDS if cid not in _ERROR_IDS]
JSON_IDS = [
    cid
    for c, cid in zip(CASES, CASE_IDS, strict=True)
    if cid not in _ERROR_IDS and (c / "in.json").exists()
]


def case_bytes(case_id: str) -> bytes:
    """The raw ``in.yaml`` bytes for a case."""
    return (CASES_DIR / case_id / "in.yaml").read_bytes()


def yaml_equal(a, b) -> bool:
    """Deep equality treating NaN==NaN and keeping bool/int distinct."""
    if isinstance(a, float) and isinstance(b, float):
        return a == b or (math.isnan(a) and math.isnan(b))
    if isinstance(a, bool) or isinstance(b, bool):
        return a is b
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(yaml_equal(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(
            yaml_equal(x, y) for x, y in zip(a, b, strict=True)
        )
    return a == b


def canonical_json(case_id: str):
    """The first canonical JSON value for a case.

    An empty ``in.json`` means the stream yields no document, which loads as
    ``None`` (comment-only or empty inputs).
    """
    text = (CASES_DIR / case_id / "in.json").read_text(encoding="utf-8").lstrip()
    if not text:
        return None
    value, _ = json.JSONDecoder().raw_decode(text)
    return value


def canonical_json_all(case_id: str) -> list:
    """Every canonical JSON value for a case, one per document in the stream.

    A multi-document case ships several JSON values in ``in.json``, concatenated
    and whitespace-separated (one per ``---`` document). Decode them in sequence;
    an empty file yields an empty list (no documents).
    """
    text = (CASES_DIR / case_id / "in.json").read_text(encoding="utf-8")
    decoder = json.JSONDecoder()
    values: list = []
    index = 0
    while True:
        while index < len(text) and text[index] in " \t\r\n":
            index += 1
        if index >= len(text):
            return values
        value, index = decoder.raw_decode(text, index)
        values.append(value)


@pytest.mark.parametrize("case_id", VALID_IDS)
def test_valid_case_loads(case_id):
    """Every valid suite case loads without raising."""
    yamlrocks.loads(case_bytes(case_id))


@pytest.mark.parametrize("case_id", JSON_IDS)
def test_valid_case_matches_canonical_json(case_id):
    """A valid case with a canonical JSON resolves to that value.

    ``loads`` resolves the first document, compared against the first canonical
    value; the all-documents semantics are checked separately below.
    """
    assert yaml_equal(yamlrocks.loads(case_bytes(case_id)), canonical_json(case_id)), (
        f"{case_id} no longer matches its canonical JSON"
    )


@pytest.mark.parametrize("case_id", JSON_IDS)
def test_valid_case_matches_canonical_json_all_documents(case_id):
    """Every document of a case resolves to its canonical JSON value.

    ``loads`` only checks document 1, so a multi-document case (the suite ships
    18 with multi-value ``in.json``) could regress in document 2+ undetected.
    ``loads_all`` compares the whole stream, value-for-value.
    """
    got = yamlrocks.loads_all(case_bytes(case_id))
    want = canonical_json_all(case_id)
    assert len(got) == len(want), (
        f"{case_id}: loaded {len(got)} documents, canonical JSON has {len(want)}"
    )
    assert all(yaml_equal(g, w) for g, w in zip(got, want, strict=True)), (
        f"{case_id} no longer matches its canonical JSON across all documents"
    )


@pytest.mark.parametrize("case_id", VALID_IDS)
def test_valid_case_round_trips_byte_identical(case_id):
    """A valid case re-emits byte-for-byte in round-trip mode."""
    inp = case_bytes(case_id)
    emitted = yamlrocks.loads(inp, option=yamlrocks.OPT_ROUND_TRIP).to_yaml()
    assert emitted == inp, f"{case_id} no longer round-trips byte-for-byte"


@pytest.mark.parametrize("case_id", ERROR_IDS)
def test_error_case_is_rejected(case_id):
    """Every case the suite marks invalid is rejected on load."""
    with pytest.raises((yamlrocks.YAMLRocksDecodeError, ValueError, TypeError)):
        yamlrocks.loads(case_bytes(case_id))


def test_suite_is_fully_checked_out():
    """Guard against a partial submodule: the suite has hundreds of cases.

    These are floors, not exact counts, so the suite can grow without churn while
    still catching a truncated or missing checkout.
    """
    assert len(CASE_IDS) >= 400
    assert len(VALID_IDS) >= 300
    assert len(ERROR_IDS) >= 90
