"""Differential oracle: yamlrocks vs PyYAML on decoded values.

The self-differential Rust fuzz target (``fuzz/fuzz_targets/differential.rs``)
checks that ``dumps`` never produces YAML that ``loads`` reads back as different
data. This is the *cross-implementation* counterpart: it loads the same input
with yamlrocks and PyYAML and compares the decoded values, so a scalar that
resolves to the wrong type, a key that shifts, or a structure that comes out
differently shows up as a mismatch instead of hiding behind "it parsed".

To keep the signal clean, yamlrocks is loaded with ``OPT_PYYAML_COMPAT`` so the
two share a scalar schema (PyYAML implements YAML 1.1; yamlrocks defaults to the
1.2 core schema). Two intentional gaps remain and are triaged, not reported:

* Timestamps and dates. YAML 1.2 core does not resolve them, so yamlrocks
  returns a string where PyYAML (1.1) returns ``datetime``/``date``/``time``.
* ``NaN`` never equals itself, so two NaNs are treated as equal here.

The module is both a library (``compare`` and the corpus walkers, used by
``test_pyyaml_differential.py``) and a CLI for extended, mutation-based runs::

    uv run --no-sync python tests/compat/pyyaml_differential.py --generate 200000
    uv run --no-sync python tests/compat/pyyaml_differential.py path/to/corpus/
"""

from __future__ import annotations

import argparse
import datetime as dt
import math
import pathlib
import random
import sys
from collections import Counter
from dataclasses import dataclass

import yaml

import yamlrocks

# Load both sides under the same scalar schema so 1.1-vs-1.2 differences (yes/no
# booleans, 0777 octals, sexagesimals, underscored numbers) do not masquerade as
# bugs. What is left after this is genuine disagreement.
_COMPAT = yamlrocks.OPT_PYYAML_COMPAT

# A generated input larger than this, or with more aliases than the limit, is
# skipped: PyYAML's safe loader expands aliases eagerly, so an alias bomb would
# take the oracle down rather than the parser under test.
_MAX_INPUT_BYTES = 64 * 1024
_MAX_ALIASES = 40


class Outcome:
    """The comparison verdict classes."""

    MATCH = "match"
    VALUE_MISMATCH = "value_mismatch"  # both load, values differ: the bug signal
    YAMLROCKS_LENIENT = "yamlrocks_lenient"  # yamlrocks loads, PyYAML rejects
    YAMLROCKS_STRICT = "yamlrocks_strict"  # PyYAML loads, yamlrocks rejects
    BOTH_REJECT = "both_reject"
    SKIP = "skip"  # not comparable (alias bomb, oversize, multi-doc count clash)


@dataclass
class Result:
    """One comparison, kept small so millions can flow through."""

    outcome: str
    detail: str = ""


def _values_equal(yr: object, py: object) -> bool:
    """Whether a yamlrocks value equals a PyYAML value, after triage.

    Leaf types must match exactly (so an int decoded as a bool, or a float that
    lost precision, is caught), except for the two intentional gaps above.
    """
    # Timestamp gap: a yamlrocks string against a PyYAML datetime/date/time is
    # the documented 1.2-core behaviour, not a disagreement.
    if isinstance(py, (dt.datetime, dt.date, dt.time)) and isinstance(yr, str):
        return True
    if isinstance(yr, bool) or isinstance(py, bool):
        # Guard the bool/int overlap (`True == 1` in Python) before the numeric
        # branch: a bool on one side and an int on the other is a real mismatch.
        return type(yr) is type(py) and yr == py
    if isinstance(yr, float) and isinstance(py, float):
        return (math.isnan(yr) and math.isnan(py)) or yr == py
    if isinstance(yr, dict) and isinstance(py, dict):
        if len(yr) != len(py):
            return False
        try:
            return all(k in yr and _values_equal(yr[k], v) for k, v in py.items())
        except TypeError:
            return False  # unhashable key surfaced differently on one side
    if isinstance(yr, (list, tuple)) and isinstance(py, (list, tuple)):
        if len(yr) != len(py):
            return False
        return all(_values_equal(a, b) for a, b in zip(yr, py, strict=True))
    return type(yr) is type(py) and yr == py


def _docs_equal(yr_docs: list[object], py_docs: list[object]) -> bool:
    """Compare two document streams element for element."""
    if len(yr_docs) != len(py_docs):
        return False
    return all(_values_equal(a, b) for a, b in zip(yr_docs, py_docs, strict=True))


def _load_yamlrocks(raw: bytes) -> tuple[bool, object]:
    try:
        return True, yamlrocks.loads_all(raw, option=_COMPAT)
    except Exception as exc:
        return False, exc


def _load_pyyaml(raw: bytes) -> tuple[bool, object]:
    try:
        return True, list(yaml.safe_load_all(raw))
    except Exception as exc:
        return False, exc


