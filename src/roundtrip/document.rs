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
use pyo3::types::{PyBytes, PyDict, PyInt, PyList, PyString, PyTuple};

use super::anchors::{
    alias_target_path, build_anchor_map, collect_alias_paths, collect_anchor_paths,
    collect_anchor_refs, detached_clone, find_anchor_path, path_precedes,
};
use super::ast::{EmptyDocuments, HeadComment, NodeStyle, YamlNode, YamlNodeKind};
use super::emit;
use super::emit::{emit_roundtrip_all_with, emit_roundtrip_with};
use super::value::{
    assigned_scalar, node_to_python_key, node_to_python_with, python_to_node, ObjectCache,
    ScalarTarget,
};
use crate::encode::NullStyle;
use crate::resolver::{ResolvedValue, ScalarKind, Schema};
use crate::scanner::ScalarStyle;

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
    /// The document at `index` of an object holding `documents` documents with
    /// content. Only ever the first step of a path, and only into an object
    /// holding several documents: a single-document object *is* its document,
    /// so its paths start inside it (see [`document_index`]).
    ///
    /// The count pins the path to the stream it was taken from. Deleting a
    /// document changes the count, so a handle taken before that resolves to
    /// nothing, instead of to whichever document moved into its position.
    Document { index: usize, documents: usize },
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
#[pyclass(name = "YAMLRocksDocument")]
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
    /// Whether reads (`to_dict`, indexing, `walk`, `keys`) resolve a plain
    /// timestamp scalar to a `date`/`datetime` (`OPT_TIMESTAMPS`, or
    /// `OPT_PYYAML_COMPAT` which implies it). Off by default.
    pub resolve_timestamps: bool,
    /// Advances with every write borrow of the tree (see [`write_document`]).
    /// An operation that must run Python code between finding its target and
    /// writing it records this first and checks it again under the write
    /// borrow, so it never writes into a tree that changed in between.
    revision: u64,
}

/// Borrow a round-trip document for reading.
///
/// Every borrow this module takes by hand goes through this or
/// [`write_document`], which turn a document that is already borrowed into an
/// ordinary `RuntimeError` instead of a panic. That happens when Python code that
/// runs during an operation (a key's `__eq__`, a value's `__float__`, a
/// callback) touches the same document, or when two threads use one document at
/// once on a free-threaded build. A `&self` method borrows through PyO3 itself,
/// which raises a `RuntimeError` the same way.
pub(crate) fn read_document<'py>(
    doc: &Bound<'py, YAMLRocksDocument>,
) -> PyResult<PyRef<'py, YAMLRocksDocument>> {
    doc.try_borrow().map_err(|_| document_in_use())
}

/// Borrow a round-trip document for writing. Each write borrow advances the
/// document's revision, so an operation that prepared its write earlier can
/// tell that something may have changed since (see [`write_unchanged`]).
pub(crate) fn write_document<'py>(
    doc: &Bound<'py, YAMLRocksDocument>,
) -> PyResult<PyRefMut<'py, YAMLRocksDocument>> {
    let mut doc = doc.try_borrow_mut().map_err(|_| document_in_use())?;
    doc.revision = doc.revision.wrapping_add(1);
    Ok(doc)
}

/// Borrow a round-trip document for writing a change that was prepared while
/// the document stood at `revision`. Raises, and writes nothing, when any write
/// borrow was taken in between: the target found earlier may no longer be
/// there, or something else may have moved into its place.
fn write_unchanged<'py>(
    doc: &Bound<'py, YAMLRocksDocument>,
    revision: u64,
) -> PyResult<PyRefMut<'py, YAMLRocksDocument>> {
    let mut doc = doc.try_borrow_mut().map_err(|_| document_in_use())?;
    if doc.revision != revision {
        return Err(document_changed());
    }
    doc.revision = doc.revision.wrapping_add(1);
    Ok(doc)
}

/// Borrow a round-trip document for reading the target a lookup found while the
/// document stood at `revision`, raising when it was written to in between.
fn read_unchanged<'py>(
    doc: &Bound<'py, YAMLRocksDocument>,
    revision: u64,
) -> PyResult<PyRef<'py, YAMLRocksDocument>> {
    let doc = read_document(doc)?;
    if doc.revision != revision {
        return Err(document_changed());
    }
    Ok(doc)
}

/// The error for a document that is already borrowed in a way that conflicts
/// with the one requested.
fn document_in_use() -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(
        "the round-trip document is in use by another call or thread",
    )
}

