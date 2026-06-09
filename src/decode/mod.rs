use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

mod merge;

use crate::parser::{Event, EventKind, Parser};
use crate::resolver::{ScalarKind, Schema};
use crate::scanner::{ScalarStyle, Span};
use merge::{apply_merge_keys, is_merge_key, key_sig};

/// Decode YAML input into a nested Rust value tree.
///
/// This is the "fast path": events are converted directly to `Value` nodes
/// without building a full AST, which the Python FFI layer then converts to
/// Python objects. When `duplicate_keys_error` is set, a mapping containing the
/// same key twice is rejected with a [`DecodeError`] instead of silently
/// keeping the last value.
pub fn decode_with(
    input: &str,
    schema: Schema,
    duplicate_keys_error: bool,
    reject_complex_keys: bool,
) -> Result<Vec<Value<'_>>, DecodeError> {
    decode_collecting(
        input,
        schema,
        duplicate_keys_error,
        reject_complex_keys,
        WarnOptions::default(),
    )
    .map(|(documents, _)| documents)
}

/// Which non-fatal diagnostics the decoder should collect for the caller to log.
/// Each is off by default and has no effect on the returned values, only on the
/// accompanying warning list.
#[derive(Clone, Copy, Default)]
pub struct WarnOptions {
    /// Report each repeated mapping key (last value still wins).
    pub duplicate_keys: bool,
    /// Report each plain scalar whose type differs between YAML 1.1 and 1.2
    /// (the 1.1-only constructs a migration to 1.2 needs to find).
    pub yaml_1_1: bool,
}

/// Like [`decode_with`], but also returns the non-fatal diagnostics selected by
/// `warn` (duplicate keys, 1.1-only syntax), for the caller to emit through
/// Python logging. The warning list is empty unless a `warn` field is set.
pub fn decode_collecting(
    input: &str,
    schema: Schema,
    duplicate_keys_error: bool,
    reject_complex_keys: bool,
    warn: WarnOptions,
) -> Result<(Vec<Value<'_>>, Vec<String>), DecodeError> {
    let mut parser = Parser::new(input);
    let mut events = parser.parse_all().map_err(|e| DecodeError {
        kind: DecodeErrorKind::Parse,
        message: e.message,
        span: e.span,
    })?;

    // Move each scalar's text out of its event up front. The decoder then owns
    // these strings and can move them straight into `Value::String`, avoiding
    // the re-allocation a borrowing `resolve` would do for every string scalar.
    let scalars = take_scalar_strings(&mut events);

    let mut decoder = Decoder::new(schema, scalars);
    decoder.duplicate_keys_error = duplicate_keys_error;
    decoder.reject_complex_keys = reject_complex_keys;
    decoder.duplicate_keys_warn = warn.duplicate_keys;
    decoder.yaml_11_warn = warn.yaml_1_1;
    let mut documents = decoder.decode_stream(&events)?;
    for document in &mut documents {
        apply_merge_keys(document);
    }
    Ok((documents, decoder.warnings))
}

/// Whether a tag is an application-defined (custom) tag rather than one of the
/// YAML core-schema tags. The core tags are the secondary shorthand (`!!str`)
/// and the `tag:yaml.org,2002:` URIs; a local `!foo` tag or any other URI (such
/// as one a `%TAG` directive expands to) is application-defined.
pub fn is_custom_tag(tag: &str) -> bool {
    !(tag.starts_with("!!") || tag.starts_with("tag:yaml.org,2002:"))
}

/// Move the text out of every `Scalar` event into a position-indexed table, so
/// the decoder can hand ownership of each string to a `Value::String` without a
/// copy. The event still carries its style (the text field is left empty).
fn take_scalar_strings<'input>(events: &mut [Event<'input>]) -> Vec<Option<Cow<'input, str>>> {
    events
        .iter_mut()
        .map(|event| match &mut event.kind {
            EventKind::Scalar(text, _) => Some(std::mem::take(text)),
            _ => None,
        })
        .collect()
}

/// Count the total number of nodes in a value tree (used to bound alias
/// expansion). The tree is bounded by `MAX_DEPTH`, but a thread with a small
/// stack could still overflow at that depth, so each level grows the native
/// stack on demand (see [`crate::stack`]).
fn count_nodes(value: &Value<'_>) -> usize {
    crate::stack::guard(|| {
        1 + match value {
            Value::Sequence(items) => items.iter().map(count_nodes).sum(),
            Value::Mapping(pairs) => pairs
                .iter()
                .map(|(k, v)| count_nodes(k) + count_nodes(v))
                .sum(),
            Value::Tagged(_, inner) => count_nodes(inner),
            _ => 0,
        }
    })
}

/// A decoded YAML value.
///
/// String scalars hold a [`Cow`] that borrows directly from the input on the
/// fast path (single-line plain scalars) and is owned only when the scanner had
/// to transform the text. Values built from Python objects (the `dumps` path)
/// are always owned, i.e. `Value<'static>`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value<'input> {
    Null,
    Bool(bool),
    Int(i64),
    /// An integer too large to fit in `i64`, carried as its exact decimal text.
    /// Python and YAML integers are both arbitrary precision.
    BigInt(Cow<'input, str>),
    Float(f64),
    String(Cow<'input, str>),
    Sequence(Vec<Value<'input>>),
    Mapping(Vec<(Value<'input>, Value<'input>)>),
    Tagged(String, Box<Value<'input>>),
}

/// What kind of decode failure occurred, so the FFI layer can raise a precise
/// Python exception (a duplicate key versus a generic syntax error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DecodeErrorKind {
    /// Malformed YAML: the catch-all for scanner/parser/resolver failures.
    #[default]
    Parse,
    /// A duplicate mapping key while `OPT_DUPLICATE_KEYS_ERROR` is set.
    DuplicateKey,
    /// A collection (mapping or sequence) used as a mapping key while
    /// `OPT_REJECT_COMPLEX_KEYS` is set. Such a key is valid YAML and is
    /// converted to a hashable Python value by default; this kind is produced
    /// only when the caller opts into rejecting it instead.
    ComplexKey,
}

/// Error during decoding.
#[derive(Debug, Clone)]
pub struct DecodeError {
    pub kind: DecodeErrorKind,
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} at line {}, column {}",
            self.message,
            self.span.line + 1,
            self.span.column + 1
        )
    }
}

