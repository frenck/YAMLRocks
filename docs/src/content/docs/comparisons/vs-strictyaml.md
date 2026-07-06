---
title: YAMLRocks vs strictyaml
description: How YAMLRocks answers strictyaml's safety concerns without crippling YAML or the throughput.
---

[strictyaml](https://pypi.org/project/strictyaml/) takes a strong position: YAML's
implicit typing and its more exotic features are footguns, so it parses only a
restricted subset and makes you supply a schema to get anything other than
strings. The concern is real. The remedy is heavy: strictyaml is pure Python
built on ruamel.yaml, it removes large parts of YAML, it is load-only, and it is
the slowest option in the field by a wide margin. YAMLRocks answers the same
concern by defaulting to YAML 1.2 and offering real JSON Schema validation,
without throwing away YAML or the speed.

## The concern, and how YAMLRocks addresses it

strictyaml's headline argument is the "Norway problem": in YAML 1.1, `no` becomes
boolean `False`, so a country list containing `NO` (Norway) silently corrupts.
YAMLRocks does not have this problem, because it defaults to the YAML 1.2 core
schema, where `no` is the string `"no"`.

```python
import yamlrocks

yamlrocks.loads(b"countries: [NO, SE, DK]")
# {'countries': ['NO', 'SE', 'DK']}   strings, not [False, 'SE', 'DK']
```

Where strictyaml removes implicit typing entirely and hands you strings, YAMLRocks
gives you correct 1.2 types by default and lets you _enforce_ a shape with JSON
Schema validation that reports line-numbered errors. You get type safety without
giving up numbers, booleans, or a dumper.

## Feature comparison

| Feature                   |      strictyaml      |       YAMLRocks       |
| ------------------------- | :------------------: | :-------------------: |
| Implicit `no` -> `False`  |   Avoided (subset)   | Avoided (1.2 default) |
| Typed values without cast |  No (strings only)   | Yes (1.2 core schema) |
| Schema validation         | Yes (own schema DSL) |   Yes (JSON Schema)   |
| Anchors/aliases           |  Rejected by design  |    Yes (preserved)    |
| Flow style `{}` / `[]`    |  Rejected by design  |          Yes          |
| Custom tags               |  Rejected by design  |          Yes          |
| Dumping arbitrary data    |  No general dumper   |          Yes          |
| Comment preservation      |   Yes (via ruamel)   |          Yes          |
| Implementation            | Pure Python (ruamel) |    Rust extension     |
| Speed (parse)             |       baseline       |     ~810x faster      |

## Speed

![YAMLRocks vs strictyaml on reading: YAMLRocks is roughly three orders of magnitude faster to load.](/benchmarks/vs-strictyaml.svg)

strictyaml is built for a specific safety story, not for throughput, and it shows:
it is roughly three orders of magnitude slower to load than YAMLRocks, and it has
no general-purpose dumper to benchmark at all.

| Operation | strictyaml | YAMLRocks | YAMLRocks is |
| --------- | ---------: | --------: | -----------: |
| load      |    ~1.38 s |   ~1.7 ms | ~810x faster |
| dump      |  no dumper |   ~1.0 ms |            - |

:::note[Reproduce it]
Wall-clock times from one machine and payload set. Run `python bench/compare.py`
to measure on your own hardware.
:::

## What strictyaml gives up

To enforce its subset, strictyaml rejects flow style, anchors and aliases, and
tags outright, and it returns every leaf as a string until a schema casts it. That
is a deliberate, sometimes reasonable trade for a locked-down config format you
fully control. But it means strictyaml cannot read a large fraction of real-world
YAML, and it cannot write YAML back out as a drop-in dumper.

YAMLRocks reads all of standard YAML 1.2, is safe by default (it never constructs
arbitrary Python objects from tags), types scalars correctly, validates against a
schema when you want enforcement, and round-trips with comments intact.

## When to choose YAMLRocks

Choose strictyaml only if you specifically want a maximally restricted YAML dialect
and are willing to pay for it in speed and in features. For everything else, safe
loading, correct 1.2 types, schema-enforced validation, reading real-world YAML,
round-trip editing, and speed, YAMLRocks covers the same safety goals without the
sacrifices.

## See also

- [Schema validation](/guides/schema-validation/): enforce a shape with
  line-numbered errors.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): why the Norway problem does not
  happen by default.
- [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/) and
  [performance](/guides/performance/).
