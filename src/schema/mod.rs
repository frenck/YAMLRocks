//! A focused JSON Schema validator that runs against the round-trip AST.
//!
//! Validating the [`YamlNode`] tree (rather than the plain `Value` tree) lets
//! every error carry the source location of the offending node, so schema
//! failures report a precise line and column.
//!
//! A practical subset of JSON Schema (draft 7-ish) is supported: `type`,
//! `enum`, `const`, `properties`, `required`, `additionalProperties` (boolean),
//! `items`, `minimum`/`maximum`, `exclusiveMinimum`/`exclusiveMaximum`,
//! `minLength`/`maxLength`, `minItems`/`maxItems`, the `allOf`/`anyOf`/`oneOf`/
//! `not` combinators, and `$ref` to local `#/...` pointers (including
//! `#/$defs/...`). Other keywords are ignored; an unresolvable `$ref` is an
//! error rather than silently permissive.

mod directive;

pub use directive::schema_ref;

use std::collections::HashMap;

use crate::decode::Value;
use crate::resolver::ResolvedValue;
use crate::roundtrip::{YamlNode, YamlNodeKind};
use crate::scanner::Span;

/// Alias-hop bound, fast-failing a pure alias cycle. Counts only alias follows,
/// not tree depth, so a legitimately deep document is not truncated.
const MAX_ALIAS_DEPTH: usize = 100;

/// Total node budget for alias expansion, shared across the whole tree. This is
/// the real bound against an alias bomb (a small document whose aliases expand
/// exponentially): the per-hop depth cap cannot catch a wide expansion. Mirrors
/// the fast path's `MAX_NODES` and the round-trip `anchors` budget so the schema
/// validator cannot be turned into an uncatchable process abort.
const MAX_ALIAS_NODES: usize = 1_000_000;

/// Maximum chain of `$ref` follows before validation gives up, fast-failing a
/// straight cyclic reference (`#/$defs/a` whose schema is `{"$ref": "#/$defs/a"}`).
/// Counts only consecutive `$ref` hops at one node; descending into a child node
/// resets it, so a legitimately deep document with many refs is not truncated.
const MAX_REF_DEPTH: usize = 128;

/// Total `$ref` follows allowed across the whole validation, shared through the
/// walk. The per-chain [`MAX_REF_DEPTH`] cannot catch a *branching* cycle (a
/// `$ref` reached twice from one node through `allOf`, which doubles the work at
/// every level for `2^depth` total calls); this budget does, mirroring the
/// alias-expansion [`MAX_ALIAS_NODES`] bound. Set far above any real schema.
const MAX_REF_FOLLOWS: usize = 100_000;

/// Validation context threaded through the recursive walk: the root schema (for
/// resolving `#/...` `$ref` pointers, which are document-relative), the scalar
/// schema to resolve leaves with, the current `$ref` chain depth, and a shared
/// total-follow budget. Cheap to copy (references plus two scalars); the budget
/// is a shared cell so every branch of the walk draws from one pool.
#[derive(Clone, Copy)]
struct Ctx<'a, 'v> {
    root: &'a Value<'v>,
    yaml_11: bool,
    ref_depth: usize,
    ref_budget: &'a std::cell::Cell<usize>,
}

impl<'a, 'v> Ctx<'a, 'v> {
    /// Context for validating a child node: the `$ref` chain depth resets because
    /// descending the document is finite progress, not a reference cycle. The
    /// shared follow budget carries over (it bounds total work, not depth).
    fn child(self) -> Self {
        Self {
            ref_depth: 0,
            ..self
        }
    }
}

/// A single schema validation failure.
pub struct SchemaError {
    pub message: String,
    pub span: Span,
    pub path: String,
}

