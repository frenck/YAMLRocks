use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{PyDate, PyDateTime, PyDelta, PyFloat, PyString, PyTuple, PyTzInfo};

use crate::decode::Value;
use crate::ffi::errors;
use crate::ffi::types::YAMLRocksTag;
use crate::resolver::timestamp::Timestamp;

use super::TagPolicy;

/// Build a Python `datetime.date`/`datetime.datetime` from a resolved timestamp.
/// A date-only value becomes a `date`; a date-time becomes a `datetime`, naive
/// when no zone was given and timezone-aware (a fixed `datetime.timezone`) when
/// an offset or `Z` was. Shared by the fast path and the round-trip AST path.
pub(crate) fn timestamp_to_py(py: Python<'_>, ts: &Timestamp) -> PyResult<Py<PyAny>> {
    match ts {
        Timestamp::Date { year, month, day } => {
            Ok(PyDate::new(py, *year, *month, *day)?.into_any().unbind())
        }
        Timestamp::DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            offset_minutes,
        } => {
            let tz = match offset_minutes {
                None => None,
                Some(offset) => {
                    let delta = PyDelta::new(py, 0, offset * 60, 0, true)?;
                    let timezone = py.import("datetime")?.getattr("timezone")?;
                    Some(timezone.call1((delta,))?.cast_into::<PyTzInfo>()?)
                }
            };
            let dt = PyDateTime::new(
                py,
                *year,
                *month,
                *day,
                *hour,
                *minute,
                *second,
                *microsecond,
                tz.as_ref(),
            )?;
            Ok(dt.into_any().unbind())
        }
    }
}

// -- Value -> Python (fast path) --
pub(super) fn resolve_tagged(
    py: Python<'_>,
    tag: &str,
    inner: Py<PyAny>,
    tags: TagPolicy<'_, '_>,
) -> PyResult<Py<PyAny>> {
    // Canonicalize a verbatim `!<uri>` to its bare URI for a registry lookup and
    // the handler callback, so both see the resolved tag (`tag:example.com,2020:foo`),
    // matching `%TAG`-shorthand expansion. A passthrough `YAMLRocksTag` keeps the
    // original `!<...>` spelling: `dumps`/`validate_tag` require a leading `!`, so a
    // bare URI could not be re-emitted and a load/dump round-trip would break.
    let canonical = crate::decode::canonical_tag(tag);
    if let Some(registry) = tags.registry {
        if let Some(func) = registry.get_item(canonical)? {
            return Ok(func.call1((inner,))?.unbind());
        }
    }
    if let Some(handler) = tags.handler {
        return Ok(handler.call1((canonical, inner))?.unbind());
    }
    if tags.passthrough {
        return Ok(YAMLRocksTag {
            tag: tag.to_owned(),
            value: inner,
        }
        .into_pyobject(py)?
        .into_any()
        .unbind());
    }
    Ok(inner)
}

/// A document-local cache of interned mapping keys: each distinct key string is
/// interned once and reused for every later occurrence within the same document.
/// Real configs repeat keys heavily (`name`, `platform`, ...), so this turns the
/// per-occurrence CPython intern-table lookup (hash + dict op) into a cheap local
/// hit, while still handing back an *interned* object so the caller's later
/// `data["key"]` lookups keep interning's speed. Keyed by the source slice, which
/// lives as long as the `Value` tree being materialized.
type KeyCache<'tree> = std::collections::HashMap<&'tree str, Py<PyAny>, ahash::RandomState>;

/// Convert a [`Value`] to Python, applying `tags` to custom-tagged nodes.
///
/// Grows the native stack on demand so a deeply nested tree (bounded by
/// `MAX_DEPTH`) cannot overflow a small thread stack during conversion; the
/// recursion re-enters here at each level. See [`crate::stack`].
pub fn value_to_python_with(
    py: Python<'_>,
    value: &Value<'_>,
    tags: TagPolicy<'_, '_>,
) -> PyResult<Py<PyAny>> {
    // Pre-sized so typical documents (a few dozen distinct keys) fill the
    // cache without a rehash.
    let mut keys = KeyCache::with_capacity_and_hasher(32, ahash::RandomState::default());
    value_to_python_cached(py, value, tags, &mut keys, 0)
}

