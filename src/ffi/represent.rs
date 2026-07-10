//! The `represent` emitter protocol for `dumps`: an optional host callback that
//! describes how its own Python objects emit, through the `Scalar`/`Sequence`/
//! `Mapping` node descriptors exposed here. See ADR-021.
//!
//! The callback is invoked for every value the emitter is about to emit,
//! builtins included. It returns a descriptor (emit it this way) or `None` (defer
//! to the built-in rendering). The descriptors carry the *original* host child
//! objects, not pre-rendered nodes, so the lowering below drives the recursion
//! and the emitter keeps owning indentation, flow, anchors, and stack depth.
//!
//! Lowering targets the round-trip [`YamlNode`] tree and the round-trip emitter,
//! which already speaks per-node style (`plain`/`single`/`double`/`literal`/
//! `folded`), per-node tags, and block/flow layout, rather than the fast `Value`
//! emitter, which speaks none of them. A value the host defers on is rendered the
//! built-in way: a compound (dict/list/set/dataclass/enum/numpy/tagged, or a
//! `default` result) decomposes into child objects that recurse back through
//! `represent` (so every value is offered to the callback), while a scalar leaf
//! goes through the shared `python_to_value` pipeline and a `Value`-to-node
//! bridge, so `default`/`serializers` and datetime/dataclass/numpy handling
//! compose and deferred output stays byte-for-byte identical to a plain `dumps`.

use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::{
    PyBool, PyBytes, PyDict, PyFloat, PyFrozenSet, PyInt, PyList, PySet, PyString, PyTuple, PyType,
};

use crate::decode::Value;
use crate::ffi::convert::{is_enum, numpy_child, python_to_value, validate_tag, EncodeCtx};
use crate::ffi::YAMLRocksTag;
use crate::resolver::{ScalarKind, Schema};
use crate::roundtrip::ast::{NodeStyle, YamlNode, YamlNodeKind};
use crate::roundtrip::value::assigned_string_style;
use crate::scanner::{ScalarStyle, Span};

pyo3::import_exception!(yamlrocks.exceptions, YAMLRocksUnserializableError);

/// Maximum object nesting depth when lowering a `represent` tree. Mirrors the
/// assign/encode depth guards so a deeply nested or self-referential object
/// cannot overflow the native stack; the recursion re-enters here per level.
const MAX_REPRESENT_DEPTH: u32 = 1000;

/// A scalar node descriptor returned by a `represent` callback: the text to emit,
/// an optional explicit tag, and a style. `style="auto"` lets the emitter apply
/// its implicit-resolver quoting; an explicit style is honored verbatim.
#[pyclass(
    name = "YAMLRocksScalar",
    module = "yamlrocks",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub struct YAMLRocksScalar {
    value: String,
    tag: Option<String>,
    /// The chosen style, or `None` for `"auto"` (let the emitter decide).
    style: Option<ScalarStyle>,
}

#[pymethods]
impl YAMLRocksScalar {
    #[new]
    #[pyo3(signature = (value, *, tag=None, style="auto"))]
    fn new(value: String, tag: Option<String>, style: Option<&str>) -> PyResult<Self> {
        Ok(Self {
            value,
            tag,
            style: parse_style(style)?,
        })
    }

    fn __repr__(&self) -> String {
        let style = self.style.map_or("auto", ScalarStyle::name);
        match &self.tag {
            Some(tag) => format!(
                "YAMLRocksScalar({:?}, tag={tag:?}, style={style:?})",
                self.value
            ),
            None => format!("YAMLRocksScalar({:?}, style={style:?})", self.value),
        }
    }
}

/// A sequence node descriptor: its items (original host objects, re-dispatched
/// through `represent`), an optional tag, and an optional flow override.
#[pyclass(name = "YAMLRocksSequence", module = "yamlrocks", frozen)]
pub struct YAMLRocksSequence {
    items: Py<PyAny>,
    tag: Option<String>,
    flow: Option<bool>,
}

#[pymethods]
impl YAMLRocksSequence {
    #[new]
    #[pyo3(signature = (items, *, tag=None, flow=None))]
    fn new(items: Py<PyAny>, tag: Option<String>, flow: Option<bool>) -> Self {
        Self { items, tag, flow }
    }
}

/// A mapping node descriptor: its `(key, value)` pairs (original host objects,
/// re-dispatched through `represent`), an optional tag, and a flow override.
#[pyclass(name = "YAMLRocksMapping", module = "yamlrocks", frozen)]
pub struct YAMLRocksMapping {
    pairs: Py<PyAny>,
    tag: Option<String>,
    flow: Option<bool>,
}

#[pymethods]
impl YAMLRocksMapping {
    #[new]
    #[pyo3(signature = (pairs, *, tag=None, flow=None))]
    fn new(pairs: Py<PyAny>, tag: Option<String>, flow: Option<bool>) -> Self {
        Self { pairs, tag, flow }
    }
}

