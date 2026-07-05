use std::borrow::Cow;

use super::char_traits::{is_blank_byte, is_flow_indicator, is_whitespace_or_break};
use super::reader::Reader;
use super::ScanError;

/// Whether the reader sits at the start of a line that is a `---` or `...`
/// document marker (column 0, the three characters, then whitespace/break/EOF).
/// Such a line cannot appear inside a quoted scalar; the quote is unterminated.
fn at_document_marker(reader: &Reader) -> bool {
    reader.column() == 0
        && (reader.check_ahead("---") || reader.check_ahead("..."))
        && reader.peek_at(3).map_or(true, is_whitespace_or_break)
}

/// Scan a single-quoted scalar: 'value'
///
/// `parent_indent` is the indentation of the surrounding block context (or
/// `i32::MIN` when inside a flow collection, which disables the check). In
/// block context every continuation line of a multi-line quoted scalar must be
/// indented strictly past `parent_indent`; a line that is not is malformed.
pub fn scan_single_quoted<'input>(
    reader: &mut Reader<'input>,
    parent_indent: i32,
) -> Result<Cow<'input, str>, ScanError> {
    let start_span = reader.span();
    reader.advance(); // skip opening '

    // Scan by tracking offsets, not by pushing each char: a single-quoted scalar
    // with no `''` escape and no line fold is exactly the input slice between the
    // quotes, so it borrows with zero allocation (the common case). An owned
    // buffer is materialized lazily the first time an escape or fold forces a
    // transformation, and thereafter each clean run is bulk-copied with
    // `push_str` rather than character by character. This mirrors `scan_plain`.
    let content_start = reader.offset();
    // `Some` once a `''` escape or a fold has forced an owned buffer; the start of
    // the current clean run (into which nothing has been copied yet) is `run_start`.
    let mut owned: Option<String> = None;
    let mut run_start = content_start;

    loop {
        if reader.is_eof() {
            return Err(ScanError::new(
                "unterminated single-quoted scalar",
                start_span,
            ));
        }
        match reader.peek() {
            '\'' => {
                let quote_off = reader.offset();
                reader.advance();
                if reader.peek() == '\'' {
                    // Escaped `''`: flush the clean run, emit one quote, continue.
                    let buf = owned.get_or_insert_with(String::new);
                    buf.push_str(reader.slice(run_start, quote_off));
                    buf.push('\'');
                    reader.advance();
                    run_start = reader.offset();
                } else {
                    // Closing quote at `quote_off`.
                    return Ok(match owned {
                        Some(mut buf) => {
                            buf.push_str(reader.slice(run_start, quote_off));
                            Cow::Owned(buf)
                        }
                        None => Cow::Borrowed(reader.slice(content_start, quote_off)),
                    });
                }
            }
            '\n' | '\r' => {
                // A fold forces an owned buffer. Copy the clean run, drop its
                // trailing blanks (single-quoted scalars have no escapes, so every
                // trailing blank is foldable whitespace), then fold the break.
                let here = reader.offset();
                let buf = owned.get_or_insert_with(String::new);
                buf.push_str(reader.slice(run_start, here));
                trim_trailing_blanks(buf);
                fold_quoted_break(reader, buf, parent_indent)?;
                // A document marker (`---`/`...`) can only begin at column 0, so it
                // is checked once here, after the fold lands on a continuation line,
                // rather than on every character.
                if at_document_marker(reader) {
                    return Err(ScanError::new(
                        "a document marker cannot appear inside a quoted scalar",
                        start_span,
                    ));
                }
                run_start = reader.offset();
            }
            _ => {
                reader.advance();
            }
        }
    }
}

/// After advancing to the content of a quoted-scalar continuation line (leading
/// spaces already skipped), reject it when it sits at or before the surrounding
/// block indentation. The YAML spec requires a flow scalar's continuation lines
/// to be indented *past* the block (so the test-suite case `QB6E`, a top-level
/// `key: "a\nb\nc"` with continuations at column 0, is an error). PyYAML is
/// lenient and accepts these; we stay spec-strict, and real configs that rely on
/// the lenient reading are recorded as known PyYAML-leniency cases in the
/// real-world suite. Blank lines and the EOF case are tolerated.
fn check_quoted_continuation_indent(reader: &Reader, parent_indent: i32) -> Result<(), ScanError> {
    if reader.is_eof() {
        return Ok(());
    }
    let ch = reader.peek();
    if ch == '\n' || ch == '\r' {
        return Ok(());
    }
    if (reader.column() as i32) <= parent_indent {
        return Err(ScanError::new(
            "a multi-line quoted scalar must be indented past its block context",
            reader.span(),
        ));
    }
    Ok(())
}

