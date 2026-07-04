//! Shared YAML emission primitives used by both the fast encoder
//! (`crate::encode`) and the round-trip emitter (`crate::roundtrip::emit`), so
//! scalar quoting and float spelling have a single definition and cannot drift
//! between the two paths.

/// Render a float the way YAML expects: `.inf`/`-.inf`/`.nan` for the special
/// values, and a trailing `.0` for whole numbers so the result reparses as a
/// float rather than an integer.
pub(crate) fn canonical_float(f: f64) -> String {
    if f.is_infinite() {
        if f.is_sign_positive() {
            ".inf".to_owned()
        } else {
            "-.inf".to_owned()
        }
    } else if f.is_nan() {
        ".nan".to_owned()
    } else {
        // Match Python's `repr`/PyYAML: scientific notation for very large or very
        // small magnitudes (decimal exponent < -4 or >= 16), plain decimal
        // otherwise. Rust's `{}` never uses exponents (so `1e308` would expand to
        // 309 digits) and `{:e}` always does; pick per magnitude.
        let exp = scientific_exponent(f);
        if !(-4..16).contains(&exp) {
            // `{:e}` yields the shortest mantissa (e.g. `6.022e23`, `1e308`). Give
            // it a decimal point and a signed, zero-padded exponent so it reads
            // back as a float in every parser (PyYAML 1.1 needs the dot).
            let sci = format!("{f:e}");
            let (mantissa, exp_str) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
            let mantissa = if mantissa.contains('.') {
                mantissa.to_owned()
            } else {
                format!("{mantissa}.0")
            };
            let exp: i32 = exp_str.parse().unwrap_or(0);
            let sign = if exp < 0 { '-' } else { '+' };
            format!("{mantissa}e{sign}{:02}", exp.abs())
        } else {
            let s = format!("{f}");
            if s.contains('.') {
                s
            } else {
                format!("{s}.0")
            }
        }
    }
}

/// The base-10 exponent of `f`'s shortest representation (the exponent `{:e}`
/// would print), used to choose between decimal and scientific notation.
fn scientific_exponent(f: f64) -> i32 {
    let sci = format!("{f:e}");
    sci.split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0)
}

/// The body of a single-quoted scalar (no surrounding quotes), doubling any `'`.
/// Returned separately from the quotes so the caller can fold it across lines.
pub(crate) fn single_quoted_body(value: &str) -> String {
    value.replace('\'', "''")
}

/// Whether `c` falls outside YAML 1.2's `c-printable` set and so cannot appear
/// raw in a scalar: it forces a plain scalar to be quoted and must be escaped
/// inside a double-quoted one. This is the emitter's mirror of the scanner's
/// `check_printable` rejection set (see `scanner::reader`), so `dumps` never
/// emits a control character that `loads` would refuse: the C0 controls except
/// tab, line feed, and carriage return; DEL; the C1 controls except NEL
/// (`U+0085`, which is printable); and the non-characters `U+FFFE`/`U+FFFF`.
pub(crate) fn is_non_printable(c: char) -> bool {
    match c as u32 {
        0x09 | 0x0a | 0x0d => false, // tab, LF, CR are allowed
        0x00..=0x1f | 0x7f => true,  // other C0 controls and DEL
        0x85 => false,               // NEL is printable in YAML 1.2
        0x80..=0x9f => true,         // the other C1 controls
        0xfffe | 0xffff => true,     // the two non-characters
        _ => false,
    }
}

