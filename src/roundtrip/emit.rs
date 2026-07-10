//! Round-trip emitter: serializes a rich [`YamlNode`] AST back to YAML bytes,
//! preserving comments, scalar styles, anchors, tags, and block/flow layout.
//!
//! The emitter mirrors the structure of the fast-path encoder but additionally
//! threads comments through every node. Head comments are emitted on their own
//! lines above a node, inline comments trail a value on the same line, and foot
//! comments follow a node after a blank line.

use super::ast::{Comments, IncludeSource, NodeStyle, YamlNode, YamlNodeKind};
use crate::encode::NullStyle;
use crate::scanner::ScalarStyle;

/// Indentation step (spaces per nesting level) for emitted block structures.
const STEP: usize = 2;

/// The UTF-8 encoding of a byte order mark (U+FEFF), restored at the head of the
/// stream when the source carried one so a round-trip stays byte-for-byte.
const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Emit a single document AST to YAML bytes, rendering synthetic (edited-in)
/// nulls in `null_style`.
pub fn emit_roundtrip_with(node: &YamlNode, null_style: NullStyle) -> Vec<u8> {
    let mut emitter = RoundTripEmitter::new(null_style);
    if node.leading_bom {
        emitter.buf.extend_from_slice(BOM);
    }
    emitter.emit_document(node);
    emitter.buf
}

/// Emit multiple document ASTs, separated by `---` markers, rendering synthetic
/// nulls in `null_style`.
pub fn emit_roundtrip_all_with(nodes: &[YamlNode], null_style: NullStyle) -> Vec<u8> {
    let mut emitter = RoundTripEmitter::new(null_style);
    if nodes.first().is_some_and(|n| n.leading_bom) {
        emitter.buf.extend_from_slice(BOM);
    }
    for (i, node) in nodes.iter().enumerate() {
        // A directive (`%TAG`/`%YAML`) is only valid at the stream start or right
        // after a `...` end marker, so a later document carrying directives needs
        // the previous one closed first. `emit_document` then emits the directives
        // and the `---`.
        if i > 0 && !node.directives.is_empty() {
            emitter.buf.extend_from_slice(b"...\n");
        } else if i > 0 && !node.explicit_start {
            // Documents need a `---` separator. A document that carries its own
            // explicit start marker emits it in `emit_document`, so only add the
            // separator for one that does not (avoiding a double `---`).
            emitter.buf.extend_from_slice(b"---\n");
        }
        emitter.emit_document(node);
    }
    emitter.buf
}

/// Emit a single document with the default null style (`null` keyword). Used by
/// include write-back, where there is no document-level style to thread.
pub fn emit_roundtrip(node: &YamlNode) -> Vec<u8> {
    emit_roundtrip_with(node, NullStyle::Null)
}

/// Dump-shaping options for a *synthetic* tree emitted through the round-trip
/// emitter (the `dumps(represent=...)` path). They only affect edited-in nodes
/// with no source layout to preserve, so the fidelity guarantee for loaded
/// documents (which use [`emit_roundtrip_with`], leaving these at their default)
/// is untouched.
#[derive(Debug, Clone, Copy, Default)]
pub struct DumpConfig {
    /// Indent a block sequence a step under its key (`k:\n  - a`, the PyYAML
    /// style) rather than at the key's own column. Synthetic sequences have no
    /// recorded source column, so without this they emit flush.
    pub indent_sequences: bool,
    /// Emit an explicit `...` end marker after the document (`OPT_EXPLICIT_END`).
    /// The `---` start marker is carried on the root node's `explicit_start`.
    pub explicit_end: bool,
    /// Spaces per nesting level for mappings and block scalars under a key: the
    /// default two, or four when `OPT_INDENT_4` is set. Zero means "unset", read
    /// through [`RoundTripEmitter::step`] as the default [`STEP`]. Block-sequence
    /// item content is always two columns past the dash regardless, matching the
    /// fast encoder.
    pub indent: usize,
    /// Align a block sequence under a key with the key's own column instead of
    /// indenting it a step (`OPT_INDENTLESS_SEQUENCES`). Only consulted on the
    /// dump path (`indent_sequences`).
    pub indentless: bool,
}

/// Emit a single synthetic document tree with dump-shaping options applied. Used
/// by the `represent` emitter path, which builds edited-in nodes and wants a
/// PyYAML-style dump rather than round-trip fidelity.
pub fn emit_roundtrip_dump(node: &YamlNode, null_style: NullStyle, dump: DumpConfig) -> Vec<u8> {
    let mut emitter = RoundTripEmitter::new(null_style);
    emitter.dump = dump;
    if node.leading_bom {
        emitter.buf.extend_from_slice(BOM);
    }
    emitter.emit_document(node);
    if dump.explicit_end {
        emitter.buf.extend_from_slice(b"...\n");
    }
    emitter.buf
}

struct RoundTripEmitter {
    buf: Vec<u8>,
    /// Dump-shaping options for a synthetic tree (the `represent` path). Default
    /// (all off) preserves the fidelity behavior for loaded documents.
    dump: DumpConfig,
    /// How a synthetic (edited-in) null is rendered. Loaded nulls ignore this and
    /// re-emit in their original form.
    null_style: NullStyle,
    /// Depth of flow collections (`{}`/`[]`) currently open. When non-zero, a
    /// plain scalar that contains a flow indicator (`,` `[` `]` `{` `}` `: `) must
    /// be quoted: those characters are ordinary text in block context (a loaded
    /// plain scalar may hold them) but would end the entry or collection early
    /// inside flow. An edited-in value assigned into a flow collection is the case
    /// that reaches here as a bare plain scalar.
    flow_depth: usize,
}

