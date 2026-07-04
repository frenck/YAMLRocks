---
title: How YAMLRocks compares
description: YAMLRocks versus every major Python YAML library at a glance, with benchmarks.
---

The Python YAML ecosystem has long forced a choice: **PyYAML** (fast with the C
loader, but YAML 1.1 only, no comments, no round-trip) or **ruamel.yaml**
(YAML 1.2 with comments and round-trip, but pure Python and slow). YAMLRocks
refuses the trade-off. It is Rust-fast _and_ round-trip capable, with native
includes, schema validation, and source tracking on top.

## At a glance

|                                       |      PyYAML      | ruamel.yaml  |       YAMLRocks        |
| ------------------------------------- | :--------------: | :----------: | :--------------------: |
| YAML 1.2                              |        No        |     Yes      |        **Yes**         |
| Implementation                        |    C + Python    | Pure Python  |        **Rust**        |
| Parse speed                           |     C loader     |     slow     | **5-10x vs PyYAML C**  |
| Dump speed                            |     C dumper     |     slow     | **15-19x vs PyYAML C** |
| Comments preserved                    |        No        |     Yes      |        **Yes**         |
| Byte-for-byte round-trip (unmodified) |        No        |    Close     |        **Yes**         |
| Native `!include` (+ write-back)      |        No        |      No      |        **Yes**         |
| JSON Schema validation                |        No        |      No      |        **Yes**         |
| Source line/column                    |        No        |   partial    |        **Yes**         |
| Safe by default (no code exec)        | No (`yaml.load`) | Yes (`safe`) |        **Yes**         |
| Bytes output (no extra encode)        |        No        |      No      |        **Yes**         |
| Free-threaded (nogil) safe            |        No        |      No      |        **Yes**         |

## Performance headline

Release-build benchmarks (`python bench/bench.py`), showing how many times
faster YAMLRocks is:

- **Parsing**: ~5-10x faster than PyYAML's C loader; ~85-135x faster than ruamel.
- **Serializing**: ~15-19x faster than PyYAML's C dumper; ~155-210x faster than
  ruamel.
- **Split configs with `!include`**: ~18x faster than a PyYAML `!include`
  constructor for hundreds of files.

These are ratios, not absolute times, and they vary with payload shape and
hardware. Run `python bench/bench.py` on your own machine to reproduce them. The
[performance guide](/guides/performance/) explains where the speed comes from
and how to measure your own workload.

## The whole field

The Python YAML ecosystem is larger than PyYAML and ruamel. There are newer
Rust-backed parsers and pure-Python contenders too. YAMLRocks leads the field on
both load and dump. Indicative wall-clock times from `python bench/compare.py`
(release build, whole payload set, fastest first):

| Library       |  Impl  |     load |      dump |
| ------------- | :----: | -------: | --------: |
| **YAMLRocks** |  Rust  |  ~1.5 ms |   ~815 µs |
| yaml-rs       |  Rust  |  ~1.8 ms |   ~1.1 ms |
| fast-yaml     |  Rust  |  ~2.5 ms |   ~1.6 ms |
| py-yaml12     |  Rust  |  ~3.5 ms |   ~1.2 ms |
| ryaml         |  Rust  |  ~4.1 ms |   ~2.9 ms |
| PyYAML (C)    |   C    | ~16.2 ms |  ~14.9 ms |
| yamlium       | Python | ~59.6 ms |  ~10.1 ms |
| PyYAML (pure) | Python |  ~150 ms |    ~87 ms |
| oyaml         | Python |  ~151 ms |    ~87 ms |
| ruamel.yaml   | Python |  ~234 ms |   ~177 ms |
| strictyaml    | Python |  ~1.14 s | no dumper |

Speed is only half of it. The Rust rivals differ in what they get right: yaml-rs,
py-yaml12, and ryaml leave `<<` merge keys unresolved, and yaml-rs, py-yaml12, and
fast-yaml misread a bare `0777` as the integer `777`. YAMLRocks is the fastest,
and the only one verified against the entire official YAML test suite.

## How to choose

- Coming from **PyYAML** (or **oyaml**, which is just PyYAML with ordered dicts)?
  Read [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/) and
  [vs oyaml](/comparisons/vs-oyaml/).
- Coming from **ruamel.yaml** for its round-trip fidelity, but tired of pure
  Python speed? Read [YAMLRocks vs ruamel.yaml](/comparisons/vs-ruamel/).
- Comparing the newer **Rust** parsers? Read
  [vs yaml-rs](/comparisons/vs-yaml-rs/),
  [vs fast-yaml](/comparisons/vs-fast-yaml/),
  [vs ryaml](/comparisons/vs-ryaml/), and
  [vs py-yaml12](/comparisons/vs-py-yaml12/).
- Looking at pure-Python **round-trip or safety** libraries? Read
  [vs yamlium](/comparisons/vs-yamlium/) and
  [vs strictyaml](/comparisons/vs-strictyaml/).

Every page carries a benchmark, a feature matrix, side-by-side code, and an
honest account of where the other library fits.

## See also

- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/) and
  [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/).
- [Performance](/guides/performance/): the benchmark methodology.
- [Round-trip editing](/guides/round-trip/): the feature that sets YAMLRocks apart
  from PyYAML and on par with ruamel.
