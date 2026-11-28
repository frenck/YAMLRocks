use pyo3::exceptions::PyTypeError;
use pyo3::ffi;
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDate, PyDateTime, PyDelta, PyFloat, PyString, PyTuple, PyTzInfo};

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

/// Scratch state shared by one fast-path conversion: the interned-key cache and
/// a stack of converted sequence items.
///
/// A sequence's children are converted onto `items` first and only then moved
/// into a new list, so the list is never observable half built. A list is
/// GC-tracked from the moment `PyList_New` returns, and a tag callback (or, on a
/// free-threaded build, another thread) can find it through `gc.get_objects()`:
/// reading a NULL slot there crashes, and a `clear()` there turns the next
/// `PyList_SET_ITEM` into a write through a stale index. One stack serves the
/// whole tree (a nested sequence pushes above its parent's items and drains back
/// down to where it started), so once it has grown this costs no allocation per
/// sequence. A sequence of plain scalars, the common case, needs none of this:
/// see [`plain_scalar_list`].
struct Scratch<'tree> {
    keys: KeyCache<'tree>,
    items: Vec<Py<PyAny>>,
}

impl Scratch<'_> {
    fn new() -> Self {
        Self {
            // Pre-sized so typical documents (a few dozen distinct keys) fill
            // the cache without a rehash.
            keys: KeyCache::with_capacity_and_hasher(32, ahash::RandomState::default()),
            // Allocated on first use: a document whose sequences hold only
            // plain scalars never needs it.
            items: Vec::new(),
        }
    }
}

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
    value_to_python_cached(py, value, tags, &mut Scratch::new(), 0)
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
    let mut scratch = Scratch::new();
    docs.iter()
        .map(|doc| value_to_python_cached(py, doc, tags, &mut scratch, 0))
        .collect()
}

fn value_to_python_cached<'tree>(
    py: Python<'_>,
    value: &'tree Value<'_>,
    tags: TagPolicy<'_, '_>,
    scratch: &mut Scratch<'tree>,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    // Checking the remaining stack costs a TLS lookup, so guard every eighth
    // level rather than every one: eight conversion frames stay far below the
    // guard's `RED_ZONE` headroom, keeping the overflow invariant intact.
    if depth & 7 == 0 {
        crate::stack::guard(|| value_to_python_inner(py, value, tags, scratch, depth))
    } else {
        value_to_python_inner(py, value, tags, scratch, depth)
    }
}

fn value_to_python_inner<'tree>(
    py: Python<'_>,
    value: &'tree Value<'_>,
    tags: TagPolicy<'_, '_>,
    scratch: &mut Scratch<'tree>,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    // The scalar arms repeat `plain_scalar_to_python` on purpose: one dispatch
    // here keeps the hottest path of `loads` as short as it can be.
    let obj = match value {
        Value::Null => py.None(),
        Value::Bool(b) => b.into_pyobject(py)?.to_owned().into_any().unbind(),
        Value::Int(i) => i.into_pyobject(py)?.into_any().unbind(),
        Value::BigInt(s) => super::py_int_from_decimal(py, s),
        Value::Float(f) => PyFloat::new(py, *f).into_any().unbind(),
        Value::String(s) => PyString::new(py, s).into_any().unbind(),
        Value::Timestamp(ts) => timestamp_to_py(py, ts)?,
        // Most sequences hold only plain scalars, which are converted straight
        // into the list's slots. Anything else (a nested collection, a tag) may
        // run Python, so its children go through the scratch stack first.
        Value::Sequence(items) if items.iter().all(is_plain_scalar) => {
            plain_scalar_list(py, items)?
        }
        Value::Sequence(items) => collection_list(py, items, tags, scratch, depth)?,
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
                    Value::String(s) => match scratch.keys.get(s.as_ref()) {
                        Some(cached) => cached.clone_ref(py),
                        None => {
                            let interned = PyString::intern(py, s).into_any().unbind();
                            scratch.keys.insert(s.as_ref(), interned.clone_ref(py));
                            interned
                        }
                    },
                    // A collection used as a key is unhashable as a Python
                    // `list`/`dict`; convert it to its hashable counterpart (a
                    // sequence becomes a tuple, a mapping a frozenset of items).
                    Value::Sequence(_) | Value::Mapping(_) => value_to_hashable_key(py, key, tags)?,
                    other => value_to_python_cached(py, other, tags, scratch, depth + 1)?,
                };
                let py_val = value_to_python_cached(py, val, tags, scratch, depth + 1)?;
                // `PyDict_SetItem` does not steal references; it increments its
                // own on success, so our `py_key`/`py_val` are released when they
                // drop at the end of the iteration.
                // SAFETY: all three pointers are live Python objects.
                let rc =
                    unsafe { ffi::PyDict_SetItem(dict.as_ptr(), py_key.as_ptr(), py_val.as_ptr()) };
                if rc != 0 {
                    return Err(dict_insert_error(py));
                }
            }
            dict.into_any().unbind()
        }
        Value::Tagged(tag, inner) => {
            let inner_py = value_to_python_cached(py, inner, tags, scratch, depth + 1)?;
            resolve_tagged(py, tag.as_str(), inner_py, tags)?
        }
    };
    Ok(obj)
}

/// Whether `value` is a plain scalar: one whose conversion only calls a CPython
/// constructor for `str`, `int` or `float` (or hands out `None`/`True`/`False`).
/// Such a conversion never runs Python code, never allocates a GC-tracked
/// object and never detaches the thread, which is what lets
/// [`plain_scalar_list`] fill a list in place. A big integer is left out: its
/// conversion can raise and clear an exception.
fn is_plain_scalar(value: &Value<'_>) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::String(_)
    )
}