/// Map a style name to a [`ScalarStyle`], or `None` for `"auto"` (and the
/// unset default). Any other name is a `ValueError`, so a typo fails loudly
/// rather than silently emitting the wrong style.
fn parse_style(style: Option<&str>) -> PyResult<Option<ScalarStyle>> {
    Ok(match style {
        None | Some("auto") => None,
        Some("plain") => Some(ScalarStyle::Plain),
        Some("single") => Some(ScalarStyle::SingleQuoted),
        Some("double") => Some(ScalarStyle::DoubleQuoted),
        Some("literal") => Some(ScalarStyle::Literal),
        Some("folded") => Some(ScalarStyle::Folded),
        Some(other) => {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "invalid scalar style {other:?}: expected one of 'auto', 'plain', \
                 'single', 'double', 'literal', 'folded'"
            )))
        }
    })
}

/// Lower a Python object into a synthetic [`YamlNode`] tree using `represent`,
/// ready for the round-trip emitter. Detects shared objects and emits them once
/// with an anchor, aliasing the repeats.
#[allow(clippy::too_many_arguments)]
pub fn represent_to_node(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    represent: &Bound<'_, PyAny>,
    encode: EncodeCtx<'_>,
    sort_keys: bool,
    flow_all: bool,
    double_quotes: bool,
    schema: Schema,
) -> PyResult<YamlNode> {
    let mut ctx = Lower {
        py,
        represent,
        encode,
        sort_keys,
        flow_all,
        double_quotes,
        schema,
        seen: HashSet::new(),
        aliased: HashSet::new(),
        retained: Vec::new(),
    };
    let mut root = ctx.lower(obj, 0, false)?;
    // Turn the raw `id()` markers left on shared nodes into real anchor names,
    // dropping them from nodes that were never aliased.
    name_anchors(&mut root, &ctx.aliased, &mut HashMap::new(), &mut 0);
    Ok(root)
}

/// Carries the lowering invariants plus the shared-object tracking. `seen` holds
/// the `id()` of every aliasable object lowered so far; `aliased` holds those
/// that turned up again (so a name is worth minting). `encode` is the full
/// `dumps` encode context, used to render a deferred scalar leaf through the same
/// pipeline as a plain `dumps` (so `default`/`serializers`/datetime compose).
struct Lower<'py, 'r, 'c> {
    py: Python<'py>,
    represent: &'r Bound<'py, PyAny>,
    encode: EncodeCtx<'c>,
    sort_keys: bool,
    flow_all: bool,
    double_quotes: bool,
    schema: Schema,
    seen: HashSet<usize>,
    aliased: HashSet<usize>,
    /// A strong reference to every object recorded in `seen`, so its `id()`
    /// (a raw pointer) stays valid for the whole lowering. Without this a
    /// temporary from `default`/numpy could be freed and its address reused by a
    /// later temporary, which would then be misread as an alias to the first.
    retained: Vec<Py<PyAny>>,
}

