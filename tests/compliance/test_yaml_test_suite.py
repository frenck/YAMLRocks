"""Compliance and round-trip tests against the official YAML test suite.

The suite (https://github.com/yaml/yaml-test-suite) is a git submodule at
``tests/data/yaml_test_suite/cases/``, tracking its ``data`` branch. Only the
single-document cases (those with a top-level ``in.yaml``) are exercised; the
multi-document cases, whose variants live in numbered subdirectories, are
skipped. yamlrocks is a pragmatic YAML 1.2 parser rather than a fully
spec-complete one, so a behavior baseline in ``expectations.json`` records the
cases it does not yet handle. The category auto-skips when the submodule is not
checked out, so a plain ``pytest`` stays green without it.

Every case has a known status, and every status is asserted (no test is
skipped). The per-case tests run only over the cases each applies to; two
set-equality guards pin the membership of the baselines so a case can never
silently change category:

* **Robustness**: every input loads to a value or raises cleanly; it never
  crashes or hangs the interpreter (the historical failure mode).
* **Round-trip stability**: every parseable case satisfies
  ``roundtrip_emit(x) == x`` byte-for-byte (the ``roundtrip_unstable`` baseline
  is the escape hatch, currently empty).
* **JSON match**: every parseable case with a canonical JSON, except those on
  the ``json_mismatch`` baseline, resolves to that JSON.
* **Parse status is pinned**: the set of inputs that do not parse equals
  ``parse_failures`` (valid documents not handled yet) plus ``rejected``
  (invalid documents correctly refused). A regression or an improvement both
  fail until the baseline is updated.
* **Error handling is tracked**: every case the suite marks invalid is either
  rejected or recorded in ``error_accepted`` (inputs the parser is currently too
  lenient about).

Each baseline only shrinks: when the parser improves, the guarding test fails
and tells you which case to remove. Regenerate the baselines with
``tests/data/yaml_test_suite/generate_expectations.py``.
"""

from __future__ import annotations

import json
import math
import pathlib

import pytest

import yamlrocks

SUITE_DIR = pathlib.Path(__file__).resolve().parents[1] / "data" / "yaml_test_suite"
CASES_DIR = SUITE_DIR / "cases"
CASES = (
    sorted(p for p in CASES_DIR.iterdir() if (p / "in.yaml").exists())
    if CASES_DIR.is_dir()
    else []
)
CASE_IDS = [p.name for p in CASES]

if not CASES:
    # The suite lives in a git submodule (see `.gitmodules`). Without it the
    # corpus is absent, so skip cleanly rather than failing the membership
    # guards, mirroring the real-world config category.
    pytest.skip(
        "YAML test suite submodule not checked out; run "
        "`git submodule update --init tests/data/yaml_test_suite/cases`",
        allow_module_level=True,
    )

_EXPECT = json.loads((SUITE_DIR / "expectations.json").read_text())
ROUNDTRIP_UNSTABLE = set(_EXPECT["roundtrip_unstable"])
JSON_MISMATCH = set(_EXPECT["json_mismatch"])
REJECTED = set(_EXPECT["rejected"])
ERROR_ACCEPTED = set(_EXPECT["error_accepted"])
PARSE_FAILURES = set(_EXPECT["parse_failures"])


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


def try_load(case: pathlib.Path):
    """Return (parsed_ok, value) for a case's input."""
    try:
        return True, yamlrocks.loads((case / "in.yaml").read_bytes())
    except Exception:
        return False, None


def canonical_json(case_id: str):
    """The first canonical JSON value for a case.

    An empty ``in.json`` means the stream yields no document, which loads as
    ``None`` (comment-only or empty inputs).
    """
    text = (CASES_DIR / case_id / "in.json").read_text().lstrip()
    if not text:
        return None
    value, _ = json.JSONDecoder().raw_decode(text)
    return value


# Parse every case once; downstream tests and guards read from this.
_PARSED = {c.name: try_load(c) for c in CASES}
PARSEABLE_IDS = [name for name in CASE_IDS if _PARSED[name][0]]
NONPARSING_IDS = {name for name in CASE_IDS if not _PARSED[name][0]}
JSON_CASE_IDS = {c.name for c in CASES if (c / "in.json").exists()}
ERROR_CASES = sorted(c.name for c in CASES if (c / "error").exists())
# Parseable cases with a canonical JSON that are not baselined as a mismatch.
JSON_MATCH_IDS = [
    name
    for name in PARSEABLE_IDS
    if name in JSON_CASE_IDS and name not in JSON_MISMATCH
]


@pytest.mark.parametrize("case_id", CASE_IDS)
def test_loads_is_robust(case_id):
    """Every suite case loads or raises cleanly without crashing or hanging."""
    try:
        yamlrocks.loads((CASES_DIR / case_id / "in.yaml").read_bytes())
    except (yamlrocks.YAMLRocksDecodeError, ValueError, TypeError):
        pass