/// The body of a double-quoted scalar (no surrounding quotes), with the escapes
/// YAML requires inside double quotes applied.
pub(crate) fn double_quoted_body(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            // Any other non-printable character must be escaped: emitting it raw
            // makes YAML a spec-compliant reader rejects. `\xNN` is the escape for
            // a code point below U+0100 (the C0/C1 controls and DEL); `\uNNNN`
            // covers the wider non-characters `U+FFFE`/`U+FFFF`.
            c if is_non_printable(c) => {
                let n = c as u32;
                if n <= 0xff {
                    out.push_str(&format!("\\x{n:02x}"));
                } else {
                    out.push_str(&format!("\\u{n:04x}"));
                }
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Append `value` to `buf` as a single-quoted YAML scalar, doubling any `'`.
pub(crate) fn push_single_quoted(buf: &mut Vec<u8>, value: &str) {
    buf.push(b'\'');
    buf.extend_from_slice(single_quoted_body(value).as_bytes());
    buf.push(b'\'');
}

/// Append `value` to `buf` as a double-quoted YAML scalar, escaping the
/// characters YAML requires inside double quotes.
pub(crate) fn push_double_quoted(buf: &mut Vec<u8>, value: &str) {
    buf.push(b'"');
    buf.extend_from_slice(double_quoted_body(value).as_bytes());
    buf.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::{canonical_float, push_double_quoted, push_single_quoted};

    #[test]
    fn canonical_float_special_values() {
        assert_eq!(canonical_float(f64::INFINITY), ".inf");
        assert_eq!(canonical_float(f64::NEG_INFINITY), "-.inf");
        assert_eq!(canonical_float(f64::NAN), ".nan");
    }

    #[test]
    fn canonical_float_appends_dot_zero_for_whole_numbers() {
        // A whole number must come back spelled as a float so it reparses as one.
        assert_eq!(canonical_float(1.0), "1.0");
        assert_eq!(canonical_float(-42.0), "-42.0");
        assert_eq!(canonical_float(0.0), "0.0");
    }

    #[test]
    fn canonical_float_keeps_existing_dot() {
        // A value already spelled with a dot must be left untouched. Turning the
        // `||` in the suffix check into `&&` would wrongly re-suffix `1.5` as
        // `1.5.0`, so this pins that branch.
        assert_eq!(canonical_float(1.5), "1.5");
        assert_eq!(canonical_float(0.0001), "0.0001");
    }

    #[test]
    fn canonical_float_uses_scientific_notation_for_extreme_magnitudes() {
        // Large or small magnitudes use scientific notation (matching PyYAML and
        // Python's repr), with a dotted mantissa and a signed exponent.
        assert_eq!(canonical_float(1e308), "1.0e+308");
        assert_eq!(canonical_float(6.022e23), "6.022e+23");
        assert_eq!(canonical_float(1e-10), "1.0e-10");
        assert_eq!(canonical_float(1e-5), "1.0e-05");
        assert_eq!(canonical_float(-1e300), "-1.0e+300");
        // The thresholds: 1e15 stays decimal, 1e16 switches to scientific.
        assert_eq!(canonical_float(1e15), "1000000000000000.0");
        assert_eq!(canonical_float(1e16), "1.0e+16");
    }

    #[test]
    fn single_quoted_doubles_apostrophes() {
        let mut buf = Vec::new();
        push_single_quoted(&mut buf, "it's a 'test'");
        assert_eq!(buf, b"'it''s a ''test'''");
    }

    #[test]
    fn single_quoted_passes_other_text_through() {
        let mut buf = Vec::new();
        push_single_quoted(&mut buf, "plain");
        assert_eq!(buf, b"'plain'");
    }

    #[test]
    fn double_quoted_escapes_each_special_character() {
        // One case per escaped arm, so deleting any arm changes the output.
        let cases: &[(&str, &[u8])] = &[
            ("a\"b", b"\"a\\\"b\""),
            ("a\\b", b"\"a\\\\b\""),
            ("a\nb", b"\"a\\nb\""),
            ("a\rb", b"\"a\\rb\""),
            ("a\tb", b"\"a\\tb\""),
            ("a\0b", b"\"a\\0b\""),
            // DEL and a C0 control escape as `\xNN`.
            ("a\x7fb", b"\"a\\x7fb\""),
            ("a\x01b", b"\"a\\x01b\""),
            // The C1 controls escape as `\xNN` too (a raw one is not printable and
            // would make YAML `loads` rejects), except NEL (`U+0085`).
            ("a\u{80}b", b"\"a\\x80b\""),
            ("a\u{9f}b", b"\"a\\x9fb\""),
            // The non-characters need the wider `\uNNNN` form.
            ("a\u{fffe}b", b"\"a\\ufffeb\""),
            ("a\u{ffff}b", b"\"a\\uffffb\""),
        ];
        for (input, expected) in cases {
            let mut buf = Vec::new();
            push_double_quoted(&mut buf, input);
            assert_eq!(&buf, expected, "input {input:?}");
        }
    }

    #[test]
    fn printable_high_controls_pass_through_double_quoted() {
        // NEL (`U+0085`) and NBSP (`U+00A0`) are printable in YAML 1.2, so they
        // stay raw rather than being escaped; this pins that they are not swept up
        // with the neighboring C1 controls.
        let mut buf = Vec::new();
        push_double_quoted(&mut buf, "a\u{85}\u{a0}b");
        assert_eq!(buf, "\"a\u{85}\u{a0}b\"".as_bytes());
    }

    #[test]
    fn double_quoted_passes_ordinary_text_through() {
        let mut buf = Vec::new();
        push_double_quoted(&mut buf, "héllo");
        assert_eq!(buf, "\"héllo\"".as_bytes());
    }
}
