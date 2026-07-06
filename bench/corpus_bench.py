#!/usr/bin/env python3
"""Benchmark loads/dumps throughput across the real-world config corpus.

Where ``bench.py`` measures a handful of hand-written payloads, this measures the
245 MB of *actual* config files under ``tests/data/realworld`` (the git
submodules). Real configs exercise the shapes invented payloads miss, block
scalars, anchors, flow collections, quoting, multi-document streams, Unicode, so
this is the honest signal for where the parser spends its time and how fast it
is on data people really have.

It is also the profiling target: point callgrind at it (one iteration) to find
the hotspots real data hits, rather than guessing from synthetic input.

    just bench-corpus                 # throughput, and the ratio vs PyYAML's C loader
    python bench/corpus_bench.py --limit 2000   # a quick subset
    python bench/corpus_bench.py --iterations 1 --no-compare   # a lean callgrind target

Coverage is reported honestly: files that fail to parse (or that PyYAML rejects)
are excluded from the timed set and the byte denominator, and the counts are
printed, so MB/s always reflects the bytes actually processed, and the
cross-library ratio is measured over the files both libraries accept.

Auto-skips when the corpus submodule is absent. Always run under a guard, e.g.:

    timeout 280 bash -c 'ulimit -v 6000000; python bench/corpus_bench.py'
"""

from __future__ import annotations

import argparse
import contextlib
import sys
import time

import yamlrocks

# Reuse the real-world suite's discovery so this benchmark and the correctness
# audit agree on exactly which files make up the corpus.
sys.path.insert(0, "tests")
from realworld.test_realworld import (  # type: ignore
    _FILES,
    KNOWN_INVALID,
    _rel,
)

try:
    import yaml as pyyaml

    _PY_C_LOADER = getattr(pyyaml, "CSafeLoader", None)
except Exception:  # pragma: no cover - optional dependency
    pyyaml = None
    _PY_C_LOADER = None

MB = 1_048_576


def _load_corpus(limit: int | None) -> list[bytes]:
    """Read every valid corpus file into memory (I/O out of the timed loop)."""
    files = [f for f in _FILES if _rel(f) not in KNOWN_INVALID]
    if limit is not None:
        files = files[:limit]
    data = []
    for path in files:
        with contextlib.suppress(OSError):
            data.append(path.read_bytes())
    return data


def _accepted(items: list, fn) -> list:
    """The subset of `items` that `fn` processes without raising. Run once, off
    the timed path, so the benchmark loop never has to swallow errors and the
    reported byte count reflects only what was actually processed."""
    ok = []
    for item in items:
        with contextlib.suppress(Exception):
            fn(item)
            ok.append(item)
    return ok


def _measure(
    label: str, payloads: list, fn, iterations: int, total_bytes: int
) -> float:
    """Best-of-`iterations` wall time for one pass over `payloads`, plus MB/s.

    `payloads` is pre-filtered to items that succeed, so the loop runs clean and
    `total_bytes` is the size of exactly what is timed."""

    def one_pass() -> float:
        start = time.perf_counter()
        for item in payloads:
            fn(item)
        return time.perf_counter() - start

    best = min(one_pass() for _ in range(iterations))
    print(f"  {label:22} {best * 1e3:8.1f} ms   {total_bytes / MB / best:7.1f} MB/s")
    return best


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--limit", type=int, default=None, help="only the first N files"
    )
    parser.add_argument(
        "--iterations", type=int, default=3, help="passes over the corpus (best wins)"
    )
    parser.add_argument(
        "--no-compare", action="store_true", help="skip the PyYAML C-loader comparison"
    )
    args = parser.parse_args()

    if not _FILES:
        print("Real-world corpus not checked out; nothing to benchmark.")
        print("Populate it with: git submodule update --init --recursive")
        return

    payloads = _load_corpus(args.limit)
    total = len(payloads)
    print(f"Corpus: {total} files, {sum(len(b) for b in payloads) / MB:.1f} MB\n")

    # -- loads --------------------------------------------------------------
    parseable = _accepted(payloads, yamlrocks.loads_all)
    ok_bytes = sum(len(b) for b in parseable)
    print(
        f"loads (parse): {len(parseable)}/{total} files parse, {ok_bytes / MB:.1f} MB"
    )
    _measure("YAMLRocks", parseable, yamlrocks.loads_all, args.iterations, ok_bytes)

    if not args.no_compare and _PY_C_LOADER is not None:

        def py_load(b: bytes) -> None:
            list(pyyaml.load_all(b, Loader=_PY_C_LOADER))

        # Compare over the files both libraries accept, so the ratio divides the
        # same bytes for each and is not skewed by differing validity.
        common = _accepted(parseable, py_load)
        common_bytes = sum(len(b) for b in common)
        print(
            f"  comparison over {len(common)} files both accept, {common_bytes / MB:.1f} MB:"
        )
        yr = _measure(
            "YAMLRocks", common, yamlrocks.loads_all, args.iterations, common_bytes
        )
        py = _measure("PyYAML (C)", common, py_load, args.iterations, common_bytes)
        print(f"  -> YAMLRocks is {py / yr:.1f}x faster on real configs")

    # -- dumps --------------------------------------------------------------
    # Serialize the fast-path-loaded data back out (the emitter's real workout);
    # size the throughput by emitted bytes, keeping only objects that dump.
    loaded, out_bytes = [], 0
    for raw in payloads:
        with contextlib.suppress(Exception):
            obj = yamlrocks.loads(raw)
            out_bytes += len(yamlrocks.dumps(obj))
            loaded.append(obj)
    print(
        f"\ndumps (serialize): {len(loaded)}/{total} objects, {out_bytes / MB:.1f} MB out"
    )
    _measure("YAMLRocks", loaded, yamlrocks.dumps, args.iterations, out_bytes)


if __name__ == "__main__":
    main()
