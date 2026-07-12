---
title: Custom tags
description: Handle application-defined YAML tags with the default behavior, tag_handler, or OPT_PASSTHROUGH_TAG.
---

A YAML tag is the `!something` prefix that annotates a node with a type. Some
tags are part of the YAML core schema (`!!str`, `!!int`), but applications also
define their own (`!vec`, `!secret`, `!include`) to carry domain meaning.
YAMLRocks gives you three ways to deal with application tags, from "ignore them" to
"interpret them yourself" to "hand them back untouched".

## The default: drop the tag, keep the value

By default an unrecognized application tag is dropped and the underlying value is
returned as-is. This is the most forgiving behavior, and it means a document with
custom tags still parses into ordinary Python objects:

```python
import yamlrocks

yamlrocks.loads(b"x: !custom foo")
# {'x': 'foo'}
```

The tag `!custom` is discarded; the scalar `foo` survives as a plain string.

## Registering tags by name

When you know which tags you want to handle, register each one by name. Pass a
`tags` mapping from a tag to a function that receives the tag's value and returns
whatever should take its place:

```python
import yamlrocks

yamlrocks.loads(b"point: !vec [1, 2]", tags={"!vec": tuple})
# {'point': (1, 2)}
```

Each function is called with the already-resolved inner value only (the name is
implied by which key matched), so plain builtins drop straight in:

```python
import yamlrocks

yamlrocks.loads(b"name: !upper hello", tags={"!upper": str.upper})
# {'name': 'HELLO'}
```

For a registry you build up across a module, `yamlrocks.YAMLRocksTags` is a `dict`
subclass that adds a `register` decorator:

```python
import yamlrocks

tags = yamlrocks.YAMLRocksTags()

@tags.register("!vec")
def make_vec(value):
    return tuple(value)

yamlrocks.loads(b"point: !vec [1, 2]", tags=tags)
# {'point': (1, 2)}
```

`tags.register("!name", func)` works as a plain call too. Because `YAMLRocksTags` is just
a `dict`, you can reuse one instance across many `loads` calls, merge two with
`update`, or inspect it like any mapping. A tag that is not registered keeps the
default behavior: the tag is dropped and the value kept.

## `tag_handler`: interpret tags yourself

Where the registry dispatches known tags by name, `tag_handler` is the catch-all
for dynamic handling or unknown tags. Pass a `tag_handler(tag, value)` callback
to take control. YAMLRocks calls it for each application-tagged node, and
whatever you return is inserted into the result:

```python
import yamlrocks

yamlrocks.loads(
    b"value: !double 5",
    tag_handler=lambda tag, value: int(value) * 2 if tag == "!double" else value,
)
# {'value': 10}
```

The handler receives the tag string and the node's already-resolved inner value,
so nested tags are handled inside-out. You can use it to build real objects from
tagged data:

```python
import yamlrocks

source = """
point: !vec
  - 1
  - 2
"""

def handler(tag, value):
    if tag == "!vec":
        return tuple(value)
    return value

yamlrocks.loads(source, tag_handler=handler)
# {'point': (1, 2)}
```

A common pattern is to leave unknown tags as-is by returning the value unchanged,
and only act on the tags you recognize, as both examples above do with their
`else` branch.

