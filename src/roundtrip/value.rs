//! Conversion between the round-trip AST and plain Python values.
//!
//! `node_to_python`/`node_to_python_with` resolve a `YamlNode` (following
//! aliases via a precomputed anchor map) to a native Python object using the
//! YAML 1.1 or 1.2 scalar schema; `python_to_node` builds a `YamlNode` from a
//! Python value for assignment back into a `YAMLRocksDocument`.

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyList, PyString, PyTuple};

use crate::resolver::{ResolvedValue, Schema};
use crate::roundtrip::ast::{YamlNode, YamlNodeKind};
use crate::scanner::ScalarStyle;

/// Resolve a node to a plain Python value (scalars, and `to_dict`/`unwrap`),
/// using `schema` for scalar typing (so under 1.1 `yes`/`no` are booleans and
/// `0777` an octal int). `anchors` resolves `*alias` nodes.
pub fn node_to_python_with(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
    anchors: &HashMap<String, YamlNode>,
) -> Py<PyAny> {
    let mut cache = HashMap::new();
    node_to_python_cached(py, node, schema, resolve_timestamps, anchors, &mut cache)
}

/// A per-conversion cache from anchor name to the single Python object built for
/// the anchored node. Sharing this across a whole-document walk makes every
/// `*alias` resolve to the *same* Python object as its `&anchor`, matching
/// PyYAML (so `d['base'] is d['ref']`), instead of an independent copy.
pub(crate) type ObjectCache = HashMap<String, Py<PyAny>>;

/// Resolve a scalar node to a Python object, honoring timestamp resolution. When
/// `resolve_timestamps` is set (`OPT_TIMESTAMPS`, or `OPT_PYYAML_COMPAT` which
/// implies it), a plain, untagged scalar matching a timestamp shape becomes a
/// `date`/`datetime`, as on the fast path; every other case falls through to the
/// normal scalar resolution.
fn scalar_to_py(
    py: Python<'_>,
    value: &str,
    style: ScalarStyle,
    tag: Option<&str>,
    schema: Schema,
    resolve_timestamps: bool,
) -> Py<PyAny> {
    if resolve_timestamps && style == ScalarStyle::Plain && tag.is_none() {
        if let Some(ts) = crate::resolver::timestamp::parse(value) {
            if let Ok(obj) = crate::ffi::convert::timestamp_to_py(py, &ts) {
                return obj;
            }
        }
    }
    resolved_to_py(py, schema.resolve(value, style, tag))
}

/// Convert a resolved scalar value into its Python object.
fn resolved_to_py(py: Python<'_>, resolved: ResolvedValue) -> Py<PyAny> {
    match resolved {
        ResolvedValue::Null => py.None(),
        ResolvedValue::Bool(b) => b.into_pyobject(py).unwrap().to_owned().into_any().unbind(),
        ResolvedValue::Int(i) => i.into_pyobject(py).unwrap().into_any().unbind(),
        ResolvedValue::BigInt(s) => crate::ffi::py_int_from_decimal(py, &s),
        ResolvedValue::Float(f) => PyFloat::new(py, f).into_any().unbind(),
        ResolvedValue::String(s) => PyString::new(py, &s).into_any().unbind(),
    }
}

