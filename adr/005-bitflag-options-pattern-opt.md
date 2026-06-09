# ADR-005: Bitflag options pattern (OPT\_\*)

**Date**: 2026-06-08
**Status**: Accepted

**Context**: We need to expose many configuration options for loads/dumps.

**Decision**: Use bitflag constants (`OPT_YAML_1_1 | OPT_SORT_KEYS`), like orjson.

**Rationale**:

- Proven ergonomic pattern from orjson
- Zero overhead (integer comparison in Rust)
- Composable and clear
- Easy to extend without breaking API
