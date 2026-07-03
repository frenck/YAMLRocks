use std::borrow::Cow;

use crate::scanner::{ScalarStyle, Span};

/// An event produced by the parser.
#[derive(Debug, Clone, PartialEq)]
pub struct Event<'input> {
    pub kind: EventKind<'input>,
    pub span: Span,
    /// Byte offset just past the source of a block-scalar event, for verbatim
    /// round-trip replay; equals `span.offset` for every other event.
    pub end_offset: usize,
}

impl<'input> Event<'input> {
    /// Pair an [`EventKind`] with the source span it was produced from.
    pub fn new(kind: EventKind<'input>, span: Span) -> Self {
        Self {
            kind,
            span,
            end_offset: span.offset,
        }
    }

    /// Like [`new`](Self::new) but with an explicit source end, for block scalars.
    pub fn with_end(kind: EventKind<'input>, span: Span, end_offset: usize) -> Self {
        Self {
            kind,
            span,
            end_offset,
        }
    }
}

/// The kind of a parser event.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind<'input> {
    StreamStart,
    StreamEnd,

    DocumentStart,
    DocumentEnd,

    /// `flow` is true for `{...}` flow mappings, false for block mappings. The
    /// distinction is needed downstream because a block mapping requires every
    /// entry to be `key: value`, while a flow mapping allows bare keys (`{a, b}`).
    MappingStart {
        flow: bool,
    },
    MappingEnd,

    /// `flow` is true for `[...]` flow sequences, false for block sequences. A
    /// block sequence's items are all introduced by `-`; a bare node where an
    /// item is expected is invalid, whereas a flow sequence lists nodes directly.
    SequenceStart {
        flow: bool,
    },
    SequenceEnd,

    BlockEnd,

    /// A mapping key marker. `explicit` is true when the source wrote the key
    /// with an explicit `?` indicator; false for an implicit `key:`. Only the
    /// round-trip composer reads it (to preserve the `?` on re-emit); the fast
    /// decode path ignores it.
    Key {
        explicit: bool,
    },
    Value,
    SequenceEntry,
    FlowEntry,

    Scalar(Cow<'input, str>, ScalarStyle),
    Anchor(String),
    Alias(String),
    Tag(String),

    Directive(String),
}
