//! Immutable, bulk snapshots of parsed nodes for host-side transformations.

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyTuple;

use super::ast::{NodeStyle, YamlNode, YamlNodeKind};
use super::value::{is_ast_merge_key, node_to_python_with};
use crate::resolver::Schema;

/// An owned node snapshot. Collections contain child snapshots rather than
/// live cursors, so walking a wide mapping never repeats root/path searches.
/// Scalar values are resolved by the same schema as `to_dict()`. Aliases and
/// merge keys remain explicit, allowing the host to choose whether to expand.
#[pyclass(name = "YAMLRocksTreeNode", module = "yamlrocks", frozen)]
pub struct YAMLRocksTreeNode {
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    value: Py<PyAny>,
    #[pyo3(get)]
    style: &'static str,
    #[pyo3(get)]
    tag: Option<String>,
    #[pyo3(get)]
    anchor: Option<String>,
    #[pyo3(get)]
    comment: Option<String>,
    #[pyo3(get)]
    comment_before: Option<String>,
    #[pyo3(get)]
    comment_after: Option<String>,
    #[pyo3(get)]
    is_merge_key: bool,
}

/// Copy a parsed tree to Python in one visit per node. Container values are
/// tuples of child nodes (sequence) or `(key_node, value_node)` pairs (mapping).
pub fn snapshot(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
) -> PyResult<Py<YAMLRocksTreeNode>> {
    crate::stack::guard(|| {
        let (kind, value) = match &node.kind {
            YamlNodeKind::Sequence(items) => {
                let children = items
                    .iter()
                    .map(|child| snapshot(py, child, schema, resolve_timestamps))
                    .collect::<PyResult<Vec<_>>>()?;
                ("sequence", PyTuple::new(py, children)?.into_any().unbind())
            }
            YamlNodeKind::Mapping(pairs) => {
                let children = pairs
                    .iter()
                    .map(|(key, value)| {
                        Ok((
                            snapshot(py, key, schema, resolve_timestamps)?,
                            snapshot(py, value, schema, resolve_timestamps)?,
                        ))
                    })
                    .collect::<PyResult<Vec<_>>>()?;
                ("mapping", PyTuple::new(py, children)?.into_any().unbind())
            }
            YamlNodeKind::Alias(name) => ("alias", name.into_pyobject(py)?.into_any().unbind()),
            YamlNodeKind::Scalar(..) | YamlNodeKind::Null => (
                "scalar",
                node_to_python_with(py, node, schema, resolve_timestamps, &HashMap::new()),
            ),
        };
        let style = match &node.kind {
            YamlNodeKind::Scalar(_, style) => style.name(),
            YamlNodeKind::Sequence(_) | YamlNodeKind::Mapping(_) => {
                if node.style == NodeStyle::Flow {
                    "flow"
                } else {
                    "block"
                }
            }
            _ => "plain",
        };
        Py::new(
            py,
            YAMLRocksTreeNode {
                kind,
                value,
                style,
                tag: node.tag.clone(),
                anchor: node.anchor.clone(),
                comment: node.comments.inline.clone(),
                comment_before: (!node.comments.head.is_empty()).then(|| {
                    node.comments
                        .head
                        .iter()
                        .map(|comment| comment.text.as_ref())
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                comment_after: (!node.comments.foot.is_empty())
                    .then(|| node.comments.foot.join("\n")),
                is_merge_key: is_ast_merge_key(node),
            },
        )
    })
}
