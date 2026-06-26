use super::{Resolver, ScalarKind};
use crate::scanner::ScalarStyle;

/// YAML 1.2 Core Schema resolver.
///
/// Resolves plain scalars to typed values following the YAML 1.2 specification:
/// - null: ~, null, Null, NULL, empty
/// - bool: true, True, TRUE, false, False, FALSE
/// - int: decimal, 0o octal, 0x hex
/// - float: decimal with dot/exponent, .inf, .nan
/// - string: everything else
pub struct Yaml12Resolver;

impl Resolver for Yaml12Resolver {
    fn classify(&self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ScalarKind {
        if let Some(tag) = tag {
            return classify_tagged(value, tag);
        }

        // Quoted and block scalars are always strings.
        if matches!(
            style,
            ScalarStyle::SingleQuoted
                | ScalarStyle::DoubleQuoted
                | ScalarStyle::Literal
                | ScalarStyle::Folded
        ) {
            return ScalarKind::Str;
        }

        // Plain scalar: resolve its type.
        classify_plain_12(value)
    }
}

fn classify_plain_12(value: &str) -> ScalarKind {
    if super::is_null(value) {
        return ScalarKind::Null;
    }

    // A plain `<<` is the merge-key indicator. Only the plain style reaches here
    // (the caller short-circuits quoted and block scalars to `Str`), so a quoted
    // `"<<"` is a literal string and is never treated as a merge key.
    if value == "<<" {
        return ScalarKind::Merge;
    }

    // Bool (strict: only true/false variants)
    if matches!(value, "true" | "True" | "TRUE") {
        return ScalarKind::Bool(true);
    }
    if matches!(value, "false" | "False" | "FALSE") {
        return ScalarKind::Bool(false);
    }

    // Every integer and float starts with a digit, a sign, or a dot (`-5`, `.5`,
    // `.inf`); anything else cannot be a number, so skip the int and float parse
    // attempts. This is the common case (names, paths, words) and the parses are
    // not free. `is_null` already consumed the empty string above, so there is a
    // first byte to read.
    if !matches!(value.as_bytes()[0], b'0'..=b'9' | b'-' | b'+' | b'.') {
        return ScalarKind::Str;
    }

    // Integer
    if let Some(int) = try_parse_int_12(value) {
        return ScalarKind::Int(int);
    }
    // An integer too large for i64 is still an integer, not a string, whether it
    // is decimal or a hex/octal literal (`0x8000000000000000`).
    if is_big_int_12(value) {
        return ScalarKind::BigInt;
    }

    // Float
    if let Some(float) = try_parse_float_12(value) {
        return ScalarKind::Float(float);
    }

    ScalarKind::Str
}

/// Whether `value` is a valid YAML 1.2 integer literal of any magnitude: a
/// decimal, hexadecimal (`0x`), or octal (`0o`) form. Only reached once
/// [`try_parse_int_12`] has failed (an overflowing or otherwise too-large
/// literal), so a `true` result means a big integer rather than a string.
fn is_big_int_12(value: &str) -> bool {
    let Some((_, rest)) = super::split_sign(value) else {
        return false;
    };
    if let Some(body) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_hexdigit())
    } else if let Some(body) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
        !body.is_empty() && body.bytes().all(|b| (b'0'..=b'7').contains(&b))
    } else {
        super::is_big_decimal_int(value)
    }
}

fn try_parse_int_12(value: &str) -> Option<i64> {
    let (negative, rest) = super::split_sign(value)?;

    let result = if rest.starts_with("0x") || rest.starts_with("0X") {
        // Hexadecimal
        super::from_radix_unsigned(&rest[2..], 16)?
    } else if rest.starts_with("0o") || rest.starts_with("0O") {
        // Octal (YAML 1.2 style)
        super::from_radix_unsigned(&rest[2..], 8)?
    } else if let Some(after_zero) = rest.strip_prefix('0') {
        // A leading zero is only a valid integer when it stands alone (`0`);
        // 1.2 has no C-style octal, so `0777` is a string, not a number.
        if after_zero.is_empty() {
            0
        } else {
            return None;
        }
    } else {
        // Decimal
        rest.parse::<i64>().ok()?
    };

    if negative {
        Some(-result)
    } else {
        Some(result)
    }
}