impl Lower<'_, '_, '_> {
    fn lower(&mut self, obj: &Bound<'_, PyAny>, depth: u32, in_flow: bool) -> PyResult<YamlNode> {
        // Grow the native stack on demand so a deeply nested object (bounded by
        // `MAX_REPRESENT_DEPTH`) cannot overflow a small thread stack. See
        // [`crate::stack`].
        crate::stack::guard(|| self.lower_inner(obj, depth, in_flow))
    }

    fn lower_inner(
        &mut self,
        obj: &Bound<'_, PyAny>,
        depth: u32,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        if depth >= MAX_REPRESENT_DEPTH {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "object is too deeply nested to serialize (possible self-reference)",
            ));
        }

        // A non-scalar object (matching PyYAML's `ignore_aliases`) is aliasable: a
        // repeat becomes an alias, and a first occurrence is marked before its
        // children are lowered so a cycle resolves to an alias instead of looping.
        let aliasable = is_aliasable(obj);
        let id = obj.as_ptr() as usize;
        if aliasable {
            if self.seen.contains(&id) {
                self.aliased.insert(id);
                return Ok(synthetic(
                    YamlNodeKind::Alias(id.to_string()),
                    NodeStyle::Block,
                    None,
                ));
            }
            self.seen.insert(id);
            // Keep the object alive so its `id()` cannot be reused mid-lowering.
            self.retained.push(obj.clone().unbind());
        }

        let mut node = self.render(obj, depth, in_flow)?;
        if !aliasable {
            return Ok(node);
        }
        // `render` either materializes a *new* node for this object (a
        // container/scalar it built, with no anchor yet) or delegates
        // transparently to a child (enum value, numpy, a `default`/serializer
        // result), returning the child's node, which already carries the child's
        // anchor or is an alias.
        let delegated = matches!(node.kind, YamlNodeKind::Alias(_)) || node.anchor.is_some();
        if !delegated {
            // A node this object materialized itself: stamp its id, which
            // `name_anchors` later turns into a real anchor name (or strips if it
            // was never aliased). A cycle back into it (e.g. `d["self"] = d`) has
            // already recorded an alias to this id, which now resolves.
            node.anchor = Some(id.to_string());
            return Ok(node);
        }
        // Delegated: this object produced no node of its own. If something inside
        // the delegated subtree aliased back to *this* object, there is no node
        // bearing its identity for that alias to resolve to (a `default` that
        // returns a container holding the original, or a value that resolves only
        // to itself). Raise, as a plain `dumps` does for such input.
        if self.aliased.contains(&id) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "cannot serialize a value that refers only to itself",
            ));
        }
        // Otherwise the delegated node keeps the child's identity; untrack this
        // object so a later occurrence re-dispatches rather than aliasing an id no
        // node defines.
        self.seen.remove(&id);
        Ok(node)
    }

    /// The node for `obj`: the representer's result, or the built-in rendering
    /// when the host defers. Alias bookkeeping is the caller's ([`lower_inner`]).
    fn render(&mut self, obj: &Bound<'_, PyAny>, depth: u32, in_flow: bool) -> PyResult<YamlNode> {
        let described = self.represent.call1((obj,))?;
        if !described.is_none() {
            return self.descriptor_to_node(&described, depth, in_flow);
        }
        self.deferred_node(obj, depth, in_flow)
    }

    /// Built-in rendering for a value the host deferred on. A compound value
    /// decomposes into child objects that recurse back through `represent` (so
    /// every value is offered to the callback); a scalar leaf goes through the
    /// shared `python_to_value` pipeline so datetime/Decimal/... and the
    /// numeric/string formatting match a plain `dumps` byte-for-byte.
    fn deferred_node(
        &mut self,
        obj: &Bound<'_, PyAny>,
        depth: u32,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        // Primitive scalars, and their subclasses (an `IntEnum`, a `str`/`bytes`
        // subclass), are converted directly, before the `serializers` registry,
        // matching the fast path's dispatch order: a registered primitive
        // subclass is emitted as its builtin, not routed through its serializer.
        if obj.is_instance_of::<PyBool>()
            || obj.is_instance_of::<PyInt>()
            || obj.is_instance_of::<PyFloat>()
            || obj.is_instance_of::<PyString>()
            || obj.is_instance_of::<PyBytes>()
        {
            let value = python_to_value(self.py, obj, self.encode)?;
            return Ok(self.value_to_node(value, in_flow));
        }
        if let Ok(dict) = obj.cast::<PyDict>() {
            // Snapshot before recursing: `represent` runs arbitrary Python that
            // could mutate the dict mid-walk (which `PyDict_Next` forbids).
            let entries: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = dict.iter().collect();
            return self.mapping_node(entries, depth, None, None, in_flow);
        }
        if let Ok(list) = obj.cast::<PyList>() {
            return self.sequence_node(list.iter().collect(), depth, None, None, in_flow);
        }
        if let Ok(tuple) = obj.cast::<PyTuple>() {
            return self.sequence_node(tuple.iter().collect(), depth, None, None, in_flow);
        }
        if let Ok(set) = obj.cast::<PySet>() {
            return self.sequence_node(set.iter().collect(), depth, None, None, in_flow);
        }
        if let Ok(set) = obj.cast::<PyFrozenSet>() {
            return self.sequence_node(set.iter().collect(), depth, None, None, in_flow);
        }
        // A load-side custom-tagged value (`OPT_PASSTHROUGH_TAG` output).
        if let Ok(tag_obj) = obj.cast::<YAMLRocksTag>() {
            let (tag, inner) = {
                let borrowed = tag_obj.borrow();
                (borrowed.tag.clone(), borrowed.value.clone_ref(self.py))
            };
            return self.tagged_node(tag, inner.bind(self.py), depth, in_flow);
        }
        // A `serializers` entry for this exact type: a custom `!tag value`.
        if let Some(registry) = self.encode.tags {
            if let Some(func) = registry.bind(self.py).get_item(obj.get_type())? {
                let result = func.call1((obj,))?;
                return self.serializer_result_node(&result, depth, in_flow);
            }
        }
        // Enum: its value, re-dispatched. Through `lower` (not `render`) so the
        // depth increments and the stack guard applies.
        if is_enum(obj)? {
            return self.lower(&obj.getattr("value")?, depth + 1, in_flow);
        }
        // Dataclass instance: a mapping of its fields.
        if !self.encode.passthrough_dataclass
            && obj.hasattr("__dataclass_fields__")?
            && !obj.is_instance_of::<PyType>()
        {
            let fields = obj.getattr("__dataclass_fields__")?;
            let mut entries: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = Vec::new();
            for name in fields.try_iter()? {
                let name = name?;
                let value = obj.getattr(name.cast::<PyString>()?.to_str()?)?;
                entries.push((name, value));
            }
            return self.mapping_node(entries, depth, None, None, in_flow);
        }
        // numpy array/scalar (opt-in): re-dispatch its list/scalar form through
        // `lower` (depth-bounded, stack-guarded).
        if self.encode.serialize_numpy {
            if let Some(child) = numpy_child(self.py, obj)? {
                return self.lower(&child, depth + 1, in_flow);
            }
        }
        // A scalar leaf (str/int/float/bool/None/datetime/Decimal/UUID/Path/...).
        // Render it through the shared pipeline with `default` disabled.
        let no_default = EncodeCtx {
            default: None,
            ..self.encode
        };
        match python_to_value(self.py, obj, no_default) {
            Ok(value) => Ok(self.value_to_node(value, in_flow)),
            // Only an *unrecognized type* falls back to `default`, matching the
            // fast path; a genuine encode error (non-UTF-8 bytes, a lone
            // surrogate) is propagated unchanged rather than masked by `default`.
            // The result re-dispatches through `lower` so its children reach
            // `represent` and a non-progressing `default` is depth-bounded.
            Err(err) if err.is_instance_of::<YAMLRocksUnserializableError>(self.py) => {
                match self.encode.default {
                    Some(default) => {
                        let result = default.call1(self.py, (obj,))?;
                        // Disable `default` while lowering its result, matching the
                        // fast path (which converts a `default` result with
                        // `default` off): a result that is itself unsupported
                        // raises rather than re-invoking `default`. `represent`
                        // still sees the result and its children.
                        let prev = self.encode.default.take();
                        let lowered = self.lower(result.bind(self.py), depth + 1, in_flow);
                        self.encode.default = prev;
                        lowered
                    }
                    None => Err(err),
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Turn a `represent` return value (a `Scalar`/`Sequence`/`Mapping`) into a
    /// node.
    fn descriptor_to_node(
        &mut self,
        described: &Bound<'_, PyAny>,
        depth: u32,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        if let Ok(scalar) = described.cast::<YAMLRocksScalar>() {
            let scalar = scalar.borrow();
            let mut node = scalar_node(
                scalar.value.clone(),
                scalar.style,
                scalar.tag.as_deref(),
                self.double_quotes,
                self.schema,
                in_flow,
            )?;
            // A scalar carries no children, so its span end stays at the start.
            node.end_offset = node.span.offset;
            return Ok(node);
        }
        if let Ok(seq) = described.cast::<YAMLRocksSequence>() {
            let seq = seq.borrow();
            let items: Vec<Bound<'_, PyAny>> = seq
                .items
                .bind(self.py)
                .try_iter()?
                .collect::<PyResult<_>>()?;
            return self.sequence_node(items, depth, seq.tag.clone(), seq.flow, in_flow);
        }
        if let Ok(map) = described.cast::<YAMLRocksMapping>() {
            let map = map.borrow();
            let mut entries: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = Vec::new();
            for pair in map.pairs.bind(self.py).try_iter()? {
                let pair = pair?;
                // Each pair must be a 2-tuple, as the type advertises. Accepting
                // any 2-item sequence would let a longer tuple silently drop
                // items, or a 2-character string be read as `(key, value)`.
                let pair = pair.cast_into::<PyTuple>().map_err(|_| {
                    pyo3::exceptions::PyValueError::new_err(
                        "a YAMLRocksMapping pair must be a (key, value) tuple",
                    )
                })?;
                if pair.len() != 2 {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "a YAMLRocksMapping pair must be a (key, value) tuple of exactly two items",
                    ));
                }
                entries.push((pair.get_item(0)?, pair.get_item(1)?));
            }
            return self.mapping_node(entries, depth, map.tag.clone(), map.flow, in_flow);
        }
        Err(pyo3::exceptions::PyTypeError::new_err(
            "a represent callback must return a yamlrocks.YAMLRocksScalar, \
             yamlrocks.YAMLRocksSequence, yamlrocks.YAMLRocksMapping, or None",
        ))
    }

    /// Build a mapping node from Python `(key, value)` pairs, sorting keys when
    /// requested and lowering each key and value (so they reach `represent`).
    fn mapping_node(
        &mut self,
        entries: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)>,
        depth: u32,
        tag: Option<String>,
        flow: Option<bool>,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        let tag = tag.map(|t| normalize_tag(&t));
        if let Some(tag) = &tag {
            validate_tag(tag)?;
        }
        let flow = self.flowing(flow, in_flow);
        let ordered = self.sorted_pairs(entries);
        let mut pairs = Vec::with_capacity(ordered.len());
        for (key, val) in &ordered {
            pairs.push((
                self.lower(key, depth + 1, flow)?,
                self.lower(val, depth + 1, flow)?,
            ));
        }
        Ok(synthetic(
            YamlNodeKind::Mapping(pairs),
            node_style(flow),
            tag,
        ))
    }

    /// Build a sequence node from Python items, lowering each (so they reach
    /// `represent`).
    fn sequence_node(
        &mut self,
        items: Vec<Bound<'_, PyAny>>,
        depth: u32,
        tag: Option<String>,
        flow: Option<bool>,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        let tag = tag.map(|t| normalize_tag(&t));
        if let Some(tag) = &tag {
            validate_tag(tag)?;
        }
        let flow = self.flowing(flow, in_flow);
        let mut out = Vec::with_capacity(items.len());
        for item in &items {
            out.push(self.lower(item, depth + 1, flow)?);
        }
        Ok(synthetic(
            YamlNodeKind::Sequence(out),
            node_style(flow),
            tag,
        ))
    }

    /// A tagged node: lower `inner` (its children still reach `represent`) and
    /// attach the validated tag. `inner` goes through `lower` at the next depth so
    /// it is depth-bounded and stack-guarded; a serializer that returns a tag
    /// wrapping its own input therefore aliases (and is rejected below) instead of
    /// recursing without bound.
    fn tagged_node(
        &mut self,
        tag: String,
        inner: &Bound<'_, PyAny>,
        depth: u32,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        let tag = normalize_tag(&tag);
        validate_tag(&tag)?;
        let mut node = self.lower(inner, depth + 1, in_flow)?;
        // An alias cannot carry a tag (nor an anchor); a tag wrapping a
        // self-referential or already-shared value is unrepresentable.
        if matches!(node.kind, YamlNodeKind::Alias(_)) {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "cannot attach a tag to a self-referential or shared value",
            ));
        }
        node.tag = Some(tag);
        Ok(node)
    }

    /// Interpret a `serializers` callback result: a `YAMLRocksTag` or a
    /// `(tag, value)` tuple, both producing a tagged node.
    fn serializer_result_node(
        &mut self,
        result: &Bound<'_, PyAny>,
        depth: u32,
        in_flow: bool,
    ) -> PyResult<YamlNode> {
        if let Ok(tag_obj) = result.cast::<YAMLRocksTag>() {
            let (tag, inner) = {
                let borrowed = tag_obj.borrow();
                (borrowed.tag.clone(), borrowed.value.clone_ref(self.py))
            };
            return self.tagged_node(tag, inner.bind(self.py), depth, in_flow);
        }
        if let Ok(tuple) = result.cast::<PyTuple>() {
            if tuple.len() == 2 {
                let tag: String = tuple.get_item(0)?.extract()?;
                let value = tuple.get_item(1)?;
                return self.tagged_node(tag, &value, depth, in_flow);
            }
        }
        Err(pyo3::exceptions::PyTypeError::new_err(
            "a serializers callback must return a yamlrocks.YAMLRocksTag or a (tag, value) tuple",
        ))
    }

    /// Whether a collection emits in flow style: forced when already inside a flow
    /// collection (a block child would be invalid there), else the descriptor's
    /// explicit `flow=`, else the document-wide `OPT_FLOW_STYLE` default.
    fn flowing(&self, flow: Option<bool>, in_flow: bool) -> bool {
        in_flow || flow.unwrap_or(self.flow_all)
    }

    /// Order a mapping's `(key, value)` pairs for emission. With `sort_keys` set,
    /// sort by a key derived directly from the Python key object (by type and
    /// value), matching the fast path's `compare_keys`: null, then booleans, then
    /// numbers numerically, then strings lexically, then everything else in input
    /// order. Deriving the sort key straight from the object (not through
    /// `python_to_value`) means `default`/`serializers` are not run an extra time
    /// on a custom key. Sorting happens here, before the values are lowered, so
    /// anchor detection follows the final emission order (sorting after would risk
    /// emitting an alias before its anchor).
    fn sorted_pairs<'a>(
        &self,
        pairs: Vec<(Bound<'a, PyAny>, Bound<'a, PyAny>)>,
    ) -> Vec<(Bound<'a, PyAny>, Bound<'a, PyAny>)> {
        if !self.sort_keys {
            return pairs;
        }
        let mut keyed: Vec<(SortKey, Bound<'a, PyAny>, Bound<'a, PyAny>)> = pairs
            .into_iter()
            .enumerate()
            .map(|(i, (k, v))| (SortKey::of(&k, i), k, v))
            .collect();
        keyed.sort_by(|(a, _, _), (b, _, _)| a.cmp(b));
        keyed.into_iter().map(|(_, k, v)| (k, v)).collect()
    }

    /// Bridge a fast-path [`Value`] (from the deferred scalar-leaf pipeline) into a
    /// synthetic node, choosing the same styles the fast encoder would so the
    /// round-trip emitter reproduces its bytes. `in_flow` forces a block scalar
    /// (`literal`/`folded`) to a quoted style, since a block scalar is invalid
    /// inside a flow collection.
    fn value_to_node(&self, value: Value<'_>, in_flow: bool) -> YamlNode {
        // Grow the native stack on demand: the `Value` tree is bounded by the
        // encoder's depth guard, but the walk still recurses per level.
        crate::stack::guard(|| self.value_to_node_inner(value, in_flow))
    }

    fn value_to_node_inner(&self, value: Value<'_>, in_flow: bool) -> YamlNode {
        use crate::decode::Value as V;
        match value {
            V::Null => synthetic(YamlNodeKind::Null, NodeStyle::Block, None),
            V::Bool(b) => scalar_plain(if b { "true" } else { "false" }),
            V::Int(i) => scalar_plain(&i.to_string()),
            V::BigInt(s) => scalar_plain(&s),
            V::Float(f) => scalar_plain(&crate::emit_util::canonical_float(f)),
            V::String(s) => {
                let style = flow_safe_style(
                    auto_string_style(&s, self.double_quotes, self.schema),
                    in_flow,
                );
                synthetic(
                    YamlNodeKind::Scalar(s.into_owned(), style),
                    NodeStyle::Block,
                    None,
                )
            }
            V::Timestamp(ts) => scalar_plain(&ts.to_iso()),
            V::Sequence(items) => {
                let items = items
                    .into_iter()
                    .map(|v| self.value_to_node(v, in_flow))
                    .collect();
                synthetic(YamlNodeKind::Sequence(items), NodeStyle::Block, None)
            }
            V::Mapping(pairs) => {
                let pairs = pairs
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            self.value_to_node(k, in_flow),
                            self.value_to_node(v, in_flow),
                        )
                    })
                    .collect();
                synthetic(YamlNodeKind::Mapping(pairs), NodeStyle::Block, None)
            }
            V::Tagged(tag, inner) => {
                let mut node = self.value_to_node(*inner, in_flow);
                node.tag = Some(tag);
                node
            }
        }
    }
}