impl std::error::Error for DecodeError {}

/// Maximum nesting depth, to bound recursion and prevent a stack overflow from
/// pathological input such as `[[[[...`.
const MAX_DEPTH: usize = 1000;

/// Maximum number of value nodes a single document may expand to. Bounds alias
/// expansion ("billion laughs") so untrusted input cannot exhaust memory. Ten
/// million nodes is far more than any real document yet caps memory at a few
/// hundred megabytes.
const MAX_NODES: usize = 10_000_000;

/// Validate a directive's text, returning whether it is a `%YAML` directive.
///
/// `%YAML` must name exactly one `major.minor` version and nothing more. `%TAG`
/// and reserved (unknown) directives are accepted, since the spec says an unknown
/// directive is ignored with a warning, not an error.
fn validate_directive(text: &str, span: Span) -> Result<bool, DecodeError> {
    // A directive line may end with a comment (a whitespace-separated token
    // starting with `#`); it is not part of the directive itself.
    let mut parts = text.split_whitespace().take_while(|p| !p.starts_with('#'));
    if parts.next() != Some("YAML") {
        return Ok(false);
    }
    let valid_version = parts.next().is_some_and(|v| {
        let mut segments = v.split('.');
        let major = segments.next();
        let minor = segments.next();
        let is_num = |s: Option<&str>| {
            s.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        };
        is_num(major) && is_num(minor) && segments.next().is_none()
    });
    // No version, a malformed one, or trailing content is all invalid.
    if !valid_version || parts.next().is_some() {
        return Err(DecodeError {
            kind: DecodeErrorKind::Parse,
            message: "invalid %YAML directive".to_owned(),
            span,
        });
    }
    Ok(true)
}

/// Extract the (handle, prefix) a `%TAG` directive defines, if the text is a
/// well-formed `%TAG` directive. Returns `None` for any other directive.
fn tag_directive_handle(text: &str) -> Option<(String, String)> {
    let mut parts = text.split_whitespace();
    if parts.next() != Some("TAG") {
        return None;
    }
    let handle = parts.next()?;
    let prefix = parts.next()?;
    // A tag handle is `!`, `!!`, or `!name!`: it starts and ends with `!`.
    if handle.starts_with('!') && handle.ends_with('!') {
        Some((handle.to_owned(), prefix.to_owned()))
    } else {
        None
    }
}

/// The named handle a node tag uses (`!name!suffix` -> `!name!`), if any. The
/// default primary (`!`) and secondary (`!!`) handles and verbatim tags
/// (`!<...>`) are always resolvable and return `None`.
fn named_tag_handle(tag: &str) -> Option<&str> {
    let rest = tag.strip_prefix('!')?;
    // `!!...` is the secondary handle; `!<...>` is verbatim; neither is a named
    // handle needing a `%TAG` definition.
    if rest.starts_with('!') || rest.starts_with('<') {
        return None;
    }
    // A named handle has a closing `!` before the suffix (`!name!suffix`).
    let end = rest.find('!')?;
    Some(&tag[..end + 2])
}

struct Decoder<'input> {
    /// The scalar-resolution schema (1.2, 1.1, or 1.1-PyYAML).
    schema: Schema,
    /// Scalar texts moved out of the events, indexed by event position.
    scalars: Vec<Option<Cow<'input, str>>>,
    // Each anchor stores its resolved value and its precomputed node count, so
    // an alias is an O(1) budget check instead of re-walking the subtree on
    // every reference (which also speeds up rejecting an alias bomb).
    anchors: HashMap<String, (Value<'input>, usize)>,
    /// Reject a mapping that repeats a key, rather than keeping the last value.
    duplicate_keys_error: bool,
    /// Reject a collection (mapping or sequence) used as a mapping key, rather
    /// than converting it to a hashable Python value (`OPT_REJECT_COMPLEX_KEYS`).
    reject_complex_keys: bool,
    /// Collect a non-fatal diagnostic for each repeated key (last value still
    /// wins), for the caller to log. Mutually meaningful only when
    /// `duplicate_keys_error` is off.
    duplicate_keys_warn: bool,
    /// Collect a diagnostic for each plain scalar whose 1.1 and 1.2 types differ
    /// (1.1-only syntax), for a migration to surface. Already gated on 1.1 mode
    /// by the caller.
    yaml_11_warn: bool,
    /// Non-fatal diagnostics gathered during decode (duplicate keys, 1.1-only
    /// syntax), surfaced to the caller to emit through Python logging.
    warnings: Vec<String>,
    /// Tag handles (`!`, `!!`, `!name!`) to their prefixes, as defined by `%TAG`
    /// directives for the document currently being decoded. Reset at each
    /// document boundary, since a directive applies only to the document it
    /// introduces. Used both to expand shorthand tags and to reject undefined
    /// named handles.
    tag_handles: HashMap<String, String>,
    pos: usize,
    depth: usize,
    nodes: usize,
}