impl RoundTripEmitter {
    fn new(null_style: NullStyle) -> Self {
        Self {
            buf: Vec::with_capacity(256),
            dump: DumpConfig::default(),
            null_style,
            flow_depth: 0,
        }
    }

    /// Spaces per nesting level for mappings and block scalars under a key: the
    /// dump config's indent when set (four for `OPT_INDENT_4`), else the default
    /// [`STEP`] of two. Block-sequence item content is always `+2` past the dash
    /// (the fast encoder's fixed offset) and does not go through here.
    fn step(&self) -> usize {
        if self.dump.indent > 0 {
            self.dump.indent
        } else {
            STEP
        }
    }

    /// The null style for `node` if it is a synthetic null, else `None` (a loaded
    /// null, which always keeps its original rendering).
    fn synthetic_null(&self, node: &YamlNode) -> Option<NullStyle> {
        if node.synthetic && matches!(node.kind, YamlNodeKind::Null) {
            Some(self.null_style)
        } else {
            None
        }
    }

    // -- Document entry --

    fn emit_document(&mut self, node: &YamlNode) {
        // Directives precede the `---` that opens their document. A `%TAG` handle
        // is in scope only for this document, so replaying it keeps a tagged node
        // (`!e!foo`) resolvable when the edited document reloads.
        for directive in &node.directives {
            self.buf.push(b'%');
            self.buf.extend_from_slice(directive.as_bytes());
            self.buf.push(b'\n');
        }
        if node.explicit_start {
            self.buf.extend_from_slice(b"---\n");
        }
        self.emit_head(&node.comments, 0);
        if let Some(ref source) = node.source {
            self.emit_directive(source);
            self.emit_inline_comment(&node.comments);
            self.buf.push(b'\n');
            self.emit_foot(&node.comments, 0);
            return;
        }
        match &node.kind {
            YamlNodeKind::Mapping(pairs) if node.style == NodeStyle::Block && !pairs.is_empty() => {
                self.emit_anchor_tag_line(node, 0);
                self.emit_block_mapping(pairs, 0);
            }
            YamlNodeKind::Sequence(items)
                if node.style == NodeStyle::Block && !items.is_empty() =>
            {
                self.emit_anchor_tag_line(node, 0);
                self.emit_block_sequence(items, 0);
            }
            _ => {
                self.emit_anchor_tag(node);
                // A root block scalar's body sits one step in, matching the fast
                // encoder's `emit_literal_block(s, step())`.
                self.emit_inline_content(node, self.step());
                self.emit_inline_comment(&node.comments);
                self.end_line();
            }
        }
        self.emit_foot(&node.comments, 0);
    }

    // -- Block mapping --

    /// Emit the blank source lines recorded before a node, preserving the
    /// author's section spacing on re-emit.
    fn emit_blank_before(&mut self, comments: &Comments) {
        for _ in 0..comments.blank_before {
            self.buf.push(b'\n');
        }
    }

    fn emit_block_mapping(&mut self, pairs: &[(YamlNode, YamlNode)], indent: usize) {
        for (i, (key, val)) in pairs.iter().enumerate() {
            self.emit_blank_before(&key.comments);
            self.emit_head(&key.comments, indent);
            self.write_indent(indent);
            if key_needs_explicit(key) || synthetic_key_needs_explicit(key, i == 0) {
                self.emit_explicit_key_pair(key, val, indent);
                continue;
            }
            self.emit_anchor_tag(key);
            self.emit_inline_content(key, indent);
            self.buf.push(b':');
            self.emit_value_after_colon(val, indent);
        }
    }

    /// Emit a mapping whose first pair shares the line opened by a sequence
    /// dash (`- key: value`). `indent` is the key column.
    fn emit_block_mapping_after_dash(&mut self, pairs: &[(YamlNode, YamlNode)], indent: usize) {
        for (i, (key, val)) in pairs.iter().enumerate() {
            // The first pair shares the dash line: the caller has already left
            // the cursor at the key column, so do not re-indent it. Later pairs
            // open their own line.
            if i > 0 {
                self.emit_blank_before(&key.comments);
                self.emit_head(&key.comments, indent);
                self.write_indent(indent);
            }
            if key_needs_explicit(key) || synthetic_key_needs_explicit(key, i == 0) {
                self.emit_explicit_key_pair(key, val, indent);
                continue;
            }
            self.emit_anchor_tag(key);
            self.emit_inline_content(key, indent);
            self.buf.push(b':');
            self.emit_value_after_colon(val, indent);
        }
    }