/// The error for an operation whose own Python code (a key's `__eq__` or
/// `__index__`, a value's conversion), or another thread, changed the document
/// while the operation was preparing.
fn document_changed() -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(
        "the round-trip document changed while the key or value was being \
         converted; nothing was changed",
    )
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

    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        if self.nodes.len() == 1 {
            let anchors = build_anchor_map(&self.nodes);
            container_len(
                py,
                &self.nodes[0],
                self.schema,
                &anchors,
                self.resolve_timestamps,
            )
        } else {
            Ok(self.nodes.len())
        }
    }

    fn __getitem__(slf: &Bound<'_, Self>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(lookup_item(slf, key)?)
    }

    fn __setitem__(
        slf: &Bound<'_, Self>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let py = slf.py();
        let (documents, revision, target) = {
            let this = read_document(slf)?;
            (this.nodes.len(), this.revision, this.scalar_target())
        };
        if documents == 1 {
            return set_child(py, slf, &[], key, value, stale_view);
        }

        // Replace a whole document. The index and the value are converted with
        // no borrow held; the write then checks that no document was added,
        // removed or changed meanwhile, so the index still means the same one.
        let idx: usize = key.extract()?;
        if idx >= documents {
            return Err(index_out_of_range());
        }
        let mut new_root = python_to_node(py, value, target)?;

        let mut this = write_unchanged(slf, revision)?;
        let node = this.nodes.get_mut(idx).ok_or_else(index_out_of_range)?;
        // The empty documents around a root belong to the stream, not to
        // the value being replaced.
        new_root.empty_documents = node.empty_documents.take();
        *node = new_root;
        node.mark_modified();
        Ok(())
    }

    fn __delitem__(slf: &Bound<'_, Self>, key: &Bound<'_, PyAny>) -> PyResult<()> {
        let revision = {
            let this = read_document(slf)?;
            if this.nodes.len() == 1 {
                drop(this);
                return del_child(slf.py(), slf, &[], key, stale_view);
            }
            this.revision
        };

        // Delete a whole document, its index converted with no borrow held.
        let idx: usize = key.extract()?;
        let mut this = write_unchanged(slf, revision)?;
        if idx >= this.nodes.len() {
            return Err(index_out_of_range());
        }
        let removed = this.nodes.remove(idx);
        if let Some(empty_documents) = removed.empty_documents {
            keep_empty_documents(&mut this.nodes, idx, *empty_documents);
        }
        // The cached source still contains the removed document; drop it so
        // the stream re-emits from the AST instead of replaying verbatim.
        this.source = None;
        Ok(())
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> bool {
        self.nodes.len() == 1 && node_contains(&self.nodes[0], key)
    }

    fn get(
        slf: &Bound<'_, Self>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        lookup_item(slf, key).or_else(|failure| failure.or_default(slf.py(), default))
    }

    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.nodes.len() == 1 {
            let anchors = build_anchor_map(&self.nodes);
            node_keys(
                py,
                &self.nodes[0],
                &anchors,
                self.schema,
                self.resolve_timestamps,
            )
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

    /// Whether the first document used an explicit `---` start marker.
    /// Read directly from the parsed tree, including before any edits.
    #[getter]
    fn explicit_start(&self) -> bool {
        self.nodes.first().is_some_and(|node| node.explicit_start)
    }

    /// The root [`YAMLRocksNode`] cursor: a metadata-bearing handle from which any node
    /// in the tree can be reached by indexing (`doc.node["server"]["port"]`).
    /// Unlike item access, indexing a `YAMLRocksNode` always yields another `YAMLRocksNode`, so
    /// comments, source location, style, anchor, and tag stay reachable down to
    /// individual scalars.
    ///
    /// In a stream of several documents this is the first document's root, as
    /// with `locate` and `anchors`; `doc[n].node` reaches any other
    /// mapping or sequence document (a scalar document reads as its plain value).
    #[getter]
    fn node(slf: &Bound<'_, Self>) -> PyResult<Py<YAMLRocksNode>> {
        let py = slf.py();
        let path = read_document(slf)?.first_document_path(Vec::new());
        Py::new(
            py,
            YAMLRocksNode {
                root: slf.clone().unbind(),
                path,
            },
        )
    }

    /// A mapping of every anchor name (`&name`) to the [`YAMLRocksNode`] that defines it,
    /// for discovering and navigating the document's anchors. Use a definition's
    /// `YAMLRocksNode.aliases` to find the `*name` references that point back at it.
    #[getter]
    fn anchors(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let this = read_document(slf)?;
        let mut found: Vec<(String, Vec<PathSeg>)> = Vec::new();
        if let Some(root) = this.nodes.first() {
            collect_anchor_paths(root, &mut Vec::new(), &mut found);
        }
        let dict = PyDict::new(py);
        for (name, path) in found {
            let node = Py::new(
                py,
                YAMLRocksNode {
                    root: slf.clone().unbind(),
                    path: this.first_document_path(path),
                },
            )?;
            dict.set_item(name, node)?;
        }
        Ok(dict.into_any().unbind())
    }

    /// Snapshot one document's values and metadata in a single tree traversal.
    /// Empty input returns None; use `loads_all` for a multi-document stream.
    fn to_tree(&self, py: Python<'_>) -> PyResult<Option<Py<super::tree::YAMLRocksTreeNode>>> {
        if self.nodes.len() > 1 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "to_tree() requires one document; use loads_all(option=OPT_ROUND_TRIP)",
            ));
        }
        self.nodes
            .first()
            .map(|node| super::tree::snapshot(py, node, self.schema, self.resolve_timestamps))
            .transpose()
    }

    /// Return the document as plain Python objects (a resolved snapshot).
    fn to_dict(&self, py: Python<'_>) -> Py<PyAny> {
        if self.nodes.len() == 1 {
            let anchors = build_anchor_map(&self.nodes);
            node_to_python_with(
                py,
                &self.nodes[0],
                self.schema,
                self.resolve_timestamps,
                &anchors,
            )
        } else {
            let list = PyList::empty(py);
            for node in &self.nodes {
                // Each document resolves its aliases against its own anchors.
                let anchors = build_anchor_map(std::slice::from_ref(node));
                let _ = list.append(node_to_python_with(
                    py,
                    node,
                    self.schema,
                    self.resolve_timestamps,
                    &anchors,
                ));
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
        if let Some(root) = self.nodes.first() {
            let anchors = build_anchor_map(std::slice::from_ref(root));
            collect_leaves(
                py,
                root,
                &mut Vec::new(),
                &mut out,
                &anchors,
                self.schema,
                self.resolve_timestamps,
            )?;
        }
        leaves_to_list(py, out)
    }

    /// Resolve a data path to the [`YAMLRocksNode`] at that location, scalar leaves
    /// included, or ``None`` when the path does not resolve.
    ///
    /// ``path`` is a sequence of mapping keys (``str``) and sequence indices
    /// (``int``), the shape a validator emits when it reports where in the data a
    /// failure occurred (``["servers", 1, "port"]``). Unlike item access, which
    /// unwraps a scalar to its plain value, this always returns a positioned node,
    /// so ``.line`` / ``.column`` / ``.file`` / ``.range()`` are available right
    /// down to a leaf. The position is the value's own (not its key's).
    ///
    /// Returns ``None`` if any segment is absent (a missing key or an out-of-range
    /// index), so a caller can retry with shorter prefixes to fall back to the
    /// nearest enclosing container. Addresses the first document of a stream.
    ///
    /// ```python
    /// doc = yamlrocks.loads(b"port: 8080\n", option=yamlrocks.OPT_ROUND_TRIP)
    /// node = doc.locate(["port"])
    /// node.line, node.column  # (1, 7), the value 8080, 1-indexed
    /// ```
    fn locate(
        slf: &Bound<'_, Self>,
        path: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<YAMLRocksNode>>> {
        // A bare string is iterable, so accepting one would silently walk its
        // characters; reject it so the `str | int` segments are explicit.
        if path.is_instance_of::<PyString>() {
            return Err(pyo3::exceptions::PyTypeError::new_err(
                "locate() takes a sequence of str/int path segments, not a str",
            ));
        }
        let mut segments: Vec<PathSeg> = Vec::new();
        for item in path.try_iter()? {
            segments.push(seg_from_key(&item?)?);
        }
        let this = read_document(slf)?;
        let segments = this.first_document_path(segments);
        let resolved = resolve_path(&this.nodes, &segments).is_some();
        if !resolved {
            return Ok(None);
        }
        let node = Py::new(
            slf.py(),
            YAMLRocksNode {
                root: slf.clone().unbind(),
                path: segments,
            },
        )?;
        Ok(Some(node))
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
            resolve_timestamps: false,
            revision: 0,
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
            resolve_timestamps: false,
            revision: 0,
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

    /// Set whether reads resolve plain timestamp scalars to `date`/`datetime`
    /// (`OPT_TIMESTAMPS` or `OPT_PYYAML_COMPAT`).
    pub fn with_resolve_timestamps(mut self, resolve_timestamps: bool) -> Self {
        self.resolve_timestamps = resolve_timestamps;
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

    /// The full path to a node at `inner` steps below the first document's
    /// root, which the document-wide accessors (`node`, `locate`, `anchors`)
    /// address.
    fn first_document_path(&self, inner: Vec<PathSeg>) -> Vec<PathSeg> {
        let mut path = document_prefix(self.nodes.len(), 0);
        path.extend(inner);
        path
    }

    /// How this document reads scalars, and so how a freshly assigned one must
    /// be written into it.
    fn scalar_target(&self) -> ScalarTarget {
        ScalarTarget {
            double_quotes: self.double_quotes,
            schema: self.schema,
            resolve_timestamps: self.resolve_timestamps,
        }
    }

    /// Re-emit the document from the AST (used when modified or sourceless).
    /// Always as a stream, even for one root: the empty documents around it
    /// are part of the stream, not of the root.
    fn emit_nodes(&self) -> Vec<u8> {
        emit_roundtrip_all_with(&self.nodes, self.null_style)
    }
}

/// Hand the empty documents of a root removed from `nodes` at `idx` to a
/// remaining neighbour, so deleting a document removes only that document.
/// The ones before it now precede the root that followed it; with no root
/// after it, they and any that ended the stream move to the new last root.
/// With no root left at all, there is nothing to carry them.
fn keep_empty_documents(nodes: &mut [YamlNode], idx: usize, removed: EmptyDocuments) {
    if let Some(next) = nodes.get_mut(idx) {
        let carried = next.empty_documents.get_or_insert_with(Default::default);
        carried.before.splice(0..0, removed.before);
    } else if let Some(last) = nodes.last_mut() {
        let carried = last.empty_documents.get_or_insert_with(Default::default);
        carried.after.extend(removed.before);
        carried.after.extend(removed.after);
    }
}

/// Prepend a `%YAML 1.2` version directive to `buf` so an upgraded document
/// declares its version (and is read back as 1.2, not re-upgraded). A leading
/// byte order mark stays first; an existing `---` document-start marker is
/// reused, otherwise one is added. Any `%YAML` directive among the leading
/// directives is dropped (the canonical 1.2 declaration replaces it); a `%TAG`
/// directive is kept in place, with `%YAML 1.2` inserted ahead of it. Comment
/// and blank lines may sit before, between, or after those directives; the
/// stamp goes ahead of all of them.
fn stamp_yaml_1_2(buf: &mut Vec<u8>) {
    const BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
    let at = if buf.starts_with(&BOM) { BOM.len() } else { 0 };
    // Walk the leading lines up to the first document's marker or content. A
    // `%YAML` line is dropped (the canonical 1.2 declaration below replaces it;
    // this also keeps re-stamping idempotent); any other directive (a `%TAG`
    // handle) is kept and skipped over so the marker is found past it, not
    // before it. Inserting `%YAML 1.2\n---\n` ahead of a `%TAG` would strand the
    // handle after a document start and fail to reload. Comment and blank lines
    // are skipped too: a `---` after them is still the first document's marker,
    // and adding another one ahead of them would add a document.
    let mut cursor = at;
    loop {
        let rest = &buf[cursor..];
        let line_len = line_length(rest);
        if rest.starts_with(b"%YAML") {
            buf.drain(cursor..cursor + line_len);
        } else if rest.starts_with(b"%") || is_comment_or_blank_line(&rest[..line_len]) {
            if line_len == 0 {
                break;
            }
            cursor += line_len;
        } else {
            break;
        }
    }
    let body = &buf[cursor..];
    let has_marker = body.starts_with(b"---")
        && body
            .get(3)
            .is_none_or(|&b| matches!(b, b'\n' | b'\r' | b' ' | b'\t'));
    let mut stamp: Vec<u8> = b"%YAML 1.2\n".to_vec();
    if !has_marker {
        // No directives precede us here (a directive always carries its own
        // `---`), so the marker can sit right after the version line.
        stamp.extend_from_slice(b"---\n");
    }
    buf.splice(at..at, stamp);
}

/// The length of the first line of `text`, its line break included (`\n`,
/// `\r\n`, or a lone `\r`), or all of `text` when it has no line break.
fn line_length(text: &[u8]) -> usize {
    match text.iter().position(|&b| b == b'\n' || b == b'\r') {
        Some(at) if text[at] == b'\r' && text.get(at + 1) == Some(&b'\n') => at + 2,
        Some(at) => at + 1,
        None => text.len(),
    }
}

/// Whether `line` holds nothing but whitespace, optionally ending in a comment.
fn is_comment_or_blank_line(line: &[u8]) -> bool {
    let content = line
        .iter()
        .position(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
    content.is_none_or(|at| line[at] == b'#')
}

/// A live proxy onto a nested node of a [`YAMLRocksDocument`], addressed by path.
///
/// A view never changes after it is made, so it is frozen: calls on it borrow
/// only its document, never the view itself.
#[pyclass(name = "YAMLRocksDocumentView", frozen)]
pub struct YAMLRocksDocumentView {
    root: Py<YAMLRocksDocument>,
    path: Vec<PathSeg>,
}

#[pymethods]
impl YAMLRocksDocumentView {
    fn __len__(&self, py: Python<'_>) -> PyResult<usize> {
        let doc = read_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        let anchors = build_anchor_map(document);
        container_len(py, node, doc.schema, &anchors, doc.resolve_timestamps)
    }

    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        Ok(get_child(
            py,
            self.root.bind(py),
            &self.path,
            key,
            stale_view,
        )?)
    }

    fn __setitem__(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        set_child(py, self.root.bind(py), &self.path, key, value, stale_view)
    }

    fn __delitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<()> {
        del_child(py, self.root.bind(py), &self.path, key, stale_view)
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        Ok(node_contains(node, key))
    }

    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        get_child(py, self.root.bind(py), &self.path, key, stale_view)
            .or_else(|failure| failure.or_default(py, default))
    }

    fn keys(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        let anchors = build_anchor_map(document);
        node_keys(py, node, &anchors, doc.schema, doc.resolve_timestamps)
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
        let doc = read_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        let anchors = build_anchor_map(document);
        Ok(node_to_python_with(
            py,
            node,
            doc.schema,
            doc.resolve_timestamps,
            &anchors,
        ))
    }

    /// Alias for [`unwrap`](Self::unwrap): the resolved plain-Python value.
    fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.unwrap(py)
    }

    /// The node's 1-based source range `(start_line, start_col, end_line,
    /// end_col)`.
    fn range(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        Ok(node_range(py, node))
    }

    /// Walk every scalar leaf below this view, yielding ``(path, value)`` pairs
    /// relative to this node.
    fn walk(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        let anchors = build_anchor_map(document);
        let mut out: Vec<(Vec<Py<PyAny>>, Py<PyAny>)> = Vec::new();
        collect_leaves(
            py,
            node,
            &mut Vec::new(),
            &mut out,
            &anchors,
            doc.schema,
            doc.resolve_timestamps,
        )?;
        leaves_to_list(py, out)
    }

    /// Serialize just this nested node back to YAML bytes.
    fn to_yaml(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_view)?;
        Ok(PyBytes::new(py, &emit_roundtrip_with(node, doc.null_style))
            .into_any()
            .unbind())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "YAMLRocksDocumentView({})",
            describe_target(self.root.bind(py), &self.path)
        )
    }
}

