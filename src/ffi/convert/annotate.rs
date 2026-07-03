use std::collections::HashMap;
use std::sync::OnceLock;

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyFloat, PyList};

use crate::ffi::types::{YAMLRocksAnnotatedDict, YAMLRocksAnnotatedList};
use crate::resolver::{ResolvedValue, Schema};
use crate::roundtrip::value::{
    is_ast_merge_key, key_is_collection, mapping_has_merge_key, merge_converted_into,
    node_to_python_cached, node_to_python_key, ObjectCache,
};
use crate::roundtrip::{YamlNode, YamlNodeKind};

use super::decode::resolve_tagged;
use super::TagPolicy;

/// Cached references to the pure-Python annotated-scalar classes.
static ANNOTATED_STR: OnceLock<Py<PyAny>> = OnceLock::new();
static ANNOTATED_INT: OnceLock<Py<PyAny>> = OnceLock::new();
static ANNOTATED_FLOAT: OnceLock<Py<PyAny>> = OnceLock::new();

/// Build an annotated number (`YAMLRocksAnnotatedInt`/`Float`) from an already
/// resolved Python `value`, carrying the same source metadata as an annotated
/// string. Used only when `OPT_ANNOTATE_NUMBERS` is set.
#[allow(clippy::too_many_arguments)]
fn annotated_number(
    py: Python<'_>,
    cell: &OnceLock<Py<PyAny>>,
    class_name: &str,
    value: Py<PyAny>,
    line: u32,
    column: u32,
    config_file: Option<&str>,
    end_line: u32,
    end_column: u32,
    style: &str,
    source_tag: Option<&str>,
    source_target: Option<&str>,
    offset: usize,
    end_offset: usize,
) -> PyResult<Py<PyAny>> {
    if cell.get().is_none() {
        let cls = py.import("yamlrocks")?.getattr(class_name)?.unbind();
        let _ = cell.set(cls);
    }
    let cls = cell.get().expect("annotated number class cached");
    Ok(cls
        .bind(py)
        .call1((
            value,
            line,
            column,
            config_file,
            end_line,
            end_column,
            style,
            source_tag,
            source_target,
            offset,
            end_offset,
        ))?
        .unbind())
}

#[allow(clippy::too_many_arguments)]
fn annotated_str(
    py: Python<'_>,
    value: &str,
    line: u32,
    column: u32,
    config_file: Option<&str>,
    end_line: u32,
    end_column: u32,
    style: &str,
    source_tag: Option<&str>,
    source_target: Option<&str>,
    offset: usize,
    end_offset: usize,
) -> PyResult<Py<PyAny>> {
    if ANNOTATED_STR.get().is_none() {
        let cls = py
            .import("yamlrocks")?
            .getattr("YAMLRocksAnnotatedStr")?
            .unbind();
        let _ = ANNOTATED_STR.set(cls);
    }
    let cls = ANNOTATED_STR.get().expect("YAMLRocksAnnotatedStr cached");
    Ok(cls
        .bind(py)
        .call1((
            value,
            line,
            column,
            config_file,
            end_line,
            end_column,
            style,
            source_tag,
            source_target,
            offset,
            end_offset,
        ))?
        .unbind())
}

// -- AST -> annotated Python --

/// Recursively convert a round-trip AST node into annotated Python objects.
/// Mappings and sequences become `YAMLRocksAnnotatedDict`/`YAMLRocksAnnotatedList` carrying their
/// source line/column and originating file; scalars resolve to plain values.
///
/// Aliases share object identity with their anchor (PyYAML's behavior): a
/// `*alias` yields the same Python object the `&anchor` produced, so mutating
/// one is seen through all references.
pub fn annotate_node(
    py: Python<'_>,
    node: &YamlNode,
    paths: &[String],
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
    annotate_numbers: bool,
) -> PyResult<Py<PyAny>> {
    let mut cache = ObjectCache::new();
    annotate_node_cached(
        py,
        node,
        paths,
        schema,
        tags,
        anchors,
        annotate_numbers,
        &mut cache,
    )
}

