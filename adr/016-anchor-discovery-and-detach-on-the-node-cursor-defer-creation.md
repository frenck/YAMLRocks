# ADR-016: Anchor discovery and detach on the YAMLRocksNode cursor; defer creation

**Date**: 2026-06-08
**Status**: Accepted; extends ADR-015 (the YAMLRocksNode cursor). **Creation since
implemented (2026-06-08), see ADR-018.**

**Context**: Round-trip mode already preserves `&anchor`/`*alias` byte-for-byte,
and `YAMLRocksNode.anchor` reads a definition's name. But there was no way to _find_
anchors (list them, go from an alias to its definition or from a definition to
its uses), to navigate _into_ an alias, or to break an alias into an independent
copy. The three asks (find, create, detach) differ sharply in risk.

**Decision**: Ship the two **safe** capabilities now (discovery and detach)
plus transparent alias-following on navigation. Defer **creation** (minting
anchors/aliases) to a separate, opt-in effort.

Surface added:

- `YAMLRocksDocument.anchors -> dict[str, YAMLRocksNode]` (name -> defining YAMLRocksNode).
- `YAMLRocksNode.is_alias`, `YAMLRocksNode.target` (alias -> definition YAMLRocksNode), `YAMLRocksNode.aliases`
  (definition -> the alias Nodes referencing it).
- `YAMLRocksNode.detach()`: replace an alias with a deep copy of its target (styles and
  comments kept, anchor stripped, inner aliases expanded); raises on a non-alias.
- Indexing a `YAMLRocksNode` that is an alias transparently follows it to the anchored
  node, so the returned child is a live handle into the shared definition.

**Rationale**:

- **Discovery and detach cannot emit invalid YAML.** Listing, following, and
  cloning only read or _remove_ an alias; none of them can produce a dangling or
  forward reference. They are pure wins for editor/refactor use cases (and Home
  Assistant's `<<`-heavy configs).
- **Creation is the footgun.** An alias is valid only if its anchor is defined
  earlier in document order, and anchor names must not collide. A writable
  `anchor`/`make_alias` therefore needs ordering/uniqueness validation to honor
  the project's "never emit invalid YAML" rule. That validation machinery is
  worth its own ADR rather than being bundled in.
- **Transparent follow over an explicit `.target` for indexing** was the user's
  call: convenience won, with `.target`/`.is_alias` still available when you want
  to know you are crossing into shared content. An edit through a followed alias
  intentionally changes the shared anchor; `detach()` is the escape hatch.

**Implementation notes**:

- Paths are addressed within the first document (consistent with the rest of the
  `YAMLRocksNode`/`YAMLRocksDocumentView` model). Anchor/alias discovery is a path-tracking walk
  over mapping values and sequence items; anchors on mapping _keys_ are not
  addressable and so are not surfaced.
- `find_anchor_path` takes the first definition in document order (well-formed
  documents have unique anchor names).
- `detached_clone` keeps styles/comments and expands inner aliases via the
  anchor-ref map, guarded by `MAX_ALIAS_DEPTH` against cyclic anchors.

**Consequences**:

- Creating brand-new anchors/aliases still requires editing the value directly;
  the safe, common "break this shared reference" path is `detach()`.
- Editing through a followed alias mutates shared state by design, documented so
  it is not a surprise.