/// Fold a line break inside a quoted scalar: collapse a lone break to a space,
/// or each blank line beyond the first to a newline. Rejects a continuation line
/// not indented past the block context. Shared by the single- and double-quoted
/// scanners.
///
/// The caller is responsible for dropping the line's trailing blanks first.
/// Single-quoted scalars trim with [`trim_trailing_blanks`]; double-quoted
/// scalars truncate to the last non-blank boundary so an *escaped* trailing tab
/// or space (which is content, not foldable whitespace) survives the fold.
fn fold_quoted_break(
    reader: &mut Reader<'_>,
    value: &mut String,
    parent_indent: i32,
) -> Result<(), ScanError> {
    reader.advance_line();
    let mut blank_lines = 0;
    loop {
        if reader.is_eof() {
            break;
        }
        // Indentation is leading spaces only; a tab is never indentation, so skip
        // just the spaces here and let the indent check see the true column. A
        // tab that follows is content separation, consumed after the check.
        while reader.peek() == ' ' {
            reader.advance();
        }
        // A line is blank when only whitespace precedes its break; a tab may pad
        // such a line but cannot indent a content line.
        let blank = match reader.peek() {
            '\n' | '\r' => true,
            '\t' => matches!(reader.peek_after_blanks(), Some('\n' | '\r')),
            _ => false,
        };
        if !blank {
            break;
        }
        blank_lines += 1;
        skip_blanks(reader);
        if reader.is_eof() {
            break;
        }
        reader.advance_line();
    }
    check_quoted_continuation_indent(reader, parent_indent)?;
    skip_blanks(reader);
    if blank_lines > 0 {
        for _ in 0..blank_lines {
            value.push('\n');
        }
    } else {
        value.push(' ');
    }
    Ok(())
}

/// Append a run of ordinary (unescaped, single-line) double-quoted content to
/// `buf`, advancing `nonblank_len` to just past the run's last non-blank byte. A
/// literal space or tab is foldable whitespace and does not move the boundary,
/// so this reproduces exactly what the old per-character bookkeeping produced.
fn push_dquoted_run(buf: &mut String, nonblank_len: &mut usize, run: &str) {
    let base = buf.len();
    buf.push_str(run);
    if let Some(i) = run.rfind(|c: char| c != ' ' && c != '\t') {
        let ch_len = run[i..].chars().next().unwrap().len_utf8();
        *nonblank_len = base + i + ch_len;
    }
}

