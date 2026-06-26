use std::borrow::Cow;

use super::{Resolver, ScalarKind};
use crate::scanner::ScalarStyle;

/// Strip YAML 1.1 underscore digit separators (`1_000`), borrowing the input when
/// it has none, which is the overwhelmingly common case. Resolving a numeric
/// scalar then costs no allocation unless a separator is actually present; the
/// previous unconditional `chars().filter().collect()` allocated a `String` for
/// every number-looking scalar (up to three times, once per parse attempt).
fn strip_underscores(value: &str) -> Cow<'_, str> {
    if value.as_bytes().contains(&b'_') {
        Cow::Owned(value.chars().filter(|&c| c != '_').collect())
    } else {
        Cow::Borrowed(value)
    }
}

/// YAML 1.1 Schema resolver.
///
/// Extends the 1.2 resolver with:
/// - Expanded booleans: yes/no, on/off, y/n (and case variants)
/// - C-style octals: 0777
/// - Binary integers: 0b1010
/// - Sexagesimal integers: 1:30 = 90
/// - Underscores in numbers: 1_000
/// - Merge key: << in mapping key position
///
/// `pyyaml_compat` narrows the boolean set to PyYAML's: the real YAML 1.1 spec
/// makes bare `y`/`Y`/`n`/`N` booleans, but PyYAML's resolver deliberately omits
/// them (keeping only `yes/no/on/off/true/false`). Setting this matches PyYAML
/// for ecosystem interop, at the cost of being intentionally off-spec.
#[derive(Default)]
pub struct Yaml11Resolver {
    pub pyyaml_compat: bool,
}

impl Resolver for Yaml11Resolver {
    fn classify(&self, value: &str, style: ScalarStyle, tag: Option<&str>) -> ScalarKind {
        if let Some(tag) = tag {
            return classify_tagged_11(value, tag, self.pyyaml_compat);
        }

        if matches!(
            style,
            ScalarStyle::SingleQuoted
                | ScalarStyle::DoubleQuoted
                | ScalarStyle::Literal
                | ScalarStyle::Folded
        ) {
            return ScalarKind::Str;
        }

        classify_plain_11(value, self.pyyaml_compat)
    }
}

fn classify_plain_11(value: &str, pyyaml_compat: bool) -> ScalarKind {
    if super::is_null(value) {
        return ScalarKind::Null;
    }

    // A plain `<<` is the merge-key indicator. Only the plain style reaches here
    // (the caller short-circuits quoted and block scalars to `Str`), so a quoted
    // `"<<"` is a literal string and is never treated as a merge key.
    if value == "<<" {
        return ScalarKind::Merge;
    }

    // Bool (expanded YAML 1.1 set)
    if let Some(b) = try_parse_bool_11(value, pyyaml_compat) {
        return ScalarKind::Bool(b);
    }

    // A 1.1 number starts with a digit, a sign, a dot (`.inf`), or an underscore:
    // the int and float parsers strip every underscore before parsing, so a
    // leading `_` (`_5` -> 5) is numeric here even though it is unusual. The bool
    // words (`yes`, `on`, ...) are letters and were just ruled out, so a value
    // starting with anything else is a string and skips the three numeric parses.
    // `is_null` already consumed the empty string, so there is a first byte.
    if !matches!(value.as_bytes()[0], b'0'..=b'9' | b'-' | b'+' | b'.' | b'_') {
        return ScalarKind::Str;
    }

    // PyYAML's resolver does not treat a leading underscore as numeric (`_5` and
    // `_1.5` are strings); only an interior separator (`1_000`) is allowed. The
    // lenient default 1.1 reading accepts the leading form, so this only narrows
    // PyYAML-compat. A sign may precede the underscore (`-_5`).
    if pyyaml_compat {
        let after_sign = value.strip_prefix(['+', '-']).unwrap_or(value);
        if after_sign.starts_with('_') {
            return ScalarKind::Str;
        }
    }

    // Integer
    if let Some(int) = try_parse_int_11(value) {
        return ScalarKind::Int(int);
    }
    // An integer too large for i64 is still an integer, not a string, whether it
    // is decimal, hex (`0x`), binary (`0b`), or C-style octal (`0777`). The 1.1
    // forms may carry underscore separators, which the parsers strip.
    if is_big_int_11(value) {
        return ScalarKind::BigInt;
    }

    // Float
    if let Some(float) = try_parse_float_11(value, pyyaml_compat) {
        return ScalarKind::Float(float);
    }

    ScalarKind::Str
}