:::note[Core tags are resolved before the handler]
The `tag_handler` is only called for application tags. Core-schema tags such as
`!!str` and `!!int` are resolved by the parser itself and never reach your
callback (see [forcing a type](#forcing-a-type-with-core-tags) below).
:::

## `OPT_PASSTHROUGH_TAG`: get `YAMLRocksTag` objects back

When you want to preserve the tag and value without committing to an
interpretation, use `OPT_PASSTHROUGH_TAG`. Each application-tagged node comes
back as a `YAMLRocksTag` object with `.tag` and `.value` attributes:

```python
import yamlrocks

result = yamlrocks.loads(b"x: !custom 5", option=yamlrocks.OPT_PASSTHROUGH_TAG)

tag = result["x"]
tag.tag      # '!custom'
tag.value    # '5'
```

This is ideal for round-tripping or for deferring the decision: you can inspect
`tag.tag`, branch on it, and convert later. You can also construct a `YAMLRocksTag`
yourself with `YAMLRocksTag(tag, value)` when building data to emit:

```python
import yamlrocks

t = yamlrocks.YAMLRocksTag("!custom", 5)
t.tag        # '!custom'
t.value      # 5
```

:::note[How the mechanisms combine]
A tagged node is resolved in a fixed order: a matching entry in `tags` wins
first, then a `tag_handler` catch-all, then `OPT_PASSTHROUGH_TAG` wrapping, and
finally the default of dropping the tag. So you can register the common tags by
name and still pass a `tag_handler` to cover everything else, or fall back to
`YAMLRocksTag` objects for tags you did not register.
:::

## Emitting custom tags

`dumps` is the write-side mirror of the read side: a `YAMLRocksTag` serializes
straight back to `!tag value`, preserving the tag and its value. It re-emits
through the normal emitter, though, so it does not reproduce the original
source byte-for-byte: quoting, comments, and spacing follow the emitter's rules
(`x: !custom "foo"` comes back as `x: !custom foo`). When you need byte-for-byte
fidelity, load with `OPT_ROUND_TRIP` instead.

```python
import yamlrocks

yamlrocks.dumps({"x": yamlrocks.YAMLRocksTag("!input", "foo")})
# b'x: !input foo\n'
```

The inner value is serialized with the normal rules, so it can be more than a
scalar. A collection drops to an indented block under the tag, and a multi-line
string becomes a tagged block scalar:

```python
import yamlrocks

yamlrocks.dumps({"opts": yamlrocks.YAMLRocksTag("!extend", {"a": 1, "b": 2})})
# b'opts: !extend\n  a: 1\n  b: 2\n'
```

### Emitting your own types as tags

When you hold your own objects (not `YAMLRocksTag` instances) and want them to
emit as a tag, pass a `serializers` registry to `dumps`. It maps a **Python
type** to a callable that returns a `YAMLRocksTag` (or a `(tag, value)` tuple).
It is the write-side mirror of the load-side `tags={"!tag": func}` registry, but
keyed the other way round: on load you dispatch on the YAML tag, on dump you
dispatch on the Python type.

```python
import yamlrocks

class Input:
    def __init__(self, name):
        self.name = name

yamlrocks.dumps(
    {"brightness": Input("kitchen")},
    serializers={Input: lambda o: yamlrocks.YAMLRocksTag("!input", o.name)},
)
# b'brightness: !input kitchen\n'
```

The registry is matched by **exact type** and is consulted before a dataclass
would otherwise be auto-serialized to a mapping, so a registered type always
wins. A YAML node carries exactly one tag, so a result whose inner value is
itself tagged (a `YAMLRocksTag` wrapping a `YAMLRocksTag`, or a serializer
firing on the inner value too) raises rather than emitting a double-tagged
scalar no parser accepts. Pair the registry with the load-side `tags` to
round-trip a custom type cleanly:

```python
import yamlrocks

class Input:
    def __init__(self, name):
        self.name = name

    def __repr__(self):
        return f"Input({self.name!r})"

out = yamlrocks.dumps(
    {"brightness": Input("kitchen")},
    serializers={Input: lambda o: yamlrocks.YAMLRocksTag("!input", o.name)},
)
yamlrocks.loads(out, tags={"!input": lambda v: Input(str(v))})
# {'brightness': Input('kitchen')}
```

:::note[`to_json` drops tags]
A tag has no JSON equivalent, so `to_json` emits the inner value and discards the
tag. Only `dumps`/`dump` preserve it.
:::

## Forcing a type with core tags

The YAML core schema defines tags that force a scalar's type regardless of how it
looks. These are resolved by the parser, so they work without any handler or
flag. `!!str` forces a value to a string, and `!!int` forces it to an integer:

```python
import yamlrocks

yamlrocks.loads(b"v: !!str 42")
# {'v': '42'}

yamlrocks.loads(b'v: !!int "42"')
# {'v': 42}
```

In the first case the number-looking `42` is kept as the string `'42'`; in the
second the quoted `"42"` is coerced back to the integer `42`. This is the YAML
way to override the default type resolution described in
[Loading YAML](/guides/loading/).

## Config tags: includes, secrets, and environment variables

YAMLRocks has first-class support for the configuration tags popularized by tools
like Home Assistant and ESPHome (they are conventions, not specific to any one
project). Each of these tags reaches outside the document (to other files, a
secrets store, or the process environment), so each has its own opt-in flag and
is inert until you set it:

| Tag                       | Flag           | Behavior                                                                       |
| ------------------------- | -------------- | ------------------------------------------------------------------------------ |
| `!include` family         | `OPT_INCLUDES` | Splits a configuration across files; covered in [includes](/guides/includes/). |
| `!secret name`            | `OPT_SECRETS`  | Looks `name` up in `secrets.yaml`, searching up to the config root.            |
| `!env_var NAME [default]` | `OPT_ENV_VAR`  | Reads an environment variable, with an optional default.                       |

The flags are independent, because each crosses a different trust boundary:
`!secret` reads a `secrets.yaml`, `!env_var` reads the process environment.
Enable only what a given document is allowed to reach, and combine them with `|`.

```python
import os
import yamlrocks

os.environ["API_TOKEN"] = "abc123"

yamlrocks.loads(b"token: !env_var API_TOKEN", option=yamlrocks.OPT_ENV_VAR)
# {'token': 'abc123'}
```

Without its flag, the tag is just an application tag and follows the default
behavior from the top of this page: the tag is dropped and the inner value is
kept:

```python
import yamlrocks

yamlrocks.loads(b"token: !env_var API_TOKEN")
# {'token': 'API_TOKEN'}
```

If you would rather resolve a secret or variable yourself (against an in-memory
store, say), leave the flag off and use a `tag_handler`, exactly as with any
other application tag:

```python
import yamlrocks

secrets = {"db_password": "hunter2"}

yamlrocks.loads(
    b"password: !secret db_password",
    tag_handler=lambda tag, value: secrets.get(value, value)
    if tag == "!secret"
    else value,
)
# {'password': 'hunter2'}
```

See the [Home Assistant recipes](/recipes/home-assistant/) for a fuller example
that wires secrets, environment variables, and includes together.

### Handling a missing secret

By default a `!secret` that names something no `secrets.yaml` defines is a hard
error (`YAMLRocksSecretNotFoundError`), and resolution stops there. That is the
right behavior for real loading: never run with a hole where a secret belongs. It
does mean a config with several undefined secrets takes one fix-and-reload cycle
per secret, which is awkward for a validation tool that wants to list them all.

Two opt-ins downgrade an _undefined_ secret to a collected, non-fatal event so a
single pass finds them all. Each resolves the missing node to `None` and
continues. (A structurally broken `secrets.yaml`, malformed, not a mapping, or
itself containing a `!secret`, still raises; that is an environment fault, not a
user omission.)

The **`on_missing_secret` callback** is invoked once per undefined secret as
`(name, file, line)`. It is observe-only (its return value is ignored); the caller
collects the misses and decides what to do, which is the clean way to drive, say,
a per-secret UI repair:

```python
import yamlrocks

source = """
a: !secret one
b: !secret two
"""

missing = []
data = yamlrocks.loads(
    source,
    option=yamlrocks.OPT_SECRETS,
    include_dir=".",
    on_missing_secret=lambda name, file, line: missing.append((name, line)),
)
# data    == {'a': None, 'b': None}
# missing == [('one', 1), ('two', 2)]
```

The **`OPT_SECRET_NOT_FOUND_WARN` flag** is the zero-code convenience: instead of a
callback, each miss is logged as a `WARNING` on the `yamlrocks` logger (same
channel as `OPT_DUPLICATE_KEYS_WARN`) and resolution continues. Reach for it in a
CLI that just wants a one-pass report; reach for the callback when you need the
misses structured. They compose if you set both.

```python
import yamlrocks

source = """
a: !secret one
b: !secret two
"""

data = yamlrocks.loads(
    source,
    option=yamlrocks.OPT_SECRETS | yamlrocks.OPT_SECRET_NOT_FOUND_WARN,
    include_dir=".",
)
# data == {'a': None, 'b': None}
# logs: secret 'one' is not defined in any secrets.yaml at <file>:1
# logs: secret 'two' is not defined in any secrets.yaml at <file>:2
```

Both default off, so unless you opt in, a missing secret still fails fast.

### Handling a missing environment variable

`!env_var` has the same pair, for the same reason. By default a _bare_ `!env_var
NAME` whose variable is unset raises `YAMLRocksEnvVarError` (a variable that
supplies a default, `!env_var NAME fallback`, just uses the default and is never a
miss). The `on_missing_env_var` callback and the `OPT_ENV_VAR_NOT_FOUND_WARN` flag
downgrade the bare-and-unset case the same way secrets are downgraded, resolving
the node to `None` and collecting every miss in one pass:

```python
import yamlrocks

source = """
host: !env_var DB_HOST
port: !env_var DB_PORT 5432
"""

missing = []
data = yamlrocks.loads(
    source,
    option=yamlrocks.OPT_ENV_VAR,
    on_missing_env_var=lambda name, file, line: missing.append((name, line)),
)
# data    == {'host': None, 'port': '5432'}   (port used its default)
# missing == [('DB_HOST', 1)]
```

The secret and env-var callbacks are independent (each fires only for its own
tag), matching the separate `OPT_SECRETS` and `OPT_ENV_VAR` flags, so a loader can
treat a missing secret and a missing variable differently.

## See also

- [Loading YAML](/guides/loading/): default type resolution.
- [Includes](/guides/includes/): the `!include` family of tags.
- [Home Assistant recipes](/recipes/home-assistant/): tags in a real config.
- [API reference](/reference/api/) and [options](/reference/options/).