/// Whether `obj` is aliasable: shared occurrences should emit an anchor and
/// alias rather than duplicate. Mirrors PyYAML's `ignore_aliases`, which never
/// aliases `None`, `bool`, `int`, `float`, `str`, or `bytes` (interned/immutable
/// scalars whose `id()` would collide spuriously), and aliases everything else,
/// so a shared mapping/sequence/set or a custom object represented as one is
/// deduped, and a cycle through any of them resolves to an alias.
fn is_aliasable(obj: &Bound<'_, PyAny>) -> bool {
    !(obj.is_none()
        || obj.is_instance_of::<PyBool>()
        || obj.is_instance_of::<PyInt>()
        || obj.is_instance_of::<PyFloat>()
        || obj.is_instance_of::<PyString>()
        || obj.is_instance_of::<PyBytes>())
}

/// A total-ordered sort key for a mapping key under `sort_keys`, derived directly
/// from the Python key object. Mirrors the fast path's `compare_keys` ranking
/// (null, bool, number, string, then everything else) so the represent path sorts
/// scalar keys identically to a plain `dumps`: integers within `i64` compare
/// exactly and larger ones as `f64` (as `compare_keys` treats a `BigInt`), and
/// integers and floats share one numeric rank compared numerically. A key that is
/// not a plain scalar (a custom object, or a special type like `datetime`/`UUID`
/// that the fast path would stringify) falls into `Other`, tie-broken by input
/// index to stay a stable no-op with a total order.
#[derive(PartialEq)]
enum SortKey {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Other(usize),
}

