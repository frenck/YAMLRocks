---
title: Migrating from PyYAML
description: Switch from PyYAML to YAMLRocks with a one-line import, and understand the behavior differences that matter.
---

If your code uses PyYAML's safe API, YAMLRocks ships a compatibility shim that lets
you switch with a one-line import change. This page shows the shim, maps the
PyYAML functions you already know to their YAMLRocks equivalents, and then walks
through the behavior differences that actually matter so nothing surprises you in
production. The headline ones: native `dumps` returns **bytes**, `yes`/`no` are
strings under YAML 1.2, and YAMLRocks never constructs arbitrary Python objects, so
it is safe by default.

## The drop-in shim

The `compat` module mirrors PyYAML's safe surface. Alias it on import and most
code keeps working unchanged:

```python
from yamlrocks import compat as yaml

data = yaml.safe_load("name: app\nport: 8080")
# {'name': 'app', 'port': 8080}

text = yaml.safe_dump(data)
# 'name: app\nport: 8080\n'  (a str, like PyYAML)
```

Note that `compat.safe_dump` returns a **`str`**, exactly like PyYAML, even
though native `yamlrocks.dumps` returns bytes. The shim exists precisely to smooth
over that and the other differences below.

## Function mapping

Every function in the table is importable from `yamlrocks.compat`:

| PyYAML               | yamlrocks.compat | Notes                                           |
| -------------------- | ---------------- | ----------------------------------------------- |
| `yaml.safe_load`     | `safe_load`      | parse the first document                        |
| `yaml.safe_load_all` | `safe_load_all`  | iterate documents in a stream                   |
| `yaml.safe_dump`     | `safe_dump`      | emit to a `str` (or a stream)                   |
| `yaml.safe_dump_all` | `safe_dump_all`  | emit several documents                          |
| `yaml.load`          | `load`           | mapped to the safe loader                       |
| `yaml.load_all`      | `load_all`       | mapped to the safe loader                       |
| `yaml.dump`          | `dump`           | mapped to the safe dumper                       |
| `yaml.dump_all`      | `dump_all`       | mapped to the safe dumper                       |
| `yaml.YAMLError`     | `YAMLError`      | alias for `yamlrocks.YAMLRocksError` (the base) |

Because the exception is the same class you catch today, your error handling
keeps working:

```python
from yamlrocks import compat as yaml

try:
    yaml.safe_load("a: 'unterminated")
except yaml.YAMLError as err:
    print("could not parse:", type(err).__name__)
# could not parse: YAMLRocksParseError
```

`safe_dump` accepts a stream as its second argument, just like PyYAML, and
writes to it instead of returning a string:

```python
import io
from yamlrocks import compat as yaml

buffer = io.StringIO()
yaml.safe_dump({"name": "app", "port": 8080}, buffer)
print(buffer.getvalue())
# name: app
# port: 8080
```

:::caution[Safe by design]
PyYAML's plain `yaml.load` can construct arbitrary Python objects from tags such
as `!!python/object`, which is why `safe_load` exists. YAMLRocks never does this:
there is no unsafe loader to migrate away from. The `compat.load`/`compat.dump`
aliases therefore behave like `safe_load`/`safe_dump`, and `Loader`/`Dumper`
keyword arguments are accepted and ignored for source compatibility.
:::

## Behavior differences that matter

The shim covers the API surface, but a handful of semantic differences are worth
understanding before you migrate. They are deliberate and, in most cases, fixes.

### Native `dumps` returns bytes, not str

This is the difference most likely to trip you up if you reach past the shim to
the native API. PyYAML's `safe_dump` returns a `str`; `yamlrocks.dumps` returns
`bytes`:

```python
import yamlrocks

yamlrocks.dumps({"name": "app"})
# b'name: app\n'

yamlrocks.dumps({"name": "app"}).decode()
# 'name: app\n'
```

If you stay on `compat.safe_dump` you get a `str` and never notice. Reach for
native `yamlrocks.dumps` when you want bytes, options, or the speed of the direct
path.

### YAML 1.2 by default: `yes`/`no` are strings

PyYAML follows YAML 1.1, where `yes`, `no`, `on`, and `off` parse as booleans.
YAMLRocks follows YAML 1.2, where they are plain strings:

```python
import yamlrocks

yamlrocks.loads(b"a: yes")
# {'a': 'yes'}

yamlrocks.loads(b"a: yes", option=yamlrocks.OPT_YAML_1_1)
# {'a': True}
```

