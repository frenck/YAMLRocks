# ADR-011: Reattach comments by source position in a post-pass

**Date**: 2026-06-08
**Status**: Accepted

**Context**: Comments are scanned with spans but must be attached to the right
AST nodes (head/inline/foot) for round-trip fidelity.

**Decision**: The scanner collects comments (only on the round-trip path, gated
by a flag so the fast path stays cheap). After the composer builds the AST, a
single document-order walk reattaches comments using line/column proximity: a
comment above a node becomes its head comment; a comment trailing a value on the
same line becomes its inline comment; leftovers become foot comments on the last
node.

**Rationale**:

- Decouples comment handling from the token/event plumbing, keeping the parser
  simple and the fast path allocation-free for comments.
- Position-based attachment is robust across nesting and matches how humans read
  comment placement.

**Consequences**:

- Inline spacing is normalized (multiple spaces before `#` collapse to one).
- Comments in unusual positions (e.g. between a key's colon and a block value)
  may attach to the nearest reasonable node rather than their exact slot.