/// What a view or cursor's `repr()` shows for the node at `path`: its kind,
/// `<stale>` when the path no longer resolves, or `<in use>` when the document
/// cannot be read right now. A `repr()` that raised would get in the way of
/// the very error messages and debuggers that call it.
fn describe_target(root: &Bound<'_, YAMLRocksDocument>, path: &[PathSeg]) -> &'static str {
    let Ok(doc) = read_document(root) else {
        return "<in use>";
    };
    resolve_path(&doc.nodes, path).map_or("<stale>", node_kind_name)
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
/// the comment trailing the entry: after the value on its own line, or after the
/// `key:`/`-` that introduces it when the value is a block written below. Both
/// read and write the bare comment text, without the leading `#`.
///
/// Like a view, a cursor never changes after it is made (its setters write to
/// the document), so it is frozen.
#[pyclass(name = "YAMLRocksNode", frozen)]
pub struct YAMLRocksNode {
    root: Py<YAMLRocksDocument>,
    path: Vec<PathSeg>,
}

#[pymethods]
impl YAMLRocksNode {
    /// The resolved plain-Python value of this node (scalar, dict, or list).
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let anchors = build_anchor_map(document);
        Ok(node_to_python_with(
            py,
            node,
            doc.schema,
            doc.resolve_timestamps,
            &anchors,
        ))
    }

    /// Replace this node's value, preserving its comments, anchor, and tag.
    #[setter]
    fn set_value(&self, py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        let root = self.root.bind(py);
        let (revision, target) = {
            let doc = read_document(root)?;
            resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
            (doc.revision, doc.scalar_target())
        };
        // Converting the value runs its own Python code, which may read this
        // document (or copy it, when the value is a view of it), so it happens
        // with no borrow held.
        let new_val = python_to_node(py, value, target)?;

        let mut doc = write_unchanged(root, revision)?;
        let node = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        replace_keeping_metadata(node, new_val);
        Ok(())
    }

    /// The node's 1-based source line.
    #[getter]
    fn line(&self, py: Python<'_>) -> PyResult<u32> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.span.line + 1)
    }

    /// The node's 1-based source column.
    #[getter]
    fn column(&self, py: Python<'_>) -> PyResult<u32> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.span.column + 1)
    }

    /// The node's 1-based ``(start_line, start_col, end_line, end_col)`` source
    /// range, for underlining the exact span in an editor. The start matches
    /// [`line`](Self::line)/[`column`](Self::column); the end is the node's source
    /// end (a collection spans to the furthest end of any child).
    fn range(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node_range(py, node))
    }

    /// The 0-based byte offset of the node's first source character.
    ///
    /// Together with [`end_offset`](Self::end_offset) this gives the node's exact
    /// source extent, so the original bytes can be sliced directly
    /// (`source[node.offset : node.end_offset]`) — something line/column alone
    /// cannot do without re-deriving offsets.
    #[getter]
    fn offset(&self, py: Python<'_>) -> PyResult<usize> {
        let doc = read_document(self.root.bind(py))?;
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
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.end_offset)
    }

    /// The path of the source file this node came from, or `None` when the
    /// document was not loaded with includes (so there is a single source).
    #[getter]
    fn file(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(doc
            .file_map
            .get(node.span.file_id as usize)
            .map(|p| p.to_string_lossy().into_owned()))
    }

    /// The comment trailing this node, or `None`. The returned text has no
    /// leading `#` or surrounding whitespace.
    ///
    /// For a value written on the key's own line this is the comment after it
    /// (`port: 8080  # the http port`). For a block mapping or sequence, whose
    /// value starts on the line below, YAML puts that comment after the key
    /// instead (`servers: # the whole pool`), and it is read there. A sequence
    /// item written under its own dash (`- # note`) works the same way.
    #[getter]
    fn comment(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.comments.inline.clone())
    }

    /// Set or clear the trailing comment. Pass the bare text (no `#`) or `None`.
    /// It is written where YAML puts it for this node: after the value, or after
    /// the `key:`/`-` when the value is a block written on the lines below.
    #[setter]
    fn set_comment(&self, py: Python<'_>, text: Option<String>) -> PyResult<()> {
        let text = inline_comment_text(text)?;
        let mut doc = write_document(self.root.bind(py))?;
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
    ///
    /// A sequence item's head block can straddle its `-`: the item itself
    /// reports the comments above the dash, while the first key inside it
    /// reports the ones written below a comment on that dash, which is where
    /// they sit relative to that key. Each writes back the part it reads.
    #[getter]
    fn comment_before(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let (node, scope) = resolve_head(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(head_comment_text(node, scope))
    }

    /// Set or clear the standalone comment above this node. A multi-line string
    /// becomes one comment line per line; `None` removes it.
    #[setter]
    fn set_comment_before(&self, py: Python<'_>, text: Option<String>) -> PyResult<()> {
        let mut doc = write_document(self.root.bind(py))?;
        let (node, scope) = resolve_head_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        // Replace only the part this cursor reads, and keep the other side of the
        // dash: the caller never saw it, so overwriting it would delete a comment
        // out from under them.
        let kept: Vec<HeadComment> = match scope {
            HeadScope::All => Vec::new(),
            HeadScope::AboveDash | HeadScope::BelowDash => node
                .comments
                .head
                .iter()
                .filter(|comment| comment.below_introducer == (scope == HeadScope::AboveDash))
                .cloned()
                .collect(),
        };
        let written = split_comment_lines(text)?
            .into_iter()
            .map(|text| HeadComment {
                text: text.into_boxed_str(),
                below_introducer: scope == HeadScope::BelowDash,
            });
        // Source order: everything above the dash, then everything below it.
        node.comments.head = if scope == HeadScope::BelowDash {
            kept.into_iter().chain(written).collect()
        } else {
            written.chain(kept).collect()
        };
        node.mark_modified();
        Ok(())
    }

    /// The trailing comment line(s) after this node, joined by newlines, or
    /// `None`. For a block collection this is the comment block at the end of the
    /// mapping or sequence, indented with its contents; for the document root it
    /// is the comment block at the end of the file.
    #[getter]
    fn comment_after(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(join_comment_lines(&node.comments.foot))
    }

    /// Set or clear the trailing comment after this node. A multi-line string
    /// becomes one comment line per line; `None` removes it. On a block mapping or
    /// sequence the block is emitted at the collection's own indent, after its
    /// last entry; on the document root it is a comment at the end of the file.
    ///
    /// A foot comment only has a well-defined rendered position after a block
    /// collection or at the document root. Setting one on a scalar or a flow
    /// collection (anywhere but the root) has nowhere to emit, so it is rejected
    /// with `ValueError` rather than stored and silently dropped on re-emit.
    /// Clearing (`None`) is always allowed.
    #[setter]
    fn set_comment_after(&self, py: Python<'_>, text: Option<String>) -> PyResult<()> {
        let mut doc = write_document(self.root.bind(py))?;
        let (root, inner) =
            split_document_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        let is_document_root = inner.is_empty();
        let node = descend_mut(root, inner).ok_or_else(stale_node)?;
        // A document root renders a foot for any node kind; elsewhere only a
        // non-empty block mapping or sequence does.
        if text.is_some() && !is_document_root && !renders_foot(node) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "comment_after is only supported on a block mapping, a block \
                 sequence, or the document root",
            ));
        }
        node.comments.foot = split_comment_lines(text)?;
        node.mark_modified();
        Ok(())
    }

    /// The presentation style: a scalar's quoting (`plain`, `single`, `double`,
    /// `literal`, `folded`), a collection's layout (`block`, `flow`), `alias`,
    /// or `null`.
    #[getter]
    fn style(&self, py: Python<'_>) -> PyResult<&'static str> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node_style_name(node))
    }

    /// The node's anchor name (`&name`), or `None`.
    #[getter]
    fn anchor(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.anchor.clone())
    }

    /// Set or clear this node's anchor (`&name`). The name must be unique: it is
    /// an error to assign a name already defined on another node, so the document
    /// can never emit two `&name` definitions. Pass `None` to remove the anchor.
    #[setter]
    fn set_anchor(&self, py: Python<'_>, name: Option<String>) -> PyResult<()> {
        let mut doc = write_document(self.root.bind(py))?;
        if let Some(ref name) = name {
            if name.is_empty() {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "anchor name cannot be empty",
                ));
            }
            // The name must be a single anchor token: an emitted `&name` is read
            // back by the same rule, so whitespace, a line break, or a flow
            // indicator (`,[]{}`) would split it and corrupt the document
            // (`anchor = "a b"` -> `&a b value`, reloading as the value `b value`).
            if let Some(bad) = name.chars().find(|&c| !crate::scanner::is_anchor_char(c)) {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid anchor name {name:?}: cannot contain whitespace, line breaks, \
                     or flow indicators (found {bad:?})"
                )));
            }
            // Anchor names are scoped to their document: another document of
            // the stream may define the same name.
            let split = split_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
            if let Some(existing) = find_anchor_path(split.document, name) {
                if existing != split.inner {
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
        let mut doc = write_document(self.root.bind(py))?;
        // An alias can only refer to an anchor of its own document.
        let split = split_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let anchor_path = find_anchor_path(split.document, &name).ok_or_else(|| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "no anchor '&{name}' is defined in this document"
            ))
        })?;
        // The alias cannot reference its own ancestor (or itself): the anchored
        // node contains the alias, so expanding it would re-insert that container
        // forever. `path_precedes` treats an ancestor as preceding its
        // descendant, so this case must be rejected explicitly.
        if split.inner.starts_with(&anchor_path) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "anchor '&{name}' contains the alias that would reference it, which is a cycle"
            )));
        }
        if !path_precedes(split.document, &anchor_path, split.inner) {
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
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.tag.clone())
    }

    /// The config/custom tag that produced this node (`!secret`, `!env_var`,
    /// `!include*`, or a custom `!mytag`), or `None`. Provenance for a resolved
    /// node, distinct from `tag` (the node's own YAML type tag).
    #[getter]
    fn source_tag(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag().map(str::to_owned))
    }

    /// The directive argument that produced this node (the secret name, include
    /// path, or env-var spec), or `None`. With `source_tag`, reconstructs the
    /// directive (e.g. `!secret db_password`).
    #[getter]
    fn source_target(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_target().map(str::to_owned))
    }

    /// Whether this node's value was produced by a `!secret` directive.
    #[getter]
    fn is_secret(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag() == Some("!secret"))
    }

    /// Whether this node's value was produced by an `!env_var` directive.
    #[getter]
    fn is_env_var(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(node.source_tag() == Some("!env_var"))
    }

    /// Whether this node's value was produced by any `!include` directive.
    #[getter]
    fn is_include(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(crate::roundtrip::ast::is_include_tag(node.source_tag()))
    }

    /// Whether this node is an alias (`*name`).
    #[getter]
    fn is_alias(&self, py: Python<'_>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let node = resolve_path(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        Ok(matches!(node.kind, YamlNodeKind::Alias(_)))
    }

    /// For an alias, the `YAMLRocksNode` of the anchor definition it points at; `None`
    /// for any non-alias node (or an alias whose anchor is undefined).
    #[getter]
    fn target(&self, py: Python<'_>) -> PyResult<Option<Py<YAMLRocksNode>>> {
        let doc = read_document(self.root.bind(py))?;
        let split = split_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let node = split.node().ok_or_else(stale_node)?;
        if let YamlNodeKind::Alias(name) = &node.kind {
            if let Some(inner) = find_anchor_path(split.document, name) {
                return Ok(Some(Py::new(
                    py,
                    YAMLRocksNode {
                        root: self.root.clone_ref(py),
                        path: split.path_to(inner),
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
        let doc = read_document(self.root.bind(py))?;
        let split = split_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let node = split.node().ok_or_else(stale_node)?;
        let mut paths: Vec<Vec<PathSeg>> = Vec::new();
        if let (Some(name), Some(root)) = (&node.anchor, split.document.first()) {
            collect_alias_paths(root, name, &mut Vec::new(), &mut paths);
        }
        paths
            .into_iter()
            .map(|inner| {
                Py::new(
                    py,
                    YAMLRocksNode {
                        root: self.root.clone_ref(py),
                        path: split.path_to(inner),
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
    fn detach(&self, py: Python<'_>) -> PyResult<Py<YAMLRocksNode>> {
        let mut doc = write_document(self.root.bind(py))?;
        let (document, node) =
            resolve_in_document(&doc.nodes, &self.path).ok_or_else(stale_node)?;
        let name = match &node.kind {
            YamlNodeKind::Alias(name) => name.clone(),
            _ => {
                return Err(pyo3::exceptions::PyTypeError::new_err(
                    "detach() is only valid on an alias node",
                ));
            }
        };
        // The alias refers to an anchor of its own document.
        let mut refs: HashMap<String, &YamlNode> = HashMap::new();
        for root in document {
            collect_anchor_refs(root, &mut refs);
        }
        let target = refs.get(&name).ok_or_else(|| {
            pyo3::exceptions::PyKeyError::new_err(format!("alias *{name} has no anchor to detach"))
        })?;
        let mut clone = detached_clone(target, &refs);
        clone.mark_modified();
        let slot = resolve_path_mut(&mut doc.nodes, &self.path).ok_or_else(stale_node)?;
        *slot = clone;
        Py::new(
            py,
            YAMLRocksNode {
                root: self.root.clone_ref(py),
                path: self.path.clone(),
            },
        )
    }

    /// Index into a child by mapping key or sequence index, returning its
    /// `YAMLRocksNode`. Indexing an alias transparently follows it to the anchor it
    /// points at, so the child is a live handle into the shared definition.
    /// Scalars have no children, so indexing one raises.
    fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<Py<YAMLRocksNode>> {
        let root = self.root.bind(py);
        // A key that is not an exact `str` or `int` runs its own `__index__` to
        // become a step. That runs with no borrow held, after the same stale
        // check every key gets first.
        let converted = if converts_without_python(key) {
            None
        } else {
            let doc = read_document(root)?;
            let base = alias_followed_path(&doc.nodes, &self.path);
            resolve_path(&doc.nodes, &base).ok_or_else(stale_node)?;
            drop(doc);
            Some(seg_from_key(key)?)
        };

        let doc = read_document(root)?;
        let mut path = alias_followed_path(&doc.nodes, &self.path);
        let node = resolve_path(&doc.nodes, &path).ok_or_else(stale_node)?;
        let seg = match converted {
            Some(seg) => seg,
            None => seg_from_key(key)?,
        };
        if child_ref(node, &seg).is_none() {
            let in_sequence = matches!(node.kind, YamlNodeKind::Sequence(_));
            drop(doc);
            return Err(missing_child(in_sequence, key));
        }
        drop(doc);

        path.push(seg);
        Py::new(
            py,
            YAMLRocksNode {
                root: self.root.clone_ref(py),
                path,
            },
        )
    }

    fn __contains__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let doc = read_document(self.root.bind(py))?;
        let base = alias_followed_path(&doc.nodes, &self.path);
        let node = resolve_path(&doc.nodes, &base).ok_or_else(stale_node)?;
        Ok(node_contains(node, key))
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "YAMLRocksNode({})",
            describe_target(self.root.bind(py), &self.path)
        )
    }
}

/// Error for a `YAMLRocksNode` whose target no longer exists.
fn stale_node() -> PyErr {
    pyo3::exceptions::PyKeyError::new_err("stale node")
}

/// Error for a `YAMLRocksDocumentView` whose target no longer exists.
fn stale_view() -> PyErr {
    pyo3::exceptions::PyKeyError::new_err("stale view")
}

/// The path to index below for a cursor at `path`: the anchor an alias there
/// points at, looked up in the alias's own document, or `path` itself for
/// anything that is not an alias with a defined anchor.
fn alias_followed_path(roots: &[YamlNode], path: &[PathSeg]) -> Vec<PathSeg> {
    split_document(roots, path)
        .and_then(|split| {
            let inner = alias_target_path(split.document, split.inner)?;
            Some(split.path_to(inner))
        })
        .unwrap_or_else(|| path.to_vec())
}

/// The presentation-style name reported by `YAMLRocksNode.style`.
pub(crate) fn node_style_name(node: &YamlNode) -> &'static str {
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

/// The `comment_before` text stored on `node` for the part `scope` addresses,
/// or `None`. The single definition shared by the cursor and the tree snapshot.
pub(crate) fn head_comment_text(node: &YamlNode, scope: HeadScope) -> Option<String> {
    let lines: Vec<String> = match scope {
        HeadScope::All => node
            .comments
            .head
            .iter()
            .map(|comment| comment.text.to_string())
            .collect(),
        HeadScope::AboveDash => node
            .comments
            .head_above_introducer()
            .map(str::to_owned)
            .collect(),
        HeadScope::BelowDash => node
            .comments
            .head_below_introducer()
            .map(str::to_owned)
            .collect(),
    };
    join_comment_lines(&lines)
}

/// Join stored comment lines (text without `#`) for return to Python.
fn join_comment_lines(lines: &[String]) -> Option<String> {
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// The line breaks only a YAML 1.1 reader honors (NEL, LS, PS). This library
/// does not break lines on them, but PyYAML does, so one inside a comment would
/// end it there and turn the rest of the text into document content.
const YAML_11_LINE_BREAKS: [char; 3] = ['\u{85}', '\u{2028}', '\u{2029}'];

/// Validate a user-supplied inline comment. It must stay on one line: a line
/// break would end the comment and emit the rest as document content (`k: v #
/// a\nnot: comment` adds a `not` key), so any break is a `ValueError`.
pub(crate) fn inline_comment_text(text: Option<String>) -> PyResult<Option<String>> {
    let has_break = text
        .as_deref()
        .is_some_and(|text| text.contains(['\n', '\r']) || text.contains(YAML_11_LINE_BREAKS));
    if has_break {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "an inline comment cannot contain a line break; use comment_before or \
             comment_after for a multi-line comment",
        ));
    }
    Ok(text)
}

/// Split a user-supplied comment string into stored lines, one per line. Every
/// YAML line break separates lines (`\r\n`, a lone `\r`, and `\n`), so none can
/// survive inside a line and turn the rest of it into document content. The
/// YAML 1.1-only breaks are rejected rather than guessed at.
pub(crate) fn split_comment_lines(text: Option<String>) -> PyResult<Vec<String>> {
    let Some(text) = text else {
        return Ok(Vec::new());
    };

    if text.contains(YAML_11_LINE_BREAKS) {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "a comment cannot contain NEL, LS, or PS line breaks; use \\n to \
             separate comment lines",
        ));
    }
    Ok(text
        .replace("\r\n", "\n")
        .split(['\n', '\r'])
        .map(str::to_owned)
        .collect())
}

/// Whether the emitter renders a node's foot comment in place: only after a
/// non-empty block mapping or sequence (an empty or flow collection emits inline
/// with no foot position). The document root is handled separately by its caller,
/// since its foot renders for any node kind.
pub(crate) fn renders_foot(node: &YamlNode) -> bool {
    if node.style != NodeStyle::Block {
        return false;
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => !pairs.is_empty(),
        YamlNodeKind::Sequence(items) => !items.is_empty(),
        _ => false,
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
/// Which part of a node's head comments a `comment_before` cursor addresses.
///
/// A sequence item's head block can straddle its `-`: comments above the dash
/// precede the whole entry, while comments below a dash comment (`- # note`) sit
/// inside it, directly above its first key. The two are stored together, so the
/// cursor decides which of them it means.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadScope {
    /// The whole block: a mapping's leading comments, a key node's own, or the
    /// document root's. Nothing divides these.
    All,
    /// The comments above a sequence item's `-`.
    AboveDash,
    /// The comments below that dash's own comment, which are the ones directly
    /// above the first key inside the item.
    BelowDash,
}

/// Resolve the node holding this cursor's "before" comments, and which part of
/// them the cursor addresses.
///
/// Returning both together is deliberate: deciding the scope separately means a
/// second copy of this walk, and the two then disagree about which node a path
/// reached.
fn resolve_head<'a>(roots: &'a [YamlNode], path: &[PathSeg]) -> Option<(&'a YamlNode, HeadScope)> {
    let split = split_document(roots, path)?;
    let root = split.document.first()?;
    match split.inner.split_last() {
        None => Some((root, HeadScope::All)),
        Some((PathSeg::Key(k), parent)) => {
            let parent_node = descend(root, parent)?;
            let YamlNodeKind::Mapping(pairs) = &parent_node.kind else {
                return None;
            };
            // The effective key is its last occurrence (see [`child_ref`]). The
            // first pair's head comment lives on the mapping node (nothing sits
            // above the first key); any later pair carries its own.
            let idx = pairs.iter().rposition(|(key, _)| scalar_eq(key, k))?;
            if idx == 0 {
                Some((parent_node, head_scope_of_parent(parent, parent_node)))
            } else {
                Some((&pairs[idx].0, HeadScope::All))
            }
        }
        // A sequence element holds its own head comment, and owns the block
        // above its dash; a key node holds its own with nothing to divide.
        Some((PathSeg::Index(_), _)) => {
            descend(root, split.inner).map(|node| (node, head_scope_of_item(node)))
        }
        Some((PathSeg::KeyNode(_), _)) => {
            descend(root, split.inner).map(|node| (node, HeadScope::All))
        }
        // A document is only ever selected by the first step, never below it.
        Some((PathSeg::Document { .. }, _)) => None,
    }
}

/// The scope for a first-key cursor, whose head comments live on the mapping
/// itself. When that mapping is a sequence item whose dash carries a comment,
/// they are the lines below that comment; with no dash comment there is nothing
/// dividing the block, and the whole of it sits above the key.
fn head_scope_of_parent(parent: &[PathSeg], parent_node: &YamlNode) -> HeadScope {
    head_scope_of_first_key(
        parent_node,
        matches!(parent.last(), Some(PathSeg::Index(_))),
    )
}

/// The scope for a mapping's first key, given whether that mapping is itself a
/// sequence item. Shared by the path-based cursor and the tree snapshot, which
/// knows the parent while it walks.
pub(crate) fn head_scope_of_first_key(mapping: &YamlNode, is_sequence_item: bool) -> HeadScope {
    if is_sequence_item && mapping.comments.inline_before_value {
        HeadScope::BelowDash
    } else {
        HeadScope::All
    }
}

/// The scope for a cursor addressing a sequence item itself. Without a comment
/// on its dash there is no dividing line, so the item owns its whole block.
pub(crate) fn head_scope_of_item(item: &YamlNode) -> HeadScope {
    if item.comments.inline_before_value {
        HeadScope::AboveDash
    } else {
        HeadScope::All
    }
}

/// Mutable counterpart to [`resolve_head`], reporting the same scope.
fn resolve_head_mut<'a>(
    roots: &'a mut [YamlNode],
    path: &[PathSeg],
) -> Option<(&'a mut YamlNode, HeadScope)> {
    let (root, inner) = split_document_mut(roots, path)?;
    match inner.split_last() {
        None => Some((root, HeadScope::All)),
        Some((PathSeg::Key(k), parent)) => {
            let parent_node = descend_mut(root, parent)?;
            // Mirror [`resolve_head`]: the last occurrence is the effective key,
            // and the first pair's head comment lives on the mapping node.
            let idx = match &parent_node.kind {
                YamlNodeKind::Mapping(pairs) => {
                    pairs.iter().rposition(|(key, _)| scalar_eq(key, k))
                }
                _ => return None,
            }?;
            if idx == 0 {
                let scope = head_scope_of_parent(parent, parent_node);
                Some((parent_node, scope))
            } else if let YamlNodeKind::Mapping(pairs) = &mut parent_node.kind {
                Some((&mut pairs[idx].0, HeadScope::All))
            } else {
                None
            }
        }
        Some((PathSeg::Index(_), _)) => descend_mut(root, inner).map(|node| {
            let scope = head_scope_of_item(node);
            (node, scope)
        }),
        Some((PathSeg::KeyNode(_), _)) => {
            descend_mut(root, inner).map(|node| (node, HeadScope::All))
        }
        Some((PathSeg::Document { .. }, _)) => None,
    }
}

/// Which of `documents` roots a path addresses, and how many of its leading
/// steps select that root.
///
/// This is the one place that decides how a path starts. A single-document
/// object is that document, so its paths start inside it. An object holding
/// several documents is a list of them, so its paths start with the
/// [`PathSeg::Document`] that picks one, taken while the object held exactly
/// that many. A path of any other shape was taken before a document was
/// deleted; it addresses nothing, because the steps after it would now be
/// read inside a different document.
fn document_index(documents: usize, path: &[PathSeg]) -> Option<(usize, usize)> {
    match path.first() {
        Some(&PathSeg::Document {
            index,
            documents: taken_from,
        }) => (documents > 1 && taken_from == documents && index < documents).then_some((index, 1)),
        _ => (documents == 1).then_some((0, 0)),
    }
}

/// The leading steps of a path into the document at `index` of an object
/// holding `documents` roots: nothing for a single document, otherwise the
/// [`PathSeg::Document`] step that selects it. The inverse of [`document_index`].
fn document_prefix(documents: usize, index: usize) -> Vec<PathSeg> {
    if documents > 1 {
        vec![PathSeg::Document { index, documents }]
    } else {
        Vec::new()
    }
}

/// A path split at the document it addresses.
struct DocumentPath<'a, 'p> {
    /// The addressed document's root, as the one-root slice that the anchor
    /// helpers take: anchors and aliases never cross a document boundary.
    document: &'a [YamlNode],
    /// The steps that select the document (empty for a single document).
    prefix: &'p [PathSeg],
    /// The steps below the document's root.
    inner: &'p [PathSeg],
}

impl<'a> DocumentPath<'a, '_> {
    /// The full path to a node at `inner` steps below this document's root.
    fn path_to(&self, inner: Vec<PathSeg>) -> Vec<PathSeg> {
        let mut path = self.prefix.to_vec();
        path.extend(inner);
        path
    }

    /// The node this path addresses, if it still exists.
    fn node(&self) -> Option<&'a YamlNode> {
        descend(self.document.first()?, self.inner)
    }
}

/// Split `path` at the document of `roots` it addresses (see [`document_index`]).
fn split_document<'a, 'p>(
    roots: &'a [YamlNode],
    path: &'p [PathSeg],
) -> Option<DocumentPath<'a, 'p>> {
    let (index, selecting) = document_index(roots.len(), path)?;
    let (prefix, inner) = path.split_at(selecting);
    Some(DocumentPath {
        document: std::slice::from_ref(roots.get(index)?),
        prefix,
        inner,
    })
}

/// Mutable counterpart to [`split_document`]: the addressed document's root and
/// the steps below it.
fn split_document_mut<'a, 'p>(
    roots: &'a mut [YamlNode],
    path: &'p [PathSeg],
) -> Option<(&'a mut YamlNode, &'p [PathSeg])> {
    let (index, selecting) = document_index(roots.len(), path)?;
    Some((roots.get_mut(index)?, &path[selecting..]))
}

/// The node a path addresses, together with the one-root slice of the document
/// it sits in (for the anchor map that resolves its aliases).
fn resolve_in_document<'a>(
    roots: &'a [YamlNode],
    path: &[PathSeg],
) -> Option<(&'a [YamlNode], &'a YamlNode)> {
    let split = split_document(roots, path)?;
    Some((split.document, split.node()?))
}

pub(crate) fn resolve_path<'a>(roots: &'a [YamlNode], path: &[PathSeg]) -> Option<&'a YamlNode> {
    resolve_in_document(roots, path).map(|(_, node)| node)
}

