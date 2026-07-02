use std::sync::OnceLock;

use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyBytes, PyDict, PyFloat, PyFrozenSet, PyInt, PyList, PySet, PyString, PyTuple, PyType,
};

use crate::decode::Value;
use crate::ffi::errors;
use crate::ffi::types::YAMLRocksTag;
use crate::typeref;

use super::EncodeCtx;

/// Extract a Python `str` as an owned UTF-8 `String`, rejecting unpaired
/// surrogates rather than silently replacing them with U+FFFD. PyO3's `Display`
/// (`to_string`) is lossy; `to_str` is strict. This keeps any data loss visible,
/// matching the strict `bytes` branch and the decode side, so a `str` carrying a
/// lone surrogate (from `surrogateescape`/`surrogatepass`) errors instead of
/// emitting mojibake that no longer round-trips.
fn pystring_to_string(s: &Bound<'_, PyString>) -> PyResult<String> {
    s.to_str().map(str::to_owned).map_err(|_| {
        errors::encode_error(
            "cannot serialize a str containing unpaired surrogates; convert it to \
             valid Unicode (or recover the original bytes and handle them \
             explicitly) before dumping"
                .to_string(),
        )
    })
}

// -- Python -> Value (for dumps) --
/// Maximum object nesting depth when serializing. A deeply nested or cyclic
/// (self-referential) Python object would otherwise recurse without bound and
/// overflow the native stack; past this depth we raise instead. Mirrors the
/// decoder's `MAX_DEPTH` and is far beyond any real document's nesting.
const MAX_ENCODE_DEPTH: u32 = 1000;

pub fn python_to_value(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Value<'static>> {
    // Grow the native stack on demand so a deeply nested object (bounded by
    // `MAX_ENCODE_DEPTH`) cannot overflow a small thread stack while building the
    // value tree; the recursion re-enters here per level. See [`crate::stack`].
    crate::stack::guard(|| python_to_value_inner(py, obj, ctx))
}

fn python_to_value_inner(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Value<'static>> {
    if ctx.depth >= MAX_ENCODE_DEPTH {
        return Err(errors::encode_error(
            "object is too deeply nested to serialize (possible self-reference)".to_string(),
        ));
    }
    let ctx = EncodeCtx {
        depth: ctx.depth + 1,
        ..ctx
    };

    if obj.is_none() {
        return Ok(Value::Null);
    }

    // Fast path: dispatch the exact builtin types by direct type-pointer
    // comparison, skipping the subclass-aware `cast` chain below. Subclasses
    // (IntEnum, numpy scalars, YAMLRocksAnnotatedStr, dataclasses, ...) fail every exact
    // check and fall through to the unchanged slow path, so behavior is
    // identical; only the common case gets faster.
    if typeref::is_exact_str(obj) {
        return Ok(Value::String(
            pystring_to_string(obj.cast::<PyString>()?)?.into(),
        ));
    }
    if typeref::is_exact_int(obj) {
        return int_to_value(obj);
    }
    if typeref::is_exact_float(obj) {
        return Ok(Value::Float(obj.extract()?));
    }
    if typeref::is_exact_bool(obj) {
        return Ok(Value::Bool(obj.cast::<PyBool>()?.is_true()));
    }
    if typeref::is_exact_list(obj) {
        return Ok(Value::Sequence(list_to_values(py, obj, ctx)?));
    }
    if typeref::is_exact_dict(obj) {
        return Ok(Value::Mapping(dict_to_pairs(py, obj, ctx)?));
    }

    if let Ok(b) = obj.cast::<PyBool>() {
        return Ok(Value::Bool(b.is_true()));
    }
    // IntEnum/StrEnum and numpy scalars subclass int/str, so these come first.
    if obj.cast::<PyInt>().is_ok() {
        return int_to_value(obj);
    }
    if obj.cast::<PyFloat>().is_ok() {
        return Ok(Value::Float(obj.extract()?));
    }
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(Value::String(pystring_to_string(s)?.into()));
    }
    if let Ok(b) = obj.cast::<PyBytes>() {
        // Decode strictly: silently replacing invalid UTF-8 would corrupt
        // binary data on the way out. Reject it instead so the loss is visible.
        return match std::str::from_utf8(b.as_bytes()) {
            Ok(s) => Ok(Value::String(s.to_owned().into())),
            Err(_) => Err(errors::encode_error(
                "cannot serialize bytes that are not valid UTF-8; decode them \
                 yourself (e.g. base64) before dumping"
                    .to_string(),
            )),
        };
    }
    if let Ok(list) = obj.cast::<PyList>() {
        return Ok(Value::Sequence(seq_to_values(py, list.try_iter()?, ctx)?));
    }
    if let Ok(tuple) = obj.cast::<PyTuple>() {
        return Ok(Value::Sequence(seq_to_values(py, tuple.try_iter()?, ctx)?));
    }
    if obj.cast::<PyDict>().is_ok() {
        // A dict subclass (OrderedDict, defaultdict, ...) reaches here; route it
        // through the same snapshotting helper as an exact dict so converting an
        // entry cannot free or resize the dict mid-walk (a use-after-free / a
        // `PyDict_Next` panic across the FFI boundary).
        return Ok(Value::Mapping(dict_to_pairs(py, obj, ctx)?));
    }
    if let Ok(set) = obj.cast::<PySet>() {
        return Ok(Value::Sequence(seq_to_values(py, set.try_iter()?, ctx)?));
    }
    if let Ok(set) = obj.cast::<PyFrozenSet>() {
        return Ok(Value::Sequence(seq_to_values(py, set.try_iter()?, ctx)?));
    }

    // A YAMLRocksTag emits a custom `!tag value`, the write-side inverse of
    // OPT_PASSTHROUGH_TAG. Its inner value is serialized with the normal rules.
    if let Ok(tag) = obj.cast::<YAMLRocksTag>() {
        return tagged_to_value(py, tag, ctx);
    }

    // The `tags` registry maps an exact Python type to a callable returning a
    // YAMLRocksTag (or `(tag, value)` tuple). Consulted before dataclass
    // auto-serialization (below), so a registered dataclass becomes `!tag ...`
    // rather than a plain mapping.
    if let Some(registry) = ctx.tags {
        if let Some(func) = registry.bind(py).get_item(obj.get_type())? {
            let result = func.call1((obj,))?;
            return tag_callback_result(py, &result, ctx);
        }
    }

    if let Some(value) = special_type_to_value(py, obj, ctx)? {
        return Ok(value);
    }

    if let Some(default_fn) = ctx.default {
        let result = default_fn.call1(py, (obj,))?;
        return python_to_value(
            py,
            result.bind(py),
            EncodeCtx {
                default: None,
                ..ctx
            },
        );
    }

    Err(errors::unserializable_error(format!(
        "type {} is not YAML serializable",
        obj.get_type().name()?
    )))
}