/// Scan a double-quoted scalar: "value"
///
/// `parent_indent` carries the same meaning as in [`scan_single_quoted`]:
/// continuation lines in block context must be indented past it.
pub fn scan_double_quoted<'input>(
    reader: &mut Reader<'input>,
    parent_indent: i32,
) -> Result<Cow<'input, str>, ScanError> {
    let start_span = reader.span();
    reader.advance(); // skip opening "

    // Like `scan_single_quoted`, scan by offset and borrow the input slice when
    // the scalar is clean: no `\` escape and no line fold means the content is
    // exactly the bytes between the quotes. An owned buffer is materialized only
    // once an escape or fold forces a transformation, and clean runs are then
    // bulk-copied rather than pushed char by char.
    let content_start = reader.offset();
    let mut owned: Option<String> = None;
    let mut run_start = content_start;
    // Length of the owned buffer up to the last non-blank (or escaped-blank)
    // character. Folding a line break strips the line's *literal* trailing spaces
    // and tabs, but an escaped `\t`/`\<TAB>`/escaped space is content and must
    // survive; tracking the boundary lets the fold truncate to it. Meaningful
    // only once `owned` is set (a fold or escape cannot happen before that).
    let mut nonblank_len = 0usize;
    loop {
        if reader.is_eof() {
            return Err(ScanError::new(
                "unterminated double-quoted scalar",
                start_span,
            ));
        }
        match reader.peek() {
            '"' => {
                let quote_off = reader.offset();
                reader.advance();
                return Ok(match owned {
                    Some(mut buf) => {
                        push_dquoted_run(
                            &mut buf,
                            &mut nonblank_len,
                            reader.slice(run_start, quote_off),
                        );
                        Cow::Owned(buf)
                    }
                    None => Cow::Borrowed(reader.slice(content_start, quote_off)),
                });
            }
            '\\' => {
                // An escape forces an owned buffer; flush the clean run first.
                let esc_off = reader.offset();
                let buf = owned.get_or_insert_with(String::new);
                push_dquoted_run(buf, &mut nonblank_len, reader.slice(run_start, esc_off));
                reader.advance();
                if reader.is_eof() {
                    return Err(ScanError::new("unterminated escape sequence", start_span));
                }
                match reader.peek() {
                    '0' => {
                        buf.push('\0');
                        reader.advance();
                    }
                    'a' => {
                        buf.push('\x07');
                        reader.advance();
                    }
                    'b' => {
                        buf.push('\x08');
                        reader.advance();
                    }
                    't' | '\t' => {
                        // `\t` and a backslash before a literal tab are both the
                        // escaped horizontal tab (`ns-esc-horizontal-tab`, x09).
                        buf.push('\t');
                        reader.advance();
                    }
                    'n' => {
                        buf.push('\n');
                        reader.advance();
                    }
                    'v' => {
                        buf.push('\x0B');
                        reader.advance();
                    }
                    'f' => {
                        buf.push('\x0C');
                        reader.advance();
                    }
                    'r' => {
                        buf.push('\r');
                        reader.advance();
                    }
                    'e' => {
                        buf.push('\x1B');
                        reader.advance();
                    }
                    ' ' => {
                        buf.push(' ');
                        reader.advance();
                    }
                    '"' => {
                        buf.push('"');
                        reader.advance();
                    }
                    '/' => {
                        buf.push('/');
                        reader.advance();
                    }
                    '\\' => {
                        buf.push('\\');
                        reader.advance();
                    }
                    'N' => {
                        buf.push('\u{0085}');
                        reader.advance();
                    }
                    '_' => {
                        buf.push('\u{00A0}');
                        reader.advance();
                    }
                    'L' => {
                        buf.push('\u{2028}');
                        reader.advance();
                    }
                    'P' => {
                        buf.push('\u{2029}');
                        reader.advance();
                    }
                    'x' => {
                        reader.advance();
                        let ch = scan_unicode_escape(reader, 2, start_span)?;
                        buf.push(ch);
                    }
                    'u' => {
                        reader.advance();
                        let ch = scan_unicode_escape(reader, 4, start_span)?;
                        buf.push(ch);
                    }
                    'U' => {
                        reader.advance();
                        let ch = scan_unicode_escape(reader, 8, start_span)?;
                        buf.push(ch);
                    }
                    '\n' | '\r' => {
                        // Line continuation
                        reader.advance_line();
                        skip_spaces(reader);
                        // Skip blank lines
                        while !reader.is_eof() && (reader.peek() == '\n' || reader.peek() == '\r') {
                            buf.push('\n');
                            reader.advance_line();
                            skip_spaces(reader);
                        }
                    }
                    other => {
                        return Err(ScanError::new(
                            format!("unknown escape character: '\\{other}'"),
                            reader.span(),
                        ));
                    }
                }
                // Escaped output is content, never foldable whitespace, even
                // when it is a tab or space: protect it from the next fold.
                nonblank_len = buf.len();
                run_start = reader.offset();
            }
            '\n' | '\r' => {
                // A fold forces an owned buffer. Copy the clean run, strip the
                // line's literal trailing blanks (everything past the last
                // non-blank boundary, keeping any escaped blanks), then fold.
                let here = reader.offset();
                let buf = owned.get_or_insert_with(String::new);
                push_dquoted_run(buf, &mut nonblank_len, reader.slice(run_start, here));
                buf.truncate(nonblank_len);
                fold_quoted_break(reader, buf, parent_indent)?;
                // A document marker (`---`/`...`) can only begin at column 0, so
                // it is checked once here, after the fold lands on a continuation
                // line, rather than on every character.
                if at_document_marker(reader) {
                    return Err(ScanError::new(
                        "a document marker cannot appear inside a quoted scalar",
                        start_span,
                    ));
                }
                run_start = reader.offset();
            }
            _ => {
                reader.advance();
            }
        }
    }
}

