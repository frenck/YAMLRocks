mod convert;
mod errors;
mod types;

use std::collections::HashMap;
use std::path::PathBuf;

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyList, PyString};

use crate::decode;
use crate::encode::{self, EmitOptions, NullStyle};
use crate::include::{IncludeResolver, ResolveTags};
use crate::resolver::Schema;
use crate::roundtrip::document::{YAMLRocksDocument, YAMLRocksDocumentView};
use crate::roundtrip::{composer, YamlNode};

pub(crate) use convert::py_int_from_decimal;
use convert::{
    annotate_node, node_to_python_with_tags, python_to_value, value_to_python_with, EncodeCtx,
    TagPolicy,
};
pub use types::{YAMLRocksAnnotatedDict, YAMLRocksAnnotatedList, YAMLRocksTag};

#[derive(Clone, Copy)]
struct AstConvertOptions<'a, 'py> {
    yaml_schema: Schema,
    tag_policy: TagPolicy<'a, 'py>,
    /// Annotate numeric scalars (int/float) too, not just strings
    /// (`OPT_ANNOTATE_NUMBERS`).
    annotate_numbers: bool,
    /// Reject a collection used as a mapping key instead of converting it
    /// (`OPT_REJECT_COMPLEX_KEYS`).
    reject_complex_keys: bool,
}

/// How to handle an undefined config-tag reference (`!secret` or `!env_var`): by
/// default neither field is active and the resolver raises. `warn` (from the
/// `*_NOT_FOUND_WARN` flag) logs each miss and continues; `callback` (the
/// `on_missing_*` argument) is invoked per miss as `(name, file, line)`. Either
/// makes the resolver collect misses rather than raise; both can be active.
struct MissingRefPolicy<'py> {
    warn: bool,
    callback: Option<Bound<'py, PyAny>>,
}

impl MissingRefPolicy<'_> {
    /// Whether the resolver should collect (not raise on) the misses this policy
    /// governs.
    fn collect(&self) -> bool {
        self.warn || self.callback.is_some()
    }
}

/// The missing-reference policies for a single load, one per config tag that can
/// be downgraded from a hard error to a collected miss.
struct MissingPolicies<'py> {
    secret: MissingRefPolicy<'py>,
    env_var: MissingRefPolicy<'py>,
}

/// Report each gathered miss: invoke the callback (if any) and log a WARNING (if
/// the flag is set), in resolution order, with 1-based locations. `warn_message`
/// builds the log text for the specific tag kind from `(name, file, line)`.
fn dispatch_missing(
    py: Python<'_>,
    misses: Vec<crate::include::MissingReference>,
    policy: &MissingRefPolicy<'_>,
    warn_message: impl Fn(&str, &str, u32) -> String,
) -> PyResult<()> {
    for miss in misses {
        let file = miss.file.to_string_lossy();
        let line = miss.line + 1;
        if let Some(callback) = &policy.callback {
            callback.call1((&miss.name, file.as_ref(), line))?;
        }
        if policy.warn {
            errors::log_warning(py, &warn_message(&miss.name, &file, line));
        }
    }
    Ok(())
}

/// Drain and report both kinds of collected misses from a resolver: undefined
/// secrets, then undefined environment variables, each through its own policy.
fn report_missing_refs(
    py: Python<'_>,
    resolver: &mut IncludeResolver,
    missing: &MissingPolicies<'_>,
) -> PyResult<()> {
    dispatch_missing(
        py,
        resolver.take_missing_secrets(),
        &missing.secret,
        |name, file, line| {
            format!("secret '{name}' is not defined in any secrets.yaml at {file}:{line}")
        },
    )?;
    dispatch_missing(
        py,
        resolver.take_missing_env_vars(),
        &missing.env_var,
        |name, file, line| format!("environment variable '{name}' is not defined at {file}:{line}"),
    )
}

// Option flags for `loads`/`dumps`, grouped by concern. The values form a
// contiguous bit set, so a new flag simply appends at the next free bit. The
// mask is `u64` because there are more than 32 flags.

// -- Reading: schema and YAML version --

pub const OPT_YAML_1_1: u64 = 1 << 0;
/// Use PyYAML's deliberately off-spec boolean set instead of the literal YAML
/// 1.1 spec: `yes/no/on/off/true/false` (and their case variants) are booleans,
/// but bare `y`/`Y`/`n`/`N` stay plain strings. The real 1.1 spec makes `y`/`n`
/// booleans; PyYAML's resolver drops them, and the PyYAML-based ecosystem (Home
/// Assistant, ESPHoME, Ansible) relies on that. Implies the YAML 1.1 schema, so
/// it works on its own, and it carries through the migration paths
/// (`OPT_YAML_1_1_WARN`, `OPT_UPGRADE_1_1`) so they agree on what is a boolean.
pub const OPT_PYYAML_COMPAT: u64 = 1 << 1;
pub const OPT_UPGRADE_1_1: u64 = 1 << 2;
/// Log a diagnostic (on the `yamlrocks` logger) for every plain scalar that
/// resolves to a different type under YAML 1.1 than under 1.2 (yes/no booleans,
/// `0777` octals, sexagesimals, ...). A migration aid: only meaningful alongside
/// `OPT_YAML_1_1` or `OPT_UPGRADE_1_1`, and a no-op without them.
pub const OPT_YAML_1_1_WARN: u64 = 1 << 3;

// -- Reading: result shape --

pub const OPT_ROUND_TRIP: u64 = 1 << 4;
pub const OPT_ANNOTATED: u64 = 1 << 5;
/// In annotated mode (`OPT_ANNOTATED`), also annotate numeric scalars: integers
/// become `YAMLRocksAnnotatedInt` and floats `YAMLRocksAnnotatedFloat`, carrying the
/// same `__line__`/`__column__`/`__file__`/`__end_line__`/`__end_column__`/
/// `__style__` as annotated strings, so a validation error on a number can point
/// at its exact line. Off by default and a no-op without `OPT_ANNOTATED`.
///
/// The trade-off: an annotated number is a `int`/`float` *subclass*, so
/// `isinstance(x, int)`, equality, and arithmetic all behave normally, but
/// `type(x) is int` is no longer `True`, and each number carries a small boxing
/// cost. Booleans and `None` are never annotated (`bool`/`NoneType` cannot be
/// subclassed in Python), which matches PyYAML.
pub const OPT_ANNOTATE_NUMBERS: u64 = 1 << 6;

// -- Reading: includes --