fn resolve_path_mut<'a>(roots: &'a mut [YamlNode], path: &[PathSeg]) -> Option<&'a mut YamlNode> {
    let (root, inner) = split_document_mut(roots, path)?;
    descend_mut(root, inner)
}

/// Follow `steps` down from `node`.
fn descend<'a>(node: &'a YamlNode, steps: &[PathSeg]) -> Option<&'a YamlNode> {
    steps.iter().try_fold(node, child_ref)
}

/// Mutable counterpart to [`descend`].
fn descend_mut<'a>(node: &'a mut YamlNode, steps: &[PathSeg]) -> Option<&'a mut YamlNode> {
    steps.iter().try_fold(node, child_ref_mut)
}

/// Resolve a single path segment to the child node it addresses: a mapping value
/// by key, a mapping key node by key (for anchor/comment access), or a sequence
/// element by index. Returns `None` when the segment does not apply to `node`
/// (an absent key, an out-of-range index, or a scalar that has no children).
pub(crate) fn child_ref<'a>(node: &'a YamlNode, seg: &PathSeg) -> Option<&'a YamlNode> {
    match (&node.kind, seg) {
        // Resolve a key to its *last* occurrence. The decoder collapses a
        // duplicate key to its last value (last-wins), so the round-trip cursor
        // points at that same entry, keeping reads, `locate`, and edits aligned
        // with what `loads()` produced. A normal (unique-key) mapping has exactly
        // one match, so this is identical there; it only differs on duplicates.
        (YamlNodeKind::Mapping(pairs), PathSeg::Key(k)) => pairs
            .iter()
            .rev()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(_, v)| v),
        (YamlNodeKind::Mapping(pairs), PathSeg::KeyNode(k)) => pairs
            .iter()
            .rev()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(key, _)| key),
        (YamlNodeKind::Sequence(items), PathSeg::Index(i)) => items.get(*i),
        _ => None,
    }
}

