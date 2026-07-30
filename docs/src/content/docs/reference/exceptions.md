---
title: Exceptions
description: The YAMLRocks error hierarchy, the metadata each error carries, and how to catch them.
---

Every error YAMLRocks raises derives from a single base, `YAMLRocksError`, which
carries a human-readable `message` plus the source location (`file`, `line`,
`column`) whenever it is known. Below the base sit two categories, each of which
also subclasses a familiar builtin so existing handlers keep working:

- **`YAMLRocksDecodeError`** (also a `ValueError`) for anything that goes wrong
  while _reading_ YAML, with a fine-grained subtree for parsing, schema
  validation, includes, secrets, and environment variables.
- **`YAMLRocksEncodeError`** (also a `TypeError`) for anything that goes wrong
  while _writing_ it.

```text
YAMLRocksError                          .message, .file, .line, .column
├── YAMLRocksDecodeError                (also ValueError)
│   ├── YAMLRocksParseError             malformed YAML syntax
│   ├── YAMLRocksDuplicateKeyError      duplicate key (OPT_DUPLICATE_KEYS_ERROR)
│   ├── YAMLRocksComplexKeyError        collection used as a key (OPT_REJECT_COMPLEX_KEYS)
│   ├── YAMLRocksSchemaError            schema validation failed (.schema_path)
│   ├── YAMLRocksIncludeError           !include family (.include_stack)
│   │   ├── YAMLRocksIncludeNotFoundError
│   │   ├── YAMLRocksCircularIncludeError
│   │   ├── YAMLRocksIncludeDepthError
│   │   └── YAMLRocksIncludeConfinementError
│   ├── YAMLRocksSecretError
│   │   └── YAMLRocksSecretNotFoundError
│   └── YAMLRocksEnvVarError
└── YAMLRocksEncodeError                (also TypeError)
    └── YAMLRocksUnserializableError
```

Catch at whatever level fits: a specific class (`YAMLRocksIncludeNotFoundError`),
a category (`YAMLRocksDecodeError`), the base (`YAMLRocksError`), or the builtin
(`ValueError`). The classes live in `yamlrocks.exceptions` and are also re-exported
on the top-level `yamlrocks` package.

## Structured location

The location is available as attributes, not just in the message text. `line` and
`column` are 1-based; `file` is the source path, or `None` for in-memory input:

<!-- verify: raises YAMLRocksParseError -->

```python
import yamlrocks

try:
    yamlrocks.loads(b'key: "unterminated')
except yamlrocks.YAMLRocksParseError as exc:
    print(exc.line, exc.column)  # 1 6
    print(exc.file)  # None (loaded from bytes)
    print(str(exc))  # unterminated double-quoted scalar at line 1, column 6
    raise
```

`load` (and `load_all`) fill in `file` with the path they read from, so an error
points at the file on disk. Because `YAMLRocksParseError` is a `YAMLRocksDecodeError`
is a `ValueError`, any of those `except` clauses catches it.

## Reading errors

`YAMLRocksDecodeError` is the category base for every read-side failure. Its
subclasses let you react to a specific cause:

| Exception                          | Raised when                                                                                                                                                    |
| ---------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `YAMLRocksParseError`              | the input is not well-formed YAML                                                                                                                              |
| `YAMLRocksDuplicateKeyError`       | a duplicate key is found under `OPT_DUPLICATE_KEYS_ERROR`                                                                                                      |
| `YAMLRocksComplexKeyError`         | a collection (sequence or mapping) is used as a mapping key under [`OPT_REJECT_COMPLEX_KEYS`](/guides/loading/#rejecting-complex-keys-opt_reject_complex_keys) |
| `YAMLRocksSchemaError`             | [schema validation](/guides/schema-validation/) fails; `.schema_path` is the JSON path of the offending node                                                   |
| `YAMLRocksIncludeNotFoundError`    | an `!include` target does not exist                                                                                                                            |
| `YAMLRocksCircularIncludeError`    | an `!include` chain forms a cycle                                                                                                                              |
| `YAMLRocksIncludeDepthError`       | an `!include` chain is too deep, or expands too many files in total (a fan-out)                                                                                |
| `YAMLRocksIncludeConfinementError` | an `!include` resolves outside `include_dir`                                                                                                                   |
| `YAMLRocksSecretNotFoundError`     | a `!secret` name is not in any `secrets.yaml`                                                                                                                  |
| `YAMLRocksEnvVarError`             | an `!env_var` is undefined and has no default                                                                                                                  |

Include errors also carry `include_stack`, the chain of `(file, line)` pairs that
led to the failure:

```python
import os
import tempfile
import yamlrocks

config = tempfile.mkdtemp()
with open(os.path.join(config, "main.yaml"), "w") as handle:
    handle.write("data: !include missing.yaml\n")

try:
    yamlrocks.load(os.path.join(config, "main.yaml"), option=yamlrocks.OPT_INCLUDES)
except yamlrocks.YAMLRocksIncludeNotFoundError as exc:
    print(exc.file is not None)  # True
    print(isinstance(exc.include_stack, list))  # True
```

## Writing errors

`YAMLRocksEncodeError` (a `TypeError`) is raised by `dumps`/`dump` when a value has
no YAML representation. The concrete class is `YAMLRocksUnserializableError`:

<!-- verify: raises YAMLRocksUnserializableError -->

```python
import yamlrocks

try:
    yamlrocks.dumps({"value": object()})
except yamlrocks.YAMLRocksEncodeError as exc:
    print(exc)
    # type object is not YAML serializable
    raise
```

The usual fix is a `default` callable that maps each unsupported value to
something YAML can represent:

```python
import yamlrocks

yamlrocks.dumps({"value": object()}, default=str)
# b'value: <object object at 0x...>\n'
```

:::tip[default turns "unsupported" into "your problem to map"]
Reach for `default` whenever you dump custom classes, `Decimal`, `pathlib.Path`,
and the like. See [dumping](/guides/dumping/) for the full type table.
:::

## Compatibility

The [PyYAML shim](/getting-started/migrating-from-pyyaml/) exposes
`yamlrocks.compat.YAMLError`, aliased to the base `YAMLRocksError`, so code written
against PyYAML that does `except yaml.YAMLError` keeps catching both read and
write errors after you switch the import:

```python
import yamlrocks
from yamlrocks import compat

print(compat.YAMLError is yamlrocks.YAMLRocksError)  # True
```

## See also

- [Loading YAML](/guides/loading/): where the decode errors come from.
- [Dumping YAML](/guides/dumping/): where the encode errors come from, and `default`.
- [Schema validation](/guides/schema-validation/): `YAMLRocksSchemaError` and `.schema_path`.
- [Includes](/guides/includes/): the include error subtree.
- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/): the `YAMLError` alias.
