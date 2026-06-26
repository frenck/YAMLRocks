---
title: Quick start
description: A five-minute tour of YAMLRocks, covering load, dump, multi-document streams, options, and round-trip editing.
---

This is a five-minute tour of YAMLRocks. By the end you will have parsed YAML into
Python objects, emitted Python objects back to YAML, handled a multi-document
stream, tuned the output with an option, and seen a round-trip edit that
preserves comments. Every block is self-contained and runnable, so copy them
into a REPL as you read.

If you have not installed YAMLRocks yet, see [installation](/getting-started/installation/).

## Loading YAML

`loads` parses the first document in its input and returns native Python
objects. It accepts `str`, `bytes`, `bytearray`, or any buffer such as a
`memoryview`:

```python
import yamlrocks

source = """
key: value
list:
  - 1
  - 2
"""

yamlrocks.loads(source)
# {'key': 'value', 'list': [1, 2]}
```

Scalars resolve to their natural Python types following the YAML 1.2 core
schema, so booleans, integers, floats, and nulls come back ready to use:

```python
source = """
count: 42
ratio: 3.14
enabled: true
empty: ~
"""

yamlrocks.loads(source)
# {'count': 42, 'ratio': 3.14, 'enabled': True, 'empty': None}
```

:::note[`yes` and `no` are strings]
Under YAML 1.2, `yes`, `no`, `on`, and `off` are plain strings, not booleans.
If you need the older YAML 1.1 behavior, pass `option=yamlrocks.OPT_YAML_1_1`. See
[YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/).
:::

## Emitting YAML

`dumps` is the reverse direction. It returns **`bytes`**, not a
string:

```python
yamlrocks.dumps({"name": "app", "ports": [80, 443]})
# b'name: app\nports:\n  - 80\n  - 443\n'
```

When you need text (to print it, or write it to a text-mode file), decode the
result:

```python
yamlrocks.dumps({"a": 1}).decode()
# 'a: 1\n'
```

:::tip[Why bytes?]
YAML is UTF-8, and most destinations (sockets, files opened in binary mode, HTTP
responses) want bytes. Returning bytes avoids an encode-then-decode round trip on
the hot path. Decode only at the boundary where you actually need a `str`.
:::

## Multiple documents

A single YAML stream can hold several documents separated by `---`. Use
`loads_all` to get them all back as a list, one entry per document:

```python
source = """
---
a: 1
---
b: 2
"""

yamlrocks.loads_all(source)
# [{'a': 1}, {'b': 2}]
```

## Tuning the output with options

Options are composable integer bit flags. Combine
flags with `|` and pass them as `option`. Here we sort the keys alphabetically
and indent with four spaces:

```python
yamlrocks.dumps(
    {"b": 2, "a": 1},
    option=yamlrocks.OPT_SORT_KEYS | yamlrocks.OPT_INDENT_4,
)
# b'a: 1\nb: 2\n'
```

There are flags for flow style, sorted keys, explicit document markers,
datetime handling, and more. The
[options reference](/reference/options/) lists the complete set.

## A round-trip teaser

YAMLRocks can load a document while preserving its comments, anchors, and exact
formatting. Pass `OPT_ROUND_TRIP` and you get back a
[`YAMLRocksDocument`](/guides/round-trip/) you can edit in place. Re-emitting changes only
what you touched and leaves the rest of the document intact:

```python
doc = yamlrocks.loads(
    b"# app config\nname: app  # service name\nport: 8080\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)
doc["port"] = 9090
print(doc.to_yaml().decode())
# # app config
# name: app  # service name
# port: 9090
```

The comments survive the edit, and only the port value changed. This is the
feature that makes YAMLRocks suitable for editing configuration files in place,
not just reading them.

## Where to go next

- [Loading YAML](/guides/loading/): every way to parse, and the type rules.
- [Dumping YAML](/guides/dumping/): emitting, formatting, and custom types.
- [Round-trip editing](/guides/round-trip/): preserve comments and formatting.
- [Includes](/guides/includes/): resolve and write back `!include` files.
- [Schema validation](/guides/schema-validation/): validate during the parse.
- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/) or
  [from ruamel.yaml](/getting-started/migrating-from-ruamel/).