fn scan_unicode_escape(
    reader: &mut Reader<'_>,
    length: usize,
    start_span: super::token::Span,
) -> Result<char, ScanError> {
    let mut code: u32 = 0;
    for _ in 0..length {
        if reader.is_eof() {
            return Err(ScanError::new("incomplete unicode escape", start_span));
        }
        let ch = reader.peek();
        let digit = match ch {
            '0'..='9' => ch as u32 - '0' as u32,
            'a'..='f' => ch as u32 - 'a' as u32 + 10,
            'A'..='F' => ch as u32 - 'A' as u32 + 10,
            _ => {
                return Err(ScanError::new(
                    format!("invalid hex digit in unicode escape: '{ch}'"),
                    reader.span(),
                ))
            }
        };
        code = code * 16 + digit;
        reader.advance();
    }
    char::from_u32(code).ok_or_else(|| {
        ScanError::new(
            format!("invalid unicode code point: U+{code:04X}"),
            start_span,
        )
    })
}

/// Scan a block scalar (literal `|` or folded `>`).
pub fn scan_block<'input>(
    reader: &mut Reader<'input>,
    literal: bool,
    parent_indent: i32,
) -> Result<Cow<'input, str>, ScanError> {
    let start_span = reader.span();
    reader.advance(); // skip '|' or '>'

    // Parse the optional chomping (`+`/`-`) and indentation (`1`-`9`) indicators.
    // They follow the `|`/`>` immediately, in either order, each at most once,
    // with no whitespace before or between them (YAML 8.1.1). Anything else ends
    // this phase; a repeated or stray indicator is then caught as unexpected
    // header content below.
    let mut chomping = Chomping::Clip; // default
    let mut seen_chomping = false;
    let mut explicit_indent: Option<u32> = None;

    while !reader.is_eof() {
        match reader.peek() {
            '-' if !seen_chomping => {
                chomping = Chomping::Strip;
                seen_chomping = true;
                reader.advance();
            }
            '+' if !seen_chomping => {
                chomping = Chomping::Keep;
                seen_chomping = true;
                reader.advance();
            }
            '1'..='9' if explicit_indent.is_none() => {
                explicit_indent = Some(reader.peek() as u32 - '0' as u32);
                reader.advance();
            }
            _ => break,
        }
    }

    // After the indicators, only whitespace, an optional comment, and the line
    // break may remain on the header line. A repeated indicator, a second
    // indentation digit (`|12`), a space-separated indicator (`| 2`), or any
    // other text lands here and is rejected.
    loop {
        if reader.is_eof() {
            break;
        }
        match reader.peek() {
            ' ' | '\t' => {
                reader.advance();
            }
            '#' if reader.prev_is_whitespace_or_start() => {
                // Comment after block scalar header.
                while !reader.is_eof() && reader.peek() != '\n' && reader.peek() != '\r' {
                    reader.advance();
                }
                break;
            }
            '\n' | '\r' => break,
            other => {
                let what = if other == '#' {
                    "a comment must be preceded by whitespace"
                } else {
                    "unexpected content after block scalar indicator"
                };
                return Err(ScanError::new(what, reader.span()));
            }
        }
    }

    // Skip the header line break
    if !reader.is_eof() && (reader.peek() == '\n' || reader.peek() == '\r') {
        reader.advance_line();
    }

    // Determine the indentation of the block content
    let indent = if let Some(explicit) = explicit_indent {
        // The indentation indicator counts spaces relative to the parent node,
        // so a nested `aaa: |2` resolves against the mapping's own indent.
        parent_indent.max(0) as u32 + explicit
    } else {
        // Auto-detect from first non-empty line, tracking the deepest leading
        // empty line so we can enforce that none is indented more than the
        // content (YAML 8.1.1.1).
        let saved = reader.save();
        let mut detected = 0u32;
        let mut max_leading_blank = 0u32;

        loop {
            if reader.is_eof() {
                break;
            }
            let mut spaces = 0u32;
            while !reader.is_eof() && reader.peek() == ' ' {
                spaces += 1;
                reader.advance();
            }
            if reader.is_eof() || reader.peek() == '\n' || reader.peek() == '\r' {
                if !reader.is_eof() {
                    reader.advance_line();
                }
                max_leading_blank = max_leading_blank.max(spaces);
                continue;
            }
            detected = spaces;
            break;
        }

        reader.restore(saved);

        // Content must be indented more than the block scalar's parent node; at
        // or below that, the scalar is empty. A document-level scalar (`--- |`)
        // has parent indent -1, so column-0 content is valid.
        if (detected as i32) <= parent_indent {
            // No content line is indented past the parent: the scalar is empty.
            // Its lines are all empty lines (no content even when over-indented),
            // so the value is empty and only `keep` retains a feed per line.
            return Ok(Cow::Owned(scan_empty_block(reader, &chomping)?));
        }
        if max_leading_blank > detected {
            return Err(ScanError::new(
                "a leading empty line in a block scalar is indented more than the content",
                start_span,
            ));
        }
        detected
    };

    // Read the block content line by line. `breaks` carries the empty-line
    // feeds owed before the next content line (seeded with any leading empties);
    // `trailing` records the trailing feeds available to `keep` chomping once
    // the block ends. A line break folds to a space only in folded mode between
    // two plain (non-blank, non-more-indented) lines with no empty line between.
    let mut value = String::new();
    let mut breaks = scan_block_breaks(reader, indent);
    let mut trailing = breaks;

    while reader.column() == indent && !reader.is_eof() && !at_document_marker(reader) {
        for _ in 0..breaks {
            value.push('\n');
        }
        let leading_non_space = !matches!(reader.peek(), ' ' | '\t');
        while !reader.is_eof() && reader.peek() != '\n' && reader.peek() != '\r' {
            value.push(reader.peek());
            reader.advance();
        }
        let had_break = !reader.is_eof();
        if had_break {
            reader.advance_line();
        }
        breaks = scan_block_breaks(reader, indent);
        let in_block = reader.column() == indent && !reader.is_eof() && !at_document_marker(reader);
        if in_block {
            if !literal && had_break && leading_non_space && !matches!(reader.peek(), ' ' | '\t') {
                // A foldable break collapses to a space, unless empty lines
                // follow, then it is absorbed and the empties become feeds.
                if breaks == 0 {
                    value.push(' ');
                }
            } else {
                value.push('\n');
            }
        } else {
            trailing = u32::from(had_break) + breaks;
            break;
        }
    }

    match chomping {
        Chomping::Strip => {}
        Chomping::Clip => {
            if !value.is_empty() {
                value.push('\n');
            }
        }
        Chomping::Keep => {
            for _ in 0..trailing {
                value.push('\n');
            }
        }
    }

    Ok(Cow::Owned(value))
}

