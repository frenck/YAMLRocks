//! Character classification predicates for the scanner.
//!
//! The YAML grammar partitions characters into a handful of classes (line
//! breaks, blanks, flow indicators, anchor characters, ...). Defining each class
//! once here, rather than re-spelling the same `matches!` set at every use site,
//! keeps the definitions consistent: a single drift between two copies of, say,
//! the flow-indicator set would be a subtle scanner bug. Every predicate is
//! `#[inline]`, so routing through them costs nothing at runtime.
//!
//! Both `char` and `u8` (ASCII byte) variants exist: the reader's hot path scans
//! raw bytes (every YAML structural character is ASCII), while the higher-level
//! scanner logic works in `char`s.

/// A line break: line feed or carriage return.
#[inline]
pub(super) fn is_break(ch: char) -> bool {
    matches!(ch, '\n' | '\r')
}

/// A blank or a line break (space, tab, LF, CR).
#[inline]
pub(super) fn is_whitespace_or_break(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r')
}

/// A flow collection indicator: `,` `[` `]` `{` `}`.
#[inline]
pub(super) fn is_flow_indicator(ch: char) -> bool {
    matches!(ch, ',' | '[' | ']' | '{' | '}')
}

/// A blank, a line break, or a flow indicator: the characters that can terminate
/// a plain scalar or a key in flow context.
#[inline]
pub(super) fn is_whitespace_or_flow(ch: char) -> bool {
    is_whitespace_or_break(ch) || is_flow_indicator(ch)
}

/// A character that may appear in an anchor or alias name: anything that is not a
/// blank, a line break, a flow indicator, or a NUL.
#[inline]
pub(super) fn is_anchor_char(ch: char) -> bool {
    !(is_whitespace_or_flow(ch) || ch == '\0')
}

/// A blank byte: space or tab.
#[inline]
pub(super) fn is_blank_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t')
}

/// A blank or line-break byte (space, tab, LF, CR).
#[inline]
pub(super) fn is_whitespace_or_break_byte(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Build the 256-entry table marking every byte that ends a plain scalar's
/// ordinary-content run. A byte is a stop byte when it is non-ASCII (`>= 0x80`,
/// so the run stays single-byte and column tracking is exact), a line break, a
/// `:`/`#` (possible indicator), a blank, or, in flow context, a flow indicator.
const fn plain_stop_table(in_flow: bool) -> [bool; 256] {
    let mut table = [false; 256];
    // Every non-ASCII byte stops the run.
    let mut b = 0x80;
    while b < 256 {
        table[b] = true;
        b += 1;
    }
    table[b'\n' as usize] = true;
    table[b'\r' as usize] = true;
    table[b':' as usize] = true;
    table[b'#' as usize] = true;
    table[b' ' as usize] = true;
    table[b'\t' as usize] = true;
    if in_flow {
        table[b',' as usize] = true;
        table[b'[' as usize] = true;
        table[b']' as usize] = true;
        table[b'{' as usize] = true;
        table[b'}' as usize] = true;
    }
    table
}

/// Plain-scalar stop table for block context.
pub(super) static PLAIN_STOP_BLOCK: [bool; 256] = plain_stop_table(false);

/// Plain-scalar stop table for flow context (block stops plus flow indicators).
/// The hot loop in [`super::reader::Reader::take_plain_run`] consumes the run of
/// bytes for which the selected table is false; a `:`/`#` that survives is
/// content (it failed the stricter end-of-scalar checks) and is consumed singly
/// by the caller.
pub(super) static PLAIN_STOP_FLOW: [bool; 256] = plain_stop_table(true);

/// Like [`plain_stop_table`] but blanks (space, tab) are *not* stops: they are
/// consumed as ordinary content so a multi-word plain scalar (`a b c`) is taken
/// in one pass. Trailing blanks are stripped by the caller via the last
/// non-blank position. Used on the single-line (un-folded) plain-scalar path.
const fn plain_line_stop_table(in_flow: bool) -> [bool; 256] {
    let mut table = plain_stop_table(in_flow);
    table[b' ' as usize] = false;
    table[b'\t' as usize] = false;
    table
}

/// Single-line plain-scalar stop table for block context (blanks are content).
pub(super) static PLAIN_LINE_STOP_BLOCK: [bool; 256] = plain_line_stop_table(false);

/// Single-line plain-scalar stop table for flow context (blanks are content).
pub(super) static PLAIN_LINE_STOP_FLOW: [bool; 256] = plain_line_stop_table(true);
