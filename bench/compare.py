"""Cross-library load/dump throughput, the data behind the docs charts.

Where ``bench.py`` prints a table of speedups relative to YAMLRocks, this module
returns structured rows for [`charts.py`](./charts.py) to plot, covering a wider
field of YAML libraries. Each contender loads (and, where it can, dumps) the same
representative payload set from ``bench.py``; the reported number is the time to
process the whole set once, a single blended score per library.

``measure()`` returns one row per *installed* library::

    {"label": str, "impl": "rust" | "c" | "python",
     "load_us": float, "dump_us": float | None}

``dump_us`` is ``None`` for a library with no usable dumper (strictyaml). Missing
libraries are skipped, so the charts render with whatever is installed. Numbers
are machine-dependent; regenerate on the machine you want to quote.
"""

from __future__ import annotations

import io
import time
from collections.abc import Callable
from typing import Any

import bench  # reuse the representative payloads
import yamlrocks


def _time(fn: Callable[[], Any], budget: float = 0.4) -> float:
    """Best (fastest) wall-clock time of one `fn()` call, in microseconds.

    One call per iteration (not batched), because a single call here processes
    the whole payload set and a slow pure-Python library can take a second per
    call. Runs at least three reps and then until `budget` seconds elapse, capped
    so a very slow contender still finishes. The minimum is reported, the usual
    way to squeeze scheduling noise out of a microbenchmark.
    """
    fn()  # warm up (imports, first-touch allocations, any lazy setup)
    best = float("inf")
    reps = 0
    start = time.perf_counter()
    while reps < 3 or (time.perf_counter() - start < budget and reps < 500):
        call_start = time.perf_counter()
        fn()
        best = min(best, time.perf_counter() - call_start)
        reps += 1
    return best * 1e6


# The payload set, reused from ``bench.py`` but with the tiny-map stress rewritten
# in block style: strictyaml rejects flow mappings (`{k: v}`) by design, so a flow
# payload would be a parse error for it rather than a slow parse. Keeping every
# payload block-style lets the whole field parse the same corpus, so the blended
# number is comparable across all of them.
_BLOCK_SMALL_OBJECTS = b"objects:\n" + b"".join(
    b"  - name: n%d\n    value: %d\n" % (i, i) for i in range(500)
)
_PAYLOADS = dict(bench.PAYLOADS)
_PAYLOADS["small_objects (500 tiny maps)"] = _BLOCK_SMALL_OBJECTS

# The payloads as text (every loader here accepts a ``str``), and the canonical
# native objects for the dump benchmark: parsed once with YAMLRocks so every
# dumper serializes exactly the same data.
_TEXTS = [payload.decode("utf-8") for payload in _PAYLOADS.values()]
_OBJECTS = [yamlrocks.loads(text) for text in _TEXTS]


class Contender:
    """One library under test: how to load and (optionally) dump with it."""

    def __init__(
        self,
        label: str,
        impl: str,
        load: Callable[[str], Any],
        dump: Callable[[Any], Any] | None,
    ) -> None:
        self.label = label
        self.impl = impl  # "rust" | "c" | "python", shown in the compare table
        self.load = load
        self.dump = dump