/// Block scalar chomping mode.
enum Chomping {
    Strip, // -
    Clip,  // default
    Keep,  // +
}

/// Scan a plain (unquoted) scalar.
// The break-out conditions below are intentionally separate `else if` arms for
// readability even though several share a `break` body.
#[allow(clippy::if_same_then_else)]
/// Returns the scalar value and the byte offset just past its last content
/// character (the source end). The reader may advance past that point while
/// folding trailing line breaks, so the returned `content_end` (not the reader's
/// final position) is the scalar's true source extent.
pub fn scan_plain<'input>(
    reader: &mut Reader<'input>,
    in_flow: bool,
    current_indent: i32,
) -> Result<(Cow<'input, str>, usize), ScanError> {
    // A single-line plain scalar is exactly the input slice `[start..content_end]`;
    // its internal spaces are contiguous in the input, and the only thing that
    // transforms the text is line folding across a newline. So scan the first
    // line by tracking offsets only (no allocation), and materialize an owned
    // buffer lazily the moment we commit to folding onto another line.
    let start = reader.offset();
    let mut content_end = start;
    // Whether the run of whitespace immediately before the cursor is non-empty
    // (used for the `#`-must-be-preceded-by-space rule on the first line).
    let mut prev_was_space = false;
    // `Some` once folding has begun; thereafter content is appended here.
    let mut owned: Option<String> = None;
    // Fold whitespace pending insertion, used only after `owned` is set.
    let mut spaces = String::new();

    let has_content = |owned: &Option<String>, content_end: usize| match owned {
        Some(s) => !s.is_empty(),
        None => content_end > start,
    };

    loop {
        if reader.is_eof() {
            break;
        }

        let ch = reader.peek();

        // End conditions
        if ch == ':' {
            let next = reader.peek_next();
            if next.map_or(true, |c| {
                is_whitespace_or_break(c) || (in_flow && is_flow_indicator(c))
            }) {
                break;
            }
        }

        if ch == '#' && has_content(&owned, content_end) {
            // A `#` only starts a comment when preceded by whitespace.
            let preceded_by_space = match &owned {
                Some(_) => spaces.ends_with(' ') || spaces.ends_with('\t'),
                None => prev_was_space,
            };
            if preceded_by_space {
                break;
            }
        }

        if in_flow && is_flow_indicator(ch) {
            break;
        }

        if ch == '\n' || ch == '\r' {
            reader.advance_line();
            let mut blank_lines = 0;

            // Skip leading blanks (spaces and tabs) and count blank lines; a
            // tab-only line folds like any other empty line.
            loop {
                skip_blanks(reader);
                if reader.is_eof() {
                    break;
                }
                if reader.peek() == '\n' || reader.peek() == '\r' {
                    blank_lines += 1;
                    reader.advance_line();
                } else {
                    break;
                }
            }

            // Check indentation: if less than or equal to current indent, stop
            if !in_flow && (reader.column() as i32) <= current_indent {
                break;
            }

            if reader.is_eof() {
                break;
            }

            // A document marker line (`---` or `...`) ends the plain scalar; it
            // is a structural boundary, not foldable content.
            if !in_flow
                && reader.column() == 0
                && (reader.check_ahead("---") || reader.check_ahead("..."))
                && reader.peek_at(3).map_or(true, is_whitespace_or_break)
            {
                break;
            }

            // Check if next content starts a new structure
            let next_ch = reader.peek();
            if matches!(
                next_ch,
                '?' | ':'
                    | '#'
                    | '&'
                    | '*'
                    | '!'
                    | '|'
                    | '>'
                    | '\''
                    | '"'
                    | '%'
                    | '@'
                    | '`'
                    | '{'
                    | '['
            ) {
                // Could be a new node; check more carefully.
                //
                // A continuation line starting with `-` is NOT handled here: we
                // only reach this point indented past the block (`column >
                // current_indent`), and a `---`/`...` document marker is one only
                // at column 0 (handled above). An *indented* `-` or `---` is
                // ordinary plain content that folds, e.g. a CRD description whose
                // line wraps as `... CamelCase.\n  --- Many .type values ...`.
                if next_ch == '#' {
                    break;
                } else if matches!(next_ch, '?' | ':')
                    && reader.peek_next().map_or(true, is_whitespace_or_break)
                {
                    break;
                } else if in_flow && matches!(next_ch, '{' | '[') {
                    // In FLOW context only a `{`/`[` continuation line ends the
                    // scalar: these are the flow indicators that open a nested
                    // collection. `'`/`"`/`|`/`>` are *not* flow indicators, so
                    // (like in block context) they are ordinary plain characters
                    // on a continuation line and fold into the scalar, matching
                    // PyYAML and the spec (`[a\n  "b]` is the one scalar `a "b`).
                    //
                    // The first-character restriction (`|`/`>`/`{`/`[`/`'`/`"`
                    // cannot *start* a plain scalar) applies only to the scalar's
                    // start, never its continuation lines.
                    break;
                }
            }

            // Committed to folding onto another line: materialize the owned
            // buffer from the slice scanned so far, then record the fold
            // whitespace for the next content character to flush.
            owned.get_or_insert_with(|| reader.slice(start, content_end).to_owned());
            spaces.clear();
            if blank_lines > 0 {
                for _ in 0..blank_lines {
                    spaces.push('\n');
                }
            } else {
                spaces.push(' ');
            }
            continue;
        }

        if ch == ' ' || ch == '\t' {
            if owned.is_some() {
                spaces.push(ch);
            }
            prev_was_space = true;
            reader.advance();
            continue;
        }

        // Regular content. Bulk-consume a run of ordinary ASCII content in one
        // step, stopping before any byte the branches above handle (a possible
        // terminator `:`/`#`, a line break, a flow indicator) or any non-ASCII
        // byte. A `:`/`#` that reached here is content (it failed the end
        // conditions), so consume it singly below; a non-ASCII char makes the run
        // empty and also falls through.
        if !matches!(ch, ':' | '#') {
            if let Some(buf) = owned.as_mut() {
                // Folding has begun: thread content through the owned buffer,
                // flushing any pending fold whitespace first.
                let run = reader.take_plain_run(in_flow);
                if !run.is_empty() {
                    if !spaces.is_empty() {
                        buf.push_str(&spaces);
                        spaces.clear();
                    }
                    buf.push_str(run);
                    prev_was_space = false;
                    content_end = reader.offset();
                    continue;
                }
            } else {
                // Single-line fast path: take the whole run *including internal
                // blanks* in one pass (`a b c`), where a blank-stopping run would
                // restart per word. The reader advances past trailing blanks to
                // the stopping byte, but `content_end` excludes them.
                let (run, run_content_end) = reader.take_plain_line_run(in_flow);
                if !run.is_empty() {
                    content_end = run_content_end;
                    // The `#`-must-follow-a-blank rule reads this on the next byte.
                    prev_was_space = run_content_end < reader.offset();
                    continue;
                }
            }
        }

        // A single content character (a content `:`/`#`, or the multi-byte char
        // the bulk run stopped at).
        if let Some(buf) = owned.as_mut() {
            if !spaces.is_empty() {
                buf.push_str(&spaces);
                spaces.clear();
            }
            buf.push(ch);
        }
        prev_was_space = false;
        reader.advance();
        content_end = reader.offset();
    }

    let value = match owned {
        Some(s) => Cow::Owned(s),
        None => Cow::Borrowed(reader.slice(start, content_end)),
    };
    Ok((value, content_end))
}

