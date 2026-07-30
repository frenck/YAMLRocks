---
title: JSON import and export
description: Convert between YAML and JSON with to_json and loads, and understand the lossy YAML-to-JSON projection.
---

YAML is a superset of JSON, which makes converting between the two easy.
Importing JSON needs no special function at all, and exporting to JSON is a
single call: `to_json`.

## Importing JSON is just `loads`

Every valid JSON document is also valid YAML 1.2, so `loads` already reads JSON:

```python
import yamlrocks

yamlrocks.loads(b'{"name": "app", "ports": [80, 443], "enabled": true}')
# {'name': 'app', 'ports': [80, 443], 'enabled': True}
```

There is no separate `from_json`; you have already been parsing JSON this whole
time.

## Exporting to JSON with `to_json`

`to_json` is the JSON counterpart of [`dumps`](/guides/dumping/). It takes any
supported Python object and returns JSON **bytes**:

```python
import yamlrocks

yamlrocks.to_json({"name": "app", "ports": [80, 443]})
# b'{"name":"app","ports":[80,443]}'
```

Output is compact by default (no spaces), like a fast JSON writer. It shares
`dumps`'s `default=` callback and the same option flags.

### YAML to JSON, and back

Combine `loads` and `to_json` to convert a YAML document to JSON, and `loads`
and `dumps` to go the other way:

```python
import yamlrocks

# YAML -> JSON
config = """
name: app
ports:
  - 80
  - 443
"""

yamlrocks.to_json(yamlrocks.loads(config))
# b'{"name":"app","ports":[80,443]}'

# JSON -> YAML
yamlrocks.dumps(yamlrocks.loads(b'{"name": "app", "ports": [80, 443]}'))
# b'name: app\nports:\n  - 80\n  - 443\n'
```

## Pretty-printing and sorting

The indent and sort options that apply to `dumps` apply here too:

```python
import yamlrocks

opt = yamlrocks.OPT_INDENT_2 | yamlrocks.OPT_SORT_KEYS
print(yamlrocks.to_json({"b": 2, "a": 1}, option=opt).decode())
# {
#   "a": 1,
#   "b": 2
# }
```

Use `OPT_INDENT_4` for four-space indentation. Without an indent option the
output stays compact.

## Exporting part of a document

With [round-trip mode](/guides/round-trip/), `to_json` accepts a `YAMLRocksDocument` or a
nested view, so you can export the whole document or just a sub-tree:

```python
import yamlrocks

doc = yamlrocks.loads(
    b"service:\n  name: web\n  ports: [80, 443]\nmeta:\n  owner: ops\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

yamlrocks.to_json(doc)  # the whole document
# b'{"service":{"name":"web","ports":[80,443]},"meta":{"owner":"ops"}}'

yamlrocks.to_json(doc["service"])  # just one sub-tree
# b'{"name":"web","ports":[80,443]}'
```

Anchors and aliases are resolved to the values they reference, so the JSON has
no `*alias` placeholders.

An empty round-trip document (for example `loads(b"", option=OPT_ROUND_TRIP)`)
has no root value, so `to_json` returns JSON `null`:

```python
import yamlrocks

empty = yamlrocks.loads(b"", option=yamlrocks.OPT_ROUND_TRIP)
assert yamlrocks.to_json(empty) == b"null"
```

## The YAML-to-JSON projection

JSON is the _lossy subset_ of YAML, so a few YAML features have to be projected
when they have no JSON equivalent. `to_json` does this consistently:

| YAML feature                                     | JSON result                              |
| ------------------------------------------------ | ---------------------------------------- |
| Tags (`!!str`, `!custom`)                        | dropped; the underlying value is emitted |
| `NaN`, `Infinity`, `-Infinity`                   | `null` (not valid JSON numbers)          |
| Non-string scalar key (`1:`, `true:`, `null:`)   | stringified: `"1"`, `"true"`, `"null"`   |
| A collection used as a key (`[a, b]: v`)         | error (no JSON representation)           |
| `datetime`, `uuid`, `Decimal`, `Enum`, sets, ... | the same form `dumps` uses               |

Stringifying non-string keys matches the canonical YAML-to-JSON mapping used by
the official YAML test suite. A collection key is the one thing JSON genuinely
cannot represent, so it raises instead of guessing:

<!-- verify: raises YAMLRocksEncodeError -->

```python
import yamlrocks

# A sequence key has no JSON form.
yamlrocks.to_json({(1, 2): "value"})
# yamlrocks.YAMLRocksEncodeError: a collection cannot be a JSON object key
```

## Off the event loop

There is no `async_to_json` (nor `async_dumps`): serializing holds the GIL while
it walks the Python object, so wrapping it in a thread buys little. On the rare
occasion you need it off the loop, wrap the sync call yourself with
`asyncio.to_thread(yamlrocks.to_json, obj)`. Loading is different: `async_loads`
and `async_load` exist because the native parse runs fully off the GIL.