def _contenders() -> list[Contender]:
    """Every installed contender, each with its native load/dump entry points.

    Every optional import catches a broad `Exception`, not just `ImportError`: a
    binary wheel can fail to import for other reasons (a missing shared library,
    an ABI mismatch), and a contender that will not import should be skipped, not
    crash the whole chart run.
    """
    out: list[Contender] = [
        Contender("YAMLRocks", "rust", yamlrocks.loads, yamlrocks.dumps),
    ]

    try:
        import yaml as pyyaml
    except Exception:
        pyyaml = None
    if pyyaml is not None:
        c_loader = getattr(pyyaml, "CSafeLoader", None)
        c_dumper = getattr(pyyaml, "CSafeDumper", None)
        if c_loader is not None:
            # The C loader and dumper come together with libyaml, but guard the
            # dumper anyway so a lone loader never registers a dumper that errors.
            out.append(
                Contender(
                    "PyYAML (C)",
                    "c",
                    lambda s: pyyaml.load(s, Loader=c_loader),
                    (lambda o: pyyaml.dump(o, Dumper=c_dumper))
                    if c_dumper is not None
                    else None,
                )
            )
        out.append(
            Contender(
                "PyYAML (pure)",
                "python",
                lambda s: pyyaml.load(s, Loader=pyyaml.SafeLoader),
                lambda o: pyyaml.dump(o, Dumper=pyyaml.SafeDumper),
            )
        )

    try:
        import ryaml

        out.append(Contender("ryaml", "rust", ryaml.loads, ryaml.dumps))
    except Exception:
        pass

    try:
        import yaml_rs

        out.append(Contender("yaml_rs", "rust", yaml_rs.loads, yaml_rs.dumps))
    except Exception:
        pass

    try:
        import yaml12  # the "py-yaml12" distribution

        out.append(
            Contender("py-yaml12", "rust", yaml12.parse_yaml, yaml12.format_yaml)
        )
    except Exception:
        pass

    try:
        import oyaml

        out.append(Contender("oyaml", "python", oyaml.safe_load, oyaml.safe_dump))
    except Exception:
        pass

    try:
        from ruamel.yaml import YAML as RuamelYAML

        ruamel = RuamelYAML(typ="safe")
        ruamel.default_flow_style = False

        def _ruamel_dump(obj: Any) -> str:
            buf = io.StringIO()
            ruamel.dump(obj, buf)
            return buf.getvalue()

        out.append(
            Contender(
                "ruamel.yaml",
                "python",
                lambda s: ruamel.load(io.StringIO(s)),
                _ruamel_dump,
            )
        )
    except Exception:
        pass

    try:
        import strictyaml

        # strictyaml has no drop-in dumper for arbitrary native data (it needs a
        # schema and its own document type), so it is load-only here. Its load
        # also returns every scalar as a string, so it is not type-for-type equal
        # to the others; it is included to place the strict, pure-Python end of
        # the field, not as a like-for-like data point.
        out.append(
            Contender("strictyaml", "python", lambda s: strictyaml.load(s).data, None)
        )
    except Exception:
        pass

    try:
        import yamlium

        out.append(
            Contender(
                "yamlium",
                "python",
                lambda s: yamlium.parse(s).to_dict(),
                lambda o: yamlium.from_dict(o).to_yaml(),
            )
        )
    except Exception:
        pass

    return out


def measure() -> list[dict[str, Any]]:
    """Time load (and dump) of the whole payload set once, per installed library.

    Sorted fastest load first, so the charts can plot in order.
    """
    rows: list[dict[str, Any]] = []
    for contender in _contenders():
        load = contender.load

        # A plain for-loop, not a list comprehension: building a list each call
        # would add allocation overhead to the measurement, which matters for the
        # fastest contenders. We only want to time the library calls.
        def _load_all() -> None:
            for text in _TEXTS:
                load(text)

        try:
            load_us = _time(_load_all)
        except Exception as error:
            # A contender that cannot parse the corpus (a parse error or an
            # unsupported construct) is reported and dropped, not fatal.
            print(f"skipping {contender.label}: cannot load the corpus ({error})")
            continue
        dump_us: float | None = None
        if contender.dump is not None:
            dump = contender.dump

            def _dump_all() -> None:
                for obj in _OBJECTS:
                    dump(obj)

            try:
                dump_us = _time(_dump_all)
            except Exception as error:
                print(f"note: {contender.label} cannot dump the corpus ({error})")
        rows.append(
            {
                "label": contender.label,
                "impl": contender.impl,
                "load_us": load_us,
                "dump_us": dump_us,
            }
        )
    rows.sort(key=lambda row: row["load_us"])
    return rows


def main() -> None:
    """Print the measured rows as a small table."""
    rows = measure()
    width = max(len(row["label"]) for row in rows)
    print(f"{'library':<{width}}  {'impl':<7}  {'load':>10}  {'dump':>10}")
    for row in rows:
        dump = "-" if row["dump_us"] is None else f"{row['dump_us']:.1f} us"
        print(
            f"{row['label']:<{width}}  {row['impl']:<7}  "
            f"{row['load_us']:.1f} us  {dump:>10}"
        )


if __name__ == "__main__":
    main()