/// Reject a tag that cannot be emitted as a single tag token. A tag is written
/// verbatim before its value (`!tag value`), so one that the scanner would read
/// only part of splits on re-parse and silently corrupts the document.
///
/// Whitespace and control characters are invalid in any tag. For a shorthand tag
/// (`!foo`, `!!foo`, `!handle!foo`) a flow indicator `,`/`]`/`}` also terminates
/// the scan, so `YAMLRocksTag("!foo,bar", "v")` would emit `!foo,bar v` and
/// reload as the tag `!foo` on a (broken) value `,bar v`. A verbatim tag
/// (`!<...>`) is delimited by its closing `>` and may carry such characters
/// (URI commas), so it is exempt from the flow-indicator check but must close.
fn validate_tag(tag: &str) -> PyResult<()> {
    if tag.is_empty() || !tag.starts_with('!') {
        return Err(errors::encode_error(format!(
            "invalid tag {tag:?}: a tag must start with '!'"
        )));
    }
    if let Some(bad) = tag.chars().find(|c| c.is_whitespace() || c.is_control()) {
        return Err(errors::encode_error(format!(
            "invalid tag {tag:?}: a tag cannot contain whitespace or control characters (found {bad:?})"
        )));
    }
    if let Some(verbatim) = tag.strip_prefix("!<") {
        // A verbatim tag is `!<` + non-empty URI + `>`; the scanner rejects an
        // unterminated or empty one on read, so refuse to emit one here too.
        if verbatim.strip_suffix('>').map_or(true, str::is_empty) {
            return Err(errors::encode_error(format!(
                "invalid tag {tag:?}: a verbatim tag must be '!<...>' with non-empty content"
            )));
        }
    } else if let Some(bad) = tag.chars().find(|c| matches!(c, ',' | ']' | '}')) {
        return Err(errors::encode_error(format!(
            "invalid tag {tag:?}: a shorthand tag cannot contain a flow indicator (found {bad:?})"
        )));
    }
    Ok(())
}

