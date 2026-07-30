---
title: Building a config editor
description: Use round-trip mode to edit YAML programmatically without losing comments or formatting.
---

YAMLRocks's round-trip mode makes it practical to build tools that edit
user-authored YAML, such as a settings UI, a migration script, or a linter that
auto-fixes, without destroying comments or reflowing the file. Load with
`OPT_ROUND_TRIP`, edit through a `YAMLRocksDocument` (or a nested `YAMLRocksDocumentView`), and
re-emit with `to_yaml()`. Unchanged nodes come back byte-for-byte; only what you
touch is re-rendered.

## The core loop

Load, read, edit, emit. Everything below is runnable: the YAML is built as bytes
in-process so there is no file to set up.

```python
import yamlrocks

source = b"""# Service configuration
server:
  host: localhost  # bind address
  port: 8080
features:
  - logging
  - metrics
"""

doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

# Read values like a dict / list.
doc["server"]["host"]  # 'localhost'

# Write values; only what you touch changes.
doc["server"]["port"] = 9090

# Emit the bytes, comments and layout intact.
doc.to_yaml()
# b'# Service configuration\nserver:\n  host: localhost  # bind address\n  port: 9090\n...'
```

An unmodified document re-emits identically, so running the loop with no edits is
a no-op:

```python
unchanged = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
assert unchanged.to_yaml() == source
```

:::note[Comment spacing]
Round-trip mode preserves the spacing before an inline `#`, including across an
edit: `host: localhost   # x` keeps its three spaces even after you change the
value. The only exception is a comment you _set_ through the `comment` API, which
uses a single space (a freshly written comment has no original spacing to keep).
:::

## Editing through a YAMLRocksDocumentView

Indexing into a nested mapping or sequence returns a `YAMLRocksDocumentView`, a live proxy
onto that subtree. Edits through a view write back to the parent document:

```python
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

server = doc["server"]  # a YAMLRocksDocumentView onto the `server` mapping
type(server).__name__  # 'YAMLRocksDocumentView'

server["host"] = "0.0.0.0"  # writes through to doc
doc.to_yaml().splitlines()[2]  # b'  host: 0.0.0.0  # bind address'
```

A view exposes the same navigation and inspection methods as the document:
`keys()`, `get()`, `to_dict()`, `walk()`, `range()`, `to_yaml()`, and
`unwrap()`.

## Mapping an edit back to a source span

The killer feature for an editor is connecting a node to its exact location in
the source text. `range()` returns `(start_line, start_col, end_line, end_col)`,
all 1-based, so you can highlight a node, place a cursor, or show a diff anchored
to the right lines:

```python
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

doc["server"].range()  # (3, 3, 4, 13) - the server block spans lines 3-4
doc["features"].range()  # (6, 3, 7, 12) - the features list spans lines 6-7
```

Combine `range()` with `walk()` to drive a "jump to definition" or inline-error
feature. `walk()` yields every scalar leaf as `(path_tuple, value)`, and you can
re-index by the path to get that node's view and span:

```python
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

for path, value in doc.walk():
    print(path, "=", value)
# ('server', 'host') = localhost
# ('server', 'port') = 8080
# ('features', 0) = logging
# ('features', 1) = metrics


# Locate the span of the node at a given path.
def span_at(document, path):
    node = document
    for key in path:
        node = node[key]
    return node.range()


span_at(doc, ("server",))  # (3, 3, 4, 13)
```

## Bulk edits with walk()

Because `walk()` exposes every leaf with its path, a find-and-replace or audit
pass is straightforward. Re-index by the path's parent to assign the new value:

```python
template = b"""# Provision me
database:
  host: CHANGE_ME
  name: app
cache:
  host: CHANGE_ME
"""

doc = yamlrocks.loads(template, option=yamlrocks.OPT_ROUND_TRIP)

for path, value in doc.walk():
    if value == "CHANGE_ME":
        node = doc
        for key in path[:-1]:
            node = node[key]
        node[path[-1]] = "db.internal"

doc.to_yaml()
# both CHANGE_ME placeholders are now db.internal; comments are preserved
```

## Reading without committing

`to_dict()` (on the document or any view) returns a plain snapshot for read-only
logic, leaving round-trip state untouched. `unwrap()` does the same for a single
view:

```python
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)

plain = doc.to_dict()  # a regular dict/list tree
subtree = doc["server"].to_dict()  # snapshot of one subtree
doc["server"].unwrap()  # equivalent for a view
```

## Emitting and saving

When you do touch real files, load with `load(..., option=OPT_ROUND_TRIP)` and
use `save()`; the document remembers where it came from. Otherwise stay in
memory with `to_yaml()`:

```python
doc = yamlrocks.loads(source, option=yamlrocks.OPT_ROUND_TRIP)
doc["server"]["port"] = 9090

emitted = yamlrocks.dumps(doc)  # bytes, identical to doc.to_yaml()
emitted == doc.to_yaml()  # True
```

The example below touches the filesystem, so it carries a skip marker for the
docs verifier, but it is the pattern you would use in a real editor:

<!-- verify: skip -->

```python
import yamlrocks

doc = yamlrocks.load("config.yaml", option=yamlrocks.OPT_ROUND_TRIP)
doc["server"]["port"] = 9090

doc.save()  # overwrite config.yaml in place
doc.save("config.new.yaml")  # or write a copy, leaving the original
```

## Tips

- Replacing a scalar with a different type is fine: `doc["count"] = 5` swaps the
  value and marks that node modified.
- Assigning a new key appends it: `doc["new"] = "value"`.
- Comments attached to a value you replace stay where they make sense; the rest
  of the document is reproduced verbatim.
- `dumps(doc)` and `doc.to_yaml()` produce the same bytes, so either works when
  you need the serialized form.

## See also

- [Round-trip editing](/guides/round-trip/): the full round-trip model.
- [Using YAMLRocks with Home Assistant](/recipes/home-assistant/): editing across
  `!include` files and saving only what changed.
- [Annotated mode](/guides/annotated/): source locations on the fast path,
  without round-trip.
- [API reference](/reference/api/): `YAMLRocksDocument` and `YAMLRocksDocumentView` in full.
