//! The scanner: the first stage of the pipeline, turning raw YAML text into a
//! flat token stream.
//!
//! It owns everything that depends on raw characters and layout: indentation
//! tracking, block versus flow context, the four scalar styles (plain, quoted,
//! literal, folded), anchors, aliases, tags, directives, and comments. Comments
//! are scanned as first-class tokens rather than discarded, which is what lets
//! the round-trip path reattach them later. Every token carries a [`Span`] so
//! later stages can report precise source locations.
//!
//! The next stage, the [`crate::parser`], consumes these tokens and groups them
//! into structural events.

mod char_traits;
pub(crate) mod comment;
mod reader;
mod scalar;
mod token;

use std::borrow::Cow;
use std::collections::VecDeque;

pub use comment::Comment;
pub use token::{Span, Token, TokenKind};

use char_traits::{is_break, is_flow_indicator, is_whitespace_or_break, is_whitespace_or_flow};
// Re-exported for the round-trip anchor setter, which validates an edited anchor
// name against the same rule the scanner uses to read one.
pub(crate) use char_traits::is_anchor_char;
use reader::Reader;

/// Maximum flow-collection nesting the scanner will open before rejecting the
/// input, mirroring the decoder's depth cap so a `[[[[...` bomb is bounded at
/// scan time. Kept in step with `decode::MAX_DEPTH`.
const MAX_FLOW_DEPTH: usize = 1000;

/// YAML scanner: converts a byte stream into a sequence of tokens.
///
/// The scanner handles indentation tracking, block/flow context management,
/// and comment extraction as first-class tokens.
pub struct Scanner<'input> {
    reader: Reader<'input>,
    // A front-drained queue: `fetch_*` push tokens to the back, `next_token`
    // pops from the front. A `VecDeque` keeps the handoff O(1) per token
    // (`Vec::remove(0)` would be O(n), quadratic over a long token stream).
    tokens: VecDeque<Token<'input>>,
    indent_stack: Vec<i32>,
    current_indent: i32,
    /// Whether the current block-indent level is a sequence (opened by a
    /// `BlockSequenceStart`). A non-`-` line at a sequence's own indent ends it;
    /// a mapping at the same indent simply continues. Parallels `indent_stack`.
    current_is_seq: bool,
    indent_kinds: Vec<bool>,
    flow_level: u32,
    /// Whether a value has been scanned since the last flow separator (`,`) or
    /// opening bracket. Used to reject empty flow entries (a leading or repeated
    /// comma). Only meaningful inside a flow collection.
    flow_value_seen: bool,
    /// Whether the previous flow token completed a node value, so the next
    /// node must be preceded by a separator (`,`, or a `:`/`?` introducing a
    /// fresh key/value). A node starting while this is set means a missing
    /// entry comma (`[a b]`). Only meaningful inside a flow collection.
    flow_need_sep: bool,
    /// The kind of each open flow collection, innermost last: `true` for a
    /// mapping (`{`), `false` for a sequence (`[`). A flow sequence's single
    /// pair requires its implicit key and `:` on one line; a flow mapping does
    /// not, so the two are distinguished here.
    flow_kinds: Vec<bool>,
    /// The source line of the node that [`flow_need_sep`] is owed for: the
    /// implicit key a following `:` would pair to. Used to enforce the
    /// single-line key rule inside a flow sequence.
    flow_pending_key_line: u32,
    /// Whether an explicit `?` key indicator is pending in the current flow
    /// entry. An explicit key may span multiple lines, so the single-line
    /// flow-sequence key rule does not apply to it.
    flow_explicit_key: bool,
    /// Whether the previous flow node was a JSON-like key (a quoted scalar or a
    /// flow collection). After such a key a `:` may introduce the value with no
    /// separating space (`"key":value`), the adjacent-value form.
    flow_prev_json: bool,
    /// Whether the cursor is still on the line of a `---` document-start marker.
    /// A block collection may not begin inline there (`--- key: value`).
    on_doc_start_line: bool,
    /// Whether a `%` directive is allowed at the current point: only at the start
    /// of the stream or right after a `...` document-end marker. Any document
    /// content or a `---` start closes the window; a `...` reopens it. A directive
    /// reached while closed (`%YAML 1.2\n---\n%YAML 1.2`) is invalid.
    directives_allowed: bool,
    /// Source line on which the outermost open flow collection began. Used to
    /// reject a multiline flow collection used as an implicit mapping key.
    flow_start_line: u32,
    /// The block indentation in effect when the outermost flow collection
    /// opened. A flow continuation line must be indented past it.
    flow_block_indent: i32,
    /// Source line of the most recent `:` value indicator. A block sequence
    /// entry on that same line (`key: - a`) is invalid: the sequence value must
    /// begin on the next line.
    value_indicator_line: u32,
    /// Source line of the most recent anchor/tag property. A block sequence
    /// entry on that same line (`&a - x`) is invalid.
    property_line: u32,
    /// Whether a block mapping value is awaiting its node (a `:` was emitted
    /// whose value has not yet been produced). A node property (anchor/tag) for
    /// that value that lands at or before the mapping's own indent is malformed
    /// (`seq:\n&anchor\n- a`, `key: &x\n!!map`).
    block_value_pending: bool,
    /// Whether the previous block token was a sequence entry `-` with no
    /// content yet. A `-` that opens a deeper indent is valid only as such an
    /// empty entry's value; otherwise it is a mis-indented item
    /// (`- key: value\n - item`). The token buffer is drained as tokens are
    /// handed out, so this cannot be read back from `tokens`.
    after_block_entry: bool,
    /// Whether reaching the current token closed one or more deeper block
    /// levels. Distinguishes a sequence item mis-indented out of a just-closed
    /// mapping (`- key: value\n - item`, an error) from a `-` that folds into a
    /// preceding plain scalar (`- a\n - b`, one scalar, no level was closed).
    unwound_block_level: bool,
    /// A pending "possible simple key" introduced by an anchor/tag property at a
    /// key position (`&a key:`). Such a key does not take the inline-key path, so
    /// its `BlockMappingStart`/`Key` must be inserted retroactively, at the
    /// property's column, once a `:` confirms it. Records (token index in
    /// `tokens`, column, line, tab-in-indent). The last flag carries whether a
    /// tab preceded the key, so a `:` can reject it; see [`Self::tab_before_token`].
    /// Block context only.
    simple_key: Option<(usize, i32, u32, bool)>,
    /// Whether a tab was consumed in the whitespace immediately before the token
    /// now being fetched (block context only; reset at the start of each
    /// [`Self::scan_to_next_token`]). A tab is valid separation before a plain or
    /// flow scalar, but it cannot indent a block collection node, so a `-` entry
    /// or a mapping key positioned right after such a tab is rejected. The check
    /// is deferred to the node fetch because the tab alone does not say which it
    /// is (`foo:\n \tbar` is a value and fine; `foo:\n \tbar: 1` is a key and not).
    tab_before_token: bool,
    stream_started: bool,
    stream_ended: bool,
    simple_key_allowed: bool,
    /// All comments seen so far, in source order. Only populated when
    /// [`Scanner::record_comments`] is enabled (the round-trip path).
    comments: Vec<Comment>,
    /// Whether to retain comments. The fast path leaves this `false` so
    /// comment scanning has near-zero cost; the round-trip path enables it.
    record_comments: bool,
}

