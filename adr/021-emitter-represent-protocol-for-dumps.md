# ADR-021: Emitter `represent` protocol for `dumps`

**Date**: 2026-07-07
**Status**: Accepted

**Context**: `dumps` can shape _unknown_ types (`serializers=` maps a Python type
to a custom `!tag`, `default=` fires for types nothing else handles), but it
cannot control how a _builtin_ renders. A host cannot say "emit this `str`
subclass masked", "format this `float` this exact way", "write this scalar as a
literal `|` block", or "force single quotes". PyYAML can, because its representers
dispatch on any type and return a node with an explicit tag and style.

This is the one gap keeping PyYAML as a dependency in downstreams that ship a
hand-rolled `Dumper` subclass (the concrete driver is ESPHome, whose parser
already moved to yamlrocks; only its `add_multi_representer` dumper remains on
`import yaml`).

**Decision**: Add one optional `dumps` parameter, `represent=`, a callable the
emitter invokes for **every** value it is about to emit, builtins included. It
returns either a node descriptor or `None`:

- `None` means "you handle it": the value falls through to the unchanged built-in
  dispatch, so a host only overrides the types it cares about and every deferred
  value renders byte-identically to a plain `dumps`.
- a descriptor, one of three shallow constructors the library exposes. They carry
  the `YAMLRocks` prefix like every other exported type, and because `Mapping` /
  `Sequence` unprefixed would shadow `typing`/`collections.abc`:
  - `YAMLRocksScalar(value: str, *, tag=None, style="auto")`
  - `YAMLRocksSequence(items, *, tag=None, flow=None)`
  - `YAMLRocksMapping(pairs, *, tag=None, flow=None)`

`YAMLRocksSequence.items` / `YAMLRocksMapping.pairs` carry the **original host
objects**, not pre-rendered nodes. The emitter re-dispatches each child through
`represent`, so recursion, anchors, indentation, and `sort_keys` stay inside the
library; the host only ever describes one level.

The `represent` path lowers to the round-trip `YamlNode` tree and emits through
the existing round-trip emitter, **not** the fast `Value` emitter. A descriptor
becomes a synthetic `YamlNode` with the requested style/tag/flow; a value the host
defers on (`represent` returns `None`) is lowered by the existing `python_to_node`
converter, the same one the document-edit path already uses. Then the tree is
emitted by `emit_roundtrip_*`.

**Route chosen (and why the first instinct was wrong):** the initial plan was to
add a per-scalar style override to the fast `Value` emitter. Investigation killed
it. The fast emitter cannot emit a literal `|` block at all, has no per-node style
or per-node flow, and always shows a tag; adding all of that to the hot emitter is
large and risky. Worse, a style channel on the decode-shared `Value` enum pollutes
~10 exhaustive match sites in decode/resolver/schema that must never see an
encode-only node. The round-trip `YamlNode` already models style (plain/single/
double/literal/folded), per-node tag, per-node flow, and anchors, and there is
already a working emitter for it plus a `python_to_node` converter. So the
represent path reuses that machinery instead of rebuilding it in the hot path.

The round-trip emitter is tuned for round-trip _fidelity_, not for _dumping_, so
the `represent` path adds dump-shaping the emitter did not have. All of the
following ship in v0.7 except `width`:

1. **Indented block sequences.** A synthetic sequence has no source column, so it
   would emit flush; a dump-config flag indents it a step under its key (the
   PyYAML style ESPHome requires).
2. **`sort_keys`.** `OPT_SORT_KEYS` sorts mapping keys on the `represent` path
   (the round-trip emitter otherwise preserves source order).
3. **PyYAML-faithful auto styling.** Under `style="auto"`, a standard tag the
   plain value already resolves to is elided, so a `!!float`-tagged `1.0e17`
   emits bare; a `!!str` on a number-looking value is quoted to stay a string;
   and a custom tag (`!extend`) is kept but force-quoted, because a plain form
   would resolve to a different tag and lose it (so `!extend my_id` emits as
   `!extend 'my_id'`). This lets a host's representers port verbatim, without
   hand-annotating a style on every call.
4. **Multiline literal default.** A deferred multi-line string defaults to a `|`
   literal block, matching the fast encoder (the literal-block decision is shared
   through `emit_util` so both agree).
5. **Anchors for shared objects.** An `id()`-keyed pass emits a shared container
   once with `&idNNN` and aliases the repeats (`*idNNN`), and resolves a cycle to
   an alias rather than looping. Scalars are never aliased, matching PyYAML.
6. **`width` (deferred).** Line-wrap composition on the `represent` path is not
   wired yet; it is the most negotiable per the reference consumer and lands
   later if needed.

**Rationale**:

- It is the general form of `serializers=`/`default=`, not a competing API: those
  keep working for the simple cases and compose with `represent`.
- Emitter-driven recursion falls out for free: the host returns original child
  objects, and the lowering recurses per child, so anchors, indentation, and
  sorting stay inside the library.
- Reuse over rebuild. The round-trip emitter already speaks every style the
  descriptor needs; the fast emitter speaks none of them and is the path we guard
  most. Deferred values stay consistent because they lower through the same
  `python_to_node` the edit path uses, which the round-trip suite already vets.

**Consequences**:

- `represent` is one more `Option` on the encode context, checked once per value
  like `default`. Absent, `dumps` is entirely unchanged and stays on the fast
  `Value` emitter, so there is no hot-path regression.
- When passed, `dumps` runs on the round-trip emitter instead of the fast one, and
  every value pays one Python call. Both are inherent to an opt-in per-value
  representer (PyYAML pays the call too) and are explicitly not the hot path.
- Two emitters now produce output, so deferred-value rendering under `represent`
  is byte-for-byte pinned against plain `dumps` in a conformance suite (mirroring
  the host's dumper) to catch drift.
- New public surface: `represent=` plus `YAMLRocksScalar`/`YAMLRocksSequence`/
  `YAMLRocksMapping` and a style literal. Additive; no existing behavior changes.
