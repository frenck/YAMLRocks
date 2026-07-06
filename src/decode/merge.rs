//! YAML merge-key (`<<`) resolution for the fast decode path.

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
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
/// is O(1) via a hash map rather than an O(n) linear scan (a single `<<` over a
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
        Value::Timestamp(ts) => {
            let _ = write!(out, "9:{};", ts.to_iso());
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
            // Rebuild the mapping in source order. An explicit (non-merge) key is
            // inserted at its position, overriding in place a value merged in
            // earlier if the same key was already pulled in (explicit keys win). A
            // `<<` expands its mapping at its own position, contributing only keys
            // not already present, so an explicit key and an earlier merge both
            // win. This keeps the fast path's key order identical to the annotated
            // and round-trip paths, which build the same shape through Python dict
            // insertion. `index` maps a key signature to its slot in `result` for
            // O(1) membership, so a `<<` over a large mapping stays linear.
            let mut result: Vec<(Value<'_>, Value<'_>)> = Vec::with_capacity(pairs.len());
            let mut index: HashMap<String, usize> = HashMap::new();
            for (key, val) in std::mem::take(pairs) {
                if is_merge_key(&key) {
                    if let Some(unmerged) = merge_into(&mut result, &mut index, val) {
                        // The `<<` value is not a mapping (or list of mappings) the
                        // parser can fold, e.g. a custom tag such as
                        // `<<: !include other.yaml` (a deferred marker resolved by
                        // the host). Keep it under the literal `<<` key rather than
                        // dropping it, so the host can run its own merge pass. (The
                        // marker is internal; it must not escape into the data.)
                        let sig = key_sig(&literal_merge_key());
                        match index.entry(sig) {
                            Entry::Vacant(slot) => {
                                slot.insert(result.len());
                                result.push((literal_merge_key(), unmerged));
                            }
                            // A second preserved `<<` in the same mapping: a
                            // mapping holds one `<<` slot, so collect into a
                            // single `<<` sequence in document order rather than
                            // dropping all but the first, matching the sequence
                            // form and the annotated/round-trip paths. The host
                            // applies its own merge semantics over the list.
                            Entry::Occupied(slot) => {
                                let idx = *slot.get();
                                collect_preserved(&mut result[idx].1, unmerged);
                            }
                        }
                    }
                } else {
                    let sig = key_sig(&key);
                    match index.get(&sig) {
                        // An explicit key already merged in: override its value in
                        // place, keeping the position it first appeared at.
                        Some(&i) => result[i].1 = val,
                        None => {
                            index.insert(sig, result.len());
                            result.push((key, val));
                        }
                    }
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

/// Collapse duplicate keys in a mapping's pairs to last-wins, keeping each key at
/// the position it first appeared, so the pairs read as the mapping's data value
/// (a repeated key resolves to its last value, exactly as building a Python dict
/// would). Only used for a `<<` merge source, where a duplicate would otherwise
/// be resolved first-wins. Runs only when a `<<` is present, over the (typically
/// small) merge source, so the extra pass is off the hot path.
fn dedup_last_wins<'i>(pairs: Vec<(Value<'i>, Value<'i>)>) -> Vec<(Value<'i>, Value<'i>)> {
    let mut seen: HashMap<String, usize> = HashMap::with_capacity(pairs.len());
    let mut out: Vec<(Value<'i>, Value<'i>)> = Vec::with_capacity(pairs.len());
    for (key, val) in pairs {
        match seen.entry(key_sig(&key)) {
            Entry::Occupied(slot) => out[*slot.get()].1 = val,
            Entry::Vacant(slot) => {
                slot.insert(out.len());
                out.push((key, val));
            }
        }
    }
    out
}

/// Collect a later preserved `<<` value into the one already under the literal
/// merge key, building a single `<<` sequence in document order. Nothing is
/// merged here: the values are deferred (a custom tag the host resolves), so we
/// only gather them so the host can apply its own merge semantics over the list.
/// A sequence value is flattened one level, so repeated deferred `<<`
/// (`<<: a` / `<<: b`) and the sequence form (`<<: [a, b]`) produce the same
/// shape, and the host sees every value instead of silently losing all but one.
fn collect_preserved<'i>(existing: &mut Value<'i>, new: Value<'i>) {
    let mut items = match std::mem::replace(existing, Value::Null) {
        Value::Sequence(items) => items,
        other => vec![other],
    };
    match new {
        Value::Sequence(more) => items.extend(more),
        other => items.push(other),
    }
    *existing = Value::Sequence(items);
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
    index: &mut HashMap<String, usize>,
    merge: Value<'i>,
) -> Option<Value<'i>> {
    match merge {
        Value::Mapping(pairs) => {
            // A merge source that repeats a key contributes that key's *last*
            // value (the value the mapping has as data), so collapse duplicates
            // last-wins before merging. Without this the first duplicate would win
            // here, so `<<: *a` with `&a {x: 1, x: 2}` merged `x: 1` while the same
            // anchor materializes as `{x: 2}` everywhere else (and in PyYAML).
            for (key, val) in dedup_last_wins(pairs) {
                // A key already present (an explicit key, or an earlier merge) is
                // not overridden; only a missing key is pulled in, at its position.
                if let Entry::Vacant(slot) = index.entry(key_sig(&key)) {
                    slot.insert(result.len());
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
                if let Some(unmerged) = merge_into(result, index, item) {
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
    fn repeated_unmergeable_merge_keys_collect_into_a_sequence() {
        // Two `<<` whose values cannot be folded (a deferred tag, a scalar)
        // collapse to one `<<` slot; keep both as a sequence in document order
        // rather than dropping all but the first.
        let mut doc = Value::Mapping(vec![(merge_key(), s("a")), (merge_key(), s("b"))]);
        apply_merge_keys(&mut doc);
        assert_eq!(
            get(&doc, "<<"),
            Some(&Value::Sequence(vec![s("a"), s("b")]))
        );
    }

    #[test]
    fn repeated_merge_flattens_a_preserved_sequence_one_level() {
        // A `<<` sequence's leftovers meeting a later `<<` flatten into the same
        // sequence, matching the shape the `<<: [a, b]` form already produces.
        let mut doc = Value::Mapping(vec![
            (merge_key(), Value::Sequence(vec![s("a"), s("b")])),
            (merge_key(), s("c")),
        ]);
        apply_merge_keys(&mut doc);
        assert_eq!(
            get(&doc, "<<"),
            Some(&Value::Sequence(vec![s("a"), s("b"), s("c")]))
        );
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
