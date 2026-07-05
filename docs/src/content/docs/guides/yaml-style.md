---
title: YAML style guide
description: The readable, consistent YAML style YAMLRocks recommends and emits, aligned with the Home Assistant YAML style guide.
---

YAML gives you many ways to write the same thing. A consistent style keeps
configurations readable, reviewable, and approachable, which matters most when
the people editing them are not YAML experts. YAMLRocks recommends the style
below and emits it by default wherever it can; the few opinionated choices are a
single option away.

It is the same style used across Home Assistant, documented in full in the
[Home Assistant YAML style guide](https://developers.home-assistant.io/docs/documenting/yaml-style-guide/).
That guide covers documentation and configuration conventions in depth; this page
describes how the formatting rules map onto YAMLRocks.

:::tip[The short version]
Two-space indentation, block style throughout, lowercase `true`/`false`, empty
values left blank, and strings quoted only when they need it. This is exactly what
YAMLRocks produces by default.
:::

## Indentation

Indent with **two spaces** per level. Never use tabs, and keep every level a
consistent two spaces deeper than its parent.

```yaml
boulder:
  grade: V4
  name: Sunlit Slab
```

YAMLRocks emits two-space indentation by default. `OPT_INDENT_4` is available if
a project insists on four, but two is the recommended width.

## Sequences

Write lists in **block style**, with each item on its own line under the key it
belongs to, indented one level:

```yaml
# Good
climbing_rack:
  - cams
  - nuts
  - quickdraws
```

Flow style (`[1, 2, 3]`) is harder to scan and is discouraged. When it genuinely
helps, for instance a short list of numbers, put a space after each comma and no
padding inside the brackets: `grades: [5, 8, 11]`.

YAMLRocks emits block sequences indented under their key by default, and uses the
spaced flow form when you ask for `OPT_FLOW_STYLE`. If you instead prefer the
"indentless" style that aligns the dashes with the key (`key:` then `- item`),
common in the Kubernetes ecosystem, pass `OPT_INDENTLESS_SEQUENCES`.

## Mappings

Use **block style** mappings only. The flow form that looks like JSON,
`{ name: basalt, igneous: true }`, is not used.

```yaml
# Good
specimen:
  name: Basalt
  origin: Iceland
```

Block mappings are the YAMLRocks default; flow mappings appear only under
`OPT_FLOW_STYLE`.

## Booleans

The only boolean spellings are lowercase **`true`** and **`false`**. Avoid the
YAML 1.1 truthy words such as `yes`, `no`, `on`, and `off`, and avoid
capitalized forms like `True`.

```yaml
# Good
polished: true
fossil: false
```

YAMLRocks emits `true`/`false` for Python booleans. On the reading side it
follows YAML 1.2, so `yes`, `no`, `on`, and `off` load as plain strings rather
than booleans, which removes a whole class of surprises. (Opt into the 1.1
reading with `OPT_YAML_1_1` only if you must.)

## Null values

Leave an empty value **blank**. Avoid the `~` spelling, and reach for the explicit
`null` keyword only when a format genuinely calls for it. This is not just a
convention: across thousands of real-world configuration files the blank form is
by far the most common, so it is what YAMLRocks emits by default.

```yaml
# Good
nickname:
locality:
```

Because it is the default, `dumps` produces it with no options at all:

```python
import yamlrocks

yamlrocks.dumps({"nickname": None})
# b'nickname:\n'
```

Some formats prefer the explicit keyword, in particular data and specification
formats such as OpenAPI, where `null` is idiomatic. Opt into it with
`OPT_NULL_AS_KEYWORD`:

```python
import yamlrocks

yamlrocks.dumps({"nickname": None}, option=yamlrocks.OPT_NULL_AS_KEYWORD)
# b'nickname: null\n'
```

## Strings

Quote a string only when leaving it bare would change its meaning, for example a
value that would otherwise read as a number, a boolean, or null. When you do
quote, **double quotes** are preferred.

```yaml
# Quote because these would parse as other types
mohs: "7"
crystalline: "true"

# No quotes needed
name: Rose quartz
locality: Minas Gerais
```

Values that are clearly identifiers, such as mineral names, climbing grades,
and similar enumerated tokens, read fine unquoted and are left bare.

YAMLRocks quotes a scalar only when it must, keeping output clean, and uses double
quotes by default to match this rule. If you prefer single quotes (which avoid
backslash escaping for values with many backslashes, such as a regex or a Windows
path), pass `OPT_SINGLE_QUOTES`:

```python
import yamlrocks

yamlrocks.dumps({"crystalline": "true"})
# b'crystalline: "true"\n'

yamlrocks.dumps({"crystalline": "true"}, option=yamlrocks.OPT_SINGLE_QUOTES)
# b"crystalline: 'true'\n"
```

## Multi-line strings

For text that spans several lines, use a block scalar rather than `\n` escapes:
**literal** (`|`) keeps the line breaks, **folded** (`>`) joins lines with
spaces.

```yaml
# Good
field_notes: |
  Found near the ridge.
  Quartz vein visible.
```

`dumps` emits a multi-line string as a literal `|` block by default, choosing the
chomping indicator so it round-trips exactly:

```python
import yamlrocks

yamlrocks.dumps({"field_notes": "Found near the ridge.\nQuartz vein visible.\n"})
# b'field_notes: |\n  Found near the ridge.\n  Quartz vein visible.\n'
```

## Comments

Put a comment on its own line above what it describes, at the same indentation as
that line. Start the text with a capital letter and leave one space after the
`#`:

```yaml
# Tumble this one until it shines
agate:
  stage: rough
```

YAMLRocks preserves comments exactly in [round-trip mode](/guides/round-trip/),
including their position and spacing, so re-emitting an edited document keeps your
comments untouched. The fast `dumps` path does not invent comments.

## Emitting style-compliant output

The style is the YAMLRocks default, so a plain `dumps` already produces it, no
options required:

```python
import yamlrocks

config = {
    "name": "Rose quartz",
    "luster": None,
    "tags": ["pink", "translucent"],
}
yamlrocks.dumps(config)
# name: Rose quartz
# luster:
# tags:
#   - pink
#   - translucent
```