impl<'input> Decoder<'input> {
    fn new(schema: Schema, scalars: Vec<Option<Cow<'input, str>>>) -> Self {
        Self {
            schema,
            scalars,
            anchors: HashMap::new(),
            duplicate_keys_error: false,
            reject_complex_keys: false,
            duplicate_keys_warn: false,
            yaml_11_warn: false,
            warnings: Vec::new(),
            tag_handles: HashMap::new(),
            pos: 0,
            depth: 0,
            nodes: 0,
        }
    }

    /// If duplicate-key checking is enabled, error when `key` was already seen in
    /// this mapping. `seen` holds an injective signature of every prior key, so
    /// membership is O(1) (a linear scan made a large mapping quadratic). The
    /// merge key `<<` is exempt: repeating it is how multiple mappings are merged.
    fn check_duplicate_key(
        &mut self,
        seen: &mut HashSet<String>,
        key: &Value<'input>,
        span: Span,
    ) -> Result<(), DecodeError> {
        // The merge key `<<` is exempt: repeating it is how multiple mappings
        // merge. With neither reporting mode on, skip the signature work entirely.
        if is_merge_key(key) || !(self.duplicate_keys_error || self.duplicate_keys_warn) {
            return Ok(());
        }
        // `insert` returns true when the key is new; a false means a duplicate.
        if seen.insert(key_sig(key)) {
            return Ok(());
        }
        let name = match key {
            Value::String(s) => s.as_ref().to_owned(),
            other => format!("{other:?}"),
        };
        if self.duplicate_keys_error {
            return Err(DecodeError {
                kind: DecodeErrorKind::DuplicateKey,
                message: format!("duplicate mapping key: {name}"),
                span,
            });
        }
        // Warn mode: keep last-wins but record a diagnostic for the caller to log.
        self.warnings.push(format!(
            "duplicate mapping key '{name}' at line {}, column {}; keeping the last value",
            span.line + 1,
            span.column + 1
        ));
        Ok(())
    }

    /// When `OPT_REJECT_COMPLEX_KEYS` is set, reject a collection (mapping or
    /// sequence) used as a mapping key with a located [`DecodeErrorKind::ComplexKey`]
    /// error, instead of letting the FFI layer convert it to a hashable Python
    /// value. A complex key is valid YAML (so this is off by default), but a
    /// consumer whose data model is scalar-keyed (such as a config loader) can opt
    /// in to catch it early, with a precise location, and render its own message.
    /// The most common trigger is an unquoted whole-value template like
    /// `key: {{ x }}`, which YAML reads as a mapping-with-a-mapping-key.
    fn reject_complex_key(&self, key: &Value<'input>, span: Span) -> Result<(), DecodeError> {
        if !self.reject_complex_keys {
            return Ok(());
        }
        let kind = match key {
            Value::Mapping(_) => "a mapping",
            Value::Sequence(_) => "a sequence",
            _ => return Ok(()),
        };
        Err(DecodeError {
            kind: DecodeErrorKind::ComplexKey,
            message: format!("complex mapping key ({kind} used as a key)"),
            span,
        })
    }