@pytest.mark.parametrize("case_id", PARSEABLE_IDS)
def test_roundtrip_byte_identical(case_id):
    """A parseable case round-trips byte-for-byte unless baselined as unstable."""
    inp = (CASES_DIR / case_id / "in.yaml").read_bytes()
    try:
        emitted = yamlrocks.loads(inp, option=yamlrocks.OPT_ROUND_TRIP).to_yaml()
        identical = emitted == inp
    except Exception:
        identical = False

    if case_id in ROUNDTRIP_UNSTABLE:
        assert not identical, (
            f"{case_id} now round-trips byte-for-byte; remove it from "
            f"expectations.json:roundtrip_unstable; the baseline only shrinks."
        )
    else:
        assert identical, (
            f"{case_id} regressed: an unmodified round-trip is no longer "
            f"byte-for-byte identical. If intentional, add it to "
            f"expectations.json:roundtrip_unstable."
        )


@pytest.mark.parametrize("case_id", JSON_MATCH_IDS)
def test_json_match(case_id):
    """A parseable, non-baselined case matches its canonical JSON."""
    _, value = _PARSED[case_id]
    assert yaml_equal(value, canonical_json(case_id)), (
        f"{case_id} regressed: no longer matches canonical JSON."
    )


@pytest.mark.parametrize("case_id", sorted(JSON_MISMATCH) or [None])
def test_json_mismatch_still_mismatches(case_id):
    """A baselined JSON mismatch still parses and still differs from canonical.

    When the parser learns to resolve one correctly this fails, prompting its
    removal from ``json_mismatch`` (the baseline only shrinks). The baseline is
    currently drained to zero; the ``[None]`` fallback keeps this a passing test
    rather than an empty-parameter skip until a future mismatch is baselined.
    """
    if case_id is None:
        assert not JSON_MISMATCH
        return
    parsed, value = _PARSED[case_id]
    assert parsed, f"{case_id} no longer parses; investigate before re-baselining."
    assert not yaml_equal(value, canonical_json(case_id)), (
        f"{case_id} now matches canonical JSON; remove it from "
        f"expectations.json:json_mismatch."
    )


@pytest.mark.parametrize("case_id", sorted(REJECTED))
def test_known_errors_still_rejected(case_id):
    """Cases baselined as invalid still raise on load."""
    inp = (CASES_DIR / case_id / "in.yaml").read_bytes()
    with pytest.raises((yamlrocks.YAMLRocksDecodeError, ValueError, TypeError)):
        yamlrocks.loads(inp)


@pytest.mark.parametrize("case_id", ERROR_CASES)
def test_error_case_rejected_or_baselined(case_id):
    """Every suite error case is rejected, or recorded as a known lenient accept.

    The suite marks these inputs invalid. yamlrocks rejects most; the ones it
    still accepts are baselined in ``error_accepted`` so the laxness stays
    visible and can only shrink. A *new* lenient accept (not in the baseline)
    fails here, and a case the parser learns to reject must be removed from the
    baseline.
    """
    accepted = _PARSED[case_id][0]
    if accepted:
        assert case_id in ERROR_ACCEPTED, (
            f"{case_id} is a YAML error case that yamlrocks now accepts without "
            f"error and is not baselined. Make the parser reject it, or, if the "
            f"laxness is known and acceptable, add it to "
            f"expectations.json:error_accepted."
        )
    else:
        assert case_id not in ERROR_ACCEPTED, (
            f"{case_id} is now correctly rejected; remove it from "
            f"expectations.json:error_accepted; the baseline only shrinks."
        )


def test_parse_status_matches_baseline():
    """The exact set of non-parsing inputs equals the baselined expectation.

    This pins the membership of ``parse_failures`` (valid documents not handled
    yet) and ``rejected`` (invalid documents correctly refused). It catches both
    a regression (a case that used to parse and no longer does) and an
    improvement (a baselined case that now parses), since the per-case tests
    parametrize over the *current* parse status and would otherwise silently
    skip such a case.
    """
    expected = PARSE_FAILURES | REJECTED
    newly_failing = NONPARSING_IDS - expected
    assert not newly_failing, (
        f"these inputs no longer parse and are not baselined: "
        f"{sorted(newly_failing)}. If a valid document regressed, fix it; if it "
        f"is now correctly rejected, add it to expectations.json."
    )
    now_parsing = expected - NONPARSING_IDS
    assert not now_parsing, (
        f"these baselined inputs now parse: {sorted(now_parsing)}. Remove them "
        f"from expectations.json:parse_failures / rejected (baselines shrink)."
    )


def test_suite_baseline_counts():
    """Coarse sanity floors; exact baseline membership is pinned per-case above.

    The ``parse_failures``, ``json_mismatch``, and ``error_accepted`` baselines
    are not failure budgets to live within; they are a precise to-do list of
    parser bugs, each pinned by the per-case tests above, which fail the moment
    a case is fixed so its entry is removed. The goal is to drive every one of
    them to zero.
    """
    # Guard valid-document coverage, not raw parseable count: correctly
    # rejecting an invalid document lowers the latter, so a floor on it would
    # punish progress. The number of *valid* cases we parse only ever rises as
    # `parse_failures` shrinks, so it is the honest regression floor.
    error_ids = set(ERROR_CASES)
    valid_parseable = [name for name in PARSEABLE_IDS if name not in error_ids]
    assert len(valid_parseable) >= 242, (
        f"only {len(valid_parseable)} valid cases parse (expected >= 242)"
    )
    # Round-trip instability is not tolerated at all.
    assert len(ROUNDTRIP_UNSTABLE) == 0