    /// Emit one mapping pair in explicit `?`/`:` block form, for a complex key
    /// that has no valid inline rendering (see [`key_needs_explicit`]).
    fn emit_explicit_key_pair(&mut self, key: &YamlNode, val: &YamlNode, indent: usize) {
        // The cursor already sits at the key column (the caller wrote the
        // indentation or a `- ` dash). Emit `? <key>` then `: <value>`, with both
        // indicators at `indent` and the key/value blocks one step deeper.
        let child = indent + STEP;
        self.buf.push(b'?');
        match &key.kind {
            YamlNodeKind::Mapping(m) if key.style == NodeStyle::Block && !m.is_empty() => {
                self.emit_anchor_tag_compact(key);
                self.buf.push(b'\n');
                self.emit_block_mapping(m, child);
            }
            YamlNodeKind::Sequence(s) if key.style == NodeStyle::Block && !s.is_empty() => {
                self.emit_anchor_tag_compact(key);
                self.buf.push(b'\n');
                self.emit_block_sequence(s, child);
            }
            // A block scalar (literal/folded) key, plus any other inline content
            // routed here defensively. The block scalar opens on the `? ` line and
            // indents its body under the key.
            _ => {
                self.buf.push(b' ');
                self.emit_anchor_tag(key);
                self.emit_inline_content(key, indent + self.step());
                self.end_line();
            }
        }
        self.write_indent(indent);
        self.buf.push(b':');
        self.emit_value_after_colon(val, indent);
    }

    /// Emit everything after a mapping key's colon.
    fn emit_value_after_colon(&mut self, val: &YamlNode, indent: usize) {
        // Grow the native stack on demand so emitting a deeply nested document
        // cannot overflow a small thread stack; this is one of the per-level
        // descent gateways. See [`crate::stack`].
        crate::stack::guard(|| self.emit_value_after_colon_inner(val, indent))
    }

    fn emit_value_after_colon_inner(&mut self, val: &YamlNode, indent: usize) {
        let child = indent + self.step();
        if let Some(ref source) = val.source {
            self.buf.push(b' ');
            self.emit_directive(source);
            self.emit_inline_comment(&val.comments);
            self.buf.push(b'\n');
            return;
        }
        match &val.kind {
            YamlNodeKind::Mapping(m) if val.style == NodeStyle::Block && !m.is_empty() => {
                self.emit_anchor_tag_compact(val);
                self.emit_inline_comment(&val.comments);
                self.buf.push(b'\n');
                self.emit_head(&val.comments, child);
                self.emit_block_mapping(m, child);
                // A trailing comment block at the end of the mapping, indented
                // with its keys.
                self.emit_foot(&val.comments, child);
            }
            YamlNodeKind::Sequence(s) if val.style == NodeStyle::Block && !s.is_empty() => {
                self.emit_anchor_tag_compact(val);
                self.emit_inline_comment(&val.comments);
                self.buf.push(b'\n');
                // Preserve the source's block-sequence indentation. YAML allows a
                // sequence under a key to sit at the key's own column (the
                // "compact" style, ubiquitous in Home Assistant configs) or
                // indented a step further. The composer records the `-` column on
                // the sequence node's span, so re-emit it where it was written
                // instead of always indenting (which would reflow every list).
                // On the dump path (`represent`), synthetic sequences have no
                // source column, so indent a step under the key (the PyYAML style)
                // instead of the flush default, unless indentless is requested
                // (`OPT_INDENTLESS_SEQUENCES`), which aligns the dashes with the
                // key's own column, matching the fast encoder.
                let seq_indent = if self.dump.indent_sequences {
                    if self.dump.indentless {
                        indent
                    } else {
                        child
                    }
                } else if (val.span.column as usize) <= indent {
                    indent
                } else {
                    child
                };
                self.emit_head(&val.comments, seq_indent);
                self.emit_block_sequence(s, seq_indent);
                // A trailing comment block at the end of the sequence, aligned
                // with its dashes.
                self.emit_foot(&val.comments, seq_indent);
            }
            // A `key:` with no value: emitted empty for a loaded null (preserving
            // it) and for a synthetic null whose style is `empty`. A synthetic
            // null styled `null`/`~` falls through to the inline arm below. A tag
            // is kept as a bare `key: !x` (never collapsed to `key:`, which drops
            // it, nor expanded to `key: !x null`, which reloads as the string
            // "null" rather than an empty value); this matches the fast path.
            _ if is_empty_scalar(val)
                && val.comments.inline.is_none()
                && val.anchor.is_none()
                && self
                    .synthetic_null(val)
                    .map_or(true, |s| s == NullStyle::Empty) =>
            {
                if let Some(ref tag) = val.tag {
                    self.buf.push(b' ');
                    self.buf.extend_from_slice(tag.as_bytes());
                }
                self.buf.push(b'\n');
            }
            _ => {
                // Restore the padding between the `:` and an inline value
                // (`example:      true`); one space for a synthetic value.
                for _ in 0..val.comments.value_pad.max(1) {
                    self.buf.push(b' ');
                }
                self.emit_anchor_tag(val);
                // A block-scalar value's body sits one step past the key, matching
                // the fast encoder (`emit_literal_block(s, indent + step())`).
                self.emit_inline_content(val, indent + self.step());
                self.emit_inline_comment(&val.comments);
                // Block scalars already end with their own newline(s).
                self.end_line();
            }
        }
    }

    /// Ensure the buffer ends with exactly one newline (block scalars emit
    /// their own trailing newlines; plain scalars do not).
    fn end_line(&mut self) {
        if self.buf.last() != Some(&b'\n') {
            self.buf.push(b'\n');
        }
    }

    // -- Block sequence --

