# ADR-013: Zero-copy scalar scanning with `Cow<'input, str>`

**Date**: 2026-06-08
**Status**: Accepted; completes the allocation-elimination half of ADR-009

**Context**: After ADR-012 the `loads` cost was dominated by the scanner: every
scalar's text was copied into an owned `String` token even when the value was a
verbatim slice of the input. A single-line plain scalar (the most common kind in
real configs) is _exactly_ `input[start..content_end]`; the only thing that
transforms a plain scalar is folding across a newline.

**Decision**: Borrow the input for the common case. `scan_plain` tracks byte
offsets and returns `Cow::Borrowed(slice)` for a single-line scalar, allocating
lazily only at the point it commits to folding onto another line. The borrowed
`Cow<'input, str>` threads through `TokenKind::Scalar`, `EventKind::Scalar`, and
a new lifetime-parameterized `Value<'input>`, so a plain string scalar travels
from input to the final `PyString` with no intermediate Rust allocation. Quoted
and block scalars (which always transform) stay `Cow::Owned`. `dumps` builds
owned values, i.e. `Value<'static>`.

**Rationale**:

- The win lands on the dominant scalar kind without changing the delicate
  plain-scalar termination logic: the existing fold algorithm is preserved
  verbatim; only the point of materialization moved.
- Lifetime threading is checked at compile time, so it cannot introduce a
  runtime regression: if it builds and the suite passes, behavior is preserved.

**Consequences**:

- Tokens and events now carry an `'input` lifetime. The round-trip composer
  materializes owned text (`value.to_string()`) at its single scalar boundary,
  so the byte-for-byte round-trip guarantee is untouched (that path always
  copied into the AST anyway).
- Measured (cumulative with ADR-012, vs the pre-optimization build): `loads`
  ~14-22% faster, `dumps` ~10-18% faster. Full suite green, byte-exact
  round-trip verified, `valgrind` memcheck clean (0 errors, 0 bytes lost).
