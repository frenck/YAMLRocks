---
title: Architecture
description: How YAMLRocks is built, from raw bytes through the scanner and parser to Python objects.
---

YAMLRocks is a Rust extension (via PyO3, built by maturin) with a thin Python
package on top. It ships its own YAML scanner and parser rather than depending on
an external one, which is what makes first-class comments, native includes, and
source tracking possible. This page is a code map: it follows a document through
the pipeline and points at where each stage lives in `src/`.

## The pipeline

```
bytes -> scanner -> tokens -> parser -> events ┬-> resolver -> Python objects (fast path)
                                               └-> composer -> AST -> YAMLRocksDocument (round-trip)
```

The split after `events` is the central design choice: one front end (scanner +
parser) feeds two back ends. The fast path is tuned for raw throughput; the
round-trip path is tuned for fidelity. Which one runs is decided by the option
flags on the call, chiefly `OPT_ROUND_TRIP`.

## Front end

### Scanner (`src/scanner/`)

A state machine over a UTF-8 reader (`reader.rs`) that tracks indentation and
block/flow context and recognizes every scalar style: plain, single- and
double-quoted, literal (`|`), and folded (`>`). Scalars are scanned in
`scalar.rs` and tokens are defined in `token.rs`.

Comments are extracted as first-class items with source spans (`comment.rs`), but
**only on the round-trip path**. The fast path skips comment retention entirely
so it does no work it will throw away.

### Parser (`src/parser/`)

Turns the token stream into a flat sequence of events (stream / document /
mapping / sequence / scalar / alias start and end), each carrying a span. The
event type lives in `event.rs`. Events are deliberately low-level and shared by
both back ends, so neither has to re-walk tokens.

### Resolver (`src/resolver/`)

