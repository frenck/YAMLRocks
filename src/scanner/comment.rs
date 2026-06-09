use super::token::Span;

/// A comment extracted during scanning, retained only on the round-trip path.
///
/// Comments are reattached to AST nodes by the composer using their `span`
/// (line/column proximity), so no position classification is stored here.
#[derive(Debug, Clone)]
pub struct Comment {
    /// The comment text, with the leading `#` and one optional space removed and
    /// any trailing whitespace trimmed.
    pub text: String,
    /// Source location of the `#`.
    pub span: Span,
}
