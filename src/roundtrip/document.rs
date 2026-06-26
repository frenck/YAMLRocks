//! Python-facing wrappers for the round-trip AST.
//!
//! [`YAMLRocksDocument`] is the root object returned by `loads(..., OPT_ROUND_TRIP)`. It
//! exposes mapping/sequence access that reads through to the live AST so that
//! edits, including deep edits like `doc["a"][0]["b"] = 1`, are retained and
//! reflected when the document is re-emitted or written back to include files.
//!
//! Deep access returns a [`YAMLRocksDocumentView`]: a lightweight proxy holding a handle
//! to the root [`YAMLRocksDocument`] plus the path to a nested node. Every read or write
//! re-navigates from the root, so views never hold stale references.

use std::collections::HashMap;
use std::path::PathBuf;

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

use super::anchors::{
    alias_target_path, build_anchor_map, collect_alias_paths, collect_anchor_paths,
    collect_anchor_refs, detached_clone, find_anchor_path, path_precedes,
};
use super::ast::{NodeStyle, YamlNode, YamlNodeKind};
use super::emit;
use super::emit::{emit_roundtrip_all_with, emit_roundtrip_with};
use super::value::{node_to_python_with, python_to_node};
use crate::encode::NullStyle;
use crate::resolver::Schema;

/// A single step along a path into the AST: a mapping value (by key), a sequence
/// element (by index), or a mapping *key node* itself.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PathSeg {
    /// The value of the mapping entry whose key resolves to this string.
    Key(String),
    /// The element at this index of a sequence.
    Index(usize),
    /// The key node of the mapping entry whose key resolves to this string.
    /// Only the anchor/alias traversal produces this (an anchor may sit on a
    /// key, `&a foo: bar`); indexed access from Python never does, so a key
    /// node is reachable for discovery but not addressed by `doc["foo"]`.
    KeyNode(String),
}

fn seg_from_key(key: &Bound<'_, PyAny>) -> PyResult<PathSeg> {
    if let Ok(idx) = key.extract::<usize>() {
        return Ok(PathSeg::Index(idx));
    }
    let s: String = key
        .extract()
        .map_err(|_| pyo3::exceptions::PyTypeError::new_err("mapping keys must be str or int"))?;
    Ok(PathSeg::Key(s))
}

/// A structure-preserving YAML document returned by `loads(..., OPT_ROUND_TRIP)`.
///
/// Holds the rich AST (comments, styles, anchors, include boundaries) and, when
/// loaded with includes, the `file_id` → source-path map used for write-back.
#[pyclass(name = "YAMLRocksDocument", skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct YAMLRocksDocument {
    pub nodes: Vec<YamlNode>,
    pub source: Option<String>,
    /// Source files indexed by `file_id`; empty when includes are not used.
    pub file_map: Vec<PathBuf>,
    /// Original source text of each included file, indexed by `file_id`
    /// (parallel to `file_map`). Lets an unmodified included file re-emit
    /// byte-for-byte on write-back; empty when includes are not used.
    pub file_sources: Vec<Option<String>>,
    /// The path this document was loaded from, enabling `save()` with no
    /// argument to write back to the same file.
    pub origin_path: Option<String>,
    /// How a synthetic (edited-in) null is rendered on re-emission. Captured from
    /// the load-time option flags; loaded nulls are unaffected.
    pub null_style: NullStyle,
    /// Whether a freshly assigned string that needs quoting uses double quotes
    /// (the default) or single. Captured from the load-time option flags; loaded
    /// strings keep their original quoting.
    pub double_quotes: bool,
    /// Whether this document was loaded with `OPT_UPGRADE_1_1`. When set,
    /// re-emission stamps a `%YAML 1.2` directive so the written-back file
    /// declares its version and is read back as 1.2 (not re-upgraded).
    pub upgraded: bool,
    /// The schema the document was loaded under. A freshly assigned scalar is
    /// quoted by this schema's rules so an edit stays the type it reads as (e.g.
    /// a string `y` keeps quotes under strict 1.1); loaded scalars keep their
    /// original text. Defaults to 1.2.
    pub schema: Schema,
}

/// Free the document's AST without unbounded native recursion. The derived drop
/// of `Vec<YamlNode>` recurses once per level of nesting, so a deeply nested
/// document (bounded by the composer's `MAX_DEPTH`) could overflow the stack when
/// the Python object is freed at GC time — on whatever thread the collector runs.
/// Dismantling each tree iteratively keeps teardown shallow. See [`crate::stack`].
impl Drop for YAMLRocksDocument {
    fn drop(&mut self) {
        for node in std::mem::take(&mut self.nodes) {
            crate::stack::drop_node_tree(node);
        }
    }
}

#[pymethods]
impl YAMLRocksDocument {
    fn __repr__(&self) -> String {
        format!("YAMLRocksDocument(documents={})", self.nodes.len())
    }

    fn __len__(&self) -> usize {
        if self.nodes.len() == 1 {
            container_len(&self.nodes[0])
        } else {
            self.nodes.len()
        }
    }