fn try_parse_float_12(value: &str) -> Option<f64> {
    if let Some(f) = super::parse_special_float(value) {
        return Some(f);
    }

    // Must contain a dot or exponent to be a float. One byte pass instead of
    // three separate `contains(char)` scans (each a full string search).
    if !value.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        return None;
    }

    value.parse::<f64>().ok()
}

fn classify_tagged(value: &str, tag: &str) -> ScalarKind {
    // An explicit core tag whose content does not match the type (`!!int nope`,
    // `!!bool maybe`) is kept as a string rather than coerced to a wrong-but-valid
    // value (which silently turned `!!int nope` into `0`). A conforming value
    // still resolves to its type, including an integer too large for i64.
    match tag {
        "!!null" | "tag:yaml.org,2002:null" => {
            if super::is_null(value) {
                ScalarKind::Null
            } else {
                ScalarKind::Str
            }
        }
        "!!bool" | "tag:yaml.org,2002:bool" => match value {
            "true" | "True" | "TRUE" => ScalarKind::Bool(true),
            "false" | "False" | "FALSE" => ScalarKind::Bool(false),
            _ => ScalarKind::Str,
        },
        "!!int" | "tag:yaml.org,2002:int" => {
            if let Some(int) = try_parse_int_12(value) {
                ScalarKind::Int(int)
            } else if is_big_int_12(value) {
                ScalarKind::BigInt
            } else {
                ScalarKind::Str
            }
        }
        "!!float" | "tag:yaml.org,2002:float" => match try_parse_float_12_tagged(value) {
            Some(float) => ScalarKind::Float(float),
            None => ScalarKind::Str,
        },
        _ => ScalarKind::Str,
    }
}