fn skip_spaces(reader: &mut Reader<'_>) {
    while !reader.is_eof() && reader.peek() == ' ' {
        reader.advance();
    }
}

/// Skip a run of inline blanks (spaces and tabs). Line folding strips the
/// whitespace on both sides of a break, and a tab-only line counts as blank.
fn skip_blanks(reader: &mut Reader<'_>) {
    while !reader.is_eof() && matches!(reader.peek(), ' ' | '\t') {
        reader.advance();
    }
}

/// Drop trailing inline blanks (spaces and tabs) from a folded scalar buffer.
/// Whitespace at the end of a line is stripped before the break is folded.
fn trim_trailing_blanks(value: &mut String) {
    while value.as_bytes().last().is_some_and(|&b| is_blank_byte(b)) {
        value.pop();
    }
}

/// Scan a block scalar with no content line: every line is empty. Such lines are
/// not content even when indented past the block, so the value is empty; only
/// `keep` chomping retains one feed per empty line. A block scalar is indented
/// with spaces, so a tab where that indentation belongs is rejected (the empty
/// line `\t` in `foo: |\n\t\nbar: 1`). Leaves the reader at the first non-blank
/// line (a sibling or dedent), which the caller's scanner reads next.
fn scan_empty_block(reader: &mut Reader<'_>, chomping: &Chomping) -> Result<String, ScanError> {
    let mut feeds = 0u32;
    while !reader.is_eof() {
        // A line is empty when only whitespace precedes its break (or EOF). A
        // non-blank line is the next sibling: leave it untouched and stop.
        let next = reader.peek_after_blanks();
        if !(next.is_none() || matches!(next, Some('\n' | '\r'))) {
            break;
        }
        while reader.peek() == ' ' {
            reader.advance();
        }
        if reader.peek() == '\t' {
            return Err(ScanError::new(
                "a tab cannot be used for indentation",
                reader.span(),
            ));
        }
        feeds += 1;
        if reader.is_eof() {
            break;
        }
        reader.advance_line();
    }
    let mut value = String::new();
    if matches!(chomping, Chomping::Keep) {
        for _ in 0..feeds {
            value.push('\n');
        }
    }
    Ok(value)
}