    fn __getitem__(slf: &Bound<'_, Self>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.borrow();
        let anchors = build_anchor_map(&this.nodes);
        let schema = this.schema;
        if this.nodes.len() == 1 {
            access_child(py, slf, &[], &this.nodes[0], key, &anchors, schema)
        } else {
            let idx: usize = key.extract().map_err(|_| {
                pyo3::exceptions::PyKeyError::new_err(
                    "multi-document access requires an integer index",
                )
            })?;
            let node = this
                .nodes
                .get(idx)
                .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("index out of range"))?;
            wrap_node(py, slf, vec![PathSeg::Index(idx)], node, &anchors, schema)
        }
    }

    fn __setitem__(
        &mut self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let double_quotes = self.double_quotes;
        let schema = self.schema;
        if self.nodes.len() == 1 {
            let anchors = build_anchor_map(&self.nodes);
            set_child(
                py,
                &mut self.nodes[0],
                key,
                value,
                double_quotes,
                schema,
                &anchors,
            )
        } else {
            let idx: usize = key.extract()?;
            let node = self
                .nodes
                .get_mut(idx)
                .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("index out of range"))?;
            *node = python_to_node(py, value, double_quotes, schema)?;
            node.mark_modified();
            Ok(())
        }
    }

    fn __delitem__(&mut self, key: &Bound<'_, PyAny>) -> PyResult<()> {
        if self.nodes.len() == 1 {
            del_child(&mut self.nodes[0], key)
        } else {
            let idx: usize = key.extract()?;
            if idx >= self.nodes.len() {
                return Err(pyo3::exceptions::PyIndexError::new_err(
                    "index out of range",
                ));
            }
            self.nodes.remove(idx);
            // The cached source still contains the removed document; drop it so
            // the stream re-emits from the AST instead of replaying verbatim.
            self.source = None;
            Ok(())
        }
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> bool {
        self.nodes.len() == 1 && node_contains(&self.nodes[0], key)
    }

    fn get(
        slf: &Bound<'_, Self>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        match Self::__getitem__(slf, key) {
            Ok(val) => Ok(val),
            Err(_) => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.nodes.len() == 1 {
            let anchors = build_anchor_map(&self.nodes);
            node_keys(py, &self.nodes[0], &anchors, self.schema)
        } else {
            Err(pyo3::exceptions::PyTypeError::new_err(
                "keys() is only available on a single mapping document",
            ))
        }
    }

    /// Serialize the document back to YAML bytes, preserving structure.
    ///
    /// If the document has not been modified since loading, the original source
    /// is returned **byte-for-byte**. Once any value is changed, the document is
    /// re-emitted from the AST (preserving comments, styles, and include
    /// directives). When loaded with includes, directives are restored.
    pub fn to_yaml(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        Ok(PyBytes::new(py, &self.root_bytes()).into_any().unbind())
    }

    /// Record the path this document was loaded from (used by `save`).
    fn set_origin(&mut self, path: String) {
        self.origin_path = Some(path);
    }

    /// The path this document was loaded from, if any.
    #[getter]
    fn origin(&self) -> Option<String> {
        self.origin_path.clone()
    }

    /// Write the document back to disk, saving **only the changed files**.
    ///
    /// * Included files whose content was modified are rewritten in place;
    ///   untouched includes are left exactly as they were.
    /// * The root file is rewritten only if root-level content changed, or when
    ///   an explicit `path` is given (a save-as).
    ///
    /// With no `path`, the document must have been loaded from disk (so it knows
    /// its origin). Returns the list of paths that were written.
    #[pyo3(signature = (path=None))]
    fn save(&self, py: Python<'_>, path: Option<String>) -> PyResult<Py<PyAny>> {
        use std::path::PathBuf;

        let mut written: Vec<String> = Vec::new();

        // 1. Included files whose own content changed.
        for (file_id, bytes) in emit::collect_changed_include_changes(&self.nodes) {
            if let Some(target) = self.file_map.get(file_id as usize) {
                write_file(target, &bytes)?;
                written.push(target.to_string_lossy().into_owned());
            }
        }

        // 2. The root file: written when an explicit path is given, or when
        //    root-level content (outside any include) changed.
        let explicit = path.is_some();
        let root_changed = explicit || self.nodes.iter().any(emit::local_modified);
        if root_changed {
            let target = path.or_else(|| self.origin_path.clone()).ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "save() needs a path: this document was not loaded from a file",
                )
            })?;
            let target = PathBuf::from(target);
            write_file(&target, &self.root_bytes())?;
            written.push(target.to_string_lossy().into_owned());
        }

        Ok(PyList::new(py, written)?.into_any().unbind())
    }

    /// The 1-based source range `(start_line, start_col, end_line, end_col)` of
    /// the document's root node.
    fn range(&self, py: Python<'_>) -> Py<PyAny> {
        match self.nodes.first() {
            Some(node) => node_range(py, node),
            None => py.None(),
        }
    }

    /// The root [`YAMLRocksNode`] cursor: a metadata-bearing handle from which any node
    /// in the tree can be reached by indexing (`doc.node["server"]["port"]`).
    /// Unlike item access, indexing a `YAMLRocksNode` always yields another `YAMLRocksNode`, so
    /// comments, source location, style, anchor, and tag stay reachable down to
    /// individual scalars.
    #[getter]
    fn node(slf: &Bound<'_, Self>) -> PyResult<Py<YAMLRocksNode>> {
        let py = slf.py();
        Py::new(
            py,
            YAMLRocksNode {
                root: slf.clone().unbind(),
                path: Vec::new(),
            },
        )
    }

    /// A mapping of every anchor name (`&name`) to the [`YAMLRocksNode`] that defines it,
    /// for discovering and navigating the document's anchors. Use a definition's
    /// `YAMLRocksNode.aliases` to find the `*name` references that point back at it.
    #[getter]
    fn anchors(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let mut found: Vec<(String, Vec<PathSeg>)> = Vec::new();
        if let Some(root) = slf.borrow().nodes.first() {
            collect_anchor_paths(root, &mut Vec::new(), &mut found);
        }
        let dict = PyDict::new(py);
        for (name, path) in found {
            let node = Py::new(
                py,
                YAMLRocksNode {
                    root: slf.clone().unbind(),
                    path,
                },
            )?;
            dict.set_item(name, node)?;
        }
        Ok(dict.into_any().unbind())
    }

    /// Return the document as plain Python objects (a resolved snapshot).
    fn to_dict(&self, py: Python<'_>) -> Py<PyAny> {
        let anchors = build_anchor_map(&self.nodes);
        if self.nodes.len() == 1 {
            node_to_python_with(py, &self.nodes[0], self.schema, &anchors)
        } else {
            let list = PyList::empty(py);
            for node in &self.nodes {
                let _ = list.append(node_to_python_with(py, node, self.schema, &anchors));
            }
            list.into_any().unbind()
        }
    }

    /// Walk every scalar leaf, yielding ``(path, value)`` pairs where ``path``
    /// is a tuple of mapping keys and sequence indices. Combine with item
    /// access to perform bulk edits:
    ///
    /// ```python
    /// for path, value in doc.walk():
    ///     if value == "TODO":
    ///         node = doc
    ///         for key in path[:-1]:
    ///             node = node[key]
    ///         node[path[-1]] = "done"
    /// ```
    fn walk(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let mut out: Vec<(Vec<Py<PyAny>>, Py<PyAny>)> = Vec::new();
        let anchors = build_anchor_map(&self.nodes);
        if let Some(root) = self.nodes.first() {
            collect_leaves(py, root, &mut Vec::new(), &mut out, &anchors, self.schema)?;
        }
        leaves_to_list(py, out)
    }
}

impl YAMLRocksDocument {
    pub fn new(nodes: Vec<YamlNode>) -> Self {
        Self {
            nodes,
            source: None,
            file_map: Vec::new(),
            file_sources: Vec::new(),
            origin_path: None,
            null_style: NullStyle::Empty,
            double_quotes: true,
            upgraded: false,
            schema: Schema::Yaml12,
        }
    }

    /// Construct a document that tracks the source files of resolved includes,
    /// along with each file's original text for byte-exact write-back.
    pub fn with_file_map(
        nodes: Vec<YamlNode>,
        file_map: Vec<PathBuf>,
        file_sources: Vec<Option<String>>,
    ) -> Self {
        Self {
            nodes,
            source: None,
            file_map,
            file_sources,
            origin_path: None,
            null_style: NullStyle::Empty,
            double_quotes: true,
            upgraded: false,
            schema: Schema::Yaml12,
        }
    }