/// Convert every document of a multi-document stream to Python, sharing one
/// interned-key cache across all of them.
///
/// The same mapping keys recur in every document of a stream (config exports,
/// log records, Kubernetes manifests), so interning each distinct key once for
/// the whole stream, rather than once per document, turns every later occurrence
/// into a cheap local cache hit instead of a fresh CPython intern (a hash plus an
/// intern-table probe). The cache is keyed by string content, so a key at a
/// different byte offset in a later document still hits. All documents borrow the
/// same input for the call's duration, so the cached key slices stay valid.
pub fn value_to_python_stream(
    py: Python<'_>,
    docs: &[Value<'_>],
    tags: TagPolicy<'_, '_>,
) -> PyResult<Vec<Py<PyAny>>> {
    let mut keys = KeyCache::with_capacity_and_hasher(32, ahash::RandomState::default());
    docs.iter()
        .map(|doc| value_to_python_cached(py, doc, tags, &mut keys, 0))
        .collect()
}

fn value_to_python_cached<'tree>(
    py: Python<'_>,
    value: &'tree Value<'_>,
    tags: TagPolicy<'_, '_>,
    keys: &mut KeyCache<'tree>,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    // Checking the remaining stack costs a TLS lookup, so guard every eighth
    // level rather than every one: eight conversion frames stay far below the
    // guard's `RED_ZONE` headroom, keeping the overflow invariant intact.
    if depth & 7 == 0 {
        crate::stack::guard(|| value_to_python_inner(py, value, tags, keys, depth))
    } else {
        value_to_python_inner(py, value, tags, keys, depth)
    }
}

fn value_to_python_inner<'tree>(
    py: Python<'_>,
    value: &'tree Value<'_>,
    tags: TagPolicy<'_, '_>,
    keys: &mut KeyCache<'tree>,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    let obj = match value {
        Value::Null => py.None(),
        Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        Value::Int(i) => i.into_pyobject(py)?.into_any().unbind(),
        Value::BigInt(s) => super::py_int_from_decimal(py, s),
        Value::Float(f) => PyFloat::new(py, *f).into_any().unbind(),
        Value::String(s) => PyString::new(py, s).into_any().unbind(),
        Value::Timestamp(ts) => timestamp_to_py(py, ts)?,
        Value::Sequence(items) => {
            // Build the list with the exact final size up front, then fill the
            // slots with `PyList_SET_ITEM` (which steals the reference). This
            // avoids the repeated reallocation and per-append bounds/refcount
            // work of `list.append`.
            let len = items.len();
            // SAFETY: `PyList_New` returns a new list of `len` NULL slots;
            // `from_owned_ptr_or_err` takes ownership (or raises on null). If a
            // child conversion fails mid-loop, the partially filled list is
            // dropped, and CPython's list deallocator `Py_XDECREF`s each slot,
            // tolerating the remaining NULLs.
            let list = unsafe {
                Bound::from_owned_ptr_or_err(py, ffi::PyList_New(len as ffi::Py_ssize_t))?
            };
            for (i, item) in items.iter().enumerate() {
                let child = value_to_python_cached(py, item, tags, keys, depth + 1)?;
                // SAFETY: `i` is in bounds (< len), the list is freshly created
                // and not shared, and `into_ptr` hands over an owned reference
                // for `PyList_SET_ITEM` to steal.
                unsafe {
                    ffi::PyList_SET_ITEM(list.as_ptr(), i as ffi::Py_ssize_t, child.into_ptr());
                }
            }
            list.into_any().unbind()
        }
        Value::Mapping(pairs) => {
            // SAFETY: `PyDict_New` returns a new empty dict; `from_owned_ptr_or_err`
            // takes ownership (or raises on null).
            let dict = unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyDict_New())? };
            for (key, val) in pairs {
                // Intern string keys: YAML configs reuse the same keys heavily
                // (e.g. "name", "platform"), so interning shares one object and
                // speeds up later dict lookups. The document-local cache interns
                // each distinct key once and reuses it for the rest of the tree.
                let py_key = match key {
                    Value::String(s) => match keys.get(s.as_ref()) {
                        Some(cached) => cached.clone_ref(py),
                        None => {
                            let interned = PyString::intern(py, s).into_any().unbind();
                            keys.insert(s.as_ref(), interned.clone_ref(py));
                            interned
                        }
                    },
                    // A collection used as a key is unhashable as a Python
                    // `list`/`dict`; convert it to its hashable counterpart (a
                    // sequence becomes a tuple, a mapping a frozenset of items).
                    Value::Sequence(_) | Value::Mapping(_) => value_to_hashable_key(py, key, tags)?,
                    other => value_to_python_cached(py, other, tags, keys, depth + 1)?,
                };
                let py_val = value_to_python_cached(py, val, tags, keys, depth + 1)?;
                // `PyDict_SetItem` does not steal references; it increments its
                // own on success, so our `py_key`/`py_val` are released when they
                // drop at the end of the iteration. YAML permits non-scalar keys,
                // which Python dicts cannot hold, so a non-zero return means an
                // unhashable key: surface a clean decode error rather than
                // leaving the exception set or panicking.
                // SAFETY: all three pointers are live Python objects.
                let rc =
                    unsafe { ffi::PyDict_SetItem(dict.as_ptr(), py_key.as_ptr(), py_val.as_ptr()) };
                if rc != 0 {
                    // SAFETY: clearing the pending CPython error so we can raise
                    // our own, more specific message.
                    unsafe { ffi::PyErr_Clear() };
                    return Err(errors::decode_message(
                        "mapping key is not hashable in Python",
                    ));
                }
            }
            dict.into_any().unbind()
        }
        Value::Tagged(tag, inner) => {
            let inner_py = value_to_python_cached(py, inner, tags, keys, depth + 1)?;
            resolve_tagged(py, tag.as_str(), inner_py, tags)?
        }
    };
    Ok(obj)
}