fn child_ref_mut<'a>(node: &'a mut YamlNode, seg: &PathSeg) -> Option<&'a mut YamlNode> {
    match (&mut node.kind, seg) {
        // Last occurrence wins, as in [`child_ref`].
        (YamlNodeKind::Mapping(pairs), PathSeg::Key(k)) => pairs
            .iter_mut()
            .rev()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(_, v)| v),
        (YamlNodeKind::Mapping(pairs), PathSeg::KeyNode(k)) => pairs
            .iter_mut()
            .rev()
            .find(|(key, _)| scalar_eq(key, k))
            .map(|(key, _)| key),
        (YamlNodeKind::Sequence(items), PathSeg::Index(i)) => items.get_mut(*i),
        _ => None,
    }
}

pub(crate) fn scalar_eq(key: &YamlNode, name: &str) -> bool {
    matches!(&key.kind, YamlNodeKind::Scalar(s, _) if s == name)
}

// -- Child access shared by YAMLRocksDocument and YAMLRocksDocumentView --
//
// Reading, writing, or deleting `container[key]` can run Python code: the key's
// `__eq__` while it is compared with the mapping's keys, its `__index__` to
// become a sequence position, and, for a write, whatever converting the value
// runs. That code is free to read or even change this same document, so none of
// it runs while the document is borrowed. An operation reads what it needs under
// a short shared borrow, runs the Python code with no borrow held, and finishes
// under one more borrow. A write checks the document's revision there and
// refuses to touch a tree that was written to in between, because the position
// it found may now hold something else.
//
// A mapping key is matched by the value it *reads as* under the schema, so
// `doc[True]` reaches a `yes:` entry under YAML 1.1 (and `doc['yes']` does not),
// consistent with `keys()`/`to_dict()` and with Python dict semantics (`True ==
// 1`). Scalar and null keys are matched this way; a collection key is not. A
// null key can be replaced and deleted, but not read, since a path step names a
// key by its text. On a duplicate key the last occurrence wins (see
// [`child_ref`]), the entry `loads()` keeps.

/// How a key is matched against the keys of one mapping.
enum KeyLookup<'k> {
    /// An exact `str` key, by far the most common. It equals a mapping key
    /// exactly when that key reads as this same string, which is decided in
    /// Rust with no Python code running (see [`key_reads_as`]).
    Text(&'k str),
    /// Any other key, compared with Python `==` against each scalar or null
    /// key as it reads. Holds each such key's position and value, taken under
    /// the borrow so the comparisons can run after it is released. Building
    /// this costs one Python object per key on every call, which only keys
    /// that are not an exact `str` pay.
    Object(Vec<(usize, Py<PyAny>)>),
}

impl<'k> KeyLookup<'k> {
    /// Prepare to match `key` against `pairs` of a document read under
    /// `schema`.
    fn new(
        py: Python<'_>,
        key: &'k Bound<'_, PyAny>,
        pairs: &[(YamlNode, YamlNode)],
        schema: Schema,
        resolve_timestamps: bool,
    ) -> Self {
        if let Some(text) = exact_text(key) {
            return Self::Text(text);
        }
        // A scalar key never resolves an alias, so no anchor map is needed.
        let no_anchors = HashMap::new();
        let keys = pairs
            .iter()
            .enumerate()
            // A null key (`~: x`, or the key `doc[None] = x` adds) is matched
            // too, so assigning or deleting it again finds the pair it made.
            .filter(|(_, (key_node, _))| {
                matches!(key_node.kind, YamlNodeKind::Scalar(..) | YamlNodeKind::Null)
            })
            .map(|(position, (key_node, _))| {
                let value =
                    node_to_python_with(py, key_node, schema, resolve_timestamps, &no_anchors);
                (position, value)
            })
            .collect();
        Self::Object(keys)
    }
}

/// The text of `key` when it is an exact `str`, the one key type matched
/// without calling into Python. A subclass may define its own `__eq__`, and a
/// string with lone surrogates is not valid UTF-8, so both take the general
/// path instead.
fn exact_text<'k>(key: &'k Bound<'_, PyAny>) -> Option<&'k str> {
    key.cast_exact::<PyString>().ok()?.to_str().ok()
}

/// Whether turning `key` into a path step ([`seg_from_key`]) runs no Python
/// code of its own, as for an exact `str` or `int`.
fn converts_without_python(key: &Bound<'_, PyAny>) -> bool {
    key.is_exact_instance_of::<PyString>() || key.is_exact_instance_of::<PyInt>()
}

