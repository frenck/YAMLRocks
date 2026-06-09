//! YAML merge-key (`<<`) resolution for the fast decode path.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Write as _;

use super::Value;

/// The resolved tag of a YAML merge key. A *plain* `<<` scalar resolves to this
/// (`ScalarKind::Merge`); a quoted `"<<"` is an ordinary string and never does.
///
/// The fast-path `Value` tree carries no scalar style, so a bare `String("<<")`
/// cannot say whether it came from a plain `<<` (a merge directive) or a quoted
/// `"<<"` (a literal key). The decoder therefore marks a *real* merge key by
/// wrapping it as a [`Value::Tagged`] under this tag, which is exactly the type a
/// plain `<<` resolves to. Merge detection then keys off the tag, not the text,
/// so a quoted `"<<"` is never mistaken for a merge directive.
pub(super) const MERGE_TAG: &str = "tag:yaml.org,2002:merge";

/// The `Value` a real (plain) `<<` merge key decodes to: the tagged marker, not a
/// bare `String("<<")` (which a quoted `"<<"` also produces).
pub(super) fn merge_key_marker() -> Value<'static> {
    Value::Tagged(MERGE_TAG.to_owned(), Box::new(literal_merge_key()))
}

/// The literal string a `<<` key falls back to when it is not consumed as a merge
/// directive (its value cannot be merged, or the marker turned up outside key
/// position), so the key survives as ordinary data rather than vanishing.
fn literal_merge_key() -> Value<'static> {
    Value::String(Cow::Borrowed("<<"))
}

/// An injective string signature of a mapping key, so membership during a merge
/// is O(1) via a `HashSet` rather than an O(n) linear scan (a single `<<` over a
/// large mapping was otherwise quadratic). It mirrors `Value`'s own equality,
/// which is type-distinct: `Int(1)` and `Float(1.0)` are different keys. Strings
/// are length-prefixed so no value can spoof another's signature.
pub(crate) fn key_sig(value: &Value<'_>) -> String {
    let mut out = String::new();
    write_key_sig(&mut out, value);
    out
}

fn write_key_sig(out: &mut String, value: &Value<'_>) {
    match value {
        Value::Null => out.push('0'),
        Value::Bool(b) => {
            let _ = write!(out, "1{}", u8::from(*b));
        }
        Value::Int(i) => {
            let _ = write!(out, "2:{i};");
        }
        Value::BigInt(s) => {
            let _ = write!(out, "3:{s};");
        }
        Value::Float(f) => {
            let _ = write!(out, "4:{};", f.to_bits());
        }
        Value::String(s) => {
            let _ = write!(out, "5:{}:{s}", s.len());
        }
        Value::Tagged(tag, inner) => {
            let _ = write!(out, "6:{}:{tag}", tag.len());
            write_key_sig(out, inner);
        }
        Value::Sequence(items) => {
            let _ = write!(out, "7:{}:", items.len());
            for item in items {
                write_key_sig(out, item);
            }
        }
        Value::Mapping(pairs) => {
            let _ = write!(out, "8:{}:", pairs.len());
            for (k, v) in pairs {
                write_key_sig(out, k);
                write_key_sig(out, v);
            }
        }
    }
}

/// Resolve YAML merge keys (`<<`) in place, matching PyYAML/ruamel behavior:
/// the mapping(s) referenced by a `<<` key are merged in, with explicitly
/// written keys taking precedence and earlier merges winning over later ones.
pub(super) fn apply_merge_keys(value: &mut Value<'_>) {
    // Grow the native stack on demand so walking a deeply nested tree (bounded by
    // `MAX_DEPTH`) cannot overflow a small thread stack. See [`crate::stack`].
    crate::stack::guard(|| apply_merge_keys_inner(value))
}

/// Replace every internal merge marker in `value` with the literal string `<<`,
/// without applying any merge semantics. A marker is only a merge directive when
/// it is a *direct* key of a mapping; one that turns up elsewhere (nested inside a
/// complex mapping/sequence key) is ordinary data, and it must never reach the
/// FFI layer, where it would surface as a bogus `!!merge` tag.
fn strip_merge_markers(value: &mut Value<'_>) {
    match value {
        Value::Tagged(tag, inner) => {
            if tag.as_str() == MERGE_TAG {
                *value = literal_merge_key();
            } else {
                strip_merge_markers(inner);
            }
        }
        Value::Sequence(items) => items.iter_mut().for_each(strip_merge_markers),
        Value::Mapping(pairs) => {
            for (key, val) in pairs.iter_mut() {
                strip_merge_markers(key);
                strip_merge_markers(val);
            }
        }
        _ => {}
    }
}