pub const OPT_INCLUDES: u64 = 1 << 7;
/// Make the `!include_dir_*` tags descend into subdirectories (`os.walk`-style)
/// instead of reading only the top level. Off by default.
pub const OPT_INCLUDE_DIR_RECURSIVE: u64 = 1 << 8;

// -- Reading: config tags (secrets and environment variables) --

pub const OPT_SECRETS: u64 = 1 << 9;
pub const OPT_ENV_VAR: u64 = 1 << 10;
/// Downgrade an undefined `!secret` from a hard error to a logged, non-fatal
/// event: instead of raising `YAMLRocksSecretNotFoundError` on the first missing
/// secret, log a `WARNING` on the `yamlrocks` logger (with the secret name and
/// requesting file/line) and resolve that node to `None`, so a single pass
/// reports every miss. Off by default (real loading stays fail-fast), requires
/// `OPT_SECRETS`, and is the zero-code counterpart to the `on_missing_secret`
/// callback. Only "name not defined" downgrades; a malformed, non-mapping, or
/// recursive `secrets.yaml` still raises.
pub const OPT_SECRET_NOT_FOUND_WARN: u64 = 1 << 11;
/// Downgrade an undefined `!env_var` (a bare variable, with no default, that is
/// unset) from a hard error to a logged, non-fatal event: instead of raising
/// `YAMLRocksEnvVarError`, log a `WARNING` on the `yamlrocks` logger and resolve
/// that node to `None`, so one pass reports every miss. Off by default, requires
/// `OPT_ENV_VAR`, and is the counterpart to the `on_missing_env_var` callback. A
/// variable that supplies a default (`!env_var NAME fallback`) is never a miss.
pub const OPT_ENV_VAR_NOT_FOUND_WARN: u64 = 1 << 12;

// -- Reading: tags and keys --

pub const OPT_PASSTHROUGH_TAG: u64 = 1 << 13;
pub const OPT_DUPLICATE_KEYS_ERROR: u64 = 1 << 14;
/// Log a non-fatal diagnostic (on the `yamlrocks` logger) for every duplicate
/// mapping key, while still keeping the last value. The non-fatal counterpart to
/// `OPT_DUPLICATE_KEYS_ERROR`; ignored when that fatal flag is also set.
pub const OPT_DUPLICATE_KEYS_WARN: u64 = 1 << 15;
/// Reject a collection (mapping or sequence) used as a mapping key, raising
/// `YAMLRocksComplexKeyError` instead of converting it to a hashable Python value
/// (a `tuple`/`frozenset`). Off by default: a complex key is valid YAML (spec
/// Example 2.11), so the spec-compliant accept-and-convert behavior is the
/// default, and this flag is for a consumer whose data model is scalar-keyed (a
/// config loader, say) that wants to catch one early with a precise location. The
/// common trigger is an unquoted whole-value template, `key: {{ x }}`, which YAML
/// reads as a mapping used as a key. Applies to every load-to-Python path (fast,
/// annotated, tag-resolving) and inside `!include`d files; `OPT_ROUND_TRIP` is
/// unaffected, as it models source bytes rather than Python containers.
pub const OPT_REJECT_COMPLEX_KEYS: u64 = 1 << 16;

// -- Writing: layout --

pub const OPT_INDENT_2: u64 = 1 << 17;
pub const OPT_INDENT_4: u64 = 1 << 18;
/// Emit a block sequence that is a mapping value at its key's column (`key:` then
/// `- item` aligned with the key) instead of indenting it one level. The
/// "indentless" style of `kubectl` and much of the Kubernetes ecosystem.
/// yamlrocks indents by default, matching the dominant configuration style.
/// Affects `dumps()`; the round-trip path preserves each sequence's source layout.
pub const OPT_INDENTLESS_SEQUENCES: u64 = 1 << 19;
pub const OPT_FLOW_STYLE: u64 = 1 << 20;
pub const OPT_SORT_KEYS: u64 = 1 << 21;
pub const OPT_EXPLICIT_START: u64 = 1 << 22;
pub const OPT_EXPLICIT_END: u64 = 1 << 23;

// -- Writing: scalar style --

/// When a scalar must be quoted, use single quotes (`'...'`) instead of the
/// default double quotes (`"..."`). yamlrocks double-quotes by default, matching
/// the recommended style; single quotes avoid backslash escaping for values with
/// many backslashes (a regex or a Windows path, say). A value that cannot be
/// single-quoted (it contains a line break) still falls back to double quotes.
pub const OPT_SINGLE_QUOTES: u64 = 1 << 24;
/// Emit null values as the explicit `null` keyword instead of the default empty
/// node (`key:`). yamlrocks leaves nulls blank by default, matching the dominant
/// real-world configuration style; this flag opts into the explicit spelling,
/// which suits data/spec formats (OpenAPI, for instance) where `null` is idiomatic.
/// Affects `dumps()` and the round-trip emitter's edited-in nulls; the per-call
/// `null_style=` argument overrides it.
pub const OPT_NULL_AS_KEYWORD: u64 = 1 << 25;
/// Emit null values as the `~` indicator instead of the default empty node. `~`
/// is unambiguous in every position, so it is used verbatim everywhere. Affects
/// `dumps()` and the round-trip emitter's edited-in nulls; the per-call
/// `null_style=` argument overrides it. Mutually exclusive with
/// `OPT_NULL_AS_KEYWORD` (setting both is a `ValueError`).
pub const OPT_NULL_AS_TILDE: u64 = 1 << 26;

// -- Writing: type serialization --

pub const OPT_SERIALIZE_NUMPY: u64 = 1 << 27;
pub const OPT_PASSTHROUGH_DATETIME: u64 = 1 << 28;
pub const OPT_PASSTHROUGH_DATACLASS: u64 = 1 << 29;
pub const OPT_OMIT_MICROSECONDS: u64 = 1 << 30;
pub const OPT_NAIVE_UTC: u64 = 1 << 31;
pub const OPT_UTC_Z: u64 = 1 << 32;

// -- Core functions --