/// Whether the mapping key `key_node` reads as the string `text` in a document
/// read under `schema`: the Rust counterpart of comparing `to_dict()`'s key
/// with `==`, for an exact `str`.
fn key_reads_as(key_node: &YamlNode, text: &str, schema: Schema, resolve_timestamps: bool) -> bool {
    let YamlNodeKind::Scalar(lexeme, style) = &key_node.kind else {
        return false;
    };
    let tag = key_node.tag.as_deref();
    // A timestamp reads as a `date` or `datetime`, which never equals a `str`.
    // The parser only accepts what `datetime` can represent, so this agrees
    // with the conversion `to_dict()` does.
    if resolve_timestamps
        && *style == ScalarStyle::Plain
        && tag.is_none()
        && crate::resolver::timestamp::parse(lexeme).is_some()
    {
        return false;
    }
    match schema.classify(lexeme, *style, tag) {
        ScalarKind::Str => lexeme == text,
        ScalarKind::Merge => text == "<<",
        _ => false,
    }
}

/// The position of the last key in `keys` that `== key`, running each
/// comparison once, last key first, until one matches. A comparison that
/// raises is raised: treating it as "no match" would let a write add a second
/// copy of a key that is already there.
fn last_equal(
    py: Python<'_>,
    keys: &[(usize, Py<PyAny>)],
    key: &Bound<'_, PyAny>,
) -> PyResult<Option<usize>> {
    for (position, candidate) in keys.iter().rev() {
        if candidate.bind(py).eq(key)? {
            return Ok(Some(*position));
        }
    }
    Ok(None)
}

/// The positions of every key in `keys` that `== key`, in order. A comparison
/// that raises is raised, before anything is removed.
fn all_equal(
    py: Python<'_>,
    keys: &[(usize, Py<PyAny>)],
    key: &Bound<'_, PyAny>,
) -> PyResult<Vec<usize>> {
    let mut positions = Vec::new();
    for (position, candidate) in keys {
        if candidate.bind(py).eq(key)? {
            positions.push(*position);
        }
    }
    Ok(positions)
}

/// The step from a mapping to the value of its pair at `position`.
fn key_step(pairs: &[(YamlNode, YamlNode)], position: usize) -> Option<PathSeg> {
    match &pairs.get(position)?.0.kind {
        YamlNodeKind::Scalar(lexeme, _) => Some(PathSeg::Key(lexeme.clone())),
        _ => None,
    }
}

/// Why a lookup returned nothing.
enum LookupFailure {
    /// The key is not there, or cannot be a key at all. `get()` returns its
    /// default for this, as `dict.get` does.
    Missing(PyErr),
    /// The lookup itself failed: comparing the key raised, or the document was
    /// in use or changed meanwhile. `get()` raises this too, since a default
    /// would pass off "could not look" as "not there".
    Failed(PyErr),
}

impl From<PyErr> for LookupFailure {
    fn from(err: PyErr) -> Self {
        Self::Failed(err)
    }
}

impl From<LookupFailure> for PyErr {
    fn from(failure: LookupFailure) -> Self {
        match failure {
            LookupFailure::Missing(err) | LookupFailure::Failed(err) => err,
        }
    }
}

impl LookupFailure {
    /// What `get()` returns for this failure: `default` for a missing key, the
    /// error itself for a lookup that failed.
    fn or_default(self, py: Python<'_>, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self {
            Self::Missing(_) => Ok(default.unwrap_or_else(|| py.None())),
            Self::Failed(err) => Err(err),
        }
    }
}

/// Read `container[key]` for the container at `path` of `root`: a scalar
/// resolves to plain Python, a mapping or sequence to a [`YAMLRocksDocumentView`]
/// one step further down.
fn get_child(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    path: &[PathSeg],
    key: &Bound<'_, PyAny>,
    stale: fn() -> PyErr,
) -> Result<Py<PyAny>, LookupFailure> {
    let missing = LookupFailure::Missing;
    let doc = read_document(root)?;
    let node = resolve_path(&doc.nodes, path).ok_or_else(|| missing(stale()))?;

    let (doc, step) = match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            match KeyLookup::new(py, key, pairs, doc.schema, doc.resolve_timestamps) {
                KeyLookup::Text(text) => {
                    let position = pairs.iter().rposition(|(key_node, _)| {
                        key_reads_as(key_node, text, doc.schema, doc.resolve_timestamps)
                    });
                    let step = position.and_then(|position| key_step(pairs, position));
                    (doc, step)
                }
                KeyLookup::Object(keys) => {
                    let revision = doc.revision;
                    drop(doc);
                    let position = last_equal(py, &keys, key)?;
                    // The position is only meaningful in the tree it was found in.
                    let doc = read_unchanged(root, revision)?;
                    let step =
                        position.and_then(|position| match &resolve_path(&doc.nodes, path)?.kind {
                            YamlNodeKind::Mapping(pairs) => key_step(pairs, position),
                            _ => None,
                        });
                    (doc, step)
                }
            }
        }
        _ if converts_without_python(key) => {
            let step = seg_from_key(key).map_err(missing)?;
            (doc, Some(step))
        }
        _ => {
            // The key's `__index__` runs with no borrow held. A position is
            // looked up afresh afterwards, like any read.
            drop(doc);
            let step = seg_from_key(key).map_err(missing)?;
            (read_document(root)?, Some(step))
        }
    };

    let (document, node) = resolve_in_document(&doc.nodes, path).ok_or_else(|| missing(stale()))?;
    let found = step.and_then(|step| Some((child_ref(node, &step)?, step)));
    let Some((child, step)) = found else {
        let in_sequence = matches!(node.kind, YamlNodeKind::Sequence(_));
        drop(doc);
        return Err(missing(missing_child(in_sequence, key)));
    };

    let anchors = build_anchor_map(document);
    let mut child_path = path.to_vec();
    child_path.push(step);
    Ok(wrap_node(
        py,
        root,
        child_path,
        child,
        &anchors,
        doc.schema,
        doc.resolve_timestamps,
    )?)
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
    resolve_timestamps: bool,
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
            Some(target) => wrap_node(py, root, path, target, anchors, schema, resolve_timestamps),
            None => Ok(py.None()),
        },
        _ => Ok(node_to_python_with(
            py,
            node,
            schema,
            resolve_timestamps,
            anchors,
        )),
    }
}

/// What a write or delete of `container[key]` found under its first, shared
/// borrow: the document's settings and revision, and what the container is.
struct Inspection<'k> {
    revision: u64,
    target: ScalarTarget,
    container: Container<'k>,
}

/// The container a write or delete addresses.
enum Container<'k> {
    Mapping(KeyLookup<'k>),
    Sequence {
        len: usize,
    },
    /// A scalar, null, or alias: nothing to write into or delete from.
    Leaf,
}

/// Take the first, shared look at the container at `path` of `root`. Runs no
/// Python code of the caller's.
fn inspect<'k>(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    path: &[PathSeg],
    key: &'k Bound<'_, PyAny>,
    stale: fn() -> PyErr,
) -> PyResult<Inspection<'k>> {
    let doc = read_document(root)?;
    let node = resolve_path(&doc.nodes, path).ok_or_else(stale)?;
    let container = match &node.kind {
        YamlNodeKind::Mapping(pairs) => Container::Mapping(KeyLookup::new(
            py,
            key,
            pairs,
            doc.schema,
            doc.resolve_timestamps,
        )),
        YamlNodeKind::Sequence(items) => Container::Sequence { len: items.len() },
        _ => Container::Leaf,
    };
    Ok(Inspection {
        revision: doc.revision,
        target: doc.scalar_target(),
        container,
    })
}

/// Where a write into a mapping lands.
enum MappingSlot<'k> {
    /// An exact `str` key, still to be matched under the write borrow.
    Text(&'k str),
    /// The value of the existing pair at this position.
    Replace(usize),
    /// A new pair with this key node.
    Add(Box<YamlNode>),
}

/// Replace the value of the pair at `position`, which the caller found in this
/// same tree.
fn replace_entry(
    pairs: &mut [(YamlNode, YamlNode)],
    position: usize,
    new_val: YamlNode,
    stale: fn() -> PyErr,
) -> PyResult<()> {
    let (_, target) = pairs.get_mut(position).ok_or_else(stale)?;
    replace_keeping_metadata(target, new_val);
    Ok(())
}

/// Write `container[key] = value` for the container at `path` of `root`.
///
/// The key and value are converted, each exactly once, with no borrow held.
/// The write then happens under one write borrow that first checks the
/// document's revision, so the slot found earlier is still the same slot.
fn set_child(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    path: &[PathSeg],
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
    stale: fn() -> PyErr,
) -> PyResult<()> {
    let Inspection {
        revision,
        target,
        container,
    } = inspect(py, root, path, key, stale)?;

    match container {
        Container::Mapping(lookup) => {
            let new_val = python_to_node(py, value, target)?;
            let slot = match lookup {
                KeyLookup::Text(text) => MappingSlot::Text(text),
                KeyLookup::Object(keys) => match last_equal(py, &keys, key)? {
                    Some(position) => MappingSlot::Replace(position),
                    // No key matched: add one, building the key node from the
                    // Python key so a non-string key (`doc[True] = ...`) emits
                    // correctly.
                    None => MappingSlot::Add(Box::new(python_to_node(py, key, target)?)),
                },
            };

            let mut doc = write_unchanged(root, revision)?;
            let pairs = match resolve_path_mut(&mut doc.nodes, path).map(|node| &mut node.kind) {
                Some(YamlNodeKind::Mapping(pairs)) => pairs,
                _ => return Err(stale()),
            };
            match slot {
                MappingSlot::Replace(position) => replace_entry(pairs, position, new_val, stale),
                MappingSlot::Add(new_key) => {
                    pairs.push((marked_modified(*new_key), marked_modified(new_val)));
                    Ok(())
                }
                MappingSlot::Text(text) => match pairs.iter().rposition(|(key_node, _)| {
                    key_reads_as(key_node, text, target.schema, target.resolve_timestamps)
                }) {
                    Some(position) => replace_entry(pairs, position, new_val, stale),
                    None => {
                        // A string that needs quoting under the schema is
                        // quoted, so the key reads back as the string it is.
                        let new_key =
                            assigned_scalar(ResolvedValue::String(text.to_owned()), target);
                        pairs.push((marked_modified(new_key), marked_modified(new_val)));
                        Ok(())
                    }
                },
            }
        }
        Container::Sequence { len } => {
            let index: usize = key.extract()?;
            if index >= len {
                return Err(index_out_of_range());
            }
            let new_val = python_to_node(py, value, target)?;

            let mut doc = write_unchanged(root, revision)?;
            let item = match resolve_path_mut(&mut doc.nodes, path).map(|node| &mut node.kind) {
                Some(YamlNodeKind::Sequence(items)) => items.get_mut(index),
                _ => None,
            };
            replace_keeping_metadata(item.ok_or_else(stale)?, new_val);
            Ok(())
        }
        Container::Leaf => Err(not_a_container()),
    }
}

/// `node`, marked as modified so the document re-emits it.
fn marked_modified(mut node: YamlNode) -> YamlNode {
    node.mark_modified();
    node
}

