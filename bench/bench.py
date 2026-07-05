#!/usr/bin/env python3
"""Benchmark YAMLRocks against PyYAML, ruamel.yaml, and yamlium.

Measures ``loads`` (parse) and ``dumps`` (serialize) throughput across a range
of representative payloads and prints a table of timings and speedups relative
to YAMLRocks. Libraries that are not installed are skipped gracefully.

Contenders:

* **PyYAML (C)** - the ``libyaml``-backed ``CSafeLoader``/``CSafeDumper`` (only
  shown when ``libyaml`` is available).
* **PyYAML (pure)** - the pure-Python ``SafeLoader``/``SafeDumper``, the
  fallback most environments use when ``libyaml`` is not installed.
* **ruamel.yaml** - the pure-Python round-trip library, in its ``safe`` mode.
* **yamlium** - a pure-Python round-trip parser; parsed via ``to_dict`` and
  emitted via ``from_dict`` so the comparison is like for like.

Usage::

    python bench/bench.py            # run all benchmarks
    python bench/bench.py --save     # also write bench/RESULTS.md

Always run under a memory/time guard during development, e.g.::

    timeout 120 bash -c 'ulimit -v 3000000; python bench/bench.py'
"""

from __future__ import annotations

import argparse
import io
import os
import sys
import tempfile
import time
from typing import NamedTuple

import yamlrocks

# -- Optional comparison libraries -------------------------------------------

try:
    import yaml as pyyaml

    # The C loader/dumper, only when libyaml is built in. Kept separate from the
    # pure-Python path so both can be reported side by side.
    _PY_C_LOADER = getattr(pyyaml, "CSafeLoader", None)
    _PY_C_DUMPER = getattr(pyyaml, "CSafeDumper", None)
except Exception:  # pragma: no cover - optional dependency
    pyyaml = None
    _PY_C_LOADER = _PY_C_DUMPER = None

try:
    from ruamel.yaml import YAML as RuamelYAML
except Exception:  # pragma: no cover - optional dependency
    RuamelYAML = None

try:
    import yamlium
except Exception:  # pragma: no cover - optional dependency
    yamlium = None


# -- Payloads ----------------------------------------------------------------

SMALL = b"""\
name: my-app
version: 1.2.3
debug: false
port: 8080
tags:
  - web
  - api
owner:
  name: Alice
  email: alice@example.com
"""

MEDIUM = (
    b"apiVersion: apps/v1\n"
    b"kind: Deployment\n"
    b"metadata:\n  name: nginx\n  labels:\n    app: nginx\n"
    b"spec:\n  replicas: 3\n  template:\n    spec:\n      containers:\n"
    + b"".join(
        b"        - name: c%d\n          image: nginx:1.25\n          ports:\n"
        b"            - containerPort: %d\n" % (i, 8000 + i)
        for i in range(10)
    )
)

# A large mapping with many repeated keys (the common config shape).
LARGE = b"items:\n" + b"".join(
    b"  - id: %d\n    name: item-%d\n    enabled: true\n    score: %d.5\n" % (i, i, i)
    for i in range(500)
)


# A deeply nested document.
def _deep(n: int) -> bytes:
    return (
        b"".join(b"  " * i + b"level%d:\n" % i for i in range(n))
        + b"  " * n
        + b"value: 1\n"
    )


DEEP = _deep(30)

# Many tiny 2-key mappings: the worst case for object creation, stressing dict
# allocation and the mapping-key interning that is YAMLRocks's strength.
SMALL_OBJECTS = b"objects:\n" + b"".join(
    b"  - {name: n%d, value: %d}\n" % (i, i) for i in range(500)
)

# A flat list of long plain (unquoted) scalars: stresses the plain-scalar
# scanner's bulk content run (the lookup-table fast path), with no structure.
STRINGS_ARRAY = b"lines:\n" + b"".join(
    b"  - this is a fairly long unquoted plain scalar line number %d with several words in it\n"
    % i
    for i in range(500)
)

