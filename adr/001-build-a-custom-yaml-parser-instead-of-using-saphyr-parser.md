# ADR-001: Build a custom YAML parser instead of using saphyr-parser

**Date**: 2026-06-08
**Status**: Accepted

**Context**: We need a YAML parser for YAMLRocks. The Rust ecosystem has saphyr-parser (YAML 1.2, actively developed, passes test suite) and unsafe-libyaml (YAML 1.1, Rust port of libyaml).

**Options considered**:

1. Use saphyr-parser as-is
2. Use unsafe-libyaml for 1.1 + saphyr-parser for 1.2
3. Build our own parser from scratch

**Decision**: Option 3, build our own.

**Rationale**:

- **Comment handling**: saphyr-parser discards comments during scanning. Adding comment support would require either contributing upstream (slow, dependent on maintainer) or maintaining a fork. Our parser has comments as first-class tokens from day one.
- **Source location tracking**: We need spans on every token for error reporting and annotated mode. Easier to bake in than bolt on.
- **Include resolution**: The scanner needs to know about file boundaries for writable includes. This is deeply integrated, not a layer on top.
- **Performance tuning**: We can optimize the hot paths for exactly the YAML patterns our users have (short keys, plain scalars, repetitive structure) rather than being general-purpose.
- **No dependency risk**: The parser is the core of the product. We should own it entirely.
- **The parser IS the product**: Unlike orjson which uses yyjson (a proven C JSON parser), the YAML parser needs features (comments, includes) that no existing parser provides. Building our own is the only way to get everything we need without compromise.

**Consequences**:

- Significantly more initial development work
- We must validate against the YAML test suite (300+ cases) from day one
- We own all parser bugs (no upstream to report to)
- Total control over the roadmap
