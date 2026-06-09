# ADR-018: Anchor/alias creation with validity guards

**Date**: 2026-06-08
**Status**: Accepted; completes the creation half deferred by ADR-016

**Context**: ADR-016 shipped anchor discovery and `detach` but deferred
_creation_ (minting new `&anchor`/`*alias`), because an alias is only valid when
its anchor exists and is defined earlier in the document, and anchor names must
not collide; a creation API has to validate this to honor the project's "never
emit invalid YAML" rule. That validation is the whole reason it was deferred;
this ADR adds it.

**Decision**: Make `YAMLRocksNode.anchor` writable and add `YAMLRocksNode.make_alias(name)`, each
validated up front:

- `anchor` setter: reject an empty name, and reject a name already defined on a
  _different_ node (so the document can never emit two `&name`). Re-assigning a
  node's own existing name is a no-op; `None` clears the anchor.
- `make_alias(name)`: the anchor must already exist (`find_anchor_path`) **and**
  precede this node in document order (`path_precedes`, a tree-order comparison:
  ancestor-before-descendant, else position within the first diverging
  container). Otherwise raise `ValueError`. On success the node's kind becomes
  `Alias(name)` and any anchor it carried is cleared (a node cannot be both).

**Rationale**:

- YAMLRocksDocument-order precedence is exactly YAML's rule (an alias binds to a _prior_
  anchor), and it is checked structurally rather than via spans, so it is correct
  for freshly-created anchors whose spans are still the default.
- Validating at the API boundary means a `YAMLRocksDocument` built through the cursor is
  always emittable; there is no separate "valid?" step and no way to produce a
  dangling or forward reference.

**Consequences**:

- Creating a recursive structure (aliasing a node to one of its own ancestors)
  is permitted, as it is valid YAML; `value`/resolution already guard cycles with
  `MAX_ALIAS_DEPTH`.
- This closes the last consciously-deferred feature from the original plan.