#[pyfunction]
#[pyo3(signature = (data, /, *, option=None, include_dir=None, schema=None, schema_resolver=None, tag_handler=None, tags=None, root_path=None, on_missing_secret=None, on_missing_env_var=None))]
#[allow(clippy::too_many_arguments)]
pub fn loads(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    option: Option<u64>,
    include_dir: Option<PathBuf>,
    schema: Option<Py<PyAny>>,
    schema_resolver: Option<Py<PyAny>>,
    tag_handler: Option<Py<PyAny>>,
    tags: Option<Py<PyAny>>,
    // The on-disk path the content was read from, when known (`load(path)`
    // passes it). Root nodes report it as their source file in annotated and
    // round-trip modes; `None` for genuinely in-memory input.
    root_path: Option<PathBuf>,
    // Called once per undefined `!secret` as `on_missing_secret(name, file, line)`
    // instead of raising, so the caller can gather every miss in one pass. The
    // node resolves to `None` and the load continues. Observe-only.
    on_missing_secret: Option<Py<PyAny>>,
    // The `!env_var` counterpart: called per undefined variable (no default) as
    // `on_missing_env_var(name, file, line)` instead of raising.
    on_missing_env_var: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    // `schema` is reserved for JSON Schema validation (see `validate_schema`,
    // applied below when provided). It is either a schema `dict` or the string
    // `"auto"`, in which case the in-file `# yaml-language-server: $schema=...`
    // directive is detected and resolved via `schema_resolver`.
    let input = extract_input(data)?;
    let schema = resolve_schema_arg(py, &input, schema.as_ref(), schema_resolver.as_ref())?;
    let opts = option.unwrap_or(0);

    let upgrade = opts & OPT_UPGRADE_1_1 != 0;
    // PyYAML-compat is a 1.1 variant (it changes only the bool set), so it
    // implies the 1.1 schema. Upgrading also reads with the 1.1 schema first.
    let pyyaml_compat = opts & OPT_PYYAML_COMPAT != 0;
    // A document's own `%YAML` directive is authoritative for schema selection
    // and overrides the flags, so a file stamped `%YAML 1.2` (e.g. by `upgrade`)
    // is read as 1.2 even under `OPT_UPGRADE_1_1`/`OPT_YAML_1_1`.
    let yaml_11 = match crate::version::leading_yaml_version(&input) {
        Some(version) => crate::version::selects_yaml_11(version),
        None => opts & OPT_YAML_1_1 != 0 || upgrade || pyyaml_compat,
    };
    let yaml_schema = Schema::new(yaml_11, pyyaml_compat);
    let round_trip = opts & OPT_ROUND_TRIP != 0;
    let annotated = opts & OPT_ANNOTATED != 0;
    let resolve = resolve_tags(opts);
    let registry = bind_registry(py, tags.as_ref())?;
    // Duplicate-key rejection applies on every load path, not just the fast one.
    let dup_error = opts & OPT_DUPLICATE_KEYS_ERROR != 0;
    // Non-fatal duplicate-key logging (ignored when the fatal flag wins).
    let dup_warn = !dup_error && opts & OPT_DUPLICATE_KEYS_WARN != 0;
    // Log 1.1-only syntax as a migration aid; only meaningful in 1.1/upgrade mode.
    let yaml_11_warn = yaml_11 && opts & OPT_YAML_1_1_WARN != 0;
    // Whether `!include_dir_*` walks subdirectories (off by default).
    let dir_recursive = opts & OPT_INCLUDE_DIR_RECURSIVE != 0;
    // Reject a collection used as a mapping key instead of converting it.
    let reject_complex_keys = opts & OPT_REJECT_COMPLEX_KEYS != 0;
    // How to handle an undefined `!secret`/`!env_var` (default: raise).
    let missing = MissingPolicies {
        secret: MissingRefPolicy {
            warn: opts & OPT_SECRET_NOT_FOUND_WARN != 0,
            callback: on_missing_secret.as_ref().map(|c| c.bind(py).clone()),
        },
        env_var: MissingRefPolicy {
            warn: opts & OPT_ENV_VAR_NOT_FOUND_WARN != 0,
            callback: on_missing_env_var.as_ref().map(|c| c.bind(py).clone()),
        },
    };

    if round_trip {
        let schema = schema.as_ref().map(|s| s.bind(py));
        return loads_roundtrip(
            py,
            &input,
            resolve,
            include_dir,
            root_path,
            upgrade,
            schema,
            yaml_schema,
            dup_error,
            dup_warn,
            yaml_11_warn,
            dir_recursive,
            &missing,
            null_style_from_opts(opts)?,
            opts & OPT_SINGLE_QUOTES == 0,
        );
    }

    // Annotated mode and any application-tag resolution need source spans, so
    // they route through the rich AST rather than the fast-path Value tree.
    if annotated || resolve.any() {
        let schema = schema.as_ref().map(|s| s.bind(py));
        let handler = tag_handler.as_ref().map(|h| h.bind(py));
        let tag_policy = TagPolicy {
            registry: registry.as_ref(),
            handler,
            passthrough: opts & OPT_PASSTHROUGH_TAG != 0,
        };
        let convert = AstConvertOptions {
            yaml_schema,
            tag_policy,
            annotate_numbers: opts & OPT_ANNOTATE_NUMBERS != 0,
            reject_complex_keys,
        };
        return loads_via_ast(
            py,
            &input,
            annotated,
            resolve,
            include_dir,
            root_path,
            schema,
            convert,
            dup_error,
            dup_warn,
            yaml_11_warn,
            dir_recursive,
            &missing,
        );
    }

    // The scan/parse/resolve work touches no Python objects, so release the GIL
    // around it. This is what lets `async_loads` run off the event loop thread
    // without blocking it (and gives true parallelism on free-threaded builds).
    let warn = decode::WarnOptions {
        duplicate_keys: dup_warn,
        yaml_1_1: yaml_11_warn,
    };
    let (documents, warnings) = py
        .detach(|| {
            decode::decode_collecting(&input, yaml_schema, dup_error, reject_complex_keys, warn)
        })
        .map_err(|e| errors::decode_error(py, &e, None))?;
    for warning in &warnings {
        errors::log_warning(py, warning);
    }

    if documents.is_empty() {
        return Ok(py.None());
    }

    if let Some(schema) = schema.as_ref() {
        validate_schema(py, &input, schema.bind(py), yaml_11)?;
    }

    let handler = tag_handler.as_ref().map(|h| h.bind(py));
    let tag_policy = TagPolicy {
        registry: registry.as_ref(),
        handler,
        passthrough: opts & OPT_PASSTHROUGH_TAG != 0,
    };
    let result = value_to_python_with(py, &documents[0], tag_policy);
    // Drop the decoded value tree iteratively so a deeply nested document cannot
    // overflow the stack on teardown. See [`crate::stack`].
    for doc in documents {
        crate::stack::drop_value_tree(doc);
    }
    result
}

