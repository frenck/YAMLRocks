//! YAML emitter for the fast path: converts a [`Value`] tree into YAML bytes.
//!
//! The emitter produces conventional block-style YAML by default, matching the
//! layout most tools and humans expect:
//!
//! ```yaml
//! mapping:
//!   key: value
//!   list:
//!     - 1
//!     - 2
//! ```
//!
//! Flow style (`{}`/`[]`) is used for empty collections always, and for every
//! collection when [`EmitOptions::flow_style`] is set.

pub mod json;

use crate::decode::Value;
use crate::resolver::{ScalarKind, Schema};
use crate::scanner::ScalarStyle;

/// How a null value is rendered. All three forms parse back to null under the
/// YAML 1.2 core schema; the choice is purely stylistic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NullStyle {
    /// The explicit `null` keyword (the default; unambiguous in every position).
    #[default]
    Null,
    /// An empty node (`key:` with nothing after the colon). Only used where it is
    /// unambiguous (a block mapping value or block sequence entry); elsewhere
    /// (top level, flow collections, a mapping key) it falls back to `null`.
    Empty,
    /// The `~` indicator (unambiguous in every position, like `null`).
    Tilde,
}

impl NullStyle {
    /// The inline token for a null in a position where an *empty* node would be
    /// ambiguous (top level, flow, a key). `Empty` falls back to `null` here.
    fn inline_token(self) -> &'static str {
        match self {
            NullStyle::Tilde => "~",
            NullStyle::Null | NullStyle::Empty => "null",
        }
    }
}

/// Options controlling YAML emission.
#[derive(Debug, Clone)]
pub struct EmitOptions {
    /// Number of spaces per indentation level for nested mappings.
    pub indent: usize,
    /// Sort mapping keys alphabetically before emitting.
    pub sort_keys: bool,
    /// Emit all collections in flow style (`{}`/`[]`).
    pub flow_style: bool,
    /// Emit an explicit `---` document-start marker.
    pub explicit_start: bool,
    /// Emit an explicit `...` document-end marker.
    pub explicit_end: bool,
    /// How null values are rendered (keyword, empty, or `~`).
    pub null_style: NullStyle,
    /// When a scalar must be quoted, use double quotes (`"..."`). When `false`,
    /// prefer single quotes (`'...'`), falling back to double only where single
    /// cannot represent the value (it contains a newline).
    pub double_quotes: bool,
    /// Emit a block sequence that is a mapping value at its key's column (its
    /// dashes align with the key) instead of indenting it one level deeper. The
    /// "indentless" style favored by `kubectl` and much of the Kubernetes
    /// ecosystem; off by default (the dominant config style indents).
    pub indentless_sequences: bool,
    /// Best-effort maximum line width. `0` disables wrapping (the default): long
    /// scalars and flow collections emit on one line. When set, scalars are
    /// folded and flow collections broken at safe points to keep lines at or
    /// below the width, but only where a break cannot change the decoded value
    /// (so a run of spaces, or a line with no break opportunity, may still
    /// exceed it). A soft limit, like PyYAML's `width`.
    pub width: usize,
    /// The YAML version the output targets. Governs quoting: strict YAML 1.1
    /// (`Schema::Yaml11`) additionally quotes the scalars only that schema reads
    /// as non-strings (bare `y`/`n` booleans, sexagesimal `1:30`), so the output
    /// re-reads identically under 1.1. The default (`Yaml12`) and the
    /// PyYAML-compat 1.1 variant keep the conservative cross-schema quoting.
    pub schema: Schema,
}

impl Default for EmitOptions {
    fn default() -> Self {
        Self {
            indent: 2,
            sort_keys: false,
            flow_style: false,
            explicit_start: false,
            explicit_end: false,
            null_style: NullStyle::Empty,
            double_quotes: true,
            indentless_sequences: false,
            width: 0,
            schema: Schema::Yaml12,
        }
    }
}

/// Encode a single [`Value`] into YAML bytes.
pub fn encode(value: &Value<'_>, options: &EmitOptions) -> Vec<u8> {
    let mut emitter = Emitter::new(options);
    if options.explicit_start {
        emitter.buf.extend_from_slice(b"---\n");
    }
    emitter.emit_root(value);
    if options.explicit_end {
        emitter.buf.extend_from_slice(b"...\n");
    }
    emitter.buf
}

struct Emitter<'a> {
    buf: Vec<u8>,
    options: &'a EmitOptions,
    /// True while emitting a mapping key. A key must stay on one line to remain a
    /// valid implicit key, so all width-driven line breaking (scalar folding and
    /// flow-separator breaks) is suppressed for the whole key subtree.
    emitting_key: bool,
}

impl<'a> Emitter<'a> {
    fn new(options: &'a EmitOptions) -> Self {
        Self {
            // Most documents emit to more than a couple hundred bytes; starting
            // larger skips the first few doubling reallocations (each of which
            // copies the whole buffer) for the common case, at a negligible cost
            // for tiny ones.
            buf: Vec::with_capacity(1024),
            options,
            emitting_key: false,
        }
    }

    /// The number of spaces each nesting level adds for mappings.
    #[inline]
    fn step(&self) -> usize {
        self.options.indent.max(1)
    }

    /// Whether `value` should be emitted as a multi-line block collection
    /// rather than inline.
    fn is_block(&self, value: &Value) -> bool {
        if self.options.flow_style {
            return false;
        }
        match value {
            Value::Sequence(items) => !items.is_empty(),
            Value::Mapping(pairs) => !pairs.is_empty(),
            _ => false,
        }
    }

    // -- Top level --

    fn emit_root(&mut self, value: &Value) {
        match value {
            Value::Mapping(pairs) if self.is_block(value) => {
                self.emit_block_mapping(pairs, 0);
            }
            Value::Sequence(items) if self.is_block(value) => {
                self.emit_block_sequence(items, 0);
            }
            Value::String(s) if Self::use_literal_block(s) => {
                self.emit_literal_block(s, self.step());
            }
            // A tagged root value: `!tag` then its inner value, which may itself
            // be a block collection or block scalar.
            Value::Tagged(tag, inner) => {
                self.buf.extend_from_slice(tag.as_bytes());
                self.emit_value_after_colon(inner, 0);
            }
            _ => {
                self.emit_inline(value, false);
                self.buf.push(b'\n');
            }
        }
    }

    // -- Block mapping --