    /// Take ownership of the scalar text recorded for the event at `pos`.
    fn take_scalar(&mut self, pos: usize) -> Cow<'input, str> {
        self.scalars
            .get_mut(pos)
            .and_then(Option::take)
            .unwrap_or_default()
    }

    /// Account for `count` newly created nodes, erroring if the budget is
    /// exceeded.
    fn charge_nodes(&mut self, count: usize, span: Span) -> Result<(), DecodeError> {
        self.nodes = self.nodes.saturating_add(count);
        if self.nodes > MAX_NODES {
            return Err(DecodeError {
                kind: DecodeErrorKind::Parse,
                message: "document expands to too many nodes (possible alias bomb)".to_owned(),
                span,
            });
        }
        Ok(())
    }

    fn decode_stream(
        &mut self,
        events: &[Event<'input>],
    ) -> Result<Vec<Value<'input>>, DecodeError> {
        let mut documents = Vec::new();
        // A directive is only meaningful at the start of the stream or right
        // after a `...` document-end. A `%` that the scanner tokenizes inside
        // document content (e.g. a plain/block scalar line beginning with `%`)
        // appears here while this is false and is left untouched, not validated.
        let mut directives_allowed = true;

        while self.pos < events.len() {
            let event = &events[self.pos];
            match &event.kind {
                EventKind::StreamStart => {
                    self.pos += 1;
                }
                EventKind::StreamEnd => {
                    self.pos += 1;
                }
                EventKind::DocumentEnd => {
                    self.pos += 1;
                    directives_allowed = true;
                }
                EventKind::DocumentStart => {
                    self.pos += 1;
                    // An explicit `---` always introduces a document, even an
                    // empty one: a lone or trailing `---` yields a null document
                    // (matching PyYAML's `safe_load_all`), so document count is a
                    // reliable signal (e.g. distinguishing a config from
                    // frontmatter-plus-empty-config).
                    let value = self.decode_node(events)?.unwrap_or(Value::Null);
                    documents.push(value);
                    self.expect_document_boundary(events)?;
                    directives_allowed = false;
                    // A `%TAG` handle goes out of scope at the document's end.
                    self.tag_handles.clear();
                    // Anchors do not span documents: an `&a` defined in one
                    // document must not be resolvable by an `*a` in the next.
                    // Clearing here turns such a cross-document reference into the
                    // "unknown alias" error it should be (matching the spec).
                    self.anchors.clear();
                    // Skip DocumentEnd if present; it re-permits directives.
                    if self.pos < events.len()
                        && matches!(events[self.pos].kind, EventKind::DocumentEnd)
                    {
                        self.pos += 1;
                        directives_allowed = true;
                    }
                }
                EventKind::Directive(_) if directives_allowed => {
                    let run_span = event.span;
                    let mut seen_yaml = false;
                    while self.pos < events.len() {
                        let EventKind::Directive(text) = &events[self.pos].kind else {
                            break;
                        };
                        let span = events[self.pos].span;
                        if validate_directive(text, span)? {
                            if seen_yaml {
                                return Err(DecodeError {
                                    kind: DecodeErrorKind::Parse,
                                    message: "duplicate %YAML directive".to_owned(),
                                    span,
                                });
                            }
                            seen_yaml = true;
                        } else if let Some((handle, prefix)) = tag_directive_handle(text) {
                            // A `%TAG` handle is in scope only for the document
                            // this run introduces, and may be declared at most
                            // once per document (like `%YAML`); a repeat is an
                            // error, not a silent last-wins override.
                            if self.tag_handles.contains_key(&handle) {
                                return Err(DecodeError {
                                    kind: DecodeErrorKind::Parse,
                                    message: format!(
                                        "duplicate %TAG directive for handle '{handle}'"
                                    ),
                                    span,
                                });
                            }
                            self.tag_handles.insert(handle, prefix);
                        }
                        self.pos += 1;
                    }
                    // A directive applies to the document it introduces, which
                    // must begin with an explicit `---` marker.
                    if self.pos >= events.len()
                        || !matches!(events[self.pos].kind, EventKind::DocumentStart)
                    {
                        return Err(DecodeError {
                            kind: DecodeErrorKind::Parse,
                            message: "directive must be followed by a document start (---)"
                                .to_owned(),
                            span: run_span,
                        });
                    }
                }
                EventKind::Directive(_) => {
                    // A directive reached here means `directives_allowed` is
                    // false: it follows document content with no intervening
                    // `...` document-end marker, which is invalid.
                    return Err(DecodeError {
                        kind: DecodeErrorKind::Parse,
                        message: "a directive must be at the start of the stream or follow a \
                                  document-end marker (...)"
                            .to_owned(),
                        span: event.span,
                    });
                }
                _ => {
                    // Implicit document.
                    let start_pos = self.pos;
                    if let Some(value) = self.decode_node(events)? {
                        documents.push(value);
                        self.expect_document_boundary(events)?;
                    }
                    directives_allowed = false;
                    self.tag_handles.clear();
                    // `decode_node` returns `None` without consuming on a bare
                    // terminator (e.g. a leading `...`); force progress so the
                    // stream loop always terminates.
                    if self.pos <= start_pos {
                        self.pos += 1;
                    }
                }
            }
        }

        Ok(documents)
    }

    /// After a document's root node, only the end of the stream or a document
    /// marker (`---`, `...`, or a directive) may follow. Anything else is a
    /// second root node with no separator (trailing content), which is an error.
    fn expect_document_boundary(&self, events: &[Event<'input>]) -> Result<(), DecodeError> {
        let Some(event) = events.get(self.pos) else {
            return Ok(());
        };
        match &event.kind {
            EventKind::StreamEnd
            | EventKind::DocumentStart
            | EventKind::DocumentEnd
            | EventKind::Directive(_) => Ok(()),
            _ => Err(DecodeError {
                kind: DecodeErrorKind::Parse,
                message: "trailing content after document".to_owned(),
                span: event.span,
            }),
        }
    }

    /// Decode a node, enforcing the recursion-depth limit around the actual
    /// work in [`decode_node_inner`].
    fn decode_node(
        &mut self,
        events: &[Event<'input>],
    ) -> Result<Option<Value<'input>>, DecodeError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            let span = events.get(self.pos).map(|e| e.span).unwrap_or_default();
            self.depth -= 1;
            return Err(DecodeError {
                kind: DecodeErrorKind::Parse,
                message: format!("maximum nesting depth ({MAX_DEPTH}) exceeded"),
                span,
            });
        }
        // Grow the native stack if it is running low, so deeply nested input
        // (bounded above by `MAX_DEPTH`) cannot overflow a small thread stack
        // before the cap fires. See [`crate::stack`].
        let result = crate::stack::guard(|| self.decode_node_inner(events));
        self.depth -= 1;
        result
    }

    fn decode_node_inner(
        &mut self,
        events: &[Event<'input>],
    ) -> Result<Option<Value<'input>>, DecodeError> {
        if self.pos >= events.len() {
            return Ok(None);
        }

        let mut current_tag: Option<String> = None;
        let mut current_anchor: Option<String> = None;

        // Consume tag and anchor events
        loop {
            if self.pos >= events.len() {
                return Ok(None);
            }
            match &events[self.pos].kind {
                EventKind::Tag(tag) => {
                    // A named tag handle must be defined by a `%TAG` directive
                    // in this document (`!prefix!A` with no `%TAG !prefix!`).
                    if let Some(handle) = named_tag_handle(tag) {
                        if !self.tag_handles.contains_key(handle) {
                            return Err(DecodeError {
                                kind: DecodeErrorKind::Parse,
                                message: format!("undefined tag handle: {handle}"),
                                span: events[self.pos].span,
                            });
                        }
                    }
                    current_tag = Some(self.expand_tag(tag));
                    self.pos += 1;
                }
                EventKind::Anchor(name) => {
                    // A node can carry only one anchor; two consumed here (with
                    // no intervening node) is invalid (`&a\n&b value`).
                    if current_anchor.is_some() {
                        return Err(DecodeError {
                            kind: DecodeErrorKind::Parse,
                            message: "a node cannot have two anchors".to_owned(),
                            span: events[self.pos].span,
                        });
                    }
                    current_anchor = Some(name.clone());
                    self.pos += 1;
                }
                _ => break,
            }
        }

        if self.pos >= events.len() {
            return Ok(None);
        }

        // A tag whose node turns out to be empty (`foo: !!str,`, `!!str : bar`)
        // applies to an empty scalar: `!!str` yields "", others null. The
        // terminator that follows is left for the enclosing collection.
        if let Some(tag) = &current_tag {
            let is_node = matches!(
                events[self.pos].kind,
                EventKind::Scalar(..)
                    | EventKind::MappingStart { .. }
                    | EventKind::SequenceStart { .. }
                    | EventKind::SequenceEntry
                    | EventKind::Alias(..)
            );
            if !is_node {
                let resolved = match self.schema.classify("", ScalarStyle::Plain, Some(tag)) {
                    ScalarKind::Null => Value::Null,
                    ScalarKind::Bool(b) => Value::Bool(b),
                    ScalarKind::Int(i) => Value::Int(i),
                    // Unreachable for the empty value, but kept for exhaustiveness.
                    ScalarKind::BigInt => Value::String(Cow::Borrowed("")),
                    ScalarKind::Float(f) => Value::Float(f),
                    ScalarKind::Str => Value::String(Cow::Borrowed("")),
                    ScalarKind::Merge => merge::merge_key_marker(),
                };
                let value = if is_custom_tag(tag) {
                    Value::Tagged(tag.clone(), Box::new(resolved))
                } else {
                    resolved
                };
                if let Some(anchor) = current_anchor {
                    self.anchors
                        .insert(anchor, (value.clone(), count_nodes(&value)));
                }
                return Ok(Some(value));
            }
        }

        let event = &events[self.pos];
        let value = match &event.kind {
            EventKind::Scalar(_, style) => {
                let style = *style;
                let pos = self.pos;
                self.pos += 1;
                // Take ownership of the scanned text; classify the type without
                // re-allocating, then move the text straight into the string
                // case instead of cloning it.
                let text = self.take_scalar(pos);
                if self.yaml_11_warn {
                    if let Some((t11, t12)) = crate::resolver::yaml_11_divergence(
                        self.schema,
                        &text,
                        style,
                        current_tag.as_deref(),
                    ) {
                        let span = events[pos].span;
                        self.warnings.push(format!(
                            "YAML 1.1 syntax '{text}' resolves as {t11} in 1.1 but {t12} in 1.2 \
                             at line {}, column {}",
                            span.line + 1,
                            span.column + 1
                        ));
                    }
                }
                let value = match self.schema.classify(&text, style, current_tag.as_deref()) {
                    ScalarKind::Null => Value::Null,
                    ScalarKind::Bool(b) => Value::Bool(b),
                    ScalarKind::Int(i) => Value::Int(i),
                    ScalarKind::BigInt => Value::BigInt(text),
                    ScalarKind::Float(f) => Value::Float(f),
                    ScalarKind::Str => Value::String(text),
                    ScalarKind::Merge => merge::merge_key_marker(),
                };
                // Preserve custom (application) tags so the FFI layer can apply
                // a tag handler or pass them through as `YAMLRocksTag` objects.
                match &current_tag {
                    Some(tag) if is_custom_tag(tag) => Value::Tagged(tag.clone(), Box::new(value)),
                    _ => value,
                }
            }

            EventKind::MappingStart { flow } => {
                let flow = *flow;
                self.pos += 1;
                let mapping = self.decode_mapping(events, flow)?;
                if let Some(ref tag) = current_tag {
                    if is_custom_tag(tag) {
                        Value::Tagged(tag.clone(), Box::new(Value::Mapping(mapping)))
                    } else {
                        Value::Mapping(mapping)
                    }
                } else {
                    Value::Mapping(mapping)
                }
            }

            EventKind::SequenceStart { flow } => {
                let flow = *flow;
                self.pos += 1;
                let sequence = self.decode_sequence(events, flow)?;
                if let Some(ref tag) = current_tag {
                    if is_custom_tag(tag) {
                        Value::Tagged(tag.clone(), Box::new(Value::Sequence(sequence)))
                    } else {
                        Value::Sequence(sequence)
                    }
                } else {
                    Value::Sequence(sequence)
                }
            }

            EventKind::Alias(name) => {
                let span = event.span;
                // An alias is a bare reference; it cannot carry its own anchor
                // or tag (`&b *a` / `!t *a` are invalid).
                if current_anchor.is_some() || current_tag.is_some() {
                    return Err(DecodeError {
                        kind: DecodeErrorKind::Parse,
                        message: "an alias node cannot have an anchor or tag".to_owned(),
                        span,
                    });
                }
                self.pos += 1;
                // Measure the expansion *before* cloning, so a "billion laughs"
                // bomb is rejected without first allocating the oversized copy.
                let size = match self.anchors.get(name) {
                    Some((_, size)) => *size,
                    None => {
                        return Err(DecodeError {
                            kind: DecodeErrorKind::Parse,
                            message: format!("unknown alias: *{name}"),
                            span,
                        });
                    }
                };
                self.charge_nodes(size, span)?;
                self.anchors.get(name).expect("anchor present").0.clone()
            }

            EventKind::SequenceEntry => {
                // A block sequence value may sit at the same indent as its key
                // (no preceding SequenceStart), so build it here.
                let sequence = self.decode_block_sequence(events)?;
                Value::Sequence(sequence)
            }

            // A bare `Key` here means an empty value followed by a sibling key
            // at the same indent (a nested mapping is always preceded by
            // `MappingStart`). The value is null; do not consume the key. The
            // collection-end markers are likewise not nodes, returning without
            // consuming lets the enclosing collection see its own terminator
            // rather than silently swallowing it.
            EventKind::Key
            | EventKind::StreamEnd
            | EventKind::DocumentEnd
            | EventKind::DocumentStart
            | EventKind::SequenceEnd
            | EventKind::MappingEnd
            | EventKind::BlockEnd => {
                // An anchor on an empty node (`a: &x` then `b: *x`) still binds:
                // the empty node is null, so register it before returning.
                if let Some(anchor) = current_anchor {
                    self.anchors.insert(anchor, (Value::Null, 1));
                }
                return Ok(None);
            }

            _ => {
                self.pos += 1;
                return self.decode_node(events);
            }
        };

        if let Some(anchor) = current_anchor {
            self.anchors
                .insert(anchor, (value.clone(), count_nodes(&value)));
        }

        Ok(Some(value))
    }

    /// Expand a shorthand tag against this document's `%TAG` handles. The
    /// secondary handle `!!` defaults to `tag:yaml.org,2002:` unless redefined,
    /// so `!!int` becomes the core int tag normally but an application tag when
    /// a `%TAG !! ...` directive is in scope. A verbatim `!<...>` tag, and a
    /// primary or local `!foo` tag that no directive redefines, are returned
    /// unchanged.
    fn expand_tag(&self, tag: &str) -> String {
        // Verbatim tags are already resolved.
        if tag.starts_with("!<") {
            return tag.to_owned();
        }
        if let Some(suffix) = tag.strip_prefix("!!") {
            return match self.tag_handles.get("!!") {
                Some(prefix) => format!("{prefix}{suffix}"),
                None => format!("tag:yaml.org,2002:{suffix}"),
            };
        }
        if let Some(handle) = named_tag_handle(tag) {
            if let Some(prefix) = self.tag_handles.get(handle) {
                let suffix = &tag[handle.len()..];
                return format!("{prefix}{suffix}");
            }
        }
        // A primary `!foo` tag expands only if `!` was redefined.
        if let Some(suffix) = tag.strip_prefix('!') {
            if let Some(prefix) = self.tag_handles.get("!") {
                return format!("{prefix}{suffix}");
            }
        }
        tag.to_owned()
    }

    /// Reject a block collection sitting in mapping-key position, delegating the
    /// detection to the shared [`block_collection_key_span`] so the fast path and
    /// the round-trip composer reject identically.
    fn reject_block_collection_key(
        &self,
        events: &[Event<'input>],
        flow: bool,
    ) -> Result<(), DecodeError> {
        if let Some(span) = crate::parser::block_collection_key_span(events, self.pos, flow) {
            return Err(DecodeError {
                kind: DecodeErrorKind::Parse,
                message: crate::parser::BLOCK_COLLECTION_KEY_MESSAGE.to_owned(),
                span,
            });
        }
        Ok(())
    }

    fn decode_mapping(
        &mut self,
        events: &[Event<'input>],
        flow: bool,
    ) -> Result<Vec<(Value<'input>, Value<'input>)>, DecodeError> {
        let mut pairs = Vec::new();
        // Signatures of the keys seen so far, for O(1) duplicate detection. Stays
        // empty (and unallocated) unless duplicate-key checking is enabled.
        let mut seen: HashSet<String> = HashSet::new();

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
                EventKind::Key => {
                    let key_span = events[self.pos].span;
                    self.pos += 1;
                    // An explicit `?` key (which reaches here with a `Key`
                    // marker) may be a block collection; only a bare key in the
                    // arm below, produced by mis-indented content, may not.
                    let key = self.decode_node(events)?.unwrap_or(Value::Null);
                    if self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value)
                    {
                        self.pos += 1;
                    }
                    let val = self.decode_node(events)?.unwrap_or(Value::Null);
                    self.reject_complex_key(&key, key_span)?;
                    self.check_duplicate_key(&mut seen, &key, key_span)?;
                    pairs.push((key, val));
                }
                EventKind::StreamEnd | EventKind::DocumentEnd | EventKind::DocumentStart => {
                    break;
                }
                _ => {
                    let start_pos = self.pos;
                    let key_span = events[self.pos].span;
                    // A bare plain scalar (no anchor/tag property) appearing where
                    // a mapping key is expected, in a block mapping, with no `:`
                    // following, is a missing colon. Keys carrying a property are
                    // structured differently by the scanner and handled as before.
                    let bare_scalar_key = matches!(events[self.pos].kind, EventKind::Scalar(..));
                    self.reject_block_collection_key(events, flow)?;
                    let key = self.decode_node(events)?.unwrap_or(Value::Null);
                    let has_value = self.pos < events.len()
                        && matches!(events[self.pos].kind, EventKind::Value);
                    if has_value {
                        self.pos += 1;
                    } else if !flow && bare_scalar_key {
                        return Err(DecodeError {
                            kind: DecodeErrorKind::Parse,
                            message: crate::parser::MISSING_COLON_MESSAGE.to_owned(),
                            span: key_span,
                        });
                    }
                    // A flow-mapping entry with a key but no `:` (`{a, b}`) has a
                    // null value. Only decode a value node when a `:` introduced
                    // one; otherwise decoding here would wrongly consume the next
                    // entry as this key's value.
                    let val = if has_value {
                        self.decode_node(events)?.unwrap_or(Value::Null)
                    } else {
                        Value::Null
                    };
                    self.reject_complex_key(&key, key_span)?;
                    self.check_duplicate_key(&mut seen, &key, key_span)?;
                    pairs.push((key, val));
                    if self.pos <= start_pos {
                        break;
                    }
                }
            }
        }

        Ok(pairs)
    }

    fn decode_sequence(
        &mut self,
        events: &[Event<'input>],
        flow: bool,
    ) -> Result<Vec<Value<'input>>, DecodeError> {
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
                    self.pos += 1;
                    if let Some(value) = self.decode_sequence_item(events, flow)? {
                        items.push(value);
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
                    // A bare node with no entry marker: the first element of a
                    // flow sequence, or (for block sequences) content the
                    // scanner attached here (multiline scalars, tab separation).
                    let start_pos = self.pos;
                    if let Some(value) = self.decode_sequence_item(events, flow)? {
                        items.push(value);
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

    /// Decode one sequence element. In a flow sequence, `key: value` is a
    /// single-pair mapping (`[a: b]` is `[{a: b}]`), so a `:` following the node
    /// is consumed and the pair wrapped in a mapping.
    fn decode_sequence_item(
        &mut self,
        events: &[Event<'input>],
        flow: bool,
    ) -> Result<Option<Value<'input>>, DecodeError> {
        // An empty block entry (the next event is a sibling `-` or the
        // sequence's end) is null. Returning here keeps `decode_node` from
        // absorbing the sibling `-` as a nested sequence (`- # empty\n- b`).
        if !flow
            && self.pos < events.len()
            && matches!(
                events[self.pos].kind,
                EventKind::SequenceEntry
                    | EventKind::SequenceEnd
                    | EventKind::MappingEnd
                    | EventKind::BlockEnd
            )
        {
            return Ok(Some(Value::Null));
        }
        // A `Key` marker (the scanner's detection of `[a: b]` or an explicit
        // `[? a : b]`) introduces a single-pair mapping element.
        if flow && self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Key) {
            let key_span = events[self.pos].span;
            self.pos += 1;
            let key = self.decode_node(events)?.unwrap_or(Value::Null);
            if self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value) {
                self.pos += 1;
            }
            let val = self.decode_node(events)?.unwrap_or(Value::Null);
            self.reject_complex_key(&key, key_span)?;
            return Ok(Some(Value::Mapping(vec![(key, val)])));
        }
        let key_span = events.get(self.pos).map(|e| e.span).unwrap_or_default();
        let Some(key) = self.decode_node(events)? else {
            return Ok(None);
        };
        // Implicit single-pair mapping with no `Key` marker (e.g. an anchored
        // key `[&c c: d]`): a `:` follows the node.
        if flow && self.pos < events.len() && matches!(events[self.pos].kind, EventKind::Value) {
            self.pos += 1;
            let val = self.decode_node(events)?.unwrap_or(Value::Null);
            self.reject_complex_key(&key, key_span)?;
            return Ok(Some(Value::Mapping(vec![(key, val)])));
        }
        Ok(Some(key))
    }

    fn decode_block_sequence(
        &mut self,
        events: &[Event<'input>],
    ) -> Result<Vec<Value<'input>>, DecodeError> {
        let mut items = Vec::new();

        loop {
            if self.pos >= events.len() {
                break;
            }

            match &events[self.pos].kind {
                EventKind::SequenceEntry => {
                    self.pos += 1;
                    // An empty entry (next is a sibling `-` or the end) is null,
                    // not a nested sequence absorbing the sibling.
                    if self.pos < events.len()
                        && matches!(
                            events[self.pos].kind,
                            EventKind::SequenceEntry
                                | EventKind::SequenceEnd
                                | EventKind::MappingEnd
                                | EventKind::BlockEnd
                        )
                    {
                        items.push(Value::Null);
                    } else if let Some(value) = self.decode_node(events)? {
                        items.push(value);
                    }
                }
                EventKind::BlockEnd => {
                    // An implicit block sequence (no `SequenceStart` of its own)
                    // sits at the same indent as its parent mapping key, so it
                    // shares that mapping's block level: the `BlockEnd` ending the
                    // level belongs to the mapping. Break without consuming it, so
                    // the enclosing mapping closes itself; consuming it here would
                    // strand the mapping and mis-read a following sibling entry.
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
    use super::{decode_with, DecodeErrorKind, Value};
    use crate::resolver::Schema;

    fn decode(input: &str) -> Vec<Value<'_>> {
        decode_with(input, Schema::Yaml12, false, false).expect("valid YAML")
    }

    fn s(text: &str) -> Value<'static> {
        Value::String(text.to_owned().into())
    }

    #[test]
    fn scalars() {
        assert_eq!(decode("42"), vec![Value::Int(42)]);
        assert_eq!(decode("true"), vec![Value::Bool(true)]);
        assert_eq!(decode("~"), vec![Value::Null]);
        assert_eq!(decode("hello"), vec![s("hello")]);
    }

    #[test]
    fn sequence_and_mapping() {
        assert_eq!(
            decode("- 1\n- 2\n"),
            vec![Value::Sequence(vec![Value::Int(1), Value::Int(2)])]
        );
        assert_eq!(
            decode("a: 1\nb: two\n"),
            vec![Value::Mapping(vec![
                (s("a"), Value::Int(1)),
                (s("b"), s("two"))
            ])]
        );
    }

    #[test]
    fn multiple_documents() {
        assert_eq!(decode("---\na: 1\n---\nb: 2\n").len(), 2);
    }

    #[test]
    fn sequence_value_at_mapping_key_indent() {
        // A block sequence value indented level with the mapping key shares the
        // mapping's block level; the following sibling entry must still parse as
        // a new sequence item, not be mis-read as a key. Regression pin for the
        // implicit-sequence BlockEnd handling.
        let out = decode("items:\n  - k:\n    - a\n    - b\n  - k2: v\n");
        let Value::Mapping(top) = &out[0] else {
            panic!("expected a mapping");
        };
        let Value::Sequence(items) = &top[0].1 else {
            panic!("expected a sequence");
        };
        assert_eq!(items.len(), 2, "both sibling entries are items");
        assert_eq!(
            items[0],
            Value::Mapping(vec![(s("k"), Value::Sequence(vec![s("a"), s("b")]))])
        );
        assert_eq!(items[1], Value::Mapping(vec![(s("k2"), s("v"))]));
    }

    #[test]
    fn duplicate_keys_warn_collects_without_erroring() {
        // Warn mode keeps last-wins and records a diagnostic per repeat.
        let warn = super::WarnOptions {
            duplicate_keys: true,
            ..Default::default()
        };
        let (documents, warnings) =
            super::decode_collecting("a: 1\nb: 2\na: 3\n", Schema::Yaml12, false, false, warn)
                .unwrap();
        assert_eq!(
            documents[0],
            Value::Mapping(vec![
                (s("a"), Value::Int(1)),
                (s("b"), Value::Int(2)),
                (s("a"), Value::Int(3)),
            ])
        );
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("duplicate mapping key 'a'"),
            "{warnings:?}"
        );
    }

    #[test]
    fn no_duplicate_warnings_without_the_flag() {
        let (_, warnings) = super::decode_collecting(
            "a: 1\na: 2\n",
            Schema::Yaml12,
            false,
            false,
            super::WarnOptions::default(),
        )
        .unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn yaml_11_warn_flags_one_one_only_syntax() {
        // Read as 1.1; flag the scalars 1.2 would type differently.
        let warn = super::WarnOptions {
            yaml_1_1: true,
            ..Default::default()
        };
        let (documents, warnings) = super::decode_collecting(
            "a: yes\nb: 0777\nc: 42\nd: hello\n",
            Schema::Yaml11,
            false,
            false,
            warn,
        )
        .unwrap();
        assert_eq!(
            documents[0],
            Value::Mapping(vec![
                (s("a"), Value::Bool(true)),
                (s("b"), Value::Int(511)),
                (s("c"), Value::Int(42)),
                (s("d"), s("hello")),
            ])
        );
        // Only `yes` and `0777` diverge from 1.2; `42` and `hello` agree.
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("'yes'") && warnings[0].contains("bool in 1.1"));
        assert!(warnings[1].contains("'0777'") && warnings[1].contains("int in 1.1"));
    }

    #[test]
    fn merge_keys_are_resolved() {
        let out = decode("base: &b\n  x: 1\nuse:\n  <<: *b\n  y: 2\n");
        let Value::Mapping(top) = &out[0] else {
            panic!("expected a mapping");
        };
        let used = &top.iter().find(|(k, _)| *k == s("use")).unwrap().1;
        let Value::Mapping(inner) = used else {
            panic!("expected a mapping");
        };
        assert!(inner.iter().any(|(k, _)| *k == s("x")), "merged key x");
        assert!(inner.iter().any(|(k, _)| *k == s("y")), "own key y");
    }

    #[test]
    fn yaml_11_mode_changes_boolean_resolution() {
        assert_eq!(
            decode_with("v: yes\n", Schema::Yaml11, false, false).unwrap(),
            vec![Value::Mapping(vec![(s("v"), Value::Bool(true))])]
        );
        // The default (1.2) keeps `yes` a string.
        assert_eq!(
            decode("v: yes\n"),
            vec![Value::Mapping(vec![(s("v"), s("yes"))])]
        );
    }

    #[test]
    fn duplicate_key_under_flag_is_its_own_kind() {
        let err = decode_with("a: 1\na: 2\n", Schema::Yaml12, true, false).unwrap_err();
        assert_eq!(err.kind, DecodeErrorKind::DuplicateKey);
        // Without the flag, duplicates are tolerated (no error).
        assert!(decode_with("a: 1\na: 2\n", Schema::Yaml12, false, false).is_ok());
    }

    #[test]
    fn syntax_error_is_a_parse_kind_with_a_message() {
        let err = decode_with("a: 'unterminated", Schema::Yaml12, false, false).unwrap_err();
        assert_eq!(err.kind, DecodeErrorKind::Parse);
        assert!(!err.message.is_empty());
    }

    #[test]
    fn deep_nesting_is_bounded_not_a_stack_overflow() {
        let err = decode_with(&"[".repeat(5000), Schema::Yaml12, false, false).unwrap_err();
        assert_eq!(err.kind, DecodeErrorKind::Parse);
    }

    #[test]
    fn reject_complex_keys_flag_rejects_collection_keys() {
        // Off by default: a complex key is accepted (FFI converts it later).
        assert!(decode_with("{a: 1}: b\n", Schema::Yaml12, false, false).is_ok());
        assert!(decode_with("[1, 2]: b\n", Schema::Yaml12, false, false).is_ok());
        // On: both mapping and sequence keys are rejected with the ComplexKey kind.
        for src in [
            "{a: 1}: b\n",
            "[1, 2]: b\n",
            "? {a: 1}\n: v\n",
            "[{a: 1}: v]\n",
        ] {
            let err = decode_with(src, Schema::Yaml12, false, true).unwrap_err();
            assert_eq!(err.kind, DecodeErrorKind::ComplexKey, "{src:?}");
        }
        // Scalar keys are unaffected even with the flag on.
        assert!(decode_with("k: v\n1: a\n", Schema::Yaml12, false, true).is_ok());
    }

    #[test]
    fn alias_bomb_is_rejected() {
        let mut src = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            let refs = vec![format!("*a{}", i - 1); 10].join(", ");
            src.push_str(&format!("a{i}: &a{i} [{refs}]\n"));
        }
        assert!(decode_with(&src, Schema::Yaml12, false, false).is_err());
    }
}