impl SortKey {
    fn of(obj: &Bound<'_, PyAny>, index: usize) -> Self {
        if obj.is_none() {
            return SortKey::Null;
        }
        // `bool` is an `int` subclass, so test it first.
        if let Ok(b) = obj.cast::<PyBool>() {
            return SortKey::Bool(b.is_true());
        }
        if obj.is_instance_of::<PyInt>() {
            // Compare exactly within `i64` and fall back to `f64` beyond it,
            // matching the fast path's `compare_keys` (which compares `i64` keys
            // exactly and a `BigInt` as `f64`, so two integers past `i64` that
            // round to the same `f64` tie and keep insertion order).
            return match obj.extract::<i64>() {
                Ok(i) => SortKey::Int(i),
                Err(_) => obj
                    .extract::<f64>()
                    .map_or(SortKey::Other(index), SortKey::Float),
            };
        }
        if obj.is_instance_of::<PyFloat>() {
            if let Ok(f) = obj.extract::<f64>() {
                return SortKey::Float(f);
            }
        }
        if let Ok(s) = obj.cast::<PyString>() {
            if let Ok(text) = s.to_str() {
                return SortKey::Str(text.to_owned());
            }
        }
        SortKey::Other(index)
    }

    fn rank(&self) -> u8 {
        match self {
            SortKey::Null => 0,
            SortKey::Bool(_) => 1,
            // Integers and floats share the numeric rank, as in `compare_keys`.
            SortKey::Int(_) | SortKey::Float(_) => 2,
            SortKey::Str(_) => 3,
            SortKey::Other(_) => 4,
        }
    }

