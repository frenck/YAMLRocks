# ADR-014: JSON export via `to_json`; no `from_json`, no async serializer

**Date**: 2026-06-08
**Status**: Accepted

**Context**: YAML is a superset of JSON, and users routinely need to convert
between the two. The question was how much API surface this warrants: a
`to_json`/`from_json` pair, partial import/export into a tree, separate update
handling, and async variants were all on the table.

**Decision**: Ship a single `to_json(obj)` that returns JSON `bytes`, the
counterpart of `dumps`. Add **no** `from_json` and **no** `async_to_json`.

**Rationale**:

- **Import is already free.** Every valid JSON document is valid YAML 1.2, so
  `loads` already parses JSON. A `from_json` would be a confusing alias.
- **Sub-tree export falls out of the existing model.** Because `to_json` accepts
  a `YAMLRocksDocument` or `YAMLRocksDocumentView`, exporting a sub-tree is just
  `to_json(doc["service"])`: no new "export parts" machinery is needed. The
  feared complexity (import/export-parts/update) was already solved by the
  round-trip primitives.
- **The projection is lossy but deterministic.** JSON is the lossy subset of
  YAML, so `to_json` fixes a consistent projection: tags dropped, non-finite
  floats become `null`, non-string scalar keys are stringified (matching the
  canonical YAML-to-JSON mapping), and a collection used as a key is the one
  thing JSON cannot represent, so it raises rather than guessing.
- **No async serializer.** `async_dumps`/`async_to_json` were removed (and never
  shipped) because serializing holds the GIL for the Python object traversal, so
  a worker-thread offload buys almost nothing. Loading is different: the native
  parse releases the GIL on byte input, so `async_loads`/`async_load` stay.

**Consequences**:

- A thin `encode/json.rs` emitter is the only new code; it reuses the existing
  `python_to_value` + `EncodeCtx` pipeline and the `default=` callback.
- Users who genuinely need off-loop serialization wrap the sync call themselves
  with `asyncio.to_thread(to_json, obj)`.
