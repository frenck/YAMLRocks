---
title: Includes
description: Resolve and write back !include directives natively, with no Python constructor.
---

Large configurations rarely live in a single file. The convention popularized by
Home Assistant is to split a configuration across many small files and stitch them
back together with `!include` tags. YAMLRocks understands these tags natively: there
is no Python constructor to register and no per-file overhead, so a configuration
spread over hundreds of files resolves in one fast pass.

Includes are opt-in. Pass `OPT_INCLUDES` and tell YAMLRocks where to look for the
referenced files. With [`load`](/guides/loading/) you usually do not even have to:
when you load a file and omit `include_dir`, includes resolve relative to that
file's own directory, which is almost always what you want.

## Supported tags

YAMLRocks resolves the native `!include` tag plus four directory-oriented variants:

| Tag                            | Reads               | Produces                      |
| ------------------------------ | ------------------- | ----------------------------- |
| `!include file.yaml`           | one file            | that file's parsed content    |
| `!include_dir_list dir`        | every file in `dir` | a `list`, one entry per file  |
| `!include_dir_named dir`       | every file in `dir` | a `dict` keyed by file stem   |
| `!include_dir_merge_list dir`  | every file in `dir` | the files' lists concatenated |
| `!include_dir_merge_named dir` | every file in `dir` | the files' mappings merged    |

The `_dir_*` tags read every YAML file in the named directory. Use `_list` when
each file contributes one item, `_named` when you want them keyed by filename, and
the `merge_` forms when each file already holds a list or mapping that should be
flattened into one.

Files in a directory are read in sorted filename order. For
`!include_dir_merge_named` that order is also the precedence: if two files define
the same key, the file sorted later wins. This is a cross-file override, so
`OPT_DUPLICATE_KEYS_ERROR` (which guards duplicates inside a single document) does
not turn it into an error. Keep keys unique across a merged directory, or rely on
the documented last-file-wins rule deliberately.

## Reading a split configuration

The example below is fully runnable: it builds a small configuration tree in a
temporary directory and then loads it. In a real project these files already exist
on disk and you simply point `load` at the entry file.

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()

with open(os.path.join(config, "configuration.yaml"), "wb") as handle:
    handle.write(
        b"name: home\n"
        b"automation: !include automations.yaml\n"
        b"sensors: !include_dir_list sensors\n"
    )

with open(os.path.join(config, "automations.yaml"), "wb") as handle:
    handle.write(b"- alias: night\n  trigger: time\n")

os.mkdir(os.path.join(config, "sensors"))
for name, body in (("a.yaml", b"name: A\n"), ("b.yaml", b"name: B\n")):
    with open(os.path.join(config, "sensors", name), "wb") as handle:
        handle.write(body)

