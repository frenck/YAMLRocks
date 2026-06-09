//! Python-facing value types exposed by the module: [`YAMLRocksTag`] for
//! custom-tagged values, and the [`YAMLRocksAnnotatedDict`]/[`YAMLRocksAnnotatedList`]
//! subclasses that carry source locations in annotated mode.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::roundtrip::ast::is_include_tag;

/// The pickled source-annotation state shared by `YAMLRocksAnnotatedDict` and
/// `YAMLRocksAnnotatedList`: `(line, column, file, end_line, end_column,
/// source_tag, source_target, offset, end_offset)`. Factored into an alias so
/// the `__setstate__` signature stays legible.
type AnnotationState = (
    u32,
    u32,
    Option<String>,
    u32,
    u32,
    Option<String>,
    Option<String>,
    usize,
    usize,
);

/// A custom-tagged YAML value, returned when `OPT_PASSTHROUGH_TAG` is set and
/// accepted by `dumps` to emit a `!tag value` back out.
///
/// Holds the original tag (e.g. `!mytag`) and the resolved inner `value`.
#[pyclass(name = "YAMLRocksTag", skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct YAMLRocksTag {
    #[pyo3(get)]
    pub tag: String,
    #[pyo3(get)]
    pub value: Py<PyAny>,
}

#[pymethods]
impl YAMLRocksTag {
    #[new]
    fn new(tag: String, value: Py<PyAny>) -> Self {
        Self { tag, value }
    }

    fn __repr__(&self) -> String {
        format!("YAMLRocksTag('{}', ...)", self.tag)
    }
}

/// A `dict` subclass carrying source-location metadata (`__line__`,
/// `__column__`, `__file__`). Returned for mappings in annotated mode,
/// mirroring Home Assistant's `NodeDictClass`.
///
/// It is a genuine `dict` subclass: constructible (`YAMLRocksAnnotatedDict()`),
/// and round-trips through `copy`/`deepcopy`/`pickle` like any plain subclass,
/// so consumers (voluptuous's `data.__class__()`, `copy`, serializers) can use it
/// directly without converting to a built-in `dict` first. `module` is set so
/// pickle can locate the class by reference.
#[pyclass(name = "YAMLRocksAnnotatedDict", extends = PyDict, module = "yamlrocks", skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct YAMLRocksAnnotatedDict {
    #[pyo3(get)]
    pub __line__: u32,
    #[pyo3(get)]
    pub __column__: u32,
    #[pyo3(get)]
    pub __file__: Option<String>,
    /// 1-based end position (just past the block's last character), mirroring
    /// PyYAML's `end_mark`. See [`crate::roundtrip::ast::YamlNode::end`].
    #[pyo3(get)]
    pub __end_line__: u32,
    #[pyo3(get)]
    pub __end_column__: u32,
    /// 0-based byte offset of this node's first source character. With
    /// [`__end_offset__`](Self::__end_offset__) it gives the exact source byte
    /// range, so the original bytes can be sliced directly.
    #[pyo3(get)]
    pub __offset__: usize,
    /// 0-based byte offset just past this node's last source character (exact,
    /// from the scanner's recorded end). See [`crate::roundtrip::ast::YamlNode`].
    #[pyo3(get)]
    pub __end_offset__: usize,
    /// The config/custom tag that produced this node (`!secret`, `!env_var`,
    /// `!include*`, or a custom `!mytag`), or `None`. See
    /// [`crate::roundtrip::ast::YamlNode::source_tag`].
    #[pyo3(get)]
    pub __source_tag__: Option<String>,
    /// The directive argument that produced this node (the secret name, include
    /// path, or env-var spec), or `None`. See
    /// [`crate::roundtrip::ast::YamlNode::source_target`].
    #[pyo3(get)]
    pub __source_target__: Option<String>,
}

#[pymethods]
impl YAMLRocksAnnotatedDict {
    /// Construct an empty annotated dict with default (zero) source locations.
    /// Extra arguments are accepted and ignored here; the inherited `dict`
    /// initializer populates the contents from them, so `type(d)(items)` works
    /// like any `dict` subclass, while voluptuous's no-argument
    /// `data.__class__()` yields an empty container with default annotations.
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self {
            __line__: 0,
            __column__: 0,
            __file__: None,
            __end_line__: 0,
            __end_column__: 0,
            __offset__: 0,
            __end_offset__: 0,
            __source_tag__: None,
            __source_target__: None,
        }
    }

    /// Whether this node's value was produced by a `!secret` directive.
    #[getter]
    fn is_secret(&self) -> bool {
        self.__source_tag__.as_deref() == Some("!secret")
    }

    /// Whether this node's value was produced by an `!env_var` directive.
    #[getter]
    fn is_env_var(&self) -> bool {
        self.__source_tag__.as_deref() == Some("!env_var")
    }

    /// Whether this node's value was produced by any `!include` directive.
    #[getter]
    fn is_include(&self) -> bool {
        is_include_tag(self.__source_tag__.as_deref())
    }

    /// Restore the source-location fields when unpickling or copying (the items
    /// are repopulated separately by the reduce protocol's dict-items iterator).
    fn __setstate__(&mut self, state: AnnotationState) {
        self.__line__ = state.0;
        self.__column__ = state.1;
        self.__file__ = state.2;
        self.__end_line__ = state.3;
        self.__end_column__ = state.4;
        self.__source_tag__ = state.5;
        self.__source_target__ = state.6;
        self.__offset__ = state.7;
        self.__end_offset__ = state.8;
    }

    /// Drive `pickle` and, by fallback, `copy`/`deepcopy`: rebuild via the
    /// no-argument constructor, repopulate the entries through the dict-items
    /// iterator (so `copy` reuses values and `deepcopy` recurses into them), and
    /// restore the annotations via `__setstate__`.
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let state = {
            let me = slf.borrow();
            (
                me.__line__,
                me.__column__,
                me.__file__.clone(),
                me.__end_line__,
                me.__end_column__,
                me.__source_tag__.clone(),
                me.__source_target__.clone(),
                me.__offset__,
                me.__end_offset__,
            )
        };
        let items = slf.as_any().cast::<PyDict>()?.call_method0("items")?;
        let reduced = (
            slf.get_type(),
            PyTuple::empty(py),
            state,
            py.None(),
            items.try_iter()?,
        );
        Ok(reduced.into_pyobject(py)?.into_any().unbind())
    }
}

