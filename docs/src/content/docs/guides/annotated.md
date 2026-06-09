---
title: Annotated mode
description: Load YAML into dict, list, and str subclasses that remember their source line, column, and file.
---

When a tool needs to point a user at the exact spot of a problem ("the port on
line 7 is out of range"), it needs to know where each value came from. Plain
parsing throws that information away the moment the text becomes objects.

`OPT_ANNOTATED` keeps it. Instead of plain containers it returns lightweight
subclasses (`YAMLRocksAnnotatedDict`, `YAMLRocksAnnotatedList`, and `YAMLRocksAnnotatedStr`) that behave
exactly like `dict`, `list`, and `str`, but also carry the source location of the
node they represent. Your existing code keeps working unchanged; the location is
there when you reach for it.

```python
import yamlrocks

data = yamlrocks.loads(
    b"name: app\nserver:\n  host: localhost\n  port: 8080\n",
    option=yamlrocks.OPT_ANNOTATED,
)

isinstance(data, dict)            # True (a real dict subclass)
data.__line__                     # 1
data.__column__                   # 1

data["server"].__line__           # 3  (the mapping body starts here)
data["server"]["host"].__line__   # 3
data["server"]["host"].__column__ # 9
```

Every annotated node exposes five attributes:

| Attribute        | Meaning                                                 |
| ---------------- | ------------------------------------------------------- |
| `__line__`       | 1-based source line where the node starts               |
| `__column__`     | 1-based source column where the node starts             |
| `__file__`       | originating file path, or `None`                        |
| `__end_line__`   | 1-based line just past the node's last character        |
| `__end_column__` | 1-based column just past the node's last character      |
| `__offset__`     | 0-based byte offset of the node's first character       |
| `__end_offset__` | 0-based byte offset just past the node's last character |

:::note[Lines and columns are 1-based]
The first character of a file is line 1, column 1, matching how editors and
compilers report positions, so you can hand these numbers straight to a user
without adjusting them.
:::

The start and end together give a full span you can underline. For a scalar the
end is just past its last character; for a mapping or sequence it reaches the end
of the block (the furthest point of any child). This mirrors the start/end marks
PyYAML exposes as `node.start_mark`/`node.end_mark`.

```python
import yamlrocks

data = yamlrocks.loads(b"key: value\nbroad: x\n", option=yamlrocks.OPT_ANNOTATED)
key = list(data)[1]                              # the 'broad' key
(key.__line__, key.__column__)                   # (2, 1)
(key.__end_line__, key.__end_column__)           # (2, 6)  (just past 'broad')
```

:::note[End positions are exact, quotes included]
The end position marks the source just past the node's last character, so for a
quoted or escaped scalar it lands past the **closing quote**, not at the shorter
decoded value's length. Start and end positions are both exact, and this matches
what round-trip [`YAMLRocksNode.range()`](/guides/round-trip/) reports.

When you need the raw bytes rather than a line/column, the byte offsets
(`__offset__` / `__end_offset__`) slice the source directly:
`source[node.__offset__ : node.__end_offset__]` yields the verbatim source token,
quotes and all.
:::

## They behave like the builtins

The whole point of annotated mode is that nothing else changes. A `YAMLRocksAnnotatedDict`
is a `dict`, a `YAMLRocksAnnotatedList` is a `list`, and a `YAMLRocksAnnotatedStr` is a `str`, so
they pass `isinstance` checks, support every method and operator, and serialize
the way you expect:

```python
import yamlrocks

source = """
name: app
server:
  host: localhost
"""

data = yamlrocks.loads(source, option=yamlrocks.OPT_ANNOTATED)

# Dict behavior.
list(data.keys())                 # ['name', 'server']
{**data["server"]}                # {'host': 'localhost'}

# Str behavior on a scalar.
host = data["server"]["host"]
host.upper()                      # 'LOCALHOST'
host == "localhost"               # True
host + ":8080"                    # 'localhost:8080'
```

Because they are genuine subclasses, you can pass annotated values to any function
that expects a plain `dict`, `list`, or `str` and it will not notice the
difference.

### Attaching attributes

Some libraries hook a type by setting a **class attribute** on it (for example
voluptuous looks for a `__voluptuous_compile__` method to compile a value). That
is supported: the annotated classes are writable, so you can attach methods or
other class attributes to them.

```python
import yamlrocks

data = yamlrocks.loads(b"name: app", option=yamlrocks.OPT_ANNOTATED)

# Attach a class attribute (here a method) to the annotated string type.
type(data["name"]).__shout__ = lambda self: self.upper() + "!"
data["name"].__shout__()          # 'APP!'
```

:::note[Class attributes, not instance attributes]
You can always attach **class** attributes (as above). Setting an arbitrary
**instance** attribute, such as `node.custom = 1`, works only on
`YAMLRocksAnnotatedStr` instances; the native dict and list nodes reject it with
`AttributeError`. Class attributes are the portable choice.
:::

## Which nodes are annotated

Mappings become `YAMLRocksAnnotatedDict`, sequences become `YAMLRocksAnnotatedList`, and string
scalars become `YAMLRocksAnnotatedStr`, **including mapping keys**, so you can point an
error at the exact key rather than only its value. By default the remaining
scalars (integers, floats, booleans, and `None`) stay as their plain Python
types. Annotated keys are still ordinary, hashable strings, so dict lookups with a
plain `str` work unchanged.

```python
import yamlrocks

data = yamlrocks.loads(
    b"server:\n  host: localhost\n  port: 8080\n",
    option=yamlrocks.OPT_ANNOTATED,
)

type(data).__name__                       # 'YAMLRocksAnnotatedDict'
next(iter(data)).__line__                 # 1  (the `server` key's own line)
type(data["server"]["host"]).__name__     # 'YAMLRocksAnnotatedStr'
type(data["server"]["port"]).__name__     # 'int'  (plain by default)
```

So by default a string value like `host` carries `__line__`/`__column__`, but an
integer like `port` does not. To locate a non-string scalar, read the position
from the mapping or sequence that contains it, or opt into numeric annotation
(below).

### Locating numbers: `OPT_ANNOTATE_NUMBERS`

Add `OPT_ANNOTATE_NUMBERS` to also annotate integers and floats, so an error on a
numeric value (an out-of-range port, say) can point at its own line. Integers
become `YAMLRocksAnnotatedInt` and floats `YAMLRocksAnnotatedFloat`, carrying the same
attributes as annotated strings:

```python
import yamlrocks

data = yamlrocks.loads(
    b"port: 8080\n",
    option=yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_ANNOTATE_NUMBERS,
)
data["port"]              # 8080
data["port"].__line__     # 1
data["port"] + 1          # 8081  (still an int in every way that matters)
```

An annotated number is an `int`/`float` _subclass_: `isinstance(x, int)`,
equality, arithmetic, and hashing all behave normally, but `type(x) is int` is
`False`, and there is a small per-number boxing cost. The flag is off by default
so the common case stays plain (and fast). `bool` and `None` are never annotated,
even with the flag: Python does not allow subclassing them, which is also why
PyYAML leaves them unannotated.

A [complex key](/guides/loading/#complex-keys) (a sequence or mapping used as a
mapping key) is rendered the same way as on the plain path, a `tuple` (a mapping
key becomes a `tuple` of its pairs), since a Python `dict` cannot key on an
unhashable annotated container. Scalar keys are still annotated; only collection
keys convert.

### Knowing how a string was written: `__style__`

A `YAMLRocksAnnotatedStr` also carries `__style__`, the source style of the scalar:
`"plain"`, `"single"` (`'...'`), `"double"` (`"..."`), `"literal"` (`|`), or
`"folded"` (`>`). This lets a tool tell a block scalar from an inline one, for
example to offset a generated `#line` directive to the block body, since a block
scalar's `__line__` points at the `|`/`>` indicator line and the content begins on
the next line. The vocabulary matches round-trip
[`YAMLRocksNode.style`](/guides/round-trip/).

```python
import yamlrocks

source = """
inline: hi
block: |
  body
"""

data = yamlrocks.loads(source, option=yamlrocks.OPT_ANNOTATED)
data["inline"].__style__          # 'plain'
data["block"].__style__           # 'literal'  (a | block; content starts at __line__ + 1)
```

### Knowing which tag produced a value: `__source_tag__`

Every annotated node carries `__source_tag__`: the tag that produced it, or
`None` for a plain inline scalar. It is the originating config directive
(`"!secret"`, `"!env_var"`, an `"!include"` family tag) when the value came from
one, or the node's own custom application tag (`"!mytag"`) otherwise. Core
`!!type` tags are not provenance and report `None`.

For the three built-in config tags there are convenience predicates,
`is_secret`, `is_env_var`, and `is_include` (the last covers all five
`!include*` variants), so a tool can react without string-matching:

```python
import os
import tempfile
import yamlrocks

workdir = tempfile.mkdtemp()
with open(os.path.join(workdir, "secrets.yaml"), "w") as f:
    f.write("api_key: s3cr3t\n")
with open(os.path.join(workdir, "configuration.yaml"), "w") as f:
    f.write("api_key: !secret api_key\ntitle: My App\n")

opt = yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_SECRETS | yamlrocks.OPT_INCLUDES
data = yamlrocks.load(os.path.join(workdir, "configuration.yaml"), option=opt)

data["api_key"].is_secret          # True  (from `api_key: !secret api_key`)
data["api_key"].__source_tag__     # '!secret'
data["api_key"].__source_target__  # 'api_key'  (the directive's argument)
data["title"].__source_tag__       # None  (a plain inline value)
```

`__source_target__` carries the directive's _argument_: the secret name for
`!secret`, the path for `!include`, the variable spec for `!env_var` (or `None`
when there is no directive). Together with `__source_tag__` it reconstructs the
original directive, e.g. `f"{n.__source_tag__} {n.__source_target__}"` gives back
`"!secret api_key"`, which is what a tool needs to redact a value _and_ re-emit
the reference rather than the resolved secret.

This is what lets a viewer or linter working on the parsed tree redact
secret-derived values, or flag where an env var or include fed a value, without a
separate bookkeeping pass. The same attribute and predicates are on the
round-trip [`YAMLRocksNode`](/guides/round-trip/).

:::note[Provenance, not the resolved tag]
`__source_tag__` records _what produced_ the node, which is why it survives even
though `!secret`/`!include` resolve the value in place (a secret's value is a
plain string by the time you see it, but `__source_tag__` still says `"!secret"`).
It is distinct from a node's YAML _type_ tag; with `OPT_PASSTHROUGH_TAG` a custom
tag instead comes back as a [`YAMLRocksTag`](/reference/api/#yamlrockstag) object.
:::

:::note[Timestamps, decimals, and UUIDs decode as strings]
YAMLRocks only ever loads seven kinds of value: `dict`, `list`, `str`, `int`,
`float`, `bool`, and `None`. A scalar that looks like a timestamp
(`2024-01-01T12:00:00`), a decimal, or a UUID is decoded as a plain string, so it
comes back as a `YAMLRocksAnnotatedStr` and carries its location like any other string.
This is the same asymmetry as [`dumps`](/guides/dumping/), which can _write_ a
`datetime`, `Decimal`, or `UUID` but never reconstructs one on load. There is
therefore no annotated `datetime` (or annotated dataclass): those types simply
never appear on the load path.
:::

Sequence elements are annotated individually, so you can locate any item:

```python
import yamlrocks

data = yamlrocks.loads(b"items:\n  - a\n  - b\n", option=yamlrocks.OPT_ANNOTATED)

type(data["items"]).__name__      # 'YAMLRocksAnnotatedList'
data["items"].__line__            # 2
data["items"][0].__line__         # 2
data["items"][1].__line__         # 3
```

## Tracking the originating file

`__file__` is `None` for input parsed from a string or bytes, since there is no
file behind it. It becomes meaningful when a value is pulled in from another file
through an [`!include`](/guides/includes/) directive: each annotated node then
reports the file it physically came from, which is exactly what you need to send a
user to the right place in a split configuration.

The example below is self-contained: it writes a small two-file configuration to a
temporary directory, then reads back the file each node belongs to.

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()

with open(os.path.join(config, "configuration.yaml"), "wb") as handle:
    handle.write(b"automation: !include automations.yaml\n")
with open(os.path.join(config, "automations.yaml"), "wb") as handle:
    handle.write(b"- alias: night\n  trigger: time\n")

data = yamlrocks.load(
    os.path.join(config, "configuration.yaml"),
    option=yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_INCLUDES,
)

# The included list and its items report the file they came from.
assert data["automation"].__file__.endswith("automations.yaml")
assert data["automation"][0].__file__.endswith("automations.yaml")
```

## Aliases share their anchor's object

An anchor (`&a`) and every alias (`*a`) that references it resolve to the **same**
annotated object, exactly as PyYAML does. They are not independent copies, so a
mutation made through one reference is visible through all of them:

```python
import yamlrocks

source = """
base: &a
  k: 1
ref: *a
"""

data = yamlrocks.loads(source, option=yamlrocks.OPT_ANNOTATED)

data["base"] is data["ref"]   # True, the same object
data["base"]["k"] = 99
data["ref"]["k"]              # 99, seen through the shared reference
```

This matters for tools that define a block once under an anchor and reuse it in
several places, then process the result in place: the work is done once and seen
everywhere. The plain [fast path](/guides/loading/) (`loads` with no options)
instead gives each alias an independent copy, which is faster when you do not need
shared identity.

## Home Assistant compatibility

Annotated mode mirrors the node classes Home Assistant uses internally for exactly
this purpose. `YAMLRocksAnnotatedDict`, `YAMLRocksAnnotatedList`, and `YAMLRocksAnnotatedStr` stand in for
Home Assistant's `NodeDictClass`, `NodeListClass`, and `NodeStrClass`, exposing
the same `__line__`, `__column__`, and `__file__` attributes. Code that inspects
those attributes to produce friendly, location-aware error messages works against
YAMLRocks's annotated objects unchanged.

:::tip[Annotated or round-trip?]
The two modes draw a clean line. Reach for `OPT_ANNOTATED` on the **loader** path,
when you only need to _read_ positions: validators, linters, and error reporters.
Its values are real `dict`/`list`/`str` objects, so they pass straight through a
validation library unchanged. Reach for [`OPT_ROUND_TRIP`](/guides/round-trip/) on
the **editor** path, when you need to _edit_ the document and write it back with
comments and formatting preserved; round-trip nodes also expose positions through
`range()`.

Annotated values deliberately carry no handle back to a round-trip node: that
would pin the whole document in memory and grow stale the moment a validator
rebuilds the data. If you locate a problem in annotated mode and then want to fix
it, re-load with `OPT_ROUND_TRIP` and navigate to the same key path
(`doc.node["server"]["port"]`). The path is the stable bridge between the modes.
:::

## See also

- [Loading YAML](/guides/loading/): plain parsing without annotations.
- [Round-trip editing](/guides/round-trip/): editable positions via `range()`.
- [Includes](/guides/includes/): where `__file__` comes from.
- [Home Assistant recipe](/recipes/home-assistant/): annotated mode in practice.
- [API reference](/reference/api/) and [options](/reference/options/).