# `load` infers include_dir from the file's own directory.
data = yamlrocks.load(
    os.path.join(config, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES,
)

assert data["automation"] == [{"alias": "night", "trigger": "time"}]
assert data["sensors"] == [{"name": "A"}, {"name": "B"}]
```

When you hold the bytes yourself rather than a path (for example, content fetched
over the network), use `loads` and pass `include_dir` explicitly so YAMLRocks knows
where the referenced files live:

```python
raw = open(os.path.join(config, "configuration.yaml"), "rb").read()

data = yamlrocks.loads(raw, option=yamlrocks.OPT_INCLUDES, include_dir=config)
assert data["sensors"] == [{"name": "A"}, {"name": "B"}]
```

:::caution[`loads` needs an explicit `include_dir`]
`loads` only sees bytes, so it cannot guess where includes live. Without
`include_dir` it has no base directory to resolve against. `load` is different: it
knows the source path and defaults `include_dir` to that file's directory.
:::

## The directory variants

The four `_dir_*` tags differ only in how they combine the files they read. The
following self-contained example exercises all of them at once:

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()

with open(os.path.join(config, "configuration.yaml"), "wb") as handle:
    handle.write(
        b"lights: !include_dir_named lights\n"
        b"packages: !include_dir_merge_named packages\n"
        b"rules: !include_dir_merge_list rules\n"
    )

os.mkdir(os.path.join(config, "lights"))
with open(os.path.join(config, "lights", "kitchen.yaml"), "wb") as handle:
    handle.write(b"brightness: 80\n")
with open(os.path.join(config, "lights", "hall.yaml"), "wb") as handle:
    handle.write(b"brightness: 40\n")

os.mkdir(os.path.join(config, "packages"))
with open(os.path.join(config, "packages", "p1.yaml"), "wb") as handle:
    handle.write(b"sensor_a: 1\n")
with open(os.path.join(config, "packages", "p2.yaml"), "wb") as handle:
    handle.write(b"sensor_b: 2\n")

os.mkdir(os.path.join(config, "rules"))
with open(os.path.join(config, "rules", "r1.yaml"), "wb") as handle:
    handle.write(b"- one\n- two\n")
with open(os.path.join(config, "rules", "r2.yaml"), "wb") as handle:
    handle.write(b"- three\n")

data = yamlrocks.load(
    os.path.join(config, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES,
)

# _named keys each file by its stem.
assert data["lights"] == {"kitchen": {"brightness": 80}, "hall": {"brightness": 40}}
# _merge_named folds the per-file mappings into one.
assert data["packages"] == {"sensor_a": 1, "sensor_b": 2}
# _merge_list concatenates the per-file lists.
assert data["rules"] == ["one", "two", "three"]
```

By default the `_dir_*` tags read only the top level of the directory. Add
`OPT_INCLUDE_DIR_RECURSIVE` to descend into subdirectories as well (top level
first, then deeper, each level visited in sorted order):

```python
data = yamlrocks.load(
    os.path.join(config, "configuration.yaml"),
    option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_INCLUDE_DIR_RECURSIVE,
)
```

Either way the walk skips hidden entries (any file or directory whose name begins
with `.`), and when `OPT_SECRETS` is active it also skips a `secrets.yaml` (it is
configuration for the [`!secret`](/guides/tags/) feature, not content to include).

## Merging an included file with `<<`

A [merge key](/guides/loading/#anchors-aliases-and-merge-keys) can take an
`!include` as its value: `<<: !include defaults.yaml` folds the included mapping
into the current one, and locally written keys still win. This is the shared
"defaults" idiom (Home Assistant's packages pattern), and it works because the
merge runs after the include resolves:

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()

with open(os.path.join(config, "defaults.yaml"), "wb") as handle:
    handle.write(b"retries: 3\ntimeout: 30\n")

with open(os.path.join(config, "service.yaml"), "wb") as handle:
    handle.write(b"service:\n  <<: !include defaults.yaml\n  timeout: 60\n")

data = yamlrocks.load(
    os.path.join(config, "service.yaml"),
    option=yamlrocks.OPT_INCLUDES,
)

# The included defaults are merged in; the local `timeout` overrides.
assert data["service"] == {"retries": 3, "timeout": 60}
```

## Editing includes and writing them back

The real power of native includes shows up when you combine `OPT_INCLUDES` with
[`OPT_ROUND_TRIP`](/guides/round-trip/). The returned `YAMLRocksDocument` presents the
configuration as one merged tree you can edit, while quietly remembering which
file each value came from. When you write, YAMLRocks puts each change back into its
own source file, and writes only the files that actually changed.

Each included file keeps its original source, so a file you did not touch is
written back **byte-for-byte**; only a file you actually edited is re-rendered.
Editing one automation never reflows the rest of `automations.yaml`.

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()

with open(os.path.join(config, "configuration.yaml"), "wb") as handle:
    handle.write(b"name: home\nautomation: !include automations.yaml\n")
with open(os.path.join(config, "automations.yaml"), "wb") as handle:
    handle.write(b"- alias: night\n  trigger: time\n")

doc = yamlrocks.load(
    os.path.join(config, "configuration.yaml"),
    option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES,
)

# The root view keeps the directive itself, not the inlined content.
assert doc.to_yaml() == b"name: home\nautomation: !include automations.yaml\n"

# Edit a value that physically lives in automations.yaml.
doc["automation"][0]["trigger"] = "state"
```

With the edit in place, there are three ways to persist it.

`dump_includes` writes the changed files to disk and leaves the rest untouched:

```python
yamlrocks.dump_includes(doc, include_dir=config)

# Only automations.yaml changed; configuration.yaml still holds the directive.
assert open(os.path.join(config, "automations.yaml"), "rb").read() == (
    b"- alias: night\n  trigger: state\n"
)
assert open(os.path.join(config, "configuration.yaml"), "rb").read() == (
    b"name: home\nautomation: !include automations.yaml\n"
)
```

`dump_includes_map` returns a `{path: bytes}` mapping of what _would_ be written,
without touching the disk: ideal for a preview, a dry run, or a diff:

```python
changes = yamlrocks.dump_includes_map(doc)
# {'.../automations.yaml': b'- alias: night\n  trigger: state\n'}

assert any(path.endswith("automations.yaml") for path in changes)
```

Finally, because the document was loaded from a file, `save()` knows its origin
and does the same selective write with no arguments, returning the list of files
it wrote:

```python
written = doc.save()
# ['.../automations.yaml']

assert all(path.endswith("automations.yaml") for path in written)
```

:::tip[Only what changed is written]
All three paths share the same rule: an included file that you did not modify is
never rewritten. That keeps timestamps stable, diffs small, and version control
quiet, even across a configuration split over hundreds of files.
:::

## A note on absolute paths

Real Home Assistant configurations live under a fixed root such as `/config`. The
blocks below show the typical shape against that root. They are marked to skip
execution because that path does not exist in the docs sandbox; the runnable
examples above use a temporary directory to prove the same behavior.

<!-- verify: skip -->

```python
import yamlrocks

# Read a real split configuration.
data = yamlrocks.load("/config/configuration.yaml", option=yamlrocks.OPT_INCLUDES)

# Edit and write back only the changed include files.
doc = yamlrocks.load(
    "/config/configuration.yaml",
    option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_INCLUDES,
)
doc["automation"][0]["trigger"] = "state"
yamlrocks.dump_includes(doc, include_dir="/config")
```

## Performance

Native include resolution is roughly **18x faster** than a PyYAML `!include`
constructor for configurations split across hundreds of files, because the work
happens in Rust without bouncing back into Python for every file. See
[performance](/guides/performance/) for the full benchmarks.

## See also

- [Round-trip editing](/guides/round-trip/): the editing model behind writable includes.
- [Loading YAML](/guides/loading/): the parsing entry points and options.
- [Annotated mode](/guides/annotated/): track which file each node came from.
- [Home Assistant recipe](/recipes/home-assistant/): includes in a real config.
- [API reference](/reference/api/) and [options](/reference/options/).
