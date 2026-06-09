use std::borrow::Cow;

use super::ScalarStyle;

/// Source location in the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub file_id: u32,
    pub line: u32,
    pub column: u32,
    pub offset: usize,
}

impl Span {
    pub fn new(file_id: u32, line: u32, column: u32, offset: usize) -> Self {
        Self {
            file_id,
            line,
            column,
            offset,
        }
    }
}

/// A token produced by the scanner.
///
/// Scalar text is a [`Cow`] borrowing directly from the input for the common
/// single-line plain case (no allocation), and owned only when the scanner had
/// to transform it (line folding, quote unescaping, block joining).
#[derive(Debug, Clone, PartialEq)]
pub struct Token<'input> {
    pub kind: TokenKind<'input>,
    pub span: Span,
    /// Byte offset just past this token's source. For a scalar it marks the true
    /// source end: past the closing quote for a quoted scalar, the last content
    /// byte for a plain one, and the end of the verbatim slice the round-trip path
    /// replays for a block scalar (see
    /// [`Comments::raw`](crate::roundtrip::ast::Comments)). For every other token
    /// it equals `span.offset`.
    pub end_offset: usize,
}

impl<'input> Token<'input> {
    pub fn new(kind: TokenKind<'input>, span: Span) -> Self {
        Self {
            kind,
            span,
            end_offset: span.offset,
        }
    }
}

/// The kind of a scanner token.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'input> {
    StreamStart,
    StreamEnd,
    DocumentStart,
    DocumentEnd,
    Directive(String),

    BlockMappingStart,
    BlockSequenceStart,
    BlockEnd,
    BlockEntry,

    FlowMappingStart,
    FlowMappingEnd,
    FlowSequenceStart,
    FlowSequenceEnd,
    FlowEntry,

    Key,
    Value,

    Scalar(Cow<'input, str>, ScalarStyle),
    Anchor(String),
    Alias(String),
    Tag(String),
}