    pub fn with_source(mut self, source: String) -> Self {
        self.source = Some(source);
        self
    }

    /// Set the null style applied to synthetic (edited-in) nulls on re-emission.
    pub fn with_null_style(mut self, null_style: NullStyle) -> Self {
        self.null_style = null_style;
        self
    }

    /// Mark the document as loaded with `OPT_UPGRADE_1_1` so re-emission stamps a
    /// `%YAML 1.2` directive.
    pub fn with_upgraded(mut self, upgraded: bool) -> Self {
        self.upgraded = upgraded;
        self
    }

    /// Set whether freshly assigned strings that need quoting use double quotes.
    pub fn with_double_quotes(mut self, double_quotes: bool) -> Self {
        self.double_quotes = double_quotes;
        self
    }

    /// Set the schema whose rules govern quoting of freshly assigned scalars, so
    /// an edit to a document loaded under YAML 1.1 stays 1.1-safe.
    pub fn with_schema(mut self, schema: Schema) -> Self {
        self.schema = schema;
        self
    }

    /// The serialized bytes of the root document (verbatim when unmodified),
    /// stamped with a `%YAML 1.2` directive when the document was upgraded.
    fn root_bytes(&self) -> Vec<u8> {
        let mut bytes = match &self.source {
            Some(source) if !nodes_modified(&self.nodes) => source.as_bytes().to_vec(),
            _ => self.emit_nodes(),
        };
        if self.upgraded {
            stamp_yaml_1_2(&mut bytes);
        }
        bytes
    }

    /// Re-emit the document from the AST (used when modified or sourceless).
    fn emit_nodes(&self) -> Vec<u8> {
        if self.nodes.len() == 1 {
            emit_roundtrip_with(&self.nodes[0], self.null_style)
        } else {
            emit_roundtrip_all_with(&self.nodes, self.null_style)
        }
    }
}

/// Prepend a `%YAML 1.2` version directive to `buf` so an upgraded document
/// declares its version (and is read back as 1.2, not re-upgraded). A leading
/// byte order mark stays first; an existing `---` document-start marker is
/// reused, otherwise one is added. Any `%YAML` directive among the leading
/// directives is dropped (the canonical 1.2 declaration replaces it); a `%TAG`
/// directive is kept in place, with `%YAML 1.2` inserted ahead of it.
fn stamp_yaml_1_2(buf: &mut Vec<u8>) {
    const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
    let at = if buf.starts_with(&BOM) { BOM.len() } else { 0 };
    // Walk the leading directive lines. A `%YAML` line is dropped (the canonical
    // 1.2 declaration below replaces it; this also keeps re-stamping idempotent);
    // any other directive (a `%TAG` handle) is kept and skipped over so the
    // marker is found past it, not before it. Inserting `%YAML 1.2\n---\n` ahead
    // of a `%TAG` would strand the handle after a document start and fail to
    // reload.
    let mut cursor = at;
    loop {
        let rest = &buf[cursor..];
        let line_len = rest.iter().position(|&b| b == b'\n').map(|n| n + 1);
        if rest.starts_with(b"%YAML") {
            match line_len {
                Some(len) => {
                    buf.drain(cursor..cursor + len);
                }
                None => buf.truncate(cursor),
            }
        } else if rest.starts_with(b"%") {
            match line_len {
                Some(len) => cursor += len,
                None => break,
            }
        } else {
            break;
        }
    }
    let body = &buf[cursor..];
    let has_marker = body.starts_with(b"---")
        && body
            .get(3)
            .map_or(true, |&b| matches!(b, b'\n' | b'\r' | b' '));
    let mut stamp: Vec<u8> = b"%YAML 1.2\n".to_vec();
    if !has_marker {
        // No directives precede us here (a directive always carries its own
        // `---`), so the marker can sit right after the version line.
        stamp.extend_from_slice(b"---\n");
    }
    buf.splice(at..at, stamp);
}

/// A live proxy onto a nested node of a [`YAMLRocksDocument`], addressed by path.
#[pyclass(name = "YAMLRocksDocumentView")]
pub struct YAMLRocksDocumentView {
    root: Py<YAMLRocksDocument>,
    path: Vec<PathSeg>,
}

#[pymethods]
impl YAMLRocksDocumentView {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        Ok(container_len(node))
    }

    fn __getitem__(slf: &Bound<'_, Self>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = slf.borrow();
        let root = this.root.clone_ref(py);
        let doc = root.borrow(py);
        let node = resolve_path(&doc.nodes, &this.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        let anchors = build_anchor_map(&doc.nodes);
        let root_bound = root.bind(py);
        access_child(py, root_bound, &this.path, node, key, &anchors, doc.schema)
    }

    fn __setitem__(
        &mut self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let double_quotes = doc.double_quotes;
        let schema = doc.schema;
        let anchors = build_anchor_map(&doc.nodes);
        let node = resolve_path_mut(&mut doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        set_child(py, node, key, value, double_quotes, schema, &anchors)
    }

    fn __delitem__(&mut self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let node = resolve_path_mut(&mut doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        del_child(node, key)
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        Ok(node_contains(node, key))
    }

    fn get(
        slf: &Bound<'_, Self>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        match Self::__getitem__(slf, key) {
            Ok(val) => Ok(val),
            Err(_) => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        let anchors = build_anchor_map(&doc.nodes);
        node_keys(py, node, &anchors, doc.schema)
    }

    /// The [`YAMLRocksNode`] cursor for this view's node: a metadata-bearing handle
    /// exposing comments, source location, style, anchor, and tag.
    #[getter]
    fn node(&self, py: Python<'_>) -> PyResult<Py<YAMLRocksNode>> {
        Py::new(
            py,
            YAMLRocksNode {
                root: self.root.clone_ref(py),
                path: self.path.clone(),
            },
        )
    }

    /// The resolved plain-Python value of this node (dict/list/scalar).
    fn unwrap(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        let anchors = build_anchor_map(&doc.nodes);
        Ok(node_to_python_with(py, node, doc.schema, &anchors))
    }

    /// Alias for [`unwrap`](Self::unwrap): the resolved plain-Python value.
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.unwrap(py)
    }

    /// The node's 1-based source range `(start_line, start_col, end_line,
    /// end_col)`.
    fn range(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        Ok(node_range(py, node))
    }

    /// Walk every scalar leaf below this view, yielding ``(path, value)`` pairs
    /// relative to this node.
    fn walk(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        let anchors = build_anchor_map(&doc.nodes);
        let mut out: Vec<(Vec<Py<PyAny>>, Py<PyAny>)> = Vec::new();
        collect_leaves(py, node, &mut Vec::new(), &mut out, &anchors, doc.schema)?;
        leaves_to_list(py, out)
    }

    /// Serialize just this nested node back to YAML bytes.
    fn to_yaml(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path)
            .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err("stale view"))?;
        Ok(PyBytes::new(py, &emit_roundtrip_with(node, doc.null_style))
            .into_any()
            .unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let doc = self.root.borrow(py);
        match resolve_path(&doc.nodes, &self.path) {
            Some(node) => format!("YAMLRocksDocumentView({})", node_kind_name(node)),
            None => "YAMLRocksDocumentView(<stale>)".to_owned(),
        }
    }
}

