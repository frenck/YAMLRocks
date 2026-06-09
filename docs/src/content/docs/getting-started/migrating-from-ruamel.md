---
title: Migrating from ruamel.yaml
description: Move from ruamel.yaml to YAMLRocks while keeping round-trip fidelity, and gaining speed and native includes.
---

[ruamel.yaml](https://yaml.dev/doc/ruamel.yaml/) is the library people reach for
when they need to preserve comments and formatting through an edit. YAMLRocks offers
that same round-trip fidelity, but it is Rust-backed and dramatically faster, it
returns a byte-for-byte round trip when you do not modify the document, and it
adds native `!include` resolution and a JSON-Schema validator. This page maps
ruamel's round-trip API onto YAMLRocks's, shows the edit workflow side by side, and
is honest about what ruamel still does that YAMLRocks does not.

## Quick comparison

| ruamel.yaml                                  | YAMLRocks                                              |
| -------------------------------------------- | ------------------------------------------------------ |
| `YAML()` instance with `typ="safe"` / `"rt"` | options on `loads` / `dumps`                           |
| `yaml.load(stream)`                          | `yamlrocks.loads(data)` / `yamlrocks.load(path)`       |
| `yaml.dump(data, stream)`                    | `yamlrocks.dumps(data)` / `yamlrocks.dump(data, path)` |
| `CommentedMap` / `CommentedSeq`              | `YAMLRocksDocument` + `YAMLRocksDocumentView`          |
| round-trip preserves comments                | `OPT_ROUND_TRIP`, byte-for-byte when unmodified        |
| pure Python                                  | Rust extension                                         |

## Plain loading and dumping

ruamel configures behavior on a `YAML()` instance; YAMLRocks takes options per call.
For ordinary safe loading, the translation is direct. The ruamel block here is
illustrative (it needs ruamel installed), while the YAMLRocks block runs as written:

<!-- verify: skip -->

```python
# ruamel.yaml
from ruamel.yaml import YAML
yaml = YAML(typ="safe")
data = yaml.load("name: app\nport: 8080")
```

```python
# yamlrocks
import yamlrocks

source = """
name: app
port: 8080
"""

data = yamlrocks.loads(source)
# {'name': 'app', 'port': 8080}
```

ruamel writes to a stream you provide. YAMLRocks returns `bytes` from `dumps` (decode
for text), or writes to a path or stream through `yamlrocks.dump`:

<!-- verify: skip -->

```python
# ruamel.yaml
import sys
yaml.dump(data, sys.stdout)
```

```python
# yamlrocks
import sys
import yamlrocks
sys.stdout.write(yamlrocks.dumps({"name": "app", "port": 8080}).decode())
# name: app
# port: 8080
```

## Round-trip editing

This is the heart of a ruamel migration. ruamel's default round-trip mode returns
`CommentedMap`/`CommentedSeq` objects that you mutate in place and dump back.
YAMLRocks returns a [`YAMLRocksDocument`](/guides/round-trip/) with the same workflow: index
into it, assign, and re-emit.

<!-- verify: skip -->

```python
# ruamel.yaml
from ruamel.yaml import YAML
import sys
yaml = YAML()              # typ="rt" is the default
doc = yaml.load("# config\nname: app  # service\nport: 8080\n")
doc["port"] = 9090
yaml.dump(doc, sys.stdout)
```

The YAMLRocks equivalent loads with `OPT_ROUND_TRIP`, edits the same way, and
re-emits with `to_yaml`:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"# config\nname: app  # service\nport: 8080\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)
doc["port"] = 9090
print(doc.to_yaml().decode())
# # config
# name: app  # service
# port: 9090
```

The comment survives the edit and only the changed line is re-rendered.

:::tip[Byte-for-byte when unmodified]
If you load a document with `OPT_ROUND_TRIP` and emit it again without changing
anything, YAMLRocks gives you back the original bytes exactly: same comments,
anchors, quoting, and indentation. ruamel reserializes through its representer,
which can normalize whitespace and quoting even on an untouched document. When an
unchanged round trip needs to stay identical, YAMLRocks preserves it more faithfully.
:::

## Comments

ruamel exposes comments through its `.ca` (comment attribute) API, which is
powerful but intricate to drive directly. YAMLRocks preserves comments
automatically during a round trip: editing a value keeps the comments around it
intact, and you rarely need to touch them at all. When you do, every
`YAMLRocksNode` has a writable `comment` (the inline `# ...`) and `comment_before`
(the standalone line(s) above a key), so you can read, set, or clear them by name:

```python
import yamlrocks

doc = yamlrocks.loads(b"name: app\nport: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)
doc.node["port"].comment = "the listen port"
doc.node["name"].comment_before = "service identity"
```

A round-trip keeps comments byte-for-byte, and editing a value preserves the
spacing around it too: the comment and its gap stay put while only the value
changes. The sole exception is a comment you _set_ through the `comment` API,
which uses a single space (a freshly written comment has no original spacing):

```python
import yamlrocks

doc = yamlrocks.loads(
    b"name: app   # spacing   is   kept\nport: 8080\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)
doc["name"] = "web"
doc.to_yaml()
# b'name: web   # spacing   is   kept\nport: 8080\n'
```

## Things ruamel does that YAMLRocks maps differently

- **Indentation control** (`yaml.indent(mapping=..., sequence=..., offset=...)`):
  use `OPT_INDENT_2` (the default) or `OPT_INDENT_4` on `dumps`. Round-trip output
  preserves the source document's own indentation rather than imposing a setting.
- **Preserve quotes** (`preserve_quotes=True`): always on in YAMLRocks's round-trip
  mode. Quoting styles are kept as written, with no flag to set.
- **Merge keys** (`<<`): resolved by default in `yamlrocks.loads`, the same as
  ruamel.
- **YAML version**: both libraries default to YAML 1.2. For 1.1 documents, read
  with `OPT_YAML_1_1` or normalize once with the
  [upgrade helper](/guides/yaml-11-vs-12/).

## What ruamel still does that YAMLRocks does not

Be honest with yourself about these before migrating:

- **Foot comments.** YAMLRocks scripts inline and leading comments per node
  (`comment` / `comment_before`), and these work on mapping values, keys, and
  sequence items, but a trailing/foot comment block at the end of a collection is
  preserved through a round-trip rather than writable through the node API.
- **Building a commented document from nothing.** YAMLRocks edits comments on a
  loaded document (including keys you add to it), but there is no constructor for
  a fresh round-trip document with no parsed source, the way ruamel can assemble a
  fully commented `CommentedMap` in memory.

:::caution[When to stick with ruamel]
If your workflow depends on foot comments or on assembling a fully commented
document from nothing, ruamel's `.ca` API still does things YAMLRocks does not. For
loading, editing existing configuration in place (comments included, on values,
keys, and sequence items), and high-volume parsing or emitting, YAMLRocks is the
faster and stricter choice.
:::

## What you gain

- **Speed.** Against ruamel, YAMLRocks parses on the order of 85 to 135 times
  faster and serializes on the order of 155 to 210 times faster in release-build
  benchmarks. See [Performance](/guides/performance/).
- **Native includes.** Resolve and write back `!include` files directly, far
  faster than a Python constructor ([Includes](/guides/includes/)).
- **Schema validation.** Validate during the parse with line-numbered errors
  ([Schema validation](/guides/schema-validation/)).
- **Source locations.** `OPT_ANNOTATED` attaches `__line__` and `__column__` to
  every node ([Annotated mode](/guides/annotated/)).

## See also

- [Round-trip editing](/guides/round-trip/): the full `YAMLRocksDocument` API.
- [Includes](/guides/includes/): native `!include` resolution and write-back.
- [Migration compatibility](/getting-started/compatibility/): the compatibility
  matrix and known migration gaps.
- [vs ruamel.yaml](/comparisons/vs-ruamel/): a feature and speed comparison.
- [Quick start](/getting-started/quick-start/): the five-minute tour.
