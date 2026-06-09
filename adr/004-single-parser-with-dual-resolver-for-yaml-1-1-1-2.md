# ADR-004: Single parser with dual resolver for YAML 1.1/1.2

**Date**: 2026-06-08
**Status**: Accepted

**Context**: We need to support both YAML 1.1 and 1.2. The syntax is nearly identical; the difference is how unquoted scalars get typed.

**Decision**: One parser, two resolver implementations behind a trait.

**Rationale**:

- The scanner and parser are identical for 1.1 and 1.2
- Only scalar type resolution differs (booleans, octals, merge keys, sexagesimal)
- This is proven by serde-saphyr which has `strict_booleans` and `legacy_octal_numbers` options
- Avoids maintaining two parsers or two code paths
- The resolver trait makes it easy to add custom schemas later