def compare(raw: bytes) -> Result:
    """Load ``raw`` with both libraries and classify the disagreement, if any."""
    if len(raw) > _MAX_INPUT_BYTES or raw.count(b"*") > _MAX_ALIASES:
        return Result(Outcome.SKIP, "oversize or alias-heavy")

    yr_ok, yr = _load_yamlrocks(raw)
    py_ok, py = _load_pyyaml(raw)

    if yr_ok and py_ok:
        # Both loaders return a list of documents on success.
        assert isinstance(yr, list) and isinstance(py, list)
        if _docs_equal(yr, py):
            return Result(Outcome.MATCH)
        return Result(Outcome.VALUE_MISMATCH, f"yamlrocks={yr!r} pyyaml={py!r}")
    if yr_ok and not py_ok:
        return Result(Outcome.YAMLROCKS_LENIENT, f"pyyaml rejected: {py}")
    if py_ok and not yr_ok:
        return Result(Outcome.YAMLROCKS_STRICT, f"yamlrocks rejected: {yr}")
    return Result(Outcome.BOTH_REJECT)


# -- Corpora ------------------------------------------------------------------

_ROOT = pathlib.Path(__file__).resolve().parents[2]
_YAML_TEST_SUITE = _ROOT / "tests" / "data" / "yaml_test_suite" / "cases"
_FUZZ_CORPUS = _ROOT / "fuzz" / "corpus" / "differential"


def yaml_test_suite_inputs() -> list[bytes]:
    """Every ``in.yaml`` from the YAML test suite submodule (empty if absent)."""
    if not _YAML_TEST_SUITE.is_dir():
        return []
    return [p.read_bytes() for p in sorted(_YAML_TEST_SUITE.rglob("in.yaml"))]


def fuzz_corpus_inputs() -> list[bytes]:
    """Every local Rust-fuzzer corpus entry (gitignored; empty if absent)."""
    if not _FUZZ_CORPUS.is_dir():
        return []
    return [p.read_bytes() for p in sorted(_FUZZ_CORPUS.iterdir()) if p.is_file()]


# -- Mutation-based generator -------------------------------------------------

# Bytes worth splicing in: YAML structure indicators and a few troublemakers.
_INTERESTING = [*b":-?#&*!|>'\"%@`{}[],\n \t", 0x00, 0x85, 0x7F, 0xEF]


def _mutate(rng: random.Random, seed: bytes) -> bytes:
    """Apply a handful of random edits to a seed input."""
    data = bytearray(seed)
    for _ in range(rng.randint(1, 6)):
        if not data:
            data = bytearray(b"a: 1\n")
        op = rng.random()
        i = rng.randrange(len(data))
        if op < 0.35:  # overwrite with an interesting byte
            data[i] = rng.choice(_INTERESTING) & 0xFF
        elif op < 0.6:  # insert
            data.insert(i, rng.choice(_INTERESTING) & 0xFF)
        elif op < 0.8:  # delete
            del data[i]
        elif op < 0.9:  # duplicate a line
            lines = bytes(data).splitlines(keepends=True)
            if lines:
                j = rng.randrange(len(lines))
                lines.insert(j, lines[j])
                data = bytearray(b"".join(lines))
        else:  # truncate
            data = data[: rng.randrange(len(data) + 1)]
    return bytes(data)


def generate(count: int, seed: int = 0) -> list[bytes]:
    """Produce ``count`` mutated inputs from the available seed corpora."""
    rng = random.Random(seed)
    seeds = yaml_test_suite_inputs() or [b"a: 1\nb: [1, 2]\nc: {d: e}\n"]
    return [_mutate(rng, rng.choice(seeds)) for _ in range(count)]


# -- CLI ----------------------------------------------------------------------


def _run(inputs: list[bytes], max_report: int) -> int:
    counts: Counter[str] = Counter()
    reported = 0
    for raw in inputs:
        result = compare(raw)
        counts[result.outcome] += 1
        # The bug signal is a value mismatch: both parsed, the data differs.
        if result.outcome == Outcome.VALUE_MISMATCH and reported < max_report:
            reported += 1
            print(f"\nVALUE_MISMATCH on {raw!r}\n  {result.detail}")
    print("\n== summary ==")
    for name in vars(Outcome):
        key = getattr(Outcome, name)
        if isinstance(key, str) and counts.get(key):
            print(f"  {key:20s} {counts[key]}")
    return counts.get(Outcome.VALUE_MISMATCH, 0)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="*",
        type=pathlib.Path,
        help="files/dirs of YAML inputs to replay",
    )
    parser.add_argument(
        "--generate", type=int, default=0, help="generate N mutated inputs"
    )
    parser.add_argument("--seed", type=int, default=0, help="RNG seed for --generate")
    parser.add_argument(
        "--max-report", type=int, default=25, help="max mismatches to print"
    )
    args = parser.parse_args(argv)

    inputs: list[bytes] = []
    for path in args.paths:
        if path.is_dir():
            inputs += [p.read_bytes() for p in sorted(path.rglob("*")) if p.is_file()]
        elif path.is_file():
            inputs.append(path.read_bytes())
    if args.generate:
        inputs += generate(args.generate, args.seed)
    if not args.paths and not args.generate:
        inputs = yaml_test_suite_inputs()

    print(f"comparing {len(inputs)} inputs")
    mismatches = _run(inputs, args.max_report)
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