/// Validate `node` against `schema`, returning all collected errors.
///
/// `yaml_11` selects the scalar schema used to resolve leaf values, so a YAML
/// 1.1 document is type-checked the way it actually parses (`yes` as a boolean,
/// `0777` as an octal int).
pub fn validate(node: &YamlNode, schema: &Value, yaml_11: bool) -> Vec<SchemaError> {
    // Resolve anchors/aliases up front so an aliased node validates as the value
    // it actually refers to, matching fast-path `loads()`. Without this, every
    // alias would look like `null` to the validator.
    let mut anchors = HashMap::new();
    collect_anchors(node, &mut anchors);
    let mut budget = MAX_ALIAS_NODES;
    let resolved = expand_aliases(node, &anchors, 0, &mut budget);

    let mut errors = Vec::new();
    let ref_budget = std::cell::Cell::new(MAX_REF_FOLLOWS);
    let ctx = Ctx {
        root: schema,
        yaml_11,
        ref_depth: 0,
        ref_budget: &ref_budget,
    };
    validate_node(&resolved, schema, "$", ctx, &mut errors);
    errors
}

/// Collect every `&anchor` in the tree to its node, so aliases can be expanded.
fn collect_anchors<'a>(node: &'a YamlNode, anchors: &mut HashMap<String, &'a YamlNode>) {
    // Grow the native stack on demand so a deeply nested document cannot overflow
    // a small thread stack during the walk. See [`crate::stack`].
    crate::stack::guard(|| collect_anchors_inner(node, anchors))
}

fn collect_anchors_inner<'a>(node: &'a YamlNode, anchors: &mut HashMap<String, &'a YamlNode>) {
    if let Some(name) = &node.anchor {
        anchors.entry(name.clone()).or_insert(node);
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                collect_anchors(key, anchors);
                collect_anchors(val, anchors);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_anchors(item, anchors);
            }
        }
        _ => {}
    }
}

/// Clone the tree, replacing each `*alias` with the node its anchor names so
/// the validator sees concrete values. `depth` counts only alias hops and
/// fast-fails a pure cycle; `budget` is a total node count shared across the
/// whole tree and is the real bound against an alias bomb. Unknown aliases,
/// chains past [`MAX_ALIAS_DEPTH`], and expansions past [`MAX_ALIAS_NODES`]
/// become null rather than recursing forever or exhausting memory.
fn expand_aliases(
    node: &YamlNode,
    anchors: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    // Grow the native stack on demand so expanding a deeply nested tree cannot
    // overflow a small thread stack; the recursion re-enters here per level. See
    // [`crate::stack`].
    crate::stack::guard(|| expand_aliases_inner(node, anchors, depth, budget))
}

fn expand_aliases_inner(
    node: &YamlNode,
    anchors: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    if depth > MAX_ALIAS_DEPTH || *budget == 0 {
        return YamlNode::new(YamlNodeKind::Null, node.span);
    }
    *budget -= 1;
    match &node.kind {
        YamlNodeKind::Alias(name) => match anchors.get(name) {
            Some(target) => {
                let mut resolved = expand_aliases(target, anchors, depth + 1, budget);
                resolved.span = node.span;
                resolved
            }
            None => YamlNode::new(YamlNodeKind::Null, node.span),
        },
        YamlNodeKind::Mapping(pairs) => {
            let resolved = pairs
                .iter()
                .map(|(k, v)| {
                    (
                        expand_aliases(k, anchors, depth, budget),
                        expand_aliases(v, anchors, depth, budget),
                    )
                })
                .collect();
            let mut out = YamlNode::new(YamlNodeKind::Mapping(resolved), node.span);
            out.tag = node.tag.clone();
            out
        }
        YamlNodeKind::Sequence(items) => {
            let resolved = items
                .iter()
                .map(|item| expand_aliases(item, anchors, depth, budget))
                .collect();
            let mut out = YamlNode::new(YamlNodeKind::Sequence(resolved), node.span);
            out.tag = node.tag.clone();
            out
        }
        _ => node.clone(),
    }
}

fn validate_node(
    node: &YamlNode,
    schema: &Value,
    path: &str,
    ctx: Ctx<'_, '_>,
    errors: &mut Vec<SchemaError>,
) {
    // Grow the native stack on demand so validating a deeply nested document
    // cannot overflow a small thread stack; the recursion re-enters here per
    // level. See [`crate::stack`].
    crate::stack::guard(|| validate_node_inner(node, schema, path, ctx, errors))
}