    fn emit_block_sequence(&mut self, items: &[YamlNode], indent: usize) {
        for item in items {
            self.emit_blank_before(&item.comments);
            self.emit_head(&item.comments, indent);
            self.write_indent(indent);
            self.emit_sequence_item_body(item, indent);
        }
    }

    /// Emit a block sequence whose first item shares the line opened by a parent
    /// sequence dash (`- - 1`, the compact nested form). The cursor already sits
    /// just past the parent `- `, so the first item is not re-indented; later
    /// items open their own line at `indent`.
    fn emit_block_sequence_after_dash(&mut self, items: &[YamlNode], indent: usize) {
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.emit_blank_before(&item.comments);
                self.emit_head(&item.comments, indent);
                self.write_indent(indent);
            }
            self.emit_sequence_item_body(item, indent);
        }
    }

    /// Emit one block-sequence item starting at the `-` (the caller has written
    /// any indentation). `indent` is the dash column.
    fn emit_sequence_item_body(&mut self, item: &YamlNode, indent: usize) {
        // A per-level descent gateway for block sequence items; grow the stack on
        // demand so deep nesting cannot overflow a small thread stack. See
        // [`crate::stack`].
        crate::stack::guard(|| self.emit_sequence_item_body_inner(item, indent))
    }

    fn emit_sequence_item_body_inner(&mut self, item: &YamlNode, indent: usize) {
        let child = indent + STEP;
        self.buf.push(b'-');
        if let Some(ref source) = item.source {
            self.buf.push(b' ');
            self.emit_directive(source);
            self.emit_inline_comment(&item.comments);
            self.buf.push(b'\n');
            return;
        }
        // A bare `-` (an empty entry) re-emits as a bare `-`: a loaded null
        // (originally written `-`, not `- null`), or a synthetic null whose
        // style is empty. An explicit `- null`/`- ~` is a scalar, not a null
        // node, so it falls through and keeps its spelling. A tag is kept as a
        // bare `- !x` (never expanded to `- !x null`, which reloads as the
        // string "null" rather than an empty value); this matches the fast path.
        if is_empty_scalar(item)
            && item.comments.inline.is_none()
            && item.anchor.is_none()
            && self
                .synthetic_null(item)
                .map_or(true, |style| style == NullStyle::Empty)
        {
            if let Some(ref tag) = item.tag {
                self.buf.push(b' ');
                self.buf.extend_from_slice(tag.as_bytes());
            }
            self.buf.push(b'\n');
            return;
        }
        match &item.kind {
            YamlNodeKind::Mapping(m) if item.style == NodeStyle::Block && !m.is_empty() => {
                if item.anchor.is_some() || item.tag.is_some() {
                    // An anchored/tagged block mapping carries its marker on the
                    // dash line, then breaks to the indented keys. Emitting it
                    // inline (`- &a key: value`) would bind the marker to the
                    // first key rather than the mapping.
                    self.emit_anchor_tag_compact(item);
                    self.buf.push(b'\n');
                    self.emit_block_mapping(m, child);
                } else {
                    self.buf.push(b' ');
                    self.emit_block_mapping_after_dash(m, child);
                }
                // A trailing comment block at the end of this item's mapping.
                self.emit_foot(&item.comments, child);
            }
            YamlNodeKind::Sequence(s) if item.style == NodeStyle::Block && !s.is_empty() => {
                if item.anchor.is_some() || item.tag.is_some() {
                    // An anchored/tagged nested block sequence carries its marker
                    // on the dash line, then breaks to the indented items (the
                    // compact `- -` form cannot hold a leading `&anchor`/tag).
                    self.emit_anchor_tag_compact(item);
                    self.buf.push(b'\n');
                    self.emit_block_sequence(s, child);
                } else if item.comments.compact {
                    // A compact nested sequence (`- - 1`) keeps its first item on
                    // this dash line; otherwise it breaks to the next, indented.
                    self.buf.push(b' ');
                    self.emit_block_sequence_after_dash(s, child);
                } else {
                    self.buf.push(b'\n');
                    self.emit_block_sequence(s, child);
                }
                // A trailing comment block at the end of this item's sequence.
                self.emit_foot(&item.comments, child);
            }
            _ => {
                // Restore the padding between the `-` and an inline item
                // (`-    item`); one space for a synthetic item.
                for _ in 0..item.comments.value_pad.max(1) {
                    self.buf.push(b' ');
                }
                self.emit_anchor_tag(item);
                self.emit_inline_content(item, child);
                self.emit_inline_comment(&item.comments);
                self.end_line();
            }
        }
    }

    // -- Inline content (scalars, aliases, flow collections) --

    /// Emit a node's value content with no surrounding indentation, head
    /// comment, or trailing newline. `indent` positions block scalar bodies.
    fn emit_inline_content(&mut self, node: &YamlNode, indent: usize) {
        // The per-level descent gateway for flow collections; grow the stack on
        // demand so deep nesting cannot overflow a small thread stack. See
        // [`crate::stack`].
        crate::stack::guard(|| self.emit_inline_content_inner(node, indent))
    }

    fn emit_inline_content_inner(&mut self, node: &YamlNode, indent: usize) {
        match &node.kind {
            // A synthetic null styled `~` renders as the tilde indicator; every
            // other null (loaded, or styled `null`/empty in a position where empty
            // is unsafe) renders as the `null` keyword.
            YamlNodeKind::Null => {
                let token = if matches!(self.synthetic_null(node), Some(NullStyle::Tilde)) {
                    b"~".as_slice()
                } else {
                    b"null".as_slice()
                };
                self.buf.extend_from_slice(token);
            }
            YamlNodeKind::Scalar(value, style) => {
                // An unmodified block scalar replays its verbatim source: folding
                // a `>` block is lossy, so the original lines cannot be rebuilt.
                if let Some(raw) = node.comments.raw.as_deref() {
                    if !node.comments.modified {
                        self.buf.extend_from_slice(raw.as_bytes());
                        return;
                    }
                }
                self.emit_scalar(value, *style, indent);
            }
            YamlNodeKind::Alias(name) => {
                self.buf.push(b'*');
                self.buf.extend_from_slice(name.as_bytes());
            }
            YamlNodeKind::Sequence(items) => self.emit_flow_sequence(items, indent),
            YamlNodeKind::Mapping(pairs) => self.emit_flow_mapping(pairs, indent),
        }
    }

    fn emit_scalar(&mut self, value: &str, style: ScalarStyle, body_indent: usize) {
        match style {
            // A plain scalar whose first character is U+FEFF cannot be emitted
            // verbatim: if it lands at the start of the stream, the scanner
            // strips the leading byte order mark and the remainder reparses as
            // something else (e.g. `\u{feff}*` would expose a bare `*` alias and
            // fail to parse). Escalate to a double-quoted scalar, which encodes
            // the mark as content regardless of position. The BOM is the only
            // plain-content character with this hazard, since the scanner never
            // produces a plain scalar starting with a bare indicator. Found by
            // the `roundtrip` fuzz target.
            ScalarStyle::Plain if value.starts_with('\u{feff}') => {
                crate::emit_util::push_double_quoted(&mut self.buf, value);
            }
            // Inside a flow collection a bare `,`/`[`/`]`/`{`/`}`/`: ` would end
            // the entry or collection early, so a plain scalar carrying one must
            // be quoted. This only bites an edited-in value: a loaded plain scalar
            // never contains a bare flow indicator (the scanner would not produce
            // it). See [`plain_unsafe_in_flow`].
            ScalarStyle::Plain if self.flow_depth > 0 && plain_unsafe_in_flow(value) => {
                crate::emit_util::push_double_quoted(&mut self.buf, value);
            }
            ScalarStyle::Plain => self.buf.extend_from_slice(value.as_bytes()),
            ScalarStyle::SingleQuoted => crate::emit_util::push_single_quoted(&mut self.buf, value),
            ScalarStyle::DoubleQuoted => crate::emit_util::push_double_quoted(&mut self.buf, value),
            ScalarStyle::Literal => self.emit_block_scalar(value, body_indent, b'|'),
            ScalarStyle::Folded => self.emit_block_scalar(value, body_indent, b'>'),
        }
    }

    /// Emit a block scalar whose body lines sit at the absolute column
    /// `body_indent`. The caller computes that column for its context (a mapping
    /// value: `key + step`; a sequence item: `dash + 2`; the document root:
    /// `step`), matching the fast encoder rather than adding a fixed step here.
    fn emit_block_scalar(&mut self, value: &str, body_indent: usize, marker: u8) {
        // The scanner already applied chomping when producing `value`, so we
        // reverse-engineer the indicator from its trailing newlines:
        //   0 trailing  → strip  (`-`)
        //   1 trailing  → clip   (default, no indicator)
        //   2+ trailing → keep   (`+`), preserving the extra blank lines
        let trailing = value.bytes().rev().take_while(|&b| b == b'\n').count();
        let body = value.trim_end_matches('\n');

        self.buf.push(marker);
        match trailing {
            0 => self.buf.push(b'-'),
            // Clip (a single trailing newline) cannot represent an all-newline
            // value whose body is empty (`"\n"`): the body collapses to nothing
            // and the lone newline is chomped away on re-read. Keep (`+`) so the
            // trailing newline survives. Mirrors the fast encoder.
            1 if body.is_empty() => self.buf.push(b'+'),
            1 => {}
            _ => self.buf.push(b'+'),
        }
        self.buf.push(b'\n');

        for line in body.split('\n') {
            if line.is_empty() {
                self.buf.push(b'\n');
            } else {
                self.write_indent(body_indent);
                self.buf.extend_from_slice(line.as_bytes());
                self.buf.push(b'\n');
            }
        }
        // For "keep", emit the blank lines beyond the single implicit newline.
        for _ in 1..trailing {
            self.buf.push(b'\n');
        }
    }

    fn emit_flow_sequence(&mut self, items: &[YamlNode], indent: usize) {
        self.buf.push(b'[');
        self.flow_depth += 1;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.buf.extend_from_slice(b", ");
            }
            self.emit_anchor_tag(item);
            self.emit_inline_content(item, indent);
        }
        self.flow_depth -= 1;
        self.buf.push(b']');
    }

    fn emit_flow_mapping(&mut self, pairs: &[(YamlNode, YamlNode)], indent: usize) {
        self.buf.push(b'{');
        self.flow_depth += 1;
        for (i, (key, val)) in pairs.iter().enumerate() {
            if i > 0 {
                self.buf.extend_from_slice(b", ");
            }
            self.emit_anchor_tag(key);
            self.emit_inline_content(key, indent);
            self.buf.extend_from_slice(b": ");
            self.emit_anchor_tag(val);
            self.emit_inline_content(val, indent);
        }
        self.flow_depth -= 1;
        self.buf.push(b'}');
    }

    // -- Comments --

    fn emit_head(&mut self, comments: &Comments, indent: usize) {
        for comment in &comments.head {
            self.emit_comment_line(comment, indent);
        }
    }

    fn emit_foot(&mut self, comments: &Comments, indent: usize) {
        for comment in &comments.foot {
            self.emit_comment_line(comment, indent);
        }
    }

    fn emit_comment_line(&mut self, comment: &str, indent: usize) {
        self.write_indent(indent);
        self.buf.push(b'#');
        if !comment.is_empty() {
            self.buf.push(b' ');
            self.buf.extend_from_slice(comment.as_bytes());
        }
        self.buf.push(b'\n');
    }

    fn emit_inline_comment(&mut self, comments: &Comments) {
        if let Some(ref comment) = comments.inline {
            // Restore the original padding before the `#` (alignment), falling
            // back to a single space for a synthetic comment (`inline_spaces` 0).
            for _ in 0..comments.inline_spaces.max(1) {
                self.buf.push(b' ');
            }
            self.buf.push(b'#');
            if !comment.is_empty() {
                self.buf.push(b' ');
                self.buf.extend_from_slice(comment.as_bytes());
            }
        }
    }

    // -- Anchors & tags --

    fn emit_anchor_tag(&mut self, node: &YamlNode) {
        if let Some(ref tag) = node.tag {
            self.buf.extend_from_slice(tag.as_bytes());
            self.buf.push(b' ');
        }
        if let Some(ref anchor) = node.anchor {
            self.buf.push(b'&');
            self.buf.extend_from_slice(anchor.as_bytes());
            self.buf.push(b' ');
        }
    }

    /// Emit a node's tag/anchor inline before a block body that follows on the
    /// next line, e.g. the `&a` in `base: &a`. Adds a leading space and no
    /// trailing space so a newline can follow cleanly.
    fn emit_anchor_tag_compact(&mut self, node: &YamlNode) {
        if let Some(ref tag) = node.tag {
            self.buf.push(b' ');
            self.buf.extend_from_slice(tag.as_bytes());
        }
        if let Some(ref anchor) = node.anchor {
            self.buf.push(b' ');
            self.buf.push(b'&');
            self.buf.extend_from_slice(anchor.as_bytes());
        }
    }

    /// Emit an anchor/tag for a block collection on its own preceding line.
    fn emit_anchor_tag_line(&mut self, node: &YamlNode, indent: usize) {
        if node.anchor.is_none() && node.tag.is_none() {
            return;
        }
        self.write_indent(indent);
        if let Some(ref tag) = node.tag {
            self.buf.extend_from_slice(tag.as_bytes());
            if node.anchor.is_some() {
                self.buf.push(b' ');
            }
        }
        if let Some(ref anchor) = node.anchor {
            self.buf.push(b'&');
            self.buf.extend_from_slice(anchor.as_bytes());
        }
        self.buf.push(b'\n');
    }

    /// Emit an include directive in value position, e.g. `!include foo.yaml`.
    fn emit_directive(&mut self, source: &IncludeSource) {
        self.buf.extend_from_slice(source.tag.as_bytes());
        self.buf.push(b' ');
        self.buf.extend_from_slice(source.target.as_bytes());
    }

    fn write_indent(&mut self, indent: usize) {
        self.buf.resize(self.buf.len() + indent, b' ');
    }
}