/// A metadata-bearing handle onto a single node of a [`YAMLRocksDocument`].
///
/// Where [`YAMLRocksDocumentView`] targets containers and resolves scalars to plain
/// Python values, a `YAMLRocksNode` is returned for *every* node (scalars included), so
/// comments, source location, scalar/collection style, anchor, and tag are
/// always reachable without knowing the node's path in advance. Obtain the root
/// cursor with `YAMLRocksDocument.node`, then index into it (`doc.node["server"]["port"]`)
/// to address any node in the tree; each index step returns another `YAMLRocksNode`.
///
/// Comments follow YAML's own placement. `comment_before` is the standalone
/// comment above a node (above its key, for a mapping value) and `comment` is
/// the inline comment trailing the value on the same line. Both read and write
/// the bare comment text, without the leading `#`.
#[pyclass(name = "YAMLRocksNode")]
pub struct YAMLRocksNode {
    root: Py<YAMLRocksDocument>,
    path: Vec<PathSeg>,
}

#[pymethods]
impl YAMLRocksNode {
    /// The resolved plain-Python value of this node (scalar, dict, or list).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let anchors = build_anchor_map(&doc.nodes);
        Ok(node_to_python_with(py, node, doc.schema, &anchors))
    }

    /// Replace this node's value, preserving its comments, anchor, and tag.
    #[setter]
    fn set_value(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let double_quotes = doc.double_quotes;
        let schema = doc.schema;
        let node = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        let mut new_val = python_to_node(py, value, double_quotes, schema)?;
        new_val.comments.head = std::mem::take(&mut node.comments.head);
        new_val.comments.inline = node.comments.inline.take();
        // Keep the alignment padding around the value so an edit preserves the
        // author's layout: the gap between the key's `:` and the value, and the
        // run of spaces before an inline `#`.
        new_val.comments.value_pad = node.comments.value_pad;
        new_val.comments.inline_spaces = node.comments.inline_spaces;
        new_val.comments.foot = std::mem::take(&mut node.comments.foot);
        new_val.anchor = node.anchor.take();
        new_val.tag = node.tag.take();
        new_val.mark_modified();
        *node = new_val;
        Ok(())
    }

    /// The node's 1-based source line.
    #[getter]
    fn line(&self, py: Python<'_>) -> PyResult<u32> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.span.line + 1)
    }

    /// The node's 1-based source column.
    #[getter]
    fn column(&self, py: Python<'_>) -> PyResult<u32> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.span.column + 1)
    }

    /// The 0-based byte offset of the node's first source character.
    ///
    /// Together with [`end_offset`](Self::end_offset) this gives the node's exact
    /// source extent, so the original bytes can be sliced directly
    /// (`source[node.offset : node.end_offset]`) — something line/column alone
    /// cannot do without re-deriving offsets.
    #[getter]
    fn offset(&self, py: Python<'_>) -> PyResult<usize> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.span.offset)
    }

    /// The 0-based byte offset just past the node's last source character.
    ///
    /// Exact (not the approximation [`range`](Self::range) derives from the
    /// post-scan text): the scanner records each scalar's true source end, and a
    /// collection spans to the furthest end of any child.
    #[getter]
    fn end_offset(&self, py: Python<'_>) -> PyResult<usize> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.end_offset)
    }

    /// The path of the source file this node came from, or `None` when the
    /// document was not loaded with includes (so there is a single source).
    #[getter]
    fn file(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(doc
            .file_map
            .get(node.span.file_id as usize)
            .map(|p| p.to_string_lossy().into_owned()))
    }

    /// The inline comment trailing this node's value, or `None`. The returned
    /// text has no leading `#` or surrounding whitespace.
    #[getter]
    fn comment(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.comments.inline.clone())
    }

    /// Set or clear the inline comment. Pass the bare text (no `#`) or `None`.
    #[setter]
    fn set_comment(&self, py: Python<'_>, text: Option<String>) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let node = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        node.comments.inline = text;
        // A freshly set comment uses default spacing (one space), not whatever
        // alignment padding the previous comment on this node happened to have.
        node.comments.inline_spaces = 0;
        node.mark_modified();
        Ok(())
    }

    /// The standalone comment line(s) above this node, joined by newlines, or
    /// `None`. For a mapping value this is the comment above its key.
    #[getter]
    fn comment_before(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_head(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(join_comment_lines(&node.comments.head))
    }

    /// Set or clear the standalone comment above this node. A multi-line string
    /// becomes one comment line per line; `None` removes it.
    #[setter]
    fn set_comment_before(&self, py: Python<'_>, text: Option<String>) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let node = resolve_head_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        node.comments.head = split_comment_lines(text);
        node.mark_modified();
        Ok(())
    }

    /// The presentation style: a scalar's quoting (`plain`, `single`, `double`,
    /// `literal`, `folded`), a collection's layout (`block`, `flow`), `alias`,
    /// or `null`.
    #[getter]
    fn style(&self, py: Python<'_>) -> PyResult<&'static str> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node_style_name(node))
    }

    /// The node's anchor name (`&name`), or `None`.
    #[getter]
    fn anchor(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.anchor.clone())
    }

    /// Set or clear this node's anchor (`&name`). The name must be unique: it is
    /// an error to assign a name already defined on another node, so the document
    /// can never emit two `&name` definitions. Pass `None` to remove the anchor.
    #[setter]
    fn set_anchor(&self, py: Python<'_>, name: Option<String>) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        if let Some(ref name) = name {
            if name.is_empty() {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "anchor name cannot be empty",
                ));
            }
            if let Some(existing) = find_anchor_path(&doc.nodes, name) {
                if existing != self.path {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "anchor name '{name}' is already used by another node"
                    )));
                }
            }
        }
        let node = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        node.anchor = name;
        node.mark_modified();
        Ok(())
    }

    /// Turn this node into an alias (`*name`) of an existing anchor, replacing
    /// its current value. The anchor must already be defined **earlier in the
    /// document** (YAML resolves an alias to a prior anchor), so this can never
    /// produce a dangling or forward reference. Use it after marking the target
    /// with `anchor = "name"`.
    fn make_alias(&self, py: Python<'_>, name: String) -> PyResult<()> {
        let mut doc = self.root.borrow_mut(py);
        let anchor_path = find_anchor_path(&doc.nodes, &name).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "no anchor '&{name}' is defined in this document"
            ))
        })?;
        // The alias cannot reference its own ancestor (or itself): the anchored
        // node contains the alias, so expanding it would re-insert that container
        // forever. `path_precedes` treats an ancestor as preceding its
        // descendant, so this case must be rejected explicitly.
        if self.path.starts_with(&anchor_path) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "anchor '&{name}' contains the alias that would reference it, which is a cycle"
            )));
        }
        if !path_precedes(&doc.nodes, &anchor_path, &self.path) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "anchor '&{name}' must be defined before the alias that references it"
            )));
        }
        let node = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        node.kind = YamlNodeKind::Alias(name);
        node.anchor = None; // a node cannot be both an alias and an anchor
        node.mark_modified();
        Ok(())
    }

    /// The node's explicit tag (e.g. `!!str`, `!custom`), or `None`.
    #[getter]
    fn tag(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.tag.clone())
    }

    /// The config/custom tag that produced this node (`!secret`, `!env_var`,
    /// `!include*`, or a custom `!mytag`), or `None`. Provenance for a resolved
    /// node, distinct from `tag` (the node's own YAML type tag).
    #[getter]
    fn source_tag(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag().map(str::to_owned))
    }

    /// The directive argument that produced this node (the secret name, include
    /// path, or env-var spec), or `None`. With `source_tag`, reconstructs the
    /// directive (e.g. `!secret db_password`).
    #[getter]
    fn source_target(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_target().map(str::to_owned))
    }

    /// Whether this node's value was produced by a `!secret` directive.
    #[getter]
    fn is_secret(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag() == Some("!secret"))
    }

    /// Whether this node's value was produced by an `!env_var` directive.
    #[getter]
    fn is_env_var(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag() == Some("!env_var"))
    }

    /// Whether this node's value was produced by any `!include` directive.
    #[getter]
    fn is_include(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(crate::roundtrip::ast::is_include_tag(node.source_tag()))
    }

    /// Whether this node is an alias (`*name`).
    #[getter]
    fn is_alias(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(matches!(node.kind, YamlNodeKind::Alias(_)))
    }

    /// For an alias, the `YAMLRocksNode` of the anchor definition it points at; `None`
    /// for any non-alias node (or an alias whose anchor is undefined).
    #[getter]
    fn target(&self, py: Python<'_>) -> PyResult<Option<Py<YAMLRocksNode>>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        if let YamlNodeKind::Alias(name) = &node.kind {
            if let Some(path) = find_anchor_path(&doc.nodes, name) {
                return Ok(Some(Py::new(
                    py,
                    YAMLRocksNode {
                        root: self.root.clone_ref(py),
                        path,
                    },
                )?));
            }
        }
        Ok(None)
    }

    /// If this node defines an anchor (`&name`), the alias `YAMLRocksNode`s that
    /// reference it, in document order. Empty for a node with no anchor.
    #[getter]
    fn aliases(&self, py: Python<'_>) -> PyResult<Vec<Py<YAMLRocksNode>>> {
        let doc = self.root.borrow(py);
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let mut paths: Vec<Vec<PathSeg>> = Vec::new();
        if let Some(name) = node.anchor.clone() {
            if let Some(root) = doc.nodes.first() {
                collect_alias_paths(root, &name, &mut Vec::new(), &mut paths);
            }
        }
        paths
            .into_iter()
            .map(|path| {
                Py::new(
                    py,
                    YAMLRocksNode {
                        root: self.root.clone_ref(py),
                        path,
                    },
                )
            })
            .collect()
    }

    /// Replace an alias (`*name`) with an independent deep copy of the anchor it
    /// points at, returning the new `YAMLRocksNode`. The copy carries the anchored node's
    /// styles and comments but no anchor of its own, and any aliases inside it
    /// are expanded, so editing it no longer affects the original. Raises if
    /// this node is not an alias.
    fn detach(slf: &Bound<'_, Self>) -> PyResult<Py<YAMLRocksNode>> {
        let py = slf.py();
        let this = slf.borrow();
        let mut doc = this.root.borrow_mut(py);
        let name = match &resolve_path(&doc.nodes, &this.path)
            .ok_or_else(stale_node)?
            .kind
        {
            YamlNodeKind::Alias(name) => name.clone(),
            _ => {
                return Err(pyo3::exceptions::PyTypeError::new_err(
                    "detach() is only valid on an alias node",
                ));
            }
        };
        let mut refs: HashMap<String, &YamlNode> = HashMap::new();
        for node in &doc.nodes {
            collect_anchor_refs(node, &mut refs);
        }
        let target = refs.get(&name).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("alias *{name} has no anchor to detach"))
        })?;
        let mut clone = detached_clone(target, &refs);
        clone.mark_modified();
        let slot = resolve_path_mut(&mut doc.nodes, &this.path).ok_or_else(stale_node)?;
        *slot = clone;
        Py::new(
            py,
            YAMLRocksNode {
                root: this.root.clone_ref(py),
                path: this.path.clone(),
            },
        )
    }

    /// Index into a child by mapping key or sequence index, returning its
    /// `YAMLRocksNode`. Indexing an alias transparently follows it to the anchor it
    /// points at, so the child is a live handle into the shared definition.
    /// Scalars have no children, so indexing one raises.
    fn __getitem__(slf: &Bound<'_, Self>, key: &Bound<'_, PyAny>) -> PyResult<Py<YAMLRocksNode>> {
        let py = slf.py();
        let this = slf.borrow();
        let doc = this.root.borrow(py);
        let base = alias_target_path(&doc.nodes, &this.path).unwrap_or_else(|| this.path.clone());
        let node = resolve_path(&doc.nodes, &base).ok_or_else(stale_node)?;
        let seg = seg_from_key(key)?;
        child_ref(node, &seg).ok_or_else(|| key_error(node, key))?;
        let mut path = base;
        path.push(seg);
        Py::new(
            py,
            YAMLRocksNode {
                root: this.root.clone_ref(py),
                path,
            },
        )
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let doc = self.root.borrow(py);
        let base = alias_target_path(&doc.nodes, &self.path).unwrap_or_else(|| self.path.clone());
        let node = resolve_path(&doc.nodes, &base).ok_or_else(stale_node)?;
        Ok(node_contains(node, key))
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let doc = self.root.borrow(py);
        match resolve_path(&doc.nodes, &self.path) {
            Some(node) => format!("YAMLRocksNode({})", node_kind_name(node)),
            None => "YAMLRocksNode(<stale>)".to_owned(),
        }
    }
}

