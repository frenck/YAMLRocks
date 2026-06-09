//! Build the Python exception hierarchy (defined in `yamlrocks.exceptions`) from
//! the core's internal error types, populating the structured `file`/`line`/
//! `column` attributes (and `include_stack`/`schema_path` where relevant).

use pyo3::import_exception;
use pyo3::prelude::*;

use crate::decode::{DecodeError, DecodeErrorKind};
use crate::include::{IncludeError, IncludeErrorKind};
use crate::scanner::ScanError;

import_exception!(yamlrocks.exceptions, YAMLRocksDecodeError);
import_exception!(yamlrocks.exceptions, YAMLRocksParseError);
import_exception!(yamlrocks.exceptions, YAMLRocksDuplicateKeyError);
import_exception!(yamlrocks.exceptions, YAMLRocksComplexKeyError);
import_exception!(yamlrocks.exceptions, YAMLRocksSchemaError);
import_exception!(yamlrocks.exceptions, YAMLRocksIncludeError);
import_exception!(yamlrocks.exceptions, YAMLRocksIncludeNotFoundError);
import_exception!(yamlrocks.exceptions, YAMLRocksCircularIncludeError);
import_exception!(yamlrocks.exceptions, YAMLRocksIncludeDepthError);
import_exception!(yamlrocks.exceptions, YAMLRocksIncludeConfinementError);
import_exception!(yamlrocks.exceptions, YAMLRocksSecretNotFoundError);
import_exception!(yamlrocks.exceptions, YAMLRocksEnvVarError);
import_exception!(yamlrocks.exceptions, YAMLRocksEncodeError);
import_exception!(yamlrocks.exceptions, YAMLRocksUnserializableError);

/// Emit a non-fatal diagnostic through Python's standard `logging`, on the
/// `yamlrocks` logger at WARNING level. Unlike a raised exception this never
/// interrupts a load: the caller keeps the (last-wins) result and continues.
///
/// Logging is chosen over `warnings.warn` deliberately: a log record is trivial
/// to route or silence with the standard logging configuration, whereas a
/// warning category obliges every caller to manage `warnings` filters. Any
/// failure to log is swallowed, since a diagnostic must never become fatal.
pub fn log_warning(py: Python<'_>, message: &str) {
    let _ = (|| -> PyResult<()> {
        let logger = py
            .import("logging")?
            .call_method1("getLogger", ("yamlrocks",))?;
        logger.call_method1("warning", (message,))?;
        Ok(())
    })();
}

/// Stamp `file`/`line`/`column` onto a freshly created error. `line`/`column`
/// are passed 0-based (as the scanner tracks them) and stored 1-based to match
/// the human-readable message text and editor conventions.
fn set_location(
    py: Python<'_>,
    err: &PyErr,
    file: Option<&str>,
    line: Option<u32>,
    column: Option<u32>,
) {
    let value = err.value(py);
    let _ = value.setattr("file", file);
    let _ = value.setattr("line", line.map(|l| l + 1));
    let _ = value.setattr("column", column.map(|c| c + 1));
}

fn located_message(message: &str, line: u32, column: u32) -> String {
    format!("{message} at line {}, column {}", line + 1, column + 1)
}

/// A malformed-YAML error from the composer/parser (round-trip and AST paths).
pub fn parse_error(py: Python<'_>, e: &ScanError, file: Option<&str>) -> PyErr {
    let err = YAMLRocksParseError::new_err(located_message(&e.message, e.span.line, e.span.column));
    set_location(py, &err, file, Some(e.span.line), Some(e.span.column));
    err
}

/// A malformed-YAML error from the fast decode path. Duplicate keys surface here
/// too; route them to their own class so callers can catch them specifically.
pub fn decode_error(py: Python<'_>, e: &DecodeError, file: Option<&str>) -> PyErr {
    let message = located_message(&e.message, e.span.line, e.span.column);
    let err = match e.kind {
        DecodeErrorKind::DuplicateKey => YAMLRocksDuplicateKeyError::new_err(message),
        DecodeErrorKind::ComplexKey => YAMLRocksComplexKeyError::new_err(message),
        DecodeErrorKind::Parse => YAMLRocksParseError::new_err(message),
    };
    set_location(py, &err, file, Some(e.span.line), Some(e.span.column));
    err
}

/// A duplicate mapping key reported by the dedicated round-trip check.
pub fn duplicate_key_error(py: Python<'_>, e: &ScanError, file: Option<&str>) -> PyErr {
    let err = YAMLRocksDuplicateKeyError::new_err(located_message(
        &e.message,
        e.span.line,
        e.span.column,
    ));
    set_location(py, &err, file, Some(e.span.line), Some(e.span.column));
    err
}

/// A JSON Schema validation failure. `schema_path` is the JSON-path of the node.
pub fn schema_error(
    py: Python<'_>,
    message: String,
    schema_path: &str,
    line: u32,
    column: u32,
) -> PyErr {
    let err = YAMLRocksSchemaError::new_err(message);
    let value = err.value(py);
    let _ = value.setattr("schema_path", schema_path);
    let _ = value.setattr("line", line + 1);
    let _ = value.setattr("column", column + 1);
    err
}

/// An `!include`/`!secret`/`!env_var` resolution failure, mapped to the precise
/// subclass and carrying the file plus (for include errors) the include chain.
pub fn include_error(py: Python<'_>, e: &IncludeError) -> PyErr {
    let message = format!("{e}");
    let file = e.path.to_string_lossy().into_owned();
    let stack: Vec<(String, u32)> = e
        .include_stack
        .iter()
        .map(|(path, line)| (path.to_string_lossy().into_owned(), line + 1))
        .collect();

    let (err, is_include) = match e.kind {
        IncludeErrorKind::NotFound => (YAMLRocksIncludeNotFoundError::new_err(message), true),
        IncludeErrorKind::Circular => (YAMLRocksCircularIncludeError::new_err(message), true),
        IncludeErrorKind::Depth => (YAMLRocksIncludeDepthError::new_err(message), true),
        IncludeErrorKind::Confinement => (YAMLRocksIncludeConfinementError::new_err(message), true),
        IncludeErrorKind::SecretNotFound => (YAMLRocksSecretNotFoundError::new_err(message), false),
        IncludeErrorKind::EnvVarUndefined => (YAMLRocksEnvVarError::new_err(message), false),
        IncludeErrorKind::Invalid => (YAMLRocksIncludeError::new_err(message), true),
    };

    let value = err.value(py);
    let _ = value.setattr("file", Some(file));
    // Carry the offending directive's location when known (e.g. an undefined
    // `!secret`/`!env_var`), so the default raise reports `.line`/`.column` too,
    // not just `.file`. Stored 1-based, matching the other located errors.
    if let Some(span) = e.span {
        let _ = value.setattr("line", Some(span.line + 1));
        let _ = value.setattr("column", Some(span.column + 1));
    }
    // include_stack lives only on the include subtree, not on secret/env errors.
    if is_include {
        let _ = value.setattr("include_stack", stack);
    }
    err
}

/// A value that cannot be serialized to YAML (no `default` handled it).
pub fn unserializable_error(message: String) -> PyErr {
    YAMLRocksUnserializableError::new_err(message)
}

/// A general write-side failure.
pub fn encode_error(message: String) -> PyErr {
    YAMLRocksEncodeError::new_err(message)
}

/// A read-side failure with no specific category or source location.
pub fn decode_message(message: impl Into<String>) -> PyErr {
    YAMLRocksDecodeError::new_err(message.into())
}
