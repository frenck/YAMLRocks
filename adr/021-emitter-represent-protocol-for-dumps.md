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
becomes a synthetic `YamlNode` with the requested style/tag/flow. A value the host
defers on (`represent` returns `None`) is rendered the built-in way: a compound
(dict/list/tuple/set/dataclass/enum/numpy/tagged, or a `default` result)
decomposes into its child Python objects, which recurse back through `represent`
so every value is offered to the callback; a scalar leaf goes through the shared
`python_to_value` pipeline (the same one a plain `dumps` uses) and a
`Value`-to-node bridge, so `default`/`serializers` and datetime/dataclass/numpy
compose and deferred output is byte-for-byte a plain `dumps`. Then the tree is
emitted by `emit_roundtrip_*`.

**Route chosen (and why the first instinct was wrong):** the initial plan was to
add a per-scalar style override to the fast `Value` emitter. Investigation killed
it. The fast emitter auto-selects a literal `|` block only for eligible multi-line
strings; it has no caller-selected per-node style, no per-node flow, and always
shows a tag; adding all of that to the hot emitter is large and risky. Worse, a
style channel on the decode-shared `Value` enum pollutes
~10 exhaustive match sites in decode/resolver/schema that must never see an
encode-only node. The round-trip `YamlNode` already models style (plain/single/
double/literal/folded), per-node tag, per-node flow, and anchors, and there is
already a working emitter for it plus the `python_to_value` pipeline to reuse for
deferred leaves. So the represent path reuses that machinery instead of
rebuilding it in the hot path.

The round-trip emitter is tuned for round-trip _fidelity_, not for _dumping_, so
the `represent` path adds dump-shaping the emitter did not have. All of the
following ship in v0.7 except line width:

1. **Indented block sequences.** A synthetic sequence has no source column, so it
   would emit flush; a dump-config flag indents it a step under its key (the
   PyYAML style ESPHome requires).
2. **`sort_keys`.** `OPT_SORT_KEYS` sorts mapping keys by the fast path's
   comparator (type then value, numbers numerically), matching a plain `dumps`
   for primitive keys (`None`/bool/int/float/str/bytes). It runs in the
   lowering, before children are lowered, so anchor detection follows the final
   emission order. That ordering constraint is also why a key that needs a
   conversion (datetime, UUID, Path, Decimal, Enum, custom object) keeps
   insertion order instead: ranking it would run the conversion twice
   (observable side effects), and the sort cannot move after lowering without
   risking an alias emitted before its anchor. A documented divergence.
3. **PyYAML-faithful auto styling.** Under `style="auto"`, a standard tag the
   plain value already resolves to is elided, so a `!!float`-tagged `1.0e17`
   emits bare; a `!!str` on a number-looking value is quoted to stay a string;
   and a custom tag (`!extend`) is kept but force-quoted, matching PyYAML's
   default style for a tagged scalar (the tag survives a plain form too; quoting
   is the style, not what preserves the tag), so `!extend my_id` emits as
   `!extend 'my_id'`. This lets a host's representers port verbatim.
4. **Multiline literal default.** A deferred multi-line string defaults to a `|`
   literal block, matching the fast encoder (the literal-block decision is shared
   through `emit_util` so both agree).
5. **Anchors for shared objects.** An `id()`-keyed pass emits a shared object
   once with `&idNNN` and aliases the repeats (`*idNNN`), and resolves a cycle to
   an alias rather than looping. Aliasability matches PyYAML's `ignore_aliases`
   (everything except `None`/`bool`/`int`/`float`/`str`/`bytes`/`()`), so a shared
   set, dataclass, or custom object represented as a mapping is deduped too. A
   tag from the wrapper channel (`YAMLRocksTag`, a `serializers` result) belongs
   to that occurrence, not to the wrapped value, so the wrapped value is
   untracked: later bare occurrences lower fresh copies, matching a plain
   `dumps` byte-for-byte instead of minting an alias that would silently
   inherit the tag. Known limitation: a tag wrapping a value that was _already_
   emitted with an anchor (or that refers back to itself) raises, because a
   YAML alias cannot take the tag and the anchor cannot be revoked. A plain
   `dumps`, which never aliases, emits it twice instead. This is the one place
   the accepted anchor divergence surfaces as an error rather than differing
   bytes; it is safe (no silent data change).
6. **Every value, composed with `default`/`serializers`.** A deferred compound is
   decomposed into its child objects, which recurse through `represent`, so no
   value is skipped inside a set/dataclass/enum/numpy/`default` result; a deferred
   scalar leaf runs the shared pipeline, so `default`/`serializers` and
   datetime/dataclass/numpy handling compose and deferred output matches a plain
   `dumps`.
7. **Flow and document markers.** `OPT_FLOW_STYLE`, `OPT_EXPLICIT_START`, and
   `OPT_EXPLICIT_END` apply. A block scalar (`literal`/`folded`) that would land
   inside a flow collection is downgraded to a quoted style, since a block scalar
   is invalid there.
8. **No line width yet (deferred).** `OPT_INDENT_4` and
   `OPT_INDENTLESS_SEQUENCES` apply on this path (threaded through the dump
   config), so indentation matches a plain `dumps`. Only `width` line-wrapping is
   not wired; passing `width` with `represent` raises rather than silently
   diverging. It is the most negotiable per the reference consumer and lands later
   if needed.

**Rationale**:

- It is the general form of `serializers=`/`default=`, not a competing API: those
  keep working for the simple cases and compose with `represent`.
- Emitter-driven recursion falls out for free: the host returns original child
  objects, and the lowering recurses per child, so anchors, indentation, and
  sorting stay inside the library.
- Reuse over rebuild. The round-trip emitter already speaks every style the
  descriptor needs; the fast emitter speaks none of them and is the path we guard
  most. Deferred scalar leaves stay consistent because they run the same
  `python_to_value` pipeline a plain `dumps` uses; only the structural
  decomposition of a deferred compound (which child objects it holds) is
  reimplemented on the represent side, and a byte-for-byte parity test guards the
  two against drift.

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