fn validate_node_inner(
    node: &YamlNode,
    schema: &Value,
    path: &str,
    ctx: Ctx<'_, '_>,
    errors: &mut Vec<SchemaError>,
) {
    // A boolean schema: `true` accepts anything, `false` rejects everything.
    if let Value::Bool(accept) = schema {
        if !*accept {
            errors.push(err(node, path, "value is not allowed (schema is false)"));
        }
        return;
    }

    let Value::Mapping(_) = schema else {
        return; // non-object schemas other than booleans are treated as permissive
    };

    // `$ref`: resolve the `#/...` pointer against the root schema and validate
    // against the target. Draft-07 semantics: a `$ref` ignores any sibling
    // keywords, so this returns afterward. An unresolvable reference is an error
    // (not silently permissive). A straight `$ref` cycle is cut by the per-chain
    // depth bound; a cycle that branches through a combinator is cut by the
    // shared total-follow budget, which a depth cap alone cannot bound.
    if let Some(Value::String(reference)) = get(schema, "$ref") {
        if ctx.ref_depth >= MAX_REF_DEPTH || ctx.ref_budget.get() == 0 {
            errors.push(err(
                node,
                path,
                "schema $ref nesting too deep (cyclic reference?)",
            ));
            return;
        }
        ctx.ref_budget.set(ctx.ref_budget.get() - 1);
        match resolve_ref(ctx.root, reference) {
            Some(target) => {
                let next = Ctx {
                    ref_depth: ctx.ref_depth + 1,
                    ..ctx
                };
                validate_node(node, target, path, next, errors);
            }
            None => errors.push(err(
                node,
                path,
                &format!("unresolvable schema $ref '{reference}'"),
            )),
        }
        return;
    }

    let resolved = resolve(node, ctx.yaml_11);

    if let Some(type_schema) = get(schema, "type") {
        check_type(node, &resolved, type_schema, path, errors);
    }
    // `enum`/`const` compare the *whole* value, nested containers included, so
    // they need the fully resolved structure. The shallow `resolved` above keeps
    // every sequence/mapping empty (enough for `type`/numeric/string checks but
    // not for equality), so resolve deeply here, only when one is present. The
    // deep tree is dropped iteratively (`drop_value_tree`): its depth is bounded
    // only by `MAX_DEPTH` over untrusted input, so a recursive teardown could
    // overflow a small thread stack and, under `panic = "abort"`, abort.
    if let Some(Value::Sequence(options)) = get(schema, "enum") {
        let value = resolve_deep(node, ctx.yaml_11);
        if !options.iter().any(|opt| value_eq(opt, &value)) {
            errors.push(err(
                node,
                path,
                "value is not one of the allowed enum values",
            ));
        }
        crate::stack::drop_value_tree(value);
    }
    if let Some(const_schema) = get(schema, "const") {
        let value = resolve_deep(node, ctx.yaml_11);
        if !value_eq(const_schema, &value) {
            errors.push(err(node, path, "value does not equal the required const"));
        }
        crate::stack::drop_value_tree(value);
    }

    check_numeric(node, &resolved, schema, path, errors);
    check_string(node, &resolved, schema, path, errors);
    check_object(node, schema, path, ctx, errors);
    check_array(node, schema, path, ctx, errors);
    check_combinators(node, schema, path, ctx, errors);
}

