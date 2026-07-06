---
title: Performance
description: How fast YAMLRocks is, why, and how to get the most from it.
---

YAMLRocks is built for speed from the ground up: a custom Rust scanner and parser,
zero-copy scalar borrowing, direct Python object construction through the CPython
API, interned mapping keys, and a release profile tuned with fat LTO. The result
is a library that is faster than PyYAML's C loader on every operation and
dramatically faster than the pure-Python round-trip libraries.

Run `python bench/bench.py` in the repository to reproduce these numbers on your
own machine. The figures below come from a release build and are indicative.
Your hardware, payload, and Python version will move them around.

## Across the field

Loading and dumping a representative set of payloads once, across ten YAML
libraries. Lower is faster, and the scale is logarithmic, so each gridline is 10x.

![Parsing throughput across libraries: YAMLRocks is fastest, ahead of yaml_rs, fast-yaml, py-yaml12, ryaml, PyYAML's C loader, and far ahead of the pure-Python libraries.](/benchmarks/load.svg)

![Serializing throughput across libraries: YAMLRocks is fastest, ahead of yaml_rs, py-yaml12, fast-yaml, ryaml, yamlium, PyYAML's C dumper, and the pure-Python libraries.](/benchmarks/dump.svg)

YAMLRocks leads on both. The closest contenders are the other Rust-backed parsers
(`yaml_rs`, `ryaml`, `py-yaml12`), and the comparison is not quite like for like:
none of them apply YAML merge keys (a `<<` is left as a literal key); `ryaml` and
`yaml_rs` reject a duplicate key outright where YAMLRocks (and PyYAML) keep the
last; `ryaml` errors on an integer larger than 64 bits and `py-yaml12` returns it
as a lossy float. `oyaml` is PyYAML with ordered dicts, so it tracks pure-Python
PyYAML exactly. `strictyaml` is deliberately restrictive (it rejects flow style
and returns every scalar as a string) and has no general dumper, so it appears in
the parsing chart only.

Regenerate the charts with `just charts`, which builds a release extension first
(a debug build is several times slower, and would understate YAMLRocks against
the other libraries' release wheels).

## Headline numbers

Every figure below is how many times **faster YAMLRocks is** than the named library.

- **Parsing**: YAMLRocks is **~6-10x faster than PyYAML's C `CSafeLoader`**, ~64-87x
  faster than pure-Python PyYAML, ~105-141x faster than ruamel.yaml, and ~27-34x
  faster than yamlium.
- **Serializing**: YAMLRocks is **~17-19x faster than PyYAML's C `CSafeDumper`**,
  ~75-94x faster than pure-Python PyYAML, ~160-208x faster than ruamel.yaml, and
  ~8-12x faster than yamlium.
- **Native includes**: YAMLRocks is **~17x faster** than a PyYAML `!include`
  constructor for configurations split across hundreds of files, exactly the
  Home Assistant startup and reload pattern.

Most environments without `libyaml` installed fall back to pure-Python PyYAML,
which is where the largest gap shows.

## Parsing (`loads`)

How many times faster YAMLRocks is at parsing each payload:

| Payload               | vs PyYAML (C) | vs PyYAML (pure) |    vs ruamel |  vs yamlium |
| --------------------- | ------------: | ---------------: | -----------: | ----------: |
| small (10 lines)      |    ~8x faster |      ~64x faster | ~105x faster | ~28x faster |
| medium (k8s manifest) |    ~9x faster |      ~80x faster | ~124x faster | ~31x faster |
| large (500 items)     |   ~10x faster |      ~87x faster | ~141x faster | ~34x faster |
| deep (30 levels)      |    ~6x faster |      ~69x faster | ~105x faster | ~27x faster |

## Serializing (`dumps`)

How many times faster YAMLRocks is at serializing each payload:

| Payload | vs PyYAML (C) | vs PyYAML (pure) |    vs ruamel |  vs yamlium |
| ------- | ------------: | ---------------: | -----------: | ----------: |
| small   |   ~18x faster |      ~94x faster | ~208x faster | ~11x faster |
| medium  |   ~18x faster |      ~92x faster | ~201x faster | ~12x faster |
| large   |   ~17x faster |      ~86x faster | ~199x faster | ~12x faster |
| deep    |   ~17x faster |      ~75x faster | ~163x faster |  ~8x faster |

yamlium emits comparatively quickly, so its dump gap is the smallest of the four,
but YAMLRocks still leads on every shape. The margin narrows as individual strings
grow very long (where yamlium serializes straight from the original Python `str`
objects and YAMLRocks copies each string once more), yet YAMLRocks stays ahead
even for an array of 500 long plain strings.

## Includes (Home Assistant-style split config)

How many times faster YAMLRocks's native include resolver is than a PyYAML
`!include` constructor:

| Files | YAMLRocks is |
| ----- | -----------: |
| 50    |  ~17x faster |
| 200   |  ~17x faster |
| 500   |  ~17x faster |

The constructor approach re-enters the Python parser once per file and rebuilds
the loader machinery each time. YAMLRocks resolves the whole include graph in Rust
in a single pass, which is why the gap stays wide as the file count grows.

## Why it is fast

The speed is not one trick; it is a stack of decisions that each remove work from
the hot path.

- **A custom Rust scanner and parser.** There is no general-purpose dependency to
  fight. The scanner is tuned for the shapes real configs use (short keys, plain
  scalars, repetitive structure), so the common cases stay on the fast path.
- **Zero-copy scalar borrowing.** A single-line plain scalar that needs no
  unescaping is read directly out of the input buffer rather than copied into a
  fresh allocation. The bytes you passed in _are_ the scalar, right up until a
  Python `str` is built from them.
- **Direct Python object construction.** Plain `loads` skips the rich round-trip
  AST (the one that preserves comments, styles, and spans). It resolves events
  into a lean Rust `Value` tree and builds the Python `dict`s and `list`s from it
  with raw CPython calls (`PyList_New` + `PyList_SET_ITEM`, `PyDict_New` +
  `PyDict_SetItem`), avoiding the per-element overhead of `append`/`__setitem__`.
- **Interned, cached mapping keys.** Repeated keys (the norm in configuration,
  where every list item shares the same fields) are interned once per document and
  reused, so the intern-table lookup happens once per distinct key rather than per
  occurrence. That cuts allocations and makes later dictionary lookups faster.
- **A tuned release profile.** The release build uses `lto = "fat"`,
  `codegen-units = 1`, and `opt-level = 3`, letting the optimizer inline across
  the whole crate.

## Getting the most from it

- **Pass `bytes`, not `str`.** YAMLRocks is happiest with `bytes`. If
  your YAML already arrives as bytes from a socket or file, hand them straight to
  `loads` and skip a UTF-8 round trip.
- **Write `dumps` output directly.** `dumps` returns `bytes`, so you can write it
  to a file or socket without an extra `.encode()`.

```python
import yamlrocks

source = """
name: app
port: 8080
"""

data = yamlrocks.loads(source)
payload = yamlrocks.dumps(data)        # already bytes
with open("out.yaml", "wb") as f:   # note "wb"
    f.write(payload)
```

- **Use the fast path, not round-trip, unless you need comments.** Plain
  `loads`/`dumps` skip building the rich `YAMLRocksDocument` tree. Reach for
  `OPT_ROUND_TRIP` only when you actually need to preserve comments and layout.
- **Prefer native includes.** For split configurations, `OPT_INCLUDES` resolves
  the whole graph in Rust, far faster than a hand-rolled `!include` constructor.
- **Build a release wheel.** Install from PyPI (`pip install yamlrocks`) or build
  with `maturin build --release`. Debug builds are many times slower; never
  benchmark one.

## Reproducing the benchmarks

The benchmark harness lives in the repository and compares YAMLRocks against PyYAML
(C loader) and ruamel.yaml across the payloads in the tables above:

```bash
python bench/bench.py
```

It prints per-payload timings and the relative speedups. Run it on the machine
and Python build you care about; numbers from someone else's laptop are only a
rough guide.

## Free-threaded Python (nogil)

YAMLRocks is free-threaded safe. On a free-threaded (nogil) CPython build, parsing
and serializing run without holding the GIL, so multiple threads can load and
dump YAML in parallel and actually use multiple cores. There is no special flag
to set: the same `loads`/`dumps` calls scale across threads on a free-threaded
interpreter.

## See also

- [Loading YAML](/guides/loading/) and [Dumping YAML](/guides/dumping/).
- [Includes](/guides/includes/): the native `!include` resolver.
- [Comparisons](/comparisons/): YAMLRocks against PyYAML and ruamel.yaml.
- [Architecture](/contributing/architecture/): how the parser is built.
