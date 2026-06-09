# ADR-002: Use pyo3-ffi (low-level) instead of high-level PyO3

**Date**: 2026-06-08
**Status**: Accepted

**Context**: We need Python bindings. PyO3 offers both high-level ergonomic bindings and low-level FFI access.

**Decision**: Use `pyo3-ffi` (low-level), following orjson's approach.

**Rationale**:

- orjson proves this works and achieves maximum performance
- High-level PyO3 adds overhead: trait impls, error conversions, GILGuard acquisition, type checking on every call
- The loads() hot path constructs many Python objects (dicts, lists, strings). Every microsecond of overhead matters.
- We need vectorcall protocol support for keyword arguments
- We need CPython version-specific optimizations (immortal objects, free-threaded mode)

**Consequences**:

- Steeper learning curve, more boilerplate
- More careful memory management (reference counting by hand)
- Maximum performance ceiling