    /// A number's `f64` value, for cross-representation numeric ordering.
    fn as_f64(&self) -> f64 {
        match self {
            SortKey::Int(i) => *i as f64,
            SortKey::Float(f) => *f,
            _ => f64::NAN,
        }
    }

    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.rank()
            .cmp(&other.rank())
            .then_with(|| match (self, other) {
                (SortKey::Bool(a), SortKey::Bool(b)) => a.cmp(b),
                // Two exact integers compare exactly; a mix with a float compares
                // numerically as `f64`.
                (SortKey::Int(a), SortKey::Int(b)) => a.cmp(b),
                (SortKey::Str(a), SortKey::Str(b)) => a.cmp(b),
                (SortKey::Other(a), SortKey::Other(b)) => a.cmp(b),
                _ if self.rank() == 2 => self.as_f64().total_cmp(&other.as_f64()),
                _ => Ordering::Equal,
            })
    }
}

/// Replace the raw `id()` markers left on aliasable nodes with real anchor names.
/// An object that turned up more than once (in `aliased`) gets a sequential
/// `id001`-style name shared by its anchor and every alias; one that never
/// repeated has its marker stripped. Pre-order, so an anchor is named before the
/// aliases that reference it.
fn name_anchors(
    node: &mut YamlNode,
    aliased: &HashSet<usize>,
    names: &mut HashMap<usize, String>,
    counter: &mut usize,
) {
    // Grow the native stack on demand so walking a deeply nested synthetic tree
    // (bounded by `MAX_REPRESENT_DEPTH`) cannot overflow a small thread stack,
    // matching every other `YamlNode` descent. See [`crate::stack`].
    crate::stack::guard(|| name_anchors_inner(node, aliased, names, counter))
}

