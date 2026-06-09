# ADR-015: A uniform `YAMLRocksNode` cursor for round-trip metadata

**Date**: 2026-06-08
**Status**: Accepted; builds on ADR-011 (comment attachment)

**Context**: Round-trip mode preserves comments, source locations, scalar/
collection styles, anchors, and tags, but item access had no way to _expose_
them at a leaf. `doc["server"]["port"]` resolves to the plain integer `8080`,
and a bare `int`/`str`/`bool`/`None` has nowhere to carry a comment or a line
number. The same gap applies uniformly to line, column, file, style, anchor,
and tag.

**Options considered**:

1. **Rich scalar subclasses** - return an `int`/`str` subclass carrying
   `.comment`, `.line`, etc.
2. **A `.__node__` accessor on returned values** - hang a handle off the value.
3. **A parallel `YAMLRocksNode` cursor** reached via `doc.node[...]`, where indexing a
   `YAMLRocksNode` always returns another `YAMLRocksNode`.

**Decision**: Option 3, a `YAMLRocksNode` handle, `{ root: Py<YAMLRocksDocument>, path }`, with
`YAMLRocksDocument.node` (and `YAMLRocksDocumentView.node`) as the root cursor.

**Rationale**:

- **Python finality kills the rich-scalar approach.** `bool` and `NoneType` are
  not subclassable, subclass metadata evaporates across arithmetic/operations,
  and an immutable, detached scalar cannot write back to the AST. So Option 1
  cannot be uniform or write-through.
- **Item access should stay value-shaped.** Most code wants the plain value;
  making every leaf a wrapper (Options 1/2) would tax the common path and
  surprise users. A _separate_ cursor keeps `doc[...]` cheap and adds metadata
  only where asked.
- **A path-addressed handle composes with the existing model.** `YAMLRocksNode` reuses
  `resolve_path`/`child_ref` and the anchor map, so it never holds a stale
  reference and resolves aliases the same way views do.
- **Comment placement matches YAML.** `comment` is the value's inline comment;
  `comment_before` is the standalone comment above the node. For a mapping pair
  the "before" comment belongs to the key, except the first key of a mapping,
  whose leading comment the composer (ADR-011) stores on the enclosing mapping -
  `resolve_head` targets whichever node actually holds it so reads, writes, and
  emission agree.

**Surface**: `value` (get/set, write-through, preserves comments/anchor/tag),
`comment`/`comment_before` (get/set bare text, `None` clears), `line`/`column`
(1-based), `file`, `style` (`plain`/`single`/`double`/`literal`/`folded` for
scalars, `block`/`flow` for collections), `anchor`, `tag`, and `YAMLRocksNode`-returning
indexing.

**Consequences**:

- `comment_after` (foot) was deliberately **not** exposed: the emitter only
  renders foot comments at the document root, so a per-node setter would
  silently no-op, worse DX than omitting it. Foot comments still round-trip at
  the document level as before.
- Surfacing `style` revealed that the composer discarded the flow/block flag, so
  an _edited_ document re-emitted flow collections (`[a, b]`) as block. Fixed by
  threading the `flow` flag from `MappingStart`/`SequenceStart` into
  `NodeStyle`; flow layout now survives a re-emit, not just an untouched
  document.