#[pyfunction]
#[pyo3(signature = (data, /, *, option=None, tag_handler=None, tags=None))]
pub fn loads_all(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    option: Option<u64>,
    tag_handler: Option<Py<PyAny>>,
    tags: Option<Py<PyAny>>,
) -> PyResult<Py<PyAny>> {
    let input = extract_input(data)?;
    let opts = option.unwrap_or(0);
    // `loads_all` is a multi-document, value-returning API: it takes no
    // `include_dir` and returns a list, so include/secret/env-var resolution and
    // the single-document round-trip option cannot be honored. Reject them rather
    // than silently ignoring them, which would hand back unresolved tags.
    const UNSUPPORTED: u64 = OPT_INCLUDES | OPT_SECRETS | OPT_ENV_VAR | OPT_ROUND_TRIP;
    if opts & UNSUPPORTED != 0 {
        return Err(PyValueError::new_err(
            "loads_all() does not support OPT_INCLUDES, OPT_SECRETS, OPT_ENV_VAR, or \
             OPT_ROUND_TRIP; use loads() for include resolution and round-trip editing",
        ));
    }
    let pyyaml_compat = opts & OPT_PYYAML_COMPAT != 0;
    // A leading `%YAML` directive overrides the flags (see `loads`). For a
    // multi-document stream this honors the first document's declaration.
    let yaml_11 = match crate::version::leading_yaml_version(&input) {
        Some(version) => crate::version::selects_yaml_11(version),
        None => opts & OPT_YAML_1_1 != 0 || opts & OPT_UPGRADE_1_1 != 0 || pyyaml_compat,
    };
    let yaml_schema = Schema::new(yaml_11, pyyaml_compat);
    let annotated = opts & OPT_ANNOTATED != 0;
    let annotate_numbers = opts & OPT_ANNOTATE_NUMBERS != 0;
    let dup_error = opts & OPT_DUPLICATE_KEYS_ERROR != 0;
    let dup_warn = !dup_error && opts & OPT_DUPLICATE_KEYS_WARN != 0;
    let yaml_11_warn = yaml_11 && opts & OPT_YAML_1_1_WARN != 0;
    let reject_complex_keys = opts & OPT_REJECT_COMPLEX_KEYS != 0;

    let registry = bind_registry(py, tags.as_ref())?;
    let handler = tag_handler.as_ref().map(|h| h.bind(py));
    let tag_policy = TagPolicy {
        registry: registry.as_ref(),
        handler,
        passthrough: opts & OPT_PASSTHROUGH_TAG != 0,
    };

    // Annotated mode needs source spans, so each document routes through the
    // rich AST rather than the fast-path `Value` tree. Empty `---` documents are
    // kept (as null) so the document count matches the fast path and PyYAML.
    if annotated {
        // Reject exactly what the default path rejects (the lenient composer
        // would otherwise annotate a malformed structure); see `loads_via_ast`.
        py.detach(|| decode::decode_with(&input, yaml_schema, false, reject_complex_keys))
            .map_err(|e| errors::decode_error(py, &e, None))?;
        let nodes = composer::compose_all(&input).map_err(|e| errors::parse_error(py, &e, None))?;
        if dup_error {
            composer::check_duplicate_keys(&nodes, yaml_schema)
                .map_err(|e| errors::duplicate_key_error(py, &e, None))?;
        } else if dup_warn {
            for warning in composer::collect_duplicate_keys(&nodes, yaml_schema) {
                errors::log_warning(py, &warning);
            }
        }
        if yaml_11_warn {
            for warning in composer::collect_yaml_11_divergences(&nodes, yaml_schema) {
                errors::log_warning(py, &warning);
            }
        }
        let list = PyList::empty(py);
        for node in &nodes {
            // Anchors do not cross document boundaries, so resolve each
            // document's aliases against only its own anchor map.
            let anchors = crate::roundtrip::anchors::build_anchor_map(std::slice::from_ref(node));
            list.append(annotate_node(
                py,
                node,
                &[],
                yaml_schema,
                tag_policy,
                &anchors,
                annotate_numbers,
            )?)?;
            // The anchor map's expanded nodes can be as deep as the document;
            // drop them iteratively so a deeply nested one cannot overflow the
            // stack on teardown. See [`crate::stack`].
            for (_, expanded) in anchors {
                crate::stack::drop_node_tree(expanded);
            }
        }
        let result = list.into_any().unbind();
        // Drop the composed AST iteratively for the same reason.
        for node in nodes {
            crate::stack::drop_node_tree(node);
        }
        return Ok(result);
    }

    let warn = decode::WarnOptions {
        duplicate_keys: dup_warn,
        yaml_1_1: yaml_11_warn,
    };
    // GIL-free scan/parse/resolve (see `loads`); materialization below re-takes it.
    let (documents, warnings) = py
        .detach(|| {
            decode::decode_collecting(&input, yaml_schema, dup_error, reject_complex_keys, warn)
        })
        .map_err(|e| errors::decode_error(py, &e, None))?;
    for warning in &warnings {
        errors::log_warning(py, warning);
    }

    let list = PyList::empty(py);
    for doc in &documents {
        list.append(value_to_python_with(py, doc, tag_policy)?)?;
    }
    let result = list.into_any().unbind();
    // Drop the decoded value tree iteratively so a deeply nested document cannot
    // overflow the stack on teardown. See [`crate::stack`].
    for doc in documents {
        crate::stack::drop_value_tree(doc);
    }
    Ok(result)
}

#[pyfunction]
#[pyo3(signature = (data, /))]
pub fn schema_ref(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    // Pure, I/O-free detection of the in-file schema directive. The scan reads
    // only the leading comment block and never touches the network.
    let input = extract_input(data)?;
    match crate::schema::schema_ref(&input) {
        Some(reference) => Ok(PyString::new(py, &reference).into_any().unbind()),
        None => Ok(py.None()),
    }
}

#[pyfunction]
#[pyo3(signature = (data, /))]
pub fn yaml_version(py: Python<'_>, data: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    // Pure, I/O-free detection of the document's `%YAML` version directive. The
    // scan reads only the stream prefix. Returns the declared `"major.minor"`
    // (e.g. `"1.1"`/`"1.2"`), or `None` when the document declares no version.
    let input = extract_input(data)?;
    match crate::version::leading_yaml_version(&input) {
        Some((major, minor)) => Ok(PyString::new(py, &format!("{major}.{minor}"))
            .into_any()
            .unbind()),
        None => Ok(py.None()),
    }
}

