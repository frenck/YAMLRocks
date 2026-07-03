use std::collections::HashSet;

use crate::parser::{Event, EventKind, Parser};
use crate::resolver::{ResolvedValue, Schema};
use crate::scanner::{Comment, ScalarStyle, ScanError, Span};

use super::ast::{Comments, NodeStyle, YamlNode, YamlNodeKind};

/// Compose a YAML string into a rich AST that preserves all structure,
/// including comments reattached to their nearest nodes.
pub fn compose(input: &str) -> Result<Vec<YamlNode>, ScanError> {
    let mut parser = Parser::new(input);
    let (events, comments) = parser.parse_all_with_comments()?;
    let mut composer = Composer::new(input);
    let mut nodes = composer.compose_stream(&events, false)?;
    attach_comments(&mut nodes, &comments, input);
    mark_leading_bom(&mut nodes, parser.had_bom());
    Ok(nodes)
}

/// Record a stream-leading byte order mark on the first document root so the
/// round-trip emitter can restore it. No-op for a stream without one (or an
/// empty stream).
fn mark_leading_bom(nodes: &mut [YamlNode], had_bom: bool) {
    if had_bom {
        if let Some(first) = nodes.first_mut() {
            first.leading_bom = true;
        }
    }
}

/// Like [`compose`], but every explicit `---` yields a document even when empty
/// (a trailing or lone `---` becomes a null node), matching PyYAML's
/// `safe_load_all` document count. Used by the multi-document annotated path
/// (`loads_all`); the round-trip path uses [`compose`], which drops empty
/// documents so byte-for-byte re-emission is unaffected.
pub fn compose_all(input: &str) -> Result<Vec<YamlNode>, ScanError> {
    let mut parser = Parser::new(input);
    let (events, comments) = parser.parse_all_with_comments()?;
    let mut composer = Composer::new(input);
    let mut nodes = composer.compose_stream(&events, true)?;
    attach_comments(&mut nodes, &comments, input);
    mark_leading_bom(&mut nodes, parser.had_bom());
    Ok(nodes)
}

/// Compose with an explicit `file_id` stamped on every span, for include
/// resolution and source-file tracking.
pub fn compose_with_file_id(input: &str, file_id: u32) -> Result<Vec<YamlNode>, ScanError> {
    let mut parser = Parser::new_with_file_id(input, file_id);
    let (events, comments) = parser.parse_all_with_comments()?;
    let mut composer = Composer::new(input);
    let mut nodes = composer.compose_stream(&events, false)?;
    attach_comments(&mut nodes, &comments, input);
    mark_leading_bom(&mut nodes, parser.had_bom());
    Ok(nodes)
}

/// Reject any block/flow mapping in the composed AST that repeats a key, the
/// AST-path equivalent of the fast decoder's `OPT_DUPLICATE_KEYS_ERROR`. The
/// merge key `<<` is exempt, matching the fast path. Keys are compared by their
/// resolved value, so `1` and `"1"` are distinct but `1` and `0x1` are not.
pub fn check_duplicate_keys(nodes: &[YamlNode], schema: Schema) -> Result<(), ScanError> {
    for node in nodes {
        check_node_duplicate_keys(node, schema)?;
    }
    Ok(())
}

fn check_node_duplicate_keys(node: &YamlNode, schema: Schema) -> Result<(), ScanError> {
    // Grow the native stack on demand: this recurses once per nesting level over
    // attacker-controlled AST depth (bounded by the composer's MAX_DEPTH), so a
    // small thread stack could otherwise overflow. See [`crate::stack`].
    crate::stack::guard(|| check_node_duplicate_keys_inner(node, schema))
}

fn check_node_duplicate_keys_inner(node: &YamlNode, schema: Schema) -> Result<(), ScanError> {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            // A `HashSet` of key signatures gives O(1) membership; a linear scan
            // made a large mapping quadratic. `<<` is exempt and never recorded.
            let mut seen: HashSet<String> = HashSet::with_capacity(pairs.len());
            for (key, val) in pairs {
                let signature = key_signature(key, schema);
                if signature != "s:<<" && !seen.insert(signature) {
                    return Err(ScanError::new(
                        format!("duplicate mapping key: {}", key_display(key)),
                        key.span,
                    ));
                }
                check_node_duplicate_keys(key, schema)?;
                check_node_duplicate_keys(val, schema)?;
            }
            Ok(())
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                check_node_duplicate_keys(item, schema)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Collect a non-fatal diagnostic for every repeated mapping key, the warn-mode
/// counterpart to [`check_duplicate_keys`]: it walks the whole tree rather than
/// stopping at the first duplicate, and never errors. Each message names the key
/// and its 1-based line/column, for the caller to emit through Python logging.
pub fn collect_duplicate_keys(nodes: &[YamlNode], schema: Schema) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        collect_node_duplicate_keys(node, schema, &mut out);
    }
    out
}

fn collect_node_duplicate_keys(node: &YamlNode, schema: Schema, out: &mut Vec<String>) {
    // Grow the native stack on demand (see `check_node_duplicate_keys`); this
    // recurses over attacker-controlled AST depth.
    crate::stack::guard(|| collect_node_duplicate_keys_inner(node, schema, out))
}

