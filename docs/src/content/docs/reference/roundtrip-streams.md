---
title: Round-trip document lists
description: Load each document as an independent round-trip object.
---

`loads_all(data, option=OPT_ROUND_TRIP)` and `load_all(source,
option=OPT_ROUND_TRIP)` return a list of independent `YAMLRocksDocument`
objects. Explicit empty documents are retained as null documents; an empty
stream returns an empty list. Anchors and version directives stay scoped to
their own documents. Joining the unedited documents' `to_yaml()` output
reproduces the decoded UTF-8 source, including separators and comments.

This API buffers the complete input. It is not an incremental stream reader.