#[pyfunction]
#[pyo3(signature = (obj, /, *, default=None, option=None, null_style=None, tags=None, width=None))]
pub fn dumps(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    default: Option<Py<PyAny>>,
    option: Option<u64>,
    null_style: Option<&str>,
    tags: Option<Py<PyDict>>,
    width: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let opts = option.unwrap_or(0);

    // A round-trip document re-emits from its own preserved layout, so the
    // emit-shaping arguments (option, null_style, width, tags, default) do not
    // apply here and are intentionally ignored; round-trip styles win.
    if let Ok(doc) = obj.cast::<YAMLRocksDocument>() {
        return doc.borrow().to_yaml(py);
    }

    // A `YAMLRocksDocumentView` is a sub-view with no document-level layout to
    // preserve, so it is resolved to its plain value (aliases expanded) and
    // serialized like any object, matching `to_json`.
    let resolved;
    let obj = if obj.is_instance_of::<YAMLRocksDocumentView>() {
        resolved = obj.call_method0("to_dict")?;
        &resolved
    } else {
        obj
    };

    let mut emit_options = build_emit_options(opts);
    // The per-call `null_style=` argument overrides the option flags.
    emit_options.null_style = match null_style {
        Some(style) => parse_null_style(style)?,
        None => null_style_from_opts(opts)?,
    };
    // Best-effort line wrapping; 0 (the default) leaves lines unwrapped.
    emit_options.width = width.unwrap_or(0);
    let ctx = EncodeCtx {
        default: default.as_ref(),
        serialize_numpy: opts & OPT_SERIALIZE_NUMPY != 0,
        omit_microseconds: opts & OPT_OMIT_MICROSECONDS != 0,
        naive_utc: opts & OPT_NAIVE_UTC != 0,
        utc_z: opts & OPT_UTC_Z != 0,
        passthrough_datetime: opts & OPT_PASSTHROUGH_DATETIME != 0,
        passthrough_dataclass: opts & OPT_PASSTHROUGH_DATACLASS != 0,
        tags: tags.as_ref(),
        depth: 0,
    };
    let value = python_to_value(py, obj, ctx)?;
    // Emission is pure Rust over an owned value tree, so release the GIL for it;
    // this is what makes `async_dumps` non-blocking on the event loop.
    let bytes = py.detach(|| {
        let bytes = encode::encode(&value, &emit_options);
        // Drop the value tree iteratively (still GIL-free) so a deeply nested
        // object cannot overflow the stack on teardown. See [`crate::stack`].
        crate::stack::drop_value_tree(value);
        bytes
    });
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

#[pyfunction]
#[pyo3(signature = (obj, /, *, default=None, option=None))]
pub fn to_json(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    default: Option<Py<PyAny>>,
    option: Option<u64>,
) -> PyResult<Py<PyAny>> {
    let opts = option.unwrap_or(0);

    // An empty round-trip document (no nodes) projects to `null`, matching the
    // fast path where empty input loads as `None`.
    if let Ok(doc) = obj.cast::<YAMLRocksDocument>() {
        if doc.borrow().nodes.is_empty() {
            return Ok(PyBytes::new(py, b"null").into_any().unbind());
        }
    }

    // A round-trip `YAMLRocksDocument`/`YAMLRocksDocumentView` is resolved to its plain value
    // (aliases expanded) before projecting to JSON; everything else is encoded
    // straight from the Python object.
    let resolved;
    let target = if obj.is_instance_of::<YAMLRocksDocument>()
        || obj.is_instance_of::<YAMLRocksDocumentView>()
    {
        resolved = obj.call_method0("to_dict")?;
        &resolved
    } else {
        obj
    };

    let ctx = EncodeCtx {
        default: default.as_ref(),
        serialize_numpy: opts & OPT_SERIALIZE_NUMPY != 0,
        omit_microseconds: opts & OPT_OMIT_MICROSECONDS != 0,
        naive_utc: opts & OPT_NAIVE_UTC != 0,
        utc_z: opts & OPT_UTC_Z != 0,
        passthrough_datetime: opts & OPT_PASSTHROUGH_DATETIME != 0,
        passthrough_dataclass: opts & OPT_PASSTHROUGH_DATACLASS != 0,
        // Custom tags carry no JSON meaning; `Value::Tagged` emits its inner
        // value and the tag is dropped, so no registry is consulted here.
        tags: None,
        depth: 0,
    };
    let value = python_to_value(py, target, ctx)?;

    // JSON is compact by default; OPT_INDENT_2/4 pretty-print, OPT_SORT_KEYS sorts.
    let json_options = encode::json::JsonOptions {
        indent: if opts & OPT_INDENT_4 != 0 {
            4
        } else if opts & OPT_INDENT_2 != 0 {
            2
        } else {
            0
        },
        sort_keys: opts & OPT_SORT_KEYS != 0,
    };
    let bytes = py.detach(|| {
        let bytes = encode::json::encode_json(&value, &json_options);
        // Drop the value tree iteratively (still GIL-free) so a deeply nested
        // object cannot overflow the stack on teardown. See [`crate::stack`].
        crate::stack::drop_value_tree(value);
        bytes
    });
    let bytes = bytes.map_err(errors::encode_error)?;
    Ok(PyBytes::new(py, &bytes).into_any().unbind())
}

#[pyfunction]
#[pyo3(signature = (doc, /, *, include_dir = None))]
pub fn dump_includes(
    py: Python<'_>,
    doc: &YAMLRocksDocument,
    include_dir: Option<PathBuf>,
) -> PyResult<()> {
    // Write-back always targets the document's tracked source paths (recorded
    // when it was loaded). `include_dir` is optional and ignored (it does not
    // rebase writes), so it is accepted only for call-site symmetry with `load`.
    let _ = include_dir;
    let changes = get_include_changes(py, doc)?;
    for (path, content) in changes {
        std::fs::write(&path, &content)
            .map_err(|e| PyValueError::new_err(format!("cannot write {}: {e}", path.display())))?;
    }
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (doc, /))]
pub fn dump_includes_map(py: Python<'_>, doc: &YAMLRocksDocument) -> PyResult<Py<PyAny>> {
    let changes = get_include_changes(py, doc)?;
    let dict = PyDict::new(py);
    for (path, content) in changes {
        dict.set_item(path.to_string_lossy().as_ref(), PyBytes::new(py, &content))?;
    }
    Ok(dict.into_any().unbind())
}

// -- Internal helpers --