/// Convert a [`YAMLRocksTag`] into a [`Value::Tagged`], serializing its inner
/// value with the normal rules. The tag and value are read out first so the
/// borrow is released before the (Python-running) recursive conversion.
fn tagged_to_value(
    py: Python<'_>,
    tag_obj: &Bound<'_, YAMLRocksTag>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Value<'static>> {
    let (tag, value) = {
        let borrowed = tag_obj.borrow();
        (borrowed.tag.clone(), borrowed.value.clone_ref(py))
    };
    validate_tag(&tag)?;
    let inner = python_to_value(py, value.bind(py), ctx)?;
    Ok(Value::Tagged(tag, Box::new(inner)))
}

/// Interpret a `tags` callback's return value, which may be a [`YAMLRocksTag`]
/// or a `(tag, value)` tuple; both produce a [`Value::Tagged`].
fn tag_callback_result(
    py: Python<'_>,
    result: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Value<'static>> {
    if let Ok(tag) = result.cast::<YAMLRocksTag>() {
        return tagged_to_value(py, tag, ctx);
    }
    if let Ok(tuple) = result.cast::<PyTuple>() {
        if tuple.len() == 2 {
            let tag: String = tuple.get_item(0)?.extract()?;
            validate_tag(&tag)?;
            let inner = python_to_value(py, &tuple.get_item(1)?, ctx)?;
            return Ok(Value::Tagged(tag, Box::new(inner)));
        }
    }
    Err(errors::encode_error(
        "a tags callback must return a YAMLRocksTag or a (tag, value) tuple".to_string(),
    ))
}

/// Convert a Python `int` to a [`Value`], falling back to a big integer (the
/// exact decimal text) when it does not fit in `i64`. Python integers are
/// arbitrary precision, so plain `i64` extraction would raise `OverflowError`.
fn int_to_value(obj: &Bound<'_, PyAny>) -> PyResult<Value<'static>> {
    match obj.extract::<i64>() {
        Ok(i) => Ok(Value::Int(i)),
        Err(_) => {
            // `obj.str()` would honor a subclass override of `__str__` (a custom
            // `int` wrapper can return arbitrary text; an `IntEnum`'s repr leaks
            // through some paths), emitting that text in place of the digits and
            // producing invalid YAML/JSON. Reduce to a true base `int` first via
            // the `__index__`/`__int__` protocol (`int(obj)`), which yields the
            // genuine numeric value, then stringify that — its `__str__` is the
            // real integer formatter, so the exact decimal is always serialized.
            let py = obj.py();
            let base = py.get_type::<PyInt>().call1((obj,))?;
            Ok(Value::BigInt(base.str()?.to_string().into()))
        }
    }
}

fn seq_to_values<'py>(
    py: Python<'py>,
    iter: impl Iterator<Item = PyResult<Bound<'py, PyAny>>>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Vec<Value<'static>>> {
    let mut items = Vec::new();
    for item in iter {
        items.push(python_to_value(py, &item?, ctx)?);
    }
    Ok(items)
}

/// Convert an exact `list`, snapshotting its elements into owned handles before
/// converting any of them.
///
/// Converting an element can run arbitrary Python (a `default` callback, the
/// `tags` registry, or the `getattr`/`call` probes for datetime/enum/dataclass/
/// `__fspath__`/numpy types), which may mutate or free this very list. Walking
/// its raw storage across that reentrancy would read a shifted or freed slot, a
/// use-after-free. Materializing owned references up front makes the walk immune.
fn list_to_values(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Vec<Value<'static>>> {
    let snapshot: Vec<Bound<'_, PyAny>> = obj.cast::<PyList>()?.iter().collect();
    snapshot
        .iter()
        .map(|item| python_to_value(py, item, ctx))
        .collect()
}

/// Convert an exact `dict`, snapshotting its entries into owned handles first.
///
/// Like [`list_to_values`], converting an entry can run Python that mutates the
/// dict mid-walk, which `PyDict_Next` explicitly forbids; the snapshot makes the
/// conversion immune to that reentrancy.
fn dict_to_pairs(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Vec<(Value<'static>, Value<'static>)>> {
    let snapshot: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> =
        obj.cast::<PyDict>()?.iter().collect();
    snapshot
        .iter()
        .map(|(key, val)| {
            Ok((
                python_to_value(py, key, ctx)?,
                python_to_value(py, val, ctx)?,
            ))
        })
        .collect()
}

/// Convert a `datetime`/`date`/`time` to an ISO 8601 string, applying the
/// datetime formatting options. Returns `None` when `obj` is not datetime-like
/// or when datetimes are being passed through to `default`.
fn datetime_to_value(
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Option<Value<'static>>> {
    // `date`, `time`, and `datetime` all expose `isoformat`; absence of it means
    // this is not a datetime-like object, so let the caller keep dispatching.
    let Ok(iso_method) = obj.getattr("isoformat") else {
        return Ok(None);
    };
    if ctx.passthrough_datetime {
        return Ok(None);
    }
    let Ok(s) = iso_method.call0()?.extract::<String>() else {
        return Ok(None);
    };
    Ok(Some(Value::String(format_datetime(&s, ctx).into())))
}

/// Apply `OPT_OMIT_MICROSECONDS` / `OPT_NAIVE_UTC` / `OPT_UTC_Z` to an ISO 8601
/// string produced by `datetime.isoformat()`.
fn format_datetime(s: &str, ctx: EncodeCtx<'_>) -> String {
    let (body, offset) = split_iso_offset(s);
    let mut body = body.to_owned();
    let mut offset = offset.to_owned();

    // A `datetime` uses a `T` date/time separator; `date`/`time` do not. The
    // naive-UTC offset only applies to a full datetime.
    let is_datetime = body.contains('T');

    if ctx.omit_microseconds {
        if let Some(dot) = body.find('.') {
            body.truncate(dot);
        }
    }
    if ctx.naive_utc && is_datetime && offset.is_empty() {
        offset = "+00:00".to_owned();
    }
    if ctx.utc_z && offset == "+00:00" {
        offset = "Z".to_owned();
    }

    body.push_str(&offset);
    body
}

/// Split a trailing UTC offset (`Z`, `+HH:MM`, or `-HH:MM`) off an ISO 8601
/// string, returning `(body, offset)` where `offset` is empty if there is none.
fn split_iso_offset(s: &str) -> (&str, &str) {
    if let Some(body) = s.strip_suffix('Z') {
        return (body, "Z");
    }
    let bytes = s.as_bytes();
    if bytes.len() >= 6 {
        let tail = &bytes[bytes.len() - 6..];
        let signed = matches!(tail[0], b'+' | b'-');
        let shaped = tail[3] == b':'
            && tail[1].is_ascii_digit()
            && tail[2].is_ascii_digit()
            && tail[4].is_ascii_digit()
            && tail[5].is_ascii_digit();
        if signed && shaped {
            let split = s.len() - 6;
            return (&s[..split], &s[split..]);
        }
    }
    (s, "")
}

/// Handle the standard-library and numpy types, returning `None`
/// when `obj` is not one of them (so the caller falls back to `default`).
fn special_type_to_value(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Option<Value<'static>>> {
    // datetime / date / time → ISO 8601 string (honoring the datetime options),
    // unless datetimes are passed through to `default`.
    if let Some(value) = datetime_to_value(obj, ctx)? {
        return Ok(Some(value));
    }

    let type_obj = obj.get_type();
    let type_name = type_obj.name()?.to_string();

    // Enum → its value (IntEnum/StrEnum are handled earlier as int/str).
    if is_enum(obj)? {
        return Ok(Some(python_to_value(py, &obj.getattr("value")?, ctx)?));
    }

    // Dataclass instance → mapping of its fields (unless passed through).
    if !ctx.passthrough_dataclass
        && obj.hasattr("__dataclass_fields__")?
        && !obj.is_instance_of::<PyType>()
    {
        let fields = obj.getattr("__dataclass_fields__")?;
        let mut pairs = Vec::new();
        for key in fields.try_iter()? {
            let key = key?;
            let name: String = key.extract()?;
            let value = obj.getattr(name.as_str())?;
            pairs.push((
                Value::String(name.into()),
                python_to_value(py, &value, ctx)?,
            ));
        }
        return Ok(Some(Value::Mapping(pairs)));
    }

    // numpy arrays and scalars (opt-in).
    if ctx.serialize_numpy {
        if let Some(value) = numpy_to_value(py, obj, ctx)? {
            return Ok(Some(value));
        }
    }

    // Decimal → number; UUID / Path / other str-able stdlib types → string.
    // Gate on the defining module as well as the name so a third-party class that
    // happens to be called `Decimal` or `UUID` is not coerced through this path.
    let type_module = || {
        type_obj
            .getattr("__module__")
            .and_then(|m| m.extract::<String>())
    };
    match type_name.as_str() {
        "Decimal" if type_module()? == "decimal" => {
            return decimal_to_value(py, obj);
        }
        "UUID" if type_module()? == "uuid" => {
            return Ok(Some(Value::String(obj.str()?.to_string().into())))
        }
        _ => {}
    }

    // os.PathLike (pathlib.Path and friends) → string path.
    if obj.hasattr("__fspath__")? {
        let path = obj.call_method0("__fspath__")?;
        if let Ok(s) = path.extract::<String>() {
            return Ok(Some(Value::String(s.into())));
        }
    }

    Ok(None)
}

/// Convert a `decimal.Decimal` to a `Value` without silently losing precision.
///
/// The old path extracted an `f64`, which rounds a 30-digit integer or a
/// high-precision fraction down to a double. Instead:
/// - a non-finite Decimal (`NaN`/`Infinity`) keeps the `f64` path, so it renders
///   as `.nan`/`.inf` like any float;
/// - an integral Decimal becomes a `BigInt` of its exact digits (arbitrary
///   precision, re-reads as an `int`);
/// - a fractional Decimal that an `f64` captures exactly (its shortest repr
///   round-trips back to the same Decimal) becomes that `Float`, so a plain
///   `Decimal("3.14")` still emits as `3.14`;
/// - any other fractional Decimal keeps its exact digits as a string, so the
///   value is preserved in the output rather than rounded away.
fn decimal_to_value(py: Python<'_>, obj: &Bound<'_, PyAny>) -> PyResult<Option<Value<'static>>> {
    // `is_finite` is false for NaN/Infinity; fall back to the float rendering.
    if !obj.call_method0("is_finite")?.extract::<bool>()? {
        let f = obj.extract::<f64>().unwrap_or(f64::NAN);
        return Ok(Some(Value::Float(f)));
    }
    // Fixed-point form (`format(d, "f")`) gives the exact digits with no
    // scientific `E` notation, so `Decimal("1E+29")` becomes `100...0`.
    let fixed: String = obj.call_method1("__format__", ("f",))?.extract()?;
    if !fixed.contains('.') {
        return Ok(Some(Value::BigInt(fixed.into())));
    }
    // Emit as a plain float when an f64 captures the value to full working
    // precision, i.e. the float's shortest repr round-trips back to the same
    // Decimal (`Decimal(repr(float(d))) == d`).
    if let Ok(f) = obj.extract::<f64>() {
        let repr = pyo3::types::PyFloat::new(py, f).repr()?;
        let round_tripped = obj.get_type().call1((repr,))?;
        if obj.eq(&round_tripped)? {
            return Ok(Some(Value::Float(f)));
        }
    }
    // Higher precision than an f64 can hold: keep the exact digits verbatim.
    Ok(Some(Value::String(fixed.into())))
}

/// Whether `obj` is an `enum.Enum` instance (detected via its metaclass).
fn is_enum(obj: &Bound<'_, PyAny>) -> PyResult<bool> {
    let metaclass = obj.get_type().get_type();
    let name = metaclass.name()?.to_string();
    Ok(name == "EnumType" || name == "EnumMeta")
}

/// Cached `numpy.ndarray` / `numpy.generic` types, resolved once. `None` means
/// numpy is not importable, so nothing can be a numpy object.
static NUMPY_TYPES: OnceLock<Option<(Py<PyAny>, Py<PyAny>)>> = OnceLock::new();

/// Convert a numpy array (`tolist`) or scalar (`item`), but only for genuine
/// numpy objects. The type is checked with `isinstance` against numpy's exported
/// `ndarray`/`generic` classes rather than a `__module__` string prefix, so a
/// look-alike from another module (e.g. `numpycompat`) does not take this path.
fn numpy_to_value(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    ctx: EncodeCtx<'_>,
) -> PyResult<Option<Value<'static>>> {
    let types = NUMPY_TYPES.get_or_init(|| {
        let numpy = py.import("numpy").ok()?;
        let ndarray = numpy.getattr("ndarray").ok()?.unbind();
        let generic = numpy.getattr("generic").ok()?.unbind();
        Some((ndarray, generic))
    });
    let Some((ndarray, generic)) = types else {
        return Ok(None);
    };
    if !(obj.is_instance(ndarray.bind(py))? || obj.is_instance(generic.bind(py))?) {
        return Ok(None);
    }
    if let Ok(list) = obj.call_method0("tolist") {
        return Ok(Some(python_to_value(py, &list, ctx)?));
    }
    if let Ok(item) = obj.call_method0("item") {
        return Ok(Some(python_to_value(py, &item, ctx)?));
    }
    Ok(None)
}
