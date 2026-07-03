//! The parser: the second stage of the pipeline, turning the scanner's flat
//! token stream into structural events.
//!
//! Where the [`crate::scanner`] deals in characters and layout, the parser deals
//! in shape: stream and document boundaries, the start and end of mappings and
//! sequences, scalars, aliases, anchors, and tags. Each [`Event`] carries the
//! [`Span`](crate::scanner::Span) it came from, so downstream stages can resolve
//! types, build Python objects, or compose a round-trip AST without ever
//! revisiting the raw text.

mod event;

pub use event::{Event, EventKind};

use crate::scanner::{Comment, ScanError, Scanner, Span, Token, TokenKind};

/// Diagnostic for a bare scalar key in a block mapping that is not followed by
/// `:`. Shared so the fast decoder and the round-trip composer report a missing
/// colon identically (same text, same span).
pub(crate) const MISSING_COLON_MESSAGE: &str = "expected ':' after mapping key";

/// Diagnostic for a block collection sitting in mapping-key position. Shared by
/// both decode paths for identical reporting.
pub(crate) const BLOCK_COLLECTION_KEY_MESSAGE: &str = "a block collection cannot be a mapping key";

/// Whether the events at `pos` begin a block collection in mapping-key position,
/// returning its node-start span if so. An implicit key must be a single-line
/// node; a block sequence or mapping reaching key position is the product of
/// mis-indented content (`key:\n  ok: 1\n wrong: 2`), not a real key. Flow
/// collections (`[a]: b`) and scalars are fine, as is any key inside a flow
/// mapping, where the surrounding `{}` govern; hence the `flow` exemption. Any
/// leading anchor/tag properties are skipped to reach the key's node-start.
pub(crate) fn block_collection_key_span(events: &[Event], pos: usize, flow: bool) -> Option<Span> {
    if flow {
        return None;
    }
    let mut i = pos;
    while i < events.len() && matches!(events[i].kind, EventKind::Anchor(_) | EventKind::Tag(_)) {
        i += 1;
    }
    let event = events.get(i)?;
    matches!(
        event.kind,
        EventKind::MappingStart { flow: false }
            | EventKind::SequenceStart { flow: false }
            | EventKind::SequenceEntry
    )
    .then_some(event.span)
}

/// YAML parser: converts a token stream into a sequence of events.
///
/// Events represent the logical structure of YAML documents
/// (stream, documents, mappings, sequences, scalars, aliases).
pub struct Parser<'input> {
    scanner: Scanner<'input>,
    peeked: Option<Token<'input>>,
    /// Source byte length, used to preallocate the events vector so it does not
    /// repeatedly reallocate as parsing fills it.
    input_len: usize,
}

impl<'input> Parser<'input> {
    /// Create a parser over `input`, with `file_id` 0 (the single-source case).
    pub fn new(input: &'input str) -> Self {
        Self {
            scanner: Scanner::new(input),
            peeked: None,
            input_len: input.len(),
        }
    }

    /// Create a parser whose events carry `file_id` in their spans, so they
    /// stay attributable to their origin file once includes are resolved.
    pub fn new_with_file_id(input: &'input str, file_id: u32) -> Self {
        Self {
            scanner: Scanner::new_with_file_id(input, file_id),
            peeked: None,
            input_len: input.len(),
        }
    }

    /// Whether the input began with a UTF-8 byte order mark (now skipped). The
    /// round-trip composer reads this to restore the mark on re-emission.
    pub fn had_bom(&self) -> bool {
        self.scanner.had_bom()
    }

