---
title: Bulk tree snapshots
description: Read values and presentation in one linear traversal.
---

`doc.to_tree()` returns an immutable `YAMLRocksTreeNode` snapshot. Unlike a
live cursor, it owns its children and never searches a path from the root.
Exporting and walking a wide mapping takes linear work in its node count.
The original document can then be edited or released independently.

Each node has `kind`, `value`, `style`, `tag`, `anchor`, `comment`,
`comment_before`, `comment_after`, and `is_merge_key` properties. Comments
are attached to their actual AST node: a comment above a mapping entry belongs
to its key node. They contain text without the `#` prefix.

| kind | value |
| --- | --- |
| scalar | Python value resolved under the document's schema |
| sequence | Tuple of child nodes |
| mapping | Tuple of `(key_node, value_node)` pairs, in source order |
| alias | Anchor name, as a string |

Aliases and merge keys remain explicit, so callers can choose expansion.
Quoted `"<<"` keys have `is_merge_key=False`; syntax recognition and scalar
resolution are performed by YAMLRocks. Scalar styles are `plain`, `single`,
`double`, `literal`, or `folded`; collections use `block` or `flow`.

An empty document wrapper returns `None`. A wrapper with multiple roots raises;
obtain individual wrappers using `loads_all(..., option=OPT_ROUND_TRIP)`.