/// Error for a `YAMLRocksNode`/`YAMLRocksDocumentView` whose target no longer exists.
fn stale_node() -> PyErr {
    pyo3::exceptions::PyKeyError::new_err("stale node")
}

/// The presentation-style name reported by `YAMLRocksNode.style`.
fn node_style_name(node: &YamlNode) -> &'static str {
    match &node.kind {
        YamlNodeKind::Scalar(_, style) => style.name(),
        YamlNodeKind::Mapping(_) | YamlNodeKind::Sequence(_) => match node.style {
            NodeStyle::Block => "block",
            NodeStyle::Flow => "flow",
        },
        YamlNodeKind::Alias(_) => "alias",
        YamlNodeKind::Null => "null",
    }
}

/// Join stored comment lines (text without `#`) for return to Python.
fn join_comment_lines(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Split a user-supplied comment string into stored lines (one per line).
fn split_comment_lines(text: Option<String>) -> Vec<String> {
    match text {
        None => Vec::new(),
        Some(s) => s.split('\n').map(|line| line.to_owned()).collect(),
    }
}

// -- Path navigation --

/// Resolve the node that owns the *head* (before) comment for `path`.
///
/// The emitter renders a head comment above a node, but where it is *stored*
/// depends on position: for a mapping value the comment above its key normally
/// lives on the key node, except the very first key of a mapping, whose leading
/// comment the composer attaches to the enclosing mapping (which is rendered
/// just above that first key). This returns whichever node actually holds it, so
/// `comment_before` reads and writes the same place the emitter renders.
fn resolve_head<'a>(roots: &'a [YamlNode], path: &[PathSeg]) -> Option<&'a YamlNode> {
    match path.split_last() {
        None => roots.first(),
        Some((PathSeg::Key(k), parent)) => {
            let parent_node = resolve_path(roots, parent)?;
            match &parent_node.kind {
                YamlNodeKind::Mapping(pairs) if is_first_key(pairs, k) => Some(parent_node),
                YamlNodeKind::Mapping(pairs) => pairs
                    .iter()
                    .find(|(key, _)| scalar_eq(key, k))
                    .map(|(key, _)| key),
                _ => None,
            }
        }
        // A sequence element or a key node holds its own head comment.
        Some((PathSeg::Index(_) | PathSeg::KeyNode(_), _)) => resolve_path(roots, path),
    }
}

