//! Round-trip emitter: serializes a rich [`YamlNode`] AST back to YAML bytes,
//! preserving comments, scalar styles, anchors, tags, and block/flow layout.
//!
//! The emitter mirrors the structure of the fast-path encoder but additionally
//! threads comments through every node. Head comments are emitted on their own
//! lines above a node, inline comments trail a value on the same line, and foot
//! comments follow a node after a blank line.

use super::ast::{Comments, EmptyDocument, IncludeSource, NodeStyle, YamlNode, YamlNodeKind};
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
    run_emitter(null_style, DumpConfig::default(), |emitter| {
        if node.leading_bom {
            emitter.buf.extend_from_slice(BOM);
        }
        emitter.emit_document(node);
    })
}

/// Run `write` on a fresh emitter and return what it wrote.
///
/// A keep-chomped block scalar (`|+`) that ended the input without a line
/// break is replayed from its source, and the emitter drops the line break it
/// closes that line with again in [`finish`](RoundTripEmitter::finish). When
/// anything follows it in the output, though (a `:` for an explicit key, a key
/// added after it), that line break stays and keep chomping adds it to the
/// value. That is only known once the output is written, so the stream is
/// then written once more with that scalar rebuilt from its value.
fn run_emitter(
    null_style: NullStyle,
    dump: DumpConfig,
    write: impl Fn(&mut RoundTripEmitter),
) -> Vec<u8> {
    let mut emitter = RoundTripEmitter::new(null_style, dump);
    write(&mut emitter);
    if !emitter.unterminated_keep_followed() {
        return emitter.finish();
    }

    let mut emitter = RoundTripEmitter::new(null_style, dump);
    emitter.replay_unterminated_keep = false;
    write(&mut emitter);
    emitter.finish()
}

/// Emit multiple document ASTs, separated by `---` markers, rendering synthetic
/// nulls in `null_style`.
///
/// The empty documents a root carries (see [`EmptyDocuments`]) are replayed
/// around it, so the stream keeps every document its source had.
///
/// [`EmptyDocuments`]: super::ast::EmptyDocuments
pub fn emit_roundtrip_all_with(nodes: &[YamlNode], null_style: NullStyle) -> Vec<u8> {
    run_emitter(null_style, DumpConfig::default(), |emitter| {
        write_stream(emitter, nodes);
    })
}

/// Write `nodes` as a stream of documents, with the empty documents they carry.
fn write_stream(emitter: &mut RoundTripEmitter, nodes: &[YamlNode]) {
    if nodes.first().is_some_and(|n| n.leading_bom) {
        emitter.buf.extend_from_slice(BOM);
    }

    let mut stream_end = StreamEnd::Nothing;
    for node in nodes {
        let empty_documents = node.empty_documents.as_deref();
        for document in empty_documents.map_or(&[][..], |empty| &empty.before) {
            stream_end = emitter.emit_empty_document(document, stream_end);
        }

        if stream_end == StreamEnd::OpenDocument {
            // A directive (`%TAG`/`%YAML`) is only valid at the stream start or
            // right after a `...` end marker, so a document carrying directives
            // needs the previous one closed first. `emit_document` then emits the
            // directives and the `---`.
            if !node.directives.is_empty() {
                emitter.buf.extend_from_slice(b"...\n");
            } else if !node.explicit_start {
                // Documents need a `---` separator. A document that carries its
                // own explicit start marker emits it in `emit_document`, so only
                // add the separator for one that does not (avoiding a double
                // `---`).
                emitter.buf.extend_from_slice(b"---\n");
            }
        }
        emitter.emit_document(node);
        stream_end = StreamEnd::OpenDocument;

        for document in empty_documents.map_or(&[][..], |empty| &empty.after) {
            stream_end = emitter.emit_empty_document(document, stream_end);
        }
    }
}

/// How the stream emitted so far ends, which decides what separator the next
/// document needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamEnd {
    /// Nothing emitted yet: the next document opens the stream.
    Nothing,
    /// A document that is still open: the next one needs a `---`, or a `...`
    /// first when it starts with directives.
    OpenDocument,
    /// A document closed by an explicit `...`: the next one may start bare or
    /// with directives.
    ClosedDocument,
}

/// Emit a single document, with the empty documents it carries, in the default
/// null style (`null` keyword). Used by include write-back, where there is no
/// document-level style to thread.
pub fn emit_roundtrip(node: &YamlNode) -> Vec<u8> {
    emit_roundtrip_all_with(std::slice::from_ref(node), NullStyle::Null)
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
    /// Prefer single quotes when the emitter must quote a scalar on its own (a
    /// flow-unsafe or BOM-leading plain), mirroring `OPT_SINGLE_QUOTES` and the
    /// fast encoder's quote preference. The default (off) keeps the loaded-
    /// document behavior: double quotes, which can escape anything.
    pub single_quotes: bool,
    /// Soft output width; zero leaves lines unwrapped.
    pub width: usize,
}

/// Emit a single synthetic document tree with dump-shaping options applied. Used
/// by the `represent` emitter path, which builds edited-in nodes and wants a
/// PyYAML-style dump rather than round-trip fidelity.
pub fn emit_roundtrip_dump(node: &YamlNode, null_style: NullStyle, dump: DumpConfig) -> Vec<u8> {
    run_emitter(null_style, dump, |emitter| {
        if node.leading_bom {
            emitter.buf.extend_from_slice(BOM);
        }
        emitter.emit_document(node);
        if dump.explicit_end {
            emitter.buf.extend_from_slice(b"...\n");
        }
    })
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
    emitting_key: bool,
    /// The key being written right before its `:` (see
    /// [`emit_inline_key`](Self::emit_inline_key)), compared by address. The
    /// `:` that follows changes how its text reads: a lone `-` or `?` there is
    /// a plain scalar (`-: v`), where anywhere else it would open an entry or
    /// a key. Only that key itself, not the content of a flow collection key.
    key_before_colon: Option<*const YamlNode>,
    /// The comments of the node whose inline comment was written last. A block
    /// scalar's inline comment can be written in two places: on the `key:` line
    /// when the scalar starts below it, or on the scalar's own header line. Both
    /// paths check this so the comment is written once, by whichever comes
    /// first. Compared by address; nodes do not move while emitting.
    inline_comment_written: Option<*const Comments>,
    /// Where a replayed block scalar ended without a line break of its own. That
    /// only happens at the very end of the input, and the emitter still closes
    /// the line after it. If nothing else follows in the output either, that
    /// line break is dropped again in [`finish`](Self::finish): under keep
    /// chomping (`|+`) it would otherwise become part of the scalar's value.
    unterminated_raw_end: Option<usize>,
    /// Whether the block scalar at [`unterminated_raw_end`](Self::unterminated_raw_end)
    /// is keep-chomped (`|+`), so a line break written after it would change
    /// its value.
    unterminated_raw_keeps: bool,
    /// Whether a keep-chomped block scalar that ended the input without a line
    /// break may be replayed from its source. Cleared for the second pass of
    /// [`run_emitter`], which rebuilds it from its value instead.
    replay_unterminated_keep: bool,
    /// The lowest column a continuation line of a multi-line plain or quoted
    /// scalar may start at in the block position being written: one past the
    /// column of the mapping key or sequence dash that holds it, zero at the
    /// document root. A replayed source line left of it would end the enclosing
    /// collection instead of continuing the scalar. Flow collections inherit it
    /// from the block position they sit in. A block scalar's content must
    /// start at it too, and its indentation indicator counts from the column
    /// just left of it (see [`parent_indent`](Self::parent_indent)).
    continuation_floor: usize,
}

impl RoundTripEmitter {
    fn new(null_style: NullStyle, dump: DumpConfig) -> Self {
        Self {
            buf: Vec::with_capacity(256),
            dump,
            null_style,
            flow_depth: 0,
            emitting_key: false,
            key_before_colon: None,
            inline_comment_written: None,
            unterminated_raw_end: None,
            unterminated_raw_keeps: false,
            replay_unterminated_keep: true,
            continuation_floor: 0,
        }
    }

    /// Whether a keep-chomped block scalar replayed without its final line
    /// break has output after it, which the line break closing its line then
    /// joins (see [`run_emitter`]).
    fn unterminated_keep_followed(&self) -> bool {
        self.unterminated_raw_keeps
            && self
                .unterminated_raw_end
                .is_some_and(|end| self.buf.len() > end + 1)
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

    /// The token a null node is written with, or `None` for a null written as
    /// its properties alone (see [`is_property_only_null`]). A synthetic null
    /// styled `~` renders as the tilde indicator; every other null (loaded, or
    /// styled `null`/empty in a position where empty is unsafe) renders as the
    /// `null` keyword.
    fn null_token(&self, node: &YamlNode) -> Option<&'static [u8]> {
        if is_property_only_null(node) {
            return None;
        }

        if matches!(self.synthetic_null(node), Some(NullStyle::Tilde)) {
            Some(b"~")
        } else {
            Some(b"null")
        }
    }