/// Replace `target` with `new_val`, keeping the metadata attached to the
/// position rather than to the value: its comments, anchor, and tag. An edit
/// then does not silently drop nearby comments or markup.
fn replace_keeping_metadata(target: &mut YamlNode, mut new_val: YamlNode) {
    new_val.comments.head = std::mem::take(&mut target.comments.head);
    new_val.comments.inline = target.comments.inline.take();
    // Keep the alignment padding around the value so an edit preserves the
    // author's layout: the gap between the key's `:` and the value, and the
    // run of spaces before an inline `#`.
    new_val.comments.value_pad = target.comments.value_pad;
    new_val.comments.inline_spaces = target.comments.inline_spaces;
    // Where that comment sits travels with it. Dropping this would move a
    // `key: # note` comment after the new value, and for a sequence item
    // would strand the head comments written below its dash.
    new_val.comments.inline_before_value = target.comments.inline_before_value;
    new_val.comments.foot = std::mem::take(&mut target.comments.foot);
    new_val.anchor = target.anchor.take();
    new_val.tag = target.tag.take();
    // Set only on a document root: the empty documents around it belong to
    // the stream, so replacing the root's value keeps them.
    new_val.empty_documents = target.empty_documents.take();
    new_val.mark_modified();
    *target = new_val;
}

/// The error for writing into, or deleting from, a scalar.
fn not_a_container() -> PyErr {
    pyo3::exceptions::PyTypeError::new_err("node is not a mapping or sequence")
}

/// The error for a sequence position past its end.
fn index_out_of_range() -> PyErr {
    pyo3::exceptions::PyIndexError::new_err("index out of range")
}

/// Collect every `*alias` name referenced anywhere in a node subtree.
fn collect_alias_names(node: &YamlNode, out: &mut std::collections::HashSet<String>) {
    match &node.kind {
        YamlNodeKind::Alias(name) => {
            out.insert(name.clone());
        }
        YamlNodeKind::Mapping(pairs) => {
            for (k, v) in pairs {
                collect_alias_names(k, out);
                collect_alias_names(v, out);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_alias_names(item, out);
            }
        }
        _ => {}
    }
}

/// Whether the document references any alias at all. A document with no aliases
/// can never be left with a dangling one, so the delete guard skips the (cloning)
/// safety check entirely, keeping the common no-anchor case cheap.
fn document_has_aliases(nodes: &[YamlNode]) -> bool {
    let mut aliases = std::collections::HashSet::new();
    for node in nodes {
        collect_alias_names(node, &mut aliases);
    }
    !aliases.is_empty()
}

/// The set of `*alias` names that no longer resolve to an `&anchor` anywhere in
/// the document. The delete guard compares this set before and after a delete so
/// only a *newly* orphaned alias blocks the operation: the round-trip/compose
/// path does not validate alias targets at load time, so a document may already
/// carry a dangling alias, and an unrelated delete must not be blamed for it.
fn dangling_alias_names(nodes: &[YamlNode]) -> std::collections::HashSet<String> {
    let mut anchors = HashMap::new();
    for node in nodes {
        collect_anchor_refs(node, &mut anchors);
    }
    let mut aliases = std::collections::HashSet::new();
    for node in nodes {
        collect_alias_names(node, &mut aliases);
    }
    aliases.retain(|name| !anchors.contains_key(name));
    aliases
}

/// The error raised when a delete would orphan an alias (leave a `*name` with no
/// `&name`), which would emit YAML that no longer parses.
fn dangling_alias_error(name: &str) -> PyErr {
    pyo3::exceptions::PyValueError::new_err(format!(
        "deleting this node would remove anchor '&{name}', still referenced by an \
         alias '*{name}' elsewhere; the result would not re-parse. Remove or \
         rewrite the alias first."
    ))
}

/// What a delete removes, decided before the write borrow.
enum Removal<'k> {
    /// Every pair whose key reads as this exact `str`, matched under the write
    /// borrow.
    Text(&'k str),
    /// The pairs at these positions.
    Pairs(Vec<usize>),
    /// The sequence item at this index.
    Item(usize),
}

/// Delete `container[key]` for the container at `path` of `root`, with the
/// same two steps as [`set_child`]: the key is matched or converted with no
/// borrow held, then the removal happens under one write borrow that checks the
/// document's revision first.
fn del_child(
    py: Python<'_>,
    root: &Bound<'_, YAMLRocksDocument>,
    path: &[PathSeg],
    key: &Bound<'_, PyAny>,
    stale: fn() -> PyErr,
) -> PyResult<()> {
    let Inspection {
        revision,
        target,
        container,
    } = inspect(py, root, path, key, stale)?;
    // Removing a key removes it from the logical mapping, so every duplicate
    // occurrence goes (otherwise a shadowed one would re-surface as the new
    // effective value).
    let removal = match container {
        Container::Mapping(KeyLookup::Text(text)) => Removal::Text(text),
        Container::Mapping(KeyLookup::Object(keys)) => Removal::Pairs(all_equal(py, &keys, key)?),
        Container::Sequence { .. } => Removal::Item(key.extract()?),
        Container::Leaf => return Err(not_a_container()),
    };

    let mut doc = write_unchanged(root, revision)?;
    let (document_root, inner) = split_document_mut(&mut doc.nodes, path).ok_or_else(stale)?;
    // Guard against orphaning an alias: if the document has aliases, snapshot
    // the tree and the aliases already dangling, delete, then roll back if the
    // delete leaves a *newly* dangling `*alias`. Aliases never reach across
    // documents, so this document is the whole of what to check.
    let snapshot = document_has_aliases(std::slice::from_ref(document_root)).then(|| {
        let before = dangling_alias_names(std::slice::from_ref(document_root));
        (document_root.clone(), before)
    });
    let node = descend_mut(document_root, inner).ok_or_else(stale)?;
    match (&mut node.kind, removal) {
        (YamlNodeKind::Mapping(pairs), Removal::Text(text)) => {
            let before = pairs.len();
            pairs.retain(|(key_node, _)| {
                !key_reads_as(key_node, text, target.schema, target.resolve_timestamps)
            });
            if pairs.len() == before {
                return Err(pyo3::exceptions::PyKeyError::new_err(key.clone().unbind()));
            }
        }
        (YamlNodeKind::Mapping(pairs), Removal::Pairs(positions)) => {
            if positions.is_empty() {
                return Err(pyo3::exceptions::PyKeyError::new_err(key.clone().unbind()));
            }
            // One pass over the pairs, however many duplicates go. The removed
            // pairs carry their own comments away; the surrounding pairs keep
            // theirs.
            let mut doomed = positions.into_iter().peekable();
            let mut position = 0;
            pairs.retain(|_| {
                let removed = doomed.next_if_eq(&position).is_some();
                position += 1;
                !removed
            });
        }
        (YamlNodeKind::Sequence(items), Removal::Item(index)) => {
            if index >= items.len() {
                return Err(index_out_of_range());
            }
            items.remove(index);
        }
        _ => return Err(stale()),
    }
    // Mark the container modified so the document re-emits from the AST instead of
    // replaying its original source bytes verbatim (those still contain the removed
    // entry, see `to_yaml`/`nodes_modified`). Re-emission walks the pairs/items and
    // emits each surviving node's own comments, so the rest stays comment-preserving.
    node.mark_modified();

    if let Some((snapshot, before)) = snapshot {
        let after = dangling_alias_names(std::slice::from_ref(document_root));
        if let Some(name) = after.difference(&before).next() {
            let name = name.clone();
            *document_root = snapshot;
            return Err(dangling_alias_error(&name));
        }
    }
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
    resolve_timestamps: bool,
) -> PyResult<Py<PyAny>> {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            Ok(
                distinct_keys(py, pairs, schema, anchors, resolve_timestamps)?
                    .into_any()
                    .unbind(),
            )
        }
        _ => Err(pyo3::exceptions::PyTypeError::new_err(
            "node is not a mapping",
        )),
    }
}

/// The distinct keys of a mapping's `pairs`, in first-appearance order, collapsed
/// by their *resolved* value exactly as `to_dict` and `loads` collapse a
/// duplicate key (last-wins). Deduping on the resolved value, not the raw lexeme,
/// keeps `keys()`/`len()` consistent with `to_dict()` even when two keys are
/// spelled differently but resolve equal (`yes`/`true` under 1.1, `1`/`0x1`). The
/// kept key is returned in its ordinary (annotated) Python form.
fn distinct_keys<'py>(
    py: Python<'py>,
    pairs: &[(YamlNode, YamlNode)],
    schema: Schema,
    anchors: &HashMap<String, YamlNode>,
    resolve_timestamps: bool,
) -> PyResult<Bound<'py, PyList>> {
    let list = PyList::empty(py);
    // The hashable resolved keys already emitted (`node_to_python_key` is the same
    // conversion `to_dict` keys its dict by, so a collection key is a tuple here).
    let seen = PyDict::new(py);
    let mut cache: ObjectCache = HashMap::new();
    for (k, _) in pairs {
        let resolved = node_to_python_key(py, k, schema, resolve_timestamps, anchors, &mut cache);
        let resolved = resolved.bind(py);
        if !seen.contains(resolved)? {
            seen.set_item(resolved, py.None())?;
            list.append(node_to_python_with(
                py,
                k,
                schema,
                resolve_timestamps,
                anchors,
            ))?;
        }
    }
    Ok(list)
}