/// Mutable counterpart to [`resolve_head`].
fn resolve_head_mut<'a>(roots: &'a mut [YamlNode], path: &[PathSeg]) -> Option<&'a mut YamlNode> {
    match path.split_last() {
        None => roots.first_mut(),
        Some((PathSeg::Key(k), parent)) => {
            let parent_node = resolve_path_mut(roots, parent)?;
            let first =
                matches!(&parent_node.kind, YamlNodeKind::Mapping(pairs) if is_first_key(pairs, k));
            if first {
                Some(parent_node)
            } else if let YamlNodeKind::Mapping(pairs) = &mut parent_node.kind {
                pairs
                    .iter_mut()
                    .find(|(key, _)| scalar_eq(key, k))
                    .map(|(key, _)| key)
            } else {
                None
            }
        }
        Some((PathSeg::Index(_) | PathSeg::KeyNode(_), _)) => resolve_path_mut(roots, path),
    }
}

/// Whether `k` names the first pair of `pairs`.
fn is_first_key(pairs: &[(YamlNode, YamlNode)], k: &str) -> bool {
    pairs.first().is_some_and(|(key, _)| scalar_eq(key, k))
}

pub(crate) fn resolve_path<'a>(roots: &'a [YamlNode], path: &[PathSeg]) -> Option<&'a YamlNode> {
    let mut node = roots.first()?;
    for seg in path {
        node = child_ref(node, seg)?;
    }
    Some(node)
}

fn resolve_path_mut<'a>(roots: &'a mut [YamlNode], path: &[PathSeg]) -> Option<&'a mut YamlNode> {
    let mut node = roots.get_mut(0)?;
    for seg in path {
        node = child_ref_mut(node, seg)?;
    }
    Some(node)
}

pub(crate) fn child_ref<'a>(node: &'a YamlNode, seg: &PathSeg) -> Option<&'a YamlNode> {
    match (&node.kind, seg) {
        (YamlNodeKind::Mapping(pairs), PathSeg::Key(k)) => pairs
            .iter()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(_, v)| v),
        (YamlNodeKind::Mapping(pairs), PathSeg::KeyNode(k)) => pairs
            .iter()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(key, _)| key),
        (YamlNodeKind::Sequence(items), PathSeg::Index(i)) => items.get(*i),
        _ => None,
    }
}

fn child_ref_mut<'a>(node: &'a mut YamlNode, seg: &PathSeg) -> Option<&'a mut YamlNode> {
    match (&mut node.kind, seg) {
        (YamlNodeKind::Mapping(pairs), PathSeg::Key(k)) => pairs
            .iter_mut()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(_, v)| v),
        (YamlNodeKind::Mapping(pairs), PathSeg::KeyNode(k)) => pairs
            .iter_mut()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(key, _)| key),
        (YamlNodeKind::Sequence(items), PathSeg::Index(i)) => items.get_mut(*i),
        _ => None,
    }
}

pub(crate) fn scalar_eq(key: &YamlNode, name: &str) -> bool {
    matches!(&key.kind, YamlNodeKind::Scalar(s, _) if s == name)
}

/// Whether `key_node`, resolved under `schema`, equals the Python `key`. Indexed
/// access and assignment match the *resolved* mapping key, so `doc[True]` reaches
/// a `yes:` entry under YAML 1.1 (and `doc['yes']` does not), consistent with
/// `keys()`/`to_dict()` and with Python dict semantics (e.g. `True == 1`). Only
/// scalar keys are addressable this way; a collection key is not.
fn key_node_matches(
    py: Python<'_>,
    key_node: &YamlNode,
    key: &Bound<'_, PyAny>,
    schema: Schema,
    anchors: &HashMap<String, YamlNode>,
) -> bool {
    if !matches!(key_node.kind, YamlNodeKind::Scalar(..)) {
        return false;
    }
    node_to_python_with(py, key_node, schema, anchors)
        .bind(py)
        .eq(key)
        .unwrap_or(false)
}

// -- Child access shared by YAMLRocksDocument and YAMLRocksDocumentView --