    /// Whether a mapping value or sequence item is written as an empty entry
    /// (`key:`, `-`), followed only by its properties and its comment.
    ///
    /// That is a null written as its properties alone, and any other empty
    /// node that reads back the same without a token: a loaded null (written
    /// empty in its source), a synthetic null in the `empty` style, or an empty
    /// plain scalar. Those last three are left to the general value arms when
    /// they carry a comment of their own.
    fn is_empty_entry(&self, node: &YamlNode) -> bool {
        if is_property_only_null(node) {
            return true;
        }

        is_empty_scalar(node)
            && node.comments.inline.is_none()
            && self
                .synthetic_null(node)
                .is_none_or(|style| style == NullStyle::Empty)
    }

    // -- Document entry --

    /// The finished output. A block scalar replayed from source without a final
    /// line break (it ended the input) stays that way when nothing follows it in
    /// the output: the line break [`end_line`](Self::end_line) added after it is
    /// not part of the source, and under keep chomping it would change the value.
    fn finish(mut self) -> Vec<u8> {
        if let Some(end) = self.unterminated_raw_end {
            if self.buf.len() == end + 1 && self.buf[end] == b'\n' {
                self.buf.truncate(end);
            }
        }
        self.buf
    }

    /// Replay an empty document from its source text, after `stream_end`, and
    /// return how the stream ends once it is written. The text carries its own
    /// `---` and comments; only a closing `...` for an open document before it
    /// can be missing, since the emitter does not keep a document's end marker.
    fn emit_empty_document(
        &mut self,
        document: &EmptyDocument,
        stream_end: StreamEnd,
    ) -> StreamEnd {
        if stream_end == StreamEnd::OpenDocument && document.has_directives {
            self.buf.extend_from_slice(b"...\n");
        }
        self.buf.extend_from_slice(document.text.as_bytes());

        if document.explicit_end {
            StreamEnd::ClosedDocument
        } else {
            StreamEnd::OpenDocument
        }
    }

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
        // A synthetic (dump-path) root block collection that carries a tag emits
        // the tag on its own line, then its body indented one step under it, so
        // the tag binds to the collection on reload, matching the fast encoder's
        // `!tag` + `emit_value_after_colon(inner, 0)`. A loaded collection keeps
        // its source column (body at 0 here), and an untagged root sits flush.
        let tagged_root = node.synthetic && node.tag.is_some();
        match &node.kind {
            YamlNodeKind::Mapping(pairs) if node.style == NodeStyle::Block && !pairs.is_empty() => {
                self.emit_anchor_tag_line(node, 0);
                let body = if tagged_root { self.step() } else { 0 };
                self.emit_block_mapping(pairs, body);
            }
            YamlNodeKind::Sequence(items)
                if node.style == NodeStyle::Block && !items.is_empty() =>
            {
                self.emit_anchor_tag_line(node, 0);
                // Indentless keeps the dashes flush with the tag even for a tagged
                // root sequence, as the fast encoder does.
                let body = if tagged_root && !self.dump.indentless {
                    self.step()
                } else {
                    0
                };
                self.emit_block_sequence(items, body);
            }
            // A tagged empty root keeps just its properties on its own line
            // (`!t`): the `null` token (`!t null`) would reload as the string
            // "null" rather than an empty value. That holds for a loaded null
            // written that way, and for a synthetic tagged null in the empty
            // style, matching the fast path's Tagged + empty-null handling. A
            // bare (untagged) null still renders as the `null` keyword via the
            // inline arm below.
            YamlNodeKind::Null
                if is_property_only_null(node)
                    || (node.synthetic
                        && node.tag.is_some()
                        && node.comments.inline.is_none()
                        && node.anchor.is_none()
                        && matches!(self.synthetic_null(node), Some(NullStyle::Empty))) =>
            {
                self.emit_properties(node);
                self.emit_inline_comment(&node.comments);
                self.buf.push(b'\n');
            }
            _ => {
                self.emit_anchor_tag(node);
                self.continuation_floor = 0;
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
            if key_needs_explicit(key, i == 0) {
                self.emit_explicit_key_pair(key, val, indent);
                continue;
            }
            self.emit_inline_key(key, indent);
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
            if key_needs_explicit(key, i == 0) {
                self.emit_explicit_key_pair(key, val, indent);
                continue;
            }
            self.emit_inline_key(key, indent);
            self.emit_value_after_colon(val, indent);
        }
    }

    /// Write a mapping key that shares its line with the `:` after it, with
    /// its properties and that `:` (`!t &a key:`): an implicit key in a block
    /// or flow mapping, or a flow key after its `? `. A key that was empty in
    /// its source is written empty again (`: value`, `!t : value`).
    ///
    /// An alias key needs a space before the `:`: anchor and alias names may
    /// contain `:` (the scanner is spec-strict here, per the YAML test suite's
    /// W5VH), so a glued `*id001:` would scan as the alias "id001:" and the
    /// document could not reload.
    fn emit_inline_key(&mut self, key: &YamlNode, indent: usize) {
        self.emit_anchor_tag(key);
        if !is_loaded_empty_key(key) {
            let outer = self.key_before_colon.replace(key as *const YamlNode);
            self.emit_key(key, indent);
            self.key_before_colon = outer;
        }

        if matches!(key.kind, YamlNodeKind::Alias(_)) {
            self.buf.push(b' ');
        }
        self.buf.push(b':');
    }