/// Emit a resolved include node's *content* (ignoring its own boundary marker)
/// as a standalone document, used to write the file the include points to.
fn emit_include_target(node: &YamlNode) -> Vec<u8> {
    let mut content = node.clone();
    content.source = None;
    emit_roundtrip(&content)
}

/// Walk a resolved include tree and produce `(target_file_id, file_bytes)` for
/// every `!include` boundary that maps to a single file. Nested includes within
/// each file are restored to their directive form.
///
/// A boundary whose own content is unmodified re-emits **verbatim** from its
/// cached original source (`file_sources`, indexed by `file_id`), so an
/// untouched included file is reproduced byte-for-byte, exactly as the root
/// document does. Only a boundary whose content actually changed is re-rendered
/// from the AST. This keeps writing back a split configuration from reflowing
/// files the edit never touched.
pub fn collect_include_changes(
    nodes: &[YamlNode],
    file_sources: &[Option<String>],
) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    for node in nodes {
        walk_boundaries(node, &mut |boundary, file_id| {
            let cached = file_sources.get(file_id as usize).and_then(Option::as_ref);
            let bytes = match cached {
                Some(src) if !local_modified(boundary) => src.clone().into_bytes(),
                _ => emit_include_target(boundary),
            };
            out.push((file_id, bytes));
        });
    }
    out
}