/// Read a child of `node`: scalars resolve to plain Python; containers return a
/// [`YAMLRocksDocumentView`] rooted at `root` with the child appended to `parent_path`.
fn access_child(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    parent_path: &[PathSeg],
    node: &YamlNode,
    key: &Bound<'_, PyAny>,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
) -> PyResult<Py<PyAny>> {
    // For a mapping, match the looked-up key against each key resolved under the
    // schema (so `doc[True]` finds a `yes:` entry), recording the matched key's
    // lexeme for the path. Sequences index by position as before.
    let seg = match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            let (k, _) = pairs
                .iter()
                .find(|(k, _)| key_node_matches(py, k, key, schema, anchors))
                .ok_or_else(|| key_error(node, key))?;
            let YamlNodeKind::Scalar(lexeme, _) = &k.kind else {
                unreachable!("key_node_matches only matches scalar keys")
            };
            PathSeg::Key(lexeme.clone())
        }
        _ => seg_from_key(key)?,
    };
    let child = child_ref(node, &seg).ok_or_else(|| key_error(node, key))?;

    let mut path = parent_path.to_vec();
    path.push(seg);
    wrap_node(py, root, path, child, anchors, schema)
}

/// Wrap a node for return to Python: scalars as plain values, containers as
/// live [`YAMLRocksDocumentView`] proxies.
fn wrap_node(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    path: Vec<PathSeg>,
    node: &YamlNode,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
) -> PyResult<Py<PyAny>> {
    match &node.kind {
        YamlNodeKind::Mapping(_) | YamlNodeKind::Sequence(_) => {
            let view = YAMLRocksDocumentView {
                root: root.clone().unbind(),
                path,
            };
            Ok(Py::new(py, view)?.into_any())
        }
        // An alias cannot resolve to a live view: it has no path of its own, so
        // we follow it to its anchor and return a plain snapshot instead.
        YamlNodeKind::Alias(name) => match anchors.get(name) {
            Some(target) => wrap_node(py, root, path, target, anchors, schema),
            None => Ok(py.None()),
        },
        _ => Ok(node_to_python_with(py, node, schema, anchors)),
    }
}

fn set_child(
    py: Python<'_>,
    node: &mut YamlNode,
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    double_quotes: bool,
    schema: Schema,
    anchors: &HashMap<String, YamlNode>,
) -> PyResult<()> {
    match &mut node.kind {
        YamlNodeKind::Mapping(pairs) => {
            let mut new_val = python_to_node(py, value, double_quotes, schema)?;
            new_val.mark_modified();
            // Match an existing key by its resolved value (so `doc[True] = x`
            // updates a `yes:` entry under 1.1), mirroring read access.
            for (k, v) in pairs.iter_mut() {
                if key_node_matches(py, k, key, schema, anchors) {
                    // Replacing a value keeps the metadata attached to it (its
                    // comments, anchor, and tag), matching `YAMLRocksNode.set_value`, so
                    // an edit does not silently drop nearby comments or markup.
                    new_val.comments.head = std::mem::take(&mut v.comments.head);
                    new_val.comments.inline = v.comments.inline.take();
                    new_val.comments.value_pad = v.comments.value_pad;
                    new_val.comments.inline_spaces = v.comments.inline_spaces;
                    new_val.comments.foot = std::mem::take(&mut v.comments.foot);
                    new_val.anchor = v.anchor.take();
                    new_val.tag = v.tag.take();
                    *v = new_val;
                    return Ok(());
                }
            }
            // No key matched: add one, building the key node from the Python key
            // so a non-string key (`doc[True] = ...`) and a string that needs
            // quoting under the schema both emit correctly.
            let mut new_key = python_to_node(py, key, double_quotes, schema)?;
            new_key.mark_modified();
            pairs.push((new_key, new_val));
            Ok(())
        }
        YamlNodeKind::Sequence(items) => {
            let idx: usize = key.extract()?;
            let target = items
                .get_mut(idx)
                .ok_or_else(|| pyo3::exceptions::PyIndexError::new_err("index out of range"))?;
            let mut new_val = python_to_node(py, value, double_quotes, schema)?;
            new_val.mark_modified();
            // Carry the replaced item's metadata (comments, anchor, tag) onto
            // the new value so editing a list entry preserves the markup around
            // it, matching `YAMLRocksNode.set_value`.
            new_val.comments.head = std::mem::take(&mut target.comments.head);
            new_val.comments.inline = target.comments.inline.take();
            new_val.comments.value_pad = target.comments.value_pad;
            new_val.comments.inline_spaces = target.comments.inline_spaces;
            new_val.comments.foot = std::mem::take(&mut target.comments.foot);
            new_val.anchor = target.anchor.take();
            new_val.tag = target.tag.take();
            *target = new_val;
            Ok(())
        }
        _ => Err(pyo3::exceptions::PyTypeError::new_err(
            "node is not a mapping or sequence",
        )),
    }
}

fn del_child(node: &mut YamlNode, key: &Bound<'_, PyAny>) -> PyResult<()> {
    match &mut node.kind {
        YamlNodeKind::Mapping(pairs) => {
            let key_str: String = key
                .extract()
                .map_err(|_| pyo3::exceptions::PyTypeError::new_err("mapping keys must be str"))?;
            // The removed pair carries its own attached comments away with it; the
            // surrounding pairs keep their nodes (and comments).
            let idx = pairs
                .iter()
                .position(|(k, _)| scalar_eq(k, &key_str))
                .ok_or_else(|| pyo3::exceptions::PyKeyError::new_err(key_str.clone()))?;
            pairs.remove(idx);
        }
        YamlNodeKind::Sequence(items) => {
            let idx: usize = key.extract()?;
            if idx >= items.len() {
                return Err(pyo3::exceptions::PyIndexError::new_err(
                    "index out of range",
                ));
            }
            items.remove(idx);
        }
        _ => {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "node is not a mapping or sequence",
            ));
        }
    }
    // Mark the container modified so the document re-emits from the AST instead of
    // replaying its original source bytes verbatim (those still contain the removed
    // entry — see `to_yaml`/`nodes_modified`). Re-emission walks the pairs/items and
    // emits each surviving node's own comments, so the rest stays comment-preserving.
    node.mark_modified();
    Ok(())
}

fn node_contains(node: &YamlNode, key: &Bound<'_, PyAny>) -> bool {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => key
            .extract::<String>()
            .map(|k| pairs.iter().any(|(key_node, _)| scalar_eq(key_node, &k)))
            .unwrap_or(false),
        _ => false,
    }
}

fn node_keys(
    py: Python<'_>,
    node: &YamlNode,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
) -> PyResult<Py<PyAny>> {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            let list = PyList::empty(py);
            for (k, _) in pairs {
                list.append(node_to_python_with(py, k, schema, anchors))?;
            }
            Ok(list.into_any().unbind())
        }
        _ => Err(pyo3::exceptions::PyTypeError::new_err(
            "node is not a mapping",
        )),
    }
}

