---
title: API reference
description: Every public function, type, and exception in YAMLRocks, with signatures.
---

This is the complete public surface of `yamlrocks`: a small set of top-level
functions, integer [`OPT_*`
flags](/reference/options/) combined with `|`, and a `default` hook for types
that are not serializable out of the box. Reading parses YAML into native Python
objects; dumping returns `bytes`.

Every signature below is the real one. Arguments before the `/` are
positional-only and arguments after the `*` are keyword-only.

## Reading

### `loads`

```python
def loads(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    root_path: str | os.PathLike[str] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> Any: ...
```

Parse the first YAML document from `data` and return native Python objects.
Empty input (or input that is only comments) returns `None`.

- `option`: a bitwise-OR of [`OPT_*` flags](/reference/options/).
- `include_dir`: base directory for `!include` resolution (with `OPT_INCLUDES`).
- `root_path`: the on-disk path the in-memory `data` stands in for. Its nodes then
  report that path, and `!include` directives in it resolve relative to that
  file's own directory. Without it, includes resolve relative to `include_dir`.
- `schema`: a JSON Schema `dict` to [validate](/guides/schema-validation/)
  against, or the string `"auto"` to validate against the document's in-file
  `# yaml-language-server: $schema=...` reference (requires `schema_resolver`).
- `schema_resolver`: a callable `ref -> dict | None` used only with
  `schema="auto"`: it receives the declared reference and returns a schema
  `dict` (or `None` to skip validation). YAMLRocks never fetches the reference
  itself. See [in-file schema references](/guides/schema-validation/#in-file-schema-references).
- `tags`: a `{tag: func}` mapping (or a [`YAMLRocksTags`](#yamlrockstags) registry) resolving
  [custom tags](/guides/tags/) by name; `func` receives the inner value.
- `tag_handler`: a catch-all callback `(tag, value) -> value` for any tag not in
  `tags`.
- `on_missing_secret`: a callback `(name, file, line) -> None` invoked once per
  undefined `!secret` (with `OPT_SECRETS`) instead of raising; the node resolves
  to `None` and the load continues, so every miss is collected in one pass. See
  [handling a missing secret](/guides/tags/#handling-a-missing-secret).
- `on_missing_env_var`: the `!env_var` counterpart (with `OPT_ENV_VAR`), called
  per bare undefined variable with no default. See
  [handling a missing environment variable](/guides/tags/#handling-a-missing-environment-variable).

With `OPT_ROUND_TRIP` it returns a [`YAMLRocksDocument`](#yamlrocksdocument) instead of a plain
value; with `OPT_ANNOTATED` it returns
[annotated subclasses](#yamlrocksannotateddict-yamlrocksannotatedlist-yamlrocksannotatedstr).

```python
import yamlrocks

source = """
name: app
port: 8080
"""

yamlrocks.loads(source)
# {'name': 'app', 'port': 8080}
```

See the [loading guide](/guides/loading/) for type resolution, block scalars,
anchors, and merge keys.

### `loads_all`

```python
def loads_all(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
```

Parse every `---`-separated document in the stream into a list, one entry per
document.

```python
import yamlrocks

source = """
---
a: 1
---
b: 2
"""

yamlrocks.loads_all(source)
# [{'a': 1}, {'b': 2}]
```

### `load`

```python
def load(
    source: str | os.PathLike[str] | Any,
    /,
    *,
    option: int | None = None,
    include_dir: str | os.PathLike[str] | None = None,
    schema: Any | None = None,
    schema_resolver: Callable[[str], Any | None] | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
    on_missing_secret: Callable[[str, str | None, int], None] | None = None,
    on_missing_env_var: Callable[[str, str | None, int], None] | None = None,
) -> Any: ...
```

The file-oriented counterpart to [`loads`](#loads). `source` is a path (`str` or
any `os.PathLike`) or an already-open file object. The keyword arguments behave
identically (there is no `root_path`: the source file's path supplies it).

With `OPT_INCLUDES` and no `include_dir`, includes resolve relative to the
source file's own directory, which is almost always what you want. A round-trip
[`YAMLRocksDocument`](#yamlrocksdocument) returned by `load` remembers where it came from in
`origin`, so `doc.save()` can write it back.

```python
import yamlrocks

with open("config.yaml", "w") as f:
    f.write("name: app\nport: 8080\n")

yamlrocks.load("config.yaml")
# {'name': 'app', 'port': 8080}
```

### `load_all`

```python
def load_all(
    source: str | os.PathLike[str] | Any,
    /,
    *,
    option: int | None = None,
    tag_handler: Callable[[str, Any], Any] | None = None,
    tags: dict[str, Callable[[Any], Any]] | None = None,
) -> list[Any]: ...
```

The file-oriented counterpart to [`loads_all`](#loads_all). Reads a multi-document
file or stream and returns every document as a list.

### `schema_ref`

```python
def schema_ref(
    data: bytes | bytearray | memoryview | str,
    /,
) -> str | None: ...
```

Return the JSON Schema reference declared by an in-file
`# yaml-language-server: $schema=...` directive, or `None` if the document does
not declare one. Only the leading comment block is inspected; the function never
parses the document body and never performs any I/O, so it is always cheap and
safe to call.

Use it to discover a document's declared schema without committing to fetching
or validating against it. To validate, pass `schema="auto"` with a
`schema_resolver` to [`loads`](#loads). See
[in-file schema references](/guides/schema-validation/#in-file-schema-references).

```python
import yamlrocks

doc = """
# yaml-language-server: $schema=https://example.com/c.json
port: 8080
"""

yamlrocks.schema_ref(doc)
# 'https://example.com/c.json'
```

### `yaml_version`

```python
def yaml_version(
    data: bytes | bytearray | memoryview | str,
    /,
) -> str | None: ...
```

Return the version declared by the document's `%YAML` directive (for example
`"1.1"` or `"1.2"`), or `None` if it declares none. Only the stream header is
inspected; the function never parses the document body and performs no I/O.

A `%YAML` directive is authoritative: `loads` selects the schema from it,
overriding `OPT_YAML_1_1`/`OPT_UPGRADE_1_1`. Use this detector to tell whether a
file has already been stamped (for example by [`upgrade`](#upgrade)) as 1.2. See
[staying upgraded](/guides/yaml-11-vs-12/#staying-upgraded).

```python
import yamlrocks

yamlrocks.yaml_version(b"%YAML 1.2\n---\nx: 1\n")  # '1.2'
yamlrocks.yaml_version(b"x: 1\n")  # None
```

## Writing

### `dumps`

```python
def dumps(
    obj: Any,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
    represent: Callable[
        [Any], YAMLRocksScalar | YAMLRocksSequence | YAMLRocksMapping | None
    ]
    | None = None,
) -> bytes: ...
```

Serialize `obj` to YAML and return `bytes`. `dumps` returns bytes,
not `str`; decode with `.decode()` if you need text.

- `default`: a callable invoked for a value YAMLRocks cannot serialize on its own.
  It receives the value and returns something serializable, or raises to signal
  that the value is unsupported.
- `option`: a bitwise-OR of [`OPT_*` flags](/reference/options/). How `None` is
  rendered is one of these: the default empty node (`key:`), `OPT_NULL_AS_KEYWORD`
  (`key: null`), or `OPT_NULL_AS_TILDE` (`key: ~`). See
  [Null style](/reference/options/#null-style).
- `serializers`: a `{type: func}` registry for emitting custom `!tag value`
  output; `func` receives a value of that exact type and returns a
  [`YAMLRocksTag`](#yamlrockstag) (or `(tag, value)` tuple). The write-side mirror
  of the load-side `tags`. See [emitting custom tags](/guides/tags/#emitting-custom-tags).
- `width`: a best-effort maximum line length. `None` (the default) leaves lines
  unwrapped; an integer folds long scalars and flow collections at safe points
  (never changing the decoded value). See
  [line width](/guides/dumping/#line-width-width). Not supported together with
  `represent` (raises).
- `represent`: a callback invoked for every value, returning a
  [`YAMLRocksScalar`](#yamlrocksscalar) / [`YAMLRocksSequence`](#yamlrockssequence) /
  [`YAMLRocksMapping`](#yamlrocksmapping) node descriptor to control how it emits,
  or `None` to defer to the built-in rendering (byte-for-byte a plain `dumps`,
  save for a few documented corners). The write-side equivalent of a PyYAML
  representer. See [full control with `represent`](/guides/dumping/#full-control-with-represent).

`dumps` also accepts a [`YAMLRocksDocument`](#yamlrocksdocument) to re-emit a round-tripped
document. A document re-emits from its own preserved layout, so the emit-shaping
arguments (`option`, `width`, `serializers`, `default`, `represent`) are ignored
for it.

```python
import yamlrocks

yamlrocks.dumps({"key": "value", "list": [1, 2]})
# b'key: value\nlist:\n  - 1\n  - 2\n'
```

See the [dumping guide](/guides/dumping/) for type mappings, the `default` hook,
and emit styles.

### `dump`

```python
def dump(
    obj: Any,
    target: str | os.PathLike[str] | Any = None,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
    serializers: dict[type, Callable[[Any], Any]] | None = None,
    width: int | None = None,
    represent: Callable[
        [Any], YAMLRocksScalar | YAMLRocksSequence | YAMLRocksMapping | None
    ]
    | None = None,
) -> None: ...
```

The file-oriented counterpart to [`dumps`](#dumps). Writes the serialized YAML to
`target`, a path or an open file object, and returns `None`.

Calling `dump(doc)` with no `target` on a round-trip [`YAMLRocksDocument`](#yamlrocksdocument) that
was loaded from disk writes only the changed files back to their original
locations.

<!-- verify: skip -->

```python
import yamlrocks

yamlrocks.dump({"name": "app", "port": 8080}, "config.yaml")
```

### `to_json`

```python
def to_json(
    obj: Any,
    /,
    *,
    default: Callable[[Any], Any] | None = None,
    option: int | None = None,
) -> bytes: ...
```

Serialize `obj` to JSON and return `bytes`. Output is compact by default;
`OPT_INDENT_2`/`OPT_INDENT_4` pretty-print and `OPT_SORT_KEYS` orders object
keys. Accepts a plain object, a [`YAMLRocksDocument`](#yamlrocksdocument), or a
[`YAMLRocksDocumentView`](#yamlrocksdocumentview) (so a sub-tree can be exported directly).

JSON is the lossy subset of YAML: tags are dropped, non-finite floats become
`null`, non-string scalar keys are stringified (`1` becomes `"1"`), and a
collection used as a key raises `YAMLRocksEncodeError`. JSON _import_ needs no special
function. JSON is valid YAML 1.2, so [`loads`](#loads) already parses it. See
the [JSON guide](/guides/json/).

```python
import yamlrocks

source = """
name: app
ports: [80, 443]
"""

yamlrocks.to_json(yamlrocks.loads(source))
# b'{"name":"app","ports":[80,443]}'
```

## Async

These coroutines run the matching sync call in a worker thread so an asyncio
application never blocks its loop, replacing hand-written `loop.run_in_executor`
plumbing. They cover the operations where the offload actually pays off: the
native scan/parse releases the GIL on byte input (so loading runs truly in
parallel, including on free-threaded CPython), and the file variants move disk
I/O off the loop too.

There is intentionally no `async_dumps` or `async_to_json`: serializing a Python
object holds the GIL for the object traversal and only frees it for the final
byte emit, so a thread offload buys little. Use `asyncio.to_thread(dumps, obj)`
directly in the rare case it matters.

| Coroutine                      | Wraps                     | Notes                                 |
| ------------------------------ | ------------------------- | ------------------------------------- |
| `async_loads(data, ...)`       | [`loads`](#loads)         | Same keyword arguments.               |
| `async_load(source, ...)`      | [`load`](#load)           | Offloads the file read and the parse. |
| `async_load_all(source, ...)`  | [`load_all`](#load_all)   | Multi-document file.                  |
| `async_loads_all(data, ...)`   | [`loads_all`](#loads_all) | Multi-document in-memory stream.      |
| `async_dump(obj, target, ...)` | [`dump`](#dump)           | Offloads the serialize and the write. |

```python
import yamlrocks


async def read_config(path):
    return await yamlrocks.async_load(path, option=yamlrocks.OPT_INCLUDES)
```

The full GIL release applies to the plain fast path. With a `tag_handler`,
`schema`, annotated mode, or round-trip, work interleaves Python calls and the
loop is freed only partially. There is no async tag resolution: a `tags` or
`tag_handler` function still runs synchronously inside the worker thread.

See [async loading](/guides/loading/#async-loading-off-the-event-loop) and
[async dumping](/guides/dumping/#async-dumping) for runnable examples and the
full rationale for there being no async serializer.

## Upgrading

### `upgrade`

```python
def upgrade(
    data: bytes | bytearray | memoryview | str,
    /,
    *,
    preserve_comments: bool = True,
) -> bytes: ...
```

Rewrite a YAML 1.1 document to canonical YAML 1.2 and return `bytes`. This
normalizes scalars whose meaning changed between the versions: `yes`/`no`/`on`/`off`
become `true`/`false`, `0777` becomes `511`, sexagesimal numbers are expanded,
and so on.

With `preserve_comments=True` (the default) it keeps comments, anchors, and
layout, changing only the scalars that differ. With `preserve_comments=False` it
reformats the document from scratch.

The result is stamped with a `%YAML 1.2` version directive so it declares itself
as 1.2 and is read back as such (not re-coerced under `OPT_UPGRADE_1_1`).
Re-upgrading an already-stamped document is idempotent. See
[the upgrade path](/guides/yaml-11-vs-12/#the-upgrade-path).

```python
import yamlrocks

source = """
enabled: yes
mode: on
"""

yamlrocks.upgrade(source)
# b'%YAML 1.2\n---\nenabled: true\nmode: true\n'
```

See [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for the full list of changes.

## Includes

These helpers operate on a round-trip [`YAMLRocksDocument`](#yamlrocksdocument) loaded with
`OPT_INCLUDES`, so that edits to values living in included files can be written
back. See the [includes guide](/guides/includes/).

### `dump_includes`

```python
def dump_includes(
    doc: YAMLRocksDocument,
    /,
    *,
    include_dir: str | os.PathLike[str] | None = None,
) -> None: ...
```

Write modified included files back to disk, each to the source path it was
loaded from (tracked on the document when it was read), not to a new location.
Only files whose content actually changed are written; the root document is left
untouched. `include_dir` is optional and ignored; it is accepted only for
call-site symmetry with `load`, and passing a different directory does **not**
rebase the writes.

### `dump_includes_map`

```python
def dump_includes_map(doc: YAMLRocksDocument, /) -> dict[str, bytes]: ...
```

Return a mapping of `{source-file path: new contents}` without writing anything.
Useful for previewing a change, staging it in a buffer, or routing the bytes
somewhere other than the filesystem.

## Types

### `YAMLRocksDocument`

Returned by `loads`/`load` with `OPT_ROUND_TRIP`. A `YAMLRocksDocument` preserves
comments, anchors, scalar styles, and formatting; editing a value re-emits the
document with the rest of it preserved. Pass it back to `dumps` or `dump`
to serialize.

| Member                    | Description                                                                                             |
| ------------------------- | ------------------------------------------------------------------------------------------------------- |
| `origin: str \| None`     | The path the document was loaded from, or `None`.                                                       |
| `__len__()`               | Number of top-level entries.                                                                            |
| `__getitem__(key)`        | Read a value. A nested mapping or sequence returns a [`YAMLRocksDocumentView`](#yamlrocksdocumentview). |
| `__setitem__(key, value)` | Set a value; the change is reflected by `to_yaml`.                                                      |
| `__delitem__(key)`        | Delete a mapping key or sequence item; raises `KeyError`/`IndexError` if absent.                        |
| `__contains__(key)`       | Membership test.                                                                                        |
| `get(key, default=None)`  | Read with a fallback.                                                                                   |
| `keys()`                  | List of top-level keys.                                                                                 |
| `set_origin(path)`        | Set the path used by `save()`.                                                                          |
| `save(path=None)`         | Write to `path` (or `origin`); returns the list of files written.                                       |
| `range()`                 | `(start_line, start_col, end_line, end_col)`, all 1-based.                                              |
| `to_yaml()`               | Re-emit the document as `bytes`.                                                                        |
| `to_dict()`               | A plain `dict`/`list` snapshot, without annotations.                                                    |
| `walk()`                  | List of `(path_tuple, value)` pairs for every leaf.                                                     |
| `locate(path)`            | The [`YAMLRocksNode`](#yamlrocksnode) at a data path (scalar leaves included), or `None` if unresolved. |
| `node`                    | The root [`YAMLRocksNode`](#yamlrocksnode) cursor for metadata access (comments, location, style).      |
| `anchors`                 | `dict[str, YAMLRocksNode]` mapping each anchor name to its defining [`YAMLRocksNode`](#yamlrocksnode).  |

```python
import yamlrocks

doc = yamlrocks.loads(
    b"# c\nname: app  # inline\nport: 8080\n", option=yamlrocks.OPT_ROUND_TRIP
)
doc["port"] = 9090
doc.to_yaml()
# b'# c\nname: app  # inline\nport: 9090\n'
```

```python
doc.keys()  # ['name', 'port']
doc.to_dict()  # {'name': 'app', 'port': 9090}
doc.walk()  # [(('name',), 'app'), (('port',), 9090)]
doc.range()  # (2, 1, 3, 11)  the body spans line 2, col 1, to the end of 'port: 8080'
```

`locate(path)` maps a data path, the kind a validator emits to say where a failure
is (`["servers", 1, "port"]`), onto the source. Unlike item access, it returns a
positioned [`YAMLRocksNode`](#yamlrocksnode) even for a scalar leaf, so a validation
error can point at the exact `file:line:column`. It returns `None` when the path
does not resolve, so a caller can retry with shorter prefixes to fall back to the
nearest enclosing container. In a multi-document stream it resolves against the
first document (a leading `int` is a key/index, not a document selector).

```python
doc = yamlrocks.loads(b"port: not-a-number\n", option=yamlrocks.OPT_ROUND_TRIP)
node = doc.locate(["port"])
node.line, node.column  # (1, 7), the value, 1-indexed
node.range()  # (1, 7, 1, 19)
doc.locate(["missing"])  # None
```

:::note[Round-trip fidelity]
An unmodified document re-emits byte-for-byte. When you edit a value, the rest of
the document is preserved, comments, blank lines, inline-comment alignment,
scalar styles, and block-versus-flow layout included.
:::

### `YAMLRocksDocumentView`

A live proxy onto a nested mapping or sequence inside a `YAMLRocksDocument`. Indexing a
`YAMLRocksDocument` into a nested node returns a `YAMLRocksDocumentView`, and edits write through
to the underlying document.

It supports the same navigation as `YAMLRocksDocument` (`__len__`, `__getitem__`,
`__setitem__`, `__delitem__`, `__contains__`, `get`, `keys`, `range`, `to_yaml`,
`to_dict`,
`walk`) plus `unwrap()`, which returns the node as plain Python objects, and
`node`, the [`YAMLRocksNode`](#yamlrocksnode) cursor at this view's position.

```python
import yamlrocks

doc = yamlrocks.loads(
    b"server:\n  host: localhost\n  port: 80\n", option=yamlrocks.OPT_ROUND_TRIP
)
server = doc["server"]  # a YAMLRocksDocumentView
server["port"] = 443  # writes through to doc
doc.to_yaml()
# b'server:\n  host: localhost\n  port: 443\n'
```

### `YAMLRocksNode`

A metadata-bearing handle onto a single node, obtained from `YAMLRocksDocument.node` (the
root cursor) or `YAMLRocksDocumentView.node`. Where item access resolves scalars to plain
values, indexing a `YAMLRocksNode` always returns another `YAMLRocksNode` (scalars included), so
comments, source location, style, anchor, and tag stay reachable for any node in
the tree. See the [round-trip guide](/guides/round-trip/#the-node-cursor-comments-styles-and-locations).

| Member             | Description                                                                                                                 |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| `value`            | The resolved Python value; assignable (keeps comments, anchor, tag).                                                        |
| `comment`          | Inline comment trailing the value, bare of `#`; assignable, `None` to clear.                                                |
| `comment_before`   | Standalone comment line(s) above the node; assignable, `None` to clear.                                                     |
| `line` / `column`  | 1-based source position.                                                                                                    |
| `range()`          | `(start_line, start_col, end_line, end_col)`, all 1-based, for underlining the span.                                        |
| `file`             | Source file path, or `None` without includes.                                                                               |
| `style`            | `plain`, `single`, `double`, `literal`, `folded`, `block`, `flow`, `alias`, or `null`.                                      |
| `anchor`           | Anchor name (`&name`), or `None`; assignable. Names must be unique; `None` clears.                                          |
| `tag`              | Explicit tag (`!!str`, `!custom`), or `None`.                                                                               |
| `is_alias`         | `True` if this node is an alias (`*name`).                                                                                  |
| `target`           | For an alias, the defining `YAMLRocksNode`; otherwise `None`.                                                               |
| `aliases`          | For a definition, the alias `YAMLRocksNode`s referencing it (else `[]`).                                                    |
| `make_alias(name)` | Replace this node with an alias of an existing, earlier-defined anchor. Raises if the anchor is missing or not yet defined. |
| `detach()`         | Replace an alias with an independent deep copy; returns the new `YAMLRocksNode`. Raises if not an alias.                    |
| `__getitem__(key)` | Index a child by key or index, returning a `YAMLRocksNode`. Following an alias is transparent.                              |

```python
import yamlrocks

doc = yamlrocks.loads(
    b"server:\n  port: 8080  # the http port\n", option=yamlrocks.OPT_ROUND_TRIP
)
port = doc.node["server"]["port"]
port.value  # 8080
port.comment  # 'the http port'
port.line  # 2
port.style  # 'plain'

port.value = 8443
port.comment = "now uses TLS"
doc.to_yaml()
# b'server:\n  port: 8443 # now uses TLS\n'
```

### `YAMLRocksAnnotatedDict` / `YAMLRocksAnnotatedList` / `YAMLRocksAnnotatedStr`

Returned by `loads`/`load` with `OPT_ANNOTATED`. These subclass the matching
builtin (`dict`, `list`, `str`) and behave exactly like it, with extra attributes
that record where the value came from:

| Attribute                        | Description                                                                                                                                                                            |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `__line__: int`                  | 1-based line where the node starts.                                                                                                                                                    |
| `__column__: int`                | 1-based column where the node starts.                                                                                                                                                  |
| `__file__: str \| None`          | Source file path, or `None` for in-memory input.                                                                                                                                       |
| `__end_line__: int`              | 1-based line just past the node's last character (like PyYAML's `end_mark`).                                                                                                           |
| `__end_column__: int`            | 1-based column just past the node's last character.                                                                                                                                    |
| `__offset__: int`                | 0-based byte offset of the node's first source character.                                                                                                                              |
| `__end_offset__: int`            | 0-based byte offset just past the node's last source character (exact, even for quoted scalars). `source[__offset__:__end_offset__]` slices the verbatim source token.                 |
| `__style__: str`                 | (`YAMLRocksAnnotatedStr` only) source style: `plain`, `single`, `double`, `literal` (`\|`), `folded` (`>`).                                                                            |
| `__source_tag__: str \| None`    | The tag that produced the node: a config directive (`!secret`/`!env_var`/`!include*`) or a custom `!mytag`; `None` for a plain inline scalar or a core `!!type` tag.                   |
| `__source_target__: str \| None` | The directive's argument (secret name, include path, env-var spec); `None` when there is no directive. With `__source_tag__`, reconstructs the directive (e.g. `!secret db_password`). |

The three booleans `is_secret`, `is_env_var`, and `is_include` are convenience
predicates over `__source_tag__` for the built-in config tags (`is_include`
spans all five `!include*` variants). They are also on the round-trip
[`YAMLRocksNode`](#yamlrocksnode).

String scalars become `YAMLRocksAnnotatedStr`. By default non-string scalars stay
plain; add `OPT_ANNOTATE_NUMBERS` to also annotate integers and floats as
`YAMLRocksAnnotatedInt` / `YAMLRocksAnnotatedFloat` (same attributes; `bool`/`None` are
never annotated, as Python forbids subclassing them).

```python
import yamlrocks

data = yamlrocks.loads(b"server:\n  host: localhost\n", option=yamlrocks.OPT_ANNOTATED)
data.__line__  # 1
data.__column__  # 1
data.__file__  # None
type(data["server"]["host"]).__name__  # 'YAMLRocksAnnotatedStr'
```

See the [annotated mode guide](/guides/annotated/).

### `YAMLRocksTag`

A custom-tagged value surfaced by `OPT_PASSTHROUGH_TAG`. Construct one with
`YAMLRocksTag(tag, value)`; passing one to `dumps` emits `!tag value`, so it
round-trips. See [emitting custom tags](/guides/tags/#emitting-custom-tags).

| Member       | Description                                                  |
| ------------ | ------------------------------------------------------------ |
| `tag: str`   | The tag, including its leading `!`, for example `"!custom"`. |
| `value: Any` | The underlying parsed scalar or node.                        |

```python
import yamlrocks

tag = yamlrocks.loads(b"v: !custom 5", option=yamlrocks.OPT_PASSTHROUGH_TAG)["v"]
tag.tag  # '!custom'
tag.value  # '5'
```

See the [custom tags guide](/guides/tags/).

### `YAMLRocksScalar`

A node descriptor returned by a [`dumps`](#dumps) `represent` callback to emit a
value as a scalar. `YAMLRocksScalar(value, *, tag=None, style="auto")`.

| Member             | Description                                                                                       |
| ------------------ | ------------------------------------------------------------------------------------------------- |
| `value: str`       | The scalar text to emit.                                                                          |
| `tag: str \| None` | An optional explicit tag (including its leading `!`).                                             |
| `style: str`       | `"auto"` (let the emitter choose), `"plain"`, `"single"`, `"double"`, `"literal"`, or `"folded"`. |

### `YAMLRocksSequence`

A node descriptor returned by a `represent` callback to emit a value as a
sequence. `YAMLRocksSequence(items, *, tag=None, flow=None)`. `items` is an
iterable of host objects, each re-dispatched through `represent`; `flow=True`
forces flow (`[a, b]`) style. See
[full control with `represent`](/guides/dumping/#full-control-with-represent).

### `YAMLRocksMapping`

A node descriptor returned by a `represent` callback to emit a value as a
mapping. `YAMLRocksMapping(pairs, *, tag=None, flow=None)`. `pairs` is an
iterable of `(key, value)` tuples, each re-dispatched through `represent`;
`flow=True` forces flow (`{a: b}`) style.

### `YAMLRocksTags`

A registry mapping custom tags to handler functions, passed as the `tags`
argument to `loads`/`load`. It is a `dict` subclass, so a plain `{tag: func}`
mapping works just as well; `YAMLRocksTags` only adds a `register` decorator. Each
function is called with the tag's resolved inner value.

| Member                     | Description                                                        |
| -------------------------- | ------------------------------------------------------------------ |
| `register(tag, func=None)` | Register `func` for `tag`. With one argument, returns a decorator. |

```python
import yamlrocks

tags = yamlrocks.YAMLRocksTags()


@tags.register("!vec")
def make_vec(value):
    return tuple(value)


yamlrocks.loads(b"p: !vec [1, 2]", tags=tags)
# {'p': (1, 2)}
```

A registered tag is resolved before a `tag_handler` catch-all. See the
[custom tags guide](/guides/tags/).

## Exceptions

| Exception              | Base         | Raised when                                              |
| ---------------------- | ------------ | -------------------------------------------------------- |
| `YAMLRocksDecodeError` | `ValueError` | Parsing or validation fails.                             |
| `YAMLRocksEncodeError` | `TypeError`  | A value is not serializable and no `default` handled it. |

`YAMLRocksDecodeError` carries the source location both in its message string (for
example `... at line 3, column 1`) and as structured `line`, `column`, and `file`
attributes (1-based; `file` is `None` for in-memory input), plus a `message` with
the text alone. See the [exceptions reference](/reference/exceptions/) for the full
set. The concrete class is a subclass such as `YAMLRocksParseError`.

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

yamlrocks.loads(b"a: 'unterminated")
# yamlrocks.YAMLRocksParseError: ... at line 1, column 4
```

See the [exceptions reference](/reference/exceptions/) for the full error model.

## Compatibility shim

For a gradual migration, `yamlrocks.compat` is a PyYAML drop-in:

```python
from yamlrocks import compat

compat.safe_load("a: 1")  # {'a': 1}
compat.safe_dump({"b": 2, "a": 1})  # 'a: 1\nb: 2\n'
```

It exposes `safe_load`, `safe_load_all`, `safe_dump`, `safe_dump_all`, `load`,
`load_all`, `dump`, `dump_all`, and `YAMLError` (which is
`yamlrocks.YAMLRocksDecodeError`). Note that `safe_dump` returns a `str` (or writes to a
stream), matching PyYAML, and `sort_keys=True` is the default there. See
[Migrating from PyYAML](/getting-started/migrating-from-pyyaml/).

## See also

- [Options](/reference/options/): every `OPT_*` flag.
- [Exceptions](/reference/exceptions/): the error model.
- [Loading](/guides/loading/) and [Dumping](/guides/dumping/): the guides.
