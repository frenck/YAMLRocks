---
title: Dumping YAML
description: Serialize Python objects to YAML bytes with dumps, including custom types and emitting options.
---

Dumping is the act of turning native Python objects back into YAML. `dumps`
takes any supported object and returns the encoded document. Its companion
`dump` writes to a file or file-like target instead. Both share the same type
rules and the same emitting options, so what you learn here applies to either.

## `dumps` returns bytes

`dumps` returns `bytes`, not `str`. The bytes are UTF-8 encoded and
end with a trailing newline:

```python
import yamlrocks

yamlrocks.dumps({"name": "app", "ports": [80, 443]})
# b'name: app\nports:\n  - 80\n  - 443\n'
```

Returning bytes is a deliberate, performance-minded choice: most destinations
for a serialized document (a file opened in binary mode, a socket, an HTTP
response body, a subprocess stdin) want bytes anyway, so handing you bytes
avoids an extra encode step.

:::tip[Write straight to a file or socket]
Because `dumps` already returns UTF-8 bytes, you can send them to a binary sink
with no conversion:

```python
import yamlrocks

payload = yamlrocks.dumps({"name": "app", "ports": [80, 443]})

with open("config.yaml", "wb") as f:   # note the "wb" binary mode
    f.write(payload)
```

If you genuinely need text, decode explicitly with `payload.decode()`. For
writing to disk you usually want [`dump`](/reference/api/) instead, which takes a
path or file object directly.
:::

## Emitting options

The shape of the output is controlled with option flags, combined with `|`. Each
flag below is shown with a runnable before-and-after so you can see exactly what
changes.

### Indentation: `OPT_INDENT_2` and `OPT_INDENT_4`

Block indentation defaults to two spaces (`OPT_INDENT_2`). Pass `OPT_INDENT_4`
for four:

```python
import yamlrocks

data = {"server": {"host": "localhost", "port": 8080}}

yamlrocks.dumps(data)
# b'server:\n  host: localhost\n  port: 8080\n'

yamlrocks.dumps(data, option=yamlrocks.OPT_INDENT_4)
# b'server:\n    host: localhost\n    port: 8080\n'
```

### Sequence indentation: `OPT_INDENTLESS_SEQUENCES`

A block sequence under a key is indented one level by default (`key:` then
`  - item`), the style most configuration ecosystems use. Pass
`OPT_INDENTLESS_SEQUENCES` to align the dashes with the key instead (`key:` then
`- item`), the "indentless" style favored by `kubectl` and much of the Kubernetes
world:

```python
import yamlrocks

data = {"ports": [80, 443]}

yamlrocks.dumps(data)
# b'ports:\n  - 80\n  - 443\n'

yamlrocks.dumps(data, option=yamlrocks.OPT_INDENTLESS_SEQUENCES)
# b'ports:\n- 80\n- 443\n'
```

### Sorting keys: `OPT_SORT_KEYS`

By default keys are emitted in insertion order. `OPT_SORT_KEYS` sorts every
mapping alphabetically, which is handy for stable diffs and reproducible output:

```python
import yamlrocks

yamlrocks.dumps({"b": 1, "a": 2})
# b'b: 1\na: 2\n'

yamlrocks.dumps({"b": 1, "a": 2}, option=yamlrocks.OPT_SORT_KEYS)
# b'a: 2\nb: 1\n'
```

### Flow style: `OPT_FLOW_STYLE`

The default is block style, where each entry sits on its own line. `OPT_FLOW_STYLE`
emits the compact JSON-like flow form with `{}` and `[]`:

```python
import yamlrocks

yamlrocks.dumps({"a": [1, 2]})
# b'a:\n  - 1\n  - 2\n'

yamlrocks.dumps({"a": [1, 2]}, option=yamlrocks.OPT_FLOW_STYLE)
# b'{a: [1, 2]}\n'
```

### Multi-line strings

