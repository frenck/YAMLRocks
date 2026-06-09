---
title: Loading YAML
description: Parse YAML into native Python objects with loads, loads_all, and load.
---

Loading is the act of turning YAML text into native Python objects. YAMLRocks gives
you three entry points: `loads` for a string or bytes you already hold, `load`
for a file on disk, and `loads_all` / `load_all` for streams that contain more
than one document. All of them share the same options and the same type rules,
so once you know one you know them all.

## `loads`: parse a string or bytes

`loads` parses the first document in its input and returns native Python
objects. The input may be `str`, `bytes`, `bytearray`, or any object that
supports the buffer protocol (such as `memoryview`):

```python
import yamlrocks

yamlrocks.loads(b"key: value")          # {'key': 'value'}
yamlrocks.loads("count: 42")            # {'count': 42}
yamlrocks.loads(bytearray(b"x: 1"))     # {'x': 1}
yamlrocks.loads(memoryview(b"x: 1"))    # {'x': 1}
```

An empty document (or input that is only comments) returns `None`:

```python
import yamlrocks

print(yamlrocks.loads(b""))             # None
print(yamlrocks.loads(b"# just a comment"))  # None
```

:::tip[Bytes are fastest]
YAMLRocks is happiest with `bytes`. If your YAML already arrives as
bytes from a socket or file, pass them straight through. A `str` is accepted and
encoded to UTF-8 internally.
:::

## Type resolution

By default YAMLRocks follows the **YAML 1.2 core schema**. Scalars resolve to Python
types as follows:

| YAML                   | Python  | Examples                      |
| ---------------------- | ------- | ----------------------------- |
| `null`, `~`, _(empty)_ | `None`  | `key:`                        |
| `true` / `false`       | `bool`  | `enabled: true`               |
| integers               | `int`   | `42`, `0xFF`, `0o17`, `-5`    |
| floats                 | `float` | `3.14`, `1e3`, `.inf`, `.nan` |
| everything else        | `str`   | `hello`, `2026-01-02`, `yes`  |

```python
import yamlrocks

source = """
n: null
b: true
i: 42
x: 0xFF
f: 3.14
s: hello
"""

yamlrocks.loads(source)
# {'n': None, 'b': True, 'i': 42, 'x': 255, 'f': 3.14, 's': 'hello'}
```

The most common surprise for people coming from PyYAML is that `yes`, `no`, `on`,
and `off` are **plain strings** in YAML 1.2, not booleans:

```python
import yamlrocks

yamlrocks.loads(b"a: yes")    # {'a': 'yes'}
```

