---
title: Converting between YAML and JSON
description: Convert YAML to JSON and JSON to YAML in Python, including multi-document streams, big integers, and stable diffs, with YAMLRocks.
---

YAML is a superset of JSON, so converting between the two is a common chore: feed
a YAML config to a tool that only speaks JSON, or pretty-print a JSON blob as
readable YAML. YAMLRocks does both with one call in each direction, safely and
fast, and it keeps the things a naive `json` round-trip drops: key order, big
integers, and multi-document streams. This recipe is the practical loop; the
[JSON guide](/guides/json/) is the reference behind it.

## YAML to JSON

Read the YAML with `loads`, write JSON with `to_json`. Both speak `bytes`, so
there is no extra encode step before you write to a file or socket:

```python
import yamlrocks

config = b"name: app\nports:\n  - 80\n  - 443\nenabled: true\n"

yamlrocks.to_json(yamlrocks.loads(config))
# b'{"name":"app","ports":[80,443],"enabled":true}'
```

The output is compact by default, like a fast JSON writer. Note `enabled: true`
became JSON `true`, not the string `"true"`: YAMLRocks defaults to YAML 1.2, so
the booleans line up with JSON's without the [Norway
problem](/guides/yaml-11-vs-12/).

## JSON to YAML

The reverse is `loads` (JSON is valid YAML, so no separate parser) then `dumps`:

```python
import yamlrocks

yamlrocks.dumps(yamlrocks.loads(b'{"name":"app","ports":[80,443]}'))
# b'name: app\nports:\n  - 80\n  - 443\n'
```

There is no `from_json`. Every valid JSON document is valid YAML 1.2, so `loads`
already reads it.

## Stable, diff-friendly JSON

For JSON you commit or compare in review, compact output is noisy. Add indentation
and sort the keys, so the same data always serializes to the same bytes and diffs
stay small:

```python
import yamlrocks

opt = yamlrocks.OPT_INDENT_2 | yamlrocks.OPT_SORT_KEYS
print(yamlrocks.to_json(yamlrocks.loads(b"name: app\nport: 8080\n"), option=opt).decode())
# {
#   "name": "app",
#   "port": 8080
# }
```

Use `OPT_INDENT_4` for four-space indentation. Without an indent option the output
stays compact. Leave out `OPT_SORT_KEYS` to keep the document's own key order,
which YAMLRocks preserves end to end.

## Multi-document YAML to a JSON array

A YAML stream can hold several documents separated by `---`. JSON has no such
separator, so the natural projection is a single JSON array. Read the stream with
`loads_all` and hand the list straight to `to_json`:

```python
import yamlrocks

stream = b"---\nname: a\n---\nname: b\n"

docs = yamlrocks.loads_all(stream)
docs
# [{'name': 'a'}, {'name': 'b'}]

yamlrocks.to_json(docs)
# b'[{"name":"a"},{"name":"b"}]'
```

## Big integers survive the trip

Python's standard `json` module handles arbitrary-precision integers, but many
converters route through a type that clamps to 64 bits. YAMLRocks carries big
integers through both YAML and JSON exactly:

```python
import yamlrocks

big = 2**70
yamlrocks.to_json({"n": big})
# b'{"n":1180591620717411303424}'

yamlrocks.loads(yamlrocks.to_json({"n": big}))["n"] == big
# True
```

## Values JSON cannot hold

A few YAML values have no JSON equivalent, so `to_json` projects them consistently
rather than guessing. `NaN` and the infinities are not valid JSON numbers, so they
become `null`:

```python
import yamlrocks

yamlrocks.to_json({"x": float("nan"), "y": float("inf")})
# b'{"x":null,"y":null}'
```

Tags are dropped to their underlying value, non-string scalar keys are stringified
(`1` becomes `"1"`), and a collection used as a key raises, because JSON genuinely
cannot represent it. The full table lives in the
[JSON guide](/guides/json/#the-yaml-to-json-projection).

## A tiny yaml2json command

Put the pieces together and you have a converter that reads YAML on stdin and
writes JSON on stdout, ready to drop into a shell pipeline. It touches the process
streams, so it carries a skip marker for the docs verifier:

<!-- verify: skip -->

```python
#!/usr/bin/env python3
"""Read YAML on stdin, write pretty, sorted JSON on stdout."""
import sys
import yamlrocks

opt = yamlrocks.OPT_INDENT_2 | yamlrocks.OPT_SORT_KEYS
data = yamlrocks.loads(sys.stdin.buffer.read())
sys.stdout.buffer.write(yamlrocks.to_json(data, option=opt))
```

Run it with `cat config.yaml | python yaml2json.py`. Swap `to_json` for `dumps`
and drop the sort to make a `json2yaml` in the same shape.

## Keeping the YAML when you convert

Plain `loads` gives you data, not the document, so comments and layout are gone.
That is exactly what you want when the destination is JSON. If instead you convert
to JSON to inspect part of a config but keep editing the YAML, load once in
[round-trip mode](/guides/round-trip/): `to_json` accepts a `YAMLRocksDocument` or a
nested view, so you can export a sub-tree while the original stays intact for
editing.

```python
import yamlrocks

doc = yamlrocks.loads(
    b"service:\n  name: web\n  ports: [80, 443]\nmeta:\n  owner: ops\n",
    option=yamlrocks.OPT_ROUND_TRIP,
)

yamlrocks.to_json(doc["service"])   # just one sub-tree, as JSON
# b'{"name":"web","ports":[80,443]}'

doc.to_yaml()                       # the YAML is untouched, comments and all
# b'service:\n  name: web\n  ports: [80, 443]\nmeta:\n  owner: ops\n'
```

## See also

- [JSON import and export](/guides/json/): the full `to_json` reference and the
  YAML-to-JSON projection rules.
- [Loading YAML](/guides/loading/) and [Dumping YAML](/guides/dumping/): the
  options that shape both directions.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): why booleans and `null` line up with
  JSON by default.
- [Building a config editor](/recipes/config-editor/): when you convert to inspect
  but keep editing the YAML.