A multi-line string is emitted as a literal `|` block by default, which is how
real-world YAML overwhelmingly writes multi-line content (embedded scripts,
certificates, descriptions) and far more readable than a double-quoted scalar
full of `\n` escapes:

```python
import yamlrocks

yamlrocks.dumps({"s": "line 1\nline 2\n"})
# b's: |\n  line 1\n  line 2\n'
```

The chomping indicator is chosen automatically so the block round-trips exactly:
`|` keeps a single trailing newline, `|-` strips it when there is none, and `|+`
keeps extra trailing blank lines. A string a literal block cannot represent
faithfully (it contains a carriage return or other control character, or its
first line begins with whitespace) falls back to a double-quoted scalar, so
`loads(dumps(x)) == x` holds for every string.

### Document markers: `OPT_EXPLICIT_START` and `OPT_EXPLICIT_END`

These add the explicit `---` start marker and `...` end marker. They are useful
when concatenating documents into a single stream:

```python
import yamlrocks

yamlrocks.dumps({"a": 1}, option=yamlrocks.OPT_EXPLICIT_START | yamlrocks.OPT_EXPLICIT_END)
# b'---\na: 1\n...\n'
```

:::note[Combining options]
Flags compose freely with `|`, and any flag that does not apply to a given call
is simply ignored:

```python
import yamlrocks

yamlrocks.dumps({"b": 1, "a": 2}, option=yamlrocks.OPT_SORT_KEYS | yamlrocks.OPT_INDENT_4)
# b'a: 2\nb: 1\n'
```

:::

### Null style: empty, `null`, or `~`

`None` is left **blank** by default (`key:` with nothing after the colon), which
is what hand-written configs and PyYAML-based tools overwhelmingly produce. Some
formats prefer the explicit `null` keyword (data and spec formats such as
OpenAPI), and some prefer the `~` indicator. Set `OPT_NULL_AS_KEYWORD` or
`OPT_NULL_AS_TILDE` to make that style the default (the two flags are mutually
exclusive):

```python
import yamlrocks

yamlrocks.dumps({"a": None, "b": None})
# b'a:\nb:\n'

yamlrocks.dumps({"a": None, "b": None}, option=yamlrocks.OPT_NULL_AS_KEYWORD)
# b'a: null\nb: null\n'

yamlrocks.dumps({"a": None}, option=yamlrocks.OPT_NULL_AS_TILDE)
# b'a: ~\n'
```

The three styles all parse back to `None`, so the choice is cosmetic. The blank
form is only used where it is unambiguous, a block mapping value or a block
sequence entry; at the top level, inside a flow collection, or as a mapping key it
falls back to `null` so the output stays valid YAML 1.2.

### Line width: `width`

By default `dumps` never wraps: a long scalar or flow collection emits on one
line. Pass `width=N` to fold lines to a best-effort maximum, the way PyYAML's
`width` does. A long scalar folds at spaces and flow collections break after
commas. A plain scalar that needs wrapping is emitted double-quoted, because a
bare plain scalar cannot fold safely (a continuation line could start an
indicator or land at an enclosing indent), whereas a break inside quotes always
folds back to a single space:

```python
import yamlrocks

config = {"description": "a fairly long sentence that we would like wrapped onto a few lines"}
yamlrocks.dumps(config, width=40)
# b'description: "a fairly long sentence\n             that we would like wrapped\n             onto a few lines"\n'
```

The one rule that is never broken is **value fidelity**: a fold only happens
where it cannot change the decoded string, so `loads(dumps(x, width=N)) == x`
always holds. That makes the width a _soft_ limit, because some lines have no
safe place to break:

- A run of two or more spaces is never split (a fold there would drop one space).
- A long word with no spaces (a URL, a token) stays on its line.
- A multi-line string emits as a literal `|` block, whose lines are preserved
  verbatim and so are not re-wrapped.