    /// Emit one mapping pair in explicit `?`/`:` block form, for a complex key
    /// that has no valid inline rendering (see [`key_needs_explicit`]).
    fn emit_explicit_key_pair(&mut self, key: &YamlNode, val: &YamlNode, indent: usize) {
        // The cursor already sits at the key column (the caller wrote the
        // indentation or a `- ` dash). Emit `? <key>` then `: <value>`, with both
        // indicators at `indent` and the key/value blocks one configured step
        // deeper (so `OPT_INDENT_4` applies to a represented block-collection key).
        let child = indent + self.step();
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
            // A key empty in its source stays empty: a bare `?`, or its
            // properties alone (`? !t`), with no space left at the line end.
            _ if is_loaded_empty_key(key) => {
                self.emit_anchor_tag_compact(key);
                self.buf.push(b'\n');
            }
            // A block scalar (literal/folded) key, plus any other inline content
            // (a flow collection, a scalar the author wrote as `? key`). A block
            // scalar opens on the `? ` line and indents its body under the key.
            _ => {
                self.buf.push(b' ');
                self.emit_anchor_tag(key);
                self.continuation_floor = indent + 1;
                self.emit_key(key, indent + self.step());
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
                // key's own column, matching the fast encoder. A tagged sequence
                // always indents under its tag even in indentless mode (as the
                // fast path does): the tag sits on the line above, so flush dashes
                // would not bind to it and the tag would be lost on reload.
                let seq_indent = if self.dump.indent_sequences {
                    if self.dump.indentless && val.tag.is_none() {
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
            // A `key:` with no value (see [`is_empty_entry`](Self::is_empty_entry)).
            // A synthetic null styled `null`/`~` falls through to the inline arm
            // below. Any tag and/or anchor is kept as bare properties
            // (`key: !x`, `key: !x &id001`, `key: &id001`), never collapsed to
            // `key:` (which drops them) nor expanded to `key: !x null` (which
            // reloads as the string "null" rather than an empty value); this
            // matches the fast path and keeps a shared tagged empty value
            // lossless under aliasing.
            _ if self.is_empty_entry(val) => self.emit_empty_entry(val, child),
            // A value the author wrote below its key, with a comment left on the
            // key line (`key: # note`). Keep both where they were: pushing the
            // comment past the value would move it onto a line it was never on.
            _ if val.comments.inline_before_value => {
                self.continuation_floor = indent + 1;
                self.emit_value_below_introducer(val, child);
            }
            _ => {
                // Restore the padding between the `:` and an inline value
                // (`example:      true`); one space for a synthetic value.
                for _ in 0..val.comments.value_pad.max(1) {
                    self.buf.push(b' ');
                }
                self.emit_anchor_tag(val);
                self.continuation_floor = indent + 1;
                // A block-scalar value's body sits one step past the key, matching
                // the fast encoder (`emit_literal_block(s, indent + step())`).
                self.emit_inline_content(val, indent + self.step());
                self.emit_inline_comment(&val.comments);
                // Block scalars already end with their own newline(s).
                self.end_line();
            }
        }
    }

    /// Emit a value that sits on the line(s) below its introducer, after the
    /// comment the author left on the `key:`/`-` line itself. The cursor sits
    /// just past that introducer; the comment closes the line, any standalone
    /// comments between the two follow, then the value at `indent`.
    fn emit_value_below_introducer(&mut self, val: &YamlNode, indent: usize) {
        // An anchor or tag stays on the introducer line, ahead of the comment,
        // where the author wrote it (`key: &a # note`). It binds to the value
        // either way, but moving it onto the value's line is a reflow this path
        // exists to avoid.
        self.emit_anchor_tag_compact(val);
        self.emit_inline_comment(&val.comments);
        self.buf.push(b'\n');
        // Only the comments written below the introducer: a sequence item's own
        // caller has already emitted the ones above its `-`, and a mapping value
        // has none there (they belong to the key, which was walked first).
        for comment in val.comments.head_below_introducer() {
            self.emit_comment_line(comment, indent);
        }
        self.write_indent(indent);
        self.emit_inline_content(val, indent);
        self.end_line();
    }

    /// Finish an empty entry (`key: !t # note`, `- &a`) on its introducer's
    /// line. The cursor sits just past the `:` or `-`. Head comments the author
    /// wrote below that line follow it at `indent`, where the value would have
    /// started.
    fn emit_empty_entry(&mut self, node: &YamlNode, indent: usize) {
        self.emit_anchor_tag_compact(node);
        self.emit_inline_comment(&node.comments);
        self.buf.push(b'\n');
        for comment in node.comments.head_below_introducer() {
            self.emit_comment_line(comment, indent);
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
            self.emit_head_above_dash(item, indent);
            self.write_indent(indent);
            self.emit_sequence_item_body(item, indent);
        }
    }

    /// Emit the standalone comments that precede a sequence item's `-`. An item
    /// whose dash line ends in a comment (`- # note`) holds its head comments
    /// *below* that dash instead, where the author wrote them, so they are left
    /// to [`emit_head_below_dash`](Self::emit_head_below_dash).
    fn emit_head_above_dash(&mut self, item: &YamlNode, indent: usize) {
        if item.comments.inline_before_value {
            for comment in item.comments.head_above_introducer() {
                self.emit_comment_line(comment, indent);
            }
        } else {
            self.emit_head(&item.comments, indent);
        }
    }

    /// The counterpart: emit the head comments an item carries *below* its dash,
    /// on the lines between a dash-line comment and the item's own content.
    fn emit_head_below_dash(&mut self, item: &YamlNode, indent: usize) {
        if item.comments.inline_before_value {
            for comment in item.comments.head_below_introducer() {
                self.emit_comment_line(comment, indent);
            }
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
                self.emit_head_above_dash(item, indent);
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
        // node, so it falls through and keeps its spelling. Any tag and/or anchor
        // is kept as bare properties (`- !x`, `- !x &id001`, `- &id001`), never
        // expanded to `- !x null` (which reloads as the string "null" rather than
        // an empty value); this matches the fast path and keeps a shared tagged
        // empty item lossless under aliasing.
        if self.is_empty_entry(item) {
            self.emit_empty_entry(item, child);
            return;
        }
        match &item.kind {
            YamlNodeKind::Mapping(m) if item.style == NodeStyle::Block && !m.is_empty() => {
                if item.anchor.is_some() || item.tag.is_some() {
                    // An anchored/tagged block mapping carries its marker on the
                    // dash line, then breaks to the indented keys. Emitting it
                    // inline (`- &a key: value`) would bind the marker to the
                    // first key rather than the mapping. The body indents by the
                    // configured step (like the fast path, which routes a tagged
                    // item through `emit_value_after_colon`), not the fixed +2 of
                    // the compact inline form below.
                    self.emit_anchor_tag_compact(item);
                    self.emit_inline_comment(&item.comments);
                    self.buf.push(b'\n');
                    self.emit_head_below_dash(item, indent + self.step());
                    self.emit_block_mapping(m, indent + self.step());
                } else if item.comments.inline.is_some() || item.comments.inline_before_value {
                    // A comment closing the dash line (`- # note`) keeps the
                    // first key off it, so the mapping opens on the next line at
                    // the item indent instead of sharing the dash. The placement
                    // flag holds this open after the comment itself is cleared,
                    // so the head comments below the dash keep their home.
                    self.emit_inline_comment(&item.comments);
                    self.buf.push(b'\n');
                    self.emit_head_below_dash(item, child);
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
                    // compact `- -` form cannot hold a leading `&anchor`/tag). A
                    // nested sequence's items sit two columns past the dash (the
                    // fast path's fixed offset), unlike a mapping item above.
                    self.emit_anchor_tag_compact(item);
                    self.emit_inline_comment(&item.comments);
                    self.buf.push(b'\n');
                    self.emit_head_below_dash(item, child);
                    self.emit_block_sequence(s, child);
                } else if item.comments.compact {
                    // A compact nested sequence (`- - 1`) keeps its first item on
                    // this dash line; otherwise it breaks to the next, indented.
                    self.buf.push(b' ');
                    self.emit_block_sequence_after_dash(s, child);
                } else {
                    // As for a mapping item, a comment closing the dash line
                    // stays there and the nested items open below it.
                    self.emit_inline_comment(&item.comments);
                    self.buf.push(b'\n');
                    self.emit_head_below_dash(item, child);
                    self.emit_block_sequence(s, child);
                }
                // A trailing comment block at the end of this item's sequence.
                self.emit_foot(&item.comments, child);
            }
            // An item written below its `-`, with a comment left on the dash
            // line; both keep their place (see the mapping-value arm).
            _ if item.comments.inline_before_value => {
                self.continuation_floor = indent + 1;
                self.emit_value_below_introducer(item, child);
            }
            _ => {
                // Restore the padding between the `-` and an inline item
                // (`-    item`); one space for a synthetic item.
                for _ in 0..item.comments.value_pad.max(1) {
                    self.buf.push(b' ');
                }
                self.emit_anchor_tag(item);
                self.continuation_floor = indent + 1;
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
            // A null written as its properties alone leaves nothing here: the
            // space after its tag or anchor already separates it from the `,`,
            // `:` or `]` that follows (`[!t , a]`, `!t : a`).
            YamlNodeKind::Null => {
                if let Some(token) = self.null_token(node) {
                    self.buf.extend_from_slice(token);
                }
            }
            YamlNodeKind::Scalar(value, style) => {
                // An unmodified block scalar replays its verbatim source, which
                // keeps the author's layout: where a folded block's lines broke
                // cannot be rebuilt from its value. So does a plain or quoted
                // scalar the author spread over several lines. Either only as
                // long as those lines still hold it where it is written.
                if let Some(raw) = node.comments.raw.as_deref() {
                    if !node.comments.modified && self.raw_replay_fits(raw, value, *style) {
                        self.buf.extend_from_slice(raw.as_bytes());
                        // Only a block scalar's source ends in its own line
                        // break; a flow scalar's always stops at its last
                        // character, and the line break after it is the
                        // emitter's to write.
                        let block = matches!(style, ScalarStyle::Literal | ScalarStyle::Folded);
                        if block && !raw.ends_with(['\n', '\r']) {
                            self.unterminated_raw_end = Some(self.buf.len());
                            self.unterminated_raw_keeps = block_header_keeps(raw);
                        }
                        return;
                    }
                }
                match style {
                    ScalarStyle::Literal | ScalarStyle::Folded if self.flow_depth == 0 => {
                        self.emit_block_scalar(node, value, *style, indent);
                    }
                    _ => {
                        let before_colon = self.key_before_colon == Some(node as *const YamlNode);
                        self.emit_scalar(value, *style, before_colon);
                    }
                }
            }
            YamlNodeKind::Alias(name) => {
                self.buf.push(b'*');
                self.buf.extend_from_slice(name.as_bytes());
            }
            YamlNodeKind::Sequence(items) => self.emit_flow_sequence(items, indent),
            YamlNodeKind::Mapping(pairs) => self.emit_flow_mapping(pairs, indent),
        }
    }

    /// Whether a scalar's verbatim source can be replayed at the current output
    /// position. A multi-line plain or quoted scalar keeps its continuation
    /// lines at their source columns, so it fits only where each of them still
    /// continues the scalar, and never as a key: an implicit key must stay on
    /// one line, and an explicit one is rebuilt from its value rather than
    /// threading the key's position through here. The checks the plain arms of
    /// [`emit_scalar`](Self::emit_scalar) make before writing a value verbatim
    /// apply to its source just the same: a leading BOM, flow-unsafe text
    /// inside a flow collection, and text that does not read back as plain
    /// ([`plain_reads_back`]; never before a key's `:` here, since keys are not
    /// replayed). Two of its arms have no counterpart: a line break in the
    /// value is what the replayed source lines hold, checked by
    /// [`continuation_lines_fit`] instead, and the dump width only shapes
    /// synthetic nodes, which have no source to replay. A quoted scalar's
    /// quotes protect its content, so only its continuation lines are checked.
    ///
    /// A block scalar keeps its body lines at their source columns too, and
    /// what they mean depends on the indentation of the node holding it: the
    /// content must sit deeper than that node, and an indentation indicator in
    /// the header (`|2`) counts from it. Without an indicator, the source
    /// reads the same as long as its first line with content still starts at
    /// or past the floor. Otherwise the source is read back as if it sat
    /// here, one column left of the floor, and replayed only when that gives
    /// the same value. A keep-chomped one that ended the input without a line
    /// break is rebuilt instead when something follows it (see [`run_emitter`]).
    ///
    /// A plain scalar's last line ending in a lone `?`, `-`, or `:` is not
    /// replayed either. In the source a flow indicator followed it (`?}`), but
    /// the emitter may write a line break there instead, and a lone indicator
    /// at the end of a line opens a key, an entry, or a value. Tabs separate
    /// it from the line start just as spaces do. This is about the source's
    /// line layout, not the value: `x\n  ?` folds to "x ?", which reads back
    /// fine on one line, so [`plain_reads_back`] does not cover it.
    fn raw_replay_fits(&self, raw: &str, value: &str, style: ScalarStyle) -> bool {
        match style {
            ScalarStyle::Literal | ScalarStyle::Folded if self.flow_depth > 0 => false,
            ScalarStyle::Literal | ScalarStyle::Folded
                if !self.replay_unterminated_keep
                    && !raw.ends_with(['\n', '\r'])
                    && block_header_keeps(raw) =>
            {
                false
            }
            ScalarStyle::Literal | ScalarStyle::Folded => match detected_block_indent(raw) {
                Some(column) => column >= self.continuation_floor,
                None => crate::scanner::reread_block_scalar(raw, self.parent_indent())
                    .is_some_and(|reread| reread == value),
            },
            _ if self.emitting_key => false,
            ScalarStyle::Plain if value.starts_with('\u{feff}') => false,
            ScalarStyle::Plain if self.flow_depth > 0 && plain_unsafe_in_flow(value) => false,
            ScalarStyle::Plain if !plain_reads_back(value, false) => false,
            ScalarStyle::Plain
                if raw.rsplit(['\n', '\r']).next().is_some_and(|last| {
                    matches!(last.trim_start_matches([' ', '\t']), "?" | "-" | ":")
                }) =>
            {
                false
            }
            _ => continuation_lines_fit(raw, self.continuation_floor),
        }
    }

    fn emit_key(&mut self, key: &YamlNode, indent: usize) {
        let was_key = self.emitting_key;
        self.emitting_key = true;
        self.emit_inline_content(key, indent);
        self.emitting_key = was_key;
    }

    /// Write a scalar from its value in its own style. `before_colon` says the
    /// scalar is a key written right before its `:` (see
    /// [`plain_reads_back`]).
    fn emit_scalar(&mut self, value: &str, style: ScalarStyle, before_colon: bool) {
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
                self.push_quoted(value);
            }
            // Inside a flow collection a bare `,`/`[`/`]`/`{`/`}`/`: ` would end
            // the entry or collection early, so a plain scalar carrying one must
            // be quoted. This only bites an edited-in value: a loaded plain scalar
            // never contains a bare flow indicator (the scanner would not produce
            // it). See [`plain_unsafe_in_flow`].
            ScalarStyle::Plain if self.flow_depth > 0 && plain_unsafe_in_flow(value) => {
                self.push_quoted(value);
            }
            // A line break in a plain value comes from a blank line in its
            // source (one blank line folds to one `\n`). Writing it verbatim
            // would put the rest of the value at column 0, where it reads as a
            // new key, a document marker, or invalid YAML. Its source lines were
            // not replayed (the value was edited, or they no longer fit where it
            // is written), so rebuild it on one line, with the break escaped.
            ScalarStyle::Plain if value.contains(['\n', '\r']) => {
                self.push_quoted(value);
            }
            // A plain scalar the scanner read only thanks to what followed it in
            // the source (`-` and `?` before a `:` that the parser took as the
            // next entry's, a value folded from lines ending in a lone `:`) is
            // quoted once that neighbour is gone. See [`plain_reads_back`].
            ScalarStyle::Plain if !plain_reads_back(value, before_colon) => {
                self.push_quoted(value);
            }
            ScalarStyle::Plain
                if self.dump.width > 0
                    && !self.emitting_key
                    && crate::emit_util::current_column(&self.buf) + value.len()
                        > self.dump.width
                    && crate::encode::has_breakable_space(value) =>
            {
                self.push_quoted(value);
            }
            ScalarStyle::Plain => self.buf.extend_from_slice(value.as_bytes()),
            // Single quotes cannot escape a line break either, so a value with
            // one takes double quotes for the same reason as a plain one above.
            ScalarStyle::SingleQuoted => {
                self.push_styled_quotes(value, !value.contains(['\n', '\r']));
            }
            ScalarStyle::DoubleQuoted => self.push_styled_quotes(value, false),
            // Block context writes these through `emit_block_scalar`, so they
            // only get here inside a flow collection, where a block scalar is
            // invalid. The producers downgrade them before that (the represent
            // lowering, the assignment style rules); if a future producer
            // misses it, emit valid YAML rather than an unparsable `[|`.
            ScalarStyle::Literal | ScalarStyle::Folded => {
                crate::emit_util::push_double_quoted(&mut self.buf, value);
            }
        }
    }

    /// Quote a scalar the emitter must quote on its own (a flow-unsafe or
    /// BOM-leading plain): double quotes by default, single when the dump
    /// prefers them and the value allows it, mirroring the fast encoder's
    /// `emit_quoted_string`.
    fn push_quoted(&mut self, value: &str) {
        self.push_styled_quotes(
            value,
            self.dump.single_quotes && crate::emit_util::single_quotable(value, false),
        );
    }

    fn push_styled_quotes(&mut self, value: &str, single: bool) {
        if self.dump.width > 0 && !self.emitting_key {
            let indent = crate::emit_util::current_column(&self.buf).max(1);
            let (quote, body) = if single {
                (b'\'', crate::emit_util::single_quoted_body(value))
            } else {
                (b'"', crate::emit_util::double_quoted_body(value))
            };
            self.buf.push(quote);
            crate::emit_util::push_folded(&mut self.buf, &body, indent, self.dump.width);
            self.buf.push(quote);
        } else if single {
            crate::emit_util::push_single_quoted(&mut self.buf, value);
        } else {
            crate::emit_util::push_double_quoted(&mut self.buf, value);
        }
    }

    fn emit_flow_separator(&mut self, indent: usize) {
        self.buf.push(b',');
        if self.dump.width > 0
            && !self.emitting_key
            && crate::emit_util::current_column(&self.buf) >= self.dump.width
        {
            self.buf.push(b'\n');
            self.write_indent(indent.max(1));
        } else {
            self.buf.push(b' ');
        }
    }

    /// Write a block scalar from its value, in its own style, with the body
    /// lines at the absolute column `body_indent`. The caller computes that
    /// column for its context (a mapping value: `key + step`; a sequence item:
    /// `dash + 2`; the document root: `step`), matching the fast encoder. The
    /// writer itself is shared with the fast encoder
    /// ([`crate::emit_util::push_block_scalar`]), so the header and folding
    /// rules cannot drift between the two.
    ///
    /// Both block styles hold the same strings, so a folded scalar stays
    /// folded and a literal one literal. A value neither can hold (a carriage
    /// return, a control character) is double-quoted instead, as it would be
    /// inside a flow collection.
    ///
    /// A trailing comment belongs on the header line (`key: | # note`). After
    /// the body it would sit on a line of its own, and for an empty body that
    /// line would be read back as the scalar's content.
    fn emit_block_scalar(
        &mut self,
        node: &YamlNode,
        value: &str,
        style: ScalarStyle,
        body_indent: usize,
    ) {
        if !crate::emit_util::block_scalar_capable(value) {
            self.push_styled_quotes(value, false);
            return;
        }

        let marker = if style == ScalarStyle::Literal {
            b'|'
        } else {
            b'>'
        };
        // At the document root there is no parent node; an indentation
        // indicator there counts from column 0.
        let parent_indent = self.parent_indent().max(0) as usize;

        crate::emit_util::push_block_scalar_header(
            &mut self.buf,
            value,
            marker,
            body_indent - parent_indent,
        );
        self.emit_inline_comment(&node.comments);
        crate::emit_util::push_block_scalar_body(&mut self.buf, value, marker, body_indent);
    }

    /// The indentation of the block mapping or sequence that holds the node
    /// being written, the one its continuation floor sits a column past: -1 at
    /// the document root.
    fn parent_indent(&self) -> i32 {
        self.continuation_floor as i32 - 1
    }

    fn emit_flow_sequence(&mut self, items: &[YamlNode], indent: usize) {
        if items.iter().any(node_has_comments) {
            self.emit_commented_flow_sequence(items, indent);
            return;
        }
        self.buf.push(b'[');
        self.flow_depth += 1;
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.emit_flow_separator(indent);
            }
            self.emit_anchor_tag(item);
            self.emit_inline_content(item, indent);
        }
        self.flow_depth -= 1;
        self.buf.push(b']');
    }