fn extract_input(data: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(s) = data.cast::<PyString>() {
        // `to_str` rejects a string carrying lone surrogates (not valid UTF-8)
        // instead of silently replacing them with U+FFFD, matching the strict
        // handling of `bytes` input and keeping any data loss visible.
        return Ok(s.to_str()?.to_owned());
    }
    if let Ok(b) = data.cast::<PyBytes>() {
        return bytes_to_string(b.as_bytes());
    }
    if let Ok(ba) = data.cast::<PyByteArray>() {
        // SAFETY: we copy out of the buffer immediately, without releasing the
        // GIL or mutating the bytearray in between.
        return bytes_to_string(unsafe { ba.as_bytes() });
    }
    // memoryview and other buffer-protocol objects.
    if let Ok(buffer) = PyBuffer::<u8>::get(data) {
        let bytes = buffer.to_vec(data.py())?;
        return bytes_to_string(&bytes);
    }
    Err(PyTypeError::new_err(
        "loads() requires str, bytes, bytearray, or a buffer (e.g. memoryview)",
    ))
}

/// The Unicode encoding of a YAML byte stream, detected per YAML 1.2 §5.2.
enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Utf32Le,
    Utf32Be,
}

/// Detect the encoding of a YAML byte stream from a leading byte order mark or,
/// failing that, the null-byte pattern of the first characters. YAML 1.2 §5.2
/// requires a stream to begin with an ASCII character (a directive, indicator,
/// or printable), so the placement of null bytes among the first few bytes is a
/// reliable signal for a BOM-less UTF-16/UTF-32 stream.
fn detect_encoding(b: &[u8]) -> Encoding {
    // Check the UTF-32 BOMs before the UTF-16 ones: a UTF-32 little-endian BOM
    // (`FF FE 00 00`) begins with the UTF-16 little-endian BOM (`FF FE`).
    if b.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
        Encoding::Utf32Be
    } else if b.starts_with(&[0xFF, 0xFE, 0x00, 0x00]) {
        Encoding::Utf32Le
    } else if b.starts_with(&[0xFE, 0xFF]) {
        Encoding::Utf16Be
    } else if b.starts_with(&[0xFF, 0xFE]) {
        Encoding::Utf16Le
    } else {
        // No (UTF-16/32) BOM: infer from null-byte placement. A leading UTF-8 BOM
        // is left for the reader to strip.
        match b {
            [0x00, 0x00, 0x00, _, ..] => Encoding::Utf32Be,
            [_, 0x00, 0x00, 0x00, ..] => Encoding::Utf32Le,
            [0x00, _, ..] => Encoding::Utf16Be,
            [_, 0x00, ..] => Encoding::Utf16Le,
            _ => Encoding::Utf8,
        }
    }
}

/// Decode YAML input bytes to a UTF-8 `String`, accepting UTF-8, UTF-16, and
/// UTF-32 (with or without a BOM) as the spec requires. A non-UTF-8 stream is
/// transcoded; any leading byte order mark survives as U+FEFF for the reader to
/// strip. Invalid input raises a clear, encoding-specific error rather than the
/// generic "invalid UTF-8" (or, worse, silently mis-parsing UTF-16 as Latin-1).
fn bytes_to_string(bytes: &[u8]) -> PyResult<String> {
    match detect_encoding(bytes) {
        Encoding::Utf8 => String::from_utf8(bytes.to_vec())
            .map_err(|e| PyValueError::new_err(format!("invalid UTF-8: {e}"))),
        Encoding::Utf16Le => decode_utf16(bytes, false),
        Encoding::Utf16Be => decode_utf16(bytes, true),
        Encoding::Utf32Le => decode_utf32(bytes, false),
        Encoding::Utf32Be => decode_utf32(bytes, true),
    }
}

fn decode_utf16(bytes: &[u8], big_endian: bool) -> PyResult<String> {
    if bytes.len() % 2 != 0 {
        return Err(PyValueError::new_err(
            "invalid UTF-16: byte count is not a multiple of 2",
        ));
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| {
            let pair = [c[0], c[1]];
            if big_endian {
                u16::from_be_bytes(pair)
            } else {
                u16::from_le_bytes(pair)
            }
        })
        .collect();
    String::from_utf16(&units)
        .map_err(|_| PyValueError::new_err("invalid UTF-16: unpaired surrogate"))
}

fn decode_utf32(bytes: &[u8], big_endian: bool) -> PyResult<String> {
    if bytes.len() % 4 != 0 {
        return Err(PyValueError::new_err(
            "invalid UTF-32: byte count is not a multiple of 4",
        ));
    }
    bytes
        .chunks_exact(4)
        .map(|c| {
            let quad = [c[0], c[1], c[2], c[3]];
            let code = if big_endian {
                u32::from_be_bytes(quad)
            } else {
                u32::from_le_bytes(quad)
            };
            char::from_u32(code)
                .ok_or_else(|| PyValueError::new_err("invalid UTF-32: not a Unicode scalar value"))
        })
        .collect()
}

/// Resolve the `schema` argument to a concrete schema object (or `None`).
///
/// - A schema `dict` (or any non-string value) is returned unchanged.
/// - The string `"auto"` triggers in-file detection: the
///   `# yaml-language-server: $schema=...` directive is read from `input` and,
///   if present, passed to `schema_resolver` (a caller-supplied callable
///   `ref -> dict | None`). Validation is skipped when there is no directive or
///   the resolver returns `None`.
///
/// YAMLRocks never fetches the reference itself; resolution (including any
/// network or filesystem access) is entirely the caller's responsibility. This
/// keeps parsing free of surprise I/O and avoids SSRF and performance hazards.
fn resolve_schema_arg(
    py: Python<'_>,
    input: &str,
    schema: Option<&Py<PyAny>>,
    schema_resolver: Option<&Py<PyAny>>,
) -> PyResult<Option<Py<PyAny>>> {
    let Some(schema) = schema else {
        if schema_resolver.is_some() {
            return Err(PyValueError::new_err(
                "schema_resolver requires schema=\"auto\"",
            ));
        }
        return Ok(None);
    };

    let bound = schema.bind(py);
    let is_auto = bound
        .cast::<PyString>()
        .is_ok_and(|s| s.to_str().is_ok_and(|s| s == "auto"));

    if !is_auto {
        return Ok(Some(schema.clone_ref(py)));
    }

    let Some(resolver) = schema_resolver else {
        return Err(PyValueError::new_err(
            "schema=\"auto\" requires a schema_resolver callable to resolve the in-file reference",
        ));
    };

    let Some(reference) = crate::schema::schema_ref(input) else {
        return Ok(None);
    };

    let resolved = resolver.bind(py).call1((reference,))?;
    if resolved.is_none() {
        return Ok(None);
    }
    Ok(Some(resolved.unbind()))
}