/// A `list` subclass carrying source-location metadata, mirroring Home
/// Assistant's `NodeListClass`. Like [`YAMLRocksAnnotatedDict`], it is a genuine
/// `list` subclass: constructible and `copy`/`deepcopy`/`pickle`-able.
#[pyclass(name = "YAMLRocksAnnotatedList", extends = PyList, module = "yamlrocks", skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct YAMLRocksAnnotatedList {
    #[pyo3(get)]
    pub __line__: u32,
    #[pyo3(get)]
    pub __column__: u32,
    #[pyo3(get)]
    pub __file__: Option<String>,
    /// 1-based end position (just past the block's last character), mirroring
    /// PyYAML's `end_mark`. See [`crate::roundtrip::ast::YamlNode::end`].
    #[pyo3(get)]
    pub __end_line__: u32,
    #[pyo3(get)]
    pub __end_column__: u32,
    /// 0-based byte offset of this node's first source character. With
    /// [`__end_offset__`](Self::__end_offset__) it gives the exact source byte
    /// range, so the original bytes can be sliced directly.
    #[pyo3(get)]
    pub __offset__: usize,
    /// 0-based byte offset just past this node's last source character (exact,
    /// from the scanner's recorded end). See [`crate::roundtrip::ast::YamlNode`].
    #[pyo3(get)]
    pub __end_offset__: usize,
    /// The config/custom tag that produced this node, or `None`. See
    /// [`crate::roundtrip::ast::YamlNode::source_tag`].
    #[pyo3(get)]
    pub __source_tag__: Option<String>,
    /// The directive argument that produced this node, or `None`. See
    /// [`crate::roundtrip::ast::YamlNode::source_target`].
    #[pyo3(get)]
    pub __source_target__: Option<String>,
}

#[pymethods]
impl YAMLRocksAnnotatedList {
    /// Construct an empty annotated list with default source locations. Extra
    /// arguments are accepted and ignored; the inherited `list` initializer
    /// populates from them.
    #[new]
    #[pyo3(signature = (*_args, **_kwargs))]
    fn new(_args: &Bound<'_, PyTuple>, _kwargs: Option<&Bound<'_, PyDict>>) -> Self {
        Self {
            __line__: 0,
            __column__: 0,
            __file__: None,
            __end_line__: 0,
            __end_column__: 0,
            __offset__: 0,
            __end_offset__: 0,
            __source_tag__: None,
            __source_target__: None,
        }
    }

    /// Whether this node's value was produced by a `!secret` directive.
    #[getter]
    fn is_secret(&self) -> bool {
        self.__source_tag__.as_deref() == Some("!secret")
    }

    /// Whether this node's value was produced by an `!env_var` directive.
    #[getter]
    fn is_env_var(&self) -> bool {
        self.__source_tag__.as_deref() == Some("!env_var")
    }

    /// Whether this node's value was produced by any `!include` directive.
    #[getter]
    fn is_include(&self) -> bool {
        is_include_tag(self.__source_tag__.as_deref())
    }

    /// Restore the source-location fields when unpickling or copying.
    fn __setstate__(&mut self, state: AnnotationState) {
        self.__line__ = state.0;
        self.__column__ = state.1;
        self.__file__ = state.2;
        self.__end_line__ = state.3;
        self.__end_column__ = state.4;
        self.__source_tag__ = state.5;
        self.__source_target__ = state.6;
        self.__offset__ = state.7;
        self.__end_offset__ = state.8;
    }

    /// Drive `pickle`/`copy`/`deepcopy`: rebuild empty, repopulate through the
    /// list-items iterator, restore annotations via `__setstate__`.
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<Py<PyAny>> {
        let py = slf.py();
        let state = {
            let me = slf.borrow();
            (
                me.__line__,
                me.__column__,
                me.__file__.clone(),
                me.__end_line__,
                me.__end_column__,
                me.__source_tag__.clone(),
                me.__source_target__.clone(),
                me.__offset__,
                me.__end_offset__,
            )
        };
        let reduced = (
            slf.get_type(),
            PyTuple::empty(py),
            state,
            slf.as_any().cast::<PyList>()?.try_iter()?,
            py.None(),
        );
        Ok(reduced.into_pyobject(py)?.into_any().unbind())
    }
}