/// Parse a value carrying an explicit `!!float` tag. Unlike the plain-scalar
/// [`try_parse_float_12`] (which requires a `.`/`e`/`E` so a bare integer stays
/// an int), an integer-form decimal like `42` is a *conforming* float under the
/// core schema and resolves to `42.0`. Hexadecimal and octal forms are not part
/// of the float production, so they fall through to a string (Rust's `f64` parse
/// rejects them), matching the schema rather than coercing `0x10` to `16.0`.
fn try_parse_float_12_tagged(value: &str) -> Option<f64> {
    if let Some(f) = super::parse_special_float(value) {
        return Some(f);
    }
    value.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::Yaml12Resolver;
    use crate::resolver::{ResolvedValue, Resolver};
    use crate::scanner::ScalarStyle;

    fn plain(value: &str) -> ResolvedValue {
        Yaml12Resolver.resolve(value, ScalarStyle::Plain, None)
    }

    #[test]
    fn resolves_null_forms() {
        for value in ["~", "null", "Null", "NULL", ""] {
            assert_eq!(plain(value), ResolvedValue::Null, "{value:?}");
        }
    }

    #[test]
    fn radix_prefix_rejects_internal_sign() {
        // `from_str_radix` itself accepts a sign; the body after `0x`/`0o` must
        // not, or `0x-5` would wrongly read as -5 instead of a string.
        for value in ["0x-5", "0x+5", "0o-5", "0o+7", "0X-1"] {
            assert!(
                matches!(plain(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
        // A real radix integer still parses.
        assert_eq!(plain("0xFF"), ResolvedValue::Int(255));
        assert_eq!(plain("0o17"), ResolvedValue::Int(15));
        assert_eq!(plain("-0x10"), ResolvedValue::Int(-16));
    }

    #[test]
    fn resolves_booleans_strictly() {
        for value in ["true", "True", "TRUE"] {
            assert_eq!(plain(value), ResolvedValue::Bool(true), "{value:?}");
        }
        for value in ["false", "False", "FALSE"] {
            assert_eq!(plain(value), ResolvedValue::Bool(false), "{value:?}");
        }
        // YAML 1.2 keeps yes/no/on/off as plain strings.
        for value in ["yes", "no", "on", "off", "y", "n"] {
            assert!(
                matches!(plain(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
    }

    #[test]
    fn resolves_integers() {
        assert_eq!(plain("0"), ResolvedValue::Int(0));
        assert_eq!(plain("42"), ResolvedValue::Int(42));
        assert_eq!(plain("-5"), ResolvedValue::Int(-5));
        assert_eq!(plain("+7"), ResolvedValue::Int(7));
        assert_eq!(plain("0xFF"), ResolvedValue::Int(255));
        assert_eq!(plain("0o17"), ResolvedValue::Int(15));
    }

    #[test]
    fn rejects_leading_zero_decimal_as_string() {
        // 1.2 has no C-style octal: a leading zero makes it a string, not 7.
        assert!(matches!(plain("0777"), ResolvedValue::String(_)));
    }

    #[test]
    fn resolves_floats() {
        assert_eq!(plain("1.5"), ResolvedValue::Float(1.5));
        assert_eq!(plain("1e3"), ResolvedValue::Float(1000.0));
        assert!(
            matches!(plain(".inf"), ResolvedValue::Float(f) if f.is_infinite() && f.is_sign_positive())
        );
        assert!(
            matches!(plain("-.inf"), ResolvedValue::Float(f) if f.is_infinite() && f.is_sign_negative())
        );
        assert!(matches!(plain(".nan"), ResolvedValue::Float(f) if f.is_nan()));
    }

    #[test]
    fn everything_else_is_a_string() {
        for value in ["hello", "2026-01-02", "1.2.3", "a: b"] {
            assert!(
                matches!(plain(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
    }

    #[test]
    fn quoted_and_block_scalars_are_always_strings() {
        for style in [
            ScalarStyle::SingleQuoted,
            ScalarStyle::DoubleQuoted,
            ScalarStyle::Literal,
            ScalarStyle::Folded,
        ] {
            assert_eq!(
                Yaml12Resolver.resolve("42", style, None),
                ResolvedValue::String("42".to_owned()),
                "{style:?}",
            );
            assert_eq!(
                Yaml12Resolver.resolve("null", style, None),
                ResolvedValue::String("null".to_owned()),
                "{style:?}",
            );
        }
    }

    #[test]
    fn explicit_tags_override_resolution() {
        let r = Yaml12Resolver;
        assert!(matches!(
            r.resolve("42", ScalarStyle::Plain, Some("!!str")),
            ResolvedValue::String(_)
        ));
        assert_eq!(
            r.resolve("7", ScalarStyle::Plain, Some("!!int")),
            ResolvedValue::Int(7)
        );
        assert_eq!(
            r.resolve("true", ScalarStyle::Plain, Some("!!bool")),
            ResolvedValue::Bool(true)
        );
        assert_eq!(
            r.resolve("null", ScalarStyle::Plain, Some("!!null")),
            ResolvedValue::Null
        );
        // The float tag drives its own classification arm.
        assert_eq!(
            r.resolve("1.5", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::Float(1.5)
        );
        // An integer-form value is a conforming float under an explicit tag
        // (`42` -> `42.0`), unlike in plain resolution where it stays an int.
        assert_eq!(
            r.resolve("42", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::Float(42.0)
        );
        // Hex/octal forms are not part of the float production, so they stay a
        // string rather than being coerced to their integer value.
        assert_eq!(
            r.resolve("0x10", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::String("0x10".to_owned())
        );
        // A non-conforming value under an explicit core tag is kept as a string,
        // not coerced to a wrong-but-valid 0/0.0/false/null.
        for (value, tag) in [
            ("nope", "!!int"),
            ("nope", "!!float"),
            ("maybe", "!!bool"),
            ("anything", "!!null"),
        ] {
            assert_eq!(
                r.resolve(value, ScalarStyle::Plain, Some(tag)),
                ResolvedValue::String(value.to_owned()),
                "{tag} {value:?}"
            );
        }
        // A conforming integer too large for i64 still resolves as an integer.
        assert_eq!(
            r.resolve("99999999999999999999", ScalarStyle::Plain, Some("!!int")),
            ResolvedValue::BigInt("99999999999999999999".to_owned())
        );
    }
}
