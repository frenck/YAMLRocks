# ADR-003: Return bytes from dumps(), not str

**Date**: 2026-06-08
**Status**: Accepted

**Context**: Should `dumps()` return `str` or `bytes`?

**Decision**: Return `bytes`, consistent with orjson.

**Rationale**:

- Avoids UTF-8 validation overhead on the output (we know it's valid, but Python's str constructor validates)
- Consistent with orjson's proven API design
- Users who need str call `.decode()`, one extra call, trivial
- More efficient for network/file I/O (bytes is what gets written anyway)
