# ADR-009: Start on high-level PyO3, defer pyo3-ffi to a performance pass

**Date**: 2026-06-08
**Status**: Accepted; supersedes ADR-002 for the current phase

**Context**: ADR-002 chose `pyo3-ffi` (low-level, orjson-style) for maximum
performance. In practice, Phases 1-3 were implemented against the high-level
PyO3 API (`#[pyclass]`, `#[pyfunction]`, `Bound`/`Py` smart pointers).

**Decision**: Build correctness-first on high-level PyO3 now; revisit `pyo3-ffi`
for the hot paths during the Phase 4 performance pass.

**Rationale**:

- The defining hard problems of Phases 1-3 are _correctness_ problems (the YAML
  scanner, comment reattachment, writable includes), not FFI overhead.
- High-level PyO3 makes the structure-preserving `YAMLRocksDocument`/`YAMLRocksDocumentView`
  proxies and the `extends = PyDict`/`PyList` annotated types straightforward
  and memory-safe. Hand-writing these against `pyo3-ffi` would be slow to get
  right and error-prone.
- The decode/encode boundary is already isolated, so the fast path can be
  rewritten against `pyo3-ffi` later without disturbing the parser or the
  round-trip layer.

**Consequences**:

- Current performance is not yet orjson-class; that is acceptable pre-benchmark.
- A future ADR will record the `pyo3-ffi` migration of the `loads`/`dumps`
  hot paths once benchmarks exist to justify the specific optimizations.