/// Resolve a node to a Python value, sharing object identity across aliases via
/// `cache`. An `&anchor`'s converted object is recorded under its name; a later
/// `*alias` returns that same object (a new reference to it). Because a valid
/// YAML alias can only reference an earlier anchor, the cache is always
/// populated by the time the alias is reached; the `anchors` map is a defensive
/// fallback for the (invalid) forward-reference case.
pub(crate) fn node_to_python_cached(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> Py<PyAny> {
    // Grow the native stack on demand so converting a deeply nested document
    // (bounded by the composer's `MAX_DEPTH`) cannot overflow a small thread
    // stack; the recursion re-enters here at each level. See [`crate::stack`].
    crate::stack::guard(|| {
        node_to_python_cached_inner(py, node, schema, resolve_timestamps, anchors, cache)
    })
}

fn node_to_python_cached_inner(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> Py<PyAny> {
    // An alias is a reference to an existing object, never a new one, so resolve
    // it (cache first, anchor-map fallback) and return without caching.
    if let YamlNodeKind::Alias(name) = &node.kind {
        if let Some(obj) = cache.get(name) {
            return obj.clone_ref(py);
        }
        return match anchors.get(name) {
            Some(target) => {
                node_to_python_cached(py, target, schema, resolve_timestamps, anchors, cache)
            }
            None => py.None(),
        };
    }

    let obj = match &node.kind {
        // An empty node can still carry a tag (`k: !!str` is "", `!!int` a typed
        // error, and so on), so resolve the tag against empty content the way the
        // fast decoder does. An untagged null, or a tag that resolves to null,
        // stays `None`.
        YamlNodeKind::Null => match node.tag.as_deref() {
            Some(tag) => resolved_to_py(py, schema.resolve("", ScalarStyle::Plain, Some(tag))),
            None => py.None(),
        },
        YamlNodeKind::Scalar(value, style) => scalar_to_py(
            py,
            value,
            *style,
            node.tag.as_deref(),
            schema,
            resolve_timestamps,
        ),
        YamlNodeKind::Sequence(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(node_to_python_cached(
                    py,
                    item,
                    schema,
                    resolve_timestamps,
                    anchors,
                    cache,
                ))
                .unwrap();
            }
            list.into_any().unbind()
        }
        YamlNodeKind::Mapping(pairs) => {
            let dict = PyDict::new(py);
            if mapping_has_merge_key(pairs) {
                let mut preserved_merge = false;
                for (key, val) in pairs {
                    let py_val =
                        node_to_python_cached(py, val, schema, resolve_timestamps, anchors, cache);
                    if is_ast_merge_key(key) {
                        // Fold the merge value's keys in, keeping any explicit
                        // key already set. A non-mergeable value stays under the
                        // literal `<<`, as the fast path preserves it.
                        if let Some(preserve) =
                            merge_converted_into(&dict, py_val.bind(py)).unwrap()
                        {
                            let py_key = node_to_python_key(
                                py,
                                key,
                                schema,
                                resolve_timestamps,
                                anchors,
                                cache,
                            );
                            store_preserved_merge(
                                &dict,
                                py_key.into_bound(py),
                                preserve,
                                &mut preserved_merge,
                            )
                            .unwrap();
                        }
                    } else {
                        // An explicit key wins over a merge, so it overwrites.
                        dict.set_item(
                            node_to_python_key(py, key, schema, resolve_timestamps, anchors, cache),
                            py_val,
                        )
                        .unwrap();
                        // An explicit `"<<"` overwrote the preserved slot; it is
                        // plain data now, so later merges must not collect into it.
                        if preserved_merge && is_literal_merge_key_text(key) {
                            preserved_merge = false;
                        }
                    }
                }
            } else {
                for (key, val) in pairs {
                    dict.set_item(
                        node_to_python_key(py, key, schema, resolve_timestamps, anchors, cache),
                        node_to_python_cached(py, val, schema, resolve_timestamps, anchors, cache),
                    )
                    .unwrap();
                }
            }
            dict.into_any().unbind()
        }
        // Handled above.
        YamlNodeKind::Alias(_) => unreachable!("alias handled before this match"),
    };

    // Record this anchor's object so a later alias yields the same instance.
    if let Some(name) = &node.anchor {
        cache.insert(name.clone(), obj.clone_ref(py));
    }
    obj
}

/// Whether a mapping-key node is a real YAML merge key (`<<`): a *plain* `<<`
/// scalar (a quoted `"<<"` is a literal string key), or a node carrying an
/// explicit `!!merge` tag. Mirrors the fast-path resolver's merge detection so
/// the AST-based conversions (round-trip `to_dict`, includes, annotated) apply
/// merges identically to `loads()`.
pub(crate) fn is_ast_merge_key(key: &YamlNode) -> bool {
    if let Some(tag) = &key.tag {
        return matches!(tag.as_str(), "!!merge" | "tag:yaml.org,2002:merge");
    }
    matches!(&key.kind, YamlNodeKind::Scalar(text, ScalarStyle::Plain) if text == "<<")
}

/// Whether any key in a mapping's pairs is a merge key, so the hot no-merge path
/// stays a plain insert loop and only mappings that actually carry `<<` pay for
/// the merge-aware handling.
pub(crate) fn mapping_has_merge_key(pairs: &[(YamlNode, YamlNode)]) -> bool {
    pairs.iter().any(|(k, _)| is_ast_merge_key(k))
}

/// Merge an already-converted `<<` value into `dict`, inserting only keys not
/// already present (so explicit keys and earlier merges win, matching PyYAML,
/// ruamel, and the fast path). Returns the object to preserve under the literal
/// `<<` key, or `None` when the value merged completely:
///
/// - A mapping merges its entries and returns `None`.
/// - A sequence merges each element and returns only the elements that could not
///   be merged, as a list (`None` if all merged), so a mergeable element is not
///   also duplicated under `<<`. Mirrors the fast path's `merge_into`.
/// - A null value contributes nothing and returns `None` (dropped), so an empty
///   `<<:` leaves no key.
/// - Anything else (a scalar or custom-tagged node) is returned as-is to
///   preserve under `<<`, exactly as the fast path does.
///
/// `dict.contains` uses Python equality, so a key already present under any
/// spelling is not overwritten.
pub(crate) fn merge_converted_into<'py>(
    dict: &Bound<'py, PyDict>,
    source: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    if source.is_none() {
        // A null merge value (`<<: ~`, an empty `<<:`, or a `null` element of a
        // merge list) contributes nothing and leaves no `<<` key, matching the
        // fast path (`Value::Null => None` in `decode::merge`). Without this it
        // fell through to the preserve arm below and kept a literal `<<: None`.
        Ok(None)
    } else if let Ok(src) = source.cast::<PyDict>() {
        for (k, v) in src.iter() {
            if !dict.contains(&k)? {
                dict.set_item(k, v)?;
            }
        }
        Ok(None)
    } else if let Ok(list) = source.cast::<PyList>() {
        let leftover = PyList::empty(source.py());
        for item in list.iter() {
            if let Some(unmerged) = merge_converted_into(dict, &item)? {
                leftover.append(unmerged)?;
            }
        }
        if leftover.is_empty() {
            Ok(None)
        } else {
            Ok(Some(leftover.into_any()))
        }
    } else {
        Ok(Some(source.clone()))
    }
}

