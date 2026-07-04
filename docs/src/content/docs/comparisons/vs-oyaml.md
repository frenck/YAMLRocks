---
title: YAMLRocks vs oyaml
description: Why oyaml is just PyYAML with ordered dicts, and why that no longer buys you anything.
---

[oyaml](https://pypi.org/project/oyaml/) is not a separate YAML implementation. It
is a small shim that imports PyYAML, registers ordered-dict representers and
constructors, and re-exports PyYAML's entire API. You use it as `import oyaml as
yaml`. Its one job was preserving mapping key order, which mattered before Python
3.7. Since then, dicts are ordered by the language and modern PyYAML already keeps
insertion order, so oyaml adds essentially nothing.

## What oyaml actually is

Everything oyaml does, PyYAML does. Its speed, its YAML 1.1 semantics, its comment
handling (none), its safety story, all of it is PyYAML's, because it _is_ PyYAML
underneath. So the real comparison is [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/),
and it applies here unchanged.

| Feature                    | oyaml (= PyYAML) |      YAMLRocks       |
| -------------------------- | :--------------: | :------------------: |
| Distinct implementation    |   No (PyYAML)    | Yes (Rust extension) |
| Preserves key order        | Yes (its point)  |         Yes          |
| YAML 1.2 by default        |        No        |         Yes          |
| Comment preservation       |        No        |         Yes          |
| Round-trip (byte-for-byte) |        No        |         Yes          |
| Native `!include`          |        No        |         Yes          |
| JSON Schema validation     |        No        |         Yes          |
| Speed (parse)              |     baseline     |     ~100x faster     |
| Speed (dump)               |     baseline     |     ~105x faster     |

![YAMLRocks vs oyaml on reading and writing: YAMLRocks is about two orders of magnitude faster, because oyaml is PyYAML underneath.](/benchmarks/vs-oyaml.svg)

## Order preservation is free now

YAMLRocks preserves mapping key order as a matter of course, the same as any modern
dict. There is nothing to add a shim for.

```python
import yamlrocks

yamlrocks.loads(b"z: 1\na: 2\nm: 3\n")
# {'z': 1, 'a': 2, 'm': 3}   source order, always
```

## When to choose YAMLRocks

There is no scenario where oyaml is preferable to plain PyYAML on a supported
Python, and none where it beats YAMLRocks. If you are on oyaml today, you are on
PyYAML with a patch that no longer does anything; move straight to YAMLRocks for
YAML 1.2, comment-preserving round-trip, native includes, schema validation, and
one to two orders of magnitude more speed.

## See also

- [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/): the comparison that actually
  applies to oyaml.
- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/).
- [Performance](/guides/performance/).
