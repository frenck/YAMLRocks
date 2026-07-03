---
title: Round-trip editing
description: Edit YAML in place while preserving comments, anchors, and formatting byte for byte.
---

Most YAML libraries treat a document as a one-way trip: you parse it into plain
Python objects, and any comments, quoting choices, anchors, and blank lines are
gone forever. Re-emitting that data produces a file that no longer looks like the
one a human wrote.

`OPT_ROUND_TRIP` keeps the trip open in both directions. Instead of plain objects
it returns a `YAMLRocksDocument`: a live, editable view over the parsed tree that still
remembers every byte of the original. An unmodified document re-emits exactly
what it parsed, and when you change a value, only that value moves. Every comment,
quote, and blank line around it stays put.

```python
import yamlrocks

source = b"""\
# Application config
name: my-app   # the service name
version: 1.0.0
"""

doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

# Nothing touched yet: the output is byte-for-byte identical to the input.
assert doc.to_yaml() == source

doc["version"] = "2.0.0"

print(doc.to_yaml().decode())
# # Application config
# name: my-app   # the service name
# version: 2.0.0
```

The `# Application config` header and the `# the service name` inline comment
survive the edit, and `version` carries its new value. This is the core promise
of round-trip mode: edits are surgical.

:::tip[When to reach for round-trip mode]
Use it whenever a human also edits the file: configuration editors, migration
tools, linters that auto-fix, or anything that writes a user's YAML back to disk.
For pure data interchange where formatting does not matter, plain
[`loads`](/guides/loading/) is smaller and faster.
:::

## Byte-for-byte for unmodified documents

A freshly parsed document that you have not modified re-emits the bytes it came
from. That includes anchors and aliases, quoting styles, and block scalars:

```python
import yamlrocks

original = b"base: &b\n  x: 1\nuse: *b\n"
doc = yamlrocks.loads(original, option=yamlrocks.OPT_ROUND_TRIP)

assert doc.to_yaml() == original
```

This makes round-trip mode safe to drop into a save pipeline: loading and saving a
file the user did not change leaves it untouched, so version control stays quiet
and diffs stay meaningful.

:::note[Byte fidelity is a UTF-8 guarantee]
The byte-for-byte promise is for UTF-8 input, which is what real configuration
files use. YAMLRocks still reads UTF-16 and UTF-32 (the YAML spec requires it,
detected by a byte order mark or the leading-byte pattern), but it decodes to
text internally and re-emits as UTF-8. So loading a UTF-16 file gives you the
right data, but re-emitting it produces the same text encoded as UTF-8, not the
original UTF-16 bytes. Convert a file to UTF-8 first if you need byte-for-byte
round-trip editing of it.
:::

## Reading values

A `YAMLRocksDocument` reads like the mapping it wraps. Scalar access returns plain Python
values; mappings and sequences return live views (more on those below):

```python
import yamlrocks

doc = yamlrocks.loads(b"name: my-app\nport: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)

doc["name"]               # 'my-app'
doc.get("missing", 0)     # 0  (default, like dict.get)
"port" in doc             # True
len(doc)                  # 2
doc.keys()                # ['name', 'port']
```

| Operation    | Method                       | Returns                              |
| ------------ | ---------------------------- | ------------------------------------ |
| Index access | `doc[key]`                   | a value or a `YAMLRocksDocumentView` |
| Safe access  | `doc.get(key, default=None)` | a value or the default               |
| Membership   | `key in doc`                 | `bool`                               |
| Length       | `len(doc)`                   | number of top-level keys             |
| Keys         | `doc.keys()`                 | a `list` of keys                     |

To get a plain snapshot with no formatting attached, call `to_dict()`. It returns
an ordinary `dict` (recursively), which is handy for comparisons, JSON
serialization, or handing data to code that does not care about layout:

```python
doc.to_dict()             # {'name': 'my-app', 'port': 8080}
```

## Deep edits write through

Indexing into a nested mapping or sequence returns a `YAMLRocksDocumentView`: a live proxy
onto that node rather than a detached copy. Assigning through a view writes back
into the document, so deep edits stick:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"server:\n  host: localhost\n  ports:\n    - 80\n    - 443\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

doc["server"]["host"] = "example.com"
doc["server"]["ports"][1] = 8443

print(doc.to_yaml().decode())
# server:
#   host: example.com
#   ports:
#     - 80
#     - 8443
```

A `YAMLRocksDocumentView` offers the same navigation surface as the `YAMLRocksDocument` itself
(indexing, `get`, `in`, `len`, `keys`), plus a few methods for inspecting the
slice it points at:

```python
view = doc["server"]