fn try_parse_bool_11(value: &str, pyyaml_compat: bool) -> Option<bool> {
    match value {
        // Bare single-letter forms are YAML 1.1 booleans, but PyYAML omits them;
        // under PyYAML-compat they fall through and stay plain strings.
        "y" | "Y" if !pyyaml_compat => Some(true),
        "n" | "N" if !pyyaml_compat => Some(false),
        "yes" | "Yes" | "YES" | "true" | "True" | "TRUE" | "on" | "On" | "ON" => Some(true),
        "no" | "No" | "NO" | "false" | "False" | "FALSE" | "off" | "Off" | "OFF" => Some(false),
        _ => None,
    }
}

/// Whether `value` is a YAML 1.1 base-10 integer once underscore digit
/// separators are removed, recognizing a decimal integer whose magnitude
/// overflows `i64`. Mirrors the lenient underscore handling of
/// [`try_parse_int_11`] (underscores are simply stripped) so the two agree on
/// what counts as a decimal integer; only the overflowing case reaches here.
fn is_big_decimal_int_11(value: &str) -> bool {
    let Some((_, rest)) = super::split_sign(value) else {
        return false;
    };
    let cleaned = strip_underscores(rest);
    let digits: &str = &cleaned;
    !digits.is_empty()
        && digits.bytes().all(|b| b.is_ascii_digit())
        // A leading zero is a C-style octal (`0777`), not a decimal big integer.
        && (digits == "0" || !digits.starts_with('0'))
}

/// Whether `value` is a valid YAML 1.1 integer literal of any magnitude:
/// decimal, hexadecimal (`0x`), binary (`0b`), or C-style octal (`0777`), with
/// underscore separators allowed. Only reached once [`try_parse_int_11`] has
/// failed, so a `true` result means a big integer rather than a string.
/// Sexagesimal (`1:30`) is intentionally not treated as a big integer.
fn is_big_int_11(value: &str) -> bool {
    let Some((_, rest)) = super::split_sign(value) else {
        return false;
    };
    let cleaned = strip_underscores(rest);
    let rest: &str = &cleaned;
    if let Some(body) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        !body.is_empty() && body.bytes().all(|b| b.is_ascii_hexdigit())
    } else if let Some(body) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
        !body.is_empty() && body.bytes().all(|b| b == b'0' || b == b'1')
    } else if rest.len() > 1 && rest.starts_with('0') {
        // C-style octal (`0777`): a leading zero then octal digits.
        rest[1..].bytes().all(|b| (b'0'..=b'7').contains(&b))
    } else {
        is_big_decimal_int_11(value)
    }
}

fn try_parse_int_11(value: &str) -> Option<i64> {
    let (negative, rest) = super::split_sign(value)?;

    // Strip underscores for parsing (allocation-free unless any are present).
    let cleaned = strip_underscores(rest);
    let clean: &str = &cleaned;
    if clean.is_empty() {
        return None;
    }

    let result = if clean.starts_with("0x") || clean.starts_with("0X") {
        // Hexadecimal
        super::from_radix_unsigned(&clean[2..], 16)?
    } else if clean.starts_with("0b") || clean.starts_with("0B") {
        // Binary
        super::from_radix_unsigned(&clean[2..], 2)?
    } else if clean.starts_with('0') && clean.len() > 1 && clean.bytes().all(|b| b.is_ascii_digit())
    {
        // C-style octal: 0777
        super::from_radix_unsigned(&clean[1..], 8)?
    } else if clean.contains(':') {
        // Sexagesimal: 1:30 = 90
        parse_sexagesimal_int(clean)?
    } else {
        clean.parse::<i64>().ok()?
    };

    if negative {
        Some(-result)
    } else {
        Some(result)
    }
}