    fn emit_block_mapping(&mut self, pairs: &[(Value, Value)], indent: usize) {
        let ordered = self.order_pairs(pairs);
        for (i, (key, val)) in ordered.iter().enumerate() {
            self.write_indent(indent);
            self.emit_block_pair(key, val, indent, i > 0);
        }
    }

    /// Emit a mapping whose first pair sits on the line already opened by a
    /// sequence dash (`- key: value`). `indent` is the column of the keys.
    fn emit_block_mapping_after_dash(&mut self, pairs: &[(Value, Value)], indent: usize) {
        let ordered = self.order_pairs(pairs);
        for (i, (key, val)) in ordered.iter().enumerate() {
            if i > 0 {
                self.write_indent(indent);
            }
            self.emit_block_pair(key, val, indent, i > 0);
        }
    }

    /// Emit one block-mapping pair (the caller has written the key's indent). A
    /// *non-first* tagged key is written in the explicit `? key` / `: value`
    /// form: an inline tagged key (`!t k:`) after a previous entry would have its
    /// `!t` read as that entry's value's node property, since a tag at the
    /// mapping's indent binds to the preceding value (a reparse error). The `?`
    /// indicator starts a fresh key that no preceding value can absorb. A first
    /// pair has no preceding sibling, so it stays in the compact inline form.
    fn emit_block_pair(&mut self, key: &Value, val: &Value, indent: usize, not_first: bool) {
        if not_first && matches!(key, Value::Tagged(..)) {
            self.buf.extend_from_slice(b"? ");
            self.emit_key(key);
            self.buf.push(b'\n');
            self.write_indent(indent);
            self.buf.push(b':');
        } else {
            self.emit_key(key);
            self.buf.push(b':');
        }
        self.emit_value_after_colon(val, indent);
    }

    /// Emit the portion after a mapping key's colon, choosing inline or block.
    fn emit_value_after_colon(&mut self, val: &Value, indent: usize) {
        // Grow the native stack on demand: emitting a deeply nested value
        // recurses once per level, and the tree (already bounded by the build
        // depth cap) could otherwise overflow a small thread stack. Every block
        // descent passes through here. See [`crate::stack`].
        crate::stack::guard(|| self.emit_value_after_colon_inner(val, indent))
    }

    fn emit_value_after_colon_inner(&mut self, val: &Value, indent: usize) {
        match val {
            Value::Mapping(m) if self.is_block(val) => {
                self.buf.push(b'\n');
                self.emit_block_mapping(m, indent + self.step());
            }
            Value::Sequence(s) if self.is_block(val) => {
                self.buf.push(b'\n');
                // Indentless style aligns the dashes with the key; the default
                // indents the sequence one level under it.
                let seq_indent = if self.options.indentless_sequences {
                    indent
                } else {
                    indent + self.step()
                };
                self.emit_block_sequence(s, seq_indent);
            }
            Value::String(s) if Self::use_literal_block(s) => {
                self.buf.push(b' ');
                self.emit_literal_block(s, indent + self.step());
            }
            // An empty-style null in block mapping-value position emits nothing
            // after the colon (`key:`), which the parser reads back as null.
            Value::Null if self.options.null_style == NullStyle::Empty => {
                self.buf.push(b'\n');
            }
            // A custom-tagged value: emit `!tag` then its inner value exactly as
            // if it followed the colon, so a scalar stays inline (`!tag foo`), a
            // collection drops to an indented block (`!tag\n  a: 1`), and a
            // multi-line string becomes a tagged block scalar (`!tag |-`).
            Value::Tagged(tag, inner) => {
                self.buf.push(b' ');
                self.buf.extend_from_slice(tag.as_bytes());
                match inner.as_ref() {
                    // A tagged block sequence always indents under the tag, even
                    // in indentless mode: the tag sits on the line above, so the
                    // indentless style (dashes at the key's column) would not bind
                    // the tag to the sequence and it would be lost on reload. The
                    // indented layout is what the default mode already produces.
                    Value::Sequence(s) if self.is_block(inner) => {
                        self.buf.push(b'\n');
                        self.emit_block_sequence(s, indent + self.step());
                    }
                    _ => self.emit_value_after_colon(inner, indent),
                }
            }
            _ => {
                self.buf.push(b' ');
                self.emit_inline(val, false);
                self.buf.push(b'\n');
            }
        }
    }

    /// Whether a multi-line string can be emitted as a literal block scalar (`|`)
    /// that reads back identically. A literal block is the dominant real-world
    /// style for multi-line content, so it is the default; strings it cannot
    /// represent faithfully fall back to a double-quoted scalar.
    ///
    /// It cannot represent: a single-line string; a carriage return or other C0
    /// control character (only `\n` and `\t` are allowed in block content); or a
    /// first content line that begins with whitespace (the block's indentation is
    /// auto-detected from it, which would silently swallow the leading spaces).
    fn use_literal_block(value: &str) -> bool {
        if !value.contains('\n') {
            return false;
        }
        if value
            .bytes()
            .any(|b| (b < 0x20 && b != b'\n' && b != b'\t') || b == 0x7f)
        {
            return false;
        }
        let first_content = value.split('\n').find(|line| !line.is_empty());
        !matches!(first_content, Some(line) if line.starts_with([' ', '\t']))
    }

    /// Emit a string as a literal block scalar (`|`), choosing the chomping
    /// indicator from the value's trailing newlines so it round-trips exactly:
    /// none → strip (`|-`), one → clip (`|`), two or more → keep (`|+`).
    fn emit_literal_block(&mut self, value: &str, indent: usize) {
        let trailing = value.bytes().rev().take_while(|&b| b == b'\n').count();
        let body = value.trim_end_matches('\n');

        self.buf.push(b'|');
        match trailing {
            0 => self.buf.push(b'-'),
            // Clip (a single trailing newline) cannot represent an all-newline
            // value whose body is empty (`"\n"`): the body collapses to nothing
            // and the lone newline is chomped away on re-read. Keep (`+`) so the
            // trailing newline survives.
            1 if body.is_empty() => self.buf.push(b'+'),
            1 => {}
            _ => self.buf.push(b'+'),
        }
        self.buf.push(b'\n');

        for line in body.split('\n') {
            if line.is_empty() {
                self.buf.push(b'\n');
            } else {
                self.write_indent(indent);
                self.buf.extend_from_slice(line.as_bytes());
                self.buf.push(b'\n');
            }
        }
        // For "keep", emit the blank lines beyond the single implicit newline.
        for _ in 1..trailing {
            self.buf.push(b'\n');
        }
    }

    // -- Block sequence --