/// Store a preserved (deferred / unmergeable) `<<` value under the literal merge
/// key. The first one is stored as-is; a later `<<` in the same mapping is
/// collected with it into a single `<<` list in document order, matching the
/// sequence form (`<<: [a, b]`), because a Python `dict` cannot hold two `<<`
/// slots and the host must see both to run its own merge over them. Nothing is
/// merged here: the values are deferred, so we only gather them and leave the
/// merge semantics to the host. A preserved value that is already a list
/// (leftovers from a `<<` sequence) is flattened one level rather than nested.
/// Assigning an equal key leaves the original key object in place, so the first
/// `<<` node's annotation is kept.
///
/// `preserved` tracks whether the `<<` slot already holds a preserved merge, so a
/// later `<<` collects into it rather than into an explicit quoted `"<<"` key. On
/// the first preserved `<<`, an explicit `"<<"` already in the dict is real data
/// and wins (a dict holds one `<<`): keep it and drop the merge rather than
/// corrupting it. The caller clears `preserved` when an explicit `"<<"` later
/// overwrites the slot, so the paths agree with the fast path.
pub(crate) fn store_preserved_merge<'py>(
    dict: &Bound<'py, PyDict>,
    key: Bound<'py, PyAny>,
    preserve: Bound<'py, PyAny>,
    preserved: &mut bool,
) -> PyResult<()> {
    if !*preserved {
        // No preserved merge in the `<<` slot yet. Only claim it when free; an
        // explicit `"<<"` key already there is real data and wins.
        if !dict.contains(&key)? {
            dict.set_item(key, preserve)?;
            *preserved = true;
        }
        return Ok(());
    }
    // A second preserved `<<`: collect into one list, in document order.
    let existing = dict
        .get_item(&key)?
        .expect("preserved `<<` slot is populated");
    let list = if let Ok(list) = existing.cast::<PyList>() {
        list.clone()
    } else {
        let list = PyList::empty(dict.py());
        list.append(&existing)?;
        dict.set_item(&key, &list)?;
        list
    };
    // Flatten a preserved list (leftovers from a `<<` sequence) one level in,
    // rather than nesting it, so the shape matches the sequence merge form.
    if let Ok(items) = preserve.cast::<PyList>() {
        for item in items.iter() {
            list.append(item)?;
        }
    } else {
        list.append(&preserve)?;
    }
    Ok(())
}

