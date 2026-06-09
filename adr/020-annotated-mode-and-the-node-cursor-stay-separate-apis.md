# ADR-020: Annotated mode and the `YAMLRocksNode` cursor stay separate APIs

**Date**: 2026-06-08
**Status**: Accepted

**Context**: `OPT_ANNOTATED` (ADR-010) and the round-trip `YAMLRocksDocument`/`YAMLRocksNode`
cursor (ADR-015) both expose source positions, which raises two questions: is
annotated mode still warranted now that a `YAMLRocksNode` can report `line`/`column`/
`file`, and should annotated values carry a back-reference to their `YAMLRocksNode` so a
reader can "upgrade" to the editable cursor in place?

**Decision**: Keep both as distinct, non-overlapping APIs. Annotated values
(`YAMLRocksAnnotatedDict`/`YAMLRocksAnnotatedList`/`YAMLRocksAnnotatedStr`) do NOT hold a reference to a
`YAMLRocksNode` or `YAMLRocksDocument`. The sanctioned bridge from a located value to an editable
one is the key path: re-load with `OPT_ROUND_TRIP` and navigate `doc.node[...]`,
not object identity. The dividing line:

- Annotated mode is the read-only loader path. It returns ordinary `dict`/`list`/
  `str` objects, so they flow through validators (voluptuous and the like)
  unchanged, with positions riding along as attributes. This is Home Assistant's
  bootstrap and reload path.
- The `YAMLRocksNode` cursor is the editing path: a live handle over the retained AST that
  preserves comments, anchors, and styles for byte-faithful write-back. This is a
  config editor, not the loader.

**Rationale**:

- Transparency is annotated mode's whole value: `isinstance(x, dict)` holds and
  `{**x}` works, so one object both validates and reports errors. A `YAMLRocksDocumentView`
  is not a `dict` and cannot be handed to the validation ecosystem.
- A `YAMLRocksNode` back-reference would force annotated mode to retain the full Rust AST
  for the lifetime of the data, erasing the memory and latency advantage that is
  its reason to exist (it currently builds the composer AST transiently and drops
  it). Annotated mode would collapse into round-trip mode with a friendlier face.
- The reference would dangle: validators routinely build new, normalized dicts
  from the parsed input, so an embedded node would point at a document position
  that no longer matches the value the caller holds.
- Two mutation truths: editing through a `.node` would change the AST, not the
  detached Python value the caller has, so the two would silently diverge.

**Consequences**:

- Choosing the mode is an up-front decision: read or validate uses annotated;
  edit or write-back uses round-trip. A consumer that needs both loads round-trip,
  which exposes values by indexing and positions via `.node`.
- Internally the two already share the composer AST (`annotate_node` walks the
  same `YamlNode`), so keeping both public surfaces costs no duplicated engine.

**Alternatives considered**: a `.node` property on annotated values (rejected:
forces AST retention and dangles after transformation); a single "both" mode
returning annotated data plus a `YAMLRocksDocument` (rejected: that is round-trip mode,
which already yields values by indexing and positions via `.node`); dropping
annotated mode in favor of `YAMLRocksNode` (rejected: `YAMLRocksNode`/`YAMLRocksDocumentView` are not
`dict`/`list`/`str` and break the drop-in validation path that is the Home
Assistant adoption story).