    fn emit_block_sequence(&mut self, items: &[Value], indent: usize) {
        // Sequence item content begins after "- ", two columns in.
        let child_indent = indent + 2;
        for item in items {
            self.write_indent(indent);
            self.buf.push(b'-');
            match item {
                Value::Mapping(m) if self.is_block(item) => {
                    self.buf.push(b' ');
                    self.emit_block_mapping_after_dash(m, child_indent);
                }
                Value::Sequence(s) if self.is_block(item) => {
                    self.buf.push(b'\n');
                    self.emit_block_sequence(s, child_indent);
                }
                Value::String(s) if Self::use_literal_block(s) => {
                    self.buf.push(b' ');
                    self.emit_literal_block(s, child_indent);
                }
                // An empty-style null as a block sequence entry is a bare `-`,
                // which the parser reads back as a null item.
                Value::Null if self.options.null_style == NullStyle::Empty => {
                    self.buf.push(b'\n');
                }
                // A tagged sequence item: `- !tag` then its inner value (inline
                // for a scalar, an indented block for a collection).
                Value::Tagged(tag, inner) => {
                    self.buf.push(b' ');
                    self.buf.extend_from_slice(tag.as_bytes());
                    match inner.as_ref() {
                        // A tagged block sequence must indent under the dash, even
                        // in indentless mode: the tag sits on the dash line, so an
                        // indentless inner sequence (dashes at the outer column)
                        // would not bind to the tag. Its dashes would read as more
                        // items of the outer sequence, dropping the tag and merging
                        // the nesting on reload. Indenting keeps it the tag's child.
                        Value::Sequence(s) if self.is_block(inner) => {
                            self.buf.push(b'\n');
                            self.emit_block_sequence(s, child_indent);
                        }
                        _ => self.emit_value_after_colon(inner, indent),
                    }
                }
                _ => {
                    self.buf.push(b' ');
                    self.emit_inline(item, false);
                    self.buf.push(b'\n');
                }
            }
        }
    }

    // -- Inline values (no trailing newline) --

    /// Emit a value on a single line. `in_flow` is true when the value sits
    /// inside a flow collection (`[...]`/`{...}`), where a string containing a
    /// flow indicator (`,[]{}`) must be quoted or it would break the structure;
    /// in block context (a block value or key) those characters are ordinary.
    fn emit_inline(&mut self, value: &Value, in_flow: bool) {
        // Every flow descent passes through here; grow the stack on demand so a
        // deeply nested flow collection cannot overflow a small thread stack.
        // See [`crate::stack`].
        crate::stack::guard(|| self.emit_inline_inner(value, in_flow))
    }

    fn emit_inline_inner(&mut self, value: &Value, in_flow: bool) {
        match value {
            Value::Null => self
                .buf
                .extend_from_slice(self.options.null_style.inline_token().as_bytes()),
            Value::Bool(true) => self.buf.extend_from_slice(b"true"),
            Value::Bool(false) => self.buf.extend_from_slice(b"false"),
            // Format the integer straight into the output buffer via itoa, a
            // specialized integer formatter that is faster than `core::fmt` and
            // produces the exact same digits, with no throwaway `String` per int.
            Value::Int(i) => {
                let mut itoa_buf = itoa::Buffer::new();
                self.buf.extend_from_slice(itoa_buf.format(*i).as_bytes());
            }
            // A big integer is already its exact decimal text; emit it verbatim
            // (it is all digits with an optional sign, so it needs no quoting).
            Value::BigInt(s) => self.buf.extend_from_slice(s.as_bytes()),
            Value::Float(f) => self.emit_float(*f),
            Value::String(s) => self.emit_string_inline(s, in_flow),
            Value::Sequence(items) => self.emit_flow_sequence(items),
            Value::Mapping(pairs) => self.emit_flow_mapping(pairs),
            Value::Tagged(tag, inner) => {
                self.buf.extend_from_slice(tag.as_bytes());
                self.buf.push(b' ');
                self.emit_inline(inner, in_flow);
            }
        }
    }

    /// Emit a mapping key inline. Keys are always rendered on a single line. A
    /// scalar key sits in block context (`false`); a collection key descends into
    /// flow internally, flagging its own elements.
    fn emit_key(&mut self, key: &Value) {
        // Suppress all width-driven line breaking for the whole key subtree: a
        // key folded or broken across lines is no longer a valid implicit key, so
        // the `:` after it detaches and the data changes on reload.
        let was_key = self.emitting_key;
        self.emitting_key = true;
        self.emit_inline(key, false);
        self.emitting_key = was_key;
    }

    fn emit_float(&mut self, value: f64) {
        self.buf
            .extend_from_slice(crate::emit_util::canonical_float(value).as_bytes());
    }

    fn emit_string_inline(&mut self, value: &str, in_flow: bool) {
        if value.is_empty() {
            let empty = if self.options.double_quotes {
                b"\"\"".as_slice()
            } else {
                b"''".as_slice()
            };
            self.buf.extend_from_slice(empty);
        } else if needs_quoting(value, self.options.schema)
            || (in_flow && has_flow_indicator(value))
        {
            // Inside a flow collection, a `,`/`[`/`]`/`{`/`}` anywhere in the
            // scalar would end the entry or collection early; quoting keeps it a
            // single value. In block context those bytes are ordinary content.
            self.emit_quoted_string(value);
        } else if self.would_wrap(value) {
            // A plain scalar that would exceed the width is emitted double-quoted
            // and folded inside the quotes, never folded as a bare plain scalar.
            // Inserting a newline into a plain scalar is unsafe: the continuation
            // line can begin with an indicator (`?`/`:`/`!`/`&`/`*`/...) or land
            // at an enclosing collection's indent, either of which changes the
            // decoded value (`M !` folding to `M\n  !` reads `!` as a tag;
            // `{k: a ? b}` folding reads `? b` as a key). A break inside quotes
            // always folds back to a single space, so quoting makes wrapping safe
            // in every position. Keys are excluded by `would_wrap`.
            self.emit_quoted_string(value);
        } else {
            self.buf.extend_from_slice(value.as_bytes());
        }
    }