impl<'input> Scanner<'input> {
    pub fn new(input: &'input str) -> Self {
        Self {
            reader: Reader::new(input),
            // Pre-sized so ordinary documents never grow them: the token
            // queue holds a handful of lookahead tokens and the indent stacks
            // track block nesting depth.
            tokens: VecDeque::with_capacity(16),
            indent_stack: Vec::with_capacity(16),
            current_indent: -1,
            current_is_seq: false,
            indent_kinds: Vec::with_capacity(16),
            flow_level: 0,
            flow_value_seen: false,
            flow_need_sep: false,
            flow_kinds: Vec::new(),
            flow_pending_key_line: 0,
            flow_explicit_key: false,
            flow_prev_json: false,
            on_doc_start_line: false,
            flow_start_line: 0,
            flow_block_indent: -1,
            value_indicator_line: u32::MAX,
            property_line: u32::MAX,
            block_value_pending: false,
            after_block_entry: false,
            unwound_block_level: false,
            simple_key: None,
            tab_before_token: false,
            directives_allowed: true,
            stream_started: false,
            stream_ended: false,
            simple_key_allowed: true,
            comments: Vec::new(),
            record_comments: false,
        }
    }

    pub fn new_with_file_id(input: &'input str, file_id: u32) -> Self {
        Self {
            reader: Reader::new_with_file_id(input, file_id),
            ..Self::new(input)
        }
    }

    /// Whether the input began with a UTF-8 byte order mark (now skipped).
    pub fn had_bom(&self) -> bool {
        self.reader.had_bom()
    }

    /// Enable retention of comments for the round-trip path.
    pub fn set_record_comments(&mut self, record: bool) {
        self.record_comments = record;
    }

    /// Take all comments collected so far, leaving the scanner's list empty.
    pub fn take_comments(&mut self) -> Vec<Comment> {
        std::mem::take(&mut self.comments)
    }