fn collect_node_duplicate_keys_inner(node: &YamlNode, schema: Schema, out: &mut Vec<String>) {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            // `HashSet` membership is O(1); a linear scan made a large mapping
            // quadratic. `<<` is exempt and never recorded.
            let mut seen: HashSet<String> = HashSet::with_capacity(pairs.len());
            for (key, val) in pairs {
                let signature = key_signature(key, schema);
                if signature != "s:<<" && !seen.insert(signature) {
                    out.push(format!(
                        "duplicate mapping key '{}' at line {}, column {}; keeping the last value",
                        key_display(key),
                        key.span.line + 1,
                        key.span.column + 1
                    ));
                }
                collect_node_duplicate_keys(key, schema, out);
                collect_node_duplicate_keys(val, schema, out);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_node_duplicate_keys(item, schema, out);
            }
        }
        _ => {}
    }
}

/// Collect a diagnostic for every plain scalar whose resolved type differs
/// between YAML 1.1 and 1.2 (the 1.1-only constructs a migration must find), for
/// the caller to emit through Python logging. The AST-path counterpart to the
/// fast decoder's per-scalar check; both share [`crate::resolver::yaml_11_divergence`].
pub fn collect_yaml_11_divergences(nodes: &[YamlNode], schema: Schema) -> Vec<String> {
    let mut out = Vec::new();
    for node in nodes {
        collect_node_divergences(node, schema, &mut out);
    }
    out
}

fn collect_node_divergences(node: &YamlNode, schema: Schema, out: &mut Vec<String>) {
    match &node.kind {
        YamlNodeKind::Scalar(text, style) => {
            if let Some((t11, t12)) =
                crate::resolver::yaml_11_divergence(schema, text, *style, node.tag.as_deref())
            {
                out.push(format!(
                    "YAML 1.1 syntax '{text}' resolves as {t11} in 1.1 but {t12} in 1.2 \
                     at line {}, column {}",
                    node.span.line + 1,
                    node.span.column + 1
                ));
            }
        }
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                collect_node_divergences(key, schema, out);
                collect_node_divergences(val, schema, out);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_node_divergences(item, schema, out);
            }
        }
        YamlNodeKind::Null | YamlNodeKind::Alias(_) => {}
    }
}

/// A canonical, type-tagged signature for a mapping key so equal keys collide
/// and differently-typed keys (`1` vs `"1"`) do not. Numeric keys collapse the
/// way Python's `dict` does (`1`, `1.0`, and `True` are one key), so the
/// duplicate-key check catches a collision that only appears once the mapping
/// becomes a `dict`. The plain merge key `<<` keeps the `s:<<` signature its
/// caller special-cases.
fn key_signature(node: &YamlNode, schema: Schema) -> String {
    match &node.kind {
        YamlNodeKind::Null => "null".to_owned(),
        YamlNodeKind::Scalar(text, style) => {
            let resolved = schema.resolve(text, *style, node.tag.as_deref());
            match resolved {
                ResolvedValue::Null => "null".to_owned(),
                // Python: `True == 1`, `False == 0`, so a bool collides with the
                // matching integer; an integral float collides with the integer.
                ResolvedValue::Bool(b) => format!("n:{}", i64::from(b)),
                ResolvedValue::Int(i) => format!("n:{i}"),
                ResolvedValue::BigInt(s) => format!("n:{s}"),
                ResolvedValue::Float(f) => {
                    if f.is_finite()
                        && f.fract() == 0.0
                        && (-9.223_372_036_854_776e18..9.223_372_036_854_776e18).contains(&f)
                    {
                        format!("n:{}", f as i64)
                    } else {
                        format!("f:{}", f.to_bits())
                    }
                }
                ResolvedValue::String(s) => format!("s:{s}"),
            }
        }
        YamlNodeKind::Sequence(items) => {
            let inner: Vec<String> = items.iter().map(|n| key_signature(n, schema)).collect();
            format!("seq:[{}]", inner.join(","))
        }
        YamlNodeKind::Mapping(pairs) => {
            let inner: Vec<String> = pairs
                .iter()
                .map(|(k, v)| format!("{}={}", key_signature(k, schema), key_signature(v, schema)))
                .collect();
            format!("map:{{{}}}", inner.join(","))
        }
        YamlNodeKind::Alias(name) => format!("alias:{name}"),
    }
}

/// A short human-readable name for a key, for the duplicate-key error message.
fn key_display(node: &YamlNode) -> String {
    match &node.kind {
        YamlNodeKind::Scalar(text, _) => text.clone(),
        other => format!("{other:?}"),
    }
}

/// Reattach scanned comments to AST nodes by source position.
///
/// Comments arrive in source order. We walk the AST in document order with a
/// cursor into the comment list and apply two rules:
///
/// * A comment that sits on its own line(s) *above* a node becomes that node's
///   head comment.
/// * A comment trailing a value on the same line becomes that value's inline
///   comment.
///
/// Any comments left after the walk (trailing the final node) become foot
/// comments on the last top-level node.
fn attach_comments(nodes: &mut [YamlNode], comments: &[Comment], input: &str) {
    // Unlike comment attachment, blank-line spacing must be recorded even for a
    // document with no comments at all, so this walk always runs.
    //
    // Blank source lines are collected up front (a line is blank when it has no
    // non-whitespace content). A node's leading blank count is then the size of
    // the intersection of the gap before it with this set, rather than `end()`
    // arithmetic, which is only a best-effort estimate for block scalars and
    // would miscount the content lines of a `|`/`>` block as blanks.
    let blank_lines: std::collections::HashSet<u32> = input
        .split('\n')
        .enumerate()
        .filter(|(_, text)| text.trim().is_empty())
        .map(|(i, _)| i as u32)
        .collect();
    let mut cursor = 0usize;
    let mut prev_line: Option<u32> = None;
    for node in nodes.iter_mut() {
        visit_for_comments(
            node,
            comments,
            &blank_lines,
            &mut cursor,
            &mut prev_line,
            true,
        );
    }
    if cursor < comments.len() {
        if let Some(last) = nodes.last_mut() {
            for comment in &comments[cursor..] {
                last.comments.foot.push(comment.text.clone());
            }
        }
    }
}

