---
title: Document start markers
description: Inspect explicit document starts without scanning source text.
---

`YAMLRocksDocument.explicit_start` reports whether the first document began
with an explicit `---` marker. It reads the parsed document metadata, including
when comments, a BOM, or a version directive precede the marker. It is read-only
and remains available after editing the document.

```python
import yamlrocks

doc = yamlrocks.loads(b"---\na: 1\n", option=yamlrocks.OPT_ROUND_TRIP)
assert doc.explicit_start is True
```
