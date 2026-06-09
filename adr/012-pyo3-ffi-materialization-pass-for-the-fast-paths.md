# ADR-012: pyo3-ffi materialization pass for the fast paths

**Date**: 2026-06-08
**Status**: Accepted; realizes the deferred performance pass of ADR-009

**Context**: ADR-009 built the library on high-level PyO3 and deferred the
`pyo3-ffi` hot-path rewrite "until benchmarks justify the specific changes". A
benchmark pass on a release build provided that justification, and pointed at
_where_ the time actually goes:

- `dumps` spends its time in the Python-object _dispatch and traversal_ (the
  per-element `cast` chain and the Python iterator protocol).
- `loads` is dominated by the scanner/parser and intermediate allocations, not
  by the construction of the final Python objects. Each string scalar was
  allocated three times: the scanner's token `String`, a second copy in the
  resolver's `value.to_owned()`, and the final `PyString`.

**Decision**: Rewrite the Python-touching hot paths against `pyo3-ffi`, and
remove the redundant string copy in the decoder, without disturbing the parser,
the round-trip layer, or any observable behavior:

1. **Decode materialization** (`value_to_python_with`): build sequences with
   `PyList_New` + `PyList_SET_ITEM` (exact size, steals the reference, no append
   reallocation) and mappings with a direct `PyDict_SetItem`. Key interning is
   retained.
2. **Encode dispatch** (`python_to_value`): a `typeref` module compares
   `Py_TYPE(obj)` against the static builtin type objects, so the common _exact_
   type is dispatched by a pointer comparison instead of the subclass-aware
   `cast` chain. Subclasses (`IntEnum`, numpy scalars, `YAMLRocksAnnotatedStr`,
   dataclasses, ...) fail every exact check and fall through to the unchanged
   slow path, so behavior is identical.
3. **Encode traversal**: exact `list`/`dict` are walked with `PyList_GET_ITEM`
   and `PyDict_Next`, avoiding the iterator-object allocation and `tp_iternext`
   per element.
4. **Decoder string move**: the fast-path decoder takes ownership of each
   scalar's scanned text up front (`take_scalar_strings`) and a new
   `Resolver::classify` returns the scalar _type_ without allocating, so a
   string scalar moves its existing buffer straight into `Value::String` rather
   than re-cloning it. `Resolver::resolve` remains for the annotated AST path,
   which does not own the source text.

**Rationale**:

- The changes are confined to the two fast paths and a leaf `typeref` module.
  The round-trip path (`compose` -> AST -> `YAMLRocksDocument`) and its byte-for-byte
  guarantee are untouched, because they never used the fast-path `Value` tree.
- Exact-type dispatch is a strict optimization: anything that is not an exact
  builtin behaves exactly as before.

**Validation**:

- Full pytest suite green under the memory cap; refcount-stability probes show
  zero object growth and stable refcounts across the new paths.
- `valgrind --tool=memcheck`: 0 errors and 0 bytes definitely lost across a
  load/dump workload exercising every new `unsafe` path.

**Measured impact** (controlled release-build micro-benchmark, min of repeated
runs, relative to the pre-change build):

- `dumps`: ~10-18% faster (small 0.92 -> 0.76 us, medium 5.43 -> 4.62 us).
- `loads`: ~5-8% faster (medium 75.6 -> 70.7 us, large 1135 -> 1040 us).

**Consequences**:

- A small amount of `unsafe` now lives in the `ffi/convert/` modules
  (`encode.rs` and `decode.rs`), each block carrying a SAFETY comment and covered
  by the valgrind pass.
- The remaining `loads` headroom is upstream of materialization: the scanner's
  token allocation and the parser's full event vector. Closing it needs a
  zero-copy/arena rewrite of the _shared_ token and event types, which would
  also touch the round-trip composer and its byte-exact guarantee. That work is
  deliberately left as a separate, larger effort rather than bundled here.
