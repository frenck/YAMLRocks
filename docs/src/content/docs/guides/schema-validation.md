---
title: Schema validation
description: Validate documents against a JSON Schema, with line-accurate errors.
---

YAMLRocks can validate a document against a [JSON Schema](https://json-schema.org/).
Pass the schema as a Python `dict` to `loads` (or `load`) through the `schema=`
keyword. If the document conforms, you get the parsed value back exactly as
without a schema. If it does not, YAMLRocks raises `YAMLRocksDecodeError` with a
precise source location and a JSON path to the offending node.

Validation runs against the rich syntax tree (the same structure that powers
round-trip mode), so every node still knows its source line and column. That is
how a schema failure can point at an exact `line, column` rather than just
"somewhere in your data".

```python
import yamlrocks

schema = {
    "type": "object",
    "required": ["name", "port"],
    "properties": {
        "name": {"type": "string", "minLength": 1},
        "port": {"type": "integer", "minimum": 1, "maximum": 65535},
        "tags": {"type": "array", "items": {"type": "string"}},
    },
    "additionalProperties": False,
}

source = """
name: app
port: 8080
"""

yamlrocks.loads(source, schema=schema)
# {'name': 'app', 'port': 8080}
```

When a value is out of range, the error names both the JSON path (`$.port`) and
the line and column in the original YAML:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

schema = {
    "type": "object",
    "required": ["name", "port"],
    "properties": {
        "name": {"type": "string", "minLength": 1},
        "port": {"type": "integer", "minimum": 1, "maximum": 65535},
    },
    "additionalProperties": False,
}

source = """
name: app
port: 70000
"""

yamlrocks.loads(source, schema=schema)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: value 70000 is greater than
#                         maximum 65535 at $.port (line 2, column 7)
```

:::tip[Errors carry line numbers because validation runs on the syntax tree]
YAMLRocks validates the syntax tree, where every node keeps the source location
it was parsed from. That is why a schema failure points at `line 2, column 7` and
not just "somewhere in your data". The JSON path (`$.port`, `$.server.host`,
`$[3]`) tells you _which_ node; the line and column tell you _where_ to fix it.
:::

## Nested objects

Schemas nest the same way your data does. A `properties` entry can itself be an
object schema with its own `required` and `properties`:

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {
        "server": {
            "type": "object",
            "required": ["host"],
            "properties": {
                "host": {"type": "string"},
                "port": {"type": "integer", "minimum": 1, "maximum": 65535},
            },
        },
    },
}

source = """
server:
  host: db
  port: 5432
"""

yamlrocks.loads(source, schema=schema)
# {'server': {'host': 'db', 'port': 5432}}
```

A violation deep in the tree reports the full path to it:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {
        "server": {
            "type": "object",
            "properties": {
                "port": {"type": "integer", "minimum": 1},
            },
        },
    },
}

source = """
server:
  port: 0
"""

yamlrocks.loads(source, schema=schema)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: value 0 is less than
#                         minimum 1 at $.server.port (line 2, column 9)
```

## Arrays

Use `items` to validate every element of a sequence against one schema, and
`minItems` / `maxItems` to bound its length:

```python
import yamlrocks

schema = {
    "type": "array",
    "items": {"type": "integer", "minimum": 0},
    "minItems": 1,
    "maxItems": 3,
}

source = """
- 1
- 2
"""

yamlrocks.loads(source, schema=schema)
# [1, 2]
```

When an element fails, the path uses array index notation (`$[1]`):

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

schema = {"type": "array", "items": {"type": "integer", "minimum": 0}}

source = """
- 1
- -5
"""

yamlrocks.loads(source, schema=schema)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: value -5 is less than
#                         minimum 0 at $[1] (line 2, column 3)
```

## Enums and constants

`enum` restricts a value to a fixed set; `const` pins it to exactly one value:

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {
        "level": {"enum": ["debug", "info", "warning", "error"]},
        "version": {"const": 1},
    },
}