/// Like [`collect_include_changes`], but only emits include files whose own
/// content was modified (ignoring changes that belong to deeper includes).
/// Used by `YAMLRocksDocument.save` to write back just the changed files.
pub fn collect_changed_include_changes(nodes: &[YamlNode]) -> Vec<(u32, Vec<u8>)> {
    let mut out = Vec::new();
    for node in nodes {
        walk_boundaries(node, &mut |boundary, file_id| {
            if local_modified(boundary) {
                out.push((file_id, emit_include_target(boundary)));
            }
        });
    }
    out
}

/// Walk the AST in document order and invoke `visit` for every include boundary
/// (a node whose source targets another file), passing the node and its
/// `file_id`. Shared by the two write-back collectors above, which differ only
/// in what they do at each boundary.
fn walk_boundaries(node: &YamlNode, visit: &mut impl FnMut(&YamlNode, u32)) {
    // Grow the native stack on demand: this recurses once per nesting level over
    // attacker-controlled AST depth. See [`crate::stack`].
    crate::stack::guard(|| walk_boundaries_inner(node, visit))
}

fn walk_boundaries_inner(node: &YamlNode, visit: &mut impl FnMut(&YamlNode, u32)) {
    if let Some(ref source) = node.source {
        if let Some(file_id) = source.target_file_id {
            visit(node, file_id);
        }
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                walk_boundaries(key, visit);
                walk_boundaries(val, visit);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                walk_boundaries(item, visit);
            }
        }
        _ => {}
    }
}