view.to_dict()            # {'host': 'example.com', 'ports': [80, 8443]}
view.unwrap()             # same plain dict/list snapshot
view.keys()               # ['host', 'ports']
```

:::note[Views are windows, not copies]
A `YAMLRocksDocumentView` stays attached to its parent `YAMLRocksDocument`. Read through it to
inspect a sub-tree, and assign through it to edit in place. When you want a
detached, formatting-free copy instead, call `unwrap()` or `to_dict()`.
:::

## Deleting keys

`del` removes a mapping key or a sequence item. The deleted entry takes its own
comments with it, while everything around it — the document header, sibling keys,
and their inline comments — stays untouched:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"# app config\nname: my-app   # the service\ndebug: true\nport: 8080\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

del doc["debug"]

print(doc.to_yaml().decode())
# # app config
# name: my-app   # the service
# port: 8080
```

Deletes write through a `YAMLRocksDocumentView` too, so a nested key can be removed
in place:

```python
doc = yamlrocks.loads(
    b"server:\n  host: localhost  # keep me\n  debug: true\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

del doc["server"]["debug"]

print(doc.to_yaml().decode())
# server:
#   host: localhost  # keep me
```

Deleting an absent mapping key raises `KeyError`; an out-of-range sequence index
raises `IndexError`:

<!-- verify: raises KeyError -->

```python
del doc["server"]["missing"]
```

## Walking the tree

`walk()` flattens the whole document into a list of `(path, value)` pairs, where
each path is a tuple of keys and indices leading to a leaf value. It is the
quickest way to scan every scalar, for example to validate values or collect the
locations you want to change:

```python
import yamlrocks

doc = yamlrocks.loads(b"name: app\nport: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)

doc.walk()
# [(('name',), 'app'), (('port',), 8080)]
```

Sequence elements appear with integer indices in their path, so nested structures
flatten predictably:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"server:\n  host: localhost\n  ports:\n    - 80\n    - 443\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

doc["server"].walk()
# [(('host',), 'localhost'), (('ports', 0), 80), (('ports', 1), 443)]
```

## Source locations with `range()`

Every `YAMLRocksDocument` and `YAMLRocksDocumentView` can report the span of source text it covers.
`range()` returns a four-tuple `(start_line, start_col, end_line, end_col)`, all
1-based, which is exactly what you need to underline a node in an editor or point
a user at a problem:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"# header\nname: app  # inline\nport: 8080\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

doc.range()               # (2, 1, 3, 11)
```

The body of this document starts on line 2 (after the header comment) at column 1,
and ends on line 3 at column 11. Views report the span of their own node, so you
can locate any nested value:

```python
doc["name"]               # 'app'
```

For read-only access to line and column on plain objects (without the editing
machinery), see [annotated mode](/guides/annotated/).

## The node cursor: comments, styles, and locations

Item access is deliberately value-shaped: `doc["server"]["port"]` gives you the
plain integer `8080`, not a wrapper. That is what you want most of the time, but
a bare `8080` has nowhere to carry its comment, its line number, or the fact
that it was written in single quotes.

`doc.node` solves that. It is a `YAMLRocksNode` cursor, and unlike item access, indexing a
`YAMLRocksNode` _always_ returns another `YAMLRocksNode` (scalars included), so every piece of
metadata stays reachable down to a single leaf:

```python
import yamlrocks

source = b"""\
# HTTP front end
server:
  host: localhost
  port: 8080  # the http port
  tags: [web, edge]
"""

doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

port = doc.node["server"]["port"]   # a YAMLRocksNode, even though the value is a scalar
port.value                # 8080
port.comment              # 'the http port'   (inline comment, no leading '#')
port.line                 # 4                 (1-based)
port.column               # 9
port.style                # 'plain'
doc.node["server"]["tags"].style   # 'flow'
```

### Reading metadata

Every `YAMLRocksNode` exposes the same attributes, whatever it points at:

| Attribute         | Meaning                                                              |
| ----------------- | -------------------------------------------------------------------- |
| `value`           | the resolved Python value (scalar, `dict`, or `list`)                |
| `comment`         | the inline comment trailing the value, or `None`                     |
| `comment_before`  | the standalone comment line(s) above the node, or `None`             |
| `line` / `column` | 1-based source position                                              |
| `file`            | the source file the node came from, or `None` without includes       |
| `style`           | `plain`, `single`, `double`, `literal`, `folded`, `block`, or `flow` |
| `anchor`          | the node's anchor name (`&name`), or `None`                          |
| `tag`             | the node's explicit tag (`!!str`, `!custom`), or `None`              |

Comment text is always bare (no leading `#`, no surrounding whitespace), so you
read and write the words, not the punctuation. A multi-line `comment_before` is
returned as one string with `\n` between the lines.

### Writing metadata

`value`, `comment`, and `comment_before` are writable, and the change re-emits in
the right place:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"# HTTP front end\nserver:\n  port: 8080  # the http port\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