# The same shape but single-quoted: stresses the quoted-scalar scanner, the
# common shape of config exports and generated YAML. Paired with STRINGS_ARRAY
# so the plain and quoted scalar paths are measured side by side.
QUOTED_STRINGS = b"lines:\n" + b"".join(
    b"  - 'this is a fairly long single-quoted scalar line number %d with several words in it'\n"
    % i
    for i in range(500)
)

PAYLOADS = {
    "small (10 lines)": SMALL,
    "medium (k8s, ~50 lines)": MEDIUM,
    "large (500 items)": LARGE,
    "deep (30 levels)": DEEP,
    "small_objects (500 tiny maps)": SMALL_OBJECTS,
    "strings_array (500 long scalars)": STRINGS_ARRAY,
    "quoted_strings (500 quoted scalars)": QUOTED_STRINGS,
}


# -- Timing helpers ----------------------------------------------------------


class Timing(NamedTuple):
    """Per-call timings in microseconds: the best (min) and the 95th percentile.

    Reporting both the best and the tail (rather than a single mean) shows the
    spread: the best is the achievable throughput, the p95 catches GC pauses and
    allocator jitter a mean would smear over.
    """

    best: float
    p95: float


def _bench(fn, *, min_time: float = 0.3) -> Timing:
    """Time `fn` over repeated batches, returning the best and p95 per-call time."""
    # Warm up.
    fn()
    samples = []
    deadline = time.perf_counter() + min_time
    batch = 50
    while time.perf_counter() < deadline or len(samples) < 3:
        start = time.perf_counter()
        for _ in range(batch):
            fn()
        elapsed = time.perf_counter() - start
        samples.append(elapsed / batch)
    samples.sort()
    # Nearest-rank p95 over the batch-average samples.
    p95 = samples[min(len(samples) - 1, int(0.95 * len(samples)))]
    return Timing(best=samples[0] * 1e6, p95=p95 * 1e6)


def _fmt(us: float) -> str:
    if us >= 1000:
        return f"{us / 1000:.2f} ms"
    return f"{us:.1f} us"


# -- Contenders --------------------------------------------------------------


def loaders():
    yield "YAMLRocks", lambda data: yamlrocks.loads(data)
    if _PY_C_LOADER is not None:
        yield "PyYAML (C)", lambda data: pyyaml.load(data, Loader=_PY_C_LOADER)
    if pyyaml is not None:
        yield "PyYAML (pure)", lambda data: pyyaml.load(data, Loader=pyyaml.SafeLoader)
    if RuamelYAML is not None:
        ry = RuamelYAML(typ="safe")
        yield "ruamel(safe)", lambda data: ry.load(io.BytesIO(data))
    if yamlium is not None:
        # yamlium parses text into a round-trip tree; to_dict yields native data,
        # the fair counterpart to the other loaders.
        yield "yamlium", lambda data: yamlium.parse(data.decode("utf-8")).to_dict()


def dumpers():
    yield "YAMLRocks", lambda obj: yamlrocks.dumps(obj)
    if _PY_C_DUMPER is not None:
        yield "PyYAML (C)", lambda obj: pyyaml.dump(obj, Dumper=_PY_C_DUMPER)
    if pyyaml is not None:
        yield "PyYAML (pure)", lambda obj: pyyaml.dump(obj, Dumper=pyyaml.SafeDumper)
    if RuamelYAML is not None:
        ry = RuamelYAML(typ="safe")
        ry.default_flow_style = False

        def _ru(obj):
            buf = io.BytesIO()
            ry.dump(obj, buf)
            return buf.getvalue()

        yield "ruamel(safe)", _ru
    if yamlium is not None:
        # Build a yamlium tree from native data, then emit it.
        yield "yamlium", lambda obj: yamlium.from_dict(obj).to_yaml()


# -- Reporting ---------------------------------------------------------------