/// Whether a mapping-key node is a literal `"<<"` (a quoted scalar whose text is
/// `<<`), as opposed to a plain `<<` merge directive (which [`is_ast_merge_key`]
/// catches). Used to clear the preserved-merge slot flag when such an explicit
/// key overwrites the `<<` slot, so a later preserved `<<` does not collect into
/// real data.
pub(crate) fn is_literal_merge_key_text(key: &YamlNode) -> bool {
    matches!(&key.kind, YamlNodeKind::Scalar(text, _) if text == "<<") && !is_ast_merge_key(key)
}

/// Whether a mapping-key node is (or, for an alias, resolves to) a collection.
/// Such a key is unhashable as a plain Python container and must be routed
/// through [`node_to_python_key`] rather than the ordinary (annotated)
/// conversion, which would build an unhashable `dict`/`list` and fail.
pub(crate) fn key_is_collection(node: &YamlNode, anchors: &HashMap<String, YamlNode>) -> bool {
    match &node.kind {
        YamlNodeKind::Sequence(_) | YamlNodeKind::Mapping(_) => true,
        YamlNodeKind::Alias(name) => anchors.get(name).is_some_and(|t| {
            matches!(t.kind, YamlNodeKind::Sequence(_) | YamlNodeKind::Mapping(_))
        }),
        _ => false,
    }
}

/// Convert a mapping-key node to a Python object that can actually index a
/// `dict`. A collection key (a sequence or mapping, which YAML permits) is
/// unhashable as a Python `list`/`dict`, so it is rendered as its hashable
/// counterpart: a sequence becomes a `tuple`, and a mapping a `tuple` of
/// `(key, value)` tuples (order-preserving and round-trip-stable), recursively.
/// This mirrors the fast path's `value_to_hashable_key`, so both paths produce
/// the same key. Scalar keys go through the ordinary conversion unchanged.
pub(crate) fn node_to_python_key(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> Py<PyAny> {
    match &node.kind {
        // A `*alias` key resolves to its target, then becomes hashable like any
        // other key. The anchor map holds alias-free clones, so this terminates.
        YamlNodeKind::Alias(name) => match anchors.get(name) {
            Some(target) => {
                node_to_python_key(py, target, schema, resolve_timestamps, anchors, cache)
            }
            None => py.None(),
        },
        YamlNodeKind::Sequence(items) => {
            let elems: Vec<Py<PyAny>> = items
                .iter()
                .map(|item| {
                    node_to_python_key(py, item, schema, resolve_timestamps, anchors, cache)
                })
                .collect();
            PyTuple::new(py, elems).unwrap().into_any().unbind()
        }
        YamlNodeKind::Mapping(pairs) => {
            let entries: Vec<Py<PyAny>> = pairs
                .iter()
                .map(|(k, v)| {
                    let key = node_to_python_key(py, k, schema, resolve_timestamps, anchors, cache);
                    let val = node_to_python_key(py, v, schema, resolve_timestamps, anchors, cache);
                    PyTuple::new(py, [key, val]).unwrap().into_any().unbind()
                })
                .collect();
            PyTuple::new(py, entries).unwrap().into_any().unbind()
        }
        // A scalar (or null) key is hashable already.
        _ => node_to_python_cached(py, node, schema, resolve_timestamps, anchors, cache),
    }
}

/// Maximum object nesting depth when building an AST node from a Python value.
/// Bounds recursion so a deeply nested or cyclic (self-referential) value raises
/// instead of overflowing the native stack. Mirrors the encoder's limit.
const MAX_ASSIGN_DEPTH: u32 = 1000;

/// How a document reads its scalars, and so how a freshly assigned scalar must
/// be written to read back as the value it was given.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScalarTarget {
    /// The document's quote preference: when a string must be quoted, double
    /// or single quotes, to match the style the document re-emits in.
    pub double_quotes: bool,
    /// The schema the document reads its scalars under.
    pub schema: Schema,
    /// Whether the document reads a plain timestamp as a `date`/`datetime`.
    pub resolve_timestamps: bool,
}

/// Build a fresh AST node from a plain Python value (for assignment) into a
/// document read the way `target` describes.
///
/// A round-trip object (a document, view, or node cursor) is not plain data
/// and is refused (see [`round_trip_object_error`]).
pub(crate) fn python_to_node(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    target: ScalarTarget,
) -> PyResult<YamlNode> {
    python_to_node_depth(py, obj, target, 0)
}

