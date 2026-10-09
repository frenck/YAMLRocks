---
title: Width with represented output
description: Use width together with custom representation.
---

`dumps(..., represent=callback, width=80)` now supports the same soft width
setting as ordinary dumping. Quoted scalars wrap at safe spaces, long plain
scalars can be quoted for safe folding, and flow collections break between
entries. Keys stay on one line. Literal/folded block scalar styles retain
their existing content and layout. `width=0` disables wrapping.

Scalar folding shares the native encoder's implementation. An existing
round-trip document still preserves its own layout when passed to `dumps`.