    fn emit_quoted_string(&mut self, value: &str) {
        // Double quotes are the default. In single-quote mode use single quotes
        // when the value can be represented that way (a single-quoted scalar
        // cannot contain a line break, nor escape a control character), otherwise
        // fall back to double.
        let single_ok = !self.options.double_quotes
            && !value.contains('\'')
            && !value.contains('\n')
            && !value.contains('\r')
            && !value.bytes().any(|b| b < 0x20 || b == 0x7f);
        if self.options.width > 0 && !self.emitting_key {
            // Fold the (already-escaped) body between the quotes. A space in the
            // body folds the same way as in a plain scalar; the surrounding
            // quotes are unaffected. Never fold a key: a quoted key spread across
            // lines stops being a valid implicit key, so the `:` detaches.
            //
            // Continuation lines indent to the scalar's start column, which always
            // sits past the enclosing key/dash (the scalar is emitted after them).
            // A multi-line quoted scalar must be indented past its block context,
            // so the line's leading indent is too shallow for a value after a dash
            // (`- k: ...`, where it would land at the mapping's own column).
            //
            // Clamp a root scalar's column 0 to 1: a continuation line at column 0
            // that happens to start with `---`/`...` re-reads as a document marker
            // ("a document marker cannot appear inside a quoted scalar"). The fold
            // already strips a continuation line's leading whitespace on reload, so
            // the extra space leaves the value unchanged.
            let cont_indent = self.current_column().max(1);
            let (quote, body) = if single_ok {
                (b'\'', crate::emit_util::single_quoted_body(value))
            } else {
                (b'"', crate::emit_util::double_quoted_body(value))
            };
            self.buf.push(quote);
            self.emit_folded(&body, cont_indent);
            self.buf.push(quote);
        } else if single_ok {
            crate::emit_util::push_single_quoted(&mut self.buf, value);
        } else {
            crate::emit_util::push_double_quoted(&mut self.buf, value);
        }
    }

    fn emit_flow_sequence(&mut self, items: &[Value]) {
        self.buf.push(b'[');
        let cont_indent = self.current_line_indent() + self.step();
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.emit_flow_separator(cont_indent);
            }
            self.emit_inline(item, true);
        }
        self.buf.push(b']');
    }

    fn emit_flow_mapping(&mut self, pairs: &[(Value, Value)]) {
        self.buf.push(b'{');
        let cont_indent = self.current_line_indent() + self.step();
        let ordered = self.order_pairs(pairs);
        for (i, (key, val)) in ordered.iter().enumerate() {
            if i > 0 {
                self.emit_flow_separator(cont_indent);
            }
            self.emit_inline(key, true);
            self.buf.extend_from_slice(b": ");
            self.emit_inline(val, true);
        }
        self.buf.push(b'}');
    }

    /// The `, ` between flow entries, broken onto a new indented line when the
    /// current line has reached the width. Whitespace between flow tokens is
    /// insignificant, so a break here never changes the decoded value, except
    /// inside a key, where it would split the implicit key across lines.
    fn emit_flow_separator(&mut self, cont_indent: usize) {
        self.buf.push(b',');
        if self.options.width > 0
            && !self.emitting_key
            && self.current_column() >= self.options.width
        {
            self.buf.push(b'\n');
            self.write_indent(cont_indent);
        } else {
            self.buf.push(b' ');
        }
    }

    // -- Helpers --

    /// Return the pairs in emission order, sorted by key when requested.
    fn order_pairs<'p, 'v>(
        &self,
        pairs: &'p [(Value<'v>, Value<'v>)],
    ) -> Vec<&'p (Value<'v>, Value<'v>)> {
        let mut refs: Vec<&(Value<'v>, Value<'v>)> = pairs.iter().collect();
        if self.options.sort_keys {
            refs.sort_by(|(a, _), (b, _)| key_sort_str(a).cmp(key_sort_str(b)));
        }
        refs
    }

    fn write_indent(&mut self, indent: usize) {
        self.buf.resize(self.buf.len() + indent, b' ');
    }

    /// The column (in bytes) reached on the current, last line of `buf`.
    fn current_column(&self) -> usize {
        match self.buf.iter().rposition(|&b| b == b'\n') {
            Some(newline) => self.buf.len() - newline - 1,
            None => self.buf.len(),
        }
    }

    /// The leading-space indentation of the current line: the parent block's
    /// indent for a value being emitted on it. Continuation lines of a folded
    /// scalar must be indented deeper than this.
    fn current_line_indent(&self) -> usize {
        let start = self
            .buf
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |newline| newline + 1);
        self.buf[start..].iter().take_while(|&&b| b == b' ').count()
    }

    /// Whether a plain `value` at the current column would exceed the width and
    /// has a foldable break point, so it is worth emitting quoted-and-folded
    /// rather than leaving it to overflow on one line. False in key position: a
    /// key must stay on one line, so width never applies to it.
    fn would_wrap(&self, value: &str) -> bool {
        self.options.width > 0
            && !self.emitting_key
            && self.current_column() + value.len() > self.options.width
            && has_breakable_space(value)
    }

    /// Append `content` to `buf`, folding it across lines so each stays at or
    /// below `self.options.width`. A fold replaces one space with a newline plus
    /// `cont_indent` spaces; on reload that whole break collapses back to the
    /// single space, so the decoded value is unchanged. Breaks are only taken at
    /// a single space flanked by non-spaces, never inside a run of spaces (which
    /// would lose one), so a run of spaces or a break-free span may exceed the
    /// width. Used only for quoted scalar bodies (a break inside quotes is safe);
    /// a plain scalar that needs wrapping is quoted first (see `emit_string_inline`).
    fn emit_folded(&mut self, content: &str, cont_indent: usize) {
        let width = self.options.width;
        let bytes = content.as_bytes();
        let breakable = |i: usize| -> bool {
            bytes[i] == b' '
                && i > 0
                && bytes[i - 1] != b' '
                && bytes.get(i + 1).is_some_and(|&b| b != b' ')
        };
        let next_break = |from: usize| (from..bytes.len()).find(|&i| breakable(i));

        let mut col = self.current_column();
        let mut pos = 0;
        while let Some(brk) = next_break(pos) {
            self.buf.extend_from_slice(&bytes[pos..brk]);
            col += brk - pos;
            // Look ahead to the next unbreakable piece to decide fold vs. space.
            let after = brk + 1;
            let piece_end = next_break(after).unwrap_or(bytes.len());
            if col + 1 + (piece_end - after) > width {
                self.buf.push(b'\n');
                self.write_indent(cont_indent);
                col = cont_indent;
            } else {
                self.buf.push(b' ');
                col += 1;
            }
            pos = after;
        }
        self.buf.extend_from_slice(&bytes[pos..]);
    }
}

/// Borrow a string view of a key for sorting; non-strings sort as empty.
fn key_sort_str<'v>(value: &'v Value<'_>) -> &'v str {
    match value {
        Value::String(s) => s.as_ref(),
        _ => "",
    }
}

