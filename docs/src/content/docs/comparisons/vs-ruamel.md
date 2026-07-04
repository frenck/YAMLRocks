---
title: YAMLRocks vs ruamel.yaml
description: A detailed comparison of YAMLRocks and ruamel.yaml, with benchmarks and side-by-side code.
---

[ruamel.yaml](https://yaml.dev/doc/ruamel.yaml/) is the reference for
comment-preserving, round-trip YAML in Python. It is excellent and
feature-rich, but it is pure Python and therefore slow. YAMLRocks offers the same
round-trip fidelity, backed by Rust, at one to two orders of magnitude more
throughput, and adds native includes and schema validation. YAMLRocks now scripts
comments per node too (inline and leading); ruamel still wins on the most
exhaustive comment surgery, covered honestly below.

## Feature comparison

| Feature                        |       ruamel.yaml       |            YAMLRocks            |
| ------------------------------ | :---------------------: | :-----------------------------: |
| YAML 1.2                       |           Yes           |               Yes               |
| Comment preservation           |           Yes           |               Yes               |
| Round-trip (unmodified)        |          Close          |          Byte-for-byte          |
| Anchors/aliases preserved      |           Yes           |               Yes               |
| Merge keys (`<<`)              |           Yes           |               Yes               |
| Implementation                 |       Pure Python       |         Rust extension          |
| Speed (parse)                  |        baseline         |         ~85-135x faster         |
| Speed (dump)                   |        baseline         |        ~155-210x faster         |
| Native `!include` + write-back |           No            |               Yes               |
| Source line/column             |         partial         |      Yes (annotated mode)       |
| JSON Schema validation         |           No            |               Yes               |
| Save only changed files        |           No            |               Yes               |
| Per-node comment editing API   | Yes (`.ca`, exhaustive) | Yes (inline, leading, trailing) |
| Output type                    |   `str` (to a stream)   |             `bytes`             |

## Speed: Rust vs pure Python

![YAMLRocks vs ruamel.yaml on reading and writing: YAMLRocks is one to two orders of magnitude faster on both.](/benchmarks/vs-ruamel.svg)

ruamel.yaml is implemented entirely in Python, which makes it flexible but slow.
YAMLRocks does the same work in Rust and materializes results across the PyO3
boundary, so the same parse or dump is one to two orders of magnitude faster.

Indicative figures from `python bench/bench.py` (release build), showing how many
times **faster YAMLRocks is** than ruamel.yaml in `safe` mode.

**Parsing (`loads`)**

| Payload           | YAMLRocks is |
| ----------------- | -----------: |
| small             |  ~90x faster |
| medium            | ~108x faster |
| large (500 items) | ~133x faster |
| deep              |  ~85x faster |

**Serializing (`dumps`)**

| Payload | YAMLRocks is |
| ------- | -----------: |
| small   | ~194x faster |
| medium  | ~185x faster |
| large   | ~191x faster |
| deep    | ~156x faster |

ruamel's round-trip mode is heavier still. YAMLRocks's round-trip path stays far
faster while preserving comments, anchors, and formatting, with byte-for-byte
output for unmodified documents.

:::note[These are ratios]
The numbers above are speed ratios, not wall-clock times. They depend on payload
shape and hardware. Reproduce them with `python bench/bench.py`.
:::

## Byte-for-byte round-trip

Both libraries preserve comments, anchors, and scalar styles. YAMLRocks goes one
step further: an unmodified round-trip reproduces the source bytes exactly, and
only the nodes you change are re-rendered. This is enforced across the entire
official YAML test suite.

```python
import yamlrocks

source = b"# service config\nname: app  # the app name\nport: 8080\n"

# Unmodified: bytes come back exactly as they went in.
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
assert doc.to_yaml() == source

# Change one value: only that line is re-rendered, comments intact.
doc["port"] = 9090
doc.to_yaml()
# b'# service config\nname: app  # the app name\nport: 9090\n'
```

The editing surface is a `YAMLRocksDocument` with `dict`/`list`-style access plus
`walk()`, `to_dict()`, and `range()` helpers, rather than ruamel's
`CommentedMap`/`CommentedSeq` types:

```python
import yamlrocks

doc = yamlrocks.loads(b"server:\n  host: localhost\n  port: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)

doc.keys()                       # ['server']
doc["server"]["port"] = 9090     # nested edit writes through
doc.to_dict()                    # {'server': {'host': 'localhost', 'port': 9090}}
doc.walk()                       # [(('server', 'host'), 'localhost'), (('server', 'port'), 9090)]
```

See [round-trip editing](/guides/round-trip/) and the
[config editor recipe](/recipes/config-editor/).

## Native includes and file-aware saving

ruamel.yaml has no `!include`. A split configuration must be stitched together by
hand, and there is no concept of writing an edited value back to the specific
file it came from.

YAMLRocks resolves `!include` and the `!include_dir_*` family natively. In
round-trip mode each node remembers its source file, so `save()` writes back
only the files that actually changed. See [includes](/guides/includes/) and the
[Home Assistant recipe](/recipes/home-assistant/) for a worked example of
editing one automation and saving only `automations.yaml`.

## Scripting comments

YAMLRocks does more than _preserve_ comments through a round-trip: every
`YAMLRocksNode` exposes a writable `comment` (the inline `# ...` after a value),
`comment_before` (the standalone line(s) above a key), and `comment_after` (the
trailing block at the foot of a mapping or sequence). A tool can read, add, edit,
remove, or move a comment (read it from one node, set it on another) and re-emit.
This works on mapping values, keys, and sequence items alike, and on keys you add
to a loaded document.

```python
import yamlrocks

doc = yamlrocks.loads(b"name: app\nport: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)

doc.node["port"].comment = "the listen port"          # inline, no '#'
doc.node["name"].comment_before = "service identity"  # standalone line above
doc.node.comment_after = "end of config"              # trailing block (foot)
doc.to_yaml()
# b'# service identity\nname: app\nport: 8080 # the listen port\n# end of config\n'
```

## Where ruamel.yaml is still richer

**Building a fully commented document from nothing** needs ruamel: YAMLRocks edits
comments on a loaded document rather than assembling one with no parsed source.
ruamel's `.ca` API also reaches a few unusual placements that `comment`,
`comment_before`, and `comment_after` do not.

On emitter configuration the two are closer than they once were: YAMLRocks's
`dumps` takes an explicit `width`, indentation options (`OPT_INDENT_2` / `_4`,
`OPT_INDENTLESS_SEQUENCES`), a `default=` hook for serializing custom objects, and
a `tags=` registry, so most representer-style needs are met. ruamel remains more
configurable for deeply custom class round-tripping and is battle-tested for
intricate document transformations.

## When to stick with ruamel.yaml

Reach for ruamel.yaml when you need to build a fully commented document from
scratch, lean on its class-based representer/constructor extension points for
deeply custom types, or perform intricate document surgery where its maturity
matters more than throughput. For high-throughput loading and dumping, native
includes, schema validation, scripting inline, leading, and trailing comments, or
byte-for-byte round-trip with file-aware saving, YAMLRocks is the faster,
batteries-included choice.

## See also

- [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/).
- [YAMLRocks vs PyYAML](/comparisons/vs-pyyaml/): the safety and speed comparison.
- [Round-trip editing](/guides/round-trip/) and [includes](/guides/includes/).
- [Performance](/guides/performance/): the benchmark methodology.
