---
title: YAMLRocks vs ryaml
description: A detailed comparison of YAMLRocks and ryaml, with benchmarks and the capabilities that set them apart.
---

[ryaml](https://pypi.org/project/ryaml/) is a Rust-backed YAML reader with a
deliberately small, `json`-module-style API. It is a fine way to turn a trusted
file into Python data, but that is all it does, and it is alpha software. YAMLRocks
is the full configuration toolkit, comment-preserving round-trip, native
includes, schema validation, source locations, and verified-correct parsing, and
it is several times faster.

## Feature comparison

| Feature                       |        ryaml         |         YAMLRocks          |
| ----------------------------- | :------------------: | :------------------------: |
| Comment-preserving round-trip |          No          |    Yes (byte-for-byte)     |
| Native `!include` + writeback |          No          |            Yes             |
| JSON Schema validation        |          No          |    Yes (line-numbered)     |
| Source line/column            |          No          |    Yes (annotated mode)    |
| Custom tag handling           |          No          |            Yes             |
| Verified vs the YAML suite    |      Not stated      |        Yes, in full        |
| Merge keys (`<<`)             |          No          |            Yes             |
| Anchors/aliases resolved      |         Yes          |            Yes             |
| Implementation                | Rust (libyaml-safer) | Rust (own scanner/emitter) |
| Speed (parse / dump)          |       baseline       |    ~2.8x / ~3.4x faster    |
| Maturity                      |     0.5.x, alpha     |    Battle-tested corpus    |

## A parser, or a toolkit

ryaml reads YAML into Python and stops. YAMLRocks does that and the rest of what a
real configuration workflow needs:

- **Comment-preserving round-trip.** Edit a value and re-emit with comments,
  anchors, and layout intact; an unmodified document is byte-for-byte identical.
  ryaml drops comments and formatting, so it can read a file but not edit one.
- **Native `!include`** with file-aware write-back across a split configuration.
- **JSON Schema validation** with line-numbered errors.
- **Annotated mode** with the source line and column on every node.
- **Custom tag handling**, and **safe-by-default** loading that never builds
  arbitrary Python objects.

See [round-trip editing](/guides/round-trip/), [includes](/guides/includes/),
[schema validation](/guides/schema-validation/), and
[annotated mode](/guides/annotated/).

## Speed

Both are Rust extensions, so this is Rust vs Rust. YAMLRocks is well ahead on both.

![YAMLRocks vs ryaml on reading and writing: YAMLRocks is several times faster on both loads and dumps.](/benchmarks/vs-ryaml.svg)

| Operation |   ryaml | YAMLRocks | YAMLRocks is |
| --------- | ------: | --------: | -----------: |
| Reading   | ~4.7 ms |   ~1.7 ms | ~2.8x faster |
| Writing   | ~3.4 ms |   ~1.0 ms | ~3.4x faster |

:::note[Reproduce it]
Wall-clock times from one machine and payload set. Run `python bench/compare.py`
to measure on your own hardware.
:::

## Correctness, verified rather than assumed

YAMLRocks's parsing is checked against the entire official YAML test suite on
every change. ryaml publishes no such guarantee, describes itself as YAML 1.1 yet
resolves `no` and `0o17` the 1.2 way, and does not implement merge keys:

```python
import yamlrocks

# Merge keys: YAMLRocks folds the base in; ryaml leaves `<<` as a literal key.
yamlrocks.loads(b"base: &b\n  timeout: 30\nsvc:\n  <<: *b\n  name: api\n")
# {'base': {'timeout': 30}, 'svc': {'timeout': 30, 'name': 'api'}}
```

The point is not one missing feature; it is that YAMLRocks commits to a schema and
proves it, so you always know which reading you get.

## Where ryaml is a reasonable pick

ryaml is a small, MIT-licensed, `json`-style reader. For quickly loading trusted
YAML 1.2 data with no writing and no merge keys, it is easy to reach for. It is
alpha (0.5.x) with a thin surface by design.

## When to choose YAMLRocks

Choose YAMLRocks when YAML is something you edit, validate, and depend on:
round-trip with comments, includes, schema validation, and source locations, on
a verified-correct parser that is also several times faster.

## See also

- [YAMLRocks vs yaml-rs](/comparisons/vs-yaml-rs/) and
  [vs py-yaml12](/comparisons/vs-py-yaml12/): the other Rust-backed parsers.
- [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/): the safety and speed comparison.
- [Performance](/guides/performance/): the benchmark methodology.