    fn emit_flow_mapping(&mut self, pairs: &[(YamlNode, YamlNode)], indent: usize) {
        if pairs
            .iter()
            .any(|(key, value)| node_has_comments(key) || node_has_comments(value))
        {
            self.emit_commented_flow_mapping(pairs, indent);
            return;
        }
        self.buf.push(b'{');
        self.flow_depth += 1;
        for (i, (key, val)) in pairs.iter().enumerate() {
            if i > 0 {
                self.emit_flow_separator(indent);
            }
            self.emit_flow_key(key, indent);
            self.buf.push(b' ');
            self.emit_anchor_tag(val);
            self.emit_inline_content(val, indent);
        }
        self.flow_depth -= 1;
        self.buf.push(b'}');
    }

    /// Write a flow mapping key and its `:`. A key the author wrote with `?`
    /// keeps it (`{? a: b}`); that also holds an empty key in place (`{? : b}`).
    fn emit_flow_key(&mut self, key: &YamlNode, indent: usize) {
        if key.explicit_key {
            self.buf.extend_from_slice(b"? ");
        }
        self.emit_inline_key(key, indent);
    }

    /// A flow sequence whose items carry comments, one item per line so each
    /// comment has a line to sit on (`[` / `  # above` / `  item, # note` / `]`).
    /// Only head and inline comments reach here: a foot comment has no place
    /// inside a flow collection and is rejected before emission.
    fn emit_commented_flow_sequence(&mut self, items: &[YamlNode], indent: usize) {
        self.buf.extend_from_slice(b"[\n");
        self.flow_depth += 1;
        let child_indent = indent + self.step();
        for (index, item) in items.iter().enumerate() {
            self.emit_head(&item.comments, child_indent);
            self.write_indent(child_indent);
            self.emit_anchor_tag(item);
            self.emit_inline_content(item, child_indent);
            if index + 1 < items.len() {
                self.buf.push(b',');
            }
            self.emit_inline_comment(&item.comments);
            self.buf.push(b'\n');
        }
        self.flow_depth -= 1;
        self.write_indent(indent);
        self.buf.push(b']');
    }

