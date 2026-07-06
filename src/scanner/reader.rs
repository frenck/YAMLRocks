use super::char_traits::{
    is_blank_byte, is_whitespace_or_break_byte, PLAIN_LINE_STOP_BLOCK, PLAIN_LINE_STOP_FLOW,
    PLAIN_STOP_BLOCK, PLAIN_STOP_FLOW,
};
use super::token::Span;
use super::ScanError;

/// Character-by-character reader over a UTF-8 string input.
///
/// Tracks line, column, and byte offset for span generation.
pub struct Reader<'input> {
    input: &'input str,
    pos: usize,
    line: u32,
    column: u32,
    file_id: u32,
    had_bom: bool,
}

/// The byte length of a UTF-8 encoded byte order mark (U+FEFF), and the byte
/// offset at which scanning starts when one is present at the head of the input.
const BOM_LEN: usize = 3;

impl<'input> Reader<'input> {
    /// Create a reader over `input`, with `file_id` 0 (the single-source case).
    pub fn new(input: &'input str) -> Self {
        Self::with_file_id(input, 0)
    }

    /// Create a reader that stamps every span with `file_id`, so spans stay
    /// attributable to their origin file once includes are resolved.
    pub fn new_with_file_id(input: &'input str, file_id: u32) -> Self {
        Self::with_file_id(input, file_id)
    }

    fn with_file_id(input: &'input str, file_id: u32) -> Self {
        // YAML 1.2 (§5.2) allows a byte order mark at the start of the stream; it
        // is encoding metadata, not content, so scanning begins past it. Skipping
        // it here (rather than slicing the input) keeps every span's byte offset
        // aligned with the original file. `had_bom` lets the round-trip path
        // restore the mark so re-emission stays byte-for-byte.
        let had_bom = input.starts_with('\u{feff}');
        Self {
            input,
            pos: if had_bom { BOM_LEN } else { 0 },
            line: 0,
            column: 0,
            file_id,
            had_bom,
        }
    }

    /// Whether the input began with a UTF-8 byte order mark (now skipped).
    #[inline]
    pub fn had_bom(&self) -> bool {
        self.had_bom
    }

    /// Current byte offset.
    #[inline]
    pub fn offset(&self) -> usize {
        self.pos
    }

    /// Current line (0-based).
    #[inline]
    pub fn line(&self) -> u32 {
        self.line
    }

    /// Current column (0-based).
    #[inline]
    pub fn column(&self) -> u32 {
        self.column
    }