source = """
level: info
version: 1
"""

yamlrocks.loads(source, schema=schema)
# {'level': 'info', 'version': 1}
```

A value outside the enum is rejected at its exact location:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {"level": {"enum": ["debug", "info", "warning", "error"]}},
}

yamlrocks.loads(b"level: verbose\n", schema=schema)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: value is not one of the
#                         allowed enum values at $.level (line 1, column 8)
```

## Combinators

`allOf`, `anyOf`, `oneOf`, and `not` compose smaller schemas. A common pattern is
"this field is either a string or an integer":

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {
        "id": {"anyOf": [{"type": "string"}, {"type": "integer"}]},
    },
}

yamlrocks.loads(b"id: 7\n", schema=schema)        # {'id': 7}
yamlrocks.loads(b"id: abc123\n", schema=schema)   # {'id': 'abc123'}
```

If the value matches none of the branches, validation fails:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

schema = {"anyOf": [{"type": "string"}, {"type": "integer"}]}

yamlrocks.loads(b"3.14", schema=schema)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: value does not match any of
#                         the anyOf schemas at $ (line 1, column 1)
```

## Patterns

`pattern` constrains a string with a regular expression, and `patternProperties`
applies a schema to every key that matches a pattern. The regex engine runs in
guaranteed linear time, so an untrusted schema pattern cannot stall the
validator, and an invalid pattern is reported as a schema error rather than
silently skipped.

```python
import yamlrocks

schema = {
    "type": "object",
    "properties": {"name": {"type": "string", "pattern": "^[a-z][a-z0-9-]*$"}},
    "patternProperties": {"^port_": {"type": "integer"}},
}

source = """
name: web-app
port_http: 80
port_https: 443
"""

yamlrocks.loads(source, schema=schema)
# {'name': 'web-app', 'port_http': 80, 'port_https': 443}
```

A property name is always treated as a string, so `propertyNames` and a
`patternProperties` key match a numeric-looking key like `123` as the text
`"123"`.

## Supported keywords

YAMLRocks implements a practical, draft-7-ish subset of JSON Schema, enough to
express the constraints configuration files actually need, without pulling in a
full validator. The supported keywords are:

| Group       | Keywords                                                                                                                                                     |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Types       | `type` (`null`, `boolean`, `integer`, `number`, `string`, `array`, `object`; a whole-number float counts as `integer`)                                       |
| Values      | `enum`, `const`                                                                                                                                              |
| Objects     | `properties`, `patternProperties`, `required`, `additionalProperties` (boolean or schema), `propertyNames`, `minProperties`, `maxProperties`, `dependencies` |
| Arrays      | `items` (single schema or the draft-7 tuple form), `additionalItems`, `minItems`, `maxItems`, `contains`, `uniqueItems`                                      |
| Numbers     | `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`                                                                                   |
| Strings     | `minLength`, `maxLength`, `pattern`                                                                                                                          |
| Combinators | `allOf`, `anyOf`, `oneOf`, `not`                                                                                                                             |
| References  | `$ref` (local `#/...` pointers, including `#/$defs/...`)                                                                                                     |

:::note[Unknown keywords are ignored]
This is a deliberately small subset. Keywords YAMLRocks does not implement (such as
`format`, or `if`/`then`/`else`) are skipped rather than rejected, so a richer
schema written for another validator still works here; it just validates against
the keywords listed above. If you need a feature that is missing, validate with a
dedicated JSON Schema library after `loads` returns.
:::

### Known limits of the built-in validator

The validator is tuned for the scalar-and-shape constraints configuration files
actually use. Three boundaries are worth knowing, and all three are reasons to
reach for a dedicated JSON Schema library when you need them:

- **`$ref` resolves local pointers only.** A `$ref` into the same schema
  (`#/$defs/...`, `#/definitions/...`, or any `#/`-path) is resolved and
  validated. A remote reference (an external URL) is not fetched; it is reported
  as an unresolvable `$ref` rather than silently passing.
