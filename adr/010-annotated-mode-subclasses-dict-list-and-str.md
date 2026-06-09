# ADR-010: Annotated mode subclasses dict, list, and str

**Date**: 2026-06-08
**Status**: Accepted

**Context**: `OPT_ANNOTATED` must return values carrying `__line__`,
`__column__`, and `__file__`, compatible with Home Assistant's
`NodeDictClass`/`NodeListClass`/`NodeStrClass`.

**Decision**: Return `YAMLRocksAnnotatedDict(dict)` and `YAMLRocksAnnotatedList(list)`
natively via `#[pyclass(extends = …)]`, and `YAMLRocksAnnotatedStr` as a thin
pure-Python `str` subclass. The remaining scalars (`int`, `float`, `bool`, `None`)
stay plain by default; `int` and `float` can be annotated by opting into
`OPT_ANNOTATE_NUMBERS`.

**Rationale**:

- PyO3 can subclass the variable-length built-ins `dict` and `list`, but not the
  immutable types `str`/`int`/`bytes`. A `str` subclass therefore needs a small
  pure-Python shim, which is cheap enough to be worth the source tracking it buys
  (strings are the next most common error target after mappings and sequences).
- Mappings and sequences are where source locations matter most for config
  validation error messages, the primary Home Assistant use case.
- `bool` and `None` can never be annotated (Python forbids subclassing them),
  which matches PyYAML.

**Consequences**:

- `YAMLRocksAnnotatedStr` lives in Python (`pysrc`) as a thin `str` subclass whose
  `__new__` attaches the location attributes; the `int`/`float` variants for
  `OPT_ANNOTATE_NUMBERS` are pure-Python too, while `YAMLRocksAnnotatedDict`/`List`
  are native Rust pyclasses.
- Numeric annotation is off by default so the common path stays plain and fast;
  see `OPT_ANNOTATE_NUMBERS`.
- Annotated values deliberately do not back-reference the round-trip AST; see
  ADR-020 for why annotated mode and the `YAMLRocksNode` cursor stay separate.