fn check_type(
    node: &YamlNode,
    resolved: &Value,
    type_schema: &Value,
    path: &str,
    errors: &mut Vec<SchemaError>,
) {
    let matches = match type_schema {
        Value::String(t) => type_matches(t, resolved),
        Value::Sequence(types) => types
            .iter()
            .any(|t| matches!(t, Value::String(name) if type_matches(name, resolved))),
        _ => true,
    };
    if !matches {
        let expected = match type_schema {
            Value::String(t) => t.to_string(),
            Value::Sequence(types) => types
                .iter()
                .filter_map(|t| match t {
                    Value::String(s) => Some(s.as_ref()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "?".to_owned(),
        };
        errors.push(err(
            node,
            path,
            &format!("expected type {expected}, found {}", type_name(resolved)),
        ));
    }
}

fn type_matches(name: &str, value: &Value) -> bool {
    match name {
        "null" => matches!(value, Value::Null),
        "boolean" => matches!(value, Value::Bool(_)),
        // `BigInt` is an integer too large for `i64`; it is still an integer (and
        // a number) for schema typing.
        "integer" => matches!(value, Value::Int(_) | Value::BigInt(_)),
        "number" => matches!(value, Value::Int(_) | Value::Float(_) | Value::BigInt(_)),
        "string" => matches!(value, Value::String(_)),
        "array" => matches!(value, Value::Sequence(_)),
        "object" => matches!(value, Value::Mapping(_)),
        _ => true,
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Int(_) | Value::BigInt(_) => "integer",
        Value::Float(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "array",
        Value::Mapping(_) => "object",
        Value::Tagged(_, inner) => type_name(inner),
    }
}

fn check_numeric(
    node: &YamlNode,
    resolved: &Value,
    schema: &Value,
    path: &str,
    errors: &mut Vec<SchemaError>,
) {
    let Some(num) = as_f64(resolved) else { return };
    if let Some(min) = get(schema, "minimum").and_then(as_f64) {
        if num < min {
            errors.push(err(
                node,
                path,
                &format!("value {num} is less than minimum {min}"),
            ));
        }
    }
    if let Some(max) = get(schema, "maximum").and_then(as_f64) {
        if num > max {
            errors.push(err(
                node,
                path,
                &format!("value {num} is greater than maximum {max}"),
            ));
        }
    }
    if let Some(v) = get(schema, "exclusiveMinimum") {
        if let Some(min) = as_f64(v) {
            if num <= min {
                errors.push(err(
                    node,
                    path,
                    &format!("value {num} is not greater than {min}"),
                ));
            }
        }
    }
    if let Some(v) = get(schema, "exclusiveMaximum") {
        if let Some(max) = as_f64(v) {
            if num >= max {
                errors.push(err(
                    node,
                    path,
                    &format!("value {num} is not less than {max}"),
                ));
            }
        }
    }
}

fn check_string(
    node: &YamlNode,
    resolved: &Value,
    schema: &Value,
    path: &str,
    errors: &mut Vec<SchemaError>,
) {
    let Value::String(s) = resolved else { return };
    let len = s.chars().count() as i64;
    if let Some(min) = get(schema, "minLength").and_then(as_count_bound) {
        if len < min {
            errors.push(err(
                node,
                path,
                &format!("string is shorter than minLength {min}"),
            ));
        }
    }
    if let Some(max) = get(schema, "maxLength").and_then(as_count_bound) {
        if len > max {
            errors.push(err(
                node,
                path,
                &format!("string is longer than maxLength {max}"),
            ));
        }
    }
}

fn check_object(
    node: &YamlNode,
    schema: &Value,
    path: &str,
    ctx: Ctx<'_, '_>,
    errors: &mut Vec<SchemaError>,
) {
    let YamlNodeKind::Mapping(pairs) = &node.kind else {
        return;
    };

    let properties = get(schema, "properties");

    // required
    if let Some(Value::Sequence(required)) = get(schema, "required") {
        for req in required {
            if let Value::String(name) = req {
                if !pairs.iter().any(|(k, _)| scalar_key_eq(k, name)) {
                    errors.push(err(
                        node,
                        path,
                        &format!("missing required property '{name}'"),
                    ));
                }
            }
        }
    }

    // properties + additionalProperties
    let additional = get(schema, "additionalProperties");
    for (key, val) in pairs {
        let Some(key_name) = scalar_key_name(key) else {
            continue;
        };
        let child_path = format!("{path}.{key_name}");
        let mut matched = false;
        if let Some(props) = properties {
            if let Some(subschema) = get(props, &key_name) {
                matched = true;
                validate_node(val, subschema, &child_path, ctx.child(), errors);
            }
        }
        if !matched {
            match additional {
                Some(Value::Bool(false)) => errors.push(err(
                    val,
                    &child_path,
                    &format!("additional property '{key_name}' is not allowed"),
                )),
                Some(sub @ Value::Mapping(_)) => {
                    validate_node(val, sub, &child_path, ctx.child(), errors)
                }
                _ => {}
            }
        }
    }
}

fn check_array(
    node: &YamlNode,
    schema: &Value,
    path: &str,
    ctx: Ctx<'_, '_>,
    errors: &mut Vec<SchemaError>,
) {
    let YamlNodeKind::Sequence(items) = &node.kind else {
        return;
    };
    let len = items.len() as i64;

    if let Some(min) = get(schema, "minItems").and_then(as_count_bound) {
        if len < min {
            errors.push(err(
                node,
                path,
                &format!("array has fewer than minItems {min}"),
            ));
        }
    }
    if let Some(max) = get(schema, "maxItems").and_then(as_count_bound) {
        if len > max {
            errors.push(err(
                node,
                path,
                &format!("array has more than maxItems {max}"),
            ));
        }
    }
    if let Some(items_schema) = get(schema, "items") {
        for (i, item) in items.iter().enumerate() {
            let child_path = format!("{path}[{i}]");
            validate_node(item, items_schema, &child_path, ctx.child(), errors);
        }
    }
}

fn check_combinators(
    node: &YamlNode,
    schema: &Value,
    path: &str,
    ctx: Ctx<'_, '_>,
    errors: &mut Vec<SchemaError>,
) {
    // Combinator sub-schemas apply to the *same* node, so `ctx` passes through
    // unchanged: the `$ref` depth is not reset (a sub-schema that is itself a
    // `$ref` still counts toward the cycle bound) and not incremented here (the
    // increment happens in `validate_node_inner` when a `$ref` is actually
    // followed).
    if let Some(Value::Sequence(subs)) = get(schema, "allOf") {
        for sub in subs {
            validate_node(node, sub, path, ctx, errors);
        }
    }
    if let Some(Value::Sequence(subs)) = get(schema, "anyOf") {
        let any = subs.iter().any(|sub| {
            let mut tmp = Vec::new();
            validate_node(node, sub, path, ctx, &mut tmp);
            tmp.is_empty()
        });
        if !any {
            errors.push(err(
                node,
                path,
                "value does not match any of the anyOf schemas",
            ));
        }
    }
    if let Some(Value::Sequence(subs)) = get(schema, "oneOf") {
        let count = subs
            .iter()
            .filter(|sub| {
                let mut tmp = Vec::new();
                validate_node(node, sub, path, ctx, &mut tmp);
                tmp.is_empty()
            })
            .count();
        if count != 1 {
            errors.push(err(
                node,
                path,
                &format!("value matches {count} of the oneOf schemas (exactly one required)"),
            ));
        }
    }
    if let Some(sub) = get(schema, "not") {
        let mut tmp = Vec::new();
        validate_node(node, sub, path, ctx, &mut tmp);
        if tmp.is_empty() {
            errors.push(err(node, path, "value must not match the 'not' schema"));
        }
    }
}

// -- Helpers --

/// Resolve a node to a plain `Value` for comparison and type checks. Schema
/// validation uses the spec schema (1.1 or 1.2); the PyYAML-compat bool variant
/// is a parse-time interop concern and does not affect type validation here.
fn resolve(node: &YamlNode, yaml_11: bool) -> Value<'static> {
    match &node.kind {
        YamlNodeKind::Null => Value::Null,
        YamlNodeKind::Scalar(text, style) => {
            let resolved = crate::resolver::Schema::new(yaml_11, false).resolve(
                text,
                *style,
                node.tag.as_deref(),
            );
            match resolved {
                ResolvedValue::Null => Value::Null,
                ResolvedValue::Bool(b) => Value::Bool(b),
                ResolvedValue::Int(i) => Value::Int(i),
                ResolvedValue::BigInt(s) => Value::BigInt(s.into()),
                ResolvedValue::Float(f) => Value::Float(f),
                ResolvedValue::String(s) => Value::String(s.into()),
            }
        }
        YamlNodeKind::Sequence(_) => Value::Sequence(Vec::new()),
        YamlNodeKind::Mapping(_) => Value::Mapping(Vec::new()),
        YamlNodeKind::Alias(_) => Value::Null,
    }
}

/// Like [`resolve`], but recurses into sequences and mappings so the full nested
/// structure is materialized. Used only by `enum`/`const`, which compare the
/// whole value; the shallow [`resolve`] leaves containers empty, which would make
/// `const {}` match any mapping and a non-empty `const`/`enum` match nothing.
fn resolve_deep(node: &YamlNode, yaml_11: bool) -> Value<'static> {
    // Grow the native stack on demand: this recurses once per nesting level over
    // the (depth-bounded) AST, matching `validate_node`. See [`crate::stack`].
    crate::stack::guard(|| match &node.kind {
        YamlNodeKind::Sequence(items) => {
            Value::Sequence(items.iter().map(|it| resolve_deep(it, yaml_11)).collect())
        }
        YamlNodeKind::Mapping(pairs) => Value::Mapping(
            pairs
                .iter()
                .map(|(k, v)| (resolve_deep(k, yaml_11), resolve_deep(v, yaml_11)))
                .collect(),
        ),
        _ => resolve(node, yaml_11),
    })
}

/// Resolve a JSON Schema `$ref` against the root schema. Only local pointers are
/// supported: a `#` fragment naming a path within the same document (the common
/// `#/$defs/name` / `#/definitions/name` form, or any deeper path). Returns the
/// referenced subschema, or `None` for a remote reference (one not starting with
/// `#`, which would need a network or external resolver) or a pointer that does
/// not exist. `None` is reported by the caller as an error, never silently
/// treated as a permissive schema.
fn resolve_ref<'a, 'v>(root: &'a Value<'v>, reference: &str) -> Option<&'a Value<'v>> {
    let pointer = reference.strip_prefix('#')?;
    if pointer.is_empty() {
        return Some(root); // `#` is the whole schema
    }
    // A non-empty fragment must be a JSON Pointer (`#/...`).
    let pointer = pointer.strip_prefix('/')?;
    let mut current = root;
    for raw in pointer.split('/') {
        // JSON Pointer unescaping: `~1` -> `/`, then `~0` -> `~` (RFC 6901).
        let token = raw.replace("~1", "/").replace("~0", "~");
        current = match current {
            Value::Mapping(_) => get(current, &token)?,
            Value::Sequence(items) => items.get(token.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Look up a key in an object schema.
fn get<'a, 'v>(schema: &'a Value<'v>, key: &str) -> Option<&'a Value<'v>> {
    match schema {
        Value::Mapping(pairs) => pairs
            .iter()
            .find(|(k, _)| matches!(k, Value::String(s) if s.as_ref() == key))
            .map(|(_, v)| v),
        _ => None,
    }
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        // A big integer is compared lossily against `minimum`/`maximum`, matching
        // how a JSON Schema validator coerces it to a number for bound checks.
        Value::BigInt(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// A non-negative count bound (`minLength`/`maxItems` and friends), accepting an
/// integer or an integer-valued float, as JSON Schema permits either spelling.
fn as_count_bound(value: &Value) -> Option<i64> {
    match value {
        Value::Int(i) => Some(*i),
        Value::Float(f) if f.fract() == 0.0 && f.is_finite() => Some(*f as i64),
        _ => None,
    }
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(i), Value::Float(f)) | (Value::Float(f), Value::Int(i)) => *i as f64 == *f,
        // Arrays are equal element-wise, in order.
        (Value::Sequence(xs), Value::Sequence(ys)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| value_eq(x, y))
        }
        // Objects are equal as sets of pairs: order does not matter, but every
        // pair on one side must have an equal pair on the other (sizes match, and
        // a loaded mapping has no duplicate keys, so this is true set equality).
        (Value::Mapping(xs), Value::Mapping(ys)) => {
            xs.len() == ys.len()
                && xs.iter().all(|(xk, xv)| {
                    ys.iter()
                        .any(|(yk, yv)| value_eq(xk, yk) && value_eq(xv, yv))
                })
        }
        _ => a == b,
    }
}

fn scalar_key_eq(key: &YamlNode, name: &str) -> bool {
    matches!(&key.kind, YamlNodeKind::Scalar(s, _) if s == name)
}

fn scalar_key_name(key: &YamlNode) -> Option<String> {
    match &key.kind {
        YamlNodeKind::Scalar(s, _) => Some(s.clone()),
        _ => None,
    }
}

fn err(node: &YamlNode, path: &str, message: &str) -> SchemaError {
    SchemaError {
        message: message.to_owned(),
        span: node.span,
        path: path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::decode::Value;
    use crate::roundtrip::composer::compose;
    use crate::roundtrip::YamlNode;

    fn node(src: &str) -> YamlNode {
        compose(src).unwrap().into_iter().next().unwrap()
    }
    fn s(text: &str) -> Value<'static> {
        Value::String(text.to_owned().into())
    }
    fn obj(pairs: Vec<(&str, Value<'static>)>) -> Value<'static> {
        Value::Mapping(pairs.into_iter().map(|(k, v)| (s(k), v)).collect())
    }
    fn errors(src: &str, schema: &Value) -> usize {
        validate(&node(src), schema, false).len()
    }

    #[test]
    fn type_check() {
        let schema = obj(vec![("type", s("integer"))]);
        assert_eq!(errors("42", &schema), 0);
        assert!(errors("hello", &schema) > 0);
    }

    #[test]
    fn required_keys() {
        let schema = obj(vec![
            ("type", s("object")),
            ("required", Value::Sequence(vec![s("a")])),
        ]);
        assert_eq!(errors("a: 1\n", &schema), 0);
        assert!(errors("b: 1\n", &schema) > 0);
    }

    #[test]
    fn enum_membership() {
        let schema = obj(vec![("enum", Value::Sequence(vec![s("debug"), s("info")]))]);
        assert_eq!(errors("info", &schema), 0);
        assert!(errors("verbose", &schema) > 0);
    }

    #[test]
    fn numeric_bounds() {
        let schema = obj(vec![("type", s("integer")), ("maximum", Value::Int(100))]);
        assert_eq!(errors("50", &schema), 0);
        assert!(errors("200", &schema) > 0);
    }

    #[test]
    fn nested_property_error_reports_json_path() {
        let schema = obj(vec![
            ("type", s("object")),
            (
                "properties",
                obj(vec![("port", obj(vec![("type", s("integer"))]))]),
            ),
        ]);
        let report = validate(&node("port: not_an_int\n"), &schema, false);
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].path, "$.port");
    }

    #[test]
    fn additional_properties_false() {
        let schema = obj(vec![
            ("type", s("object")),
            (
                "properties",
                obj(vec![("a", obj(vec![("type", s("integer"))]))]),
            ),
            ("additionalProperties", Value::Bool(false)),
        ]);
        assert_eq!(errors("a: 1\n", &schema), 0);
        assert!(errors("a: 1\nb: 2\n", &schema) > 0);
    }

    #[test]
    fn anyof_combinator() {
        let schema = obj(vec![(
            "anyOf",
            Value::Sequence(vec![
                obj(vec![("type", s("string"))]),
                obj(vec![("type", s("integer"))]),
            ]),
        )]);
        assert_eq!(errors("7", &schema), 0); // integer branch
        assert_eq!(errors("hello", &schema), 0); // string branch
        assert!(errors("1.5", &schema) > 0); // a float matches neither
    }

    #[test]
    fn aliases_are_resolved_before_validation() {
        // The aliased value must validate as the integer it refers to, not null.
        let schema = obj(vec![
            ("type", s("object")),
            (
                "properties",
                obj(vec![("use", obj(vec![("type", s("integer"))]))]),
            ),
        ]);
        assert_eq!(errors("base: &a 7\nuse: *a\n", &schema), 0);
    }
}