/// Whether the string contains a flow indicator (`,` `[` `]` `{` `}`) anywhere.
/// Such a character is ordinary content in a block scalar but ends an entry or
/// collection inside a flow context, so a flow-context scalar carrying one must
/// be quoted. ([`needs_quoting`] already covers a *leading* indicator; this also
/// catches one in the middle or at the end.)
fn has_flow_indicator(value: &str) -> bool {
    value
        .bytes()
        .any(|b| matches!(b, b',' | b'[' | b']' | b'{' | b'}'))
}

/// Whether `value` contains a space flanked by non-spaces: the only place a fold
/// may break, since breaking inside a run of spaces would drop one. Used to
/// decide whether wrapping a long scalar can actually shorten any line.
fn has_breakable_space(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        b == b' ' && i > 0 && bytes[i - 1] != b' ' && bytes.get(i + 1).is_some_and(|&n| n != b' ')
    })
}

/// Determine whether a plain scalar string would be misinterpreted and thus
/// needs quoting. Shared with the round-trip assignment path so an edited string
/// value is quoted by exactly the same rules as a freshly dumped one.
pub(crate) fn needs_quoting(value: &str, schema: Schema) -> bool {
    if value.is_empty() {
        return true;
    }

    // A leading byte order mark (U+FEFF) at stream start is stripped by the
    // scanner as an encoding marker, so an unquoted value beginning with one
    // would lose it. Quoting moves a real `\u{feff}` inside the quotes, where it
    // survives. (The round-trip emitter has the same guard.)
    if value.starts_with('\u{feff}') {
        return true;
    }

    // Strings that collide with YAML keywords (null/bool/inf/nan) or common
    // YAML 1.1 boolean spellings. Quoting these keeps `dumps` output stable
    // regardless of the reader's schema version. The single-letter forms
    // (`y`/`Y`/`n`/`N`) are deliberately omitted: they are common, legitimate
    // 1.2 keys and values (coordinates, counts), and quoting them everywhere
    // would harm the default-schema output for the same narrow, explicitly
    // opted-in 1.1 benefit as the sexagesimal case noted below.
    if matches!(
        value,
        "null"
            | "Null"
            | "NULL"
            | "~"
            | "true"
            | "True"
            | "TRUE"
            | "false"
            | "False"
            | "FALSE"
            | ".inf"
            | ".Inf"
            | ".INF"
            | "-.inf"
            | "-.Inf"
            | "-.INF"
            | "+.inf"
            | "+.Inf"
            | "+.INF"
            | ".nan"
            | ".NaN"
            | ".NAN"
            | "yes"
            | "Yes"
            | "YES"
            | "no"
            | "No"
            | "NO"
            | "on"
            | "On"
            | "ON"
            | "off"
            | "Off"
            | "OFF"
            // The document-end marker: an unquoted `...` reparses as the end of
            // the document (yielding null), so the string is lost. Its sibling
            // `---` is already covered by the leading-`-` indicator check below.
            | "..."
            // The merge indicator: an unquoted `<<` key reparses as a merge key,
            // changing the document. Quoting keeps it a literal string.
            | "<<"
    ) {
        return true;
    }

    // A string beginning with the document-end marker `...` followed by a space
    // or tab also reparses as the end of the document (the marker needs only
    // trailing whitespace, not a bare line), so everything after it is lost. The
    // exact `...` is covered by the keyword match above; the `---` document-start
    // marker (bare or with trailing content) is covered by the leading-`-`
    // indicator check below.
    if value.as_bytes().starts_with(b"...") && matches!(value.as_bytes().get(3), Some(b' ' | b'\t'))
    {
        return true;
    }

    // Strict YAML 1.1 reads bare `y`/`Y`/`n`/`N` as booleans. The default schema
    // and the PyYAML-compat 1.1 variant both treat them as plain strings (the
    // common, legitimate config use), so they are quoted only when the output
    // targets strict 1.1, where leaving them bare would flip a string to a bool.
    if schema == Schema::Yaml11 && matches!(value, "y" | "Y" | "n" | "N") {
        return true;
    }

    let bytes = value.as_bytes();

    // Leading indicator characters that change the meaning of the line.
    if matches!(
        bytes[0],
        b'-' | b'?'
            | b':'
            | b','
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'#'
            | b'&'
            | b'*'
            | b'!'
            | b'|'
            | b'>'
            | b'\''
            | b'"'
            | b'%'
            | b'@'
            | b'`'
    ) {
        return true;
    }

    // Leading or trailing whitespace must be quoted to survive a round-trip.
    if bytes[0] == b' ' || bytes[0] == b'\t' {
        return true;
    }
    if matches!(bytes[bytes.len() - 1], b' ' | b'\t') {
        return true;
    }

    // Sequences that introduce structure or comments inside the scalar, plus any
    // control character (C0 or DEL): a raw control byte in a plain scalar makes
    // YAML a spec-compliant reader rejects, so force quoting (and, in the quoted
    // path, escaping).
    for i in 0..bytes.len() {
        match bytes[i] {
            b':' if i + 1 == bytes.len() || bytes[i + 1] == b' ' => return true,
            b'#' if i > 0 && bytes[i - 1] == b' ' => return true,
            0x00..=0x1f | 0x7f => return true,
            _ => {}
        }
    }

    // Quote iff the value would resolve back to a *number* under the YAML 1.2 or
    // 1.1 schema, so a dumped string stays a string when re-loaded under either.
    // Reusing the resolvers (rather than a looser "looks numeric" heuristic) means
    // a string is quoted exactly when it has to be: `1.5`, `0x1F`, and the 1.1
    // forms `0777`/`0b101`/`1_000` are quoted, but `2.0.0` and `0.0.0.0` are
    // numbers under neither schema and emit unquoted (matching PyYAML and ruamel).
    // This also makes emit-quoting agree with decode-resolution by construction:
    // `dumps` never produces a scalar `loads` reads back as a different type.
    //
    // When the output targets a YAML 1.1 schema, quote exactly what *that* schema
    // re-reads as a number, so the value survives a round-trip through it. Both
    // 1.1 variants read sexagesimal (`1:30`, `10:20:30`) and leading-
    // underscore numerics (`_5`); PyYAML-compat differs from strict 1.1 only in
    // its boolean set (the bare `y`/`n` case handled above), not in numbers. The
    // 1.2 default keeps its conservative cross-schema quoting: it quotes 1.1
    // numbers for stability but leaves sexagesimal bare (it overlaps timestamps,
    // and 1.2 does not read it), exactly as before.
    //
    // Cheap gate first: every YAML int/float begins with a digit, sign, or dot
    // (and, under 1.1 only, a leading `_`), so a value that does not cannot be a
    // number. This skips the (relatively costly) resolver classification for the
    // overwhelmingly common non-numeric string value.
    let numeric_start = matches!(bytes[0], b'0'..=b'9' | b'+' | b'-' | b'.')
        || (bytes[0] == b'_' && schema != Schema::Yaml12);
    if !numeric_start {
        return false;
    }
    if resolves_to_number(value, Schema::Yaml12) {
        return true;
    }
    match schema {
        Schema::Yaml12 => !value.contains(':') && resolves_to_number(value, Schema::Yaml11),
        // A 1.1 reader (PyYAML, ruamel) also reads a timestamp/date (`2020-01-02`,
        // `2020-01-02T10:00:00Z`) as a `datetime`, not a string. yamlrocks does
        // not resolve timestamps itself, so `resolves_to_number` misses them;
        // quote them here so a dumped string re-reads as a string under 1.1
        // rather than flipping to a date. The 1.2 default leaves them bare (1.2
        // core does not read timestamps), as before.
        Schema::Yaml11 | Schema::Yaml11PyYaml => {
            resolves_to_number(value, schema) || is_yaml_11_timestamp(value)
        }
    }
}

