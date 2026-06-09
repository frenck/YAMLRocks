---
title: How YAMLRocks compares
description: YAMLRocks versus PyYAML and ruamel.yaml at a glance, with benchmarks.
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

## How to choose

- Coming from **PyYAML** and want the same safe-by-default loading, but faster,
  on YAML 1.2, with comments and includes? Read [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/).
- Coming from **ruamel.yaml** for its round-trip fidelity, but tired of pure
  Python speed? Read [YAMLRocks vs ruamel.yaml](/comparisons/vs-ruamel/).

Both pages carry full benchmark tables, feature matrices, side-by-side code,
migration notes, and an honest "when to stick with the other library" section.

## See also

- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/) and
  [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/).
- [Performance](/guides/performance/): the benchmark methodology.
- [Round-trip editing](/guides/round-trip/): the feature that sets YAMLRocks apart
  from PyYAML and on par with ruamel.