    /// Reject any character outside the YAML 1.2 printable set (`c-printable`),
    /// pointing the error at the first offending character. Raw control
    /// characters are ill-formed input and PyYAML rejects them too; an *escaped*
    /// control in a double-quoted scalar is unaffected, since the scalar scanner
    /// produces it later from an escape sequence, not from a raw byte here. Runs
    /// once, before the first token, so every load path is covered.
    ///
    /// This is a byte scan, not a `char` scan: the fast path is a handful of
    /// comparisons per byte, so validating the whole input stays cheap. The only
    /// non-ASCII offenders are the C1 controls (`U+0080..=U+009F`, encoded
    /// `0xC2 0x80..0x9F`) and the two non-characters `U+FFFE`/`U+FFFF` (encoded
    /// `0xEF 0xBF 0xBE`/`0xBF`), so they need a small look-ahead on their lead
    /// byte; every other lead/continuation byte falls straight through.
    pub fn check_printable(&self) -> Result<(), ScanError> {
        let bytes = self.input.as_bytes();
        let start = if self.had_bom { BOM_LEN } else { 0 };
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            // Fast path: printable ASCII (`0x20..=0x7E`) is the overwhelming
            // majority of bytes, and `wrapping_sub` folds the range check into a
            // single comparison that pipelines cleanly.
            if b.wrapping_sub(0x20) < 0x5f {
                i += 1;
                continue;
            }
            let bad = if b < 0x20 {
                b != b'\t' && b != b'\n' && b != b'\r'
            } else if b == 0x7f {
                true
            } else if b == 0xc2 && i + 1 < bytes.len() {
                // C1 control, but NEL (`U+0085`, `0xC2 0x85`) is printable.
                let n = bytes[i + 1];
                (0x80..=0x9f).contains(&n) && n != 0x85
            } else if b == 0xef && i + 2 < bytes.len() {
                bytes[i + 1] == 0xbf && (bytes[i + 2] == 0xbe || bytes[i + 2] == 0xbf)
            } else {
                false
            };
            if bad {
                return Err(self.non_printable_error(i));
            }
            i += 1;
        }
        Ok(())
    }

    /// Build the error for a non-printable byte at `offset`, decoding the
    /// character and computing its line/column from the preceding text. Only
    /// runs on the rejection path, so the extra scan here is off the hot path.
    fn non_printable_error(&self, offset: usize) -> ScanError {
        let start = if self.had_bom { BOM_LEN } else { 0 };
        let mut line = 0u32;
        let mut column = 0u32;
        let mut prev_cr = false;
        for ch in self.input[start..offset].chars() {
            match ch {
                '\n' => {
                    // A `\n` right after a `\r` is the tail of one CRLF break,
                    // already counted, so do not count it twice.
                    if !prev_cr {
                        line += 1;
                        column = 0;
                    }
                    prev_cr = false;
                }
                '\r' => {
                    line += 1;
                    column = 0;
                    prev_cr = true;
                }
                _ => {
                    column += 1;
                    prev_cr = false;
                }
            }
        }
        let ch = self.input[offset..].chars().next().unwrap_or('\u{0}');
        ScanError::new(
            format!("disallowed control character U+{:04X}", ch as u32),
            Span::new(self.file_id, line, column, offset),
        )
    }

    /// Create a span at the current position.
    #[inline]
    pub fn span(&self) -> Span {
        Span::new(self.file_id, self.line, self.column, self.pos)
    }

    /// Whether we've reached the end of input.
    #[inline]
    pub fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    /// Peek at the current character without advancing.
    ///
    /// YAML scanning is overwhelmingly ASCII (structure, indentation, keys), so
    /// a single-byte fast path avoids building a `Chars` iterator on every call;
    /// only a UTF-8 lead byte (`>= 0x80`) falls back to a full decode.
    #[inline]
    pub fn peek(&self) -> char {
        match self.input.as_bytes().get(self.pos) {
            None => '\0',
            Some(&b) if b < 0x80 => b as char,
            Some(_) => self.input[self.pos..].chars().next().unwrap_or('\0'),
        }
    }

    /// The byte length of the (UTF-8) character starting at `pos`, or `None` if
    /// `pos` is at or past the end. ASCII is a one-byte fast path.
    #[inline]
    fn char_len_at(&self, pos: usize) -> Option<usize> {
        match self.input.as_bytes().get(pos) {
            None => None,
            Some(&b) if b < 0x80 => Some(1),
            Some(_) => Some(self.input[pos..].chars().next()?.len_utf8()),
        }
    }

    /// The character starting at `pos`, or `None` if `pos` is at or past the end.
    #[inline]
    fn char_at(&self, pos: usize) -> Option<char> {
        match self.input.as_bytes().get(pos) {
            None => None,
            Some(&b) if b < 0x80 => Some(b as char),
            Some(_) => self.input[pos..].chars().next(),
        }
    }

    /// Peek at the next character (one ahead).
    #[inline]
    pub fn peek_next(&self) -> Option<char> {
        let next_pos = self.pos + self.char_len_at(self.pos)?;
        self.char_at(next_pos)
    }

    /// Peek at a character N positions ahead.
    pub fn peek_at(&self, n: usize) -> Option<char> {
        let mut pos = self.pos;
        for _ in 0..n {
            pos += self.char_len_at(pos)?;
        }
        self.char_at(pos)
    }

    /// The first character at or after the cursor that is not an inline blank
    /// (space or tab), without consuming anything; `None` at end of input.
    ///
    /// Inline blanks are single-byte ASCII, so this scans the run in one pass,
    /// unlike a `peek_at(i)` loop that re-walks from the cursor on every step
    /// (quadratic in the run length).
    pub fn peek_after_blanks(&self) -> Option<char> {
        let bytes = self.input.as_bytes();
        let mut pos = self.pos;
        while bytes.get(pos).is_some_and(|&b| is_blank_byte(b)) {
            pos += 1;
        }
        self.char_at(pos)
    }

    /// Bulk-consume a run of ordinary plain-scalar content, stopping before the
    /// first byte that ends the run (a terminator, a flow indicator in flow
    /// context, or any non-ASCII byte) or at EOF. Returns the consumed slice.
    ///
    /// The stop condition is a single indexed lookup into a precomputed
    /// [`PLAIN_STOP_BLOCK`]/[`PLAIN_STOP_FLOW`] table (branch-free over the whole
    /// byte range), so a long run of content is skipped in one tight pass instead
    /// of peeking character by character. Because the table stops at every
    /// non-ASCII byte, every consumed byte is single-byte ASCII and `column`
    /// advances by the byte length, keeping column tracking exact.
    #[inline]
    pub fn take_plain_run(&mut self, in_flow: bool) -> &'input str {
        let table = if in_flow {
            &PLAIN_STOP_FLOW
        } else {
            &PLAIN_STOP_BLOCK
        };
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = self.pos;
        while i < bytes.len() && !table[bytes[i] as usize] {
            i += 1;
        }
        self.pos = i;
        self.column += (i - start) as u32;
        &self.input[start..i]
    }

    /// Bulk-consume a run of plain-scalar content *including internal blanks*,
    /// stopping at a line break, a `:`/`#` (possible indicator), a flow indicator
    /// in flow context, or a non-ASCII byte. Returns the consumed slice and the
    /// byte offset just past the last *non-blank* character (the content end:
    /// trailing blanks are not content and are stripped from a plain scalar).
    ///
    /// This takes a whole multi-word run (`a b c`) in one pass on the common
    /// single-line plain-scalar path, where [`take_plain_run`](Self::take_plain_run)
    /// would stop at every blank. The reader still advances past the trailing
    /// blanks (to the stopping byte) so the caller's loop resumes at the
    /// terminator, but the returned content end excludes them.
    #[inline]
    pub fn take_plain_line_run(&mut self, in_flow: bool) -> (&'input str, usize) {
        let table = if in_flow {
            &PLAIN_LINE_STOP_FLOW
        } else {
            &PLAIN_LINE_STOP_BLOCK
        };
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = self.pos;
        while i < bytes.len() && !table[bytes[i] as usize] {
            i += 1;
        }
        self.pos = i;
        self.column += (i - start) as u32;
        // Trailing blanks before the stopping byte are not content, so the
        // content ends at the last non-blank byte. Trimming once here keeps the
        // scan loop above a pure table walk instead of testing every byte.
        let mut content_end = i;
        while content_end > start && is_blank_byte(bytes[content_end - 1]) {
            content_end -= 1;
        }
        (&self.input[start..i], content_end)
    }

    /// Bulk-consume a run of ordinary single-quoted content, stopping before the
    /// closing quote `'`, a line break, or EOF. The caller handles the stopping
    /// byte: a `'` (close or `''` escape) or a break (fold).
    ///
    /// The next stop byte is found with a SIMD [`memchr::memchr3`], so a long
    /// scalar (a description, a base64 blob) is skipped in a few vector loads
    /// rather than one comparison per byte. `column` is per character, so it
    /// advances by the run's byte length when the run is pure ASCII (the common
    /// case, checked with a SIMD `is_ascii`) and by an exact character count only
    /// when the run carries multi-byte content. That count is the number of
    /// non-continuation bytes (`0b10xxxxxx` bytes are UTF-8 tails): the input is
    /// already valid UTF-8, so this counts code points without a second decode.
    #[inline]
    pub fn take_single_quoted_run(&mut self) {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let stop = memchr::memchr3(b'\'', b'\n', b'\r', &bytes[start..])
            .map_or(bytes.len(), |i| start + i);
        let run = &bytes[start..stop];
        self.column += if run.is_ascii() {
            run.len() as u32
        } else {
            run.iter().filter(|&&b| (b & 0xc0) != 0x80).count() as u32
        };
        self.pos = stop;
    }

    /// Bulk-consume a run of ordinary double-quoted content, stopping before the
    /// closing quote `"`, an escape `\`, a line break, or any non-ASCII byte (and
    /// at EOF). ASCII-only, so `column` stays exact; see
    /// [`take_single_quoted_run`](Self::take_single_quoted_run).
    #[inline]
    pub fn take_double_quoted_run(&mut self) {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let mut i = self.pos;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'"' || b == b'\\' || b == b'\n' || b == b'\r' || b >= 0x80 {
                break;
            }
            i += 1;
        }
        self.column += (i - start) as u32;
        self.pos = i;
    }

    /// Bulk-consume the rest of the current line (to the next `\n`/`\r`, or EOF)
    /// and return it. The block-scalar scanner copies whole content lines into
    /// its owned buffer, so a per-character loop there is pure overhead; the line
    /// end is found with a SIMD [`memchr::memchr2`]. `column` advances by the byte
    /// length for a pure-ASCII line (the common case) and by a code-point count
    /// (non-continuation bytes) otherwise; see
    /// [`take_single_quoted_run`](Self::take_single_quoted_run).
    #[inline]
    pub fn take_until_line_break(&mut self) -> &'input str {
        let bytes = self.input.as_bytes();
        let start = self.pos;
        let stop =
            memchr::memchr2(b'\n', b'\r', &bytes[start..]).map_or(bytes.len(), |i| start + i);
        let run = &self.input[start..stop];
        self.column += if run.is_ascii() {
            run.len() as u32
        } else {
            run.bytes().filter(|&b| (b & 0xc0) != 0x80).count() as u32
        };
        self.pos = stop;
        run
    }

    /// Check if the next character is a line break or EOF.
    #[inline]
    pub fn check_next_is_break_or_eof(&self) -> bool {
        matches!(self.peek_next(), None | Some('\n' | '\r'))
    }

    /// Check if the next character is an inline blank (space or tab). A block
    /// entry `-` may be separated from its value by either.
    #[inline]
    pub fn check_next_is_blank(&self) -> bool {
        matches!(self.peek_next(), Some(' ' | '\t'))
    }

    /// Check if the input starting at the current position matches the given string.
    pub fn check_ahead(&self, expected: &str) -> bool {
        self.input[self.pos..].starts_with(expected)
    }

    /// Whether the byte just before the cursor is whitespace, a line break, or
    /// the start of input. Used to decide whether a `#` begins a comment (it
    /// only does when preceded by a blank or at line/input start).
    #[inline]
    pub fn prev_is_whitespace_or_start(&self) -> bool {
        // A skipped leading byte order mark is not content, so the first real
        // character sits at the logical start of the stream (a `#` there opens a
        // comment just as it would at offset 0).
        if self.pos == 0 || (self.had_bom && self.pos == BOM_LEN) {
            return true;
        }
        is_whitespace_or_break_byte(self.input.as_bytes()[self.pos - 1])
    }

    /// Bulk-skip a run of spaces, advancing the column by the run length.
    ///
    /// Spaces are single-byte, so the run is found with one tight byte scan and
    /// `pos`/`column` update once, instead of a bounds-checked peek/advance
    /// cycle per character. Indentation makes these runs long and frequent.
    #[inline]
    pub fn skip_spaces(&mut self) {
        let bytes = self.input.as_bytes();
        let mut i = self.pos;
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        self.column += (i - self.pos) as u32;
        self.pos = i;
    }

    /// Advance by one character.
    #[inline]
    pub fn advance(&mut self) {
        if self.is_eof() {
            return;
        }
        let ch = self.peek();
        self.pos += ch.len_utf8();
        self.column += 1;
    }

    /// Advance past a line break (handles \r\n, \n, \r).
    pub fn advance_line(&mut self) {
        if self.is_eof() {
            return;
        }
        let ch = self.peek();
        if ch == '\r' {
            self.pos += 1;
            if !self.is_eof() && self.peek() == '\n' {
                self.pos += 1;
            }
        } else if ch == '\n' {
            self.pos += 1;
        }
        self.line += 1;
        self.column = 0;
    }

    /// Advance by N characters.
    pub fn advance_n(&mut self, n: usize) {
        for _ in 0..n {
            self.advance();
        }
    }

    /// Get a slice of the original input.
    pub fn slice(&self, start: usize, end: usize) -> &'input str {
        &self.input[start..end]
    }

    /// Save the current position for later restoration.
    pub fn save(&self) -> SavedPosition {
        SavedPosition {
            pos: self.pos,
            line: self.line,
            column: self.column,
        }
    }

    /// Restore a previously saved position.
    pub fn restore(&mut self, saved: SavedPosition) {
        self.pos = saved.pos;
        self.line = saved.line;
        self.column = saved.column;
    }
}

/// An opaque snapshot of a [`Reader`]'s position, taken by
/// [`Reader::save`] and handed back to [`Reader::restore`] to backtrack.
pub struct SavedPosition {
    pos: usize,
    line: u32,
    column: u32,
}