If your documents rely on the old behavior, pass `OPT_YAML_1_1` to opt back in,
or run them through [`yamlrocks.upgrade`](/guides/yaml-11-vs-12/) once to normalize
`yes` to `true` permanently. See
[YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for the full list of differences.

### Key order is preserved by default

Native `yamlrocks.dumps` preserves insertion order; it does not sort keys unless you
ask. PyYAML sorts by default, and `compat.safe_dump` keeps that PyYAML default
(`sort_keys=True`) so the shim's output matches what PyYAML would have produced.
To sort with the native API, pass `OPT_SORT_KEYS`:

```python
import yamlrocks

yamlrocks.dumps({"b": 2, "a": 1})
# b'b: 2\na: 1\n'   (insertion order)

yamlrocks.dumps({"b": 2, "a": 1}, option=yamlrocks.OPT_SORT_KEYS)
# b'a: 1\nb: 2\n'   (sorted)
```

### Aliases and anchors

PyYAML resolves an alias (`*a`) to the _same_ object as its anchor (`&a`), so a
mutation through one reference is visible through every other. YAMLRocks matches
this on the paths that build rich objects: [annotated mode](/guides/annotated/),
[round-trip mode](/guides/round-trip/), and any load that resolves custom tags.

```python
import yamlrocks

source = """
base: &a
  k: 1
ref: *a
"""

data = yamlrocks.loads(source, option=yamlrocks.OPT_ANNOTATED)
data["base"] is data["ref"]   # True, the same object (as in PyYAML)
data["base"]["k"] = 99
data["ref"]["k"]              # 99, seen through the shared reference
```

The plain fast path (`loads` with no options, which is what the `compat` shim
uses) instead gives each alias an independent copy. The values compare equal, but
they are separate objects, which is faster. Reach for `OPT_ANNOTATED` (or
`OPT_ROUND_TRIP`) when you depend on shared-reference identity.

### Complex keys load instead of raising

PyYAML's `SafeLoader` rejects a sequence or mapping used as a mapping key with
`found unhashable key`. That is a limitation of fitting YAML onto a Python `dict`,
not a rule of the spec; complex keys are valid YAML (the spec even has a worked
example). YAMLRocks accepts them, rendering a sequence key as a `tuple` and a
mapping key as a `tuple` of its `(key, value)` pairs:

```python
import yamlrocks

yamlrocks.loads(b"[a, b]: paired\n")
# {('a', 'b'): 'paired'}
```

If you are migrating a test that asserted PyYAML raised on such input, that
document is valid YAML and now loads. See
[complex keys](/guides/loading/#complex-keys). If you specifically _want_ PyYAML's
reject-on-complex-key behavior back (for example because your data model is
strictly scalar-keyed), set `OPT_REJECT_COMPLEX_KEYS`, which raises
`YAMLRocksComplexKeyError` with a source location instead of converting.

:::tip[Common gotchas]

- Decoding `dumps` output: native `dumps` returns bytes; call `.decode()` if you
  need a `str`.
- Boolean-looking strings: a column of `yes`/`no` values that used to be booleans
  now load as strings unless you set `OPT_YAML_1_1`.
- Sorting: native output keeps insertion order; add `OPT_SORT_KEYS` to match
  PyYAML's sorted output.
- Tags: YAMLRocks will not build arbitrary objects, so any code that relied on
  `!!python/...` tags needs a different approach.

:::

## Going further

Once you are on YAMLRocks, you can drop the shim and adopt the native features
PyYAML never had. Source locations for validation, and round-trip editing that
preserves comments, are both a single option away:

```python
import yamlrocks

text = b"server:\n  host: localhost\n  port: 8080  # default\n"

# Source locations: each node carries its line and column.
data = yamlrocks.loads(text, option=yamlrocks.OPT_ANNOTATED)
print(data["server"].__line__)
# 2  (the server block's body starts on line 2)

# Round-trip editing: change a value, keep every comment.
doc = yamlrocks.loads(text, option=yamlrocks.OPT_ROUND_TRIP)
doc["server"]["port"] = 9090
print(doc.to_yaml().decode())
# server:
#   host: localhost
#   port: 9090  # default
```

## See also

- [Quick start](/getting-started/quick-start/): the five-minute tour.
- [Migration compatibility](/getting-started/compatibility/): the compatibility
  matrix and known migration gaps.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): why `yes` is a string now.
- [Round-trip editing](/guides/round-trip/) and
  [annotated mode](/guides/annotated/): features beyond PyYAML.
- [vs PyYAML](/comparisons/vs-pyyaml/): a feature and speed comparison.
- [Security](/reference/security/): why YAMLRocks is safe by default.
