---
title: Using YAMLRocks with Home Assistant
description: Load, edit, and save Home Assistant YAML configuration with includes, secrets, and source-tracked errors.
---

Home Assistant configurations are split across many files with `!include`,
secrets, environment variables, and source-tracked errors. YAMLRocks implements
that whole model natively and adds writable includes, so it is a strong fit for
tools that read or edit HA configuration. This recipe walks the full loop: load a
split config, surface good errors, edit one automation, and save only the file
that changed.

## The tags Home Assistant uses

Home Assistant configures itself with `!include`, `!secret`, and `!env_var` (the
same conventions ESPHome and similar tools use). YAMLRocks resolves all of them
natively, matching `annotatedyaml`'s semantics. Each tag reaches outside the
document, so each has its own opt-in flag and is inert until you set it:

| Tag                            | Flag           | Behavior                                                           |
| ------------------------------ | -------------- | ------------------------------------------------------------------ |
| `!include file.yaml`           | `OPT_INCLUDES` | Inline another file                                                |
| `!include_dir_list dir`        | `OPT_INCLUDES` | One list entry per file                                            |
| `!include_dir_merge_list dir`  | `OPT_INCLUDES` | Concatenate the lists from each file                               |
| `!include_dir_named dir`       | `OPT_INCLUDES` | Mapping keyed by file stem                                         |
| `!include_dir_merge_named dir` | `OPT_INCLUDES` | Merge the mappings from each file                                  |
| `!secret name`                 | `OPT_SECRETS`  | Look up `name` in `secrets.yaml` (searching up to the config root) |
| `!env_var NAME [default]`      | `OPT_ENV_VAR`  | Read an environment variable, with an optional default             |

Combine the flags you need with `|`. To resolve a config that uses includes and
secrets, pass `OPT_INCLUDES | OPT_SECRETS`.

:::caution[Each tag needs its own flag]
Without its flag, a tag is treated as an ordinary custom tag rather than
resolved: the tag is dropped and the inner value is kept. The flags are
independent because each crosses a different trust boundary. `!secret` reads a
`secrets.yaml` and `!env_var` reads the process environment, so enabling one does
not enable the others.
:::

## Loading a real configuration

Against a live Home Assistant install you point `load` at `configuration.yaml`.
When loading from a path, `include_dir` defaults to the file's own directory, so
includes and secrets resolve exactly as Home Assistant resolves them:

<!-- verify: skip -->

```python
import yamlrocks

config = yamlrocks.load(
    "/config/configuration.yaml",
    option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
)
```