/// Validate `input` against a JSON Schema (a Python dict). Re-parses to the
/// rich AST so that errors carry an accurate source location.
fn validate_schema(
    py: Python<'_>,
    input: &str,
    schema: &Bound<'_, PyAny>,
    yaml_11: bool,
) -> PyResult<()> {
    let nodes = composer::compose(input).map_err(|e| errors::parse_error(py, &e, None))?;
    validate_schema_root(py, &nodes, Some(schema), yaml_11)
}

fn validate_schema_root(
    py: Python<'_>,
    nodes: &[YamlNode],
    schema: Option<&Bound<'_, PyAny>>,
    yaml_11: bool,
) -> PyResult<()> {
    let Some(schema) = schema else {
        return Ok(());
    };
    let Some(root) = nodes.first() else {
        return Ok(());
    };
    validate_schema_node(py, root, schema, yaml_11)
}

fn validate_schema_node(
    py: Python<'_>,
    root: &YamlNode,
    schema: &Bound<'_, PyAny>,
    yaml_11: bool,
) -> PyResult<()> {
    let schema_value = python_to_value(py, schema, EncodeCtx::default())?;
    let schema_errors = crate::schema::validate(root, &schema_value, yaml_11);
    if let Some(e) = schema_errors.first() {
        return Err(errors::schema_error(
            py,
            format!(
                "schema validation failed: {} at {} (line {}, column {})",
                e.message,
                e.path,
                e.span.line + 1,
                e.span.column + 1,
            ),
            &e.path,
            e.span.line,
            e.span.column,
        ));
    }
    Ok(())
}

/// Map the option bits to the set of application tags to resolve.
fn resolve_tags(opts: u64) -> ResolveTags {
    ResolveTags {
        includes: opts & OPT_INCLUDES != 0,
        secrets: opts & OPT_SECRETS != 0,
        env_var: opts & OPT_ENV_VAR != 0,
    }
}

/// Bind the optional `tags` argument as a `{tag: callable}` dict.
///
/// `YAMLRocksTags` is a `dict` subclass, so both a plain mapping and a `YAMLRocksTags` registry
/// arrive here as a `PyDict`. Anything else is a usage error.
fn bind_registry<'py>(
    py: Python<'py>,
    tags: Option<&Py<PyAny>>,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    match tags {
        Some(obj) => {
            let dict = obj.bind(py).cast::<PyDict>().map_err(|_| {
                PyTypeError::new_err(
                    "tags must be a dict (or Tags) mapping each tag name to a callable",
                )
            })?;
            Ok(Some(dict.clone()))
        }
        None => Ok(None),
    }
}

#[allow(clippy::too_many_arguments)]
fn loads_roundtrip(
    py: Python<'_>,
    input: &str,
    tags: ResolveTags,
    include_dir: Option<PathBuf>,
    root_path: Option<PathBuf>,
    upgrade: bool,
    schema: Option<&Bound<'_, PyAny>>,
    yaml_schema: Schema,
    dup_error: bool,
    dup_warn: bool,
    yaml_11_warn: bool,
    dir_recursive: bool,
    missing: &MissingPolicies<'_>,
    null_style: NullStyle,
    double_quotes: bool,
) -> PyResult<Py<PyAny>> {
    let (mut nodes, file_map, file_sources) = if tags.any() {
        let base_dir = include_dir.unwrap_or_else(|| PathBuf::from("."));
        let mut resolver = IncludeResolver::new(base_dir, tags)
            .with_dir_recursive(dir_recursive)
            .with_collect_missing_secrets(missing.secret.collect())
            .with_collect_missing_env_vars(missing.env_var.collect());
        let nodes = resolver
            .load_str(input, root_path)
            .map_err(|e| errors::include_error(py, &e))?;
        // Surface any non-fatal diagnostics gathered during resolution.
        for warning in resolver.take_warnings() {
            errors::log_warning(py, &warning);
        }
        report_missing_refs(py, &mut resolver, missing)?;
        let (file_map, file_sources) = resolver.into_parts();
        (nodes, file_map, file_sources)
    } else {
        let nodes = composer::compose(input).map_err(|e| errors::parse_error(py, &e, None))?;
        (nodes, Vec::new(), Vec::new())
    };

    // Flag 1.1-only syntax against the original 1.1 nodes, before any upgrade
    // rewrites them to canonical 1.2.
    if yaml_11_warn {
        for warning in composer::collect_yaml_11_divergences(&nodes, yaml_schema) {
            errors::log_warning(py, &warning);
        }
    }

    // Round-trip intentionally returns a `YAMLRocksDocument` even for empty or
    // comment-only input: the fast and annotated paths drop to `None` because
    // there is no value, but round-trip must keep the source and comments so it
    // can re-emit them byte-for-byte.
    if upgrade {
        crate::roundtrip::upgrade::upgrade_to_yaml_1_2(&mut nodes, yaml_schema);
    }
    if dup_error {
        composer::check_duplicate_keys(&nodes, yaml_schema)
            .map_err(|e| errors::duplicate_key_error(py, &e, None))?;
    } else if dup_warn {
        for warning in composer::collect_duplicate_keys(&nodes, yaml_schema) {
            errors::log_warning(py, &warning);
        }
    }
    validate_schema_root(py, &nodes, schema, yaml_schema.is_yaml_11())?;
    let doc = YAMLRocksDocument::with_file_map(nodes, file_map, file_sources)
        .with_source(input.to_owned())
        .with_null_style(null_style)
        .with_double_quotes(double_quotes)
        .with_upgraded(upgrade);
    Ok(Py::new(py, doc)?.into_any())
}