fn apply_merge_keys_inner(value: &mut Value<'_>) {
    match value {
        Value::Mapping(pairs) => {
            for (key, val) in pairs.iter_mut() {
                // A *direct* `<<` marker key is a merge directive, resolved below.
                // A marker nested inside a complex (mapping/sequence) key is just
                // the literal string `<<`, so strip it here so it never escapes.
                if !is_merge_key(key) {
                    strip_merge_markers(key);
                }
                apply_merge_keys(val);
            }
            if !pairs.iter().any(|(k, _)| is_merge_key(k)) {
                return;
            }
            let mut result: Vec<(Value<'_>, Value<'_>)> = Vec::with_capacity(pairs.len());
            let mut merges: Vec<(Value<'_>, Value<'_>)> = Vec::new();
            for (key, val) in std::mem::take(pairs) {
                if is_merge_key(&key) {
                    merges.push((key, val));
                } else {
                    result.push((key, val));
                }
            }
            // Track present keys by signature so a merge skips already-present keys
            // in O(1); the explicit (non-merge) keys seed it.
            let mut seen: HashSet<String> = result.iter().map(|(k, _)| key_sig(k)).collect();
            for (_marker, merge) in merges {
                if let Some(unmerged) = merge_into(&mut result, &mut seen, merge) {
                    // The `<<` value is not a mapping (or list of mappings) the
                    // parser can fold, e.g. a custom tag such as
                    // `<<: !include other.yaml` (a deferred marker resolved by
                    // the host). Keep the key as the literal string `<<` with its
                    // resolved value rather than silently dropping it, so the host
                    // can run its own merge pass over it. (The marker is internal;
                    // it must not escape into the returned data.)
                    result.push((literal_merge_key(), unmerged));
                }
            }
            *pairs = result;
        }
        Value::Sequence(items) => {
            for item in items {
                apply_merge_keys(item);
            }
        }
        Value::Tagged(tag, inner) => {
            if tag.as_str() == MERGE_TAG {
                // A merge marker that survived outside key position (e.g. an
                // anchored plain `<<` aliased into a value) is just the literal
                // string `<<`, so normalize it before it reaches the FFI layer.
                *value = literal_merge_key();
            } else {
                apply_merge_keys(inner);
            }
        }
        _ => {}
    }
}

pub(super) fn is_merge_key(key: &Value<'_>) -> bool {
    matches!(key, Value::Tagged(tag, _) if tag == MERGE_TAG)
}