fn name_anchors_inner(
    node: &mut YamlNode,
    aliased: &HashSet<usize>,
    names: &mut HashMap<usize, String>,
    counter: &mut usize,
) {
    if let Some(marker) = node.anchor.take() {
        // A marker is always a decimal `id()`; a real anchor never reaches here.
        if let Ok(id) = marker.parse::<usize>() {
            if aliased.contains(&id) {
                let name = names.entry(id).or_insert_with(|| {
                    *counter += 1;
                    format!("id{counter:03}")
                });
                node.anchor = Some(name.clone());
            }
        }
    }
    match &mut node.kind {
        YamlNodeKind::Alias(marker) => {
            if let Ok(id) = marker.parse::<usize>() {
                if let Some(name) = names.get(&id) {
                    *marker = name.clone();
                }
            }
        }
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                name_anchors(key, aliased, names, counter);
                name_anchors(val, aliased, names, counter);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                name_anchors(item, aliased, names, counter);
            }
        }
        _ => {}
    }
}

/// A synthetic plain scalar node, for a value that is inherently plain-safe (a
/// bool/int/float/timestamp token from the fast-path `Value`).
fn scalar_plain(text: &str) -> YamlNode {
    synthetic(
        YamlNodeKind::Scalar(text.to_owned(), ScalarStyle::Plain),
        NodeStyle::Block,
        None,
    )
}

/// Build a synthetic node (edited-in, not parsed) with the given kind, layout,
/// and tag. Synthetic nulls follow the document's null style on re-emission.
fn synthetic(kind: YamlNodeKind, style: NodeStyle, tag: Option<String>) -> YamlNode {
    let mut node = YamlNode::new(kind, Span::default());
    node.synthetic = true;
    node.style = style;
    node.tag = tag;
    node
}

/// Block or flow layout for a collection node.
fn node_style(flow: bool) -> NodeStyle {
    if flow {
        NodeStyle::Flow
    } else {
        NodeStyle::Block
    }
}

/// Downgrade a block scalar style (`literal`/`folded`) to double-quoted when the
/// scalar sits inside a flow collection, where a block scalar is invalid.
fn flow_safe_style(style: ScalarStyle, in_flow: bool) -> ScalarStyle {
    if in_flow && matches!(style, ScalarStyle::Literal | ScalarStyle::Folded) {
        ScalarStyle::DoubleQuoted
    } else {
        style
    }
}

