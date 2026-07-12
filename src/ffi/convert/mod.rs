//! Conversions between Rust values and Python objects.
//!
//! Split by direction and shape:
//!
//! * [`decode`] turns the fast-path [`Value`](crate::decode::Value) tree into
//!   Python objects, honoring the [`TagPolicy`].
//! * [`annotate`] turns a round-trip AST into annotated `dict`/`list` subclasses
//!   carrying source locations.
//! * [`encode`] turns arbitrary Python objects (datetime, uuid, Decimal, Enum,
//!   dataclasses, and, opt-in, numpy) into a `Value` tree for emission.
//!
//! The two context types shared across those modules ([`TagPolicy`] and
//! [`EncodeCtx`]) live here.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

mod annotate;
mod decode;
mod encode;

pub use annotate::{annotate_node, node_to_python_with_tags};
pub(crate) use decode::timestamp_to_py;
pub use decode::{value_to_python_stream, value_to_python_with};
pub use encode::python_to_value;
pub(crate) use encode::{
    bad_serializer_result, datetime_to_value, is_enum, nested_tag_error, numpy_child, validate_tag,
};

/// Build a Python `int` from a decimal integer string (with an optional sign),
/// for integers too large to fit in `i64`. The digits are already validated as a
/// base-10 integer, so this is infallible in practice; on the unexpected failure
/// (out of memory) it clears the error and falls back to the string.
pub(crate) fn py_int_from_decimal(py: Python<'_>, digits: &str) -> Py<PyAny> {
    // YAML 1.1 permits underscore digit separators (`1_000_000`), and the
    // resolver accepts them leniently (including `1__000`/`1000_`, matching
    // PyYAML). `PyLong_FromString` only accepts *well-placed* underscores, so
    // strip them all first; otherwise a big integer the resolver classified as
    // such would fail to parse here and silently degrade to a string. The common
    // path (the 1.2 schema and the encoder) has no underscores and skips the
    // allocation entirely.
    let cleaned;
    let digits = if digits.as_bytes().contains(&b'_') {
        cleaned = digits.replace('_', "");
        cleaned.as_str()
    } else {
        digits
    };
    if let Ok(c) = std::ffi::CString::new(digits) {
        // SAFETY: `c` is a valid null-terminated C string; base 10. The result is
        // a new reference (or null with an exception set, handled below).
        let ptr = unsafe { pyo3::ffi::PyLong_FromString(c.as_ptr(), std::ptr::null_mut(), 10) };
        if !ptr.is_null() {
            // SAFETY: `ptr` is a non-null owned reference from `PyLong_FromString`.
            return unsafe { Bound::from_owned_ptr(py, ptr) }.unbind();
        }
        // SAFETY: an exception is set; clear it before the string fallback.
        unsafe { pyo3::ffi::PyErr_Clear() };
    }
    PyString::new(py, digits).into_any().unbind()
}

/// How custom-tagged values are surfaced to Python.
///
/// Resolution order for a custom tag: a matching entry in `registry` (a
/// `{tag: func}` mapping whose function receives just the inner value), then the
/// `handler` catch-all `(tag, value)` callback, then passthrough `YAMLRocksTag`
/// objects,
/// and finally the default of dropping the tag and keeping the inner value.
#[derive(Clone, Copy, Default)]
pub struct TagPolicy<'a, 'py> {
    /// Optional `{tag: func}` registry; `func` is called with the inner value.
    pub registry: Option<&'a Bound<'py, PyDict>>,
    /// Optional `tag_handler(tag, value)` callback.
    pub handler: Option<&'a Bound<'py, PyAny>>,
    /// Whether to wrap custom-tagged values in `YAMLRocksTag` objects.
    pub passthrough: bool,
}

/// Conversion context for `dumps`.
#[derive(Clone, Copy, Default)]
pub struct EncodeCtx<'a> {
    pub default: Option<&'a Py<PyAny>>,
    pub serialize_numpy: bool,
    /// Drop the microseconds field when formatting datetimes/times.
    pub omit_microseconds: bool,
    /// Treat a naive datetime as UTC (append a `+00:00` offset).
    pub naive_utc: bool,
    /// Render a UTC offset as `Z` rather than `+00:00`.
    pub utc_z: bool,
    /// Do not auto-serialize `datetime`/`date`/`time`; route them to `default`.
    pub passthrough_datetime: bool,
    /// Do not auto-serialize dataclass instances; route them to `default`.
    pub passthrough_dataclass: bool,
    /// Optional `{type: func}` registry mapping a Python type to a callable that
    /// returns a `YAMLRocksTag` (or `(tag, value)` tuple) for emitting a custom
    /// `!tag value`. Consulted by exact type before dataclass auto-serialization,
    /// the write-side mirror of the load-side `tags` registry.
    pub tags: Option<&'a Py<PyDict>>,
    /// Current recursion depth, bounding nesting so a deeply nested or cyclic
    /// object raises instead of overflowing the native stack. Starts at 0.
    pub depth: u32,
}