fn container_len(
    py: Python<'_>,
    node: &YamlNode,
    schema: Schema,
    anchors: &HashMap<String, YamlNode>,
    resolve_timestamps: bool,
) -> PyResult<usize> {
    Ok(match &node.kind {
        // Count distinct keys, matching the last-wins logical mapping (so
        // `len(doc) == len(doc.keys())` even with a duplicate key), by the same
        // resolved-key rule as `distinct_keys`/`to_dict`.
        YamlNodeKind::Mapping(pairs) => {
            distinct_keys(py, pairs, schema, anchors, resolve_timestamps)?.len()
        }
        YamlNodeKind::Sequence(items) => items.len(),
        _ => 1,
    })
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

/// The error for a child that is not there: an `IndexError` in a sequence, a
/// `KeyError` naming the key anywhere else. Formatting the key runs its
/// `__str__`, so callers build this after releasing the document.
fn missing_child(in_sequence: bool, key: &Bound<'_, PyAny>) -> PyErr {
    if in_sequence {
        index_out_of_range()
    } else {
        pyo3::exceptions::PyKeyError::new_err(format!("{key}"))
    }
}

/// `doc[key]` on a whole round-trip object: a child of its only document, or
/// one of the documents of a stream.
fn lookup_item(
    slf: &Bound<'_, YAMLRocksDocument>,
    key: &Bound<'_, PyAny>,
) -> Result<Py<PyAny>, LookupFailure> {
    let py = slf.py();
    if read_document(slf)?.nodes.len() == 1 {
        return get_child(py, slf, &[], key, stale_view);
    }

    // The index is converted with no borrow held: an `__index__` method is
    // free to read this document.
    let idx: usize = key.extract().map_err(|_| {
        LookupFailure::Missing(pyo3::exceptions::PyKeyError::new_err(
            "multi-document access requires an integer index",
        ))
    })?;
    let this = read_document(slf)?;
    let node = this
        .nodes
        .get(idx)
        .ok_or_else(|| LookupFailure::Missing(index_out_of_range()))?;
    // Anchors do not cross document boundaries, so a document's
    // aliases resolve against its own anchors only.
    let anchors = build_anchor_map(std::slice::from_ref(node));
    Ok(wrap_node(
        py,
        slf,
        document_prefix(this.nodes.len(), idx),
        node,
        &anchors,
        this.schema,
        this.resolve_timestamps,
    )?)
}

/// Write `bytes` to `path`, surfacing IO errors as Python `OSError`.
///
/// Uses an atomic, symlink-safe write so a source file swapped for a symlink
/// between load and `save` cannot redirect the write outside the config tree.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> PyResult<()> {
    crate::safe_io::atomic_write(path, bytes).map_err(|e| {
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
    resolve_timestamps: bool,
) -> PyResult<()> {
    // Grow the native stack on demand so walking a deeply nested document cannot
    // overflow a small thread stack. See [`crate::stack`].
    crate::stack::guard(|| {
        collect_leaves_inner(py, node, path, out, anchors, schema, resolve_timestamps)
    })
}

fn collect_leaves_inner(
    py: Python<'_>,
    node: &YamlNode,
    path: &mut Vec<Py<PyAny>>,
    out: &mut Vec<(Vec<Py<PyAny>>, Py<PyAny>)>,
    anchors: &HashMap<String, YamlNode>,
    schema: Schema,
    resolve_timestamps: bool,
) -> PyResult<()> {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                if let YamlNodeKind::Scalar(_, _) = &key.kind {
                    // Resolve the path key under the schema so `walk()` paths match
                    // `keys()` and indexed access (a `yes:` key is `True` under 1.1).
                    path.push(node_to_python_with(
                        py,
                        key,
                        schema,
                        resolve_timestamps,
                        anchors,
                    ));
                    collect_leaves(py, val, path, out, anchors, schema, resolve_timestamps)?;
                    path.pop();
                }
            }
        }
        YamlNodeKind::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(i.into_pyobject(py)?.into_any().unbind());
                collect_leaves(py, item, path, out, anchors, schema, resolve_timestamps)?;
                path.pop();
            }
        }
        _ => out.push((
            path.clone(),
            node_to_python_with(py, node, schema, resolve_timestamps, anchors),
        )),
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
    use super::{
        document_index, document_prefix, keep_empty_documents, key_reads_as, resolve_head,
        resolve_head_mut, resolve_path, resolve_path_mut, stamp_yaml_1_2, HeadScope, PathSeg,
    };
    use crate::encode::NullStyle;
    use crate::resolver::Schema;
    use crate::roundtrip::ast::{YamlNode, YamlNodeKind};
    use crate::roundtrip::composer::compose;
    use crate::roundtrip::emit::emit_roundtrip_all_with;

    /// Remove root `idx` the way `del doc[idx]` does and re-emit the stream.
    fn delete_root(input: &str, idx: usize) -> String {
        let mut nodes = compose(input).unwrap();
        let removed = nodes.remove(idx);
        if let Some(empty_documents) = removed.empty_documents {
            keep_empty_documents(&mut nodes, idx, *empty_documents);
        }
        String::from_utf8(emit_roundtrip_all_with(&nodes, NullStyle::Null)).unwrap()
    }

    #[test]
    fn deleting_a_root_keeps_the_empty_documents_before_it() {
        assert_eq!(
            delete_root("a: 1\n---\n# x\n---\nb: 2\n---\nc: 3\n", 1),
            "a: 1\n---\n# x\n---\nc: 3\n"
        );
    }

    #[test]
    fn deleting_the_last_root_keeps_the_empty_documents_around_it() {
        assert_eq!(
            delete_root("a: 1\n---\n# x\n---\nb: 2\n--- # y\n", 1),
            "a: 1\n---\n# x\n--- # y\n"
        );
    }

    #[test]
    fn deleting_a_root_closes_its_empty_documents_before_a_directive() {
        // The empty document now sits right before a root with a `%YAML`
        // directive, which is only valid after a `...`.
        assert_eq!(
            delete_root("---\n---\nb: 1\n...\n%YAML 1.2\n---\nc: 2\n", 0),
            "---\n...\n%YAML 1.2\n---\nc: 2\n"
        );
    }

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
    fn finds_the_marker_past_leading_comments() {
        assert_eq!(
            stamp(b"# head\n\n---\n---\na: 1\n"),
            b"%YAML 1.2\n# head\n\n---\n---\na: 1\n"
        );
        assert_eq!(stamp(b"# head\na: 1\n"), b"%YAML 1.2\n---\n# head\na: 1\n");
    }

    #[test]
    fn finds_directives_past_leading_comments() {
        assert_eq!(
            stamp(b"# head\n%TAG !e! tag:x:\n---\na: 1\n"),
            b"%YAML 1.2\n# head\n%TAG !e! tag:x:\n---\na: 1\n"
        );
        assert_eq!(
            stamp(b"%TAG !e! tag:x:\n# head\n---\na: 1\n"),
            b"%YAML 1.2\n%TAG !e! tag:x:\n# head\n---\na: 1\n"
        );
        assert_eq!(
            stamp(b"# head\n%YAML 1.1\n---\na: 1\n"),
            b"%YAML 1.2\n# head\n---\na: 1\n"
        );
        assert_eq!(
            stamp(b"# head\r\n%YAML 1.2\r\n---\r\na: 1\r\n"),
            b"%YAML 1.2\n# head\r\n---\r\na: 1\r\n"
        );
    }

    #[test]
    fn does_not_skip_past_a_lone_carriage_return() {
        // A lone `\r` ends the comment line; the `---` after it is the marker.
        assert_eq!(
            stamp(b"# head\r---\ra: 1\r"),
            b"%YAML 1.2\n# head\r---\ra: 1\r"
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

    fn key(name: &str) -> PathSeg {
        PathSeg::Key(name.to_owned())
    }

    /// The text of the scalar a path resolves to, if it resolves to one.
    fn scalar_at(roots: &[YamlNode], path: &[PathSeg]) -> Option<String> {
        match &resolve_path(roots, path)?.kind {
            YamlNodeKind::Scalar(text, _) => Some(text.clone()),
            _ => None,
        }
    }

    #[test]
    fn a_single_document_path_starts_inside_the_document() {
        assert_eq!(document_index(1, &[]), Some((0, 0)));
        assert_eq!(document_index(1, &[key("a")]), Some((0, 0)));
        assert_eq!(document_index(1, &[PathSeg::Index(3)]), Some((0, 0)));
    }

    #[test]
    fn a_stream_path_starts_with_the_document_it_selects() {
        let second = PathSeg::Document {
            index: 1,
            documents: 2,
        };
        assert_eq!(document_index(2, &[second.clone(), key("b")]), Some((1, 1)));
        assert_eq!(document_prefix(2, 1), vec![second]);
        assert_eq!(document_prefix(1, 0), Vec::<PathSeg>::new());
    }

    #[test]
    fn a_stream_path_without_a_document_step_addresses_nothing() {
        assert_eq!(document_index(2, &[]), None);
        assert_eq!(document_index(2, &[PathSeg::Index(1)]), None);
        assert_eq!(document_index(0, &[]), None);
    }

    #[test]
    fn a_path_taken_before_a_document_was_deleted_addresses_nothing() {
        let taken_from_three = PathSeg::Document {
            index: 0,
            documents: 3,
        };
        // Two documents left: index 0 now holds what was the second one.
        assert_eq!(
            document_index(2, std::slice::from_ref(&taken_from_three)),
            None
        );
        // One document left: the steps would otherwise be read inside it.
        assert_eq!(document_index(1, &[taken_from_three]), None);
        let out_of_range = PathSeg::Document {
            index: 2,
            documents: 2,
        };
        assert_eq!(document_index(2, &[out_of_range]), None);
    }

    #[test]
    fn resolves_a_path_inside_the_selected_document() {
        let roots = compose("a: 1\n---\nb: [x, y]\n").unwrap();
        let second = document_prefix(roots.len(), 1);
        let mut path = second.clone();
        path.extend([key("b"), PathSeg::Index(1)]);
        assert_eq!(scalar_at(&roots, &path).as_deref(), Some("y"));
        let mut first = document_prefix(roots.len(), 0);
        first.push(key("a"));
        assert_eq!(scalar_at(&roots, &first).as_deref(), Some("1"));
        // `b` exists only in the second document.
        let mut wrong_document = document_prefix(roots.len(), 0);
        wrong_document.push(key("b"));
        assert!(resolve_path(&roots, &wrong_document).is_none());
        assert!(std::ptr::eq(
            resolve_path(&roots, &second).unwrap(),
            &roots[1]
        ));
    }

    #[test]
    fn a_stream_path_goes_stale_once_a_document_is_deleted() {
        let mut roots = compose("- a\n- b\n---\n- p\n- q\n").unwrap();
        let mut path = document_prefix(roots.len(), 1);
        path.push(PathSeg::Index(1));
        assert_eq!(scalar_at(&roots, &path).as_deref(), Some("q"));
        roots.remove(0);
        // Without the count, `[1]` would now read `b` inside the survivor.
        assert!(resolve_path(&roots, &path).is_none());
        assert!(resolve_path_mut(&mut roots, &path).is_none());
    }

    #[test]
    fn a_key_reads_as_a_string_only_where_the_document_reads_it_as_one() {
        let roots = compose("yes: 1\n'yes': 2\n1: 3\n2024-01-02: 4\n<<: {}\n!!str 5: 6\n").unwrap();
        let YamlNodeKind::Mapping(pairs) = &roots[0].kind else {
            panic!("the document is a mapping");
        };
        let reads = |position: usize, text: &str, schema: Schema, timestamps: bool| {
            key_reads_as(&pairs[position].0, text, schema, timestamps)
        };

        assert!(reads(0, "yes", Schema::Yaml12, false));
        assert!(!reads(0, "yes", Schema::Yaml11, false));
        assert!(reads(1, "yes", Schema::Yaml11, false));
        assert!(!reads(2, "1", Schema::Yaml12, false));
        assert!(reads(3, "2024-01-02", Schema::Yaml12, false));
        assert!(!reads(3, "2024-01-02", Schema::Yaml12, true));
        assert!(reads(4, "<<", Schema::Yaml12, false));
        assert!(reads(5, "5", Schema::Yaml12, false));
        assert!(!reads(5, "6", Schema::Yaml12, false));
    }

    #[test]
    fn resolves_the_head_comment_of_a_stream_document() {
        let mut roots = compose("a: 1\n---\n# above b\nb: 2\nc: 3\n").unwrap();
        let second = document_prefix(roots.len(), 1);
        let (root, scope) = resolve_head(&roots, &second).unwrap();
        assert!(std::ptr::eq(root, &roots[1]));
        assert!(scope == HeadScope::All);
        let mut first_key = second.clone();
        first_key.push(key("b"));
        let (holder, _) = resolve_head(&roots, &first_key).unwrap();
        assert!(std::ptr::eq(holder, &roots[1]));
        let mut later_key = second;
        later_key.push(key("c"));
        assert!(resolve_head_mut(&mut roots, &later_key).is_some());
    }
}