/// Raise the running "last source line seen" to at least `line`.
fn bump(prev_line: &mut Option<u32>, line: u32) {
    *prev_line = Some(prev_line.map_or(line, |p| p.max(line)));
}

/// Flag a block-sequence item that opened on its parent's dash line (`- - 1`),
/// the compact nested form, so the emitter keeps it on that line instead of
/// breaking to the next. `dash_line` is the source line of the parent `-`; a
/// block sequence whose own first `-` shares that line was written compactly.
fn mark_compact_nested_sequence(node: &mut YamlNode, dash_line: u32) {
    if matches!(node.kind, YamlNodeKind::Sequence(_))
        && node.style == NodeStyle::Block
        && node.span.line == dash_line
    {
        node.comments.compact = true;
    }
}

/// Capture the padding between a block sequence `-` and an inline item sharing
/// its line (`-    item`), mirroring the mapping `:` rule. Skipped for a block
/// item, or one carrying an anchor or tag (emitted between the dash and the
/// item), where `node.span.column` would overcount the gap.
fn mark_dash_pad(node: &mut YamlNode, dash_span: Span) {
    let inline = matches!(node.kind, YamlNodeKind::Scalar(..) | YamlNodeKind::Alias(_))
        || node.style == NodeStyle::Flow;
    if inline
        && node.span.line == dash_span.line
        && node.span.column > dash_span.column
        && node.anchor.is_none()
        && node.tag.is_none()
    {
        node.comments.value_pad = node.span.column - dash_span.column - 1;
    }
}

/// Count the blank source lines strictly between `prev` and `block_start`.
fn blanks_between(
    blank_lines: &std::collections::HashSet<u32>,
    prev: u32,
    block_start: u32,
) -> u32 {
    ((prev + 1)..block_start)
        .filter(|line| blank_lines.contains(line))
        .count() as u32
}

/// Walk a single node, recording its leading blank lines, consuming head
/// comments before it, and capturing an inline comment (with its spacing) after
/// it. `prev_line` threads the last source line seen so blank gaps can be
/// measured. `is_value` marks nodes that may own a trailing inline comment
/// (mapping values, sequence items, document roots); mapping keys never do.
fn visit_for_comments(
    node: &mut YamlNode,
    comments: &[Comment],
    blank_lines: &std::collections::HashSet<u32>,
    cursor: &mut usize,
    prev_line: &mut Option<u32>,
    is_value: bool,
) {
    // Grow the native stack on demand so reattaching comments over a deeply
    // nested document cannot overflow a small thread stack. See [`crate::stack`].
    crate::stack::guard(|| {
        visit_for_comments_inner(node, comments, blank_lines, cursor, prev_line, is_value)
    })
}

fn visit_for_comments_inner(
    node: &mut YamlNode,
    comments: &[Comment],
    blank_lines: &std::collections::HashSet<u32>,
    cursor: &mut usize,
    prev_line: &mut Option<u32>,
    is_value: bool,
) {
    let line = node.span.line;
    let column = node.span.column;

    // This node's leading block starts at its first head comment (if one sits
    // above it) or at the node's own line. Blank source lines between the
    // previous content and that start are preserved so section spacing survives.
    let block_start = if *cursor < comments.len() && comments[*cursor].span.line < line {
        comments[*cursor].span.line
    } else {
        line
    };
    if let Some(prev) = *prev_line {
        node.comments.blank_before = blanks_between(blank_lines, prev, block_start);
    }

    // Head: standalone comment lines strictly above this node.
    while *cursor < comments.len() && comments[*cursor].span.line < line {
        node.comments.head.push(comments[*cursor].text.clone());
        bump(prev_line, comments[*cursor].span.line);
        *cursor += 1;
    }
    bump(prev_line, line);

    // Recurse into children in document order.
    match &mut node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs.iter_mut() {
                visit_for_comments(key, comments, blank_lines, cursor, prev_line, false);
                // Padding between the key's `:` and an inline value sharing its
                // line (`example:      true`). The colon sits just past the key,
                // so the gap is the value column minus the key end, less one.
                // Restricted to a plain scalar key (its end column equals its
                // source width, with no quotes or escapes to throw off the math)
                // whose value carries no anchor or tag (those are emitted between
                // the colon and the value, overcounting `val.span.column`).
                let (key_end_line, key_end_col) = key.end();
                let plain_key = matches!(&key.kind, YamlNodeKind::Scalar(_, ScalarStyle::Plain));
                if plain_key
                    && key_end_line == val.span.line
                    && val.span.column > key_end_col
                    && val.anchor.is_none()
                    && val.tag.is_none()
                {
                    val.comments.value_pad = val.span.column - key_end_col - 1;
                }
                visit_for_comments(val, comments, blank_lines, cursor, prev_line, true);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items.iter_mut() {
                visit_for_comments(item, comments, blank_lines, cursor, prev_line, true);
            }
        }
        _ => {}
    }
    // Advance past this node. A block scalar's `end()` underestimates (its text
    // is folded/chomped), so use the line span of its verbatim source instead;
    // this also covers the trailing blank lines that source includes, so the
    // next node's `blank_before` does not re-count them.
    match &node.comments.raw {
        Some(raw) => bump(prev_line, line + raw.matches('\n').count() as u32),
        None => bump(prev_line, node.end().0),
    }

    // Inline: a comment trailing this value on its own start line.
    if is_value && *cursor < comments.len() {
        let comment = &comments[*cursor];
        if comment.span.line == line && comment.span.column > column {
            // Preserve the gap between the value's end and the `#` so alignment
            // padding survives a re-emit. The value ends on this line for the
            // single-line scalars inline comments attach to; otherwise the gap is
            // left uncaptured (re-emits as one space).
            let (end_line, end_col) = node.end();
            if end_line == comment.span.line && comment.span.column > end_col {
                node.comments.inline_spaces = comment.span.column - end_col;
            }
            node.comments.inline = Some(comment.text.clone());
            *cursor += 1;
        }
    }
}