:::note[Want the old 1.1 behavior?]
Pass `option=yamlrocks.OPT_YAML_1_1` to get `yes`/`no` booleans, `0777` octals, and
the rest of the YAML 1.1 schema. See
[YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for the full list of differences and
why 1.2 is the safer default.
:::

## `load`: parse a file

`load` is the file-oriented counterpart to `loads`. Pass it a path (a `str` or
any `os.PathLike`) or an open file object:

```python
import yamlrocks

with open("config.yaml", "w") as f:
    f.write("name: app\nport: 8080\n")

yamlrocks.load("config.yaml")           # {'name': 'app', 'port': 8080}

with open("config.yaml") as f:
    yamlrocks.load(f)                   # {'name': 'app', 'port': 8080}
```

`load` shines with split configurations: when you set `OPT_INCLUDES` and do not
pass an `include_dir`, includes resolve relative to the file's own directory,
which is almost always what you want. See [includes](/guides/includes/).

## Multiple documents

A single YAML stream can hold several documents separated by `---`. Use
`loads_all` (or `load_all` for a file) to get them all as a list:

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

`loads_all` and `load_all` accept `option`, `tag_handler`, and `tags`, the same
as their single-document twins. They do **not** take `schema=` or `include_dir`:
schema validation and `!include` resolution are single-document operations, so
apply them per document instead. Iterate the result and call `loads` with a
`schema` on each, or split the stream and resolve includes one document at a
time.

## Block scalars

Literal (`|`) and folded (`>`) block scalars are fully supported, including the
chomping indicators (`-` strip, `+` keep):

```python
import yamlrocks

literal = """
text: |
  line 1
  line 2
"""

yamlrocks.loads(literal)["text"]
# 'line 1\nline 2\n'

folded = """
text: >
  one
  long
  paragraph
"""

yamlrocks.loads(folded)["text"]
# 'one long paragraph\n'
```

A literal block keeps newlines verbatim; a folded block joins lines with spaces.

## Anchors, aliases, and merge keys

Anchors (`&name`) mark a node, aliases (`*name`) reuse it, and the merge key
(`<<`) folds one mapping into another. YAMLRocks resolves all three while parsing:

```python
import yamlrocks

alias = """
base: &b
  x: 1
use: *b
"""

yamlrocks.loads(alias)
# {'base': {'x': 1}, 'use': {'x': 1}}

merge = """
base: &b {x: 1}
use:
  <<: *b
  y: 2
"""

yamlrocks.loads(merge)
# {'base': {'x': 1}, 'use': {'y': 2, 'x': 1}}
```

Explicit keys win over merged ones, and earlier merges win over later ones,
matching PyYAML and ruamel.yaml.

:::caution[Alias expansion is bounded]
A malicious document can use nested aliases to blow up exponentially (the
"billion laughs" attack). YAMLRocks caps total node expansion and nesting depth, so
such input raises `YAMLRocksDecodeError` instead of exhausting memory. See
[security](/reference/security/).
:::

## Duplicate keys

By default a repeated mapping key keeps the **last** value, as PyYAML does:

```python
import yamlrocks

source = """
a: 1
a: 2
"""

yamlrocks.loads(source)                 # {'a': 2}
```

Pass `OPT_DUPLICATE_KEYS_ERROR` to reject duplicates instead. The error reports
the line and column of the offending key:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

source = """
a: 1
b: 2
a: 3
"""

yamlrocks.loads(source, option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)
# yamlrocks.YAMLRocksDuplicateKeyError: duplicate mapping key: a at line 3, column 1
```

The merge key `<<` is exempt, since repeating it is how multiple mappings are
merged.

## Complex keys

YAML lets a mapping key be any node, including a sequence or another mapping (a
"complex key"). [Example 2.11 of the spec](https://yaml.org/spec/1.2.2/#example-mapping-between-sequences),
"Mapping between Sequences," is built on exactly this. A Python `dict`, however,
needs **hashable** keys, and a `list` or `dict` is unhashable. Rather than reject
valid YAML, YAMLRocks renders a complex key as its hashable counterpart: a sequence
becomes a `tuple`, and a mapping becomes a `tuple` of its `(key, value)` pairs (in
order). A `tuple` is used (rather than a `frozenset`) so the key survives a
`dumps`/`loads` round-trip unchanged: a `frozenset` re-serializes as a sequence
and would reload as a different type.

```python
import yamlrocks

# A sequence key becomes a tuple.
data = yamlrocks.loads(b"[a, b]: paired\n")
assert data == {("a", "b"): "paired"}

# A mapping key becomes a tuple of its (key, value) pairs.
source = """
? {x: 1}
: nested
"""

data = yamlrocks.loads(source)
assert data == {(("x", 1),): "nested"}
```

The conversion is recursive, so nested collections inside a key are made hashable
too. It applies on every load path that builds Python values, plain `loads`,
[annotated mode](/guides/annotated/), and custom-tag resolution, so they all
produce the same key.

:::note[More compliant than PyYAML]
PyYAML's `SafeLoader` rejects a complex key with `found unhashable key`, which is a
limitation of mapping YAML onto a Python `dict`, not a rule of the YAML spec.
YAMLRocks accepts the document instead (ruamel.yaml does too, via its own wrapper
types). If you are migrating tests that expected PyYAML to raise on a complex key,
those documents are valid YAML and now load.
:::

### Rejecting complex keys: `OPT_REJECT_COMPLEX_KEYS`

Accept-and-convert is the right default, but some consumers have a strictly
scalar-keyed data model (a config loader, say) where a complex key is always a
mistake, and would rather catch it early with a precise location than convert it
and fail vaguely later. `OPT_REJECT_COMPLEX_KEYS` switches to that behavior: a
collection used as a mapping key raises `YAMLRocksComplexKeyError` instead of
converting.

```python
import yamlrocks

try:
    yamlrocks.loads(b"{a: 1}: b\n", option=yamlrocks.OPT_REJECT_COMPLEX_KEYS)
except yamlrocks.YAMLRocksComplexKeyError as err:
    print(err.line, err.column)
# 1 1
```

`YAMLRocksComplexKeyError` is a [`YAMLRocksDecodeError`](/reference/exceptions/) (so
`except YAMLRocksError` and `except ValueError` still catch it) and carries
`.file`/`.line`/`.column` pointing at the offending key, including when the key is
inside an [`!include`](/guides/includes/)d file. The flag rejects **any** complex
key (both sequence and mapping keys), applies on the plain, annotated, and
tag-resolving paths, and leaves scalar keys untouched. `OPT_ROUND_TRIP` is
unaffected, since a `YAMLRocksDocument` models source bytes rather than Python containers.

:::tip[The unquoted-template trap]
The most common way to hit this by accident is an unquoted template that occupies
a whole value:

```yaml
state: { { states('sensor.x') } } # YAML sees a mapping used as a key
```

Because the value starts with `{`, YAML reads it as a flow mapping in key
position, not as text. Quoting it (`state: "{{ states('sensor.x') }}"`) makes it a
plain string. `OPT_REJECT_COMPLEX_KEYS` turns this typo into an immediate, located
error rather than a value that fails later. (An _embedded_ template like
`name: app_{{ env }}` starts with a normal character, so it is already a plain
scalar and is unaffected.)
:::

## Custom tags

By default an unrecognized tag like `!mytag` is dropped and its underlying value
kept. To intercept tags, pass a `tag_handler` callback, or use
`OPT_PASSTHROUGH_TAG` to receive `YAMLRocksTag` objects. See [custom tags](/guides/tags/):

```python
import yamlrocks

yamlrocks.loads(
    b"value: !double 5",
    tag_handler=lambda tag, value: int(value) * 2 if tag == "!double" else value,
)
# {'value': 10}
```

## Async loading: off the event loop

Each loader has an `async` counterpart: `async_loads`, `async_load`, and
`async_load_all`. They take the same arguments as their synchronous twins and
return the same values, but run the work in a worker thread so an asyncio
application never blocks its loop while parsing:

```python
import asyncio
import yamlrocks

source = """
name: app
port: 8080
"""

async def main():
    data = await yamlrocks.async_loads(source)
    return data

asyncio.run(main())
# {'name': 'app', 'port': 8080}
```

`async_load` and `async_load_all` move the file read off the loop as well, so a
slow disk does not stall it either:

```python
import asyncio
import yamlrocks

with open("config.yaml", "w") as f:
    f.write("name: app\nport: 8080\n")

async def main():
    return await yamlrocks.async_load("config.yaml")

asyncio.run(main())
# {'name': 'app', 'port': 8080}
```

What makes this more than a convenience wrapper is that the native scan and
parse release the GIL on byte input. The worker thread does the heavy parsing
while the event loop keeps running, so other coroutines genuinely make progress
during a large parse rather than waiting behind it. You can `asyncio.gather`
several loads and let them overlap:

```python
import asyncio
import yamlrocks

async def main():
    docs = [b"a: %d" % i for i in range(3)]
    return await asyncio.gather(*(yamlrocks.async_loads(d) for d in docs))

asyncio.run(main())
# [{'a': 0}, {'a': 1}, {'a': 2}]
```

:::note[The GIL release applies to the fast path]
The full GIL release covers plain parsing. When a call also runs your Python
code (a `tag_handler`, a `tags` function, `schema` validation, annotated mode,
or round-trip), that work still holds the GIL inside the worker thread, so the
loop is freed only partially. There is no async tag resolution.
:::

For serializing there is deliberately no async loader counterpart on the dump
side beyond file I/O; see [async dumping](/guides/dumping/#async-dumping) for why
and the recommended workaround.

## When parsing fails

A genuinely malformed document raises `YAMLRocksDecodeError`, a subclass of
`ValueError`. The message carries the source location:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

yamlrocks.loads(b"a: 'unterminated")
# yamlrocks.YAMLRocksParseError: unterminated single-quoted scalar at line 1, column 4
```

See [exceptions](/reference/exceptions/) for the full error model.

## See also

- [Dumping YAML](/guides/dumping/): the reverse direction.
- [Round-trip editing](/guides/round-trip/): load while preserving comments.
- [Annotated mode](/guides/annotated/): load with source line and column.
- [Schema validation](/guides/schema-validation/): validate while parsing.
- [API reference](/reference/api/) and [options](/reference/options/).
