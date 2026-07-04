---
title: YAMLRocks vs py-yaml12
description: A detailed comparison of YAMLRocks and py-yaml12, with benchmarks and the capabilities that set them apart.
---

[py-yaml12](https://pypi.org/project/py-yaml12/) is a Rust-backed YAML 1.2 parser
(built on the saphyr crate) with genuinely good first-class custom-tag handling.
It is a clean data parser, and its tag support is a real strength. But it is a
one-way parser: it does not edit YAML, resolve includes, or validate against a
schema, and YAMLRocks is both more complete and faster.

## Feature comparison

| Feature                       |     py-yaml12     |         YAMLRocks          |
| ----------------------------- | :---------------: | :------------------------: |
| Comment-preserving round-trip |        No         |    Yes (byte-for-byte)     |
| Native `!include` + writeback |        No         |            Yes             |
| JSON Schema validation        |        No         |    Yes (line-numbered)     |
| Source line/column            |        No         |    Yes (annotated mode)    |
| Custom tag handling           | Yes (first-class) |            Yes             |
| Verified vs the YAML suite    |    Claims full    |        Yes, in full        |
| Multi-document streams        |        Yes        |            Yes             |
| Merge keys (`<<`)             |        No         |            Yes             |
| Implementation                |   Rust (saphyr)   | Rust (own scanner/emitter) |
| Speed (parse / dump)          |     baseline      |    ~2.3x / ~1.5x faster    |

## A parser, or a toolkit

py-yaml12's tag handling is legitimately nice: a `Yaml(value, tag)` wrapper and
`handlers=` callables transform tagged nodes at parse time. YAMLRocks matches that
(a `tags=` registry and a `tag_handler`) and then adds the whole
configuration-editing layer py-yaml12 has no answer for:

- **Comment-preserving round-trip.** Edit a value and re-emit with comments,
  anchors, and formatting intact; unmodified documents come back byte-for-byte.
  py-yaml12 reads data but cannot write an edited file back.
- **Native `!include`** with file-aware write-back across split configurations.
- **JSON Schema validation** with line-numbered errors.
- **Annotated mode** with the source line and column on every node.

See [custom tags](/guides/tags/), [round-trip editing](/guides/round-trip/),
[includes](/guides/includes/), and [schema validation](/guides/schema-validation/).

## Speed

Both are Rust extensions built for correctness and speed. YAMLRocks is faster on
both directions.

![YAMLRocks vs py-yaml12 on reading and writing: YAMLRocks is faster on both loads and dumps.](/benchmarks/vs-py-yaml12.svg)

| Operation | py-yaml12 | YAMLRocks | YAMLRocks is |
| --------- | --------: | --------: | -----------: |
| Reading   |   ~3.5 ms |   ~1.5 ms | ~2.3x faster |
| Writing   |   ~1.2 ms |   ~815 µs | ~1.5x faster |

:::note[Reproduce it]
Wall-clock times from one machine and payload set. Run `python bench/compare.py`
to measure on your own hardware.
:::

## Correctness, verified rather than assumed

Both aim at YAML 1.2, and py-yaml12 states it targets the test suite. YAMLRocks
runs that suite in CI on every change (load, round-trip, and canonical result all
checked), and it also resolves merge keys and the 1.2 scalar edges py-yaml12 does
not:

```python
import yamlrocks

# Merge keys: YAMLRocks folds the base in; py-yaml12 leaves `<<` as a literal key.
yamlrocks.loads(b"base: &b\n  timeout: 30\nsvc:\n  <<: *b\n  name: api\n")
# {'base': {'timeout': 30}, 'svc': {'timeout': 30, 'name': 'api'}}

# `0777` is a string in YAML 1.2; py-yaml12 returns the number 777.
yamlrocks.loads(b"mode: 0777")
# {'mode': '0777'}
```

## Where py-yaml12 is a reasonable pick

For turning trusted YAML 1.2 data into Python objects with rich custom tags,
py-yaml12 is a solid, MIT-licensed choice, and its tag API is genuinely good. If
you never edit YAML, resolve includes, or need schema validation, it fits.

## When to choose YAMLRocks

Choose YAMLRocks when you want that same clean 1.2 parsing and tag handling plus
the editing toolkit around it, comment-preserving round-trip, includes, schema
validation, source locations, verified correctness including merge keys, and more
speed.

## See also

- [YAMLRocks vs yaml-rs](/comparisons/vs-yaml-rs/) and
  [vs ryaml](/comparisons/vs-ryaml/): the other Rust-backed parsers.
- [Custom tags](/guides/tags/) and [performance](/guides/performance/).