#[allow(clippy::only_used_in_recursion)]
fn python_to_node_depth(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    target: ScalarTarget,
    depth: u32,
) -> PyResult<YamlNode> {
    // Grow the native stack on demand so assigning a deeply nested object
    // (bounded by `MAX_ASSIGN_DEPTH`) cannot overflow a small thread stack; the
    // recursion re-enters here per level. See [`crate::stack`].
    crate::stack::guard(|| python_to_node_depth_inner(py, obj, target, depth))
}

#[allow(clippy::only_used_in_recursion)]
fn python_to_node_depth_inner(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    target: ScalarTarget,
    depth: u32,
) -> PyResult<YamlNode> {
    use crate::scanner::Span;
    let span = Span::default();

    if depth >= MAX_ASSIGN_DEPTH {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "value is too deeply nested to assign (possible self-reference)",
        ));
    }

    if obj.is_none() {
        return Ok(assigned_null());
    }
    if let Ok(b) = obj.extract::<bool>() {
        return Ok(assigned_scalar(ResolvedValue::Bool(b), target));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(assigned_scalar(ResolvedValue::Int(i), target));
    }
    // A Python int too large for i64 is still an integer (arbitrary precision):
    // store its exact decimal text as a plain scalar so it round-trips as an int
    // rather than being lossily coerced to a float below.
    if obj.cast::<pyo3::types::PyInt>().is_ok() {
        let digits = obj.str()?.to_string();
        return Ok(assigned_scalar(ResolvedValue::BigInt(digits), target));
    }
    if let Ok(f) = obj.extract::<f64>() {
        return Ok(assigned_scalar(ResolvedValue::Float(f), target));
    }
    if let Ok(s) = obj.extract::<String>() {
        return Ok(assigned_scalar(ResolvedValue::String(s), target));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        // Snapshot the elements before converting any: conversion runs arbitrary
        // Python code (a value's `__float__`, `__str__`, ...), and if that mutates
        // the list, iterating it live could misbehave. Mirrors the fast path.
        let snapshot: Vec<Bound<'_, PyAny>> = list.iter().collect();
        let mut items = Vec::with_capacity(snapshot.len());
        for item in &snapshot {
            items.push(python_to_node_depth(py, item, target, depth + 1)?);
        }
        return Ok(YamlNode::new(YamlNodeKind::Sequence(items), span));
    }
    if let Ok(dict) = obj.cast::<PyDict>() {
        // Snapshot the entries before converting any: conversion runs arbitrary
        // Python code (a key/value's `__float__`, `__index__`, `__str__`, ...),
        // and if that mutates the dict, iterating it live panics ("dictionary
        // changed size during iteration"). Mirrors the fast path's `dict_to_pairs`.
        let snapshot: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = dict.iter().collect();
        let mut pairs = Vec::with_capacity(snapshot.len());
        for (k, v) in &snapshot {
            pairs.push((
                python_to_node_depth(py, k, target, depth + 1)?,
                python_to_node_depth(py, v, target, depth + 1)?,
            ));
        }
        return Ok(YamlNode::new(YamlNodeKind::Mapping(pairs), span));
    }
    if let Some(error) = round_trip_object_error(obj) {
        return Err(error);
    }

    let repr = obj.repr()?.to_string();
    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot convert {repr} to a YAML node"
    )))
}

/// The error for assigning a round-trip object, or `None` for anything else.
///
/// A document, view, or node cursor is a live handle onto a document, not data,
/// so it cannot be assigned as a value; its plain data can. The message names
/// the type alone: a handle's `repr()` reads its document, and the error must
/// not depend on that document being readable.
fn round_trip_object_error(obj: &Bound<'_, PyAny>) -> Option<PyErr> {
    use crate::roundtrip::document::{YAMLRocksDocument, YAMLRocksDocumentView, YAMLRocksNode};

    let (name, plain_data) = if obj.is_instance_of::<YAMLRocksDocumentView>() {
        ("YAMLRocksDocumentView", "view.to_dict()")
    } else if obj.is_instance_of::<YAMLRocksNode>() {
        ("YAMLRocksNode", "node.value")
    } else if obj.is_instance_of::<YAMLRocksDocument>() {
        ("YAMLRocksDocument", "doc.to_dict()")
    } else {
        return None;
    };
    Some(pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot convert a {name} to a YAML node; assign its plain data instead, \
         such as {plain_data}"
    )))
}