/// Consume empty lines of a block scalar, returning how many were skipped.
///
/// Skips up to `indent` leading spaces, then each following all-blank line (a
/// line break with no more than `indent` spaces). Leaves the reader at the next
/// content line, a dedent, or EOF. A line with more than `indent` spaces is
/// content (trailing whitespace), so it is left in place.
fn scan_block_breaks(reader: &mut Reader<'_>, indent: u32) -> u32 {
    let mut breaks = 0;
    while reader.column() < indent && reader.peek() == ' ' {
        reader.advance();
    }
    while !reader.is_eof() && matches!(reader.peek(), '\n' | '\r') {
        reader.advance_line();
        breaks += 1;
        while reader.column() < indent && reader.peek() == ' ' {
            reader.advance();
        }
    }
    breaks
}

#[cfg(test)]
mod tests {
    use crate::scanner::{ScalarStyle, Scanner, TokenKind};

    /// Collect every scalar token's (value, style) from a source string.
    fn scalars(src: &str) -> Vec<(String, ScalarStyle)> {
        let mut scanner = Scanner::new(src);
        let mut out = Vec::new();
        loop {
            match scanner.next_token().expect("scans cleanly") {
                token if matches!(token.kind, TokenKind::StreamEnd) => break,
                token => {
                    if let TokenKind::Scalar(value, style) = token.kind {
                        out.push((value.into_owned(), style));
                    }
                }
            }
        }
        out
    }