fn parse_sexagesimal_int(value: &str) -> Option<i64> {
    let mut result: i64 = 0;
    for (i, part) in value.split(':').enumerate() {
        // A per-segment sign (`5:+30`) is not sexagesimal; the value's own sign is
        // already handled by `split_sign` before this is reached.
        if part.starts_with(['+', '-']) {
            return None;
        }
        let n: i64 = part.parse().ok()?;
        // Only the leading segment may exceed 59 (e.g. `90:00`); every later
        // segment is a base-60 digit and must stay in 0..60. Keying this off the
        // position rather than the running total keeps a legitimate leading `0`
        // (as in `0:30`) from being mistaken for the first segment again later.
        if i > 0 && !(0..60).contains(&n) {
            return None;
        }
        // A value too large to hold in an i64 is not representable as an int, so
        // fall back to a string (`None`) instead of overflowing, exactly as the
        // plain-decimal and radix paths above do via `.ok()?`.
        result = result.checked_mul(60)?.checked_add(n)?;
    }
    Some(result)
}

fn try_parse_float_11(value: &str, pyyaml_compat: bool) -> Option<f64> {
    if let Some(f) = super::parse_special_float(value) {
        return Some(f);
    }

    // Strip underscores (allocation-free unless any are present).
    let cleaned = strip_underscores(value);
    let clean: &str = &cleaned;

    if !clean.bytes().any(|b| matches!(b, b'.' | b'e' | b'E')) {
        return None;
    }

    // Sexagesimal float: 1:30.5
    if clean.contains(':') && clean.contains('.') {
        return parse_sexagesimal_float(clean);
    }

    // PyYAML's resolver is stricter than the lenient 1.1 reading: a base-10 float
    // must carry a `.`, and an exponent must have an explicit sign. So `1e3` and
    // `1.0e3` are plain strings under PyYAML, while `1.5e+3` is a float. The
    // default (non-compat) 1.1 mode keeps the lenient reading.
    if pyyaml_compat && !pyyaml_float_shape(clean) {
        return None;
    }

    clean.parse::<f64>().ok()
}

/// Whether `value` matches PyYAML's base-10 float shape: it contains a `.`, and
/// any `e`/`E` exponent is immediately followed by an explicit `+`/`-` sign.
fn pyyaml_float_shape(value: &str) -> bool {
    if !value.contains('.') {
        return false;
    }
    match value.bytes().position(|b| b == b'e' || b == b'E') {
        Some(pos) => matches!(value.as_bytes().get(pos + 1), Some(b'+' | b'-')),
        None => true,
    }
}

fn parse_sexagesimal_float(value: &str) -> Option<f64> {
    // The sign applies to the whole magnitude, not just the high-order segment
    // (`-1:30.5` is `-90.5`, not `-29.5`). Strip it first and reapply at the end,
    // mirroring the integer path; the callers pass the signed string in as-is.
    let (negative, rest) = super::split_sign(value)?;
    let parts: Vec<&str> = rest.split(':').collect();
    // A sexagesimal value has at least one colon; the caller only routes here
    // when one is present, but guard anyway.
    if parts.len() < 2 {
        return None;
    }

    let last = parts.len() - 1;
    let mut result: f64 = 0.0;
    for (i, part) in parts.iter().enumerate() {
        // Only the final segment may carry a fraction (`1:30.5`). A dot in any
        // earlier segment (`1.5:30`) is not a valid sexagesimal value; PyYAML's
        // resolver rejects it, so it stays a string here too.
        if i != last && part.contains('.') {
            return None;
        }
        // Every segment after the first is a base-60 digit: its integer part
        // must be in 0..60 (`1:70.5` is invalid). The first (high-order) segment
        // is unbounded (`90:00.0`). This mirrors `parse_sexagesimal_int`, which
        // already validated the no-fraction form.
        if i > 0 {
            match part.split('.').next().unwrap_or("").parse::<u32>() {
                Ok(n) if n < 60 => {}
                _ => return None,
            }
        }
        let n: f64 = part.parse().ok()?;
        result = result * 60.0 + n;
    }
    Some(if negative { -result } else { result })
}

