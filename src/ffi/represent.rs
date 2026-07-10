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
//! emitter, which speaks none of them. A deferred value (one `represent` returns
//! `None` for) runs through the same `python_to_value` encode pipeline as a plain
//! `dumps`, then a `Value`-to-node bridge, so `default`/`serializers` and the
//! datetime/dataclass/numpy handling compose and deferred output stays
//! byte-for-byte identical to a plain `dumps`.

use std::collections::{HashMap, HashSet};

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use crate::decode::Value;
use crate::ffi::convert::{python_to_value, EncodeCtx};
use crate::resolver::{ScalarKind, Schema};
use crate::roundtrip::ast::{NodeStyle, YamlNode, YamlNodeKind};
use crate::roundtrip::value::assigned_string_style;
use crate::scanner::{ScalarStyle, Span};

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
    #[pyo3(signature = (value, *, tag=None, style=None))]
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
/// ready for the round-trip emitter. Detects shared container objects and emits
/// them once with an anchor, aliasing the repeats.
pub fn represent_to_node(
    py: Python<'_>,
    obj: &Bound<'_, PyAny>,
    represent: &Bound<'_, PyAny>,
    encode: EncodeCtx<'_>,
    sort_keys: bool,
    double_quotes: bool,
    schema: Schema,
) -> PyResult<YamlNode> {
    let mut ctx = Lower {
        py,
        represent,
        encode,
        sort_keys,
        double_quotes,
        schema,
        seen: HashSet::new(),
        aliased: HashSet::new(),
    };
    let mut root = ctx.lower(obj, 0)?;
    // Turn the raw `id()` markers left on shared nodes into real anchor names,
    // dropping them from nodes that were never aliased.
    name_anchors(&mut root, &ctx.aliased, &mut HashMap::new(), &mut 0);
    Ok(root)
}

/// Carries the lowering invariants plus the shared-object tracking. `seen` holds
/// the `id()` of every container lowered so far; `aliased` holds those that
/// turned up again (so a name is worth minting). `encode` is the full `dumps`
/// encode context, used to render a deferred value through the same pipeline as
/// a plain `dumps` (so `default`/`serializers`/datetime/dataclass all compose).
struct Lower<'py, 'r, 'c> {
    py: Python<'py>,
    represent: &'r Bound<'py, PyAny>,
    encode: EncodeCtx<'c>,
    sort_keys: bool,
    double_quotes: bool,
    schema: Schema,
    seen: HashSet<usize>,
    aliased: HashSet<usize>,
}

