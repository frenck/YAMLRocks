---
title: YAMLRocks vs fast-yaml
description: A detailed comparison of YAMLRocks and fast-yaml, with benchmarks and the capabilities that set them apart.
---

[fast-yaml](https://github.com/bug-ops/fast-yaml) (the `fastyaml-rs` package) is a
Rust-backed YAML 1.2 parser built on the saphyr crate, with a PyYAML-style
`safe_load`/`safe_dump` API and built-in linting. It is a capable, fast reader,
and of the newer Rust parsers it gets the most right. But it is still a one-way
parser: it does not edit YAML, resolve includes, or validate against a schema,
and YAMLRocks is both more complete and faster.

## Feature comparison

| Feature                       |   fast-yaml   |         YAMLRocks          |
| ----------------------------- | :-----------: | :------------------------: |
| Comment-preserving round-trip |      No       |    Yes (byte-for-byte)     |
| Native `!include` + writeback |      No       |            Yes             |
| JSON Schema validation        |      No       |    Yes (line-numbered)     |
| Source line/column            |      No       |    Yes (annotated mode)    |
| Custom tag handling           |  Unverified   |            Yes             |
| Verified vs the YAML suite    |  Not stated   |        Yes, in full        |
| Merge keys (`<<`)             |      Yes      |            Yes             |
| Anchors/aliases resolved      |      Yes      |            Yes             |
| Multi-document streams        |      Yes      |            Yes             |
| Implementation                | Rust (saphyr) | Rust (own scanner/emitter) |
| Speed (parse / dump)          |   baseline    |    ~1.6x / ~2.0x faster    |

## A parser, or a toolkit

fast-yaml reads YAML into Python and lints it. YAMLRocks does that and the rest of
what a real configuration workflow needs:

- **Comment-preserving round-trip.** Load with `OPT_ROUND_TRIP`, change a value,
  and re-emit with comments, anchors, and formatting intact; an unmodified
  document comes back byte-for-byte. fast-yaml explicitly does not preserve
  comments, so it can read a file but not edit one.
- **Native `!include`** with file-aware write-back across a split configuration.
- **JSON Schema validation** with line-numbered errors.
- **Annotated mode** with the source line and column on every node.
- **Custom tag handling**, and **safe-by-default** loading.

See [round-trip editing](/guides/round-trip/), [includes](/guides/includes/),
[schema validation](/guides/schema-validation/), and
[annotated mode](/guides/annotated/).

## Speed

Both are Rust extensions built on strict YAML 1.2. YAMLRocks is faster on both
directions.

![YAMLRocks vs fast-yaml on reading and writing: YAMLRocks is faster on both loads and dumps.](/benchmarks/vs-fast-yaml.svg)

| Operation | fast-yaml | YAMLRocks | YAMLRocks is |
| --------- | --------: | --------: | -----------: |
| Reading   |   ~2.5 ms |   ~1.5 ms | ~1.6x faster |
| Writing   |   ~1.6 ms |   ~815 µs | ~2.0x faster |

:::note[Reproduce it]
Wall-clock times from one machine and payload set. Run `python bench/compare.py`
to measure both on your own hardware.
:::

## Correctness, verified rather than assumed

fast-yaml is the most correct of the newer Rust parsers, it resolves merge keys,
where yaml-rs, ryaml, and py-yaml12 leave `<<` as a literal key. That makes the
remaining difference the interesting one: YAMLRocks checks its parsing against the
entire official YAML test suite on every change (load, round-trip, and canonical
result), and fast-yaml still misreads a leading-zero integer:

```python
import yamlrocks

# `0777` is a string in YAML 1.2 (the octal form is `0o777`), not the number 777.
yamlrocks.loads(b"mode: 0777")
# {'mode': '0777'}
# fast-yaml returns {'mode': 777}.
```

The point is not one scalar; it is that "verified against the whole suite" is a
guarantee fast-yaml does not make, and the edges are where that shows.

## Where fast-yaml is a reasonable pick

fast-yaml is a fast, dual-licensed (MIT/Apache-2.0) 1.2 parser with a familiar
`safe_load`/`safe_dump` surface and a handy built-in linter. If you want a quick,
drop-in-flavored reader for trusted YAML and never write edited files back, it is
a reasonable choice, and more faithful than its Rust peers.

## When to choose YAMLRocks

Choose YAMLRocks when YAML is something you edit, include, validate, and depend
on: comment-preserving round-trip, native includes, schema validation, and source
locations, on a parser that is verified correct against the full YAML test suite
and is faster too.

## See also

- [YAMLRocks vs yaml-rs](/comparisons/vs-yaml-rs/),
  [vs ryaml](/comparisons/vs-ryaml/), and
  [vs py-yaml12](/comparisons/vs-py-yaml12/): the other Rust-backed parsers.
- [Performance](/guides/performance/): the benchmark methodology.
