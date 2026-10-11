//! Immutable, bulk snapshots of parsed nodes for host-side transformations.

use std::collections::HashMap;

use pyo3::prelude::*;
use pyo3::types::PyTuple;

use super::ast::{YamlNode, YamlNodeKind};
use super::document::{
    head_comment_text, head_scope_of_first_key, head_scope_of_item, node_style_name, HeadScope,
};
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
///
/// Every node reports the same metadata a [`YAMLRocksNode`] cursor at that
/// position does. The cursor finds `comment_before` from context (the first
/// key's comments live on its mapping, a sequence item's are split at its dash),
/// so this walk passes that context down instead of re-resolving a path per
/// node, which keeps the export linear.
///
/// [`YAMLRocksNode`]: super::document::YAMLRocksNode
pub fn snapshot(
    py: Python<'_>,
    root: &YamlNode,
    schema: Schema,
    resolve_timestamps: bool,
) -> PyResult<Py<YAMLRocksTreeNode>> {
    let walk = Walk {
        py,
        schema,
        resolve_timestamps,
    };
    walk.node(root, head_comment_text(root, HeadScope::All), false)
}

/// The fixed inputs of one snapshot walk.
struct Walk<'py> {
    py: Python<'py>,
    schema: Schema,
    resolve_timestamps: bool,
}

impl Walk<'_> {
    /// Snapshot `node`, whose `comment_before` the caller has already resolved
    /// from its position. `is_sequence_item` decides where the first key of a
    /// mapping item finds its comments.
    fn node(
        &self,
        node: &YamlNode,
        comment_before: Option<String>,
        is_sequence_item: bool,
    ) -> PyResult<Py<YAMLRocksTreeNode>> {
        crate::stack::guard(|| {
            let py = self.py;
            let (kind, value) = match &node.kind {
                YamlNodeKind::Sequence(items) => {
                    let children = items
                        .iter()
                        .map(|item| {
                            let before = head_comment_text(item, head_scope_of_item(item));
                            self.node(item, before, true)
                        })
                        .collect::<PyResult<Vec<_>>>()?;
                    ("sequence", PyTuple::new(py, children)?.into_any().unbind())
                }
                YamlNodeKind::Mapping(pairs) => {
                    let children = pairs
                        .iter()
                        .enumerate()
                        .map(|(index, (key, value))| {
                            // Nothing sits above a mapping's first key: its
                            // comments are stored on the mapping itself.
                            let entry_before = if index == 0 {
                                head_comment_text(
                                    node,
                                    head_scope_of_first_key(node, is_sequence_item),
                                )
                            } else {
                                head_comment_text(key, HeadScope::All)
                            };
                            let key_before = head_comment_text(key, HeadScope::All);
                            Ok((
                                self.node(key, key_before, false)?,
                                self.node(value, entry_before, false)?,
                            ))
                        })
                        .collect::<PyResult<Vec<_>>>()?;
                    ("mapping", PyTuple::new(py, children)?.into_any().unbind())
                }
                YamlNodeKind::Alias(name) => ("alias", name.into_pyobject(py)?.into_any().unbind()),
                YamlNodeKind::Scalar(..) | YamlNodeKind::Null => (
                    "scalar",
                    node_to_python_with(
                        py,
                        node,
                        self.schema,
                        self.resolve_timestamps,
                        &HashMap::new(),
                    ),
                ),
            };
            Py::new(
                py,
                YAMLRocksTreeNode {
                    kind,
                    value,
                    style: node_style_name(node),
                    tag: node.tag.clone(),
                    anchor: node.anchor.clone(),
                    comment: node.comments.inline.clone(),
                    comment_before,
                    comment_after: (!node.comments.foot.is_empty())
                        .then(|| node.comments.foot.join("\n")),
                    is_merge_key: is_ast_merge_key(node),
                },
            )
        })
    }
}
