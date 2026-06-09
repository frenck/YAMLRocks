---
title: Options
description: Every OPT_* flag, grouped by what it affects, with runnable examples.
---

Options are integer bit flags. Combine them with
the bitwise-OR operator `|` and pass the result as `option`:

```python
import yamlrocks

yamlrocks.dumps({"b": 1, "a": 2}, option=yamlrocks.OPT_SORT_KEYS | yamlrocks.OPT_INDENT_4)
# b'a: 2\nb: 1\n'
```

A flag that does not apply to a given call is ignored, so it is safe to keep a
single `option` constant and reuse it across `loads` and `dumps`.

## Parsing

These flags change how `loads`, `loads_all`, `load`, and `load_all` interpret a
document.

| Flag                         | Effect                                                                                                                                                                                                                                                                                                                                                            |
| ---------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `OPT_YAML_1_1`               | Use the YAML 1.1 schema: `yes`/`no`/`on`/`off` booleans (and bare `y`/`n`, per the spec), `0777` octals, and sexagesimal numbers.                                                                                                                                                                                                                                 |
| `OPT_PYYAML_COMPAT`          | Use PyYAML's deliberately off-spec boolean set: `yes`/`no`/`on`/`off`/`true`/`false` but **not** bare `y`/`n` (they stay strings). Implies the 1.1 schema; matches the PyYAML ecosystem.                                                                                                                                                                          |
| `OPT_ROUND_TRIP`             | Return a [`YAMLRocksDocument`](/reference/api/#yamlrocksdocument) preserving comments, anchors, and formatting.                                                                                                                                                                                                                                                   |
| `OPT_ANNOTATED`              | Return [`dict`/`list`/`str` subclasses](/reference/api/#yamlrocksannotateddict-yamlrocksannotatedlist-yamlrocksannotatedstr) carrying source line and column.                                                                                                                                                                                                     |
| `OPT_ANNOTATE_NUMBERS`       | With `OPT_ANNOTATED`, also annotate `int`/`float` (as `YAMLRocksAnnotatedInt`/`Float`) so numbers carry a source location. Off by default; makes `type(x) is int` false. `bool`/`None` stay plain.                                                                                                                                                                |
| `OPT_REJECT_COMPLEX_KEYS`    | Reject a collection (sequence or mapping) used as a mapping key, raising [`YAMLRocksComplexKeyError`](/reference/exceptions/) with a location, instead of converting it to a `tuple` (a mapping key becomes a `tuple` of its pairs). Off by default (accept-and-convert is spec-compliant). For scalar-keyed consumers; catches the unquoted-template typo early. |
| `OPT_INCLUDES`               | Resolve the `!include` family of tags. Needs `include_dir`, or use `load()` so includes resolve next to the file.                                                                                                                                                                                                                                                 |
| `OPT_INCLUDE_DIR_RECURSIVE`  | Make the `!include_dir_*` tags descend into subdirectories (`os.walk`-style); off by default (top level only).                                                                                                                                                                                                                                                    |
| `OPT_SECRETS`                | Resolve `!secret name` against a `secrets.yaml`, searching up to the config root. Needs `include_dir`, or use `load()`.                                                                                                                                                                                                                                           |
| `OPT_SECRET_NOT_FOUND_WARN`  | Downgrade an undefined `!secret` from a hard error to a logged WARNING that resolves to `None`, so one pass reports every miss. Off by default (loading stays fail-fast); needs `OPT_SECRETS`. For a structured per-miss callback, see [`on_missing_secret`](/reference/api/#loads).                                                                              |
| `OPT_ENV_VAR`                | Resolve `!env_var NAME [default]` against the process environment.                                                                                                                                                                                                                                                                                                |
| `OPT_ENV_VAR_NOT_FOUND_WARN` | Downgrade a bare undefined `!env_var` (no default) from a hard error to a logged WARNING that resolves to `None`, so one pass reports every miss. Off by default; needs `OPT_ENV_VAR`. For a structured per-miss callback, see [`on_missing_env_var`](/reference/api/#loads).                                                                                     |
| `OPT_UPGRADE_1_1`            | Read as YAML 1.1 but always emit canonical 1.2, a bridge for [easing into 1.2](/guides/yaml-11-vs-12/#easing-into-yaml-12).                                                                                                                                                                                                                                       |
| `OPT_YAML_1_1_WARN`          | Log (on the `yamlrocks` logger) each scalar that resolves to a different type under 1.1 than 1.2, a migration aid. Pair with `OPT_YAML_1_1` or `OPT_UPGRADE_1_1`; a no-op without them.                                                                                                                                                                           |
| `OPT_PASSTHROUGH_TAG`        | Surface custom-tagged values as [`YAMLRocksTag`](/reference/api/#yamlrockstag) objects instead of dropping the tag.                                                                                                                                                                                                                                               |
| `OPT_DUPLICATE_KEYS_ERROR`   | Raise `YAMLRocksDecodeError` on a repeated mapping key (default: last wins). The merge key `<<` is exempt.                                                                                                                                                                                                                                                        |
| `OPT_DUPLICATE_KEYS_WARN`    | Log a repeated mapping key on the `yamlrocks` logger (`WARNING`) but keep the last value. Ignored when `OPT_DUPLICATE_KEYS_ERROR` is also set.                                                                                                                                                                                                                    |

By default `yes` is a plain string in YAML 1.2; `OPT_YAML_1_1` switches to the
older schema where it is a boolean:

```python
import yamlrocks

yamlrocks.loads(b"a: yes")                              # {'a': 'yes'}
yamlrocks.loads(b"a: yes", option=yamlrocks.OPT_YAML_1_1)  # {'a': True}
```

`OPT_DUPLICATE_KEYS_ERROR` rejects a repeated key with the offending location in
the message:

<!-- verify: raises YAMLRocksDecodeError -->

```python
import yamlrocks

source = """
a: 1
a: 2
"""

yamlrocks.loads(source, option=yamlrocks.OPT_DUPLICATE_KEYS_ERROR)
# yamlrocks.YAMLRocksDecodeError: duplicate mapping key: a at line 2, column 1
```

For a non-fatal middle ground, `OPT_DUPLICATE_KEYS_WARN` keeps the last value but
logs each duplicate on the `yamlrocks` logger, so you can surface them through
your existing logging configuration without catching an exception:

```python
import logging
import yamlrocks

source = """
a: 1
a: 2
"""

logging.getLogger("yamlrocks")  # configure/route as you like
yamlrocks.loads(source, option=yamlrocks.OPT_DUPLICATE_KEYS_WARN)
# logs WARNING "duplicate mapping key 'a' ...", returns {'a': 2}
```

When moving a YAML 1.1 codebase to 1.2, `OPT_YAML_1_1_WARN` (alongside
`OPT_YAML_1_1` or `OPT_UPGRADE_1_1`) logs every scalar that the two schemas type
differently, so you can find each `yes`/`no` boolean, `0777` octal, or
sexagesimal that would silently become a plain string under 1.2:

```python
import yamlrocks

source = """
enabled: yes
mask: 0777
"""

yamlrocks.loads(
    source,
    option=yamlrocks.OPT_YAML_1_1 | yamlrocks.OPT_YAML_1_1_WARN,
)
# logs "YAML 1.1 syntax 'yes' resolves as bool in 1.1 but str in 1.2 ..." (and 0777)
```

## Emitting

These flags change how `dumps` and `dump` format output. Indentation defaults to
two spaces.

| Flag                       | Effect                                                         |
| -------------------------- | -------------------------------------------------------------- |
| `OPT_INDENT_2`             | Two-space indentation (the default).                           |
| `OPT_INDENT_4`             | Four-space indentation.                                        |
| `OPT_SORT_KEYS`            | Sort mapping keys alphabetically.                              |
| `OPT_FLOW_STYLE`           | Emit flow style (`{}` / `[]`) instead of block style.          |
| `OPT_SINGLE_QUOTES`        | Use single quotes when quoting, instead of the default double. |
| `OPT_INDENTLESS_SEQUENCES` | Align a sequence's dashes with its key instead of indenting.   |
| `OPT_EXPLICIT_START`       | Emit a `---` document-start marker.                            |
| `OPT_EXPLICIT_END`         | Emit a `...` document-end marker.                              |
| `OPT_NULL_AS_KEYWORD`      | Emit `None` as the explicit `null` keyword instead of blank.   |
| `OPT_NULL_AS_TILDE`        | Emit `None` as the `~` indicator instead of blank.             |

```python
import yamlrocks

yamlrocks.dumps({"a": [1, 2]}, option=yamlrocks.OPT_FLOW_STYLE)
# b'{a: [1, 2]}\n'

yamlrocks.dumps({"a": 1}, option=yamlrocks.OPT_EXPLICIT_START | yamlrocks.OPT_EXPLICIT_END)
# b'---\na: 1\n...\n'
```

### Null style

When `dumps` serializes `None`, it leaves the value **blank** by default
(`key:`), matching the dominant real-world configuration style.
`OPT_NULL_AS_KEYWORD` switches the default to the explicit `null` keyword and
`OPT_NULL_AS_TILDE` to `~`; the two flags are mutually exclusive (setting both
raises `ValueError`). The per-call `null_style` argument of `dumps`/`dump`
overrides the flag for one call. The accepted styles are `"null"`, `"empty"`, and
`"~"`. All three parse back to `None`, so the choice is purely cosmetic.

The blank form is used only where it reads back unambiguously as null: a block
mapping value (`key:`) or a block sequence entry (`-`). At the top level, inside a
flow collection, or as a mapping key it falls back to `null`, so the output is
always valid YAML 1.2. The `~` form is unambiguous everywhere and is used as-is.

```python
import yamlrocks

yamlrocks.dumps({"a": None})
# b'a:\n'

yamlrocks.dumps({"a": None}, null_style="null")
# b'a: null\n'

yamlrocks.dumps({"a": None}, null_style="~")
# b'a: ~\n'

# The argument overrides the flag for one call.
yamlrocks.dumps({"a": None}, option=yamlrocks.OPT_NULL_AS_KEYWORD, null_style="empty")
# b'a:\n'
```

:::note[Round-trip editing]
The same default applies when you edit a round-trip
[`YAMLRocksDocument`](/reference/api/#yamlrocksdocument) and assign `None` to a key: it re-emits as
a blank value (`key:`). `OPT_NULL_AS_KEYWORD` or `OPT_NULL_AS_TILDE` at load time
switches assigned nulls to `null` or `~`. A null that was already in the file
keeps its original form, so an untouched value always re-emits byte-for-byte.

```python
import yamlrocks

doc = yamlrocks.loads(b"x: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
doc["x"] = None
doc.to_yaml()
# b'x:\n'
```

:::

## Type serialization

These flags control how `dumps` renders standard-library values.

| Flag                        | Effect                                                                                             |
| --------------------------- | -------------------------------------------------------------------------------------------------- |
| `OPT_SERIALIZE_NUMPY`       | Serialize numpy arrays and scalars (off by default; without it they raise `YAMLRocksEncodeError`). |
| `OPT_OMIT_MICROSECONDS`     | Drop the microsecond field from `datetime` and `time`.                                             |
| `OPT_NAIVE_UTC`             | Treat a naive `datetime` as UTC, appending a `+00:00` offset.                                      |
| `OPT_UTC_Z`                 | Render a `+00:00` UTC offset as `Z`.                                                               |
| `OPT_PASSTHROUGH_DATETIME`  | Do not auto-serialize `datetime`/`date`/`time`; route them to `default`.                           |
| `OPT_PASSTHROUGH_DATACLASS` | Do not auto-serialize dataclass instances; route them to `default`.                                |

The datetime flags compose. A UTC datetime renders in full ISO 8601 by default;
combine `OPT_OMIT_MICROSECONDS` and `OPT_UTC_Z` for the compact form:

```python
import datetime
import yamlrocks

dt = datetime.datetime(2026, 6, 5, 12, 30, 45, 123456, tzinfo=datetime.timezone.utc)

yamlrocks.dumps(dt)
# b'2026-06-05T12:30:45.123456+00:00\n'

yamlrocks.dumps(dt, option=yamlrocks.OPT_OMIT_MICROSECONDS | yamlrocks.OPT_UTC_Z)
# b'2026-06-05T12:30:45Z\n'
```

The passthrough flags hand a value to your `default` callback instead of using
the built-in renderer, so you can emit a custom format:

```python
import datetime
import yamlrocks

def default(obj):
    if isinstance(obj, datetime.datetime):
        return obj.strftime("%Y/%m/%d")
    raise TypeError

yamlrocks.dumps(
    {"when": datetime.datetime(2026, 6, 5)},
    default=default,
    option=yamlrocks.OPT_PASSTHROUGH_DATETIME,
)
# b'when: 2026/06/05\n'
```

## See also

- [API reference](/reference/api/): the functions these flags configure.
- [Dumping](/guides/dumping/): emit styles and the `default` hook.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): what `OPT_YAML_1_1` changes.