/// Merge a `<<` value (a mapping, or a sequence of mappings) into `result`,
/// adding only keys that are not already present.
///
/// Returns `Some(value)` when the value could not be merged because it is not a
/// mapping or list of mappings (a custom tag, a scalar, ...), so the caller can
/// preserve it under `<<` instead of dropping it. An empty (`null`) merge value
/// contributes nothing and is ignored.
fn merge_into<'i>(
    result: &mut Vec<(Value<'i>, Value<'i>)>,
    seen: &mut HashSet<String>,
    merge: Value<'i>,
) -> Option<Value<'i>> {
    match merge {
        Value::Mapping(pairs) => {
            for (key, val) in pairs {
                // `insert` returns false when the key is already present (an
                // explicit key, or an earlier merge), so it is not overridden.
                if seen.insert(key_sig(&key)) {
                    result.push((key, val));
                }
            }
            None
        }
        Value::Sequence(items) => {
            // Merge each mapping element; keep any non-mergeable elements so they
            // can be preserved under `<<` rather than silently lost.
            let mut leftover = Vec::new();
            for item in items {
                if let Some(unmerged) = merge_into(result, seen, item) {
                    leftover.push(unmerged);
                }
            }
            (!leftover.is_empty()).then_some(Value::Sequence(leftover))
        }
        // An empty merge contributes nothing; drop it silently as before.
        Value::Null => None,
        // Anything else (a custom tag, scalar, ...) is handed back to preserve.
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use super::apply_merge_keys;
    use crate::decode::Value;
    use std::borrow::Cow;

    fn s(text: &'static str) -> Value<'static> {
        Value::String(Cow::Borrowed(text))
    }

    fn merge_key() -> Value<'static> {
        super::merge_key_marker()
    }

    /// Look up a key in a mapping value, for assertions.
    fn get<'a>(value: &'a Value<'static>, key: &str) -> Option<&'a Value<'static>> {
        match value {
            Value::Mapping(pairs) => pairs
                .iter()
                .find(|(k, _)| matches!(k, Value::String(t) if t.as_ref() == key))
                .map(|(_, v)| v),
            _ => None,
        }
    }

    #[test]
    fn merges_a_single_mapping_without_overriding_explicit_keys() {
        let base = Value::Mapping(vec![(s("a"), Value::Int(1)), (s("b"), Value::Int(2))]);
        let mut doc = Value::Mapping(vec![
            (merge_key(), base),
            (s("b"), Value::Int(20)),
            (s("c"), Value::Int(3)),
        ]);
        apply_merge_keys(&mut doc);
        // Explicit `b` wins; `a` is pulled in from the merge; `c` stays.
        assert_eq!(get(&doc, "a"), Some(&Value::Int(1)));
        assert_eq!(get(&doc, "b"), Some(&Value::Int(20)));
        assert_eq!(get(&doc, "c"), Some(&Value::Int(3)));
        // The `<<` key itself is consumed.
        assert_eq!(get(&doc, "<<"), None);
    }

    #[test]
    fn merges_a_sequence_of_mappings_earlier_winning() {
        // `<<: [first, second]`: earlier mappings win over later ones.
        let first = Value::Mapping(vec![(s("a"), Value::Int(1))]);
        let second = Value::Mapping(vec![(s("a"), Value::Int(99)), (s("b"), Value::Int(2))]);
        let mut doc = Value::Mapping(vec![(merge_key(), Value::Sequence(vec![first, second]))]);
        apply_merge_keys(&mut doc);
        assert_eq!(get(&doc, "a"), Some(&Value::Int(1)));
        assert_eq!(get(&doc, "b"), Some(&Value::Int(2)));
    }

    #[test]
    fn preserves_non_mapping_merge_value() {
        // A `<<` whose value is a custom tag (or any non-mapping) is preserved
        // under `<<`, not dropped, so a host can resolve it later.
        let tagged = Value::Tagged("!t".to_owned(), Box::new(s("foo")));
        let mut doc = Value::Mapping(vec![(s("existing"), s("value")), (merge_key(), tagged)]);
        apply_merge_keys(&mut doc);
        assert_eq!(get(&doc, "existing"), Some(&s("value")));
        match get(&doc, "<<") {
            Some(Value::Tagged(tag, _)) => assert_eq!(tag, "!t"),
            other => panic!("expected the << key preserved with its tag, got {other:?}"),
        }
    }

    #[test]
    fn empty_merge_value_is_dropped() {
        // An empty `<<:` (null) contributes nothing and leaves no `<<` key.
        let mut doc = Value::Mapping(vec![(s("a"), Value::Int(1)), (merge_key(), Value::Null)]);
        apply_merge_keys(&mut doc);
        assert_eq!(get(&doc, "a"), Some(&Value::Int(1)));
        assert_eq!(get(&doc, "<<"), None);
    }

    #[test]
    fn recurses_into_sequence_items() {
        // A merge nested inside a sequence element must still be resolved.
        let base = Value::Mapping(vec![(s("a"), Value::Int(1))]);
        let nested = Value::Mapping(vec![(merge_key(), base)]);
        let mut doc = Value::Sequence(vec![nested]);
        apply_merge_keys(&mut doc);
        match &doc {
            Value::Sequence(items) => {
                assert_eq!(get(&items[0], "a"), Some(&Value::Int(1)));
                assert_eq!(get(&items[0], "<<"), None);
            }
            other => panic!("expected sequence, got {other:?}"),
        }
    }

    #[test]
    fn recurses_into_tagged_values() {
        // A merge nested inside a tagged value must still be resolved.
        let base = Value::Mapping(vec![(s("a"), Value::Int(1))]);
        let inner = Value::Mapping(vec![(merge_key(), base)]);
        let mut doc = Value::Tagged("!custom".to_owned(), Box::new(inner));
        apply_merge_keys(&mut doc);
        match &doc {
            Value::Tagged(_, inner) => {
                assert_eq!(get(inner, "a"), Some(&Value::Int(1)));
                assert_eq!(get(inner, "<<"), None);
            }
            other => panic!("expected tagged value, got {other:?}"),
        }
    }
}