/// Load via the rich AST, used for annotated mode and application-tag
/// resolution. Both need source spans that the fast-path `Value` tree omits.
#[allow(clippy::too_many_arguments)]
fn loads_via_ast(
    py: Python<'_>,
    input: &str,
    annotated: bool,
    tags: ResolveTags,
    include_dir: Option<PathBuf>,
    root_path: Option<PathBuf>,
    schema: Option<&Bound<'_, PyAny>>,
    convert: AstConvertOptions<'_, '_>,
    dup_error: bool,
    dup_warn: bool,
    yaml_11_warn: bool,
    dir_recursive: bool,
    missing: &MissingPolicies<'_>,
) -> PyResult<Py<PyAny>> {
    // Annotated mode must reject exactly what the default (fast) path rejects:
    // the round-trip composer is deliberately lenient (it preserves bytes over
    // enforcing grammar), so without this gate an invalid document would be
    // silently annotated into a malformed structure. Validate the document's
    // structure through the same decoder the default path uses and surface its
    // error verbatim (same message, same line/column). The decoder touches no
    // Python objects, so run it with the GIL released. Round-trip and pure
    // tag-resolution loads stay lenient and skip this.
    //
    // The gate also runs for a tag-only load when `OPT_REJECT_COMPLEX_KEYS` is
    // set, so a rejected complex key surfaces the same located error there too.
    if annotated || convert.reject_complex_keys {
        py.detach(|| {
            decode::decode_with(
                input,
                convert.yaml_schema,
                false,
                convert.reject_complex_keys,
            )
        })
        .map_err(|e| errors::decode_error(py, &e, None))?;
    }

    let (nodes, file_map, file_sources) = if tags.any() {
        let base_dir = include_dir.unwrap_or_else(|| PathBuf::from("."));
        let mut resolver = IncludeResolver::new(base_dir, tags)
            .with_dir_recursive(dir_recursive)
            .with_collect_missing_secrets(missing.secret.collect())
            .with_collect_missing_env_vars(missing.env_var.collect());
        let nodes = resolver
            .load_str(input, root_path)
            .map_err(|e| errors::include_error(py, &e))?;
        // Surface any non-fatal diagnostics gathered during resolution.
        for warning in resolver.take_warnings() {
            errors::log_warning(py, &warning);
        }
        report_missing_refs(py, &mut resolver, missing)?;
        let (file_map, file_sources) = resolver.into_parts();
        (nodes, file_map, file_sources)
    } else {
        let nodes = composer::compose(input).map_err(|e| errors::parse_error(py, &e, None))?;
        (nodes, Vec::new(), Vec::new())
    };

    // The root gate above validates the root document, but the round-trip
    // composer that resolves `!include`d files is lenient, so a structurally
    // invalid included file would slip through and only blow up later during
    // Python conversion (with no source location). Validate each included file
    // through the same decoder, so its structural error surfaces as the same
    // `YAMLRocksParseError` it would raise as a root document, with `.file` set to
    // that file. Index 0 is the root, already validated; skip it.
    if annotated || convert.reject_complex_keys {
        for (path, source) in file_map.iter().zip(&file_sources).skip(1) {
            if let Some(content) = source {
                decode::decode_with(
                    content,
                    convert.yaml_schema,
                    false,
                    convert.reject_complex_keys,
                )
                .map_err(|e| errors::decode_error(py, &e, Some(&path.to_string_lossy())))?;
            }
        }
    }

    if nodes.is_empty() {
        return Ok(py.None());
    }

    if dup_error {
        composer::check_duplicate_keys(&nodes, convert.yaml_schema)
            .map_err(|e| errors::duplicate_key_error(py, &e, None))?;
    } else if dup_warn {
        for warning in composer::collect_duplicate_keys(&nodes, convert.yaml_schema) {
            errors::log_warning(py, &warning);
        }
    }
    if yaml_11_warn {
        for warning in composer::collect_yaml_11_divergences(&nodes, convert.yaml_schema) {
            errors::log_warning(py, &warning);
        }
    }

    validate_schema_root(py, &nodes, schema, convert.yaml_schema.is_yaml_11())?;

    let paths: Vec<String> = file_map
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    let anchors = crate::roundtrip::anchors::build_anchor_map(&nodes);
    if annotated {
        annotate_node(
            py,
            &nodes[0],
            &paths,
            convert.yaml_schema,
            convert.tag_policy,
            &anchors,
            convert.annotate_numbers,
        )
    } else {
        node_to_python_with_tags(
            py,
            &nodes[0],
            convert.yaml_schema,
            convert.tag_policy,
            &anchors,
        )
    }
}

/// Re-emit each source file touched by an include-aware document, returning the
/// new bytes keyed by path. Errors if the document was not loaded with
/// `OPT_INCLUDES`, since there is then no file map to write back to.
fn get_include_changes(
    py: Python<'_>,
    doc: &YAMLRocksDocument,
) -> PyResult<HashMap<PathBuf, Vec<u8>>> {
    if doc.file_map.is_empty() {
        return Err(PyValueError::new_err(
            "document was not loaded with OPT_INCLUDES; nothing to write back",
        ));
    }
    crate::include::compute_include_changes(&doc.nodes, &doc.file_map, &doc.file_sources)
        .map_err(|e| errors::include_error(py, &e))
}

fn build_emit_options(opts: u64) -> EmitOptions {
    let indent = if opts & OPT_INDENT_4 != 0 { 4 } else { 2 };

    EmitOptions {
        indent,
        sort_keys: opts & OPT_SORT_KEYS != 0,
        flow_style: opts & OPT_FLOW_STYLE != 0,
        explicit_start: opts & OPT_EXPLICIT_START != 0,
        explicit_end: opts & OPT_EXPLICIT_END != 0,
        // The null style is resolved separately (it can fail) and assigned by the
        // caller; default it here so this stays infallible.
        null_style: NullStyle::Empty,
        double_quotes: opts & OPT_SINGLE_QUOTES == 0,
        indentless_sequences: opts & OPT_INDENTLESS_SEQUENCES != 0,
        // Resolved separately from the `width=` argument by the caller.
        width: 0,
    }
}

/// Resolve the null style from the option flags. The default is an empty node
/// (`key:`), matching the dominant real-world configuration style; the two flags
/// opt into the alternatives and are mutually exclusive (setting both is a
/// `ValueError`). Shared by `dumps` and the round-trip emitter's edited-in nulls.
pub fn null_style_from_opts(opts: u64) -> PyResult<NullStyle> {
    match (
        opts & OPT_NULL_AS_KEYWORD != 0,
        opts & OPT_NULL_AS_TILDE != 0,
    ) {
        (true, true) => Err(PyValueError::new_err(
            "OPT_NULL_AS_KEYWORD and OPT_NULL_AS_TILDE are mutually exclusive",
        )),
        (true, false) => Ok(NullStyle::Null),
        (false, true) => Ok(NullStyle::Tilde),
        (false, false) => Ok(NullStyle::Empty),
    }
}

/// Map the `null_style=` argument to a [`NullStyle`]. Accepts `"null"`, `"empty"`,
/// and `"~"`/`"tilde"`; anything else is a `ValueError`.
fn parse_null_style(value: &str) -> PyResult<NullStyle> {
    match value {
        "null" => Ok(NullStyle::Null),
        "empty" => Ok(NullStyle::Empty),
        "~" | "tilde" => Ok(NullStyle::Tilde),
        other => Err(PyValueError::new_err(format!(
            "null_style must be 'null', 'empty', or '~', not {other:?}"
        ))),
    }
}