/// Convert a plain scalar (see [`is_plain_scalar`]), or `None` for any other
/// value.
#[inline]
fn plain_scalar_to_python(py: Python<'_>, value: &Value<'_>) -> Option<Py<PyAny>> {
    let obj = match value {
        Value::Null => py.None(),
        Value::Bool(b) => PyBool::new(py, *b).to_owned().into_any().unbind(),
        Value::Int(i) => {
            let Ok(int) = i.into_pyobject(py);
            int.into_any().unbind()
        }
        Value::Float(f) => PyFloat::new(py, *f).into_any().unbind(),
        Value::String(s) => PyString::new(py, s).into_any().unbind(),
        _ => return None,
    };
    Some(obj)
}

/// Build a list of plain scalars (see [`is_plain_scalar`]), converting each
/// one straight into its slot.
///
/// This is the common case and skips the scratch stack entirely. It is sound
/// only because no conversion in the fill loop can run Python code, trigger a
/// garbage collection or reach a point where another thread could stop the
/// world, so nothing can observe the list while a slot is still NULL.
///
/// Kept out of line, like [`collection_list`], so the per-value dispatch in
/// `value_to_python_inner` stays small.
#[inline(never)]
fn plain_scalar_list(py: Python<'_>, items: &[Value<'_>]) -> PyResult<Py<PyAny>> {
    // SAFETY: `PyList_New` returns a new list of `items.len()` NULL slots;
    // `from_owned_ptr_or_err` takes ownership (or raises on null). Should a
    // conversion below panic, dropping the list releases the slots filled so
    // far, and CPython's list deallocator tolerates the NULL ones.
    let list = unsafe {
        Bound::from_owned_ptr_or_err(py, ffi::PyList_New(items.len() as ffi::Py_ssize_t))?
    };
    for (i, item) in items.iter().enumerate() {
        let child = plain_scalar_to_python(py, item).expect("caller checked is_plain_scalar");
        // SAFETY: `i` is in bounds (< len) and each slot is filled exactly once.
        // Only plain scalar conversions run between `PyList_New` and this
        // store, so the list has not been observable (see above). `into_ptr`
        // hands over an owned reference for `PyList_SET_ITEM` to steal.
        unsafe {
            ffi::PyList_SET_ITEM(list.as_ptr(), i as ffi::Py_ssize_t, child.into_ptr());
        }
    }
    Ok(list.into_any().unbind())
}

/// Build a list whose items are not all plain scalars (a nested collection, a
/// tag, a timestamp), any of which may run Python while it converts.
///
/// Every child is converted onto the scratch stack before the list exists, and
/// the list is then created and filled with no Python call in between. See
/// [`Scratch`].
#[inline(never)]
fn collection_list<'tree>(
    py: Python<'_>,
    items: &'tree [Value<'_>],
    tags: TagPolicy<'_, '_>,
    scratch: &mut Scratch<'tree>,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    // A failing child abandons the whole conversion, and the items already
    // pushed are released when the scratch state drops.
    let base = scratch.items.len();
    // Reserve the whole sequence at once: growing by doubling would copy the
    // stack repeatedly for a long sequence.
    scratch.items.reserve(items.len());
    for item in items {
        let child = value_to_python_cached(py, item, tags, scratch, depth + 1)?;
        scratch.items.push(child);
    }

    // Build the list with the exact final size, then move the children in with
    // `PyList_SET_ITEM` (which steals the reference). This avoids the repeated
    // reallocation and per-append bounds and refcount work of `list.append`.
    let len = items.len();
    debug_assert_eq!(scratch.items.len(), base + len);
    // SAFETY: `PyList_New` returns a new list of `len` NULL slots;
    // `from_owned_ptr_or_err` takes ownership (or raises on null).
    let list =
        unsafe { Bound::from_owned_ptr_or_err(py, ffi::PyList_New(len as ffi::Py_ssize_t))? };

    // SAFETY: the scratch stack holds exactly the `len` owned children above
    // `base`, so each read is in bounds and slot `i < len` is filled exactly
    // once. Each `PyList_SET_ITEM` steals the reference it is handed, and
    // `set_len` then forgets the moved-from entries without releasing them.
    // From `PyList_New` to the last store nothing runs Python or allocates a
    // Python object, so no callback, finalizer or garbage collection can reach
    // the list while a slot is NULL, and on a free-threaded build there is no
    // safe point at which another thread could stop the world to inspect it.
    unsafe {
        let children = scratch.items.as_ptr().add(base);
        for i in 0..len {
            let child = children.add(i).read();
            ffi::PyList_SET_ITEM(list.as_ptr(), i as ffi::Py_ssize_t, child.into_ptr());
        }
        scratch.items.set_len(base);
    }
    Ok(list.into_any().unbind())
}

/// The error to raise after `PyDict_SetItem` failed while building a mapping.
///
/// A key can be unhashable (a tag callback may return a `list`), which CPython
/// reports as a `TypeError`; only that one becomes a clean decode error, with
/// the original kept as its cause. Anything else, such as a `KeyboardInterrupt`
/// or a `MemoryError` raised from a key's `__hash__` or `__eq__`, says nothing
/// about the document and propagates unchanged.
fn dict_insert_error(py: Python<'_>) -> PyErr {
    let original = PyErr::fetch(py);
    if !original.is_instance_of::<PyTypeError>(py) {
        return original;
    }

    let error = errors::decode_message("mapping key is not hashable in Python");
    error.set_cause(py, Some(original));
    error
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