def run() -> str:
    lines = []

    def out(s=""):
        print(s)
        lines.append(s)

    out("# YAMLRocks benchmarks")
    out()
    out(f"Python {sys.version.split()[0]}")
    out()

    out("## loads (parse)")
    out()
    for name, data in PAYLOADS.items():
        out(f"### {name}")
        results = {label: _bench(lambda: fn(data)) for label, fn in loaders()}
        _table(out, results)
        out()

    out("## dumps (serialize)")
    out()
    for name, data in PAYLOADS.items():
        obj = yamlrocks.loads(data)
        out(f"### {name}")
        results = {label: _bench(lambda: fn(obj)) for label, fn in dumpers()}
        _table(out, results)
        out()

    out("## includes (Home Assistant-style split config)")
    out()
    _bench_includes(out)

    return "\n".join(lines)


def _make_include_tree(root_dir: str, count: int) -> bytes:
    """Write a root config that ``!include``s ``count`` small files.

    Mimics a Home Assistant configuration split across many files, e.g.
    ``automation: !include automations/0001.yaml`` repeated hundreds of times.
    """
    inc_dir = os.path.join(root_dir, "packages")
    os.makedirs(inc_dir, exist_ok=True)
    root_lines = []
    for i in range(count):
        name = f"pkg_{i:04d}"
        with open(os.path.join(inc_dir, f"{name}.yaml"), "w") as f:
            f.write(
                f"name: {name}\n"
                f"enabled: true\n"
                f"settings:\n  retries: {i % 5}\n  timeout: {10 + i}\n"
                f"items:\n  - a\n  - b\n  - c\n"
            )
        root_lines.append(f"{name}: !include packages/{name}.yaml")
    return ("\n".join(root_lines) + "\n").encode()


def _pyyaml_include_loader():
    """A PyYAML SafeLoader with an ``!include`` constructor (HA-style)."""
    if pyyaml is None:
        return None

    class IncludeLoader(pyyaml.SafeLoader):
        pass

    def _construct_include(loader, node):
        rel = loader.construct_scalar(node)
        path = os.path.join(loader._root_dir, rel)
        with open(path, "rb") as f:
            return pyyaml.load(f, Loader=lambda s: _bound(s, loader._root_dir))

    def _bound(stream, root):
        ldr = IncludeLoader(stream)
        ldr._root_dir = root
        return ldr

    IncludeLoader.add_constructor("!include", _construct_include)
    return IncludeLoader, _bound


def _bench_includes(out) -> None:
    for count in (50, 200, 500):
        with tempfile.TemporaryDirectory() as tmp:
            root = _make_include_tree(tmp, count)
            out(f"### {count} included files")

            results = {
                "YAMLRocks": _bench(
                    lambda: yamlrocks.loads(
                        root, option=yamlrocks.OPT_INCLUDES, include_dir=tmp
                    ),
                    min_time=0.5,
                )
            }

            loader = _pyyaml_include_loader()
            if loader is not None:
                _, bind = loader

                def _py_load():
                    return pyyaml.load(io.BytesIO(root), Loader=lambda s: bind(s, tmp))

                results["PyYAML (pure, !include ctor)"] = _bench(_py_load, min_time=0.5)

            _table(out, results)
            out()


def _table(out, results: dict) -> None:
    base = results.get("YAMLRocks")
    out("| library | best | p95 | YAMLRocks is |")
    out("| --- | ---: | ---: | ---: |")
    for label, timing in results.items():
        # Frame the comparison as YAMLRocks's advantage: how many times faster
        # YAMLRocks is than this library, on the best (achievable) time.
        if label == "YAMLRocks":
            rel = "baseline"
        elif base:
            rel = f"{timing.best / base.best:.1f}x faster"
        else:
            rel = "-"
        out(f"| {label} | {_fmt(timing.best)} | {_fmt(timing.p95)} | {rel} |")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--save", action="store_true", help="write bench/RESULTS.md")
    args = parser.parse_args()
    report = run()
    if args.save:
        import pathlib

        path = pathlib.Path(__file__).parent / "RESULTS.md"
        path.write_text(report + "\n")
        print(f"\nSaved {path}")


if __name__ == "__main__":
    main()