/// A freshly assigned null. It is marked synthetic so re-emission applies the
/// document's null style without disturbing loaded (untouched) nulls.
pub(crate) fn assigned_null() -> YamlNode {
    let mut node = YamlNode::new(YamlNodeKind::Null, crate::scanner::Span::default());
    node.synthetic = true;
    node
}

/// A freshly assigned scalar holding `value`, written so it reads back as that
/// same value in a document read the way `target` describes: a number or
/// boolean in its plain canonical spelling, and a string quoted where the
/// document would otherwise read it as something else.
pub(crate) fn assigned_scalar(value: ResolvedValue, target: ScalarTarget) -> YamlNode {
    let (text, style) = match value {
        ResolvedValue::Null => return assigned_null(),
        ResolvedValue::Bool(b) => (
            if b { "true" } else { "false" }.to_owned(),
            ScalarStyle::Plain,
        ),
        ResolvedValue::Int(i) => (i.to_string(), ScalarStyle::Plain),
        ResolvedValue::BigInt(digits) => (digits, ScalarStyle::Plain),
        ResolvedValue::Float(f) => (crate::emit_util::canonical_float(f), ScalarStyle::Plain),
        ResolvedValue::String(s) => {
            let style = assigned_string_style(&s, target.double_quotes, target.schema);
            // The quoting rules are the fast encoder's, which know the schema but
            // not whether this document reads timestamps. Under `OPT_TIMESTAMPS`
            // a plain date-shaped string would read back as a `date`.
            let reads_as_date = style == ScalarStyle::Plain
                && target.resolve_timestamps
                && crate::resolver::timestamp::parse(&s).is_some();
            let style = if !reads_as_date {
                style
            } else if crate::emit_util::single_quotable(&s, target.double_quotes) {
                ScalarStyle::SingleQuoted
            } else {
                ScalarStyle::DoubleQuoted
            };
            (s, style)
        }
    };
    YamlNode::new(
        YamlNodeKind::Scalar(text, style),
        crate::scanner::Span::default(),
    )
}

/// Choose the scalar style for a freshly assigned string. Unquoted when it can be
/// plain; otherwise the document's quote preference (double by default). A
/// single-quoted scalar cannot hold a line break or a literal quote, so those
/// fall back to double even in single-quote mode, mirroring the fast encoder.
pub(crate) fn assigned_string_style(
    value: &str,
    double_quotes: bool,
    schema: Schema,
) -> ScalarStyle {
    // Use the fast encoder's quoting rules verbatim rather than a second,
    // weaker check: a divergent copy let edited values (newlines, `...`, number
    // and bool/null look-alikes, leading indicators) emit unquoted and reparse
    // as a different value or as broken YAML, defeating round-trip fidelity. The
    // document's schema governs, so an edit to a 1.1 document quotes a `y`/`1:30`
    // that only 1.1 would re-read as a non-string.
    if !crate::encode::needs_quoting(value, schema) {
        ScalarStyle::Plain
    } else if crate::emit_util::single_quotable(value, double_quotes) {
        ScalarStyle::SingleQuoted
    } else {
        // A line break or a control/non-printable character cannot be single
        // quoted, so double-quote it (where it can be escaped), matching the
        // fast encoder.
        ScalarStyle::DoubleQuoted
    }
}

#[cfg(test)]
mod tests {
    use super::{assigned_scalar, ScalarTarget};
    use crate::resolver::{ResolvedValue, Schema};
    use crate::roundtrip::ast::YamlNodeKind;
    use crate::scanner::ScalarStyle;

    #[test]
    fn a_date_shaped_string_is_quoted_only_where_timestamps_are_read() {
        let style_for = |resolve_timestamps| {
            let target = ScalarTarget {
                double_quotes: true,
                schema: Schema::Yaml12,
                resolve_timestamps,
            };
            match assigned_scalar(ResolvedValue::String("2024-01-02".to_owned()), target).kind {
                YamlNodeKind::Scalar(_, style) => Some(style),
                _ => None,
            }
        };
        assert_eq!(style_for(false), Some(ScalarStyle::Plain));
        assert_eq!(style_for(true), Some(ScalarStyle::DoubleQuoted));
    }
}
