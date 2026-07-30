---
title: YAML 1.1 vs 1.2
description: Why YAMLRocks defaults to YAML 1.2, what changes in 1.1 mode, and how to upgrade old documents.
---

YAML has two schema versions in common use, and they disagree about what a bare
scalar like `no` or `0777` means. YAMLRocks follows the **YAML 1.2 core schema** by
default, which is the modern, stricter, and safer choice. You can opt into the
older 1.1 rules per call when you must read documents written for them.

## A quick contrast

The scanner and parser are identical between the two modes; only scalar type
resolution differs. The same input can therefore yield different Python types:

```python
import yamlrocks

yamlrocks.loads(b"enabled: yes")  # {'enabled': 'yes'}
yamlrocks.loads(b"enabled: yes", option=yamlrocks.OPT_YAML_1_1)  # {'enabled': True}
```

In 1.2 the word `yes` is just a string. In 1.1 it is a boolean. That single
difference is the source of a famous class of configuration bugs.

## The Norway problem

The canonical example is a list of country codes. Norway's ISO code is `NO`,
and under YAML 1.1's generous boolean rules `NO` resolves to `False`:

```python
import yamlrocks

countries = """
codes:
  - NO
  - SE
  - NL
"""

yamlrocks.loads(countries)
# {'codes': ['NO', 'SE', 'NL']}

yamlrocks.loads(countries, option=yamlrocks.OPT_YAML_1_1)
# {'codes': [False, 'SE', 'NL']}
```

Under 1.1, Norway silently disappears from the list and becomes a boolean. This
is exactly the kind of corruption that is hard to spot and harder to debug, and
it is why YAML 1.2 narrowed booleans down to just `true` and `false`.

## What changes between the schemas

The differences are all about which bare scalars get special meaning. YAML 1.1
recognizes more "magic" words and number formats; YAML 1.2 keeps almost
everything as a plain string unless it is unambiguously a number, boolean, or
null.

| Input            | YAML 1.2 (default)   | YAML 1.1 (`OPT_YAML_1_1`) |
| ---------------- | -------------------- | ------------------------- |
| `yes` / `no`     | `'yes'` / `'no'`     | `True` / `False`          |
| `on` / `off`     | `'on'` / `'off'`     | `True` / `False`          |
| `true` / `false` | `True` / `False`     | `True` / `False`          |
| `0777`           | `'0777'` (string)    | `511` (octal int)         |
| `0o777`          | `511` (octal int)    | `511` (octal int)         |
| `1:30:00`        | `'1:30:00'` (string) | `5400` (sexagesimal int)  |
| `<<` merge key   | merges               | merges                    |

Two of these are worth seeing directly. The leading-zero octal form behaves very
differently:

```python
import yamlrocks

yamlrocks.loads(b"perm: 0777")  # {'perm': '0777'}
yamlrocks.loads(b"perm: 0777", option=yamlrocks.OPT_YAML_1_1)  # {'perm': 511}
```

So does the sexagesimal (base-60) number form, which 1.1 used for things like
durations:

```python
import yamlrocks

yamlrocks.loads(b"duration: 1:30:00")
# {'duration': '1:30:00'}

yamlrocks.loads(b"duration: 1:30:00", option=yamlrocks.OPT_YAML_1_1)
# {'duration': 5400}
```

The merge key `<<` behaves the same in both modes; it is listed here only
because it is sometimes mistaken for a version difference. It works regardless of
the schema:

```python
import yamlrocks

source = """
base: &b {x: 1}
use:
  <<: *b
  y: 2
"""

yamlrocks.loads(source)
# {'base': {'x': 1}, 'use': {'x': 1, 'y': 2}}
```

## Why 1.2 is the default

YAML 1.2 is the safer default precisely because it does less guessing. Fewer
bare words carry hidden meaning, so a value you wrote as text stays text. The
Norway problem, the surprise octal in `0777`, and accidental sexagesimal numbers
all go away. Modern tooling (including the JSON-compatible side of YAML) expects
1.2 semantics, so it is also the most interoperable choice.

Reach for 1.1 only when you are consuming documents that were authored for it,
such as some older Home Assistant or Ansible configurations that rely on
`yes`/`no` booleans.

## Opting into 1.1

`OPT_YAML_1_1` is a per-call option, so you can mix schemas in the same program:
read legacy files in 1.1 mode while everything else stays on 1.2:

