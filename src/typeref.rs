//! Fast exact-type checks via direct `Py_TYPE` pointer comparison.
//!
//! pyo3's `cast::<T>()` performs *subclass-aware* checks (`PyXxx_Check`), which
//! walk the MRO. On the encode hot path the overwhelmingly common case is an
//! *exact* builtin type, so comparing `Py_TYPE(obj)` against the static type
//! object is both correct and cheaper than a subclass check.
//!
//! These predicates are deliberately strict: a subclass (an `IntEnum`, a numpy
//! scalar, an `YAMLRocksAnnotatedStr`, ...) does **not** match its builtin base here, so
//! callers must fall through to the subclass-aware slow path for anything that
//! returns `false`. That keeps the fast path a pure optimization with no change
//! in observable behavior.

use std::ptr;

use pyo3::ffi;
use pyo3::prelude::*;

/// The exact type object of `obj` (its `ob_type`), without touching the MRO.
#[inline]
fn type_ptr(obj: &Bound<'_, PyAny>) -> *mut ffi::PyTypeObject {
    // SAFETY: `obj` is a live, non-null Python object for the duration of the
    // borrow, so reading its type pointer is sound.
    unsafe { ffi::Py_TYPE(obj.as_ptr()) }
}

/// Generate an `is_exact_<name>` predicate comparing against a static builtin
/// type object.
macro_rules! exact_type {
    ($name:ident, $type_obj:ident) => {
        #[doc = concat!("Whether `obj`'s exact type is the builtin `", stringify!($type_obj), "`.")]
        #[inline]
        pub fn $name(obj: &Bound<'_, PyAny>) -> bool {
            ptr::eq(
                type_ptr(obj).cast::<ffi::PyObject>(),
                ptr::addr_of_mut!(ffi::$type_obj).cast::<ffi::PyObject>(),
            )
        }
    };
}

exact_type!(is_exact_bool, PyBool_Type);
exact_type!(is_exact_int, PyLong_Type);
exact_type!(is_exact_float, PyFloat_Type);
exact_type!(is_exact_str, PyUnicode_Type);
exact_type!(is_exact_list, PyList_Type);
exact_type!(is_exact_dict, PyDict_Type);
