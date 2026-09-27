---
title: Comments on represented values
description: Restore comments when constructing a new YAML output document.
---

`YAMLRocksScalar`, `YAMLRocksSequence`, and `YAMLRocksMapping` accept the
keyword-only arguments `comment`, `comment_before`, and `comment_after`.
Use them with `dumps(..., represent=...)` to construct output with comments,
styles, and tags from fresh Python data, without reparsing generated YAML.

Comments contain text without a leading `#`. Before/after comments can contain
multiple lines. A mapping value's `comment_before` appears above its key.
The existing round-trip emitter owns comment placement and formatting.