/// Maximum nesting depth for the round-trip composer, bounding recursion the
/// same way the fast-path decoder does.
const MAX_DEPTH: usize = 1000;

/// The byte offset at which each source line begins, for turning a byte offset
/// into an `(line, column)` pair. Line breaks are counted exactly as the scanner's
/// reader counts them: `\r\n` and `\r` are single breaks, and a leading BOM is not
/// content, so line 0 begins just after it (matching `column == 0` at the first
/// real character).
fn build_line_starts(input: &str) -> Vec<usize> {
    let first = if input.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut starts = vec![first];
    let bytes = input.as_bytes();
    let mut i = first;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                starts.push(i + 1);
                i += 1;
            }
            b'\r' => {
                // `\r\n` is one break; the next line starts past both bytes.
                if bytes.get(i + 1) == Some(&b'\n') {
                    starts.push(i + 2);
                    i += 2;
                } else {
                    starts.push(i + 1);
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    starts
}

struct Composer<'input> {
    /// The original source, so a block scalar's verbatim slice can be captured
    /// for round-trip replay.
    input: &'input str,
    /// Byte offset of the start of each source line, so a node's exact
    /// `end_offset` can be turned into an `(end_line, end_column)` pair. Built
    /// once from `input`; see [`build_line_starts`].
    line_starts: Vec<usize>,
    pos: usize,
    pending_tag: Option<String>,
    pending_anchor: Option<String>,
    pending_comments: Vec<String>,
    depth: usize,
}

impl<'input> Composer<'input> {
    fn new(input: &'input str) -> Self {
        Self {
            input,
            line_starts: build_line_starts(input),
            pos: 0,
            pending_tag: None,
            pending_anchor: None,
            pending_comments: Vec::new(),
            depth: 0,
        }
    }