port = doc.node["server"]["port"]
port.value = 8443
port.comment = "now uses TLS"
doc.node["server"].comment_before = "HTTP front end (TLS)"

print(doc.to_yaml().decode())
# # HTTP front end (TLS)
# server:
#   port: 8443 # now uses TLS
```

Setting `value` keeps the node's comments, anchor, and tag, so editing a value
never silently drops the comment beside it. Set `comment` or `comment_before` to
`None` to remove a comment entirely.

:::note[Where "before" comments live]
For a mapping pair, the comment _above_ the line belongs to the key, and
`comment_before` reads and writes it there. The one subtlety is the very first
key of a mapping: a comment above it is also the document's (or sub-tree's)
leading comment, so `doc.node["server"].comment_before` and the parent's own
leading comment are the same text. Setting it replaces that one comment.
:::

A `YAMLRocksDocumentView` exposes the same cursor through its own `.node`, so
`doc["server"].node["port"]` and `doc.node["server"]["port"]` reach the same
node.

## Anchors and aliases

Round-trip mode keeps `&anchor` definitions and `*alias` references intact, and
the node cursor lets you find, follow, and detach them.

```python
import yamlrocks

source = b"""\
defaults: &d
  retries: 3
  timeout: 30
prod:
  <<: *d
  timeout: 60
staging: *d
"""

doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
```

### Finding anchors

`YAMLRocksDocument.anchors` maps every anchor name to the `YAMLRocksNode` that defines it, and a
definition's `aliases` lists the references that point back at it: the basis for
"find usages" or a safe rename:

```python
doc.anchors                       # {'d': YAMLRocksNode(mapping)}
defaults = doc.anchors["d"]
defaults.value                    # {'retries': 3, 'timeout': 30}
len(defaults.aliases)             # 2  (the `<<: *d` merge and `staging: *d`)
```

On an alias node, `is_alias` is `True` and `target` is the defining `YAMLRocksNode`, so
you can hop from a use to its definition (and read _its_ comment or line):

```python
staging = doc.node["staging"]
staging.is_alias                  # True
staging.target.anchor             # 'd'
```

### Following aliases

Indexing an alias follows it transparently to the anchor it points at, so you can
read straight through a `*alias`:

```python
doc.node["staging"]["retries"].value   # 3
```

Because the alias and its anchor are the _same_ node, an edit made through a
followed alias changes the shared definition, and therefore every use of it:

```python
doc.node["staging"]["retries"].value = 99
doc.node["defaults"]["retries"].value  # 99  (the anchor itself changed)
```

That is usually what you want for shared config. When it is not, detach first.

### Detaching an alias

`detach()` replaces a `*alias` with an independent deep copy of the anchor it
referenced. The copy keeps the original's styles and comments but carries no
anchor of its own, and any aliases nested inside it are expanded, so editing it
no longer touches the original:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"defaults: &d\n  retries: 3\n  timeout: 30\nstaging: *d\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

doc.node["staging"].detach()
doc.node["staging"]["retries"].value = 7

print(doc.to_yaml().decode())
# defaults: &d
#   retries: 3
#   timeout: 30
# staging:
#   retries: 7
#   timeout: 30
```

`defaults` keeps its `&d` anchor and its original `retries: 3`; only the
now-independent `staging` block changed. Calling `detach()` on a node that is not
an alias raises `TypeError`.

### Creating anchors and aliases

`anchor` is writable, and `make_alias(name)` turns a node into a reference to an
existing anchor. Mark the shared node, then point others at it:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"defaults:\n  retries: 3\nprod:\n  retries: 5\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

doc.node["defaults"].anchor = "d"   # mark &d
doc.node["prod"].make_alias("d")    # prod: *d

print(doc.to_yaml().decode())
# defaults: &d
#   retries: 3
# prod: *d
```

Creation is validated so it can never emit a broken document:

- An anchor name must be **unique**. Assigning a name already used by another
  node raises `ValueError` (the document would otherwise have two `&name`).
- `make_alias` requires the anchor to **already exist and appear earlier** in the
  document; a missing or forward reference raises `ValueError`, because YAML
  resolves an alias only to a prior anchor.

Set `anchor` to `None` to remove an anchor. To break an existing alias into an
independent copy instead, use [`detach()`](#detaching-an-alias).

## Emitting

There are two equivalent ways to render a `YAMLRocksDocument` back to YAML bytes. Both
return `bytes`, like every other emitter in YAMLRocks:

```python
import yamlrocks

doc = yamlrocks.loads(b"name: app\n", option=yamlrocks.OPT_ROUND_TRIP)