/// Convert a [`Value`] used as a mapping key into a *hashable* Python object.
///
/// YAML allows a collection (sequence or mapping) to be a mapping key, but a
/// Python `list`/`dict` is unhashable and cannot index a `dict`. A sequence is
/// rendered as a `tuple`, and a mapping as a `tuple` of `(key, value)` tuples
/// (preserving order), recursively, so nested collection keys are hashable too.
/// A `tuple` is used for the mapping (rather than a `frozenset`) so the key
/// survives a `dumps`/`loads` round-trip: a `frozenset` re-serializes as a
/// sequence, which would reload as a `tuple` and no longer compare equal. Scalars
/// (and tagged values) fall back to the ordinary conversion.
fn value_to_hashable_key(
    py: Python<'_>,
    value: &Value<'_>,
    tags: TagPolicy<'_, '_>,
) -> PyResult<Py<PyAny>> {
    // Complex keys are rare, so unlike the value conversion above this guards
    // every level; the depth is still bounded by the decoder's `MAX_DEPTH`.
    crate::stack::guard(|| value_to_hashable_key_inner(py, value, tags))
}

fn value_to_hashable_key_inner(
    py: Python<'_>,
    value: &Value<'_>,
    tags: TagPolicy<'_, '_>,
) -> PyResult<Py<PyAny>> {
    let obj = match value {
        Value::Sequence(items) => {
            let elems = items
                .iter()
                .map(|item| value_to_hashable_key(py, item, tags))
                .collect::<PyResult<Vec<_>>>()?;
            PyTuple::new(py, elems)?.into_any().unbind()
        }
        Value::Mapping(pairs) => {
            let entries = pairs
                .iter()
                .map(|(k, v)| {
                    let key = value_to_hashable_key(py, k, tags)?;
                    let val = value_to_hashable_key(py, v, tags)?;
                    Ok(PyTuple::new(py, [key, val])?.into_any().unbind())
                })
                .collect::<PyResult<Vec<Py<PyAny>>>>()?;
            PyTuple::new(py, entries.iter())?.into_any().unbind()
        }
        other => value_to_python_with(py, other, tags)?,
    };
    Ok(obj)
}