    /// Parse all events from the input.
    pub fn parse_all(&mut self) -> Result<Vec<Event<'input>>, ScanError> {
        // Preallocate from the source length. Structured YAML runs 2-4 bytes
        // per event (a terse `k:` line is under 2), so a small document gets a
        // full-density reserve (half its byte length): for a document that fits
        // comfortably in one allocation, a single doubling right at the end of
        // the parse costs more than the over-reserve. A large document instead
        // gets the statistical density (an eighth): growth doublings amortize
        // over its parse time, while a full-density reserve would allocate far
        // past what scalar-heavy input ever fills. The cap keeps a crafted
        // input from forcing an outsized up-front allocation; past it the
        // vector simply grows as before.
        let estimate = (self.input_len / 8).max((self.input_len / 2).min(4096));
        let mut events = Vec::with_capacity(estimate.clamp(8, 65_536));
        loop {
            let event = self.next_event()?;
            let is_end = matches!(event.kind, EventKind::StreamEnd);
            events.push(event);
            if is_end {
                break;
            }
        }
        Ok(events)
    }

    /// Parse all events and return the comments seen along the way, in source
    /// order. Used by the round-trip composer to reattach comments to nodes.
    pub fn parse_all_with_comments(
        &mut self,
    ) -> Result<(Vec<Event<'input>>, Vec<Comment>), ScanError> {
        self.scanner.set_record_comments(true);
        let events = self.parse_all()?;
        let comments = self.scanner.take_comments();
        Ok((events, comments))
    }

    /// Produce the next event.
    pub fn next_event(&mut self) -> Result<Event<'input>, ScanError> {
        let token = self.next_token()?;
        self.token_to_event(token)
    }

    fn next_token(&mut self) -> Result<Token<'input>, ScanError> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }
        self.scanner.next_token()
    }

    fn token_to_event(&mut self, token: Token<'input>) -> Result<Event<'input>, ScanError> {
        let span = token.span;
        let end_offset = token.end_offset;
        match token.kind {
            TokenKind::StreamStart => Ok(Event::new(EventKind::StreamStart, span)),
            TokenKind::StreamEnd => Ok(Event::new(EventKind::StreamEnd, span)),

            TokenKind::DocumentStart => Ok(Event::new(EventKind::DocumentStart, span)),
            TokenKind::DocumentEnd => Ok(Event::new(EventKind::DocumentEnd, span)),

            TokenKind::BlockMappingStart => {
                Ok(Event::new(EventKind::MappingStart { flow: false }, span))
            }
            TokenKind::BlockSequenceStart => {
                Ok(Event::new(EventKind::SequenceStart { flow: false }, span))
            }
            TokenKind::BlockEnd => Ok(Event::new(EventKind::BlockEnd, span)),

            TokenKind::FlowMappingStart => {
                Ok(Event::new(EventKind::MappingStart { flow: true }, span))
            }
            TokenKind::FlowMappingEnd => Ok(Event::new(EventKind::MappingEnd, span)),
            TokenKind::FlowSequenceStart => {
                Ok(Event::new(EventKind::SequenceStart { flow: true }, span))
            }
            TokenKind::FlowSequenceEnd => Ok(Event::new(EventKind::SequenceEnd, span)),

            TokenKind::Key { explicit } => {
                // Key is followed by the actual key value; carry whether the
                // source used an explicit `?` so the round-trip emitter can
                // restore it.
                Ok(Event::new(EventKind::Key { explicit }, span))
            }
            TokenKind::Value => Ok(Event::new(EventKind::Value, span)),

            TokenKind::BlockEntry => Ok(Event::new(EventKind::SequenceEntry, span)),
            TokenKind::FlowEntry => Ok(Event::new(EventKind::FlowEntry, span)),

            TokenKind::Scalar(value, style) => Ok(Event::with_end(
                EventKind::Scalar(value, style),
                span,
                end_offset,
            )),

            TokenKind::Anchor(name) => Ok(Event::new(EventKind::Anchor(name), span)),
            TokenKind::Alias(name) => Ok(Event::new(EventKind::Alias(name), span)),
            TokenKind::Tag(tag) => Ok(Event::new(EventKind::Tag(tag), span)),

            TokenKind::Directive(text) => Ok(Event::new(EventKind::Directive(text), span)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EventKind, Parser};

    fn scalars(src: &str) -> usize {
        Parser::new(src)
            .parse_all()
            .unwrap()
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Scalar(..)))
            .count()
    }

    #[test]
    fn block_mapping_event_stream() {
        let events = Parser::new("a: 1\n").parse_all().unwrap();
        assert!(matches!(
            events.first().unwrap().kind,
            EventKind::StreamStart
        ));
        assert!(events
            .iter()
            .any(|e| matches!(e.kind, EventKind::MappingStart { flow: false })));
        assert_eq!(scalars("a: 1\n"), 2); // key + value
    }

    #[test]
    fn explicit_marker_emits_document_start() {
        let explicit = Parser::new("---\na: 1\n").parse_all().unwrap();
        assert!(explicit
            .iter()
            .any(|e| matches!(e.kind, EventKind::DocumentStart)));
        // An implicit document carries no explicit DocumentStart event.
        let implicit = Parser::new("a: 1\n").parse_all().unwrap();
        assert!(!implicit
            .iter()
            .any(|e| matches!(e.kind, EventKind::DocumentStart)));
    }

    #[test]
    fn flow_sequence_event_stream() {
        let events = Parser::new("[1, 2, 3]\n").parse_all().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e.kind, EventKind::SequenceStart { flow: true })));
        assert_eq!(scalars("[1, 2, 3]\n"), 3);
    }

    #[test]
    fn anchor_and_alias_events() {
        let events = Parser::new("base: &a 1\nuse: *a\n").parse_all().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Anchor(n) if n == "a")));
        assert!(events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Alias(n) if n == "a")));
    }

    #[test]
    fn tag_event() {
        let events = Parser::new("v: !mytag 5\n").parse_all().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(&e.kind, EventKind::Tag(t) if t == "!mytag")));
    }

    #[test]
    fn malformed_input_errors() {
        assert!(Parser::new("a: 'unterminated").parse_all().is_err());
    }
}
