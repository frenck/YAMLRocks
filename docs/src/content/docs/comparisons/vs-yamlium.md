---
title: YAMLRocks vs yamlium
description: A detailed comparison of YAMLRocks and yamlium, with benchmarks and the capabilities that set them apart.
---

[yamlium](https://pypi.org/project/yamlium/) is a pure-Python, dependency-free
YAML library that preserves comments and structure through a manipulation API,
a real feature for a zero-dependency package. Where YAMLRocks pulls ahead is
everything around that: it is one to two orders of magnitude faster, it commits
to YAML 1.2 and proves it against the test suite, and it adds native includes and
schema validation that yamlium has no equivalent for.

## Feature comparison

| Feature                       |     yamlium     |      YAMLRocks       |
| ----------------------------- | :-------------: | :------------------: |
| Comment-preserving round-trip |       Yes       | Yes (byte-for-byte)  |
| Native `!include` + writeback |       No        |         Yes          |
| JSON Schema validation        |       No        | Yes (line-numbered)  |
| Source line/column            |       No        | Yes (annotated mode) |
| Verified vs the YAML suite    |   Not stated    |     Yes, in full     |
| Declared YAML version         |  Undocumented   |  1.2 (1.1 optional)  |
| Anchors / merge keys          | Preserved / yes |   Preserved / yes    |
| Implementation                |   Pure Python   |    Rust extension    |
| Speed (parse / dump)          |    baseline     |  ~42x / ~11x faster  |

## Speed: Rust vs pure Python

yamlium is pure Python, which makes it portable but slow. YAMLRocks does the same
work in Rust, one to two orders of magnitude faster on both directions.

![YAMLRocks vs yamlium on reading and writing: YAMLRocks is an order of magnitude faster on both loads and dumps.](/benchmarks/vs-yamlium.svg)

| Operation | yamlium | YAMLRocks | YAMLRocks is |
| --------- | ------: | --------: | -----------: |
| Reading   |  ~71 ms |   ~1.7 ms |  ~42x faster |
| Writing   |  ~11 ms |   ~1.0 ms |  ~11x faster |

:::note[Reproduce it]
Wall-clock times from one machine and payload set. Run `python bench/compare.py`
to measure on your own hardware.
:::

## Same round-trip, more of everything else

yamlium's comment-and-structure-preserving editing is its selling point, and
YAMLRocks matches it: load with `OPT_ROUND_TRIP`, edit through a node API, and
re-emit with comments, anchors, and layout intact, byte-for-byte when unmodified.
On that same foundation YAMLRocks adds what yamlium does not have:

- **Native `!include`** with file-aware write-back across a split configuration.
- **JSON Schema validation** with line-numbered errors.
- **Annotated mode** with the source line and column on every node.
- **Safe-by-default** loading and correct YAML 1.2 typing.

See [round-trip editing](/guides/round-trip/), [includes](/guides/includes/),
[schema validation](/guides/schema-validation/), and
[annotated mode](/guides/annotated/).

## Correctness, verified rather than assumed

YAMLRocks commits to the YAML 1.2 core schema and checks it against the entire
official test suite in CI. yamlium does not document which YAML version it
targets, and its scalar resolution lands between the two in surprising ways:

```python
import yamlrocks

yamlrocks.loads(b"mode: 0777")   # {'mode': '0777'}  a string, per YAML 1.2
yamlrocks.loads(b"mask: 0o17")   # {'mask': 15}      the 1.2 octal
# yamlium returns {'mode': 777} and {'mask': '0o17'}.
```

When you cannot tell which spec a library follows, you cannot tell what a value
will become. YAMLRocks makes that explicit and pins it.

## Where yamlium is a reasonable pick

yamlium is a fair choice when you specifically need a pure-Python,
zero-dependency package (no binary wheel to install) and can accept its speed and
its scalar quirks. Its comment-editing API is nicely done.

## When to choose YAMLRocks

Choose YAMLRocks when you can install a wheel and want the same comment-preserving
round-trip with real speed, native includes, schema validation, source
locations, and correctness that is verified rather than undocumented.

## See also

- [Round-trip editing](/guides/round-trip/): the comment-preserving workflow.
- [YAMLRocks vs ruamel.yaml](/comparisons/vs-ruamel/): the other round-trip
  comparison.
- [Performance](/guides/performance/): the benchmark methodology.
