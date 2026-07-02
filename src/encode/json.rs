//! JSON serialization of the internal [`Value`] tree.
//!
//! JSON is the lossy subset of YAML, so this emitter applies a fixed projection:
//!
//! * tags are dropped (`Value::Tagged` emits its inner value);
//! * non-finite floats (`NaN`, `±Inf`) become `null`, since JSON has no literal
//!   for them (this matches orjson);
//! * a non-string scalar mapping key is stringified to its JSON scalar text
//!   (`1` -> `"1"`, `true` -> `"true"`, `null` -> `"null"`), matching the
//!   canonical projection the YAML test suite uses;
//! * a collection used as a mapping key has no JSON representation and is an
//!   error.

use std::fmt::Write;

use crate::decode::Value;

/// Formatting options for JSON output.
#[derive(Clone, Copy, Default)]
pub struct JsonOptions {
    /// Spaces per indent level; `0` emits compact output (no spaces, no breaks).
    pub indent: usize,
    /// Sort object keys lexicographically by their JSON string form.
    pub sort_keys: bool,
}

/// Serialize a value tree to JSON bytes, or return an error message for content
/// JSON cannot represent (a collection used as a mapping key).
pub fn encode_json(value: &Value<'_>, options: &JsonOptions) -> Result<Vec<u8>, String> {
    let mut out = String::new();
    write_value(&mut out, value, options, 0)?;
    Ok(out.into_bytes())
}

fn write_value(
    out: &mut String,
    value: &Value<'_>,
    options: &JsonOptions,
    depth: usize,
) -> Result<(), String> {
    // Grow the native stack on demand so writing a deeply nested value cannot
    // overflow a small thread stack; the recursion re-enters here per level. See
    // [`crate::stack`].
    crate::stack::guard(|| write_value_inner(out, value, options, depth))
}

fn write_value_inner(
    out: &mut String,
    value: &Value<'_>,
    options: &JsonOptions,
    depth: usize,
) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        // A big integer's decimal text is a valid JSON number.
        Value::BigInt(s) => out.push_str(s),
        Value::Float(f) => write_float(out, *f),
        Value::String(s) => write_string(out, s),
        Value::Sequence(items) => write_array(out, items, options, depth)?,
        Value::Mapping(pairs) => write_object(out, pairs, options, depth)?,
        // JSON has no tags; emit the value the tag wraps.
        Value::Tagged(_, inner) => write_value(out, inner, options, depth)?,
    }
    Ok(())
}

fn write_float(out: &mut String, value: f64) {
    if !value.is_finite() {
        // NaN and ±Infinity are not valid JSON; project them to null.
        out.push_str("null");
        return;
    }
    // Reuse the shared canonical spelling (the YAML path uses it too), so a large
    // or tiny magnitude emits in scientific notation (`1.0e+16`) rather than
    // Rust's default `{}`, which never uses exponents and would expand `1e100` to
    // 101 digits. The `.`/mantissa keeps it distinguishable from an integer, and
    // the special values are already handled above (so `.inf`/`.nan` never reach
    // here). The exponent form (`1.0e+16`) is valid JSON and parses back exactly.
    out.push_str(&crate::emit_util::canonical_float(value));
}

fn write_array(
    out: &mut String,
    items: &[Value<'_>],
    options: &JsonOptions,
    depth: usize,
) -> Result<(), String> {
    if items.is_empty() {
        out.push_str("[]");
        return Ok(());
    }
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        newline_indent(out, options, depth + 1);
        write_value(out, item, options, depth + 1)?;
    }
    newline_indent(out, options, depth);
    out.push(']');
    Ok(())
}

fn write_object(
    out: &mut String,
    pairs: &[(Value<'_>, Value<'_>)],
    options: &JsonOptions,
    depth: usize,
) -> Result<(), String> {
    if pairs.is_empty() {
        out.push_str("{}");
        return Ok(());
    }
    // Resolve every key to its JSON string first (this is where a collection key
    // is rejected), so sorting and emission work on plain strings.
    let mut entries: Vec<(String, &Value<'_>)> = Vec::with_capacity(pairs.len());
    for (key, val) in pairs {
        entries.push((key_string(key)?, val));
    }
    if options.sort_keys {
        entries.sort_by(|a, b| a.0.cmp(&b.0));
    }

    out.push('{');
    for (i, (key, val)) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        newline_indent(out, options, depth + 1);
        write_string(out, key);
        out.push(':');
        if options.indent > 0 {
            out.push(' ');
        }
        write_value(out, val, options, depth + 1)?;
    }
    newline_indent(out, options, depth);
    out.push('}');
    Ok(())
}

/// The JSON object-key string for a mapping key. Scalars stringify to their JSON
/// scalar text; a collection key has no JSON form and is an error.
fn key_string(key: &Value<'_>) -> Result<String, String> {
    match key {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Int(i) => Ok(i.to_string()),
        Value::BigInt(s) => Ok(s.to_string()),
        Value::Float(f) => {
            let mut s = String::new();
            write_float(&mut s, *f);
            Ok(s)
        }
        Value::String(s) => Ok(s.as_ref().to_owned()),
        Value::Tagged(_, inner) => key_string(inner),
        Value::Sequence(_) | Value::Mapping(_) => {
            Err("a collection cannot be a JSON object key".to_owned())
        }
    }
}

fn newline_indent(out: &mut String, options: &JsonOptions, depth: usize) {
    if options.indent > 0 {
        out.push('\n');
        for _ in 0..(options.indent * depth) {
            out.push(' ');
        }
    }
}

/// Write `value` as a quoted, escaped JSON string.
fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            // U+2028/U+2029 are valid in JSON but break JavaScript `eval`/JSONP;
            // escape them for parity with orjson and safe embedding in scripts.
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out.push('"');
}
