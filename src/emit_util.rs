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
            // Any other control character (C0 or DEL) must be escaped: a raw
            // control byte makes YAML a spec-compliant reader rejects. `\xNN` is
            // the YAML double-quote escape for a code point below U+0100.
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
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
        ];
        for (input, expected) in cases {
            let mut buf = Vec::new();
            push_double_quoted(&mut buf, input);
            assert_eq!(&buf, expected, "input {input:?}");
        }
    }

    #[test]
    fn double_quoted_passes_ordinary_text_through() {
        let mut buf = Vec::new();
        push_double_quoted(&mut buf, "héllo");
        assert_eq!(buf, "\"héllo\"".as_bytes());
    }
}