The only component that differs between YAML 1.1 and 1.2. A `Resolver` trait has
two implementations, `yaml12.rs` (the default core schema) and `yaml11.rs`
(`yes`/`no` booleans, `0777` octals, sexagesimal numbers). The resolver decides
how a plain scalar gets typed; everything upstream is schema-agnostic. This is
[ADR-004](https://github.com/frenck/yamlrocks/blob/main/adr/004-single-parser-with-dual-resolver-for-yaml-1-1-1-2.md):
a single parser with a dual resolver.

## The two decode paths, and why

### Fast path (`src/decode/`, `src/encode/`)

Events become a compact `Value` tree (an internal enum), which is then
materialized into Python objects. Merge keys (`<<`) are resolved here. This path
is allocation-light and never touches comments, which is why it carries the bulk
of the throughput advantage over PyYAML and ruamel.

`src/encode/` is the reverse: native Python objects to YAML bytes for `dumps`.

### Round-trip path (`src/roundtrip/`)

Events become a rich `YamlNode` AST (`ast.rs`) built by the composer
(`composer.rs`), carrying comments, scalar styles, anchors, and include markers.
A post-pass reattaches comments to nodes by source position ([ADR-011](https://github.com/frenck/yamlrocks/blob/main/adr/011-reattach-comments-by-source-position-in-a-post-pass.md)): a comment
above a node becomes its head comment, a comment trailing a value on the same
line becomes its inline comment. The emitter (`emit.rs`) reproduces the document,
and `document.rs` backs the Python `YAMLRocksDocument`/`YAMLRocksDocumentView` types. An unmodified
document returns its original source verbatim; only changed nodes are
re-rendered. `upgrade.rs` implements `yamlrocks.upgrade()` on top of this AST.

## Zero-copy scalar borrowing

Scalars are borrowed from the input buffer wherever possible. The scanner returns
`Cow<'input, str>` (see `src/scanner/scalar.rs`): a plain scalar with no escapes
borrows directly from the input bytes (the `Borrowed` variant, no allocation),
and only a scalar that needs unescaping or unfolding allocates an `Owned` string.
The lifetime `'input` threads through events and the `Value` tree
(`src/decode/mod.rs`), so a typical document is parsed with very few string
allocations. Strings are only copied into owned Python objects at the final
materialization step.

## PyO3 FFI materialization (`src/ffi/`)

The PyO3 module lives here:

- `mod.rs`: the `#[pyfunction]` entry points registered on the module: `loads`,
  `loads_all`, `dumps`, `to_json`, `schema_ref`, `yaml_version`, `dump_includes`,
  `dump_includes_map`, plus the internal round-trip helpers (`loads_roundtrip`,
  `loads_via_ast`). The public `load`, `load_all`, `dump`, and `upgrade` are thin
  Python wrappers in `pysrc/yamlrocks/__init__.py` that call these.
- `convert/`: the materialization layer that turns the internal `Value` tree
  into Python objects. The hot path (`value_to_python_with` in `convert/decode.rs`)
  builds containers with raw CPython calls (`PyList_New` + `PyList_SET_ITEM`,
  `PyDict_New` + `PyDict_SetItem`) to avoid per-element overhead, then hands back
  owned `Py` handles. `convert/encode.rs` is the reverse direction for `dumps`,
  and `convert/annotate.rs` produces
  `YAMLRocksAnnotatedDict`/`YAMLRocksAnnotatedList`/`YAMLRocksAnnotatedStr`.
- `types.rs`: the Python-facing `#[pyclass]` types defined here are `YAMLRocksTag`
  and the annotated containers `YAMLRocksAnnotatedDict`/`YAMLRocksAnnotatedList`
  (`YAMLRocksAnnotatedStr` is a pure-Python subclass;
  `YAMLRocksDocument`/`YAMLRocksDocumentView`/`YAMLRocksNode` live in
  `roundtrip/document.rs`).

The decode and encode hot paths drop to raw `pyo3-ffi`: list and dict
materialization, exact-type dispatch, and direct iteration all go through the
CPython API, and single-line plain scalars are parsed zero-copy by borrowing
straight from the input ([ADR-012](https://github.com/frenck/yamlrocks/blob/main/adr/012-pyo3-ffi-materialization-pass-for-the-fast-paths.md)
and [ADR-013](https://github.com/frenck/yamlrocks/blob/main/adr/013-zero-copy-scalar-scanning-with-cow-input-str.md)).
High-level PyO3 is kept only
where it makes the structure-preserving `YAMLRocksDocument` proxies and the `dict`/`list`
subclasses memory-safe and tractable ([ADR-009](https://github.com/frenck/yamlrocks/blob/main/adr/009-start-on-high-level-pyo3-defer-pyo3-ffi-to-a-performance-pass.md),
which supersedes the original
low-level `pyo3-ffi` plan in [ADR-002](https://github.com/frenck/yamlrocks/blob/main/adr/002-use-pyo3-ffi-low-level-instead-of-high-level-pyo3.md)). The remaining headroom is a full arena
rewrite of the scanner, deliberately left as a separate effort.

## Includes (`src/include/`)

Resolves the `!include` and `!include_dir_*` family (gated by `OPT_INCLUDES`),
plus `!secret` (gated by `OPT_SECRETS`) and `!env_var` (gated by `OPT_ENV_VAR`).
A `ResolveTags` struct threads which tags are enabled through the resolver, so
each tag is inert unless its flag is set. Each resolved node's span records the
file it came from, which is what lets round-trip `save()` and `dump_includes()`
write an edit back to the correct source file. Include cycles
(`a.yaml -> b.yaml -> a.yaml`) are detected and rejected before recursing.

## Schema (`src/schema/`)

A focused JSON Schema validator (`mod.rs`) that runs against the AST, so a
validation error carries the precise line and column of the offending node rather
than a path-only message.

## Security and limits

Untrusted input is bounded at several points (see [security](/reference/security/)):

- **Nesting depth** is capped at `MAX_DEPTH = 1000` in both the fast-path decoder
  (`src/decode/mod.rs`) and the round-trip composer (`src/roundtrip/composer.rs`),
  preventing stack exhaustion from deeply nested input.
- **Alias expansion** is bounded by a node budget, `MAX_NODES = 10_000_000` in
  `src/decode/mod.rs`. Expansion is measured before an alias is cloned, so a
  "billion laughs" document is rejected instead of exhausting memory.
- **Include cycles** are rejected in `src/include/mod.rs`.
- **No arbitrary object construction**: tags never instantiate Python objects.
  Unknown tags keep their underlying scalar unless a `tag_handler` or
  `OPT_PASSTHROUGH_TAG` opts in.

## Design records

The reasoning behind the major choices (custom parser over saphyr-parser, bytes
output from `dumps`, dual resolver, annotated `dict`/`list` subclasses,
position-based comment attachment, the PyO3 strategy) is recorded as ADRs in
[the `adr/` folder](https://github.com/frenck/yamlrocks/tree/main/adr).
Start there before proposing a structural change.

## See also

- [Security](/reference/security/): the limits above, from a user's view.
- [Round-trip editing](/guides/round-trip/): the feature the round-trip path
  exists to serve.
- [Includes](/guides/includes/): the include and write-back model.