/// Whether a node's *own* file content was modified, that is, a modification that
/// is not delegated to a nested include boundary (which owns its own file).
pub fn local_modified(node: &YamlNode) -> bool {
    if node.comments.modified {
        return true;
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => pairs
            .iter()
            .any(|(k, v)| local_modified(k) || (v.source.is_none() && local_modified(v))),
        YamlNodeKind::Sequence(items) => items
            .iter()
            .any(|it| it.source.is_none() && local_modified(it)),
        _ => false,
    }
}

/// Whether a node represents an empty scalar (renders as nothing after `key:`).
fn is_empty_scalar(node: &YamlNode) -> bool {
    match &node.kind {
        YamlNodeKind::Null => true,
        YamlNodeKind::Scalar(value, ScalarStyle::Plain) => value.is_empty(),
        _ => false,
    }
}

/// Whether a plain scalar's content would be misread inside a flow collection.
/// The flow indicators `,` `[` `]` `{` `}` end an entry or the collection, and a
/// `: ` (colon then space) or a trailing `:` starts a mapping value, so a plain
/// scalar containing any of them must be quoted when emitted in flow context.
fn plain_unsafe_in_flow(value: &str) -> bool {
    value
        .bytes()
        .any(|b| matches!(b, b',' | b'[' | b']' | b'{' | b'}'))
        || value.contains(": ")
        || value.ends_with(':')
}

/// Whether a mapping key cannot be written as an inline implicit key and must
/// use the explicit `?`/`:` block form. A non-empty block collection and a block
/// scalar (literal/folded) have no valid single-line representation: flattening
/// them inline would emit a block scalar inside a flow collection (illegal) or a
/// multiline flow collection as an implicit key (rejected). Everything else
/// (plain or quoted scalars, aliases, empty or single-line flow collections)
/// stays inline. Found by the `roundtrip` fuzz target.
fn key_needs_explicit(key: &YamlNode) -> bool {
    // The author wrote an explicit `?`: preserve it on re-emit (fidelity), as
    // long as the key can still open with a `? ` line. A block collection or a
    // block scalar is emitted explicitly anyway, below.
    if key.explicit_key && matches!(&key.kind, YamlNodeKind::Scalar(..) | YamlNodeKind::Alias(_)) {
        return true;
    }
    match &key.kind {
        YamlNodeKind::Mapping(m) => key.style == NodeStyle::Block && !m.is_empty(),
        YamlNodeKind::Sequence(s) => key.style == NodeStyle::Block && !s.is_empty(),
        YamlNodeKind::Scalar(_, ScalarStyle::Literal | ScalarStyle::Folded) => true,
        _ => false,
    }
}

/// Whether a synthetic (dump-path) key must switch to the explicit `? key` form
/// because it is not the first entry and carries a tag or anchor. An inline
/// `!tag key:` (or `&a key:`) after a previous entry has its property read as the
/// preceding value's node property, which is a reparse error when that value was
/// empty (`k:\n!tag key: v`) and a silent parity break otherwise. The explicit
/// `?` indicator opens a fresh key the preceding value cannot absorb, matching
/// the fast emitter. A loaded key is never synthetic, so its inline tag/anchor is
/// left untouched for byte-for-byte fidelity.
fn synthetic_key_needs_explicit(key: &YamlNode, is_first: bool) -> bool {
    !is_first && key.synthetic && (key.tag.is_some() || key.anchor.is_some())
}

#[cfg(test)]
mod tests {
    use super::{emit_roundtrip_with, NullStyle};
    use crate::roundtrip::ast::{YamlNode, YamlNodeKind};
    use crate::scanner::Span;

    fn scalar(text: &str) -> YamlNode {
        YamlNode::new(
            YamlNodeKind::Scalar(text.to_owned(), crate::scanner::ScalarStyle::Plain),
            Span::default(),
        )
    }

    fn synthetic_null() -> YamlNode {
        let mut n = YamlNode::new(YamlNodeKind::Null, Span::default());
        n.synthetic = true;
        n
    }