    /// Produce the next token from the input stream.
    pub fn next_token(&mut self) -> Result<Token<'input>, ScanError> {
        loop {
            // A pending simple key means a `Key` (and possibly a
            // `BlockMappingStart`) may still need to be inserted before the
            // already-buffered tokens, so do not hand any out until it resolves.
            if self.simple_key.is_none() {
                if let Some(token) = self.tokens.pop_front() {
                    return Ok(token);
                }
            }
            if self.stream_ended {
                if let Some(token) = self.tokens.pop_front() {
                    return Ok(token);
                }
                let span = self.reader.span();
                return Ok(Token::new(TokenKind::StreamEnd, span));
            }
            self.fetch_more_tokens()?;
        }
    }

    fn fetch_more_tokens(&mut self) -> Result<(), ScanError> {
        if !self.stream_started {
            self.stream_started = true;
            let span = self.reader.span();
            self.tokens
                .push_back(Token::new(TokenKind::StreamStart, span));
            return Ok(());
        }

        let line_before = self.reader.line();
        self.scan_to_next_token()?;

        // Inside a flow collection, a continuation line must be indented past
        // the block context that the collection opened in. The YAML spec (and
        // the test suite, e.g. `9C9N`) treats a line at or below that column as
        // having escaped the collection, so it is malformed, even though PyYAML
        // leniently accepts it. We stay spec-strict; real configs that rely on
        // the lenient reading are recorded in the real-world suite's xfails.
        if self.flow_level > 0
            && !self.reader.is_eof()
            && self.reader.line() > line_before
            && (self.reader.column() as i32) <= self.flow_block_indent
        {
            return Err(ScanError::new(
                "flow content must be indented more than its block context",
                self.reader.span(),
            ));
        }

        if self.flow_level == 0 && !self.reader.is_eof() {
            let column = self.reader.column() as i32;
            self.unwound_block_level = false;
            self.unwind_indents(column);
            self.close_seq_if_non_entry();
        }

        if self.reader.is_eof() && !self.stream_ended {
            if self.flow_level > 0 {
                return Err(ScanError::new(
                    "unclosed flow collection at end of input",
                    self.reader.span(),
                ));
            }
            // A simple key never completed (no `:`); its tokens stand as-is.
            self.simple_key = None;
            self.unwind_indents(-1);
            self.stream_ended = true;
            let span = self.reader.span();
            self.tokens
                .push_back(Token::new(TokenKind::StreamEnd, span));
            return Ok(());
        }

        if self.reader.is_eof() {
            return Ok(());
        }

        self.fetch_next_token()
    }

    fn fetch_next_token(&mut self) -> Result<(), ScanError> {
        let ch = self.reader.peek();

        if self.flow_level > 0 {
            return self.fetch_flow_token(ch);
        }

        self.fetch_block_token(ch)
    }

    fn fetch_flow_token(&mut self, ch: char) -> Result<(), ScanError> {
        // A `---`/`...` document marker at column 0 cannot appear inside a flow
        // collection; the collection spans documents, which is invalid.
        if self.reader.column() == 0
            && (self.reader.check_ahead("---") || self.reader.check_ahead("..."))
            && self.reader.peek_at(3).map_or(true, is_whitespace_or_break)
        {
            return Err(ScanError::new(
                "a document marker is not allowed inside a flow collection",
                self.reader.span(),
            ));
        }
        // Whether the previous flow node was a JSON-like key, allowing this `:`
        // to introduce its value with no separating space. Consumed by this
        // token; a quoted scalar / flow collection re-arms it.
        let prev_json = self.flow_prev_json;
        self.flow_prev_json = false;

        // A new node may not begin while a separator is still owed from the
        // previous value (`[a b]`). The structural tokens `]`, `}`, `,` are
        // separators, and `#` errors on its own below. A `:` is never a node
        // start: even adjacent (`"foo":bar`, or a `:bar` continuation on the
        // next line) it pairs a value to the preceding node as its key. A `?`
        // or `-` followed by a blank/flow indicator is likewise a separator.
        let is_separator = matches!(ch, ']' | '}' | ',' | '#' | ':')
            || (matches!(ch, '?' | '-')
                && self.reader.peek_next().map_or(true, is_whitespace_or_flow));
        if !is_separator && self.flow_need_sep {
            return Err(ScanError::new(
                "missing ',' between flow collection entries",
                self.reader.span(),
            ));
        }

        // A `:` that pairs the preceding node as an implicit key: inside a flow
        // sequence (a single-pair entry) the key and its `:` must be on one
        // line (`[ key\n : value ]` and `[ "key"\n :value ]` are invalid). A
        // flow mapping permits the `:` on a following line.
        if ch == ':'
            && self.flow_need_sep
            && !self.flow_explicit_key
            && self.flow_kinds.last() == Some(&false)
            && self.flow_pending_key_line != self.reader.line()
        {
            return Err(ScanError::new(
                "the implicit key of a flow sequence pair must be on one line",
                self.reader.span(),
            ));
        }

        match ch {
            '[' => self.fetch_flow_collection_start(TokenKind::FlowSequenceStart),
            '{' => self.fetch_flow_collection_start(TokenKind::FlowMappingStart),
            ']' => self.fetch_flow_collection_end(TokenKind::FlowSequenceEnd),
            '}' => self.fetch_flow_collection_end(TokenKind::FlowMappingEnd),
            ',' => self.fetch_flow_entry(),
            ':' if prev_json || self.reader.peek_next().map_or(true, is_whitespace_or_flow) => {
                self.fetch_value()
            }
            '?' if self.reader.peek_next().map_or(true, is_whitespace_or_flow) => self.fetch_key(),
            // A bare `-` (followed by a flow indicator or blank) cannot start a
            // plain scalar in flow context (`[-]`, `[-, -]`).
            '-' if self.reader.peek_next().map_or(true, is_whitespace_or_flow) => {
                Err(ScanError::new(
                    "a plain scalar cannot be a bare '-' here",
                    self.reader.span(),
                ))
            }
            // A node (scalar, alias, or a node carrying an anchor/tag, including
            // an anchored empty node) satisfies the pending flow entry.
            '\'' => self.flow_node(Self::fetch_single_quoted_scalar),
            '"' => self.flow_node(Self::fetch_double_quoted_scalar),
            '&' => self.flow_node(Self::fetch_anchor),
            '*' => self.flow_node(Self::fetch_alias),
            '!' => self.flow_node(Self::fetch_tag),
            '#' => Err(ScanError::new(
                "a comment must be preceded by whitespace",
                self.reader.span(),
            )),
            // `@` and the backtick are reserved indicators in YAML: they cannot
            // start a plain scalar (they must be quoted), matching PyYAML.
            '@' | '`' => Err(reserved_indicator_error(ch, self.reader.span())),
            _ => self.flow_node(Self::fetch_plain_scalar),
        }
    }

    /// Run a value-producing fetch and mark that this flow entry now has a value.
    fn flow_node(
        &mut self,
        fetch: fn(&mut Self) -> Result<(), ScanError>,
    ) -> Result<(), ScanError> {
        fetch(self)?;
        self.flow_value_seen = true;
        Ok(())
    }

    fn fetch_block_token(&mut self, ch: char) -> Result<(), ScanError> {
        // Any token other than a node property (anchor/tag) settles a pending
        // mapping value: a scalar/collection fills it, a new key/entry leaves it
        // null. Only a property keeps the value open, so the indentation of a
        // property continuation can still be checked against the key.
        if !matches!(ch, '&' | '!') {
            self.block_value_pending = false;
        }
        // Each block token clears the "just after a `-`" state; a block entry
        // re-sets it at the end of `fetch_block_entry`. Capture it first so the
        // entry can tell whether it follows an empty parent entry.
        let after_entry = self.after_block_entry;
        self.after_block_entry = false;
        // Directives are only valid before a document. A `%` keeps the window
        // open and `...` reopens it (handled in their fetchers); anything else
        // here, document content or a `---` start, closes it.
        let is_directive = ch == '%' && self.reader.column() == 0;
        // A document-end marker is only one at column 0; an indented `...` is a
        // plain scalar (`key: ...`, `- ...`), so it must not reopen the directive
        // window any more than it ends the document below.
        let is_doc_end = ch == '.'
            && self.reader.column() == 0
            && self.reader.check_ahead("...")
            && self.reader.peek_at(3).map_or(true, is_whitespace_or_break);
        if !is_directive && !is_doc_end {
            self.directives_allowed = false;
        }
        match ch {
            '-' if self.reader.check_next_is_blank()
                || self.reader.check_next_is_break_or_eof() =>
            {
                self.fetch_block_entry(after_entry)
            }
            // `---`/`...` are document markers only at the start of a line
            // (column 0). Indented, they are ordinary plain-scalar content in a
            // mapping value or sequence item (`key: ...`, `- ---`), which the spec
            // and PyYAML both read as the literal string, not a marker.
            '-' if self.reader.column() == 0
                && self.reader.check_ahead("---")
                && self.reader.peek_at(3).map_or(true, is_whitespace_or_break) =>
            {
                self.fetch_document_start()
            }
            '.' if self.reader.column() == 0
                && self.reader.check_ahead("...")
                && self.reader.peek_at(3).map_or(true, is_whitespace_or_break) =>
            {
                self.fetch_document_end()
            }
            '%' if self.reader.column() == 0 => self.fetch_directive(),
            '[' => self.fetch_flow_collection_start(TokenKind::FlowSequenceStart),
            '{' => self.fetch_flow_collection_start(TokenKind::FlowMappingStart),
            ']' => self.fetch_flow_collection_end(TokenKind::FlowSequenceEnd),
            '}' => self.fetch_flow_collection_end(TokenKind::FlowMappingEnd),
            '?' if self.reader.peek_next().map_or(true, is_whitespace_or_break) => self.fetch_key(),
            ':' if self.reader.peek_next().map_or(true, is_whitespace_or_break) => {
                self.fetch_value()
            }
            // A plain scalar can contain a comma but cannot begin with one: the
            // comma is a flow indicator excluded from the plain first character
            // (e.g. a tag immediately followed by `,`, as in `!!str, xxx`).
            ',' => Err(ScanError::new(
                "a plain scalar cannot start with ','",
                self.reader.span(),
            )),
            '\'' => self.fetch_single_quoted_scalar(),
            '"' => self.fetch_double_quoted_scalar(),
            '|' => self.fetch_block_scalar(true),
            '>' => self.fetch_block_scalar(false),
            '&' => self.fetch_anchor(),
            '*' => self.fetch_alias(),
            '!' => self.fetch_tag(),
            '#' => Err(ScanError::new(
                "a comment must be preceded by whitespace",
                self.reader.span(),
            )),
            // `@` and the backtick are reserved indicators in YAML: they cannot
            // start a plain scalar (they must be quoted), matching PyYAML.
            '@' | '`' => Err(reserved_indicator_error(ch, self.reader.span())),
            _ => self.fetch_plain_scalar(),
        }
    }

    // -- Skip whitespace and comments --

    fn scan_to_next_token(&mut self) -> Result<(), ScanError> {
        // A `#` only begins a comment when it is at the start of input/line or
        // preceded by whitespace; otherwise it is not a comment indicator and a
        // `#` reaching the token dispatcher is rejected as stray content.
        //
        // A tab is never indentation (YAML indents with spaces), but it is valid
        // separation before a plain or flow scalar. Whether a leading or
        // separating tab is an error therefore depends on the node it precedes,
        // so we only record that a tab was seen and defer the verdict to the node
        // fetch (`fetch_block_entry`, the key promotions); see `tab_before_token`.
        self.tab_before_token = false;
        loop {
            match self.reader.peek() {
                ' ' => {
                    // Indentation and separation are runs of spaces; skip the
                    // whole run in one pass rather than one byte per loop turn.
                    self.reader.skip_spaces();
                }
                '\t' => {
                    if self.flow_level == 0 {
                        self.tab_before_token = true;
                    } else if self.reader.column() == 0 {
                        // A flow collection spanning lines indents continuation
                        // lines with spaces, so a tab opening a line is invalid
                        // indentation (`[\n\tfoo]`). It is allowed only when it
                        // merely separates flow punctuation (`\t[`) or sits on a
                        // blank line; a tab after leading spaces is separation,
                        // not indentation, so this only fires at column 0.
                        let next = self.reader.peek_after_blanks();
                        let separating = next.map_or(true, |c| is_break(c) || is_flow_indicator(c));
                        if !separating {
                            return Err(tab_indent_error(self.reader.span()));
                        }
                    }
                    self.reader.advance();
                }
                '\n' | '\r' => {
                    self.reader.advance_line();
                    self.on_doc_start_line = false;
                    // Only a tab on the token's own line is indentation for it; a
                    // tab on an intervening blank line is just blank content.
                    self.tab_before_token = false;
                    // A block simple key must complete on its own line; once the
                    // line ends with no `:`, it was an ordinary node.
                    if self
                        .simple_key
                        .is_some_and(|(_, _, line, _)| line != self.reader.line())
                    {
                        self.simple_key = None;
                    }
                    if self.flow_level == 0 {
                        self.simple_key_allowed = true;
                    }
                }
                '#' if self.reader.prev_is_whitespace_or_start() => {
                    self.scan_comment();
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn scan_comment(&mut self) {
        let span = self.reader.span();
        self.reader.advance(); // skip '#'
                               // Skip a single optional space after '#'; the rest is the comment text.
        if self.reader.peek() == ' ' {
            self.reader.advance();
        }
        let start = self.reader.offset();
        while !self.reader.is_eof() && !is_break(self.reader.peek()) {
            self.reader.advance();
        }
        if !self.record_comments {
            return;
        }
        // Trim trailing whitespace so emitted comments stay tidy.
        let text = self
            .reader
            .slice(start, self.reader.offset())
            .trim_end()
            .to_owned();
        self.comments.push(Comment { text, span });
    }

    /// Check if ':' (value indicator) follows the current position, skipping spaces.
    /// Used after scanning a scalar to detect mapping keys.
    /// Only skips spaces on the same line; does not cross line boundaries.
    fn check_value_after_scalar(&mut self) -> bool {
        let saved = self.reader.save();
        self.reader.skip_spaces();
        if !self.reader.is_eof() && self.reader.peek() == ':' {
            let next = self.reader.peek_next();
            if next.map_or(true, is_whitespace_or_break)
                || (self.flow_level > 0 && next.is_some_and(is_flow_indicator))
            {
                return true;
            }
        }
        self.reader.restore(saved);
        false
    }

    /// After a complete node in block context (a quoted scalar or a closed flow
    /// collection), only a line break, a comment, or `:` (making it a mapping
    /// key) may follow on the line. Trailing plain content (`key: "x" junk`,
    /// `{a: b}junk`) is invalid. In flow context the surrounding separators
    /// (`,` `]` `}`) govern instead, so this is a no-op there.
    fn check_no_block_trailing(&self, context: &str) -> Result<(), ScanError> {
        if self.flow_level > 0 {
            return Ok(());
        }
        match self.reader.peek_after_blanks() {
            None | Some('\n' | '\r' | '#' | ':') => Ok(()),
            Some(_) => Err(ScanError::new(
                format!("unexpected content after {context}"),
                self.reader.span(),
            )),
        }
    }

    // -- Indentation --

    /// Open a new block-context indentation level, emitting `kind` (a
    /// `BlockMappingStart`/`BlockSequenceStart`) stamped with `span`: the
    /// position of the first key or entry, so collection nodes report an
    /// accurate source location.
    fn roll_indent(
        &mut self,
        column: i32,
        span: Span,
        kind: TokenKind<'input>,
    ) -> Result<(), ScanError> {
        if self.flow_level > 0 {
            return Ok(());
        }
        // A block mapping or sequence cannot begin on the `---` line itself; the
        // marker line may only carry a scalar or a flow node.
        if self.on_doc_start_line {
            return Err(ScanError::new(
                "a block collection cannot start on the same line as ---",
                span,
            ));
        }
        if column > self.current_indent {
            self.indent_stack.push(self.current_indent);
            self.indent_kinds.push(self.current_is_seq);
            self.current_indent = column;
            self.current_is_seq = matches!(kind, TokenKind::BlockSequenceStart);
            self.tokens.push_back(Token::new(kind, span));
        }
        Ok(())
    }

    fn unwind_indents(&mut self, column: i32) {
        if self.flow_level > 0 {
            return;
        }
        while self.current_indent > column {
            let span = self.reader.span();
            self.tokens.push_back(Token::new(TokenKind::BlockEnd, span));
            self.current_indent = self.indent_stack.pop().unwrap_or(-1);
            self.current_is_seq = self.indent_kinds.pop().unwrap_or(false);
            self.unwound_block_level = true;
        }
    }

    /// Close a block sequence when a non-`-` line appears at the sequence's own
    /// indent (`- a\n- b\ninvalid`). A block sequence's items are all introduced
    /// by `-`; other content at its indent belongs to an enclosing level.
    fn close_seq_if_non_entry(&mut self) {
        // Only when the current level is a sequence, the cursor sits exactly at
        // its indent, and the next token is not a block entry `-`.
        if !self.current_is_seq || self.flow_level > 0 {
            return;
        }
        if self.reader.column() as i32 != self.current_indent {
            return;
        }
        let is_block_entry = self.reader.peek() == '-'
            && (self.reader.check_next_is_blank() || self.reader.check_next_is_break_or_eof());
        if is_block_entry {
            return;
        }
        let span = self.reader.span();
        self.tokens.push_back(Token::new(TokenKind::BlockEnd, span));
        self.current_indent = self.indent_stack.pop().unwrap_or(-1);
        self.current_is_seq = self.indent_kinds.pop().unwrap_or(false);
    }

    // -- Document markers --

    fn fetch_document_start(&mut self) -> Result<(), ScanError> {
        self.unwind_indents(-1);
        self.current_indent = -1;
        let span = self.reader.span();
        self.reader.advance_n(3); // skip '---'
        self.simple_key_allowed = false;
        self.on_doc_start_line = true;
        self.tokens
            .push_back(Token::new(TokenKind::DocumentStart, span));
        Ok(())
    }

    fn fetch_document_end(&mut self) -> Result<(), ScanError> {
        self.unwind_indents(-1);
        self.current_indent = -1;
        let span = self.reader.span();
        self.reader.advance_n(3); // skip '...'
                                  // Only spaces and a comment may follow `...` on its line; a node cannot
                                  // (unlike `---`, which may carry inline content).
        let next = self.reader.peek_after_blanks();
        if !matches!(next, None | Some('\n' | '\r' | '#')) {
            return Err(ScanError::new(
                "unexpected content after document-end marker (...)",
                span,
            ));
        }
        self.simple_key_allowed = false;
        // A `...` ends the document, so directives may introduce the next one.
        self.directives_allowed = true;
        self.tokens
            .push_back(Token::new(TokenKind::DocumentEnd, span));
        Ok(())
    }

    fn fetch_directive(&mut self) -> Result<(), ScanError> {
        // A directive is valid only at the start of the stream or right after a
        // `...` document-end marker; otherwise the previous document is still
        // open and must be closed with `...` first.
        if !self.directives_allowed {
            return Err(ScanError::new(
                "a directive must start the stream or follow a document-end marker (...)",
                self.reader.span(),
            ));
        }
        self.unwind_indents(-1);
        let span = self.reader.span();
        self.reader.advance(); // skip '%'
        let start = self.reader.offset();
        while !self.reader.is_eof() && !is_break(self.reader.peek()) {
            self.reader.advance();
        }
        let text = self.reader.slice(start, self.reader.offset()).to_owned();
        self.simple_key_allowed = false;
        self.tokens
            .push_back(Token::new(TokenKind::Directive(text), span));
        Ok(())
    }

    // -- Flow collections --

    fn fetch_flow_collection_start(&mut self, kind: TokenKind<'input>) -> Result<(), ScanError> {
        let span = self.reader.span();
        // Bound flow nesting in the scanner so a `[[[[...` bomb trips the depth
        // guard here (before EOF) rather than building an unbounded token run.
        if self.flow_level as usize >= MAX_FLOW_DEPTH {
            return Err(ScanError::new(
                format!("maximum nesting depth ({MAX_FLOW_DEPTH}) exceeded"),
                span,
            ));
        }
        if self.flow_level == 0 {
            self.flow_start_line = span.line;
            self.flow_block_indent = self.current_indent;
            // A top-level flow collection may itself be a block mapping key
            // (`[a, b]: c`). Record it as a possible simple key so a following
            // `:` inserts the mapping start and `Key` marker before it.
            self.note_possible_simple_key(span);
        }
        self.reader.advance();
        self.flow_level += 1;
        self.flow_kinds
            .push(matches!(kind, TokenKind::FlowMappingStart));
        self.simple_key_allowed = true;
        // A freshly opened collection has no value in its first entry yet.
        self.flow_value_seen = false;
        self.flow_need_sep = false;
        self.flow_explicit_key = false;
        self.tokens.push_back(Token::new(kind, span));
        Ok(())
    }

    fn fetch_flow_collection_end(&mut self, kind: TokenKind<'input>) -> Result<(), ScanError> {
        let span = self.reader.span();
        if self.flow_level == 0 {
            let bracket = if matches!(kind, TokenKind::FlowSequenceEnd) {
                ']'
            } else {
                '}'
            };
            return Err(ScanError::new(
                format!("unexpected '{bracket}' with no open flow collection"),
                span,
            ));
        }
        self.reader.advance();
        self.flow_level -= 1;
        // The closing indicator must match the opener: `]` closes a `[` and `}`
        // closes a `{`. A mismatch (`[a, b}` or `{a: b]`) is invalid YAML, which
        // the spec and the YAML test suite reject rather than silently accept.
        let opened_mapping = self.flow_kinds.pop();
        let closing_mapping = matches!(kind, TokenKind::FlowMappingEnd);
        if opened_mapping != Some(closing_mapping) {
            let (opener, closer) = if closing_mapping {
                ('[', '}')
            } else {
                ('{', ']')
            };
            return Err(ScanError::new(
                format!("mismatched flow collection: '{closer}' cannot close a '{opener}'"),
                span,
            ));
        }
        self.simple_key_allowed = false;
        // A closed collection is itself a value in the entry that contained it,
        // and a JSON-like key that a following `:` may bind adjacently.
        self.flow_value_seen = true;
        self.flow_need_sep = true;
        self.flow_pending_key_line = span.line;
        self.flow_prev_json = true;
        self.tokens.push_back(Token::new(kind, span));
        if self.flow_level == 0 {
            // Back at block level, a closed flow collection is a complete node;
            // trailing plain content on the line (`{a: b}junk`) is invalid.
            self.check_no_block_trailing("flow collection")?;
            // A flow collection used as an implicit key (`[a]: b`) must fit on a
            // single line.
            if span.line != self.flow_start_line && self.followed_by_value_indicator() {
                return Err(ScanError::new(
                    "a multiline flow collection cannot be an implicit mapping key",
                    span,
                ));
            }
        }
        Ok(())
    }

    /// Whether the next non-space character on the line is a `:` value indicator.
    fn followed_by_value_indicator(&self) -> bool {
        self.reader.peek_after_blanks() == Some(':')
    }

    fn fetch_flow_entry(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        // A `,` must separate two values; an empty entry (a leading or repeated
        // comma, as in `[, a]` or `[a, , b]`) is invalid.
        if !self.flow_value_seen {
            return Err(ScanError::new("empty flow entry (stray ',')", span));
        }
        self.reader.advance(); // skip ','
        self.simple_key_allowed = true;
        self.flow_value_seen = false;
        self.flow_need_sep = false;
        self.flow_explicit_key = false;
        self.tokens
            .push_back(Token::new(TokenKind::FlowEntry, span));
        Ok(())
    }

    // -- Block entries --

    fn fetch_block_entry(&mut self, after_entry: bool) -> Result<(), ScanError> {
        let column = self.reader.column() as i32;
        let span = self.reader.span();
        // A tab cannot indent a block sequence entry, whether it leads the line
        // or separates the entry from a parent indicator (`-\t-`, `- \t-`).
        if self.tab_before_token {
            return Err(tab_indent_error(span));
        }
        // A block sequence value must begin on the line after its mapping key,
        // not inline after the `:` (`key: - a`).
        if span.line == self.value_indicator_line {
            return Err(ScanError::new(
                "a block sequence cannot start on the same line as its mapping key",
                span,
            ));
        }
        // Nor inline after an anchor/tag property (`&a - x`).
        if span.line == self.property_line {
            return Err(ScanError::new(
                "a block sequence entry cannot follow an anchor or tag on the same line",
                span,
            ));
        }
        // A `-` that opens a deeper indent level inside an existing sequence,
        // reached by closing a deeper block (a mapping), is a mis-indented item
        // (`- key: value\n - item`). It is valid only as an empty parent entry's
        // value, which directly follows that entry's own `-`. When no level was
        // closed the `-` belongs to a folding plain scalar (`- a\n - b`), so it
        // is left alone.
        if column > self.current_indent
            && self.current_is_seq
            && !after_entry
            && self.unwound_block_level
        {
            return Err(ScanError::new(
                "a sequence entry is indented past its sequence",
                span,
            ));
        }
        self.roll_indent(column, span, TokenKind::BlockSequenceStart)?;
        self.reader.advance(); // skip '-'
        if self.reader.peek() == ' ' {
            self.reader.advance();
        }
        self.simple_key_allowed = true;
        self.after_block_entry = true;
        self.tokens
            .push_back(Token::new(TokenKind::BlockEntry, span));
        Ok(())
    }

    // -- Key / Value --

    fn fetch_key(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        // A `?` opens a block mapping; a tab cannot indent it, so a leading tab
        // before the `?` is invalid.
        if self.flow_level == 0 && self.tab_before_token {
            return Err(tab_indent_error(span));
        }
        if self.flow_level == 0 {
            let column = self.reader.column() as i32;
            self.roll_indent(column, span, TokenKind::BlockMappingStart)?;
        }
        self.reader.advance(); // skip '?'

        // Nor can a tab indent the explicit key's own node (`?\tkey`).
        if self.flow_level == 0 && self.reader.peek() == '\t' {
            return Err(tab_indent_error(self.reader.span()));
        }
        if !self.reader.is_eof() && self.reader.peek() == ' ' {
            self.reader.advance();
        }
        // After `?`, a simple key is allowed: the explicit key's node may be a
        // compact block mapping or sequence on the same line (`? a: b`, spec
        // example 8.19). Flow already allowed it; block must too.
        self.simple_key_allowed = true;
        // A `?` introduces a fresh key, satisfying any owed separator. Mark it
        // explicit so the single-line flow-sequence key rule is relaxed.
        self.flow_need_sep = false;
        if self.flow_level > 0 {
            self.flow_explicit_key = true;
        }
        self.tokens
            .push_back(Token::new(TokenKind::Key { explicit: true }, span));
        Ok(())
    }

    fn fetch_value(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        if self.flow_level == 0 {
            if let Some((index, key_col, key_line, key_had_tab)) = self
                .simple_key
                .take()
                .filter(|&(_, _, line, _)| line == span.line)
            {
                // A tab cannot indent the mapping this key opens.
                if key_had_tab {
                    return Err(tab_indent_error(span));
                }
                // A property-prefixed key (`&a key:`) confirmed by this `:`.
                // Insert the mapping-start and key marker retroactively at the
                // key's own column, before its buffered tokens.
                let key_span = Span {
                    line: key_line,
                    column: key_col as u32,
                    ..span
                };
                let mut at = index;
                if key_col > self.current_indent {
                    self.indent_stack.push(self.current_indent);
                    self.indent_kinds.push(self.current_is_seq);
                    self.current_indent = key_col;
                    self.current_is_seq = false;
                    self.tokens
                        .insert(at, Token::new(TokenKind::BlockMappingStart, key_span));
                    at += 1;
                }
                self.tokens
                    .insert(at, Token::new(TokenKind::Key { explicit: false }, key_span));
            } else {
                let column = self.reader.column() as i32;
                self.roll_indent(column, span, TokenKind::BlockMappingStart)?;
            }
        }
        self.reader.advance(); // skip ':'
        if !self.reader.is_eof() && self.reader.peek() == ' ' {
            self.reader.advance();
        }
        // After `:`, a simple key is allowed so the value may be a compact block
        // mapping on the same line (the `: moon: white` value of an explicit key,
        // spec example 8.19).
        self.simple_key_allowed = true;
        // A `:` introduces a fresh value, satisfying any owed separator and
        // closing out any explicit key.
        self.flow_need_sep = false;
        self.flow_explicit_key = false;
        if self.flow_level == 0 {
            self.block_value_pending = true;
        }
        self.tokens.push_back(Token::new(TokenKind::Value, span));
        Ok(())
    }

    /// Reject a node property (anchor/tag) that introduces a pending block
    /// mapping value but lands at or before the mapping's own indent. Such a
    /// property must be indented past the key (`seq:\n&anchor\n- a`, and the
    /// tag in `key: &x\n!!map`). An inline property (`key: &x value`) sits to
    /// the right of the key, so its column exceeds the indent and it passes.
    fn check_property_value_indent(&self, span: Span) -> Result<(), ScanError> {
        if self.flow_level == 0
            && self.block_value_pending
            && (self.reader.column() as i32) <= self.current_indent
        {
            return Err(ScanError::new(
                "a node property must be indented past its mapping key",
                span,
            ));
        }
        Ok(())
    }

    /// Record a possible block simple key at the current property token, so a
    /// later `:` on the same line can retroactively insert the mapping start and
    /// `Key` marker before the property's tokens. Only the first property of a
    /// potential key is recorded.
    fn note_possible_simple_key(&mut self, span: Span) {
        if self.flow_level == 0 && self.simple_key_allowed && self.simple_key.is_none() {
            self.simple_key = Some((
                self.tokens.len(),
                span.column as i32,
                span.line,
                self.tab_before_token,
            ));
        }
    }

    // -- Anchors and aliases --

    fn fetch_anchor(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        self.check_property_value_indent(span)?;
        self.note_possible_simple_key(span);
        self.reader.advance(); // skip '&'
        let start = self.reader.offset();
        while !self.reader.is_eof() && is_anchor_char(self.reader.peek()) {
            self.reader.advance();
        }
        if self.reader.offset() == start {
            return Err(ScanError::new("expected anchor name", span));
        }
        let name = self.reader.slice(start, self.reader.offset()).to_owned();
        self.simple_key_allowed = false;
        if self.flow_level == 0 {
            self.property_line = span.line;
        }
        self.tokens
            .push_back(Token::new(TokenKind::Anchor(name), span));
        Ok(())
    }

    fn fetch_alias(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        // An alias may itself be a mapping key (`*a : value`); record it as a
        // possible simple key so a following `:` can insert the mapping start
        // and `Key` marker before it, as for an anchor/tag-prefixed key.
        self.note_possible_simple_key(span);
        self.reader.advance(); // skip '*'
        let start = self.reader.offset();
        while !self.reader.is_eof() && is_anchor_char(self.reader.peek()) {
            self.reader.advance();
        }
        if self.reader.offset() == start {
            return Err(ScanError::new("expected alias name", span));
        }
        let name = self.reader.slice(start, self.reader.offset()).to_owned();
        self.simple_key_allowed = false;
        // An alias is a complete node; a following node needs a separator.
        if self.flow_level > 0 {
            self.flow_need_sep = true;
            self.flow_pending_key_line = span.line;
        }
        self.tokens
            .push_back(Token::new(TokenKind::Alias(name), span));
        Ok(())
    }

    // -- Tags --

    fn fetch_tag(&mut self) -> Result<(), ScanError> {
        let span = self.reader.span();
        self.check_property_value_indent(span)?;
        self.note_possible_simple_key(span);
        if self.flow_level == 0 {
            self.property_line = span.line;
        }
        self.reader.advance(); // skip first '!'
        let start = self.reader.offset();

        if self.reader.peek() == '!' {
            // Secondary tag: !!tag
            self.reader.advance();
            while !self.reader.is_eof()
                && !is_whitespace_or_break(self.reader.peek())
                && self.reader.peek() != ','
                && self.reader.peek() != ']'
                && self.reader.peek() != '}'
            {
                self.reader.advance();
            }
            let suffix = self.reader.slice(start, self.reader.offset());
            let tag = format!("!{suffix}");
            self.simple_key_allowed = false;
            self.tokens.push_back(Token::new(TokenKind::Tag(tag), span));
        } else if self.reader.peek() == '<' {
            // Verbatim tag: !<tag>. The body is URI text with no whitespace or
            // line breaks, terminated by `>`. Stopping only at `>`/EOF would let
            // an unterminated `!<foo` swallow the rest of the document, so a
            // break or whitespace inside the body ends the scan and (with a
            // missing `>`) is reported instead of silently consumed.
            self.reader.advance(); // skip '<'
            let inner_start = self.reader.offset();
            while !self.reader.is_eof()
                && self.reader.peek() != '>'
                && !is_whitespace_or_break(self.reader.peek())
            {
                self.reader.advance();
            }
            if self.reader.peek() != '>' {
                return Err(ScanError::new(
                    "unterminated verbatim tag: expected '>'",
                    span,
                ));
            }
            let tag = self
                .reader
                .slice(inner_start, self.reader.offset())
                .to_owned();
            self.reader.advance(); // skip '>'
            if tag.is_empty() {
                return Err(ScanError::new("a verbatim tag must not be empty", span));
            }
            self.simple_key_allowed = false;
            self.tokens
                .push_back(Token::new(TokenKind::Tag(format!("!<{tag}>")), span));
        } else {
            // Primary tag: !tag
            while !self.reader.is_eof()
                && !is_whitespace_or_break(self.reader.peek())
                && self.reader.peek() != ','
                && self.reader.peek() != ']'
                && self.reader.peek() != '}'
            {
                self.reader.advance();
            }
            let suffix = self.reader.slice(start, self.reader.offset());
            let tag = if suffix.is_empty() {
                "!".to_owned()
            } else {
                format!("!{suffix}")
            };
            self.simple_key_allowed = false;
            self.tokens.push_back(Token::new(TokenKind::Tag(tag), span));
        }

        Ok(())
    }

    // -- Scalars --

    fn fetch_single_quoted_scalar(&mut self) -> Result<(), ScanError> {
        let was_simple_key_allowed = self.simple_key_allowed;
        let had_leading_tab = self.tab_before_token;
        let span = self.reader.span();
        let parent_indent = self.quoted_parent_indent();
        let value = scalar::scan_single_quoted(&mut self.reader, parent_indent)?;
        self.finish_quoted_scalar(
            value,
            ScalarStyle::SingleQuoted,
            span,
            was_simple_key_allowed,
            had_leading_tab,
        )
    }

    fn fetch_double_quoted_scalar(&mut self) -> Result<(), ScanError> {
        let was_simple_key_allowed = self.simple_key_allowed;
        let had_leading_tab = self.tab_before_token;
        let span = self.reader.span();
        let parent_indent = self.quoted_parent_indent();
        let value = scalar::scan_double_quoted(&mut self.reader, parent_indent)?;
        self.finish_quoted_scalar(
            value,
            ScalarStyle::DoubleQuoted,
            span,
            was_simple_key_allowed,
            had_leading_tab,
        )
    }

    /// The block indentation a quoted scalar's continuation lines must beat, or
    /// `i32::MIN` inside a flow collection (where the check does not apply).
    fn quoted_parent_indent(&self) -> i32 {
        if self.flow_level == 0 {
            self.current_indent
        } else {
            i32::MIN
        }
    }

    /// Shared tail of the single- and double-quoted scalar fetchers, which differ
    /// only in their scan function and `ScalarStyle`. The reader sits just past
    /// the closing quote. Emits the scalar as a mapping key when a `:` follows it,
    /// otherwise as a plain value.
    fn finish_quoted_scalar(
        &mut self,
        value: Cow<'input, str>,
        style: ScalarStyle,
        span: Span,
        was_simple_key_allowed: bool,
        had_leading_tab: bool,
    ) -> Result<(), ScanError> {
        let content_end = self.reader.offset();
        let end_line = self.reader.line();
        self.simple_key_allowed = false;

        if was_simple_key_allowed
            && !(self.flow_level > 0 && self.flow_explicit_key)
            && self.check_value_after_scalar()
        {
            if self.flow_level == 0 && span.line != end_line {
                return Err(ScanError::new(
                    "an implicit mapping key cannot span multiple lines",
                    span,
                ));
            }
            if self.flow_level == 0 {
                // A tab cannot indent the mapping this quoted key opens, matching
                // the plain-key rule (`a:\n\t"b": c`).
                if had_leading_tab {
                    return Err(tab_indent_error(span));
                }
                let key_col = span.column as i32;
                self.roll_indent(key_col, span, TokenKind::BlockMappingStart)?;
            }
            self.tokens
                .push_back(Token::new(TokenKind::Key { explicit: false }, span));
            let mut scalar = Token::new(TokenKind::Scalar(value, style), span);
            scalar.end_offset = content_end;
            self.tokens.push_back(scalar);
            let val_span = self.reader.span();
            if self.flow_level == 0 {
                self.value_indicator_line = val_span.line;
                self.block_value_pending = true;
            }
            self.reader.advance(); // skip ':'
            if !self.reader.is_eof() && self.reader.peek() == ' ' {
                self.reader.advance();
            }
            self.tokens
                .push_back(Token::new(TokenKind::Value, val_span));
            self.flow_need_sep = false;
            return Ok(());
        }

        self.check_no_block_trailing("quoted scalar")?;
        let mut scalar = Token::new(TokenKind::Scalar(value, style), span);
        scalar.end_offset = content_end;
        self.tokens.push_back(scalar);
        if self.flow_level > 0 {
            self.flow_need_sep = true;
            self.flow_pending_key_line = span.line;
            self.flow_prev_json = true;
        }
        Ok(())
    }

    fn fetch_block_scalar(&mut self, literal: bool) -> Result<(), ScanError> {
        let span = self.reader.span();
        let value = scalar::scan_block(&mut self.reader, literal, self.current_indent)?;
        let style = if literal {
            ScalarStyle::Literal
        } else {
            ScalarStyle::Folded
        };
        self.simple_key_allowed = true;
        // A block scalar is a complete node and can never be an implicit key (a
        // key must fit on one line). A property (tag or anchor) before it noted a
        // speculative simple key at its own token; the block scalar consumes all
        // of its lines in one go, so the per-line clearing in `scan_to_next_token`
        // never runs to retract that note. Clear it here, or the stale entry
        // suppresses the next line's key detection (`- !t |-\n  x\n- !t k: v`).
        self.simple_key = None;
        // Record the block's source extent so the round-trip path can replay it
        // verbatim (folding a `>` block discards the original line breaks).
        let mut token = Token::new(TokenKind::Scalar(value, style), span);
        token.end_offset = self.reader.span().offset;
        self.tokens.push_back(token);
        Ok(())
    }

    fn fetch_plain_scalar(&mut self) -> Result<(), ScanError> {
        let was_simple_key_allowed = self.simple_key_allowed;
        let start_line = self.reader.line();
        let span = self.reader.span();
        // Captured before scanning: whether a tab preceded this scalar. It only
        // matters if the scalar turns out to be a mapping key (a tab cannot
        // indent the mapping); a plain scalar value after a tab is fine.
        let had_leading_tab = self.tab_before_token;
        let (value, content_end) =
            scalar::scan_plain(&mut self.reader, self.flow_level > 0, self.current_indent)?;

        // scan_plain may consume line breaks. If we crossed a line boundary
        // in block context, a new simple key is allowed on the next line.
        self.simple_key_allowed = self.flow_level == 0 && self.reader.line() > start_line;

        // scan_plain can swallow the line break while folding/checking the next
        // line, bypassing the newline handler in scan_to_next_token. A pending
        // simple key from a now-past line can no longer complete, so drop it
        // here (`k:\n  !!str bar\n&a z: v`).
        if self.flow_level == 0
            && self
                .simple_key
                .is_some_and(|(_, _, line, _)| line != self.reader.line())
        {
            self.simple_key = None;
        }

        // An explicit `?` flow key already emitted its `Key` marker; this scalar
        // is that key's content, not a fresh implicit key, so do not re-key it.
        let explicit_key_pending = self.flow_level > 0 && self.flow_explicit_key;
        if was_simple_key_allowed
            && !explicit_key_pending
            && !self.reader.is_eof()
            && self.reader.peek() == ':'
        {
            let next = self.reader.peek_next();
            // A `:` on a later line that is less indented than this scalar is
            // the value indicator of an enclosing construct (e.g. an explicit
            // `?` key whose block content ends with this scalar), not this
            // scalar's own key indicator. Leave it to the outer level rather
            // than reading the scalar as a multi-line implicit key.
            let shallow_colon = self.flow_level == 0
                && self.reader.line() > start_line
                && (self.reader.column() as i32) < span.column as i32;
            if !shallow_colon
                && (next.map_or(true, is_whitespace_or_break)
                    || (self.flow_level > 0 && next.is_some_and(is_flow_indicator)))
            {
                if self.flow_level == 0 && self.reader.line() > start_line {
                    return Err(ScanError::new(
                        "an implicit mapping key cannot span multiple lines",
                        span,
                    ));
                }
                // Inside a flow sequence, a single-pair key and its `:` must
                // share a line (`[ key\n : value ]` is invalid); a flow mapping
                // allows the `:` on a following line.
                if self.flow_level > 0
                    && !self.flow_explicit_key
                    && self.flow_kinds.last() == Some(&false)
                    && self.reader.line() != start_line
                {
                    return Err(ScanError::new(
                        "the implicit key of a flow sequence pair must be on one line",
                        span,
                    ));
                }
                if self.flow_level == 0 {
                    // A tab cannot indent the mapping this key opens, so a key
                    // reached past a leading tab (`foo:\n \tbar: 1`) is invalid,
                    // unlike the same scalar used as a plain value (`foo:\n \tbar`).
                    if had_leading_tab {
                        return Err(tab_indent_error(span));
                    }
                    let key_col = span.column as i32;
                    self.roll_indent(key_col, span, TokenKind::BlockMappingStart)?;
                }
                self.tokens
                    .push_back(Token::new(TokenKind::Key { explicit: false }, span));
                let mut scalar = Token::new(TokenKind::Scalar(value, ScalarStyle::Plain), span);
                scalar.end_offset = content_end;
                self.tokens.push_back(scalar);
                let val_span = self.reader.span();
                if self.flow_level == 0 {
                    self.value_indicator_line = val_span.line;
                    self.block_value_pending = true;
                }
                self.reader.advance(); // skip ':'
                if !self.reader.is_eof() && self.reader.peek() == ' ' {
                    self.reader.advance();
                }
                self.tokens
                    .push_back(Token::new(TokenKind::Value, val_span));
                // The `:` just emitted introduces a value; no separator owed.
                self.flow_need_sep = false;
                return Ok(());
            }
        }

        // In a flow collection, a plain value that folds across a line break and
        // then meets a `:` was really a new mapping key with no separating comma
        // before it (`{ foo: 1\n bar: 2 }`). An explicit `?` key is exempt: its
        // multi-line content legitimately precedes the `:` value indicator.
        if self.flow_level > 0
            && !explicit_key_pending
            && self.reader.line() > start_line
            && !self.reader.is_eof()
            && self.reader.peek() == ':'
            && self
                .reader
                .peek_next()
                .map_or(true, |c| is_whitespace_or_break(c) || is_flow_indicator(c))
        {
            return Err(ScanError::new(
                "missing ',' between flow collection entries",
                span,
            ));
        }

        let mut scalar = Token::new(TokenKind::Scalar(value, ScalarStyle::Plain), span);
        scalar.end_offset = content_end;
        self.tokens.push_back(scalar);
        // A completed plain value owes a separator before the next flow node.
        if self.flow_level > 0 {
            self.flow_need_sep = true;
            self.flow_pending_key_line = span.line;
        }
        Ok(())
    }
}

/// Style of a YAML scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarStyle {
    Plain,
    SingleQuoted,
    DoubleQuoted,
    Literal,
    Folded,
}

impl ScalarStyle {
    /// A stable, lowercase name for the style, used wherever the style is exposed
    /// to Python (the round-trip `YAMLRocksNode.style` and the annotated `__style__`). The
    /// two block styles are `"literal"` (`|`) and `"folded"` (`>`).
    pub fn name(self) -> &'static str {
        match self {
            ScalarStyle::Plain => "plain",
            ScalarStyle::SingleQuoted => "single",
            ScalarStyle::DoubleQuoted => "double",
            ScalarStyle::Literal => "literal",
            ScalarStyle::Folded => "folded",
        }
    }
}

/// Error during scanning.
#[derive(Debug, Clone)]
pub struct ScanError {
    pub message: String,
    pub span: Span,
}

impl ScanError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl std::fmt::Display for ScanError {
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

impl std::error::Error for ScanError {}

/// Build the error for a reserved indicator (`@` or `` ` ``) found where a plain
/// scalar would start. The YAML spec reserves these two characters; a value that
/// needs to begin with one must be quoted.
fn reserved_indicator_error(ch: char, span: Span) -> ScanError {
    ScanError::new(
        format!("'{ch}' is a reserved indicator and cannot start a plain scalar"),
        span,
    )
}

/// The error for a tab where a block collection node expects indentation. YAML
/// indents with spaces only, so a `-` entry or a mapping key reached past a tab
/// is invalid (a tab before a plain or flow scalar is valid separation, though).
fn tab_indent_error(span: Span) -> ScanError {
    ScanError::new("a tab cannot be used for indentation", span)
}