fn classify_tagged_11(value: &str, tag: &str, pyyaml_compat: bool) -> ScalarKind {
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
        "!!bool" | "tag:yaml.org,2002:bool" => match try_parse_bool_11(value, pyyaml_compat) {
            Some(b) => ScalarKind::Bool(b),
            None => ScalarKind::Str,
        },
        "!!int" | "tag:yaml.org,2002:int" => {
            if let Some(int) = try_parse_int_11(value) {
                ScalarKind::Int(int)
            } else if is_big_int_11(value) {
                ScalarKind::BigInt
            } else {
                ScalarKind::Str
            }
        }
        "!!float" | "tag:yaml.org,2002:float" => match try_parse_float_11_tagged(value) {
            Some(float) => ScalarKind::Float(float),
            None => ScalarKind::Str,
        },
        "!!merge" | "tag:yaml.org,2002:merge" => ScalarKind::Merge,
        _ => ScalarKind::Str,
    }
}

/// Parse a value carrying an explicit `!!float` tag under YAML 1.1. Unlike the
/// plain-scalar [`try_parse_float_11`] (which requires a `.`/`e`/`E` so a bare
/// integer stays an int), an integer-form value is a conforming float here:
/// `42` resolves to `42.0` and the sexagesimal `1:30` to `90.0`, matching
/// PyYAML. Hexadecimal and octal forms are not part of the float production, so
/// they fall through to a string rather than being coerced.
fn try_parse_float_11_tagged(value: &str) -> Option<f64> {
    if let Some(f) = super::parse_special_float(value) {
        return Some(f);
    }
    // Strip underscores (allocation-free unless any are present).
    let cleaned = strip_underscores(value);
    let clean: &str = &cleaned;
    if clean.contains(':') {
        // Sexagesimal, integer (`1:30`) or fractional (`1:30.5`).
        return parse_sexagesimal_float(clean);
    }
    clean.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::Yaml11Resolver;
    use crate::resolver::{ResolvedValue, Resolver};
    use crate::scanner::ScalarStyle;

    fn plain(value: &str) -> ResolvedValue {
        Yaml11Resolver::default().resolve(value, ScalarStyle::Plain, None)
    }

    fn plain_pyyaml(value: &str) -> ResolvedValue {
        Yaml11Resolver {
            pyyaml_compat: true,
        }
        .resolve(value, ScalarStyle::Plain, None)
    }

    #[test]
    fn expanded_booleans() {
        for value in ["yes", "Yes", "YES", "on", "On", "y", "Y", "true", "True"] {
            assert_eq!(plain(value), ResolvedValue::Bool(true), "{value:?}");
        }
        for value in ["no", "No", "NO", "off", "Off", "n", "N", "false", "False"] {
            assert_eq!(plain(value), ResolvedValue::Bool(false), "{value:?}");
        }
    }

    #[test]
    fn pyyaml_compat_drops_single_letter_booleans() {
        // PyYAML omits bare y/Y/n/N from its bool set; under compat they stay
        // strings, while the multi-letter forms remain booleans.
        for value in ["y", "Y", "n", "N"] {
            assert!(
                matches!(plain_pyyaml(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
        assert_eq!(plain_pyyaml("yes"), ResolvedValue::Bool(true));
        assert_eq!(plain_pyyaml("on"), ResolvedValue::Bool(true));
        assert_eq!(plain_pyyaml("off"), ResolvedValue::Bool(false));
        assert_eq!(plain_pyyaml("no"), ResolvedValue::Bool(false));
    }

    #[test]
    fn pyyaml_compat_narrows_floats_and_leading_underscore() {
        // PyYAML keeps `1e3`/`1.0e3` (no dot, or unsigned exponent) and a
        // leading-underscore value as strings; the lenient default 1.1 reading
        // (`plain`) parses them as numbers.
        for value in ["1e3", "1.0e3", "_5", "_1.5"] {
            assert!(
                matches!(plain_pyyaml(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
        assert_eq!(plain_pyyaml("1.5e+3"), ResolvedValue::Float(1500.0));
        assert_eq!(plain_pyyaml("5_"), ResolvedValue::Int(5));
        // The default 1.1 mode stays lenient.
        assert_eq!(plain("1e3"), ResolvedValue::Float(1000.0));
        assert_eq!(plain("_5"), ResolvedValue::Int(5));
    }

    #[test]
    fn c_style_octal() {
        // YAML 1.1 reads a leading-zero number as octal: 0777 == 511.
        assert_eq!(plain("0777"), ResolvedValue::Int(511));
        // A bare zero stays decimal: the octal branch needs more than one digit,
        // otherwise `0` would parse an empty octal body and fall through.
        assert_eq!(plain("0"), ResolvedValue::Int(0));
    }

    #[test]
    fn shares_the_common_scalar_forms() {
        assert_eq!(plain("42"), ResolvedValue::Int(42));
        assert_eq!(plain("1.5"), ResolvedValue::Float(1.5));
        assert_eq!(plain("~"), ResolvedValue::Null);
        assert!(matches!(plain("hello"), ResolvedValue::String(_)));
    }

    #[test]
    fn quoting_forces_string_even_for_yes() {
        assert_eq!(
            Yaml11Resolver::default().resolve("yes", ScalarStyle::DoubleQuoted, None),
            ResolvedValue::String("yes".to_owned())
        );
    }

    #[test]
    fn sexagesimal_integers() {
        // Classic base-60: 1:30 == 90, and a leading segment may exceed 59.
        assert_eq!(plain("1:30"), ResolvedValue::Int(90));
        assert_eq!(plain("90:00"), ResolvedValue::Int(5400));
        // A leading zero segment must not be mistaken for "no segment yet": a
        // later out-of-range segment is still rejected.
        assert_eq!(plain("0:30"), ResolvedValue::Int(30));
        assert!(matches!(plain("0:90"), ResolvedValue::String(_)));
        // A chain long enough to overflow i64 during base-60 accumulation must
        // fall back to a string, not panic (debug) or wrap (release). Found by
        // the `decode` fuzz target.
        assert!(matches!(
            plain("2:0:0:0:08:0:0:0:0:0:08:0:0:0:0:0"),
            ResolvedValue::String(_)
        ));
    }

    #[test]
    fn parses_radix_prefixed_and_signed_integers() {
        // Hex, binary, and their uppercase prefixes each have their own branch.
        assert_eq!(plain("0xFF"), ResolvedValue::Int(255));
        assert_eq!(plain("0XFF"), ResolvedValue::Int(255));
        assert_eq!(plain("0b1010"), ResolvedValue::Int(10));
        assert_eq!(plain("0B1010"), ResolvedValue::Int(10));
        // Underscores are stripped before parsing.
        assert_eq!(plain("1_000"), ResolvedValue::Int(1000));
        // The negative branch must actually negate the parsed magnitude.
        assert_eq!(plain("-5"), ResolvedValue::Int(-5));
        assert_eq!(plain("-0xFF"), ResolvedValue::Int(-255));
        // A sign *inside* the radix body is not an integer (`0x-5`, not -5); the
        // C-style octal and sexagesimal segment forms reject it too.
        for value in ["0x-5", "0x+5", "0b-1", "5:+30"] {
            assert!(
                matches!(plain(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
    }

    #[test]
    fn parses_floats_via_each_marker() {
        // A dot, a lowercase exponent, and an uppercase exponent each qualify a
        // scalar as a float; the guard must accept any one of them.
        assert_eq!(plain("1.5"), ResolvedValue::Float(1.5));
        assert_eq!(plain("1e3"), ResolvedValue::Float(1000.0));
        assert_eq!(plain("1E3"), ResolvedValue::Float(1000.0));
        // Underscores are stripped here too.
        assert_eq!(plain("1_000.5"), ResolvedValue::Float(1000.5));
    }

    #[test]
    fn sexagesimal_floats() {
        // Base-60 with a fractional final segment: 1:30.5 == 90.5, and the
        // running accumulation must hold across more than two segments.
        assert_eq!(plain("1:30.5"), ResolvedValue::Float(90.5));
        assert_eq!(plain("1:2:3.5"), ResolvedValue::Float(3723.5));
        // The sexagesimal-float path requires BOTH a colon and a dot. A colon
        // with an exponent but no dot (`1:3e2`) is not sexagesimal and must stay
        // a string, not be read as 1*60 + 300.
        assert!(matches!(plain("1:3e2"), ResolvedValue::String(_)));
        // The high-order segment is unbounded; later segments are base-60 digits.
        assert_eq!(plain("90:00.0"), ResolvedValue::Float(5400.0));
        // The sign applies to the whole magnitude, not just the first segment.
        assert_eq!(plain("-1:30.5"), ResolvedValue::Float(-90.5));
        assert_eq!(plain("+1:30.5"), ResolvedValue::Float(90.5));
        // A base-60 digit out of range, a fraction on a non-final segment, or a
        // fractional first segment are all invalid and stay strings.
        for value in ["1:70.5", "1:60.0", "1.5:30", "1:5.5:30"] {
            assert!(
                matches!(plain(value), ResolvedValue::String(_)),
                "{value:?}"
            );
        }
    }

    #[test]
    fn tagged_scalars_classify_by_tag() {
        let r = Yaml11Resolver::default();
        // A conforming value resolves to the tagged type.
        assert_eq!(
            r.resolve("~", ScalarStyle::Plain, Some("!!null")),
            ResolvedValue::Null
        );
        assert_eq!(
            r.resolve("yes", ScalarStyle::Plain, Some("!!bool")),
            ResolvedValue::Bool(true)
        );
        assert_eq!(
            r.resolve("0777", ScalarStyle::Plain, Some("!!int")),
            ResolvedValue::Int(511)
        );
        assert_eq!(
            r.resolve("1.5", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::Float(1.5)
        );
        // Integer-form and sexagesimal values are conforming floats under an
        // explicit tag (`42` -> `42.0`, `1:30` -> `90.0`), matching PyYAML.
        assert_eq!(
            r.resolve("42", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::Float(42.0)
        );
        assert_eq!(
            r.resolve("1:30", ScalarStyle::Plain, Some("!!float")),
            ResolvedValue::Float(90.0)
        );
    }

    #[test]
    fn explicit_tag_with_nonconforming_content_stays_a_string() {
        // An explicit core tag whose content does not match the type is kept as a
        // string, not coerced to a wrong-but-valid value (`!!int nope` was `0`).
        let r = Yaml11Resolver::default();
        for (value, tag) in [
            ("nope", "!!int"),
            ("x", "!!float"),
            ("maybe", "!!bool"),
            ("text", "!!null"),
        ] {
            assert_eq!(
                r.resolve(value, ScalarStyle::Plain, Some(tag)),
                ResolvedValue::String(value.to_owned()),
                "{tag} {value:?}"
            );
        }
    }

    #[test]
    fn merge_tag_resolves_to_the_merge_key() {
        // The !!merge tag classifies as a merge key regardless of the text, and
        // the default `resolve` maps that to the literal `<<`.
        assert_eq!(
            Yaml11Resolver::default().resolve("whatever", ScalarStyle::Plain, Some("!!merge")),
            ResolvedValue::String("<<".to_owned())
        );
    }
}