fn container_len(node: &YamlNode) -> usize {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => pairs.len(),
        YamlNodeKind::Sequence(items) => items.len(),
        _ => 1,
    }
}

fn node_kind_name(node: &YamlNode) -> &'static str {
    match &node.kind {
        YamlNodeKind::Mapping(_) => "mapping",
        YamlNodeKind::Sequence(_) => "sequence",
        YamlNodeKind::Scalar(_, _) => "scalar",
        YamlNodeKind::Alias(_) => "alias",
        YamlNodeKind::Null => "null",
    }
}

fn key_error(node: &YamlNode, key: &Bound<'_, PyAny>) -> PyErr {
    match &node.kind {
        YamlNodeKind::Sequence(_) => pyo3::exceptions::PyIndexError::new_err("index out of range"),
        _ => pyo3::exceptions::PyKeyError::new_err(format!("{key}")),
    }
}

/// Write `bytes` to `path`, surfacing IO errors as Python `OSError`.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> PyResult<()> {
    std::fs::write(path, bytes).map_err(|e| {
        pyo3::exceptions::PyOSError::new_err(format!("cannot write {}: {e}", path.display()))
    })
}

/// The 1-based `(start_line, start_column, end_line, end_column)` source range
/// of a node, as a Python tuple.
///
/// The end position is the node's exact source end (`end_line`/`end_column`,
/// derived by the composer from the true end byte offset), so it is correct for
/// quoted and escaped scalars, not just plain ones, and is the furthest child end
/// for a collection.
fn node_range(py: Python<'_>, node: &YamlNode) -> Py<PyAny> {
    let range = (
        node.span.line + 1,
        node.span.column + 1,
        node.end_line + 1,
        node.end_column + 1,
    );
    range
        .into_pyobject(py)
        .map(|t| t.into_any().unbind())
        .unwrap_or_else(|_| py.None())
}

/// Whether any node in the document has been modified since loading.
fn nodes_modified(nodes: &[YamlNode]) -> bool {
    nodes.iter().any(node_modified)
}

fn node_modified(node: &YamlNode) -> bool {
    // Grow the native stack on demand: this walks the whole tree (once per
    // `to_yaml` on an unmodified document) and would otherwise overflow a small
    // thread stack on a deeply nested document. See [`crate::stack`].
    crate::stack::guard(|| {
        if node.comments.modified {
            return true;
        }
        match &node.kind {
            YamlNodeKind::Mapping(pairs) => pairs
                .iter()
                .any(|(key, val)| node_modified(key) || node_modified(val)),
            YamlNodeKind::Sequence(items) => items.iter().any(node_modified),
            _ => false,
        }
    })
}

// -- Traversal --

/// Recursively collect `(path, value)` for every scalar leaf under `node`.
fn collect_leaves(
    py: Python<'_>,
    node: &YamlNode,
    path: &mut Vec<Py<PyAny>>,
    out: &mut Vec<(Vec<Py<PyAny>>, Py<PyAny>)>,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
) -> PyResult<()> {
    // Grow the native stack on demand so walking a deeply nested document cannot
    // overflow a small thread stack. See [`crate::stack`].
    crate::stack::guard(|| collect_leaves_inner(py, node, path, out, anchors, schema))
}

fn collect_leaves_inner(
    py: Python<'_>,
    node: &YamlNode,
    path: &mut Vec<Py<PyAny>>,
    out: &mut Vec<(Vec<Py<PyAny>>, Py<PyAny>)>,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
) -> PyResult<()> {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                if let YamlNodeKind::Scalar(_, _) = &key.kind {
                    // Resolve the path key under the schema so `walk()` paths match
                    // `keys()` and indexed access (a `yes:` key is `True` under 1.1).
                    path.push(node_to_python_with(py, key, schema, anchors));
                    collect_leaves(py, val, path, out, anchors, schema)?;
                    path.pop();
                }
            }
        }
        YamlNodeKind::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(i.into_pyobject(py)?.into_any().unbind());
                collect_leaves(py, item, path, out, anchors, schema)?;
                path.pop();
            }
        }
        _ => out.push((path.clone(), node_to_python_with(py, node, schema, anchors))),
    }
    Ok(())
}

/// Convert collected leaves into a list of `(path_tuple, value)` tuples.
fn leaves_to_list(py: Python<'_>, leaves: Vec<(Vec<Py<PyAny>>, Py<PyAny>)>) -> PyResult<Py<PyAny>> {
    let list = PyList::empty(py);
    for (path, value) in leaves {
        let tuple = PyTuple::new(py, path)?;
        list.append((tuple, value))?;
    }
    Ok(list.into_any().unbind())
}

#[cfg(test)]
mod tests {
    use super::stamp_yaml_1_2;

    fn stamp(input: &[u8]) -> Vec<u8> {
        let mut buf = input.to_vec();
        stamp_yaml_1_2(&mut buf);
        buf
    }

    #[test]
    fn prepends_version_to_a_bare_document() {
        assert_eq!(stamp(b"a: 1\n"), b"%YAML 1.2\n---\na: 1\n");
    }

    #[test]
    fn reuses_an_existing_document_marker() {
        assert_eq!(stamp(b"---\na: 1\n"), b"%YAML 1.2\n---\na: 1\n");
    }

    #[test]
    fn replaces_a_declared_version() {
        assert_eq!(stamp(b"%YAML 1.1\n---\na: 1\n"), b"%YAML 1.2\n---\na: 1\n");
    }

    #[test]
    fn keeps_a_tag_directive_after_the_version() {
        // The `%YAML 1.2` must sit *before* the `%TAG`, never displace it past
        // the `---` (which would leave the handle undefined).
        assert_eq!(
            stamp(b"%TAG !e! tag:x:\n---\nv: !e!foo 1\n"),
            b"%YAML 1.2\n%TAG !e! tag:x:\n---\nv: !e!foo 1\n",
        );
    }

    #[test]
    fn replaces_version_but_keeps_tag_directive() {
        assert_eq!(
            stamp(b"%YAML 1.1\n%TAG !e! tag:x:\n---\nv: !e!foo 1\n"),
            b"%YAML 1.2\n%TAG !e! tag:x:\n---\nv: !e!foo 1\n",
        );
    }

    #[test]
    fn preserves_a_leading_byte_order_mark() {
        let bom = [0xEF, 0xBB, 0xBF];
        let mut input = bom.to_vec();
        input.extend_from_slice(b"a: 1\n");
        let mut expected = bom.to_vec();
        expected.extend_from_slice(b"%YAML 1.2\n---\na: 1\n");
        assert_eq!(stamp(&input), expected);
    }
}