```python
import yamlrocks

yamlrocks.loads(b"feature: on", option=yamlrocks.OPT_YAML_1_1)
# {'feature': True}
```

The flag composes with the other parsing options using `|`, just like any other
flag.

### PyYAML-compatible booleans

There are three scalar schemas in play: **YAML 1.2** (the default), **strict YAML
1.1** (`OPT_YAML_1_1`), and **PyYAML-compat** (`OPT_PYYAML_COMPAT`). The third is
not a fourth set of rules to learn: it is strict YAML 1.1 with exactly one
exception.

`OPT_YAML_1_1` follows the literal YAML 1.1 spec, where bare `y`/`Y`/`n`/`N` are
booleans alongside `yes`/`no`/`on`/`off`. That is spec-correct, but it is a sharp
edge: a `y:` key, common for coordinates, becomes the boolean key `True`, and a
value `n` becomes `False`. PyYAML deliberately drops the single-letter forms (its
resolver lists `y`/`n` as candidates but its regex does not match them), and the
PyYAML-based ecosystem (Home Assistant, ESPHome, Ansible) relies on that.

`OPT_PYYAML_COMPAT` is that one exception: identical to strict 1.1 in every way
(`yes`/`no`/`on`/`off`/`true`/`false` and their case variants are booleans,
`0777` is octal, `1:30` is sexagesimal), **except** bare `y`/`Y`/`n`/`N` stay
strings. If you are reading configs written for the PyYAML ecosystem, this is the
flag you want. It implies the 1.1 schema, so it works on its own, and it carries
through the migration paths above, `OPT_UPGRADE_1_1` and `OPT_YAML_1_1_WARN` both
treat `y`/`n` as strings under it.

```python
import yamlrocks

source = """
y: 2
on: 5
"""

yamlrocks.loads(source, option=yamlrocks.OPT_PYYAML_COMPAT)
# {'y': 2, True: 5}   ('y' stays a string key; 'on' is the boolean True)
```

## The upgrade path

Rather than carrying 1.1 documents forever, you can rewrite them to canonical 1.2
once. `yamlrocks.upgrade` reads a document with the 1.1 schema and emits 1.2 bytes,
turning `yes`/`no` into `true`/`false`, leading-zero octals into plain decimals,
and so on:

```python
import yamlrocks

source = """
enabled: yes
mode: on
"""

yamlrocks.upgrade(source)
# b'%YAML 1.2\n---\nenabled: true\nmode: true\n'

yamlrocks.upgrade(b"perm: 0777\n")
# b'%YAML 1.2\n---\nperm: 511\n'
```