    fn first(src: &str) -> (String, ScalarStyle) {
        scalars(src).into_iter().next().expect("a scalar")
    }

    fn errors(src: &str) -> bool {
        let mut scanner = Scanner::new(src);
        loop {
            match scanner.next_token() {
                Ok(t) if matches!(t.kind, TokenKind::StreamEnd) => return false,
                Ok(_) => {}
                Err(_) => return true,
            }
        }
    }

    #[test]
    fn plain_scalar() {
        assert_eq!(first("hello\n"), ("hello".to_owned(), ScalarStyle::Plain));
    }

    #[test]
    fn single_quoted_doubles_the_quote_to_escape() {
        let (value, style) = first("'it''s here'\n");
        assert_eq!(value, "it's here");
        assert_eq!(style, ScalarStyle::SingleQuoted);
    }

    #[test]
    fn double_quoted_backslash_escapes() {
        assert_eq!(first("\"a\\nb\\tc\"\n").0, "a\nb\tc");
    }

    #[test]
    fn double_quoted_unicode_escapes() {
        assert_eq!(first("\"\\u0041\\u00e9\"\n").0, "Aé");
    }

    #[test]
    fn literal_block_keeps_newlines() {
        let (value, style) = first("|\n  line1\n  line2\n");
        assert_eq!(value, "line1\nline2\n");
        assert_eq!(style, ScalarStyle::Literal);
    }

    #[test]
    fn folded_block_joins_with_spaces() {
        let (value, style) = first(">\n  a\n  b\n");
        assert_eq!(value, "a b\n");
        assert_eq!(style, ScalarStyle::Folded);
    }

    #[test]
    fn block_strip_chomping_drops_trailing_newline() {
        assert_eq!(first("|-\n  text\n").0, "text");
    }

    #[test]
    fn block_keep_chomping_retains_trailing_newlines() {
        assert_eq!(first("|+\n  text\n\n").0, "text\n\n");
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(errors("'oops"));
        assert!(errors("\"oops"));
    }

    #[test]
    fn invalid_escape_is_an_error() {
        assert!(errors("\"bad \\q escape\"\n"));
    }

    #[test]
    fn malformed_block_scalar_header_is_an_error() {
        // At most one chomping and one indentation indicator, no whitespace
        // before or between them. The lenient scan accepted all of these.
        assert!(errors("|+-\n  a\n")); // two chomping indicators
        assert!(errors("|12\n  a\n")); // two indentation digits
        assert!(errors("| 2\n  a\n")); // whitespace before the indicator
    }

    #[test]
    fn valid_block_scalar_headers_scan() {
        // Either indicator order, with or without a trailing comment, is fine.
        assert_eq!(first("|2-\n   a\n").0, " a");
        assert_eq!(first("|-2\n   a\n").0, " a");
        assert_eq!(first("| # c\n  a\n").0, "a\n");
    }
}