/// Whether `value` matches the YAML 1.1 timestamp shape a 1.1 reader (PyYAML,
/// ruamel) resolves to a date/datetime. Mirrors PyYAML's timestamp regex: a
/// `YYYY-MM-DD` date, or a full `YYYY-M-D(T| )H:MM:SS(.frac)?( ?TZ)?` datetime.
fn is_yaml_11_timestamp(value: &str) -> bool {
    let b = value.as_bytes();
    let n = b.len();
    let is_digit = |i: usize| i < n && b[i].is_ascii_digit();
    // Year: exactly four digits then `-`.
    if !(is_digit(0) && is_digit(1) && is_digit(2) && is_digit(3)) || n < 5 || b[4] != b'-' {
        return false;
    }
    // The date-only form is exactly `YYYY-MM-DD` (two-digit month and day).
    if n == 10 && is_digit(5) && is_digit(6) && b[7] == b'-' && is_digit(8) && is_digit(9) {
        return true;
    }
    // Full datetime: month and day may be one or two digits.
    let mut i = 5;
    let take_1_2_digits = |start: usize| -> Option<usize> {
        let mut j = start;
        while j < n && b[j].is_ascii_digit() && j - start < 2 {
            j += 1;
        }
        (j > start).then_some(j)
    };
    let Some(j) = take_1_2_digits(i) else {
        return false;
    };
    i = j;
    if i >= n || b[i] != b'-' {
        return false;
    }
    i += 1;
    let Some(j) = take_1_2_digits(i) else {
        return false;
    };
    i = j;
    // Separator: `T`/`t`, or one or more spaces/tabs.
    if i < n && (b[i] == b'T' || b[i] == b't') {
        i += 1;
    } else if i < n && (b[i] == b' ' || b[i] == b'\t') {
        while i < n && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
    } else {
        return false;
    }
    // Hour: one or two digits, then `:MM:SS`.
    let Some(j) = take_1_2_digits(i) else {
        return false;
    };
    i = j;
    let two_digits = |start: usize| -> bool {
        start + 1 < n && b[start].is_ascii_digit() && b[start + 1].is_ascii_digit()
    };
    if i >= n || b[i] != b':' || !two_digits(i + 1) {
        return false;
    }
    i += 3;
    if i >= n || b[i] != b':' || !two_digits(i + 1) {
        return false;
    }
    i += 3;
    // Optional fractional seconds `.` then zero or more digits.
    if i < n && b[i] == b'.' {
        i += 1;
        while i < n && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    // Optional time zone: spaces/tabs then `Z`, or `+`/`-`H(H)(:MM)?.
    while i < n && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if i == n {
        return true;
    }
    if b[i] == b'Z' {
        return i + 1 == n;
    }
    if b[i] == b'+' || b[i] == b'-' {
        i += 1;
        let Some(j) = take_1_2_digits(i) else {
            return false;
        };
        i = j;
        if i < n {
            // Optional `:MM`.
            if b[i] != b':' || !two_digits(i + 1) {
                return false;
            }
            i += 3;
        }
        return i == n;
    }
    false
}

/// Whether `value` resolves to an integer or float (not a string, bool, or null)
/// as a plain scalar under `schema`. Used to decide whether `dumps` must quote a
/// string so it does not re-read as a number.
fn resolves_to_number(value: &str, schema: Schema) -> bool {
    matches!(
        schema.classify(value, ScalarStyle::Plain, None),
        ScalarKind::Int(_) | ScalarKind::BigInt | ScalarKind::Float(_)
    )
}

#[cfg(test)]
mod tests {
    use super::{encode, EmitOptions, NullStyle};
    use crate::decode::Value;
    use crate::resolver::Schema;

    fn s(text: &str) -> Value<'static> {
        Value::String(text.to_owned().into())
    }
    fn emit(value: &Value<'_>, options: &EmitOptions) -> String {
        String::from_utf8(encode(value, options)).unwrap()
    }

    /// Decode `out` under the 1.2 schema, asserting it is a single document, and
    /// return that document (borrowing `out`).
    fn reparse(out: &str) -> Value<'_> {
        let mut docs =
            crate::decode::decode_with(out, crate::resolver::Schema::Yaml12, false, false).unwrap();
        assert_eq!(docs.len(), 1, "one document: {out:?}");
        docs.remove(0)
    }

    #[test]
    fn a_key_is_never_wrapped_across_the_width() {
        // A mapping key must stay on one line to remain a valid implicit key.
        // Wrapping a long key at a small width would split it across lines,
        // detaching the `:` so the value reads back as a separate node. The key
        // needs quoting (control byte) and is long enough to tempt a fold.
        let v = Value::Mapping(vec![(s("a long \u{1} key here"), Value::Null)]);
        let out = emit(
            &v,
            &EmitOptions {
                width: 1,
                ..EmitOptions::default()
            },
        );
        assert_eq!(reparse(&out), v, "key round-trips: {out:?}");
    }

    #[test]
    fn a_plain_value_with_an_indicator_word_wraps_safely_under_width() {
        // A plain value `M !` folded as plain to honor the width would put `!` at
        // line start, where it reads as a tag (a phantom key). Quote-to-wrap emits
        // it double-quoted and folds inside the quotes, where the break is safe.
        let v = Value::Sequence(vec![Value::Mapping(vec![(s("-"), s("M !"))])]);
        let out = emit(
            &v,
            &EmitOptions {
                width: 1,
                ..EmitOptions::default()
            },
        );
        assert_eq!(reparse(&out), v, "block plain value round-trips: {out:?}");
    }

    #[test]
    fn a_flow_plain_value_with_an_indicator_wraps_safely_under_width() {
        // Inside flow, folding `a ? b` as plain would start a line with `?` (an
        // explicit-key indicator). Quote-to-wrap keeps it a single value.
        let v = Value::Mapping(vec![(s("k"), s("a ? b c d e f"))]);
        let out = emit(
            &v,
            &EmitOptions {
                flow_style: true,
                width: 8,
                ..EmitOptions::default()
            },
        );
        assert_eq!(reparse(&out), v, "flow plain value round-trips: {out:?}");
    }

    #[test]
    fn scalar_mapping() {
        let v = Value::Mapping(vec![(s("a"), Value::Int(1)), (s("b"), Value::Bool(true))]);
        assert_eq!(emit(&v, &EmitOptions::default()), "a: 1\nb: true\n");
    }

    #[test]
    fn block_sequence() {
        let v = Value::Sequence(vec![Value::Int(1), Value::Int(2)]);
        assert_eq!(emit(&v, &EmitOptions::default()), "- 1\n- 2\n");
    }

    #[test]
    fn nested_block_uses_indent() {
        let v = Value::Mapping(vec![(
            s("a"),
            Value::Mapping(vec![(s("b"), Value::Int(1))]),
        )]);
        assert_eq!(emit(&v, &EmitOptions::default()), "a:\n  b: 1\n");
        let four = EmitOptions {
            indent: 4,
            ..EmitOptions::default()
        };
        assert_eq!(emit(&v, &four), "a:\n    b: 1\n");
    }

    #[test]
    fn null_style_empty_is_default() {
        // The default null style is an empty node, matching the dominant
        // real-world configuration style.
        let v = Value::Mapping(vec![(s("a"), Value::Null)]);
        assert_eq!(emit(&v, &EmitOptions::default()), "a:\n");
        // The explicit keyword is opt-in.
        let keyword = EmitOptions {
            null_style: NullStyle::Null,
            ..EmitOptions::default()
        };
        assert_eq!(emit(&v, &keyword), "a: null\n");
    }

    #[test]
    fn null_style_empty_in_block_positions() {
        let opts = EmitOptions {
            null_style: NullStyle::Empty,
            ..EmitOptions::default()
        };
        let v = Value::Mapping(vec![
            (s("a"), Value::Null),
            (s("l"), Value::Sequence(vec![Value::Int(1), Value::Null])),
        ]);
        assert_eq!(emit(&v, &opts), "a:\nl:\n  - 1\n  -\n");
    }

    #[test]
    fn null_style_tilde_everywhere() {
        let opts = EmitOptions {
            null_style: NullStyle::Tilde,
            ..EmitOptions::default()
        };
        let v = Value::Mapping(vec![(s("a"), Value::Null)]);
        assert_eq!(emit(&v, &opts), "a: ~\n");
        // Tilde is also used at the top level and in flow, where empty is unsafe.
        assert_eq!(emit(&Value::Null, &opts), "~\n");
    }

    #[test]
    fn null_style_empty_falls_back_where_ambiguous() {
        let opts = EmitOptions {
            null_style: NullStyle::Empty,
            ..EmitOptions::default()
        };
        // Top-level scalar and a mapping key cannot be empty: fall back to `null`.
        assert_eq!(emit(&Value::Null, &opts), "null\n");
        let keyed = Value::Mapping(vec![(Value::Null, Value::Int(1))]);
        assert_eq!(emit(&keyed, &opts), "null: 1\n");
    }

    #[test]
    fn indentless_sequences() {
        let v = Value::Mapping(vec![(
            s("key"),
            Value::Sequence(vec![Value::Int(1), Value::Int(2)]),
        )]);
        // Default indents the sequence under its key.
        assert_eq!(emit(&v, &EmitOptions::default()), "key:\n  - 1\n  - 2\n");
        // Indentless aligns the dashes with the key.
        let opts = EmitOptions {
            indentless_sequences: true,
            ..EmitOptions::default()
        };
        assert_eq!(emit(&v, &opts), "key:\n- 1\n- 2\n");
    }

    #[test]
    fn tagged_sequence_value_keeps_its_tag_in_indentless_mode() {
        // A tagged sequence value must indent under the tag even in indentless
        // mode: dashes at the key's column would sit below the tag's line without
        // binding to it, so the tag would be lost on reload. The tagged sequence
        // therefore emits indented, and the document round-trips.
        let v = Value::Mapping(vec![(
            s("k"),
            Value::Tagged(
                "!t".to_owned(),
                Box::new(Value::Sequence(vec![s("a"), s("b")])),
            ),
        )]);
        let opts = EmitOptions {
            indentless_sequences: true,
            ..EmitOptions::default()
        };
        let out = emit(&v, &opts);
        assert_eq!(out, "k: !t\n  - a\n  - b\n");
        assert_eq!(reparse(&out), v, "tag survives the round-trip: {out:?}");
    }

    #[test]
    fn tagged_sequence_item_keeps_its_tag_in_indentless_mode() {
        // A tagged block sequence that is itself an *item* of a sequence must
        // indent under the dash even in indentless mode: the tag sits on the dash
        // line, so an indentless inner sequence (dashes at the outer column) would
        // read as more items of the outer sequence, dropping the tag and merging
        // the nesting on reload.
        let v = Value::Sequence(vec![
            Value::Null,
            Value::Tagged(
                "!t".to_owned(),
                Box::new(Value::Sequence(vec![Value::Null])),
            ),
            Value::Null,
        ]);
        let opts = EmitOptions {
            indentless_sequences: true,
            ..EmitOptions::default()
        };
        let out = emit(&v, &opts);
        assert_eq!(out, "-\n- !t\n  -\n-\n");
        assert_eq!(reparse(&out), v, "tag survives the round-trip: {out:?}");
    }

    #[test]
    fn flow_style() {
        let opts = EmitOptions {
            flow_style: true,
            ..EmitOptions::default()
        };
        let v = Value::Mapping(vec![(
            s("a"),
            Value::Sequence(vec![Value::Int(1), Value::Int(2)]),
        )]);
        assert_eq!(emit(&v, &opts), "{a: [1, 2]}\n");
    }

    #[test]
    fn sort_keys() {
        let opts = EmitOptions {
            sort_keys: true,
            ..EmitOptions::default()
        };
        let v = Value::Mapping(vec![
            (s("c"), Value::Int(1)),
            (s("a"), Value::Int(2)),
            (s("b"), Value::Int(3)),
        ]);
        assert_eq!(emit(&v, &opts), "a: 2\nb: 3\nc: 1\n");
    }

    #[test]
    fn ambiguous_strings_are_quoted() {
        let v = Value::Mapping(vec![
            (s("x"), s("yes")),
            (s("y"), s("true")),
            (s("z"), s("null")),
        ]);
        // Double quotes are the default.
        let out = emit(&v, &EmitOptions::default());
        assert!(out.contains("\"yes\""), "{out}");
        assert!(out.contains("\"true\""), "{out}");
        assert!(out.contains("\"null\""), "{out}");
        // Single quotes are opt-in.
        let single = EmitOptions {
            double_quotes: false,
            ..EmitOptions::default()
        };
        assert!(emit(&v, &single).contains("'yes'"));
    }

    #[test]
    fn document_marker_strings_are_quoted() {
        // A `...` document-end marker needs only trailing whitespace, so a string
        // like `... y` re-reads as the end of the document and loses everything
        // after it unless quoted. The exact `...` and any `---`-prefixed string
        // are covered too; a `...` with non-whitespace after it (`...x`) is not a
        // marker and stays bare.
        for marker in ["...", "... y", "...  z", "---", "--- y"] {
            let out = emit(&s(marker), &EmitOptions::default());
            assert_eq!(reparse(&out), s(marker), "marker round-trips: {out:?}");
        }
        // Not a marker: no trailing whitespace, so no quoting is forced.
        assert_eq!(emit(&s("...x"), &EmitOptions::default()), "...x\n");
    }

    #[test]
    fn folded_root_scalar_never_starts_a_line_with_a_marker() {
        // Folding a quoted scalar at the document root indents continuation lines
        // to column 0, where a piece like `---`/`...` re-reads as a document marker
        // ("a document marker cannot appear inside a quoted scalar"). Continuation
        // indent is clamped to 1 so no line begins at column 0; the leading space
        // is stripped on reload, so the value is unchanged.
        let opts = EmitOptions {
            width: 1,
            ..EmitOptions::default()
        };
        for value in ["p= ---  ]", "x --- y", "a ... b", "..."] {
            let out = emit(&s(value), &opts);
            assert_eq!(
                reparse(&out),
                s(value),
                "folded marker round-trips: {out:?}"
            );
        }
    }

    #[test]
    fn numeric_looking_string_is_quoted() {
        let v = Value::Mapping(vec![(s("version"), s("1.0"))]);
        let out = emit(&v, &EmitOptions::default());
        assert!(out.contains("\"1.0\""), "{out}");
    }

    #[test]
    fn yaml_1_1_quotes_what_its_schema_reads_as_non_strings() {
        // `y`/`N` are booleans only under strict 1.1; sexagesimal (`10:20:30`) is
        // a number under both 1.1 variants. Each schema quotes exactly what it
        // would re-read as a non-string, so the output round-trips under it.
        let v = Value::Mapping(vec![
            (s("a"), s("y")),
            (s("b"), s("N")),
            (s("c"), s("10:20:30")),
        ]);
        // Default (1.2): none of these are non-strings, so all bare.
        let dflt = emit(&v, &EmitOptions::default());
        assert_eq!(dflt, "a: y\nb: N\nc: 10:20:30\n");
        // Strict 1.1: bare `y`/`N` bools and sexagesimal all quoted.
        let y11 = emit(
            &v,
            &EmitOptions {
                schema: Schema::Yaml11,
                ..EmitOptions::default()
            },
        );
        assert_eq!(y11, "a: \"y\"\nb: \"N\"\nc: \"10:20:30\"\n");
        // PyYAML-compat 1.1: bare `y`/`N` are NOT booleans (left bare), but it
        // still reads sexagesimal, so `10:20:30` is quoted.
        let pyyaml = emit(
            &v,
            &EmitOptions {
                schema: Schema::Yaml11PyYaml,
                ..EmitOptions::default()
            },
        );
        assert_eq!(pyyaml, "a: y\nb: N\nc: \"10:20:30\"\n");
    }

    #[test]
    fn yaml_11_dump_quotes_timestamp_strings() {
        // A 1.1 reader resolves these to a date/datetime, so a dumped *string*
        // must be quoted under a 1.1 target to survive as a string.
        for schema in [Schema::Yaml11, Schema::Yaml11PyYaml] {
            let v = Value::Mapping(vec![(s("d"), s("2020-01-02"))]);
            let out = emit(
                &v,
                &EmitOptions {
                    schema,
                    ..EmitOptions::default()
                },
            );
            assert_eq!(out, "d: \"2020-01-02\"\n", "{schema:?}");
        }
        // The 1.2 default leaves a timestamp bare (1.2 core does not read it).
        let v = Value::Mapping(vec![(s("d"), s("2020-01-02"))]);
        assert_eq!(emit(&v, &EmitOptions::default()), "d: 2020-01-02\n");
    }

    #[test]
    fn timestamp_shape_matcher_matches_pyyaml_forms() {
        use super::is_yaml_11_timestamp;
        // Matched: date, and datetime with `T`/space, fractional seconds, zones.
        for t in [
            "2020-01-02",
            "2020-1-2T10:00:00",
            "2001-12-15 2:59:43.10",
            "2020-01-02t10:00:00.5Z",
            "2020-01-02 10:00:00 +05:00",
            "2020-01-02T10:00:00-5",
        ] {
            assert!(is_yaml_11_timestamp(t), "should match {t:?}");
        }
        // Not matched: single-digit date-only, trailing junk, wrong shape.
        for t in [
            "2020-1-2",
            "2020-01-02x",
            "2020-01-02T10:00",
            "hello",
            "1:30",
            "2020-01-02T10:00:00Zx",
        ] {
            assert!(!is_yaml_11_timestamp(t), "should not match {t:?}");
        }
    }

    #[test]
    fn explicit_document_markers() {
        let opts = EmitOptions {
            explicit_start: true,
            explicit_end: true,
            ..EmitOptions::default()
        };
        let v = Value::Mapping(vec![(s("a"), Value::Int(1))]);
        assert_eq!(emit(&v, &opts), "---\na: 1\n...\n");
    }
}