    /// The exact 0-based `(line, column)` of the source position at byte `offset`,
    /// matching the scanner's line/column basis (a leading BOM and `\r`/`\r\n`/`\n`
    /// breaks are counted exactly as the reader counts them, and the column is a
    /// character count, not a byte count). `offset` must be a character boundary,
    /// which every node `end_offset` is.
    fn end_position(&self, offset: usize) -> (u32, u32) {
        // The line is the one whose start is the greatest offset not exceeding
        // `offset`; binary_search lands on it directly (exact hit = line start,
        // column 0) or just past it (take the preceding line).
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(next) => next.saturating_sub(1),
        };
        let line_start = self.line_starts[line].min(offset);
        let column = self.input[line_start..offset].chars().count();
        (line as u32, column as u32)
    }

    fn compose_stream(
        &mut self,
        events: &[Event],
        keep_empty: bool,
    ) -> Result<Vec<YamlNode>, ScanError> {
        let mut documents = Vec::new();
        // Directives (`%YAML`/`%TAG`) appear before the `---` of the document
        // they introduce; collect them until that document's node exists, then
        // hand them over so re-emission can replay them.
        let mut pending_directives: Vec<String> = Vec::new();

        while self.pos < events.len() {
            match &events[self.pos].kind {
                EventKind::StreamStart | EventKind::StreamEnd => {
                    self.pos += 1;
                }
                EventKind::DocumentStart => {
                    let marker_span = events[self.pos].span;
                    self.pos += 1;
                    if let Some(mut node) = self.compose_node(events)? {
                        // An explicit `---` produces this event; record it so the
                        // marker is preserved on re-emission.
                        node.explicit_start = true;
                        node.directives = std::mem::take(&mut pending_directives);
                        documents.push(node);
                    } else if keep_empty {
                        // A lone/trailing `---` with no body is still a document
                        // (a null one) when the caller counts documents.
                        let mut node = YamlNode::new(YamlNodeKind::Null, marker_span);
                        node.explicit_start = true;
                        node.directives = std::mem::take(&mut pending_directives);
                        documents.push(node);
                    } else {
                        // An empty document dropped here still consumed its own
                        // directives; clear them so a `%TAG` scoped to it cannot
                        // leak onto the next document.
                        pending_directives.clear();
                    }
                    if self.pos < events.len()
                        && matches!(events[self.pos].kind, EventKind::DocumentEnd)
                    {
                        self.pos += 1;
                    }
                }
                EventKind::Directive(text) => {
                    pending_directives.push(text.clone());
                    self.pos += 1;
                }
                _ => {
                    let start_pos = self.pos;
                    if let Some(mut node) = self.compose_node(events)? {
                        // The parser requires a `---` after any directive, so a
                        // document reaching this arm normally has none pending.
                        // Attach and mark explicit defensively: emitting the
                        // directives without their `---` would be invalid YAML.
                        node.directives = std::mem::take(&mut pending_directives);
                        if !node.directives.is_empty() {
                            node.explicit_start = true;
                        }
                        documents.push(node);
                    }
                    // Force progress past a bare terminator (e.g. a leading
                    // `...`) so the stream loop always terminates.
                    if self.pos <= start_pos {
                        self.pos += 1;
                    }
                }
            }
        }

        Ok(documents)
    }

    fn compose_node(&mut self, events: &[Event]) -> Result<Option<YamlNode>, ScanError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let span = events.get(self.pos).map(|e| e.span).unwrap_or_default();
            self.depth -= 1;
            return Err(ScanError::new(
                format!("maximum nesting depth ({MAX_DEPTH}) exceeded"),
                span,
            ));
        }
        // Grow the native stack if it is running low, so deeply nested input
        // (bounded above by `MAX_DEPTH`) cannot overflow a small thread stack
        // before the cap fires. See [`crate::stack`].
        let result = crate::stack::guard(|| self.compose_node_inner(events));
        self.depth -= 1;
        result
    }

    fn compose_node_inner(&mut self, events: &[Event]) -> Result<Option<YamlNode>, ScanError> {
        if self.pos >= events.len() {
            return Ok(None);
        }

        // Consume annotations
        loop {
            if self.pos >= events.len() {
                return Ok(None);
            }
            match &events[self.pos].kind {
                EventKind::Tag(tag) => {
                    self.pending_tag = Some(tag.clone());
                    self.pos += 1;
                }
                EventKind::Anchor(name) => {
                    self.pending_anchor = Some(name.clone());
                    self.pos += 1;
                }
                _ => break,
            }
        }

        if self.pos >= events.len() {
            return Ok(None);
        }

        let event = &events[self.pos];
        let span = event.span;
        let event_end = event.end_offset;
        let tag = self.pending_tag.take();
        let anchor = self.pending_anchor.take();
        let comments = Comments {
            head: std::mem::take(&mut self.pending_comments),
            ..Comments::default()
        };

        let mut node = match &event.kind {
            EventKind::Scalar(value, style) => {
                self.pos += 1;
                // The round-trip AST owns its scalar text (the document outlives
                // the borrowed input), so materialize the borrowed `Cow` here.
                YamlNode::new(YamlNodeKind::Scalar(value.to_string(), *style), span)
            }

            EventKind::MappingStart { flow, .. } => {
                self.pos += 1;
                let style = if *flow {
                    NodeStyle::Flow
                } else {
                    NodeStyle::Block
                };
                let pairs = self.compose_mapping(events, *flow)?;
                YamlNode::new(YamlNodeKind::Mapping(pairs), span).with_style(style)
            }

            EventKind::SequenceStart { flow, .. } => {
                self.pos += 1;
                let style = if *flow {
                    NodeStyle::Flow
                } else {
                    NodeStyle::Block
                };
                let items = self.compose_sequence(events, *flow)?;
                YamlNode::new(YamlNodeKind::Sequence(items), span).with_style(style)
            }

            EventKind::Alias(name) => {
                self.pos += 1;
                YamlNode::new(YamlNodeKind::Alias(name.clone()), span)
            }

            EventKind::SequenceEntry => {
                let items = self.compose_block_sequence(events)?;
                YamlNode::new(YamlNodeKind::Sequence(items), span)
            }

            // Terminators: return without consuming so the enclosing collection
            // loop can observe them. A bare `Key` means an empty value followed
            // by a sibling key (a nested mapping is always preceded by
            // `MappingStart`), so it is a null value, not a new mapping.
            EventKind::Key { .. }
            | EventKind::StreamEnd
            | EventKind::DocumentEnd
            | EventKind::DocumentStart
            | EventKind::BlockEnd
            | EventKind::MappingEnd
            | EventKind::SequenceEnd
            | EventKind::Value => {
                // A tag or anchor with no following node still yields a node, so
                // the property is not silently dropped: `key: !include` with no
                // argument becomes a tagged null the include resolver rejects,
                // and `key: &a` an anchored null. Without a pending property
                // there is genuinely no node, so the terminator is left for the
                // enclosing collection to observe.
                if tag.is_none() && anchor.is_none() {
                    return Ok(None);
                }
                YamlNode::new(YamlNodeKind::Null, span)
            }

            _ => {
                self.pos += 1;
                return self.compose_node(events);
            }
        };

        node.anchor = anchor;
        node.tag = tag;
        node.comments = comments;

        // Record the node's exact source end. A scalar event carries its own end;
        // a collection spans to the furthest end of any child (matching the
        // line/column basis of [`YamlNode::end`]); everything else is zero-width.
        node.end_offset = match &node.kind {
            YamlNodeKind::Scalar(..) => event_end,
            YamlNodeKind::Mapping(pairs) => pairs
                .iter()
                .flat_map(|(k, v)| [k.end_offset, v.end_offset])
                .max()
                .unwrap_or(span.offset),
            YamlNodeKind::Sequence(items) => items
                .iter()
                .map(|child| child.end_offset)
                .max()
                .unwrap_or(span.offset),
            _ => span.offset,
        };

        // Exact end line/column, derived from the exact end byte offset. Unlike
        // `YamlNode::end`, this is correct for quoted and escaped scalars (it
        // lands past the closing quote, not at the unescaped value's length).
        let (end_line, end_column) = self.end_position(node.end_offset);
        node.end_line = end_line;
        node.end_column = end_column;

        // A block scalar cannot be reconstructed (folding a `>` discards the
        // original line breaks), so keep its verbatim source for the emitter to
        // replay.
        if let YamlNodeKind::Scalar(_, ScalarStyle::Literal | ScalarStyle::Folded) = &node.kind {
            if event_end > span.offset {
                node.comments.raw = self.input.get(span.offset..event_end).map(str::to_owned);
            }
        }

        Ok(Some(node))
    }

    fn compose_mapping(
        &mut self,
        events: &[Event],
        flow: bool,
    ) -> Result<Vec<(YamlNode, YamlNode)>, ScanError> {
        let mut pairs = Vec::new();

        loop {
            if self.pos >= events.len() {
                break;
            }
            match &events[self.pos].kind {
                EventKind::MappingEnd | EventKind::BlockEnd => {
                    self.pos += 1;
                    break;
                }
                EventKind::FlowEntry => {
                    self.pos += 1;
                }
                EventKind::Key { explicit } => {
                    let explicit = *explicit;
                    self.pos += 1;
                    let mut key = self
                        .compose_node(events)?
                        .unwrap_or_else(|| YamlNode::new(YamlNodeKind::Null, Span::default()));
                    // Remember an author-written `?` so re-emission after an edit
                    // keeps the explicit-key form instead of collapsing it.
                    key.explicit_key = explicit;
                    if self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value)
                    {
                        self.pos += 1;
                    }
                    let val = self
                        .compose_node(events)?
                        .unwrap_or_else(|| YamlNode::new(YamlNodeKind::Null, Span::default()));
                    pairs.push((key, val));
                }
                EventKind::StreamEnd | EventKind::DocumentEnd | EventKind::DocumentStart => {
                    break;
                }
                _ => {
                    let start_pos = self.pos;
                    let key_span = events[self.pos].span;
                    // A bare plain scalar (no anchor/tag property) in key position
                    // with no `:` after it is a missing colon; a property-carrying
                    // key is structured differently and handled as before.
                    let bare_scalar_key = matches!(events[self.pos].kind, EventKind::Scalar(..));
                    // Reject a block collection sitting in mapping-key position
                    // (mis-indented content), using the same shared check as the
                    // fast decoder so both paths agree on what is valid.
                    if let Some(span) =
                        crate::parser::block_collection_key_span(events, self.pos, flow)
                    {
                        return Err(ScanError::new(
                            crate::parser::BLOCK_COLLECTION_KEY_MESSAGE,
                            span,
                        ));
                    }
                    let key = self
                        .compose_node(events)?
                        .unwrap_or_else(|| YamlNode::new(YamlNodeKind::Null, Span::default()));
                    let has_value = self.pos < events.len()
                        && matches!(events[self.pos].kind, EventKind::Value);
                    if has_value {
                        self.pos += 1;
                    } else if !flow && bare_scalar_key {
                        // A bare key in a block mapping must be followed by `:`;
                        // its absence is mis-indented content, rejected by the fast
                        // decoder and PyYAML alike.
                        return Err(ScanError::new(
                            crate::parser::MISSING_COLON_MESSAGE,
                            key_span,
                        ));
                    }
                    // A flow-mapping entry with a key but no `:` (`{a, b}`) has a
                    // null value; composing one here would wrongly absorb the next
                    // entry as this key's value.
                    let val = if has_value {
                        self.compose_node(events)?
                            .unwrap_or_else(|| YamlNode::new(YamlNodeKind::Null, Span::default()))
                    } else {
                        YamlNode::new(YamlNodeKind::Null, Span::default())
                    };
                    pairs.push((key, val));
                    // Guard against a stuck cursor (e.g. trailing terminators).
                    if self.pos <= start_pos {
                        break;
                    }
                }
            }
        }

        Ok(pairs)
    }

    fn compose_sequence(
        &mut self,
        events: &[Event],
        flow: bool,
    ) -> Result<Vec<YamlNode>, ScanError> {
        let mut items = Vec::new();

        loop {
            if self.pos >= events.len() {
                break;
            }
            match &events[self.pos].kind {
                EventKind::SequenceEnd | EventKind::BlockEnd => {
                    self.pos += 1;
                    break;
                }
                EventKind::SequenceEntry | EventKind::FlowEntry => {
                    let dash_span = events[self.pos].span;
                    // Only a block `-` entry can be empty-as-null; a flow `,`
                    // keeps the flow-pair-aware item path.
                    let block = matches!(events[self.pos].kind, EventKind::SequenceEntry);
                    self.pos += 1;
                    let item = if block {
                        self.compose_block_entry(events, dash_span)?
                    } else {
                        self.compose_sequence_item(events, flow)?
                    };
                    if let Some(mut node) = item {
                        mark_compact_nested_sequence(&mut node, dash_span.line);
                        if block {
                            mark_dash_pad(&mut node, dash_span);
                        }
                        items.push(node);
                    }
                }
                EventKind::MappingEnd => {
                    self.pos += 1;
                    break;
                }
                EventKind::StreamEnd | EventKind::DocumentEnd | EventKind::DocumentStart => {
                    break;
                }
                _ => {
                    let start_pos = self.pos;
                    if let Some(node) = self.compose_sequence_item(events, flow)? {
                        items.push(node);
                    } else {
                        break;
                    }
                    if self.pos <= start_pos {
                        break;
                    }
                }
            }
        }

        Ok(items)
    }

    /// Compose one sequence element, preserving the flow single-pair mapping form.
    /// In a flow sequence `a: b` is a one-entry mapping (`[a: b]` is `[{a: b}]`):
    /// a `Key` marker, or a `:` following a bare node, introduces the pair, which
    /// is wrapped in a single-element flow mapping so the structure survives the
    /// round-trip (mirrors the fast path's `decode_sequence_item`).
    fn compose_sequence_item(
        &mut self,
        events: &[Event],
        flow: bool,
    ) -> Result<Option<YamlNode>, ScanError> {
        let null = |span| YamlNode::new(YamlNodeKind::Null, span);

        // An explicit `Key` marker (`[a: b]`, `[? a : b]`) opens a single-pair
        // mapping element.
        if let Some(EventKind::Key { explicit }) =
            (self.pos < events.len() && flow).then(|| &events[self.pos].kind)
        {
            let explicit = *explicit;
            let pair_span = events[self.pos].span;
            self.pos += 1;
            let mut key = self
                .compose_node(events)?
                .unwrap_or_else(|| null(pair_span));
            key.explicit_key = explicit;
            if self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value) {
                self.pos += 1;
            }
            let val = self
                .compose_node(events)?
                .unwrap_or_else(|| null(pair_span));
            return Ok(Some(
                YamlNode::new(YamlNodeKind::Mapping(vec![(key, val)]), pair_span)
                    .with_style(NodeStyle::Flow),
            ));
        }

        // An implicit single-pair mapping with an empty key (`[: v]`): a `:` with
        // no preceding node opens the pair, so the key is null (mirrors the fast
        // path's empty-key handling in `decode_sequence_item`).
        if flow && self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value) {
            let pair_span = events[self.pos].span;
            self.pos += 1;
            let val = self
                .compose_node(events)?
                .unwrap_or_else(|| null(pair_span));
            return Ok(Some(
                YamlNode::new(
                    YamlNodeKind::Mapping(vec![(null(pair_span), val)]),
                    pair_span,
                )
                .with_style(NodeStyle::Flow),
            ));
        }

        let Some(key) = self.compose_node(events)? else {
            return Ok(None);
        };
        // An implicit single-pair mapping with no `Key` marker (`[&c c: d]`): a
        // `:` follows the composed node.
        if flow && self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value) {
            let pair_span = key.span;
            self.pos += 1;
            let val = self
                .compose_node(events)?
                .unwrap_or_else(|| null(pair_span));
            return Ok(Some(
                YamlNode::new(YamlNodeKind::Mapping(vec![(key, val)]), pair_span)
                    .with_style(NodeStyle::Flow),
            ));
        }
        Ok(Some(key))
    }

    /// Compose the value of a block sequence entry whose `-` was just consumed.
    /// An empty entry (the next event is a sibling `-`, a terminator, or a sibling
    /// mapping key) is a null node, mirroring the fast decoder's
    /// `decode_block_sequence`. Returning here keeps
    /// [`compose_node`](Self::compose_node) from absorbing the sibling `-` as a
    /// nested sequence, so `-\n-\n-` stays three nulls, not `[[[]]]`. `Key`
    /// covers the indentless case: an empty entry whose sibling mapping key sits
    /// at the sequence's own column dedents straight to a `Key` with no
    /// `BlockEnd` (the sequence shares the mapping's level), e.g. `9:\n-\nq:`. A
    /// non-empty `- key: val` entry opens with `MappingStart`, never a bare `Key`,
    /// so this never drops real content.
    fn compose_block_entry(
        &mut self,
        events: &[Event],
        dash_span: Span,
    ) -> Result<Option<YamlNode>, ScanError> {
        if self.pos < events.len()
            && matches!(
                events[self.pos].kind,
                EventKind::SequenceEntry
                    | EventKind::SequenceEnd
                    | EventKind::MappingEnd
                    | EventKind::BlockEnd
                    | EventKind::Key { .. }
            )
        {
            return Ok(Some(YamlNode::new(YamlNodeKind::Null, dash_span)));
        }
        self.compose_node(events)
    }

    fn compose_block_sequence(&mut self, events: &[Event]) -> Result<Vec<YamlNode>, ScanError> {
        let mut items = Vec::new();

        loop {
            if self.pos >= events.len() {
                break;
            }
            match &events[self.pos].kind {
                EventKind::SequenceEntry => {
                    let dash_span = events[self.pos].span;
                    self.pos += 1;
                    if let Some(mut node) = self.compose_block_entry(events, dash_span)? {
                        mark_compact_nested_sequence(&mut node, dash_span.line);
                        mark_dash_pad(&mut node, dash_span);
                        items.push(node);
                    }
                }
                EventKind::BlockEnd => {
                    // An implicit block sequence shares its parent mapping's
                    // indent level (see the fast decoder's `decode_block_sequence`
                    // for the full rationale), so the `BlockEnd` belongs to that
                    // mapping: break without consuming it.
                    break;
                }
                EventKind::MappingEnd => {
                    self.pos += 1;
                    break;
                }
                EventKind::StreamEnd | EventKind::DocumentEnd | EventKind::DocumentStart => {
                    break;
                }
                _ => break,
            }
        }

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::compose;
    use crate::roundtrip::ast::{NodeStyle, YamlNode, YamlNodeKind};
    use crate::scanner::ScalarStyle;

    fn root(src: &str) -> YamlNode {
        compose(src)
            .expect("valid YAML")
            .into_iter()
            .next()
            .expect("one document")
    }

    fn get<'a>(node: &'a YamlNode, key: &str) -> &'a YamlNode {
        let YamlNodeKind::Mapping(pairs) = &node.kind else {
            panic!("expected a mapping");
        };
        &pairs
            .iter()
            .find(|(k, _)| matches!(&k.kind, YamlNodeKind::Scalar(s, _) if s == key))
            .expect("key present")
            .1
    }

    fn style(node: &YamlNode) -> ScalarStyle {
        let YamlNodeKind::Scalar(_, st) = &node.kind else {
            panic!("expected a scalar");
        };
        *st
    }

    #[test]
    fn parses_nested_structure() {
        let r = root("a: 1\nlist:\n  - x\n  - y\n");
        assert!(matches!(r.kind, YamlNodeKind::Mapping(_)));
        assert!(matches!(get(&r, "list").kind, YamlNodeKind::Sequence(_)));
    }

    #[test]
    fn empty_indentless_entry_before_a_sibling_key_keeps_its_null() {
        // The composer shares the fast decoder's empty-entry rule: an empty
        // indentless `-` whose sibling mapping key dedents to a bare `Key` (no
        // `BlockEnd`) is a null entry, not a dropped one. Keeping it in the AST is
        // what lets an unmodified document re-emit byte-for-byte.
        let src = "9:\n-\nq:\n";
        let r = root(src);
        let YamlNodeKind::Sequence(items) = &get(&r, "9").kind else {
            panic!("expected a sequence under key 9");
        };
        assert_eq!(items.len(), 1, "the empty entry survives as one null");
        assert!(matches!(items[0].kind, YamlNodeKind::Null));
        let out = crate::roundtrip::emit::emit_roundtrip(&r);
        assert_eq!(out, src.as_bytes(), "re-emits byte-for-byte");
    }

    #[test]
    fn leading_bom_is_skipped_and_recorded() {
        // The BOM is not content: the first key must be `name`, not `\u{feff}name`.
        let r = root("\u{feff}name: value\n");
        assert!(r.leading_bom);
        assert!(matches!(&get(&r, "name").kind, YamlNodeKind::Scalar(s, _) if s == "value"));

        // And it round-trips byte-for-byte (the emitter restores the mark).
        let src = "\u{feff}name: value\n";
        let out = crate::roundtrip::emit::emit_roundtrip(&root(src));
        assert_eq!(out, src.as_bytes());

        // No BOM means no flag and no emitted mark.
        let plain = root("name: value\n");
        assert!(!plain.leading_bom);
    }

    #[test]
    fn preserves_scalar_styles() {
        let r = root("a: plain\nb: 'single'\nc: \"double\"\n");
        assert_eq!(style(get(&r, "a")), ScalarStyle::Plain);
        assert_eq!(style(get(&r, "b")), ScalarStyle::SingleQuoted);
        assert_eq!(style(get(&r, "c")), ScalarStyle::DoubleQuoted);
    }

    #[test]
    fn preserves_block_scalar_styles() {
        let r = root("lit: |\n  hello\nfold: >\n  world\n");
        assert_eq!(style(get(&r, "lit")), ScalarStyle::Literal);
        assert_eq!(style(get(&r, "fold")), ScalarStyle::Folded);
    }

    #[test]
    fn captures_inline_and_head_comments() {
        let r = root("# leading\nkey: value # trailing\n");
        let YamlNodeKind::Mapping(pairs) = &r.kind else {
            panic!("mapping");
        };
        let (k, v) = &pairs[0];
        // A leading comment before the first key is preserved in head position
        // (on the document root or the key itself, depending on attachment).
        let head_seen = r.comments.head.iter().chain(&k.comments.head);
        assert!(head_seen.into_iter().any(|c| c.contains("leading")));
        assert_eq!(v.comments.inline.as_deref(), Some("trailing"));
    }

    #[test]
    fn captures_anchor_and_alias() {
        let r = root("base: &a 1\nuse: *a\n");
        assert_eq!(get(&r, "base").anchor.as_deref(), Some("a"));
        assert!(matches!(&get(&r, "use").kind, YamlNodeKind::Alias(name) if name == "a"));
    }

    #[test]
    fn captures_tags() {
        let r = root("v: !mytag 5\n");
        assert_eq!(get(&r, "v").tag.as_deref(), Some("!mytag"));
    }

    #[test]
    fn records_explicit_document_start() {
        assert!(root("---\na: 1\n").explicit_start);
        assert!(!root("a: 1\n").explicit_start);
    }

    #[test]
    fn distinguishes_flow_from_block() {
        let r = root("flow: [1, 2]\nblock:\n  - 1\n");
        assert_eq!(get(&r, "flow").style, NodeStyle::Flow);
        assert_eq!(get(&r, "block").style, NodeStyle::Block);
    }

    #[test]
    fn tracks_source_spans() {
        // "port: 8080" is the second line (0-based line 1); the value 8080
        // starts at column 6 (0-based).
        let r = root("a: 1\nport: 8080\n");
        let port = get(&r, "port");
        assert_eq!(port.span.line, 1);
        assert_eq!(port.span.column, 6);
    }

    #[test]
    fn handles_multiple_documents() {
        assert_eq!(compose("---\na: 1\n---\nb: 2\n").unwrap().len(), 2);
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(compose("a: 'unterminated").is_err());
    }

    fn scalar_text<'a>(node: &'a YamlNode, key: &str) -> &'a str {
        match &get(node, key).kind {
            YamlNodeKind::Scalar(text, _) => text,
            _ => panic!("expected a scalar"),
        }
    }

    #[test]
    fn block_scalars_fold_and_chomp_correctly() {
        // Literal keeps newlines; folded joins lines with spaces. This pins the
        // scanner's block-scalar handling.
        let r = root("lit: |\n  line1\n  line2\nfold: >\n  a\n  b\n");
        assert_eq!(scalar_text(&r, "lit"), "line1\nline2\n");
        assert_eq!(scalar_text(&r, "fold"), "a b\n");
    }

    #[test]
    fn plain_scalar_folds_continuation_beginning_with_indicator() {
        // The first line is an ordinary plain scalar; its continuation line
        // begins with `|` (an indicator that is special only at a node's start),
        // which in block context is just a scalar character, so the plain scalar
        // keeps folding. Regression pin for the scanner's continuation rule.
        let r = root("note: hello\n  | round\n");
        assert_eq!(scalar_text(&r, "note"), "hello | round");
    }
}
