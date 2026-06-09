# ADR-019: In-file schema references (yaml-language-server directive)

**Date**: 2026-06-08
**Status**: Accepted

**Context**: Editors/tooling let a YAML file name its own JSON Schema via a header
comment: `# yaml-language-server: $schema=<URL-or-path>`. Users want YAMLRocks to
recognize this. The hard constraint: parse-time URL fetching is unacceptable
(SSRF, latency, surprise I/O).

**Decision**: Split detection from resolution.

- `yamlrocks.schema_ref(data) -> str | None`: a pure detector (Rust
  `src/schema/directive.rs`) that scans only the leading comment block and returns
  the declared reference. No body parse, no I/O. Always safe.
- `loads(..., schema="auto", schema_resolver=<callable ref -> dict | None>)` -
  opt-in validation: when `schema="auto"`, detect the reference and hand it to the
  caller-supplied resolver, then validate against whatever dict it returns (skip if
  `None` or no directive). YAMLRocks never resolves the reference itself. Threaded
  through `load`/`async_loads`/`async_load`. Guard rails: `schema="auto"` without a
  resolver, or a resolver without `"auto"`, raises `ValueError` to prevent
  ambiguous schema sources. The existing `schema=<dict>` path is unchanged.

**Rationale**: Cheap, safe detection is separated from costly/risky resolution;
the network/filesystem policy is fully owned by the caller (a cache, a bundled
file, an allow-listed fetch). The existing AST validator is reused verbatim, so
in-file validation composes with round-trip, annotated, include, and YAML-1.1
modes for free.

**Alternatives considered**: a boolean that auto-fetches (rejected: SSRF/I/O); a
built-in allow-list resolver (rejected: policy belongs to the caller); a separate
`validate_with_ref()` function (rejected: redundant with `loads`); using the
round-trip comment machinery for detection (rejected: a textual scan of the
leading block is cheaper and works without round-trip mode).

**Consequences**: `schema_ref` is always free and safe; validation requires an
explicit, auditable resolver. `src/schema/directive.rs` also introduced the
project's first Rust `#[cfg(test)]` unit tests (the directive grammar is pure and
worth testing directly).