#[allow(clippy::too_many_arguments)]
fn annotate_node_cached(
    py: Python<'_>,
    node: &YamlNode,
    paths: &[String],
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
    annotate_numbers: bool,
    cache: &mut ObjectCache,
) -> PyResult<Py<PyAny>> {
    // Grow the native stack on demand so a deeply nested document (bounded by the
    // composer's `MAX_DEPTH`) cannot overflow a small thread stack during the
    // annotated walk; the recursion re-enters here at each level. See
    // [`crate::stack`].
    crate::stack::guard(|| {
        annotate_node_cached_inner(
            py,
            node,
            paths,
            schema,
            tags,
            anchors,
            annotate_numbers,
            cache,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn annotate_node_cached_inner(
    py: Python<'_>,
    node: &YamlNode,
    paths: &[String],
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
    annotate_numbers: bool,
    cache: &mut ObjectCache,
) -> PyResult<Py<PyAny>> {
    // An alias resolves to its anchor's existing object (the same instance),
    // never a fresh one. The cache is populated by the time a valid alias is
    // reached; the anchor map is a defensive fallback for invalid forward refs.
    if let YamlNodeKind::Alias(name) = &node.kind {
        if let Some(obj) = cache.get(name) {
            return Ok(obj.clone_ref(py));
        }
        return match anchors.get(name) {
            Some(target) => annotate_node_cached(
                py,
                target,
                paths,
                schema,
                tags,
                anchors,
                annotate_numbers,
                cache,
            ),
            None => Ok(py.None()),
        };
    }

    // Borrow the file path and provenance tags: the scalar branch only needs a
    // `&str`, so cloning per node would heap-allocate a filename for every
    // scalar just to borrow it. Only the two container branches store an owned
    // `String` in their pyclass, and they clone locally.
    let config_file = paths.get(node.span.file_id as usize).map(String::as_str);
    let line = node.span.line + 1;
    let column = node.span.column + 1;
    // 1-based end position (just past the node's last character), mirroring
    // PyYAML's `end_mark`, so consumers can report a full source span. Exact for
    // quoted and escaped scalars (the composer derives it from the true end byte
    // offset), unlike the approximate `YamlNode::end`.
    let end_line = node.end_line + 1;
    let end_column = node.end_column + 1;
    // 0-based source byte range. Exact (the scanner records each scalar's true
    // end), so a consumer can slice the verbatim source bytes of a node.
    let offset = node.span.offset;
    let end_offset = node.end_offset;
    // The config/custom tag that produced this node (`!secret`, `!env_var`,
    // `!include*`, or a custom `!mytag`), and the directive argument it carried,
    // for provenance.
    let source_tag = node.source_tag();
    let source_target = node.source_target();

    let obj = match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            let init = PyClassInitializer::from(YAMLRocksAnnotatedDict {
                __line__: line,
                __column__: column,
                __file__: config_file.map(str::to_owned),
                __end_line__: end_line,
                __end_column__: end_column,
                __offset__: offset,
                __end_offset__: end_offset,
                __source_tag__: source_tag.map(str::to_owned),
                __source_target__: source_target.map(str::to_owned),
            });
            let obj = Bound::new(py, init)?;
            let dict = obj.as_any().cast::<PyDict>()?;
            let has_merge = mapping_has_merge_key(pairs);
            for (key, val) in pairs {
                let py_val = annotate_node_cached(
                    py,
                    val,
                    paths,
                    schema,
                    tags,
                    anchors,
                    annotate_numbers,
                    cache,
                )?;
                // Apply a `<<` merge like the fast path, so annotated data reads
                // the same as `loads()`. The merged-in entries keep their own
                // annotations (they come from the source mapping's nodes).
                if has_merge && is_ast_merge_key(key) {
                    if let Some(preserve) = merge_converted_into(dict, py_val.bind(py))? {
                        let py_key = annotate_node_cached(
                            py,
                            key,
                            paths,
                            schema,
                            tags,
                            anchors,
                            annotate_numbers,
                            cache,
                        )?;
                        if !dict.contains(&py_key)? {
                            dict.set_item(py_key, preserve)?;
                        }
                    }
                    continue;
                }
                // Annotate keys too, not just values: a string key becomes an
                // `YAMLRocksAnnotatedStr` carrying its own line/column/file, so callers can
                // point an error at the exact key. `YAMLRocksAnnotatedStr` is a `str`
                // subclass (hashable, equal to the plain string), so dict lookups
                // with a plain `str` keep working; non-string keys stay plain.
                // A collection key cannot be an (unhashable) annotated container;
                // render it as a hashable tuple (of tuples), matching the fast path.
                let py_key = if key_is_collection(key, anchors) {
                    node_to_python_key(py, key, schema, anchors, cache)
                } else {
                    annotate_node_cached(
                        py,
                        key,
                        paths,
                        schema,
                        tags,
                        anchors,
                        annotate_numbers,
                        cache,
                    )?
                };
                dict.set_item(py_key, py_val)?;
            }
            Ok(obj.into_any().unbind())
        }
        YamlNodeKind::Sequence(items) => {
            let init = PyClassInitializer::from(YAMLRocksAnnotatedList {
                __line__: line,
                __column__: column,
                __file__: config_file.map(str::to_owned),
                __end_line__: end_line,
                __end_column__: end_column,
                __offset__: offset,
                __end_offset__: end_offset,
                __source_tag__: source_tag.map(str::to_owned),
                __source_target__: source_target.map(str::to_owned),
            });
            let obj = Bound::new(py, init)?;
            let list = obj.as_any().cast::<PyList>()?;
            for item in items {
                list.append(annotate_node_cached(
                    py,
                    item,
                    paths,
                    schema,
                    tags,
                    anchors,
                    annotate_numbers,
                    cache,
                )?)?;
            }
            Ok(obj.into_any().unbind())
        }
        // String scalars become YAMLRocksAnnotatedStr (with the source location).
        // With OPT_ANNOTATE_NUMBERS, ints/floats are likewise annotated
        // (YAMLRocksAnnotatedInt/Float); otherwise, and for bool/null, they resolve
        // to plain Python values.
        YamlNodeKind::Scalar(value, style) => {
            let resolved = schema.resolve(value, *style, node.tag.as_deref());
            match resolved {
                ResolvedValue::String(s) => annotated_str(
                    py,
                    &s,
                    line,
                    column,
                    config_file,
                    end_line,
                    end_column,
                    style.name(),
                    source_tag,
                    source_target,
                    offset,
                    end_offset,
                ),
                ResolvedValue::Int(i) if annotate_numbers => annotated_number(
                    py,
                    &ANNOTATED_INT,
                    "YAMLRocksAnnotatedInt",
                    i.into_pyobject(py)?.into_any().unbind(),
                    line,
                    column,
                    config_file,
                    end_line,
                    end_column,
                    style.name(),
                    source_tag,
                    source_target,
                    offset,
                    end_offset,
                ),
                ResolvedValue::Float(f) if annotate_numbers => annotated_number(
                    py,
                    &ANNOTATED_FLOAT,
                    "YAMLRocksAnnotatedFloat",
                    PyFloat::new(py, f).into_any().unbind(),
                    line,
                    column,
                    config_file,
                    end_line,
                    end_column,
                    style.name(),
                    source_tag,
                    source_target,
                    offset,
                    end_offset,
                ),
                _ => Ok(node_to_python_cached(py, node, schema, anchors, cache)),
            }
        }
        _ => Ok(node_to_python_cached(py, node, schema, anchors, cache)),
    }?;

    let obj = apply_tag_policy(py, node, obj, tags)?;
    // Record this anchor's object so a later alias yields the same instance.
    if let Some(name) = &node.anchor {
        cache.insert(name.clone(), obj.clone_ref(py));
    }
    Ok(obj)
}

/// Recursively convert an AST node into plain Python objects, applying custom
/// tag handling along the way. `anchors` resolves `*alias` nodes, which share
/// object identity with their anchor (see [`annotate_node`]).
pub fn node_to_python_with_tags(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
) -> PyResult<Py<PyAny>> {
    let mut cache = ObjectCache::new();
    node_to_python_with_tags_cached(py, node, schema, tags, anchors, &mut cache)
}

fn node_to_python_with_tags_cached(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> PyResult<Py<PyAny>> {
    // Grow the native stack on demand so a deeply nested tree cannot overflow a
    // small thread stack; the recursion re-enters here per level. See
    // [`crate::stack`].
    crate::stack::guard(|| {
        node_to_python_with_tags_cached_inner(py, node, schema, tags, anchors, cache)
    })
}

fn node_to_python_with_tags_cached_inner(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    tags: TagPolicy<'_, '_>,
    anchors: &HashMap<String, YamlNode>,
    cache: &mut ObjectCache,
) -> PyResult<Py<PyAny>> {
    if let YamlNodeKind::Alias(name) = &node.kind {
        if let Some(obj) = cache.get(name) {
            return Ok(obj.clone_ref(py));
        }
        return match anchors.get(name) {
            Some(target) => {
                node_to_python_with_tags_cached(py, target, schema, tags, anchors, cache)
            }
            None => Ok(py.None()),
        };
    }

    let obj = match &node.kind {
        YamlNodeKind::Sequence(items) => {
            let list = PyList::empty(py);
            for item in items {
                list.append(node_to_python_with_tags_cached(
                    py, item, schema, tags, anchors, cache,
                )?)?;
            }
            list.into_any().unbind()
        }
        YamlNodeKind::Mapping(pairs) => {
            let dict = PyDict::new(py);
            let has_merge = mapping_has_merge_key(pairs);
            for (key, val) in pairs {
                let py_val =
                    node_to_python_with_tags_cached(py, val, schema, tags, anchors, cache)?;
                if has_merge && is_ast_merge_key(key) {
                    if let Some(preserve) = merge_converted_into(&dict, py_val.bind(py))? {
                        let py_key =
                            node_to_python_with_tags_cached(py, key, schema, tags, anchors, cache)?;
                        if !dict.contains(&py_key)? {
                            dict.set_item(py_key, preserve)?;
                        }
                    }
                    continue;
                }
                let py_key = if key_is_collection(key, anchors) {
                    node_to_python_key(py, key, schema, anchors, cache)
                } else {
                    node_to_python_with_tags_cached(py, key, schema, tags, anchors, cache)?
                };
                dict.set_item(py_key, py_val)?;
            }
            dict.into_any().unbind()
        }
        _ => node_to_python_cached(py, node, schema, anchors, cache),
    };

    let obj = apply_tag_policy(py, node, obj, tags)?;
    if let Some(name) = &node.anchor {
        cache.insert(name.clone(), obj.clone_ref(py));
    }
    Ok(obj)
}

fn apply_tag_policy(
    py: Python<'_>,
    node: &YamlNode,
    obj: Py<PyAny>,
    tags: TagPolicy<'_, '_>,
) -> PyResult<Py<PyAny>> {
    let Some(tag) = node
        .tag
        .as_deref()
        .filter(|tag| crate::decode::is_custom_tag(tag))
    else {
        return Ok(obj);
    };

    resolve_tagged(py, tag, obj, tags)
}