    /// Re-emit the round-trip AST of `input` and return the re-emitted bytes as
    /// text, asserting the result re-composes (is valid YAML).
    fn reemit(input: &str) -> String {
        let nodes = crate::roundtrip::composer::compose(input).unwrap();
        let out = super::emit_roundtrip_all_with(&nodes, NullStyle::Null);
        let text = String::from_utf8(out).unwrap();
        assert!(
            crate::roundtrip::composer::compose(&text).is_ok(),
            "re-emit of {input:?} does not re-parse:\n{text}"
        );
        text
    }

    /// Whether two YAML strings decode (fast path) to the same value tree, so a
    /// re-emit that changed only layout still preserves the data.
    fn same_value(a: &str, b: &str) -> bool {
        use crate::resolver::Schema;
        let da = crate::decode::decode_with(a, Schema::Yaml12, false, false);
        let db = crate::decode::decode_with(b, Schema::Yaml12, false, false);
        match (da, db) {
            (Ok(va), Ok(vb)) => va == vb,
            _ => false,
        }
    }

    /// A complex mapping key (a key that is itself a collection or a block
    /// scalar) must re-emit in the explicit `?`/`:` block form, not be flattened
    /// into an inline flow collection that is invalid YAML. Regression for a
    /// `roundtrip`-fuzz find (`?\r-\r>`, an explicit sequence-of-folded-scalar
    /// key) plus the data-preserving cases around it.
    #[test]
    fn complex_keys_use_explicit_form() {
        // The exact fuzz finding: the old emitter produced `[>-\n\n]:` (a block
        // scalar inside a flow collection), which the composer rejects. It must
        // now re-parse.
        reemit("?\r-\r>");

        // A block-sequence key and a literal-block-scalar key both have real
        // value-tree meaning; re-emit must preserve it, not just stay valid.
        let seq_key = "? - a\n  - b\n: v\n";
        assert!(same_value(seq_key, &reemit(seq_key)));
        let scalar_key = "? |\n  hi\n: v\n";
        assert!(same_value(scalar_key, &reemit(scalar_key)));

        // Keys that already have a valid inline form are left untouched: a plain
        // scalar key, and single-line flow collections.
        for unchanged in ["k: v\n", "[a, b]: v\n", "{a: 1}: v\n"] {
            assert!(same_value(unchanged, &reemit(unchanged)));
        }
    }

    /// A plain scalar whose content begins with U+FEFF must re-emit as a quoted
    /// scalar, not verbatim: emitted plain at the stream start, its leading byte
    /// order mark would be stripped on re-parse and the remainder (`*`) would
    /// reparse as a bare alias and fail. Regression for a `roundtrip`-fuzz find.
    #[test]
    fn leading_bom_plain_scalar_is_quoted_on_reemit() {
        let nodes = crate::roundtrip::composer::compose("\n\u{FEFF}*").unwrap();
        let out = super::emit_roundtrip_all_with(&nodes, NullStyle::Null);
        let text = std::str::from_utf8(&out).unwrap();
        // The re-emitted document must parse again (the whole point of the fix).
        let reparsed = crate::roundtrip::composer::compose(text).unwrap();
        // ...and still mean the same scalar, `\u{feff}*`.
        assert!(matches!(
            &reparsed[0].kind,
            YamlNodeKind::Scalar(s, _) if s == "\u{feff}*"
        ));
    }

    /// An author-written explicit `?` key survives a re-emit (after an edit),
    /// while an implicit key never sprouts one. Regression for the explicit-key
    /// fidelity fix.
    #[test]
    fn explicit_scalar_key_is_preserved_on_reemit() {
        for src in [
            "? explicit\n: value\n",
            "? a\n: 1\n? b\n: 2\n",
            "map:\n  ? k\n  : v\n",
        ] {
            let out = reemit(src);
            assert!(
                out.contains("? "),
                "explicit `?` dropped for {src:?}:\n{out}"
            );
            assert!(same_value(src, &out));
        }
        let implicit = reemit("a: 1\nb: 2\n");
        assert!(
            !implicit.contains('?'),
            "implicit key gained a `?`:\n{implicit}"
        );
    }

    /// A loaded (non-synthetic) null re-emits empty regardless of the style, while
    /// a synthetic null follows it: the guarantee that an untouched value is never
    /// restyled.
    #[test]
    fn loaded_null_ignores_style_synthetic_follows_it() {
        let doc = |val: YamlNode, style| {
            let root = YamlNode::new(
                YamlNodeKind::Mapping(vec![(scalar("k"), val)]),
                Span::default(),
            );
            String::from_utf8(emit_roundtrip_with(&root, style)).unwrap()
        };
        // Loaded empty null: always `k:` (preserved), even under the tilde style.
        let loaded = || YamlNode::new(YamlNodeKind::Null, Span::default());
        assert_eq!(doc(loaded(), NullStyle::Empty), "k:\n");
        assert_eq!(doc(loaded(), NullStyle::Tilde), "k:\n");
        assert_eq!(doc(loaded(), NullStyle::Null), "k:\n");
        // Synthetic null: follows the document style.
        assert_eq!(doc(synthetic_null(), NullStyle::Empty), "k:\n");
        assert_eq!(doc(synthetic_null(), NullStyle::Tilde), "k: ~\n");
        assert_eq!(doc(synthetic_null(), NullStyle::Null), "k: null\n");
    }
}
