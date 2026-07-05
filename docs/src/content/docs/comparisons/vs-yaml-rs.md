---
title: YAMLRocks vs yaml-rs
description: A detailed comparison of YAMLRocks and yaml-rs, with benchmarks and the capabilities that set them apart.
---

[yaml-rs](https://pypi.org/project/yaml-rs/) is a young Rust-backed YAML 1.2
parser that markets itself as the fastest in the ecosystem. It is genuinely fast,
and the closest competitor to YAMLRocks on raw throughput. But it is only a
parser: text in, Python values out. YAMLRocks is the whole toolkit real
configuration work needs, comment-preserving round-trip, native includes, schema
validation, source locations, rigorously verified correctness, and it is faster
on top.

## Feature comparison

| Feature                       |           yaml-rs           |         YAMLRocks          |
| ----------------------------- | :-------------------------: | :------------------------: |
| YAML 1.2                      |             Yes             |            Yes             |
| Comment-preserving round-trip |             No              |    Yes (byte-for-byte)     |
| Native `!include` + writeback |             No              |            Yes             |
| JSON Schema validation        |             No              |    Yes (line-numbered)     |
| Source line/column            |             No              |    Yes (annotated mode)    |
| Custom tag handling           |             No              |            Yes             |
| Verified vs the YAML suite    |         Not stated          |        Yes, in full        |
| Anchors/aliases resolved      |             Yes             |            Yes             |
| Merge keys (`<<`)             |             No              |            Yes             |
| Timestamp resolution          | On by default, incl. quoted | Opt-in, plain scalars only |
| Implementation                |     Rust (saphyr fork)      | Rust (own scanner/emitter) |
| Speed (parse / dump)          |          baseline           |    ~1.2x / ~1.4x faster    |
| Maturity                      |    0.1.x, public domain     |    Battle-tested corpus    |

## A parser, or a toolkit

This is the difference that matters. yaml-rs reads YAML into Python values and
stops there. YAMLRocks does that and everything a real configuration workflow
needs around it:

- **Comment-preserving round-trip.** Load with `OPT_ROUND_TRIP`, change a value,
  and re-emit with every comment, anchor, and formatting choice intact; an
  unmodified document comes back byte-for-byte. yaml-rs discards comments and
  layout, so it cannot edit a file, only read it.
- **Native `!include`.** YAMLRocks resolves `!include` and the `!include_dir_*`
  family, and in round-trip mode writes edits back to the specific file each node
  came from. Split configurations (Home Assistant, Kubernetes overlays) work out
  of the box.
- **JSON Schema validation** with line-numbered errors, so a bad config is
  rejected with a message that points at the line, not a stack trace.
- **Annotated mode** attaches the source line and column to every node, which is
  what lets a tool underline the exact place a value came from.
- **Custom tag handling** through a `tags=` registry or a `tag_handler`.
- **Safe by default.** Loading never constructs arbitrary Python objects.

See [round-trip editing](/guides/round-trip/), [includes](/guides/includes/),
[schema validation](/guides/schema-validation/), and
[annotated mode](/guides/annotated/).

## Speed

Both are Rust extensions, so this is Rust vs Rust, not Rust vs Python. yaml-rs is
the nearest rival on the field, and YAMLRocks is still ahead on both directions.

![YAMLRocks vs yaml-rs on reading and writing: YAMLRocks is faster on both loads and dumps.](/benchmarks/vs-yaml-rs.svg)

| Operation | yaml-rs | YAMLRocks | YAMLRocks is |
| --------- | ------: | --------: | -----------: |
| Reading   | ~1.8 ms |   ~1.5 ms | ~1.2x faster |
| Writing   | ~1.1 ms |   ~815 µs | ~1.4x faster |

The gap is smaller than against a pure-Python library, as you would expect from
two native implementations. Speed is real, but it is not the reason to choose
between them, the capabilities above and the correctness below are.

:::note[Reproduce it]
Wall-clock times from one machine and payload set, not a fixed law. Run
`python bench/compare.py` to measure both on your own hardware.
:::

## Correctness, verified rather than assumed

YAMLRocks's parsing is checked against the entire official YAML test suite on
every change: a case must load, round-trip, and match its canonical result, or
CI fails. That is the substantive correctness claim, and it is why the edges hold
up. yaml-rs makes no such published guarantee, and it shows in cases like these:

```python
import yamlrocks

# Merge keys: YAMLRocks folds the base in; yaml-rs leaves `<<` as a literal key.
yamlrocks.loads(b"base: &b\n  timeout: 30\nsvc:\n  <<: *b\n  name: api\n")
# {'base': {'timeout': 30}, 'svc': {'timeout': 30, 'name': 'api'}}

# Leading-zero integers: `0777` is a string in YAML 1.2, not the number 777.
yamlrocks.loads(b"mode: 0777")
# {'mode': '0777'}
```

Timestamps are a sharper example. yaml-rs resolves a date/datetime scalar to a
Python object by default, and it does so even for a **quoted** scalar. In YAML a
quoted scalar is explicitly a string, so quoting is exactly how you say "keep
this as text", and yaml-rs ignores that. YAMLRocks makes timestamp resolution
opt-in ([`OPT_TIMESTAMPS`](/reference/options/#timestamps)) and only ever applies
it to plain scalars, matching PyYAML and the spec:

```python
import yamlrocks

# A quoted scalar is a string, even with timestamp resolution enabled.
yamlrocks.loads(b'when: "2024-01-15"', option=yamlrocks.OPT_TIMESTAMPS)
# {'when': '2024-01-15'}
# yaml-rs returns {'when': datetime.date(2024, 1, 15)}, quoting ignored.
```

These are not the reason to switch on their own; they are evidence that "verified
against the whole suite" is a real difference, not a slogan.

## Where yaml-rs is a reasonable pick

yaml-rs is a small, fast, public-domain (Unlicense) parser. If all you need is to
turn a trusted YAML 1.2 file into Python data as quickly as possible, and never
write YAML back out, it does that one job well. It is early software (0.1.x) with
a deliberately thin surface.

## When to choose YAMLRocks

Choose YAMLRocks when YAML is something you edit and depend on, not just read:
comment-preserving round-trip, native includes, schema validation, and source
locations, on a parser that is verified correct against the full YAML test suite
and is faster too. That is almost every real configuration use.

## See also

- [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/): the safety and speed comparison.
- [YAMLRocks vs ryaml](/comparisons/vs-ryaml/) and
  [vs py-yaml12](/comparisons/vs-py-yaml12/): the other Rust-backed parsers.
- [Performance](/guides/performance/): the benchmark methodology.
