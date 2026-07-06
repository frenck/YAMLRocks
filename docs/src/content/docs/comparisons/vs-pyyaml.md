---
title: YAMLRocks vs PyYAML
description: A detailed comparison of YAMLRocks and PyYAML, with benchmarks and side-by-side code.
---

[PyYAML](https://pyyaml.org/) is the de-facto standard YAML library for Python.
It is mature and widely used, but it is YAML 1.1 only, discards comments, has no
round-trip mode, and its safest, fastest path still trails a modern Rust
implementation. YAMLRocks is a drop-in-friendly alternative that is faster on every
operation, safe by default, and far more capable.

## Feature comparison

| Feature                       |      PyYAML       |            YAMLRocks             |
| ----------------------------- | :---------------: | :------------------------------: |
| YAML 1.2                      |   No (1.1 only)   |   Yes (1.2 default, 1.1 mode)    |
| Speed (parse)                 |     C loader      | ~6-10x faster than the C loader  |
| Speed (dump)                  |     C dumper      | ~17-19x faster than the C dumper |
| Comment preservation          |        No         |               Yes                |
| Round-trip (byte-for-byte)    |        No         |               Yes                |
| Anchors/aliases preserved     |        No         |               Yes                |
| Merge keys (`<<`)             |        Yes        |               Yes                |
| Native `!include`             |        No         |      Yes (with write-back)       |
| Source line/column            |        No         |       Yes (annotated mode)       |
| JSON Schema validation        |        No         |    Yes (line-numbered errors)    |
| Arbitrary object construction | Yes (`yaml.load`) |          None by design          |
| Output type                   |       `str`       |    `bytes` (no extra encode)     |
| datetime/date/time dump       |      partial      |    Yes (with offset control)     |
| Free-threaded (nogil) safe    |        No         |               Yes                |

## Safety: no arbitrary object construction

PyYAML's headline footgun is `yaml.load` with the default loader, which
constructs arbitrary Python objects from tags like `!!python/object/apply`. That
is remote code execution waiting to happen, which is why PyYAML added
`safe_load` and now warns when you call `load` without an explicit loader.

YAMLRocks has no unsafe path to forget. Tags never construct Python objects. An
unrecognized tag keeps its underlying scalar, or you opt in to handle it
yourself with a `tag_handler` or `OPT_PASSTHROUGH_TAG`:

```python
import yamlrocks

# A tag never executes anything. The value is just the scalar underneath.
yamlrocks.loads(b"value: !something 42")          # {'value': '42'}

# Opt in to interpret a tag, on your terms.
yamlrocks.loads(
    b"value: !double 5",
    tag_handler=lambda tag, value: int(value) * 2 if tag == "!double" else value,
)                                              # {'value': 10}
```

There is no `yamlrocks.load` that behaves like `yaml.load`. The safe behavior is
the only behavior. See [security](/reference/security/) and
[custom tags](/guides/tags/).

## YAML 1.2 by default: the Norway problem

PyYAML follows YAML 1.1, where `yes`, `no`, `on`, `off`, and the country code
`NO` parse as booleans. This famously corrupts configuration files. YAMLRocks
defaults to YAML 1.2, where those are plain strings:

```python
import yamlrocks

yamlrocks.loads(b"country: NO")                   # {'country': 'NO'}
yamlrocks.loads(b"enabled: yes")                  # {'enabled': 'yes'}
```

If you need the old behavior for a specific document, opt in with
`OPT_YAML_1_1`, and use [`yamlrocks.upgrade()`](/guides/yaml-11-vs-12/) to migrate
legacy 1.1 files to canonical 1.2:

```python
import yamlrocks

yamlrocks.loads(b"enabled: yes", option=yamlrocks.OPT_YAML_1_1)   # {'enabled': True}
yamlrocks.upgrade(b"enabled: yes\nmode: on\n")
# b'%YAML 1.2\n---\nenabled: true\nmode: true\n'
```

The same 1.1 heritage means PyYAML also resolves implicit timestamps and
sexagesimals, so a plain `2024-01-15` becomes a `datetime.date` and a bare
`13:30:45` becomes the integer `48645` (base-60), a config value silently turned
into a large number. YAMLRocks stays clean by default and makes timestamp typing
an explicit opt-in that never touches other 1.1 forms:

```python
import datetime
import yamlrocks

yamlrocks.loads(b"at: 13:30:45")                                 # {'at': '13:30:45'}
yamlrocks.loads(b"on: 2024-01-15", option=yamlrocks.OPT_TIMESTAMPS)  # {'on': datetime.date(2024, 1, 15)}
```

See [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for the full list of differences,
and [timestamps](/reference/options/#timestamps) for the opt-in.

## Comments and round-trip editing

PyYAML throws comments away on load and cannot reproduce a file. There is no
supported way to load `config.yaml`, change one value, and write it back without
losing the comments and reflowing everything.

YAMLRocks's round-trip mode preserves comments, anchors, and layout, and re-emits
only what changed:

```python
import yamlrocks

source = b"# service config\nname: app  # the app name\nport: 8080\n"
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
doc["port"] = 9090
doc.to_yaml()
# b'# service config\nname: app  # the app name\nport: 9090\n'
```

The comment survives, and the unchanged lines are reproduced verbatim. See
[round-trip editing](/guides/round-trip/) and the
[config editor recipe](/recipes/config-editor/).

## Bytes out, no extra encode

`yaml.safe_dump` returns a `str`, so writing to a file or socket means a second
UTF-8 encode. `yamlrocks.dumps` returns `bytes` directly, ready to
write:

```python
import yamlrocks

yamlrocks.dumps({"key": "value", "list": [1, 2]})
# b'key: value\nlist:\n  - 1\n  - 2\n'
```

## Native includes

PyYAML has no `!include`. The common workaround is a custom constructor that you
wire up yourself and that cannot write changes back. YAMLRocks resolves `!include`
and the `!include_dir_*` family natively, and round-trip mode can save an edited
value back into the exact file it came from. See [includes](/guides/includes/)
and the [Home Assistant recipe](/recipes/home-assistant/).

## Pain points YAMLRocks resolves

Beyond the headline differences above, YAMLRocks quietly fixes a long list of
recurring PyYAML frustrations:

| Common PyYAML frustration                                                            | YAMLRocks                                                                                                        |
| ------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| Install fails to build (Cython errors, no wheel for your platform or Python)         | Pure Rust via maturin: prebuilt wheels, no C toolchain, and it builds and runs on free-threaded CPython and 3.14 |
| An integer beyond 64 bits raises `OverflowError` on dump, or loads as a string       | Arbitrary-precision integers load, dump, and round-trip exactly                                                  |
| Large or small floats dump as long digit strings (`1e308` becomes 309 digits)        | Scientific notation matching Python's `repr` (`1.0e+308`, `6.022e+23`)                                           |
| Multi-line strings dump as one ugly `"a\nb\n"` line                                  | A readable literal `\|` block by default, with the right chomping                                                |
| `OrderedDict`, `Decimal`, `Enum`, `UUID`, or `pathlib.Path` raise `RepresenterError` | Serialized natively, no custom representer needed                                                                |
| An unknown or custom tag raises `ConstructorError`                                   | Kept as its underlying value, or surfaced as a `YAMLRocksTag`; never an error                                    |
| Characters above U+FFFF (emoji, rare scripts) are mishandled                         | Full Unicode, including in escapes and as mapping keys                                                           |
| `yaml.dump` output differs across platforms or appends a stray blank line            | Deterministic output, exactly one trailing newline                                                               |

Each of these is exercised by the test suite, and the emitter's output is checked
to be [yamllint](https://yamllint.readthedocs.io/)-clean by default.

## Performance

![YAMLRocks vs PyYAML's C loader on reading and writing: YAMLRocks is about ten times faster on both.](/benchmarks/vs-pyyaml.svg)

Indicative figures from `python bench/bench.py` (release build), showing how many
times **faster YAMLRocks is** than PyYAML. The C loader (`libyaml`) is the harder
target; the pure-Python loader is what most environments fall back to.

**Parsing (`loads`)**

| Payload               | vs PyYAML (C) | vs PyYAML (pure) |
| --------------------- | ------------: | ---------------: |
| small (10 lines)      |    ~8x faster |      ~64x faster |
| medium (k8s manifest) |    ~9x faster |      ~80x faster |
| large (500 items)     |   ~10x faster |      ~87x faster |
| deep (30 levels)      |    ~6x faster |      ~69x faster |

**Serializing (`dumps`)**

| Payload | vs PyYAML (C) | vs PyYAML (pure) |
| ------- | ------------: | ---------------: |
| small   |   ~18x faster |      ~94x faster |
| medium  |   ~18x faster |      ~92x faster |
| large   |   ~17x faster |      ~86x faster |
| deep    |   ~17x faster |      ~75x faster |

**Split configuration with `!include`** (Home Assistant style): YAMLRocks's native
resolver versus a PyYAML `!include` constructor.

| Files | YAMLRocks is |
| ----- | -----------: |
| 50    |  ~17x faster |
| 200   |  ~17x faster |
| 500   |  ~17x faster |

:::note[These are ratios]
The numbers above are speed ratios, not wall-clock times. They depend on payload
shape and hardware. Reproduce them with `python bench/bench.py`.
:::

## Migrating

Use the [compatibility shim](/getting-started/migrating-from-pyyaml/) for a
near drop-in switch:

```python
import yamlrocks.compat as yaml

yaml.safe_load(b"a: 1")            # {'a': 1}
yaml.safe_dump({"a": 1})          # 'a: 1\n'  (a str, matching PyYAML)
```

`safe_load`, `safe_load_all`, `safe_dump`, and `safe_dump_all` map straight
across, with `sort_keys=True` defaulting as it does in PyYAML. The shim's `load`
and `dump` map onto the safe variants, because YAMLRocks never executes code from
tags. For new code, prefer the native `loads`/`dumps` API and its `bytes` output.

## When to stick with PyYAML

PyYAML is a reasonable choice when you only need basic 1.1 loading, want zero
native dependencies, or rely on its custom `Loader`/`Dumper` extension points and
`add_constructor`/`add_representer` hooks. YAMLRocks does not expose those
constructor hooks; it provides `tag_handler`, `OPT_PASSTHROUGH_TAG`, and the
`default` callback instead. For everything else (speed, round-trip, includes,
validation, YAML 1.2, and safety) YAMLRocks is the upgrade.

## See also

- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/): the full
  shim reference.
- [YAMLRocks vs ruamel.yaml](/comparisons/vs-ruamel/): the round-trip comparison.
- [Security](/reference/security/): the safety model in detail.
- [Round-trip editing](/guides/round-trip/) and [includes](/guides/includes/).