doc.to_yaml()             # b'name: app\n'
yamlrocks.dumps(doc)         # b'name: app\n'  (accepts a YAMLRocksDocument too)
```

Use `doc.to_yaml()` when you have a `YAMLRocksDocument` in hand; reach for
`yamlrocks.dumps(doc)` when a `YAMLRocksDocument` flows through code that already calls
`dumps` on whatever it is given.

## Saving to disk

A `YAMLRocksDocument` loaded from a file with [`load`](/guides/loading/) remembers where it
came from. Its `origin` attribute holds that path, and `save()` writes the
document back, returning the list of files it wrote:

<!-- verify: skip -->

```python
import yamlrocks

doc = yamlrocks.load("/config/app.yaml", option=yamlrocks.OPT_ROUND_TRIP)
doc["port"] = 9090

doc.origin                # '/config/app.yaml'
doc.save()                # ['/config/app.yaml']  - written in place
```

A document parsed with `loads` (from bytes, not a file) has `origin == None`. Give
it a destination with `set_origin`, or pass a path straight to `save`. The
following example is fully self-contained: it creates a real file in a temporary
directory, edits it, and saves it back.

```python
import os
import tempfile
import yamlrocks

work = tempfile.mkdtemp()
path = os.path.join(work, "app.yaml")
with open(path, "wb") as handle:
    handle.write(b"# service\nname: app\nport: 8080\n")

doc = yamlrocks.load(path, option=yamlrocks.OPT_ROUND_TRIP)
assert doc.origin == path

doc["port"] = 9090
written = doc.save()
assert written == [path]

# Only `port` changed; the comment and the rest are intact.
assert open(path, "rb").read() == b"# service\nname: app\nport: 9090\n"

# Redirect a document to a new path, then save a copy there.
other = os.path.join(work, "copy.yaml")
doc.set_origin(other)
doc.save()
assert os.path.exists(other)
```

You can also pass an explicit path to `save(path)` for a one-off write without
changing `origin`.

:::tip[Includes save too]
When a round-trip document was assembled from `!include` directives, `save()`
writes back only the included files you actually changed and leaves the rest
alone. See [includes](/guides/includes/) for the full story.
:::

## What is preserved (and one thing that normalizes)

Round-trip mode keeps the parts of a document that carry human intent:

- Head, inline, and trailing comments
- Inline-comment alignment (`x: 1      # note`) and `key:` / `-` to value
  padding (`example:    true`)
- Single- and double-quoting, and literal (`|`) / folded (`>`) block scalars
- Flow (`[a, b]`, `{a: 1}`) versus block collection layout, and a block
  sequence's indentation (`-` at the key's column versus indented a step)
- An explicit `---` document-start marker
- Blank lines and indentation
- Anchors (`&name`), aliases (`*name`), and merge keys (`<<`)
- Custom tags and `!include` directives (see [includes](/guides/includes/))

Editing a value keeps all of that alignment intact: only the value itself
changes, and the spacing and comment on its line come along untouched.

```python
import yamlrocks

source = b"name: app   # three spaces before the hash\nport: 8080\n"
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

# Untouched: byte-for-byte identical.
assert doc.to_yaml() == source

# Editing the value keeps the comment and its three-space gap; only "app" moves.
doc["name"] = "web"
doc.to_yaml()
# b'name: web   # three spaces before the hash\nport: 8080\n'
```

There is one honest exception, and it applies only to a comment you write
yourself: a comment **set** through the [`comment`](#yamlrocksnode) API uses a
single space before the `#`, because a freshly written comment has no original
spacing to keep.

## Upgrading 1.1 spellings in place

Because round-trip mode is byte-preserving, a document written with YAML 1.1
spellings (`yes`/`no`, `0777`) is dumped back out exactly as written: the
legacy forms survive. To accept that input but emit canonical 1.2 while keeping
comments and layout, add `OPT_UPGRADE_1_1`:

```python
import yamlrocks

source = b"# device settings\nenabled: yes # was on\nmask: 0777\n"

doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_UPGRADE_1_1)
doc.to_yaml()
# b'%YAML 1.2\n---\n# device settings\nenabled: true # was on\nmask: 511\n'
```

The re-emitted document is stamped with a `%YAML 1.2` directive so it declares
itself upgraded and is read back as 1.2, not re-coerced. This is the gentle way
to ease a configuration off the old schema without a reformat. See
[easing into YAML 1.2](/guides/yaml-11-vs-12/#easing-into-yaml-12).

## See also

- [Loading YAML](/guides/loading/): plain parsing without the editing layer.
- [Includes](/guides/includes/): edit and save values that live in `!include`d files.
- [Annotated mode](/guides/annotated/): read-only line and column for every node.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): the `OPT_UPGRADE_1_1` bridge mode.
- [Dumping YAML](/guides/dumping/): emit plain Python objects.
- [API reference](/reference/api/) and [options](/reference/options/).