The output opens with a **`%YAML 1.2` version directive**. That single line is
what makes the migration stick: it declares the document as 1.2, so any later
read interprets it with the modern schema, even a read that asks for 1.1. Without
it, a file you upgrade today could be re-coerced tomorrow (more on that in
[Staying upgraded](#staying-upgraded) below). Upgrading an already-stamped
document is idempotent: the directive is never doubled.

By default `upgrade` preserves comments, anchors, and layout, changing only the
scalars that actually differ between the schemas (plus the directive). Pass
`preserve_comments=False` to reformat the document from scratch instead.

:::tip[Migrating a codebase]
`upgrade` is ideal for a one-time migration: run it over your old YAML files,
commit the canonical 1.2 output, and drop `OPT_YAML_1_1` from your loaders. From
then on the safer default protects every read.
:::

## Easing into YAML 1.2

A one-time `upgrade` is not always practical. When you do not control the input,
or you want to keep accepting legacy files while standardizing everything you
write on the modern schema, reach for **`OPT_UPGRADE_1_1`**. It is a persistent
"read 1.1, always write 1.2" mode: set it once on your loader, and every value
you read is interpreted with the 1.1 rules while every value you dump comes out
as canonical 1.2. No per-file conversion, and no manual `upgrade` call.

On the fast path, the upgrade is automatic. Reading with the 1.1 schema turns
`yes` into a real `True` and `0777` into `511`, and `dumps` is always canonical
1.2, so a round trip standardizes the spellings for you:

```python
import yamlrocks

source = """
enabled: yes
mask: 0777
"""

obj = yamlrocks.loads(source, option=yamlrocks.OPT_UPGRADE_1_1)
# {'enabled': True, 'mask': 511}

yamlrocks.dumps(obj)
# b'enabled: true\nmask: 511\n'
```

In round-trip mode the same flag rewrites the scalars in place while keeping the
comments, anchors, and layout untouched, so you can upgrade a file's _spelling_
without reformatting it:

```python
import yamlrocks

source = b"# device settings\nenabled: yes # was on\nmask: 0777\n"

doc = yamlrocks.loads(
    source, option=yamlrocks.OPT_ROUND_TRIP | yamlrocks.OPT_UPGRADE_1_1
)
doc.to_yaml()
# b'%YAML 1.2\n---\n# device settings\nenabled: true # was on\nmask: 511\n'
```

The re-emitted document is stamped with `%YAML 1.2`, and `doc.save()` writes that
stamp back to the file, so the next time your loader reads it the directive takes
over. (The fast `dumps` path above is not stamped: its output is already
canonical 1.2 with no ambiguous tokens, so it reads the same either way. Stamp it
yourself with `upgrade` if you persist it.)

:::note[Why this matters]
This is the gentle on-ramp to YAML 1.2. A project that still receives `yes`/`no`
configurations (older Home Assistant or Ansible setups, for example) can adopt
`OPT_UPGRADE_1_1` to accept that input today while every file it writes back is
already clean 1.2. Once the inputs have caught up, drop the flag and the strict
default takes over. Without it, plain `OPT_ROUND_TRIP` is byte-preserving by
design and would faithfully write the old `yes`/`0777` spellings straight back
out.
:::

### Staying upgraded

A persistent "always read 1.1" loader has a subtle trap: once you have upgraded a
file, reading it again in 1.1 mode would re-interpret it. If a user later edits
the upgraded file and writes `mode: yes` meaning the **string** `"yes"`, a 1.1
read would turn it back into a boolean. The migration would never truly finish.

The `%YAML 1.2` stamp closes that trap, because **a document's own `%YAML`
directive is authoritative**: it selects the schema and overrides the flags. A
stamped file is read as 1.2 even under `OPT_UPGRADE_1_1`, so values added after
the upgrade keep their 1.2 meaning:

```python
import yamlrocks

upgraded = yamlrocks.upgrade(b"enabled: yes\n")
edited = upgraded + b"note: yes\n"  # the user adds a string later

yamlrocks.loads(edited, option=yamlrocks.OPT_UPGRADE_1_1)
# {'enabled': True, 'note': 'yes'}          (note stays a string)
```

The same rule runs in both directions: a document that declares `%YAML 1.1` is
read with the 1.1 schema even by default, because it explicitly said so.

```python
import yamlrocks

yamlrocks.loads(b"%YAML 1.1\n---\nenabled: yes\n")  # {'enabled': True}
```

To check whether a file already carries a declaration, use `yaml_version`, a pure
detector that reads only the stream header (no body parse, no I/O):

```python
import yamlrocks

yamlrocks.yaml_version(b"%YAML 1.2\n---\nx: 1\n")  # '1.2'
yamlrocks.yaml_version(b"x: 1\n")  # None
```

### Finding the 1.1-isms

To audit _where_ a configuration leans on the old schema, add
`OPT_YAML_1_1_WARN` to either `OPT_YAML_1_1` or `OPT_UPGRADE_1_1`. It logs a
message (on the `yamlrocks` logger) for every scalar that the two schemas type
differently, with its line and column, so the diagnostics flow through your
existing logging setup rather than an exception you have to catch:

```python
import yamlrocks

source = """
enabled: yes
mask: 0777
port: 8080
"""

yamlrocks.loads(
    source,
    option=yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_YAML_1_1_WARN,
)
# logs: YAML 1.1 syntax 'yes' resolves as bool in 1.1 but str in 1.2 at line 1, column 10
# logs: YAML 1.1 syntax '0777' resolves as int in 1.1 but str in 1.2 at line 2, column 7
# (port: 8080 is an int in both schemas, so it is not flagged)
```

## See also

- [Loading YAML](/guides/loading/): type resolution and parsing options.
- [Dumping YAML](/guides/dumping/): how YAMLRocks quotes ambiguous strings.
- [Home Assistant recipes](/recipes/home-assistant/): working with 1.1-era configs.
- [Options reference](/reference/options/): every flag in one place.