impl Lower<'_, '_, '_> {
    fn lower(&mut self, obj: &Bound<'_, PyAny>, depth: u32) -> PyResult<YamlNode> {
        // Grow the native stack on demand so a deeply nested object (bounded by
        // `MAX_REPRESENT_DEPTH`) cannot overflow a small thread stack. See
        // [`crate::stack`].
        crate::stack::guard(|| self.lower_inner(obj, depth))
    }

    fn lower_inner(&mut self, obj: &Bound<'_, PyAny>, depth: u32) -> PyResult<YamlNode> {
        if depth >= MAX_REPRESENT_DEPTH {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "object is too deeply nested to serialize (possible self-reference)",
            ));
        }

        // Only real containers (dict/list/tuple) are aliasable; scalars are never
        // shared the way PyYAML aliases (it ignores str/int/float/bool/None), and
        // interning would make their `id()` collide spuriously. A container seen
        // before becomes an alias; otherwise mark it before recursing so a cycle
        // (a container holding itself) resolves to an alias instead of looping.
        let container = is_container(obj);
        let id = obj.as_ptr() as usize;
        if container {
            if self.seen.contains(&id) {
                self.aliased.insert(id);
                return Ok(synthetic(
                    YamlNodeKind::Alias(id.to_string()),
                    NodeStyle::Block,
                    None,
                ));
            }
            self.seen.insert(id);
        }

        let mut node = self.build_node(obj, depth)?;
        if container {
            // Stamp the raw id; `name_anchors` later turns it into a real anchor
            // name or strips it if this object was never aliased.
            node.anchor = Some(id.to_string());
        }
        Ok(node)
    }

    /// Build the node for `obj` (representer result, or the built-in rendering
    /// when the host defers), without the shared-object bookkeeping.
    fn build_node(&mut self, obj: &Bound<'_, PyAny>, depth: u32) -> PyResult<YamlNode> {
        let described = self.represent.call1((obj,))?;
        if !described.is_none() {
            return self.descriptor_to_node(&described, depth);
        }

        // Deferred: recurse into containers so nested values still reach
        // `represent`; a leaf goes through the full `dumps` pipeline below.
        if let Ok(dict) = obj.cast::<PyDict>() {
            // Snapshot before recursing: `represent` runs arbitrary Python that
            // could mutate the dict mid-walk (which `PyDict_Next` forbids).
            let snapshot: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = dict.iter().collect();
            let ordered = self.sorted_pairs(snapshot)?;
            let mut pairs = Vec::with_capacity(ordered.len());
            for (k, v) in &ordered {
                pairs.push((self.lower(k, depth + 1)?, self.lower(v, depth + 1)?));
            }
            return Ok(synthetic(
                YamlNodeKind::Mapping(pairs),
                NodeStyle::Block,
                None,
            ));
        }
        if let Ok(list) = obj.cast::<PyList>() {
            let snapshot: Vec<Bound<'_, PyAny>> = list.iter().collect();
            let items = self.lower_all(&snapshot, depth)?;
            return Ok(synthetic(
                YamlNodeKind::Sequence(items),
                NodeStyle::Block,
                None,
            ));
        }
        if let Ok(tuple) = obj.cast::<PyTuple>() {
            let snapshot: Vec<Bound<'_, PyAny>> = tuple.iter().collect();
            let items = self.lower_all(&snapshot, depth)?;
            return Ok(synthetic(
                YamlNodeKind::Sequence(items),
                NodeStyle::Block,
                None,
            ));
        }

        // A deferred non-container leaf. Run it through the same encode pipeline a
        // plain `dumps` uses (`default`, `serializers`, datetime/dataclass/numpy
        // auto-serialization), then bridge the resulting `Value` to a node. This
        // is what makes deferred values render byte-for-byte like a plain `dumps`
        // and `represent` compose with `default`/`serializers`.
        let value = python_to_value(self.py, obj, self.encode)?;
        Ok(self.value_to_node(value))
    }

    /// Turn a `represent` return value (a `Scalar`/`Sequence`/`Mapping`) into a
    /// node.
    fn descriptor_to_node(
        &mut self,
        described: &Bound<'_, PyAny>,
        depth: u32,
    ) -> PyResult<YamlNode> {
        if let Ok(scalar) = described.cast::<YAMLRocksScalar>() {
            let scalar = scalar.borrow();
            let mut node = scalar_node(
                scalar.value.clone(),
                scalar.style,
                scalar.tag.as_deref(),
                self.double_quotes,
                self.schema,
            );
            // A scalar carries no children, so its span end stays at the start.
            node.end_offset = node.span.offset;
            return Ok(node);
        }
        if let Ok(seq) = described.cast::<YAMLRocksSequence>() {
            let seq = seq.borrow();
            let snapshot: Vec<Bound<'_, PyAny>> = seq
                .items
                .bind(self.py)
                .try_iter()?
                .collect::<PyResult<_>>()?;
            let items = self.lower_all(&snapshot, depth)?;
            return Ok(synthetic(
                YamlNodeKind::Sequence(items),
                flow_style(seq.flow),
                seq.tag.clone(),
            ));
        }
        if let Ok(map) = described.cast::<YAMLRocksMapping>() {
            let map = map.borrow();
            let mut entries: Vec<(Bound<'_, PyAny>, Bound<'_, PyAny>)> = Vec::new();
            for pair in map.pairs.bind(self.py).try_iter()? {
                let pair = pair?;
                entries.push((pair.get_item(0)?, pair.get_item(1)?));
            }
            let ordered = self.sorted_pairs(entries)?;
            let mut pairs = Vec::with_capacity(ordered.len());
            for (key, val) in &ordered {
                pairs.push((self.lower(key, depth + 1)?, self.lower(val, depth + 1)?));
            }
            return Ok(synthetic(
                YamlNodeKind::Mapping(pairs),
                flow_style(map.flow),
                map.tag.clone(),
            ));
        }
        Err(pyo3::exceptions::PyTypeError::new_err(
            "a represent callback must return a yamlrocks.YAMLRocksScalar, \
             YAMLRocksSequence, YAMLRocksMapping, or None",
        ))
    }

    /// Lower a snapshot of child objects, each through `represent`.
    fn lower_all(&mut self, items: &[Bound<'_, PyAny>], depth: u32) -> PyResult<Vec<YamlNode>> {
        items
            .iter()
            .map(|item| self.lower(item, depth + 1))
            .collect()
    }

    /// Order a mapping's `(key, value)` pairs for emission. With `sort_keys` set,
    /// sort by the key resolved through the encode pipeline and the fast path's
    /// key comparator, so the represent path orders keys identically to a plain
    /// `dumps` (numbers numerically, not lexically). Sorting happens here, before
    /// the values are lowered, so anchor detection follows the final emission
    /// order (sorting after would risk emitting an alias before its anchor). A key
    /// the pipeline cannot resolve keeps its input position (a stable no-op).
    fn sorted_pairs<'a>(
        &self,
        pairs: Vec<(Bound<'a, PyAny>, Bound<'a, PyAny>)>,
    ) -> PyResult<Vec<(Bound<'a, PyAny>, Bound<'a, PyAny>)>> {
        if !self.sort_keys {
            return Ok(pairs);
        }
        let mut keyed: Vec<(Option<Value<'static>>, Bound<'a, PyAny>, Bound<'a, PyAny>)> = pairs
            .into_iter()
            .map(|(k, v)| {
                let key = python_to_value(self.py, &k, self.encode).ok();
                (key, k, v)
            })
            .collect();
        keyed.sort_by(|(a, _, _), (b, _, _)| match (a, b) {
            (Some(a), Some(b)) => crate::encode::compare_keys(a, b),
            _ => std::cmp::Ordering::Equal,
        });
        Ok(keyed.into_iter().map(|(_, k, v)| (k, v)).collect())
    }

    /// Bridge a fast-path [`Value`] (from the deferred-leaf pipeline) into a
    /// synthetic node, choosing the same styles the fast encoder would so the
    /// round-trip emitter reproduces its bytes.
    fn value_to_node(&self, value: Value<'_>) -> YamlNode {
        // Grow the native stack on demand: the `Value` tree is bounded by the
        // encoder's depth guard, but the walk still recurses per level.
        crate::stack::guard(|| self.value_to_node_inner(value))
    }

    fn value_to_node_inner(&self, value: Value<'_>) -> YamlNode {
        use crate::decode::Value as V;
        match value {
            V::Null => synthetic(YamlNodeKind::Null, NodeStyle::Block, None),
            V::Bool(b) => scalar_plain(if b { "true" } else { "false" }),
            V::Int(i) => scalar_plain(&i.to_string()),
            V::BigInt(s) => scalar_plain(&s),
            V::Float(f) => scalar_plain(&crate::emit_util::canonical_float(f)),
            V::String(s) => {
                let style = auto_string_style(&s, self.double_quotes, self.schema);
                synthetic(
                    YamlNodeKind::Scalar(s.into_owned(), style),
                    NodeStyle::Block,
                    None,
                )
            }
            V::Timestamp(ts) => scalar_plain(&ts.to_iso()),
            V::Sequence(items) => {
                let items = items.into_iter().map(|v| self.value_to_node(v)).collect();
                synthetic(YamlNodeKind::Sequence(items), NodeStyle::Block, None)
            }
            V::Mapping(pairs) => {
                let pairs = pairs
                    .into_iter()
                    .map(|(k, v)| (self.value_to_node(k), self.value_to_node(v)))
                    .collect();
                synthetic(YamlNodeKind::Mapping(pairs), NodeStyle::Block, None)
            }
            V::Tagged(tag, inner) => {
                let mut node = self.value_to_node(*inner);
                node.tag = Some(tag);
                node
            }
        }
    }
}

