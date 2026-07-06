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


def _time_pass(fn, payloads: list[bytes]) -> tuple[float, int]:
    """One pass over the corpus; returns (seconds, files-that-succeeded)."""
    ok = 0
    start = time.perf_counter()
    for raw in payloads:
        try:
            fn(raw)
            ok += 1
        except Exception:
            # A handful of files are invalid or need options this path does not
            # apply; they are not the point of a throughput measurement.
            pass
    return time.perf_counter() - start, ok


def _report(label: str, payloads: list, fn, iterations: int, total_bytes: int) -> float:
    best = min(_time_pass(fn, payloads)[0] for _ in range(iterations))
    mb = total_bytes / 1_048_576
    print(f"  {label:22} {best * 1e3:8.1f} ms   {mb / best:7.1f} MB/s")
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
    in_bytes = sum(len(b) for b in payloads)
    print(f"Corpus: {len(payloads)} files, {in_bytes / 1_048_576:.1f} MB\n")

    print("loads (parse):")
    yr_loads = _report(
        "YAMLRocks", payloads, yamlrocks.loads_all, args.iterations, in_bytes
    )
    if not args.no_compare and _PY_C_LOADER is not None:
        py_loads = _report(
            "PyYAML (C)",
            payloads,
            lambda b: list(pyyaml.load_all(b, Loader=_PY_C_LOADER)),
            args.iterations,
            in_bytes,
        )
        print(f"  -> YAMLRocks is {py_loads / yr_loads:.1f}x faster on real configs\n")

    # Dump the fast-path-loaded data back out (the emitter's real-world workout).
    # Size the throughput by the emitted bytes, the natural measure for a dump.
    loaded, out_bytes = [], 0
    for raw in payloads:
        try:
            obj = yamlrocks.loads(raw)
            loaded.append(obj)
            out_bytes += len(yamlrocks.dumps(obj))
        except Exception:
            pass
    print("dumps (serialize):")
    _report("YAMLRocks", loaded, yamlrocks.dumps, args.iterations, out_bytes)


if __name__ == "__main__":
    main()