:::tip[Loading from the event loop]
Inside an async integration, use `await yamlrocks.async_load(...)` instead of
wrapping `load` in `async_add_executor_job`. It moves both the file reads and
the parse off the event loop, and the native core releases the GIL so the loop
keeps running. See [Async](/reference/api/#async).
:::

## A fully runnable example

The block below builds a small Home Assistant-style configuration in a temporary
directory and loads it, so you can run it as-is. It mirrors the real layout:
`configuration.yaml` pulls in `automations.yaml`, and a value is read from
`secrets.yaml`.

```python
import os
import tempfile
import yamlrocks

workdir = tempfile.mkdtemp()

with open(os.path.join(workdir, "configuration.yaml"), "wb") as f:
    f.write(
        b"# Home Assistant configuration\n"
        b"homeassistant:\n"
        b"  name: Home\n"
        b"  latitude: !secret home_latitude\n"
        b"automation: !include automations.yaml\n"
    )

with open(os.path.join(workdir, "automations.yaml"), "wb") as f:
    f.write(
        b"# Automations\n"
        b"- alias: Morning lights\n"
        b"  trigger:\n"
        b"    - platform: time\n"
        b'      at: "07:00:00"\n'
        b"  action:\n"
        b"    - service: light.turn_on\n"
    )

with open(os.path.join(workdir, "secrets.yaml"), "wb") as f:
    f.write(b"home_latitude: 52.3676\n")

config = yamlrocks.load(
    os.path.join(workdir, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
)

config["homeassistant"]["name"]  # 'Home'
config["homeassistant"]["latitude"]  # 52.3676  (resolved from secrets.yaml)
config["automation"][0]["alias"]  # 'Morning lights'
```

The `!include` is inlined and the `!secret` is resolved, just as Home Assistant
would do when starting up. The config reaches two files and a secrets store, so
it asks for both `OPT_INCLUDES` and `OPT_SECRETS`.

## Source locations for error messages

Add `OPT_ANNOTATED` to attach `__line__`, `__column__`, and `__file__` to every
mapping and sequence, so a validation tool can point users at the precise
location of a problem, including which included file it lives in:

```python
annotated = yamlrocks.load(
    os.path.join(workdir, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS | yamlrocks.OPT_ANNOTATED,
)

automations = annotated["automation"]
automations.__line__  # 2  (line within automations.yaml)
automations.__file__  # '.../automations.yaml'
automations[0].__line__  # 2
```

Because `__file__` follows the value across an `!include`, an error message can
read like `automations.yaml, line 2` even though the user started from
`configuration.yaml`. See [annotated mode](/guides/annotated/).

## Catching the unquoted-template typo early

A frequent configuration mistake is an unquoted template that occupies a whole
value:

```yaml
state: { { states('sensor.x') } } # meant as a template; YAML sees a mapping key
```

Because the value starts with `{`, YAML reads it as a flow mapping in key
position (a [complex key](/guides/loading/#complex-keys)), which YAMLRocks accepts
and converts by default, so the mistake only surfaces vaguely later. Add
`OPT_REJECT_COMPLEX_KEYS` to turn it into an immediate, located error that a
config loader can wrap with a "did you forget to quote a template?" hint:

```python
import yamlrocks

opt = (
    yamlrocks.OPT_INCLUDES | yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_REJECT_COMPLEX_KEYS
)
try:
    yamlrocks.loads(b"state: {{ states('sensor.x') }}\n", option=opt)
except yamlrocks.YAMLRocksComplexKeyError as err:
    print(err.line, err.column)  # 1 9
```

`YAMLRocksComplexKeyError` carries `.file`/`.line`/`.column` (and is a
`YAMLRocksDecodeError`, so existing `except` clauses keep working). An _embedded_
template like `name: app_{{ env }}` starts with a normal character, so it is a
plain string and is unaffected. Configs are scalar-keyed, so a complex key is
always a mistake there; this flag makes that explicit.

## Reporting every missing secret at once

A missing `!secret` is a hard error by default, which stops at the first one. For
a setup check (or a UI that surfaces a fixable issue per missing secret), you
usually want _all_ of them in one pass. Pass an `on_missing_secret` callback: it
fires once per undefined secret with `(name, file, line)`, the node resolves to
`None`, and the load continues, so you collect the full list without booting on a
hole:

```python
import os
import tempfile
import yamlrocks

checkdir = tempfile.mkdtemp()
with open(os.path.join(checkdir, "configuration.yaml"), "wb") as f:
    f.write(b"db: !secret db_password\napi: !secret api_token\n")
# note: no secrets.yaml, so both are undefined

missing = []
yamlrocks.load(
    os.path.join(checkdir, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
    on_missing_secret=lambda name, file, line: missing.append((name, line)),
)
# missing == [('db_password', 1), ('api_token', 2)]
```

The callback carries only the name and location, never a resolved value, so it is
exactly the placeholder set a per-secret "repair" needs and leaks nothing. A CLI
that just wants a logged summary can instead set `OPT_SECRET_NOT_FOUND_WARN`,
which logs each miss on the `yamlrocks` logger and continues, with no callback to
write. Both default off, so normal startup stays fail-fast. Only an _undefined_
secret downgrades; a broken `secrets.yaml` still raises. See
[handling a missing secret](/guides/tags/#handling-a-missing-secret).

`!env_var` has the same pair, `on_missing_env_var` and
`OPT_ENV_VAR_NOT_FOUND_WARN`, for a bare variable with no default; the two
callbacks are independent, so a missing secret and a missing variable can become
different repairs.

## Editing and saving, the right way

This is where YAMLRocks shines. Load with round-trip mode, edit a value, and
`save()` writes back **only the file that changed**, preserving comments and the
`!include` directive in `configuration.yaml`:

```python
doc = yamlrocks.load(
    os.path.join(workdir, "configuration.yaml"),
    option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
)

# Rename an automation that lives in automations.yaml.
doc["automation"][0]["alias"] = "Evening lights"

written = doc.save()
# ['.../automations.yaml'] - configuration.yaml is untouched.

written[0].endswith("automations.yaml")  # True
```

The edited value lives in `automations.yaml`, so that is the only file rewritten.
`configuration.yaml` and its `!include automations.yaml` line are left exactly as
they were. This makes UI-driven editing safe: a tool can load the whole resolved
config, let the user change any value, and persist it without rewriting unrelated
files or stripping comments.

:::tip[Preview before writing]
To see what would change without touching disk, use
`yamlrocks.dump_includes_map(doc)`, which returns a `{path: new_bytes}` dict of just
the modified files. `yamlrocks.dump_includes(doc, include_dir=workdir)` performs the
same write as `save()` when you want to target an explicit directory.
:::

## Secrets are never leaked back

In round-trip mode, `!secret`, `!env_var`, and `!include` keep their directive
form when the document is re-emitted, so resolved secret values never end up
written back into a file:

```python
rt = yamlrocks.load(
    os.path.join(workdir, "configuration.yaml"),
    option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES | yamlrocks.OPT_SECRETS,
)
rt.to_yaml()
# the homeassistant.latitude line is still `!secret home_latitude`,
# not the resolved 52.3676
```

## Migrating an annotatedyaml-based tool

Replace the `annotatedyaml` load call with `yamlrocks.load(..., option=OPT_INCLUDES
| OPT_SECRETS | OPT_ANNOTATED)`. The returned `YAMLRocksAnnotatedDict`/`YAMLRocksAnnotatedList` are real
`dict`/`list` subclasses carrying `__line__`, `__column__`, and `__file__`,
equivalent to `NodeDictClass`/`NodeListClass`. String **keys and values** become
`YAMLRocksAnnotatedStr` (the equivalent of `NodeStrClass`), each carrying its own location,
so an error can be attributed to the exact key. Nodes from the top-level file
report the real path you passed to `load()` as their `__file__`.

Numbers can carry locations too: add `OPT_ANNOTATE_NUMBERS` alongside
`OPT_ANNOTATED` and integers and floats come back as `YAMLRocksAnnotatedInt` /
`YAMLRocksAnnotatedFloat` (real `int`/`float` subclasses with the same
`__line__`/`__column__`/`__file__`). It is a separate flag because the wrapper
has a small cost, so you opt in only where a number's location matters.

```python
import yamlrocks

data = yamlrocks.loads(
    b"http:\n  server_port: 8123\n",
    option=yamlrocks.OPT_ANNOTATED | yamlrocks.OPT_ANNOTATE_NUMBERS,
)
data["http"]["server_port"].__line__  # 2
```

The only scalars that stay plain are `bool` and `None`: Python forbids
subclassing `bool`, and `None` is a singleton, so neither can carry attributes.

## See also

- [Includes](/guides/includes/): the full include and write-back model.
- [Annotated mode](/guides/annotated/): `__line__`/`__column__`/`__file__`.
- [Round-trip editing](/guides/round-trip/) and the
  [config editor recipe](/recipes/config-editor/).
- [Custom tags](/guides/tags/): handling tags beyond the HA set.