    /// A flow mapping whose entries carry comments, one entry per line. A key
    /// never carries an inline or foot comment (it must stay on one line), so
    /// only head comments above the entry and the value's inline comment render.
    fn emit_commented_flow_mapping(&mut self, pairs: &[(YamlNode, YamlNode)], indent: usize) {
        self.buf.extend_from_slice(b"{\n");
        self.flow_depth += 1;
        let child_indent = indent + self.step();
        for (index, (key, value)) in pairs.iter().enumerate() {
            self.emit_head(&key.comments, child_indent);
            self.emit_head(&value.comments, child_indent);
            self.write_indent(child_indent);
            self.emit_flow_key(key, child_indent);
            self.buf.push(b' ');
            self.emit_anchor_tag(value);
            self.emit_inline_content(value, child_indent);
            if index + 1 < pairs.len() {
                self.buf.push(b',');
            }
            self.emit_inline_comment(&value.comments);
            self.buf.push(b'\n');
        }
        self.flow_depth -= 1;
        self.write_indent(indent);
        self.buf.push(b'}');
    }

    // -- Comments --

    fn emit_head(&mut self, comments: &Comments, indent: usize) {
        for comment in &comments.head {
            self.emit_comment_line(&comment.text, indent);
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
        let Some(ref comment) = comments.inline else {
            return;
        };
        // Already written for this node, on its `key:` line or its header line.
        let this = comments as *const Comments;
        if self.inline_comment_written == Some(this) {
            return;
        }

        self.push_inline_comment(comment, comments.inline_spaces);
        self.inline_comment_written = Some(this);
    }

    /// Write ` # comment`, restoring the original padding before the `#`
    /// (alignment) or a single space for a synthetic comment (`spaces` 0).
    fn push_inline_comment(&mut self, comment: &str, spaces: u32) {
        for _ in 0..spaces.max(1) {
            self.buf.push(b' ');
        }
        self.buf.push(b'#');
        if !comment.is_empty() {
            self.buf.push(b' ');
            self.buf.extend_from_slice(comment.as_bytes());
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
        self.emit_properties(node);
        self.buf.push(b'\n');
    }

    /// Emit a node's tag and anchor (`!t &a`) with no space before or after.
    fn emit_properties(&mut self, node: &YamlNode) {
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
/// The empty documents around it are that file's own, kept on its include
/// source; the ones on the node belong to the stream that includes it.
fn emit_include_target(node: &YamlNode) -> Vec<u8> {
    let mut content = node.clone();
    content.empty_documents = content
        .source
        .take()
        .and_then(|source| source.empty_documents);
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

/// Whether `node` is a loaded null that carries a tag or an anchor: one the
/// author wrote as its properties alone (`key: !t`, `[!t , a]`, `!!str : v`).
/// It keeps that form in every position. Adding a token would change the
/// value: `!t null` is the string "null" under the tag `!t`, and `!!str null`
/// is the string "null" where `!!str` alone is the empty string. A synthetic
/// null is left to the document's null style instead, as the fast encoder does.
fn is_property_only_null(node: &YamlNode) -> bool {
    matches!(node.kind, YamlNodeKind::Null)
        && !node.synthetic
        && (node.tag.is_some() || node.anchor.is_some())
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

/// Whether a plain scalar's text, written as is, reads back as that same plain
/// scalar in any position the emitter writes it. It does not when it is a lone
/// `-`, `?` or `:`, or starts with one followed by a space (an entry, a key, a
/// value), or holds `: ` (a value indicator) or ` #` (a comment). A trailing `:`
/// makes a value read as a mapping key. Right before a key's own `:`
/// (`before_colon`), a lone indicator and a trailing `:` are plain text: the
/// `:` that follows keeps them from standing alone (`-: v`, `a:: v`).
///
/// The scanner never reads such a plain scalar on its own; one reaches the
/// emitter when the source put it next to a character that kept it plain.
fn plain_reads_back(value: &str, before_colon: bool) -> bool {
    let holds_indicator = value.contains(": ")
        || value.contains(" #")
        || value.contains("\t#")
        || ["- ", "? ", ": "]
            .iter()
            .any(|indicator| value.starts_with(indicator));
    if holds_indicator {
        return false;
    }

    let stands_alone = matches!(value, "-" | "?" | ":") || value.ends_with(':');
    before_colon || !stands_alone
}

/// Whether a block mapping key is written in the explicit `?`/`:` form rather
/// than as an implicit `key:`. This is the one place that decides it; `is_first`
/// says whether the key opens its mapping.
///
/// - The author wrote the `?`: it stays. Every key can open a `? ` line, an
///   empty one included (a bare `?`).
/// - The key cannot be written on one line, which an implicit key must be: a
///   non-empty block collection, a block scalar (literal/folded), or a flow
///   collection whose comments spread it over several lines. Found by the
///   `roundtrip` fuzz target.
/// - A synthetic (dump-path) key with a tag or anchor that does not open its
///   mapping: the fast emitter writes it as `? !tag key`, and the `represent`
///   output must match it byte for byte. A loaded key keeps its inline
///   property (`!tag key:`), which its source already reads back as.
///
/// Everything else (plain or quoted scalars, aliases, empty keys, single-line
/// flow collections) stays implicit.
fn key_needs_explicit(key: &YamlNode, is_first: bool) -> bool {
    if key.explicit_key {
        return true;
    }

    let spans_lines = match &key.kind {
        YamlNodeKind::Mapping(m) if key.style == NodeStyle::Block => !m.is_empty(),
        YamlNodeKind::Sequence(s) if key.style == NodeStyle::Block => !s.is_empty(),
        YamlNodeKind::Mapping(_) | YamlNodeKind::Sequence(_) => flow_holds_comments(key),
        YamlNodeKind::Scalar(_, ScalarStyle::Literal | ScalarStyle::Folded) => true,
        _ => false,
    };
    if spans_lines {
        return true;
    }

    !is_first && key.synthetic && (key.tag.is_some() || key.anchor.is_some())
}

/// Whether a flow collection, at any depth, holds a comment. The emitter gives
/// each commented entry a line of its own, so such a collection is written over
/// several lines.
fn flow_holds_comments(node: &YamlNode) -> bool {
    // Grow the native stack on demand: this recurses once per nesting level of
    // a key, whose depth the input controls. See [`crate::stack`].
    crate::stack::guard(|| match &node.kind {
        YamlNodeKind::Mapping(pairs) => pairs.iter().any(|(key, value)| {
            node_has_comments(key)
                || node_has_comments(value)
                || flow_holds_comments(key)
                || flow_holds_comments(value)
        }),
        YamlNodeKind::Sequence(items) => items
            .iter()
            .any(|item| node_has_comments(item) || flow_holds_comments(item)),
        _ => false,
    })
}

/// Whether a mapping key was written empty in its source (`: value`, `? `, a
/// key that is only its properties, `!t : value`), so it is written empty
/// again. A synthetic null key follows the document's null style instead.
fn is_loaded_empty_key(key: &YamlNode) -> bool {
    matches!(key.kind, YamlNodeKind::Null) && !key.synthetic
}

/// Whether the continuation lines of a multi-line plain or quoted scalar's
/// source (`raw`, every line after its first) still continue it when replayed
/// with `floor` as the lowest column they may start at. Blank and space-only
/// lines are empty lines of the scalar, which need no indentation. A line at
/// column 0 must also not start like a `---`/`...` document marker; the source
/// could not have held one there, but the check costs nothing and keeps the
/// replay from ever writing one.
fn continuation_lines_fit(raw: &str, floor: usize) -> bool {
    raw.split(['\n', '\r']).skip(1).all(|line| {
        let indent = line.bytes().take_while(|&byte| byte == b' ').count();
        if indent == line.len() {
            return true;
        }

        indent >= floor && !(indent == 0 && (line.starts_with("---") || line.starts_with("...")))
    })
}

/// The column a reader detects a block scalar's indentation at, from its
/// source (`raw`, from the `|`/`>` header through its last line): the leading
/// spaces of its first line with content. Every other line then reads the same
/// wherever the scalar sits, as long as that column is still past its parent.
///
/// `None` when the header carries an indentation indicator, which fixes the
/// indentation relative to the parent instead, or when no line has content,
/// where the line that follows the scalar takes part in the detection.
///
/// This runs for every replayed block scalar, so it reads only the header and
/// the leading spaces of the lines up to the first with content.
fn detected_block_indent(raw: &str) -> Option<usize> {
    let bytes = raw.as_bytes();
    let header_end = bytes
        .iter()
        .position(|&byte| matches!(byte, b'\n' | b'\r'))?;

    // The indicators follow the `|`/`>` directly; a space, tab or `#` ends them.
    let has_indentation_indicator = bytes[..header_end]
        .iter()
        .skip(1)
        .take_while(|&&byte| !matches!(byte, b' ' | b'\t' | b'#'))
        .any(u8::is_ascii_digit);
    if has_indentation_indicator {
        return None;
    }

    let mut rest = &bytes[header_end..];
    loop {
        let line_start = rest
            .iter()
            .position(|&byte| !matches!(byte, b'\n' | b'\r'))?;
        rest = &rest[line_start..];

        let spaces = rest.iter().position(|&byte| byte != b' ')?;
        if !matches!(rest[spaces], b'\n' | b'\r') {
            return Some(spaces);
        }
        rest = &rest[spaces..];
    }
}

/// Whether a block scalar's source (`raw`, from its `|`/`>` header on) has a
/// keep chomping indicator (`|+`, `>2+`), so its trailing line breaks are part
/// of its value.
fn block_header_keeps(raw: &str) -> bool {
    raw.bytes()
        .skip(1)
        .take_while(|&byte| !matches!(byte, b' ' | b'\t' | b'#' | b'\n' | b'\r'))
        .any(|byte| byte == b'+')
}

fn node_has_comments(node: &YamlNode) -> bool {
    !node.comments.head.is_empty()
        || node.comments.inline.is_some()
        || !node.comments.foot.is_empty()
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

    /// Every empty document survives a re-emit from the tree, byte for byte,
    /// wherever it sits in the stream.
    #[test]
    fn empty_documents_survive_a_reemit() {
        for input in [
            "---\n---\na: 1\n",
            "a: 1\n---\n---\nb: 2\n",
            "a: 1\n---\n",
            "a: 1\n--- # note\n",
            "---\n# only a comment\n---\na: 1\n",
            "a: 1\n---\n# one\n---\n# two\n---\nb: 2\n",
            "a: 1\n---\n# trailing document\n",
            "---\n...\na: 1\n",
            "---\n...\n---\na: 1\n",
            "%YAML 1.2\n---\n---\na: 1\n",
            "a: 1\n...\n%YAML 1.2\n---\n# Empty\n...\n",
            "\u{FEFF}# head\n---\n---\na: 1\n",
        ] {
            assert_eq!(reemit(input), input);
        }
    }

    /// The comment of an empty document after a block scalar stays in its own
    /// document. Folded into the preceding root, it would replay as a line of
    /// the scalar's content.
    #[test]
    fn empty_document_comment_does_not_join_a_block_scalar() {
        let input = "%YAML 1.2\n--- |\n%!PS-Adobe-2.0\n...\n%YAML 1.2\n---\n# Empty\n...\n";
        let out = reemit(input);
        assert!(same_value(&out, input), "data changed: {out:?}");
        assert!(
            out.ends_with("...\n%YAML 1.2\n---\n# Empty\n...\n"),
            "{out:?}"
        );
    }

    /// A document with directives after an empty one that is still open gets
    /// the `...` it needs; one after a closed empty document does not get a
    /// second.
    #[test]
    fn directives_after_an_empty_document_close_it_once() {
        let mut nodes =
            crate::roundtrip::composer::compose("---\n...\n%TAG !e! tag:x:\n---\n!e!t a\n")
                .unwrap();
        assert_eq!(
            super::emit_roundtrip_all_with(&nodes, NullStyle::Null),
            b"---\n...\n%TAG !e! tag:x:\n---\n!e!t a\n"
        );

        let empty = nodes[0].empty_documents.as_deref_mut().unwrap();
        empty.before[0].text = "---\n".to_owned();
        empty.before[0].explicit_end = false;
        assert_eq!(
            super::emit_roundtrip_all_with(&nodes, NullStyle::Null),
            b"---\n...\n%TAG !e! tag:x:\n---\n!e!t a\n"
        );
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

    /// An unmodified plain or quoted scalar spread over several lines replays
    /// its source lines, in every block and flow position it can take, and the
    /// line break after it (it never ends in one itself) is still written.
    #[test]
    fn multiline_flow_scalars_replay_their_source() {
        for src in [
            "b: text\n\n  ...\nc: 3\n",
            "b: text\n\n  more\n",
            "b: x\n  y\n",
            "r:\n  - x\n\n    y\n  - z\n",
            "- 'a\n\n  b'\n",
            "f: [x\n\n  y, z]\n",
            "f: {k: x\n\n  y}\n",
            "b: 'x\n\n  y'\n",
            "b: \"x\n\n  y\"\n",
            "b: \"x\\\n  y\"\n",
            "b: # note\n  x\n\n  y\n",
            "text\n\nmore\n",
        ] {
            assert_eq!(reemit(src), src);
        }
    }

    /// Where a scalar's source lines would no longer sit past its key, or the
    /// scalar is a key, the source is not replayed. A value holding a line
    /// break is then double-quoted, which keeps it on one line; one without
    /// stays in its own style, folded onto one line.
    #[test]
    fn multiline_flow_scalars_rebuild_where_their_source_does_not_fit() {
        // The emitter indents a nested mapping two columns, past the source's
        // continuation lines at column 2.
        assert_eq!(reemit("a:\n b: x\n\n  y\n"), "a:\n  b: \"x\\ny\"\n");
        assert_eq!(reemit("a:\n b: 'x\n\n  y'\n"), "a:\n  b: \"x\\ny\"\n");
        assert_eq!(reemit("a:\n b: 'x\n  y'\n"), "a:\n  b: 'x y'\n");
        assert_eq!(reemit("a:\n b: \"x\n\n  y\"\n"), "a:\n  b: \"x\\ny\"\n");

        // Keys are rebuilt from their value: an implicit key must stay on one
        // line, so the replay is never tried there.
        assert_eq!(reemit("? x\n\n  y\n: v\n"), "? \"x\\ny\"\n: v\n");
        assert_eq!(reemit("{x\n\n  y: v}\n"), "{\"x\\ny\": v}\n");

        // A last line holding a lone `?` was closed by a `,` in the source; the
        // commented layout below puts a line break after it instead, which
        // would turn it into an explicit key.
        let lone_indicator = "{a: x\n  ?, # c\n b: 1}\n";
        let out = reemit(lone_indicator);
        assert!(!out.contains("\n  ?\n"), "lone `?` replayed:\n{out}");
        assert!(same_value(lone_indicator, &out));

        // The same with a tab in front of the indicator, which is white space
        // before it just as a space is.
        let after_a_tab = "{k: 1, # c\n b: x\n \t?}\n";
        assert!(same_value(after_a_tab, &reemit(after_a_tab)));
    }

    /// An edited plain or single-quoted value holding a line break is never
    /// written with that break raw (which would land the rest of the value at
    /// column 0); it takes double quotes. Without a break the style is kept.
    #[test]
    fn edited_scalar_with_a_line_break_is_double_quoted() {
        use crate::scanner::ScalarStyle;

        let doc = |text: &str, style| {
            let mut value = YamlNode::new(
                YamlNodeKind::Scalar(text.to_owned(), style),
                Span::default(),
            );
            value.mark_modified();
            let root = YamlNode::new(
                YamlNodeKind::Mapping(vec![(scalar("k"), value)]),
                Span::default(),
            );
            String::from_utf8(emit_roundtrip_with(&root, NullStyle::Null)).unwrap()
        };

        assert_eq!(doc("a\n...", ScalarStyle::Plain), "k: \"a\\n...\"\n");
        assert_eq!(doc("a\nb", ScalarStyle::SingleQuoted), "k: \"a\\nb\"\n");
        assert_eq!(doc("a\tb", ScalarStyle::SingleQuoted), "k: 'a\tb'\n");
        assert_eq!(doc("a b", ScalarStyle::Plain), "k: a b\n");
    }

    /// An unmodified block scalar replays its source wherever that source still
    /// reads back as its value: in every block position, with an indentation
    /// indicator whose parent did not move, and with a header comment.
    #[test]
    fn block_scalars_replay_their_source() {
        for src in [
            "a: |\n  x\n",
            "a: >\n  folded\n  text\n\n  more\n",
            "a:\n  b: |2\n     x\n    y\n",
            "- |1\n  x\n- >-\n  y\n",
            "- - |\n    x\n",
            "- k: |2\n     x\n",
            "? |\n  key\n: v\n",
            "a: | # note\n  x\n",
            "a: # note\n  |\n  x\n",
            "|\n  root\n",
            ">2\n   root\n",
            "a: |+\n  x\n\n",
        ] {
            assert_eq!(reemit(src), src);
        }

        // A key the emitter moves keeps its scalar's source as long as that
        // still reads back the same there.
        assert_eq!(reemit("a:\n b: |\n   x\n"), "a:\n  b: |\n   x\n");
    }

    /// Where a block scalar's source would read back as something else, it is
    /// rewritten from its value instead: its body would no longer sit past its
    /// key, or its indentation indicator would count from a parent that moved.
    #[test]
    fn block_scalars_rebuild_where_their_source_does_not_fit() {
        // The emitter indents the nested mapping two columns, onto the source's
        // body lines at column 2.
        assert_eq!(reemit("a:\n b: |\n  x\n"), "a:\n  b: |\n    x\n");
        assert_eq!(reemit("a:\n b: >\n  x\n  y\n"), "a:\n  b: >\n    x y\n");

        // `|1` under a key at column 1 means column 2; under the key moved to
        // column 2 it would mean column 3.
        assert_eq!(reemit("a:\n b: |1\n   x\n"), "a:\n  b: |2\n     x\n");
    }

    /// A modified block scalar is rewritten in its own style with a header and
    /// body that read back as its value: a folded one keeps its line breaks, a
    /// leading space keeps its indentation indicator, and a comment goes on the
    /// header line.
    #[test]
    fn modified_block_scalars_keep_their_value() {
        let modified = |src: &str, comment: Option<&str>| {
            let mut nodes = crate::roundtrip::composer::compose(src).unwrap();
            let YamlNodeKind::Mapping(pairs) = &mut nodes[0].kind else {
                panic!("not a mapping: {src:?}");
            };
            let value = &mut pairs.last_mut().unwrap().1;
            value.comments.inline = comment.map(str::to_owned);
            value.mark_modified();
            let text =
                String::from_utf8(super::emit_roundtrip_all_with(&nodes, NullStyle::Null)).unwrap();
            assert!(same_value(src, &text), "data changed: {src:?} -> {text:?}");
            text
        };

        assert_eq!(
            modified("a: 1\nb: >\n  ab\n\n  cd\n", Some("t")),
            "a: 1\nb: > # t\n  ab\n\n  cd\n"
        );
        assert_eq!(
            modified("a: 1\nb: |2\n   x\n  y\n", Some("t")),
            "a: 1\nb: |2 # t\n   x\n  y\n"
        );
        assert_eq!(
            modified("b: >\n  a\n    more\n  b\n\n\n  c\n", None),
            "b: >\n  a\n    more\n  b\n\n\n  c\n"
        );
        assert_eq!(modified("b: |+\n\n", None), "b: |+\n\n");
        assert_eq!(modified("b: >-\n\n  \n", None), "b: >-\n\n");
    }

    /// A block-styled value no block scalar can hold (a carriage return)
    /// falls back to double quotes, with its comment after it.
    #[test]
    fn block_scalar_with_a_carriage_return_is_double_quoted() {
        use crate::scanner::ScalarStyle;

        for style in [ScalarStyle::Literal, ScalarStyle::Folded] {
            let mut value = YamlNode::new(
                YamlNodeKind::Scalar("a\rb\n".to_owned(), style),
                Span::default(),
            );
            value.comments.inline = Some("note".to_owned());
            value.mark_modified();
            let root = YamlNode::new(
                YamlNodeKind::Mapping(vec![(scalar("k"), value)]),
                Span::default(),
            );
            assert_eq!(
                String::from_utf8(emit_roundtrip_with(&root, NullStyle::Null)).unwrap(),
                "k: \"a\\rb\\n\" # note\n"
            );
        }
    }

    /// The detected indentation is the leading spaces of the first line with
    /// content; an indentation indicator or a scalar without content leaves it
    /// to a full read.
    #[test]
    fn detected_block_indent_reads_the_first_content_line() {
        use super::detected_block_indent;

        assert_eq!(detected_block_indent("|\n  x\n"), Some(2));
        assert_eq!(
            detected_block_indent(">- # note\n\n   \n    x\n  y\n"),
            Some(4)
        );
        assert_eq!(detected_block_indent("|+\r\n\r\n  \tx\r\n"), Some(2));
        assert_eq!(detected_block_indent("|\nx\n"), Some(0));
        assert_eq!(detected_block_indent("|2\n   x\n"), None);
        assert_eq!(detected_block_indent("|-1 # c\n  x\n"), None);
        assert_eq!(detected_block_indent("|+\n\n  \n"), None);
    }

    /// The fit rule itself: every continuation line that holds content must
    /// start at or past the floor, blank lines are free, and a column-0 line
    /// never starts like a document marker.
    #[test]
    fn continuation_lines_fit_the_floor() {
        use super::continuation_lines_fit;

        assert!(continuation_lines_fit("x\n\n  y", 1));
        assert!(continuation_lines_fit("x\n\n  y", 2));
        assert!(!continuation_lines_fit("x\n\n  y", 3));
        assert!(continuation_lines_fit("'x\n   \n  y'", 2));
        assert!(continuation_lines_fit("x\r\n\r\n  y", 2));
        assert!(!continuation_lines_fit("x\n\t y", 1));
        assert!(continuation_lines_fit("x\ny", 0));
        assert!(!continuation_lines_fit("x\n...", 0));
        assert!(!continuation_lines_fit("'x\n---'", 0));
    }

    /// A node written as its tag or anchor alone, with no content, re-emits
    /// exactly as written in every position and under every null style. A
    /// `null` token after the tag would read back as the string "null".
    #[test]
    fn tagged_empty_nodes_reemit_without_a_null_token() {
        for input in [
            // Mapping values, with and without an anchor or a comment.
            "k: !t\n",
            "k: !t # note\n",
            "k: !t &a # note\n",
            "k: &a # note\n",
            "k: !!str # note\n",
            "k: !t # note\n  # below\nz: 1\n",
            // Sequence items.
            "- !t\n- 2\n",
            "- !t # note\n- 2\n",
            "- !t &a # note\n- 2\n",
            "- !t # note\n  # below\n- 2\n",
            // Mapping keys.
            "!t : a\n",
            "!!null : a\n",
            "!t &a : a\n",
            // Flow collection entries.
            "f: [!t , a]\n",
            "f: [a, !!str ]\n",
            "f: {b: !t , !!str : c}\n",
            // The document root.
            "!t\n",
            "!t &a\n",
            "!\n",
            // The YAML test suite's FH7J, with its second mapping moved up onto
            // the dash line as the emitter writes it.
            "- !!str\n- !!null : a\n  b: !!str\n- !!str : !!null\n",
        ] {
            for style in [NullStyle::Empty, NullStyle::Tilde, NullStyle::Null] {
                let nodes = crate::roundtrip::composer::compose(input).unwrap();
                let out = super::emit_roundtrip_all_with(&nodes, style);
                assert_eq!(std::str::from_utf8(&out).unwrap(), input, "{style:?}");
            }
        }
    }

    /// A tagged empty root keeps its comment on the same line as the tag.
    #[test]
    fn tagged_empty_root_keeps_its_comment() {
        let mut root = crate::roundtrip::composer::compose("!t\n")
            .unwrap()
            .remove(0);
        root.comments.inline = Some("note".to_owned());
        let out = String::from_utf8(emit_roundtrip_with(&root, NullStyle::Null)).unwrap();
        assert_eq!(out, "!t # note\n");
    }

    /// A synthetic tagged null (the `represent` path) still follows the null
    /// style, so its output stays byte-identical to the fast encoder's.
    #[test]
    fn synthetic_tagged_null_follows_the_null_style() {
        let doc = |style| {
            let mut value = synthetic_null();
            value.tag = Some("!t".to_owned());
            let root = YamlNode::new(
                YamlNodeKind::Mapping(vec![(scalar("k"), value)]),
                Span::default(),
            );
            String::from_utf8(emit_roundtrip_with(&root, style)).unwrap()
        };

        assert_eq!(doc(NullStyle::Empty), "k: !t\n");
        assert_eq!(doc(NullStyle::Tilde), "k: !t ~\n");
        assert_eq!(doc(NullStyle::Null), "k: !t null\n");
    }

    /// A key empty in its source comes back empty, in its own form, in every
    /// position and under every null style.
    #[test]
    fn empty_keys_reemit_as_written() {
        for input in [
            ": v\n",
            ":\n",
            "a: 1\n: v\n",
            "a:\n: v\n",
            "- : v\n",
            "?\n: v\n",
            "? !t\n: v\n",
            "? &a\n: v\n",
            "f: {: v}\n",
            "f: {a: 1, : v}\n",
            "f: {? : v}\n",
            "f: [{: v}]\n",
        ] {
            for style in [NullStyle::Empty, NullStyle::Tilde, NullStyle::Null] {
                let nodes = crate::roundtrip::composer::compose(input).unwrap();
                let out = super::emit_roundtrip_all_with(&nodes, style);
                assert_eq!(std::str::from_utf8(&out).unwrap(), input, "{style:?}");
            }
        }
    }

    /// A synthetic null key (assigned through the API) is a `null` or `~`
    /// token, as the fast encoder writes it, never an empty key.
    #[test]
    fn synthetic_null_key_follows_the_null_style() {
        let doc = |style| {
            let root = YamlNode::new(
                YamlNodeKind::Mapping(vec![(synthetic_null(), scalar("v"))]),
                Span::default(),
            );
            String::from_utf8(emit_roundtrip_with(&root, style)).unwrap()
        };

        assert_eq!(doc(NullStyle::Empty), "null: v\n");
        assert_eq!(doc(NullStyle::Tilde), "~: v\n");
        assert_eq!(doc(NullStyle::Null), "null: v\n");
    }

    /// Where a key's source form would not read back, the re-emit switches to
    /// one that does and keeps the data.
    #[test]
    fn keys_whose_source_form_does_not_reload_are_rewritten() {
        for (input, expected) in [
            // An explicit key before an anchored key: the `:` the source left
            // out keeps the anchor off the explicit key.
            ("?\n&a : 1\n", "?\n:\n&a : 1\n"),
            ("a: 1\n? b\n&x c: 3\n", "a: 1\n? b\n:\n&x c: 3\n"),
            // A comment inside a flow key spreads it over several lines.
            ("[Y]:\t#\n", "? [\n    Y   #\n  ]\n:\n"),
            ("{a}: #\n", "? {\n    a: null     #\n  }\n:\n"),
            // A lone indicator that only stayed plain thanks to the `:` after it.
            ("{i: -:}\n", "{i: \"-\", : null}\n"),
            ("{i: ?:}\n", "{i: \"?\", : null}\n"),
            (": &a\n*a ?:\n", ": &a\n*a : \"?\"\n:\n"),
            ("{}::\n", "{}: \":\"\n:\n"),
            // A plain value folded from lines that end in a lone `:`.
            ("-  e\n ::\n", "-  \"e :\"\n- :\n"),
            // The same with the colon kept in the value: its source lines fit,
            // but replayed bare they would read back as "e a".
            ("-   e\n a::\n", "-   \"e a:\"\n- :\n"),
            // A keep-chomped block scalar that ended the input, now followed by
            // the explicit key's `:`.
            ("? |+\n x", "? |-\n  x\n:\n"),
        ] {
            let out = reemit(input);
            assert_eq!(out, expected, "{input:?}");
            assert!(
                same_value(input, &out),
                "data changed for {input:?}: {out:?}"
            );
        }
    }

    /// The one decision on the explicit key form: kept where the author wrote
    /// it, needed where the key cannot sit on one line, and for a synthetic
    /// key with properties after the first entry.
    #[test]
    fn key_needs_explicit_decides_the_key_form() {
        // Whether the first key of the mapping `input` needs the explicit form.
        let first_key_explicit = |input: &str, is_first: bool| {
            let root = crate::roundtrip::composer::compose(input)
                .unwrap()
                .remove(0);
            let YamlNodeKind::Mapping(mut pairs) = root.kind else {
                panic!("not a mapping: {input:?}");
            };
            super::key_needs_explicit(&pairs.remove(0).0, is_first)
        };

        for input in [
            "? a\n: 1\n",
            "?\n: 1\n",
            "? [a]\n: 1\n",
            "? - a\n: 1\n",
            "? |\n x\n: 1\n",
            "[a]: # c\n",
            "[[a]]: # c\n",
        ] {
            assert!(first_key_explicit(input, true), "{input:?}");
        }

        for input in ["a: 1\n", ": 1\n", "!t : 1\n", "[a]: 1\n"] {
            assert!(!first_key_explicit(input, true), "{input:?}");
            assert!(!first_key_explicit(input, false), "{input:?}");
        }

        let mut synthetic = scalar("k");
        synthetic.synthetic = true;
        synthetic.tag = Some("!t".to_owned());
        assert!(super::key_needs_explicit(&synthetic, false));
        assert!(!super::key_needs_explicit(&synthetic, true));
    }

    /// Which plain texts read back as themselves, and which only do right
    /// before a key's `:`.
    #[test]
    fn plain_reads_back_where_the_scanner_reads_it_plain() {
        for value in ["a", "a-b", "a:b", "http://x", "a#b", "-a", "?a", ":a"] {
            assert!(super::plain_reads_back(value, false), "{value:?}");
            assert!(super::plain_reads_back(value, true), "{value:?}");
        }

        for value in ["-", "?", ":", "a:", "e :"] {
            assert!(!super::plain_reads_back(value, false), "{value:?}");
            assert!(super::plain_reads_back(value, true), "{value:?}");
        }

        for value in ["a: b", "a #b", "a\t#b", "- a", "? a", ": a"] {
            assert!(!super::plain_reads_back(value, false), "{value:?}");
            assert!(!super::plain_reads_back(value, true), "{value:?}");
        }
    }

    /// A keep chomping indicator is found wherever it sits in the header.
    #[test]
    fn block_header_keeps_reads_the_chomping_indicator() {
        assert!(super::block_header_keeps("|+\n x"));
        assert!(super::block_header_keeps(">2+\n  x"));
        assert!(super::block_header_keeps("|+2 # c\n  x"));
        assert!(!super::block_header_keeps("|\n x"));
        assert!(!super::block_header_keeps("|- # a+b\n x"));
        assert!(!super::block_header_keeps("|2\n  +"));
    }

    /// A keep-chomped block scalar that ended the input keeps its value when
    /// output follows it, and its source form when nothing does.
    #[test]
    fn unterminated_keep_block_scalar_is_rebuilt_only_when_followed() {
        assert_eq!(reemit("k: |+\n  x"), "k: |+\n  x");
        assert_eq!(reemit("- |+\n x"), "- |+\n x");

        let mut nodes = crate::roundtrip::composer::compose("k: |+\n  x").unwrap();
        let YamlNodeKind::Mapping(pairs) = &mut nodes[0].kind else {
            panic!("not a mapping");
        };
        pairs.push((scalar("b"), scalar("1")));
        let out = super::emit_roundtrip_all_with(&nodes, NullStyle::Null);
        let out = std::str::from_utf8(&out).unwrap();

        assert_eq!(out, "k: |-\n  x\nb: 1\n");
        assert!(same_value(out, "k: x\nb: 1\n"));
    }
}
