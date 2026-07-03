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
    anchors: &HashMap<String, YamlNode>,
) -> Py<PyAny> {
    let mut cache = HashMap::new();
    node_to_python_cached(py, node, schema, anchors, &mut cache)
}

/// A per-conversion cache from anchor name to the single Python object built for
/// the anchored node. Sharing this across a whole-document walk makes every
/// `*alias` resolve to the *same* Python object as its `&anchor`, matching
/// PyYAML (so `d['base'] is d['ref']`), instead of an independent copy.
pub(crate) type ObjectCache = HashMap<String, Py<PyAny>>;

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
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> Py<PyAny> {
    // Grow the native stack on demand so converting a deeply nested document
    // (bounded by the composer's `MAX_DEPTH`) cannot overflow a small thread
    // stack; the recursion re-enters here at each level. See [`crate::stack`].
    crate::stack::guard(|| node_to_python_cached_inner(py, node, schema, anchors, cache))
}

fn node_to_python_cached_inner(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
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
            Some(target) => node_to_python_cached(py, target, schema, anchors, cache),
            None => py.None(),
        };
    }

    let obj = match &node.kind {
        YamlNodeKind::Null => py.None(),
        YamlNodeKind::Scalar(value, style) => {
            let resolved = schema.resolve(value, *style, node.tag.as_deref());
            match resolved {
                ResolvedValue::Null => py.None(),
                ResolvedValue::Bool(b) => {
                    b.into_pyobject(py).unwrap().to_owned().into_any().unbind()
                }
                ResolvedValue::Int(i) => i.into_pyobject(py).unwrap().into_any().unbind(),
                ResolvedValue::BigInt(s) => crate::ffi::py_int_from_decimal(py, &s),
                ResolvedValue::Float(f) => PyFloat::new(py, f).into_any().unbind(),
                ResolvedValue::String(s) => PyString::new(py, &s).into_any().unbind(),
            }
        }
        YamlNodeKind::Sequence(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(node_to_python_cached(py, item, schema, anchors, cache))
                    .unwrap();
            }
            list.into_any().unbind()
        }
        YamlNodeKind::Mapping(pairs) => {
            let dict = PyDict::new(py);
            if mapping_has_merge_key(pairs) {
                for (key, val) in pairs {
                    let py_val = node_to_python_cached(py, val, schema, anchors, cache);
                    if is_ast_merge_key(key) {
                        // Fold the merge value's keys in, keeping any explicit
                        // key already set. A non-mergeable value stays under the
                        // literal `<<`, as the fast path preserves it.
                        if let Some(preserve) =
                            merge_converted_into(&dict, py_val.bind(py)).unwrap()
                        {
                            let py_key = node_to_python_key(py, key, schema, anchors, cache);
                            if !dict.contains(&py_key).unwrap() {
                                dict.set_item(py_key, preserve).unwrap();
                            }
                        }
                    } else {
                        // An explicit key wins over a merge, so it overwrites.
                        dict.set_item(node_to_python_key(py, key, schema, anchors, cache), py_val)
                            .unwrap();
                    }
                }
            } else {
                for (key, val) in pairs {
                    dict.set_item(
                        node_to_python_key(py, key, schema, anchors, cache),
                        node_to_python_cached(py, val, schema, anchors, cache),
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
/// - Anything else (a scalar or custom-tagged node) is returned as-is to
///   preserve under `<<`, exactly as the fast path does.
///
/// `dict.contains` uses Python equality, so a key already present under any
/// spelling is not overwritten.
pub(crate) fn merge_converted_into<'py>(
    dict: &Bound<'py, PyDict>,
    source: &Bound<'py, PyAny>,
) -> PyResult<Option<Bound<'py, PyAny>>> {
    if let Ok(src) = source.cast::<PyDict>() {
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
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> Py<PyAny> {
    match &node.kind {
        // A `*alias` key resolves to its target, then becomes hashable like any
        // other key. The anchor map holds alias-free clones, so this terminates.
        YamlNodeKind::Alias(name) => match anchors.get(name) {
            Some(target) => node_to_python_key(py, target, schema, anchors, cache),
            None => py.None(),
        },
        YamlNodeKind::Sequence(items) => {
            let elems: Vec<Py<PyAny>> = items
                .iter()
                .map(|item| node_to_python_key(py, item, schema, anchors, cache))
                .collect();
            PyTuple::new(py, elems).unwrap().into_any().unbind()
        }
        YamlNodeKind::Mapping(pairs) => {
            let entries: Vec<Py<PyAny>> = pairs
                .iter()
                .map(|(k, v)| {
                    let key = node_to_python_key(py, k, schema, anchors, cache);
                    let val = node_to_python_key(py, v, schema, anchors, cache);
                    PyTuple::new(py, [key, val]).unwrap().into_any().unbind()
                })
                .collect();
            PyTuple::new(py, entries).unwrap().into_any().unbind()
        }
        // A scalar (or null) key is hashable already.
        _ => node_to_python_cached(py, node, schema, anchors, cache),
    }
}

/// Maximum object nesting depth when building an AST node from a Python value.
/// Bounds recursion so a deeply nested or cyclic (self-referential) value raises
/// instead of overflowing the native stack. Mirrors the encoder's limit.
const MAX_ASSIGN_DEPTH: u32 = 1000;

/// Build a fresh AST node from a plain Python value (for assignment).
///
/// `double_quotes` is the document's quote preference: when a freshly assigned
/// string must be quoted, it picks double or single quotes to match the style the
/// document re-emits in.
pub(crate) fn python_to_node(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    double_quotes: bool,
    schema: Schema,
) -> PyResult<YamlNode> {
    python_to_node_depth(py, obj, double_quotes, schema, 0)
}

#[allow(clippy::only_used_in_recursion)]
fn python_to_node_depth(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    double_quotes: bool,
    schema: Schema,
    depth: u32,
) -> PyResult<YamlNode> {
    // Grow the native stack on demand so assigning a deeply nested object
    // (bounded by `MAX_ASSIGN_DEPTH`) cannot overflow a small thread stack; the
    // recursion re-enters here per level. See [`crate::stack`].
    crate::stack::guard(|| python_to_node_depth_inner(py, obj, double_quotes, schema, depth))
}

#[allow(clippy::only_used_in_recursion)]
fn python_to_node_depth_inner(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    double_quotes: bool,
    schema: Schema,
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
        let mut node = YamlNode::new(YamlNodeKind::Null, span);
        // Mark it synthetic so re-emission applies the document's null style
        // without disturbing loaded (untouched) nulls.
        node.synthetic = true;
        return Ok(node);
    }
    if let Ok(b) = obj.extract::<bool>() {
        let s = if b { "true" } else { "false" };
        return Ok(YamlNode::new(
            YamlNodeKind::Scalar(s.to_owned(), ScalarStyle::Plain),
            span,
        ));
    }
    if let Ok(i) = obj.extract::<i64>() {
        return Ok(YamlNode::new(
            YamlNodeKind::Scalar(i.to_string(), ScalarStyle::Plain),
            span,
        ));
    }
    // A Python int too large for i64 is still an integer (arbitrary precision):
    // store its exact decimal text as a plain scalar so it round-trips as an int
    // rather than being lossily coerced to a float below.
    if obj.cast::<pyo3::types::PyInt>().is_ok() {
        return Ok(YamlNode::new(
            YamlNodeKind::Scalar(obj.str()?.to_string(), ScalarStyle::Plain),
            span,
        ));
    }
    if let Ok(f) = obj.extract::<f64>() {
        let s = crate::emit_util::canonical_float(f);
        return Ok(YamlNode::new(
            YamlNodeKind::Scalar(s, ScalarStyle::Plain),
            span,
        ));
    }
    if let Ok(s) = obj.extract::<String>() {
        let style = assigned_string_style(&s, double_quotes, schema);
        return Ok(YamlNode::new(YamlNodeKind::Scalar(s, style), span));
    }
    if let Ok(list) = obj.cast::<PyList>() {
        // Snapshot the elements before converting any: conversion runs arbitrary
        // Python code (a value's `__float__`, `__str__`, ...), and if that mutates
        // the list, iterating it live could misbehave. Mirrors the fast path.
        let snapshot: Vec<Bound<'_, PyAny>> = list.iter().collect();
        let mut items = Vec::with_capacity(snapshot.len());
        for item in &snapshot {
            items.push(python_to_node_depth(
                py,
                item,
                double_quotes,
                schema,
                depth + 1,
            )?);
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
                python_to_node_depth(py, k, double_quotes, schema, depth + 1)?,
                python_to_node_depth(py, v, double_quotes, schema, depth + 1)?,
            ));
        }
        return Ok(YamlNode::new(YamlNodeKind::Mapping(pairs), span));
    }

    let repr = obj.repr()?.to_string();
    Err(pyo3::exceptions::PyTypeError::new_err(format!(
        "cannot convert {repr} to a YAML node"
    )))
}

/// Choose the scalar style for a freshly assigned string. Unquoted when it can be
/// plain; otherwise the document's quote preference (double by default). A
/// single-quoted scalar cannot hold a line break or a literal quote, so those
/// fall back to double even in single-quote mode, mirroring the fast encoder.
fn assigned_string_style(value: &str, double_quotes: bool, schema: Schema) -> ScalarStyle {
    // Use the fast encoder's quoting rules verbatim rather than a second,
    // weaker check: a divergent copy let edited values (newlines, `...`, number
    // and bool/null look-alikes, leading indicators) emit unquoted and reparse
    // as a different value or as broken YAML, defeating round-trip fidelity. The
    // document's schema governs, so an edit to a 1.1 document quotes a `y`/`1:30`
    // that only 1.1 would re-read as a non-string.
    if !crate::encode::needs_quoting(value, schema) {
        ScalarStyle::Plain
    } else if double_quotes || value.contains('\'') || value.contains('\n') || value.contains('\r')
    {
        ScalarStyle::DoubleQuoted
    } else {
        ScalarStyle::SingleQuoted
    }
}