/// Whether `obj` is a container PyYAML would consider for aliasing (a mapping or
/// sequence); scalars and everything else are never aliased.
fn is_container(obj: &Bound<'_, PyAny>) -> bool {
    obj.is_instance_of::<PyDict>()
        || obj.is_instance_of::<PyList>()
        || obj.is_instance_of::<PyTuple>()
}

/// Replace the raw `id()` markers left on container nodes with real anchor names.
/// A container that turned up more than once (in `aliased`) gets a sequential
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

/// A `flow=True` override maps to flow layout; `False` and the unset default map
/// to block, the dominant configuration style.
fn flow_style(flow: Option<bool>) -> NodeStyle {
    if flow == Some(true) {
        NodeStyle::Flow
    } else {
        NodeStyle::Block
    }
}

/// Build a scalar node from a descriptor's value, style, and tag, applying
/// PyYAML-faithful auto styling when the style is unset.
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
fn scalar_node(
    value: String,
    style: Option<ScalarStyle>,
    tag: Option<&str>,
    double_quotes: bool,
    schema: Schema,
) -> YamlNode {
    if let Some(style) = style {
        return synthetic(
            YamlNodeKind::Scalar(value, style),
            NodeStyle::Block,
            tag.map(str::to_owned),
        );
    }
    let Some(tag) = tag else {
        let style = auto_string_style(&value, double_quotes, schema);
        return synthetic(YamlNodeKind::Scalar(value, style), NodeStyle::Block, None);
    };
    let plain_kind = schema.classify(&value, ScalarStyle::Plain, None);
    match standard_tag_kind(tag) {
        // The plain value already resolves to the tag's type: the tag is
        // redundant, so drop it. A string still runs through the quoting rule
        // (a plain `[x]` would be unsafe); a bool/int/float/null token is
        // inherently plain-safe (`true`, `1.0e17`), so emit it plain.
        Some(std) if kind_matches(plain_kind, std) => {
            let style = if matches!(std, StdKind::Str) {
                auto_string_style(&value, double_quotes, schema)
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
    }
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
/// PyYAML's default, unless a line break forces double quotes.
fn forced_quote_style(value: &str) -> ScalarStyle {
    if value.contains('\n') || value.contains('\r') {
        ScalarStyle::DoubleQuoted
    } else {
        ScalarStyle::SingleQuoted
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