- **Object rules apply to scalar keys.** `properties`, `required`, and
  `additionalProperties` match string keys. A YAML _collection key_
  (`[a, b]: ...`) is not a JSON object key and is not subject to these rules, so
  it neither satisfies `required` nor trips `additionalProperties: false`.
- **The first error is reported.** Validation stops at the first violation and
  raises it with its path, line, and column. It does not accumulate every problem
  in one pass, so fixing one error may reveal the next on the following run.

## In-file schema references

Editors such as VS Code (through the
[`yaml-language-server`](https://github.com/redhat-developer/yaml-language-server)
extension) let a document declare its own schema with a comment, conventionally
on the first line:

```yaml
# yaml-language-server: $schema=https://example.com/config.schema.json
name: app
port: 8080
```

YAMLRocks recognizes this directive, but treats detecting it and acting on it as
two separate steps, on purpose.

### Detecting the reference

`schema_ref` reads the leading comment block and returns the declared reference,
or `None` if the document does not declare one. It only inspects comments at the
top of the file; it never parses the body and never performs any I/O, so it is
always cheap and safe to call:

```python
import yamlrocks

doc = b"# yaml-language-server: $schema=https://example.com/config.schema.json\nport: 8080\n"

yamlrocks.schema_ref(doc)
# 'https://example.com/config.schema.json'

yamlrocks.schema_ref(b"port: 8080\n")
# None
```

### Validating against the declared schema

:::caution[YAMLRocks never fetches the reference for you]
A schema reference is usually a URL. Fetching arbitrary URLs at parse time would
mean surprise network I/O, unpredictable latency, and a server-side request
forgery (SSRF) risk, so YAMLRocks does **not** do it. Resolving a reference to
an actual schema is always under your control.
:::

To validate against the in-file reference, pass `schema="auto"` together with a
`schema_resolver`, a callable that receives the reference string and returns a
schema `dict` (or `None` to decline). YAMLRocks detects the directive, calls
your resolver, and validates against whatever it returns. If there is no
directive, or the resolver returns `None`, validation is skipped and the parsed
value is returned as usual.

```python
import yamlrocks

# A real resolver might read from a local cache, a bundled file, or an
# allow-listed fetch. Here we just map known references to schemas.
SCHEMAS = {
    "https://example.com/config.schema.json": {
        "type": "object",
        "required": ["name", "port"],
        "properties": {
            "name": {"type": "string"},
            "port": {"type": "integer", "minimum": 1, "maximum": 65535},
        },
    },
}

def resolve(ref):
    return SCHEMAS.get(ref)

doc = b"# yaml-language-server: $schema=https://example.com/config.schema.json\nname: app\nport: 8080\n"

yamlrocks.loads(doc, schema="auto", schema_resolver=resolve)
# {'name': 'app', 'port': 8080}
```

A document that declares a schema and violates it fails exactly like the
explicit `schema=` path, with a line-accurate error:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

SCHEMAS = {
    "https://example.com/config.schema.json": {
        "type": "object",
        "properties": {"port": {"type": "integer"}},
    },
}

doc = b"# yaml-language-server: $schema=https://example.com/config.schema.json\nport: not-a-number\n"

yamlrocks.loads(doc, schema="auto", schema_resolver=SCHEMAS.get)
# yamlrocks.YAMLRocksDecodeError: schema validation failed: expected type integer,
#                         found string at $.port (line 2, column 7)
```

This keeps the network decision where it belongs: in your hands. A resolver can
consult a local cache, load a schema bundled with your application, or perform a
fetch restricted to hosts you trust.

## See also

- [Loading YAML](/guides/loading/): the parsing entry points `schema=` plugs into.
- [Exceptions](/reference/exceptions/): the full `YAMLRocksDecodeError` model.
- [Annotated mode](/guides/annotated/): keep source locations on every node.
- [API reference](/reference/api/) and [options](/reference/options/).