/// Build a scalar node from a descriptor's value, style, and tag, applying
/// PyYAML-faithful auto styling when the style is unset. Any provided tag is
/// validated with the emit-side tag rules.
///
/// An explicit style is honored verbatim, tag kept. For `auto` we mirror
/// PyYAML's rule so a host's representers port without hand-annotating styles:
///
/// - No tag: quote only when a plain rendering would reload as a different type
///   ([`assigned_string_style`]).
/// - A standard tag the plain value already resolves to (`!!bool` on `true`,
///   `!!float` on `1.0e17`): elide the tag, render like an untagged value. The
///   tag is redundant.
/// - `!!str` on a value that would plainly resolve to a non-string (`"true"`):
///   quote it so it stays a string, and elide the (now-implied) tag.
/// - Any other tag, including every custom tag (`!extend`, `!secret`, ...):
///   keep the tag and force quotes, because a plain form would resolve to a
///   different tag and lose it. This is why `!extend my_id` emits as
///   `!extend 'my_id'`.
///
/// A block scalar style inside a flow collection is downgraded to a quoted style
/// (`in_flow`), since block scalars are invalid there.
fn scalar_node(
    value: String,
    style: Option<ScalarStyle>,
    tag: Option<&str>,
    double_quotes: bool,
    schema: Schema,
    in_flow: bool,
) -> PyResult<YamlNode> {
    // Normalize a canonical core tag to its `!!X` shorthand, then validate.
    let tag = tag.map(normalize_tag);
    if let Some(tag) = &tag {
        validate_tag(tag)?;
    }
    let tag = tag.as_deref();
    if let Some(style) = style {
        return Ok(synthetic(
            YamlNodeKind::Scalar(value, flow_safe_style(style, in_flow)),
            NodeStyle::Block,
            tag.map(str::to_owned),
        ));
    }
    let Some(tag) = tag else {
        let style = flow_safe_style(auto_string_style(&value, double_quotes, schema), in_flow);
        return Ok(synthetic(
            YamlNodeKind::Scalar(value, style),
            NodeStyle::Block,
            None,
        ));
    };
    let plain_kind = schema.classify(&value, ScalarStyle::Plain, None);
    let node = match standard_tag_kind(tag) {
        // The plain value already resolves to the tag's type: the tag is
        // redundant, so drop it. A string still runs through the quoting rule
        // (a plain `[x]` would be unsafe); a bool/int/float/null token is
        // inherently plain-safe (`true`, `1.0e17`), so emit it plain.
        Some(std) if kind_matches(plain_kind, std) => {
            let style = if matches!(std, StdKind::Str) {
                flow_safe_style(auto_string_style(&value, double_quotes, schema), in_flow)
            } else {
                ScalarStyle::Plain
            };
            synthetic(YamlNodeKind::Scalar(value, style), NodeStyle::Block, None)
        }
        // A `!!str` on a value that would resolve to a non-string: quote to keep
        // it a string; the quoting implies the str tag, so drop it.
        Some(StdKind::Str) => synthetic(
            YamlNodeKind::Scalar(value.clone(), forced_quote_style(&value)),
            NodeStyle::Block,
            None,
        ),
        // A custom tag (or a standard one the value does not match): keep the tag
        // and force quotes so a plain form cannot silently drop or re-resolve it.
        _ => synthetic(
            YamlNodeKind::Scalar(value.clone(), forced_quote_style(&value)),
            NodeStyle::Block,
            Some(tag.to_owned()),
        ),
    };
    Ok(node)
}

/// A YAML standard scalar type, recognized in both `!!x` and full
/// `tag:yaml.org,2002:x` spellings.
#[derive(Clone, Copy)]
enum StdKind {
    Null,
    Bool,
    Int,
    Float,
    Str,
}

/// Normalize a canonical core tag (`tag:yaml.org,2002:X`) to its `!!X` shorthand,
/// so a callback ported from a PyYAML representer (which uses the canonical URI
/// tags) both passes tag validation and gets the standard-tag elision. Any other
/// tag is left as written.
fn normalize_tag(tag: &str) -> String {
    match tag.strip_prefix("tag:yaml.org,2002:") {
        Some(suffix) => format!("!!{suffix}"),
        None => tag.to_owned(),
    }
}

/// Classify a tag as a standard YAML scalar type, or `None` for a custom tag.
fn standard_tag_kind(tag: &str) -> Option<StdKind> {
    match tag {
        "!!null" | "tag:yaml.org,2002:null" => Some(StdKind::Null),
        "!!bool" | "tag:yaml.org,2002:bool" => Some(StdKind::Bool),
        "!!int" | "tag:yaml.org,2002:int" => Some(StdKind::Int),
        "!!float" | "tag:yaml.org,2002:float" => Some(StdKind::Float),
        "!!str" | "tag:yaml.org,2002:str" => Some(StdKind::Str),
        _ => None,
    }
}

/// Whether a resolved plain-scalar kind is the type a standard tag names.
fn kind_matches(kind: ScalarKind, std: StdKind) -> bool {
    matches!(
        (kind, std),
        (ScalarKind::Null, StdKind::Null)
            | (ScalarKind::Bool(_), StdKind::Bool)
            | (ScalarKind::Int(_) | ScalarKind::BigInt, StdKind::Int)
            | (ScalarKind::Float(_), StdKind::Float)
            | (ScalarKind::Str, StdKind::Str)
    )
}

/// The quote style for a scalar that must be quoted (a custom-tagged value, or a
/// string that would otherwise reparse as another type). Single quotes, matching
/// PyYAML's default, unless the value cannot be single-quoted (a line break, a
/// literal single quote, or a control/non-printable character), which forces
/// double quotes, where it can be escaped.
fn forced_quote_style(value: &str) -> ScalarStyle {
    if crate::emit_util::single_quotable(value, false) {
        ScalarStyle::SingleQuoted
    } else {
        ScalarStyle::DoubleQuoted
    }
}

/// The style for an untagged string under `auto`: a `|` literal block for
/// multi-line content it can represent (matching the fast encoder), otherwise the
/// shared quoting rule. Keeps a deferred multi-line string from double-quoting.
fn auto_string_style(value: &str, double_quotes: bool, schema: Schema) -> ScalarStyle {
    if crate::emit_util::use_literal_block(value) {
        ScalarStyle::Literal
    } else {
        assigned_string_style(value, double_quotes, schema)
    }
}