```python
import yamlrocks

yamlrocks.dumps({"url": "https://example.com/a/very/long/unbreakable/path/here"}, width=20)
# b'url: https://example.com/a/very/long/unbreakable/path/here\n'
```

This is the knob to reach for when a project requires lines at or under a length
(for example to satisfy [yamllint](https://yamllint.readthedocs.io/)'s
`line-length` rule, whose default also exempts a single unbreakable word).

`width` applies only to the fast `dumps` path. Round-trip mode preserves the
original layout byte-for-byte, so it is unaffected.

## Quoting

YAMLRocks quotes scalars only when needed to keep the document unambiguous. A
string that would otherwise parse back as another type (a bool, a number, null)
is quoted automatically, so a round-trip never silently changes a value:

```python
import yamlrocks

yamlrocks.dumps({"version": "1.0", "flag": "yes"})
# b'version: "1.0"\nflag: "yes"\n'
```

Here `1.0` is quoted so it stays a string rather than becoming the float `1.0`,
and `yes` is quoted so it is not mistaken for a YAML 1.1 boolean by downstream
tools. See [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for why that matters.

Quoting uses **double quotes** by default. Pass `OPT_SINGLE_QUOTES` to use single
quotes instead, which avoid backslash escaping for values that contain many
backslashes (a regex or a Windows path, say); a value that cannot be single-quoted
(it contains a line break) still falls back to double quotes.

```python
import yamlrocks

yamlrocks.dumps({"flag": "yes"}, option=yamlrocks.OPT_SINGLE_QUOTES)
# b"flag: 'yes'\n"
```

## Supported types

Beyond the core YAML types, YAMLRocks serializes a set of common Python types
directly. The table below
lists the built-in mapping:

| Python type          | YAML output         | Notes                                                   |
| -------------------- | ------------------- | ------------------------------------------------------- |
| `dict`               | mapping             | keys in insertion order, or sorted with `OPT_SORT_KEYS` |
| `list`, `tuple`      | sequence            |                                                         |
| `str`                | scalar              | quoted only when needed                                 |
| `int`, `float`       | scalar              |                                                         |
| `bool`               | `true` / `false`    |                                                         |
| `None`               | `null`              |                                                         |
| `datetime`           | ISO 8601 timestamp  | see datetime options below                              |
| `date`               | `'YYYY-MM-DD'`      |                                                         |
| `time`               | `HH:MM:SS[.ffffff]` |                                                         |
| `uuid.UUID`          | scalar string       | unquoted; a UUID is not a YAML number                   |
| `decimal.Decimal`    | scalar              | exact, no float rounding                                |
| `enum.Enum`          | the member's value  |                                                         |
| dataclass instance   | mapping of fields   |                                                         |
| `pathlib.Path`       | scalar string       |                                                         |
| numpy array / scalar | sequence / scalar   | requires `OPT_SERIALIZE_NUMPY`                          |

A few of these are worth seeing in action:

```python
import yamlrocks
import uuid
import decimal
import enum
import pathlib
from dataclasses import dataclass

yamlrocks.dumps({"id": uuid.UUID("12345678-1234-5678-1234-567812345678")})
# b'id: 12345678-1234-5678-1234-567812345678\n'

yamlrocks.dumps({"price": decimal.Decimal("3.14")})
# b'price: 3.14\n'

class Color(enum.Enum):
    GREEN = "green"

yamlrocks.dumps({"color": Color.GREEN})
# b'color: green\n'

@dataclass
class Point:
    x: int
    y: int

yamlrocks.dumps(Point(1, 2))
# b'x: 1\ny: 2\n'

yamlrocks.dumps({"path": pathlib.Path("/etc/app/config.yaml")})
# b'path: /etc/app/config.yaml\n'
```

### Datetime options

A timezone-aware `datetime` serializes to a full ISO 8601 timestamp by default:

```python
import yamlrocks
import datetime

dt = datetime.datetime(
    2026, 6, 5, 12, 30, 45, 123456, tzinfo=datetime.timezone.utc
)

yamlrocks.dumps(dt)
# b'2026-06-05T12:30:45.123456+00:00\n'
```

Three flags adjust how timestamps render:

- `OPT_OMIT_MICROSECONDS` drops the microsecond component.
- `OPT_NAIVE_UTC` treats a naive datetime (no `tzinfo`) as UTC and appends the
  `+00:00` offset.
- `OPT_UTC_Z` renders a `+00:00` offset as the shorter `Z`.

```python
import yamlrocks
import datetime

dt = datetime.datetime(
    2026, 6, 5, 12, 30, 45, 123456, tzinfo=datetime.timezone.utc
)

yamlrocks.dumps(dt, option=yamlrocks.OPT_OMIT_MICROSECONDS | yamlrocks.OPT_UTC_Z)
# b'2026-06-05T12:30:45Z\n'

naive = datetime.datetime(2026, 6, 5, 12, 30, 45)

yamlrocks.dumps(naive)
# b'2026-06-05T12:30:45\n'

yamlrocks.dumps(naive, option=yamlrocks.OPT_NAIVE_UTC | yamlrocks.OPT_UTC_Z)
# b'2026-06-05T12:30:45Z\n'
```

### numpy

numpy support is off by default, so an unflagged numpy value is treated as an
unsupported type:

<!-- verify: raises YAMLRocksEncodeError -->

```python
import yamlrocks
import numpy as np

yamlrocks.dumps({"a": np.array([1, 2])})
# yamlrocks.YAMLRocksUnserializableError: type ndarray is not YAML serializable
```

Pass `OPT_SERIALIZE_NUMPY` to serialize arrays and scalars:

```python
import yamlrocks
import numpy as np

yamlrocks.dumps({"a": np.array([1, 2])}, option=yamlrocks.OPT_SERIALIZE_NUMPY)
# b'a:\n  - 1\n  - 2\n'

yamlrocks.dumps({"n": np.int64(5)}, option=yamlrocks.OPT_SERIALIZE_NUMPY)
# b'n: 5\n'
```

:::note[Why numpy is opt-in]
numpy is a heavy, optional dependency. Keeping it behind a flag means YAMLRocks
never imports it unless you ask, so projects that do not use numpy pay nothing.
:::

## Custom types and the `default` callback

When YAMLRocks meets a value it does not know how to serialize, it calls your
`default` callback with that value. Return something serializable and YAMLRocks
emits that instead:

```python
import yamlrocks

yamlrocks.dumps(
    {"point": complex(1, 2)},
    default=lambda o: [o.real, o.imag] if isinstance(o, complex) else o,
)
# b'point:\n  - 1.0\n  - 2.0\n'
```

The callback can return a nested structure, and YAMLRocks will serialize that in
turn, so you can map a custom object onto a mapping or sequence:

```python
import yamlrocks

class Money:
    def __init__(self, amount, currency):
        self.amount = amount
        self.currency = currency

def encode(obj):
    if isinstance(obj, Money):
        return {"amount": obj.amount, "currency": obj.currency}
    raise TypeError

yamlrocks.dumps({"total": Money(42, "EUR")}, default=encode)
# b'total:\n  amount: 42\n  currency: EUR\n'
```

### When nothing handles a value

If no `default` is given, or `default` raises, or it returns a value that is
itself unsupported, YAMLRocks raises `YAMLRocksEncodeError`. This is a subclass of
`TypeError`, so existing `except TypeError` handlers keep working:

<!-- verify: raises YAMLRocksEncodeError -->

```python
import yamlrocks

yamlrocks.dumps({"x": object()})
# yamlrocks.YAMLRocksUnserializableError: type object is not YAML serializable
```

### Passthrough: route built-in types to `default`

Sometimes you want to override how a type YAMLRocks already supports is emitted.
The passthrough flags tell YAMLRocks to skip its built-in handling for a type and
send it to `default` instead.

`OPT_PASSTHROUGH_DATETIME` routes `datetime`, `date`, and `time` to `default`:

```python
import yamlrocks
import datetime

yamlrocks.dumps(
    datetime.date(2026, 6, 5),
    option=yamlrocks.OPT_PASSTHROUGH_DATETIME,
    default=lambda o: o.strftime("%d/%m/%Y"),
)
# b'05/06/2026\n'
```

`OPT_PASSTHROUGH_DATACLASS` routes dataclass instances to `default`, so you can
emit them in a custom shape instead of a field mapping:

```python
import yamlrocks
from dataclasses import dataclass

@dataclass
class Point:
    x: int
    y: int

yamlrocks.dumps(
    Point(1, 2),
    option=yamlrocks.OPT_PASSTHROUGH_DATACLASS,
    default=lambda o: [o.x, o.y],
)
# b'- 1\n- 2\n'
```

:::caution[Passthrough needs a `default`]
A passthrough flag only diverts the type to your callback; it does not provide a
fallback. If you enable a passthrough flag without a `default` that handles the
diverted type, YAMLRocks raises `YAMLRocksEncodeError`.
:::

## Full control with `represent`

`default` and `serializers` shape _unknown_ types. When you need to control how
**any** value emits, builtins included, with a specific tag or scalar style, pass
a `represent` callback. YAMLRocks calls it for every value it is about to emit.
Return a node descriptor to say exactly how to render that value, or `None` to
defer to the built-in rendering:

```python
import yamlrocks

class Secret:
    def __init__(self, name):
        self.name = name

def represent(value):
    if isinstance(value, Secret):
        return yamlrocks.YAMLRocksScalar(value.name, tag="!secret")
    return None

yamlrocks.dumps({"password": Secret("wifi"), "ssid": "home"}, represent=represent)
# b"password: !secret 'wifi'\nssid: home\n"
```

The value that returned `None` (`"home"`) rendered exactly as a plain `dumps`
would. The `Secret` became a `!secret` node. Note the single quotes: a scalar
carrying a custom tag is quoted automatically, because a plain `!secret wifi`
would reload as a plain string and lose the tag. This mirrors PyYAML, so a host's
representers port across without hand-annotating quote styles.

### The node descriptors

`represent` returns one of three descriptors, or `None`:

<!-- verify: skip -->

```python
yamlrocks.YAMLRocksScalar(value, *, tag=None, style="auto")
yamlrocks.YAMLRocksSequence(items, *, tag=None, flow=None)
yamlrocks.YAMLRocksMapping(pairs, *, tag=None, flow=None)
```

- `tag` writes an explicit tag. A standard tag the value already resolves to
  (`!!bool` on `true`, `!!float` on `1.0e17`) is elided; a custom tag is kept.
- `style` is one of `"auto"`, `"plain"`, `"single"`, `"double"`, `"literal"`
  (a `|` block), or `"folded"` (a `>` block). `"auto"` lets the emitter quote as
  needed; an explicit style is honored verbatim, so it is on you to pick one the
  value can be represented in losslessly (forcing `"plain"` on `"[x]"`, say,
  would reload as a list). A block style inside a flow collection is downgraded
  to a quoted style, since block scalars are invalid there.
- `items` and `pairs` hold your **original** objects, not pre-rendered nodes.
  YAMLRocks re-dispatches each child through `represent`, so you only ever
  describe one level. Indentation, flow, `sort_keys`, and shared-object anchoring
  stay with the library.

A forced block scalar, for example, is just a style:

```python
import yamlrocks

def represent(value):
    if isinstance(value, str) and value.startswith("return"):
        return yamlrocks.YAMLRocksScalar(value, tag="!lambda", style="literal")
    return None

yamlrocks.dumps({"on_press": "return x + 1;"}, represent=represent)
# b'on_press: !lambda |-\n  return x + 1;\n'
```

### Shared objects become anchors

Because the emitter drives the recursion, it sees the whole object graph. A value
that appears more than once emits once with an anchor and aliases the repeats, so
the YAML stays compact and the anchor/alias structure is preserved:

```python
import yamlrocks

shared = {"host": "localhost", "port": 8080}
yamlrocks.dumps({"primary": shared, "backup": shared}, represent=lambda v: None)
# b'primary: &id001\n  host: localhost\n  port: 8080\nbackup: *id001\n'
```

On a plain `loads`, an alias reloads as an equal but distinct object (the fast
loader copies the anchored value); load with `OPT_ANNOTATED` if you need the
reloaded Python objects to share identity the way the anchors imply.

`represent` composes with everything else. It runs first; a value it defers on
(`None`) falls through to the normal pipeline, so `default`, `serializers`, and
the datetime/dataclass/numpy handling still apply, and a deferred value renders
byte-for-byte as it would from a plain `dumps`. `represent` is offered every
value, including those nested inside a deferred set, dataclass, or `default`
result. `OPT_SORT_KEYS` (by type and value, numbers numerically),
`OPT_FLOW_STYLE`, `OPT_EXPLICIT_START`, `OPT_EXPLICIT_END`, the null-style flags,
and the quote-style flag all apply, to what `represent` returns and to deferred
values alike. Block indentation is a fixed two spaces on this path, so
`OPT_INDENT_4`, `OPT_INDENTLESS_SEQUENCES`, and `width` do not apply here.

## Writing to a file with `dump`

`dump` is the file-oriented counterpart to `dumps`. Give it a path or an open
file object as the target. It takes the same `default` and `option` arguments:

<!-- verify: skip -->

```python
import yamlrocks

yamlrocks.dump({"name": "app", "port": 8080}, "config.yaml")

with open("config.yaml", "wb") as f:
    yamlrocks.dump({"name": "app", "port": 8080}, f)
```

For round-trip documents loaded from disk, `dump(doc)` with no target writes
only the files that actually changed, including any split-out includes. See
[round-trip editing](/guides/round-trip/) and [includes](/guides/includes/) for
the `dump_includes` helpers that go with that workflow.

## Async dumping

`dump` has an `async` counterpart, `async_dump`, which writes a file off the
event loop. It takes the same arguments and runs the serialize-and-write in a
worker thread, so a slow disk does not stall an asyncio application:

```python
import asyncio
import yamlrocks

async def main():
    await yamlrocks.async_dump({"name": "app", "port": 8080}, "config.yaml")

asyncio.run(main())

with open("config.yaml") as f:
    f.read()
# 'name: app\nport: 8080\n'
```

### There is no async serializer

`async_dump` exists only because it does file I/O. There is deliberately **no**
`async_dumps` and no `async_to_json`. Unlike parsing (where the native scan
releases the GIL and genuinely runs off the loop thread), serializing must walk
the Python object graph, and that traversal holds the GIL the whole time. Moving
it to a worker thread buys little, because the worker still cannot run in
parallel with the loop while it holds the GIL.

On the rare occasion you do need an in-memory serialize off the loop, wrap the
synchronous call yourself:

```python
import asyncio
import yamlrocks

async def main():
    return await asyncio.to_thread(yamlrocks.dumps, {"name": "app"})

asyncio.run(main())
# b'name: app\n'
```

The same reasoning and workaround apply to JSON export; see
[the JSON guide](/guides/json/#off-the-event-loop). For the load side, where
async genuinely runs off the loop, see
[async loading](/guides/loading/#async-loading-off-the-event-loop).

## See also

- [Loading YAML](/guides/loading/): the reverse direction.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): why some strings get quoted.
- [Round-trip editing](/guides/round-trip/): emit while preserving comments.
- [Custom tags](/guides/tags/): handling application-defined tags.
- [API reference](/reference/api/) and [options](/reference/options/).
