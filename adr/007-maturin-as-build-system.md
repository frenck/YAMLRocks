# ADR-007: maturin as build system

**Date**: 2026-06-08
**Status**: Accepted

**Context**: We need to build a Rust extension module for Python.

**Decision**: Use maturin.

**Rationale**:

- Industry standard for Rust+Python
- Handles wheel building for all platforms
- Integrates with PyPI publishing
- Used by orjson
