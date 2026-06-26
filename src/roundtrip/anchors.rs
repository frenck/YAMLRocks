//! Anchor and alias resolution over the round-trip AST.
//!
//! Pure logic (no Python) used by `YAMLRocksDocument`/`YAMLRocksNode`: locating a `&anchor`'s
//! defining path, ordering paths in document order so an alias can only point
//! back at an earlier anchor, detaching an alias into an independent deep copy,
//! and building the anchor map that the AST-to-Python conversion expands aliases
//! against.

use std::collections::HashMap;

use crate::roundtrip::ast::{YamlNode, YamlNodeKind};

use super::document::{child_ref, resolve_path, scalar_eq, PathSeg};

pub(crate) fn alias_target_path(roots: &[YamlNode], path: &[PathSeg]) -> Option<Vec<PathSeg>> {
    match &resolve_path(roots, path)?.kind {
        YamlNodeKind::Alias(name) => find_anchor_path(roots, name),
        _ => None,
    }
}

/// Whether path `a` strictly precedes path `b` in document (emission) order.
///
/// Walks both paths from the root: an ancestor precedes its descendants, and at
/// the first diverging step the order is decided by position within the shared
/// container (key order for a mapping, index for a sequence). Used to enforce
/// that an alias's anchor is defined before the alias.
pub(crate) fn path_precedes(roots: &[YamlNode], a: &[PathSeg], b: &[PathSeg]) -> bool {
    let mut node = roots.first();
    let mut i = 0;
    loop {
        match (a.get(i), b.get(i)) {
            (None, None) => return false,    // same node: not strictly before
            (None, Some(_)) => return true,  // `a` is an ancestor of `b`
            (Some(_), None) => return false, // `b` is an ancestor of `a`
            (Some(sa), Some(sb)) => {
                if sa == sb {
                    node = node.and_then(|n| child_ref(n, sa));
                    i += 1;
                    continue;
                }
                return seg_order(node, sa) < seg_order(node, sb);
            }
        }
    }
}

/// Position of `seg` within its container `node` (key index for a mapping,
/// element index for a sequence). Missing entries sort last.
pub(crate) fn seg_order(node: Option<&YamlNode>, seg: &PathSeg) -> usize {
    match (node.map(|n| &n.kind), seg) {
        // A key node and its value share the entry's position. Comparing the two
        // (only reachable for a same-entry self-reference like `&a k: *a`) yields
        // "neither precedes", which is acceptable: the order matters only when
        // creating a new alias via the edit API, never on load.
        (Some(YamlNodeKind::Mapping(pairs)), PathSeg::Key(k) | PathSeg::KeyNode(k)) => pairs
            .iter()
            .position(|(key, _)| scalar_eq(key, k))
            .unwrap_or(usize::MAX),
        (_, PathSeg::Index(i)) => *i,
        _ => usize::MAX,
    }
}

/// The path to the node carrying `&name` in the first document, by document
/// order (the first match wins; well-formed documents have unique anchors).
pub(crate) fn find_anchor_path(roots: &[YamlNode], name: &str) -> Option<Vec<PathSeg>> {
    let mut found = None;
    walk_anchor(roots.first()?, name, &mut Vec::new(), &mut found);
    found
}

pub(crate) fn walk_anchor(
    node: &YamlNode,
    name: &str,
    prefix: &mut Vec<PathSeg>,
    found: &mut Option<Vec<PathSeg>>,
) {
    // Grow the native stack on demand: this recurses once per nesting level over
    // attacker-controlled AST depth. See [`crate::stack`].
    crate::stack::guard(|| walk_anchor_inner(node, name, prefix, found))
}

fn walk_anchor_inner(
    node: &YamlNode,
    name: &str,
    prefix: &mut Vec<PathSeg>,
    found: &mut Option<Vec<PathSeg>>,
) {
    if found.is_some() {
        return;
    }
    if node.anchor.as_deref() == Some(name) {
        *found = Some(prefix.clone());
        return;
    }
    for_each_child(node, prefix, &mut |child, p| {
        walk_anchor(child, name, p, found)
    });
}

/// Collect `(anchor_name, path)` for every `&name` definition in `node`.
pub(crate) fn collect_anchor_paths(
    node: &YamlNode,
    prefix: &mut Vec<PathSeg>,
    out: &mut Vec<(String, Vec<PathSeg>)>,
) {
    if let Some(name) = &node.anchor {
        out.push((name.clone(), prefix.clone()));
    }
    for_each_child(node, prefix, &mut |child, p| {
        collect_anchor_paths(child, p, out)
    });
}

/// Collect the path of every `*name` alias referencing `name` in `node`.
pub(crate) fn collect_alias_paths(
    node: &YamlNode,
    name: &str,
    prefix: &mut Vec<PathSeg>,
    out: &mut Vec<Vec<PathSeg>>,
) {
    if let YamlNodeKind::Alias(n) = &node.kind {
        if n == name {
            out.push(prefix.clone());
        }
        return;
    }
    for_each_child(node, prefix, &mut |child, p| {
        collect_alias_paths(child, name, p, out)
    });
}

/// Visit each child of `node` that anchor/alias traversal can address: a mapping
/// key node (`PathSeg::KeyNode`), its value (`PathSeg::Key`), and a sequence
/// element (`PathSeg::Index`), pushing the segment onto `prefix` for the call.
/// The key is visited before its value, matching document order. Only scalar
/// keys are addressable; a complex (mapping/sequence) key and its value are
/// skipped, as the path model has no segment for them.
pub(crate) fn for_each_child(
    node: &YamlNode,
    prefix: &mut Vec<PathSeg>,
    visit: &mut dyn FnMut(&YamlNode, &mut Vec<PathSeg>),
) {
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (key, val) in pairs {
                if let YamlNodeKind::Scalar(name, _) = &key.kind {
                    // The key node first: an anchor may sit on it (`&a foo: bar`).
                    prefix.push(PathSeg::KeyNode(name.clone()));
                    visit(key, prefix);
                    prefix.pop();
                    prefix.push(PathSeg::Key(name.clone()));
                    visit(val, prefix);
                    prefix.pop();
                }
            }
        }
        YamlNodeKind::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                prefix.push(PathSeg::Index(i));
                visit(item, prefix);
                prefix.pop();
            }
        }
        _ => {}
    }
}

/// Deep-clone `node` into an independent subtree: anchors stripped and inner
/// aliases expanded to copies of their targets, but styles and comments kept.
/// Used by `YAMLRocksNode.detach`. `refs` maps anchor names to their (real) nodes.
pub(crate) fn detached_clone(node: &YamlNode, refs: &HashMap<String, &YamlNode>) -> YamlNode {
    let mut budget = MAX_ALIAS_NODES;
    detached_clone_budgeted(node, refs, 0, &mut budget)
}

fn detached_clone_budgeted(
    node: &YamlNode,
    refs: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    crate::stack::guard(|| detached_clone_budgeted_inner(node, refs, depth, budget))
}

fn detached_clone_budgeted_inner(
    node: &YamlNode,
    refs: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    // Two independent guards: `depth` (alias hops only) fast-fails a pure cycle,
    // while `budget` (a total node count, mirroring the fast path's `MAX_NODES`)
    // bounds an alias bomb whose expansion multiplies across few hops. A deep but
    // acyclic subtree is bounded only by the generous node budget, so it is kept.
    if depth > MAX_ALIAS_DEPTH || *budget == 0 {
        return YamlNode::new(YamlNodeKind::Null, node.span);
    }
    *budget -= 1;
    let kind = match &node.kind {
        YamlNodeKind::Alias(name) => match refs.get(name) {
            Some(target) => return detached_clone_budgeted(target, refs, depth + 1, budget),
            None => YamlNodeKind::Null,
        },
        YamlNodeKind::Mapping(pairs) => YamlNodeKind::Mapping(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        detached_clone_budgeted(k, refs, depth, budget),
                        detached_clone_budgeted(v, refs, depth, budget),
                    )
                })
                .collect(),
        ),
        YamlNodeKind::Sequence(items) => YamlNodeKind::Sequence(
            items
                .iter()
                .map(|item| detached_clone_budgeted(item, refs, depth, budget))
                .collect(),
        ),
        other => other.clone(),
    };
    let mut out = YamlNode::new(kind, node.span);
    out.style = node.style;
    out.tag = node.tag.clone();
    out.comments = node.comments.clone();
    out
}

/// Maximum number of alias indirections to follow, fast-failing a cyclic anchor
/// (`&x` containing `*x`). Only alias hops count toward this; the total size of
/// the expansion is bounded separately by [`MAX_ALIAS_NODES`].
const MAX_ALIAS_DEPTH: usize = 100;

/// Total nodes all anchor expansions in one document may produce, the bound that
/// actually stops an alias bomb (an exponential blowup the per-hop depth cap
/// cannot catch). It is a single budget shared across every anchor, so a bomb
/// spread over many anchors cannot multiply past it. Set far above any real
/// round-trip document (config-sized, not bulk data) and below the fast path's
/// `MAX_NODES`, because a round-trip `YamlNode` is much heavier than a `Value`.
const MAX_ALIAS_NODES: usize = 1_000_000;

/// Collect every `&anchor` in `nodes` to an alias-free clone of its node, so
/// `*alias` access resolves to the value it refers to (instead of `None`).
/// Aliases inside the anchored nodes are expanded too, which also breaks any
/// cycle, so the returned values never contain aliases.
pub fn build_anchor_map(nodes: &[YamlNode]) -> HashMap<String, YamlNode> {
    let mut refs: HashMap<String, &YamlNode> = HashMap::new();
    for node in nodes {
        collect_anchor_refs(node, &mut refs);
    }
    // One budget shared across all anchors: a bomb cannot escape it by spreading
    // its expansion over many anchors.
    let mut budget = MAX_ALIAS_NODES;
    refs.iter()
        .map(|(name, node)| (name.clone(), expand_aliases(node, &refs, 0, &mut budget)))
        .collect()
}

pub(crate) fn collect_anchor_refs<'a>(
    node: &'a YamlNode,
    refs: &mut HashMap<String, &'a YamlNode>,
) {
    if let Some(name) = &node.anchor {
        refs.entry(name.clone()).or_insert(node);
    }
    match &node.kind {
        YamlNodeKind::Mapping(pairs) => {
            for (k, v) in pairs {
                collect_anchor_refs(k, refs);
                collect_anchor_refs(v, refs);
            }
        }
        YamlNodeKind::Sequence(items) => {
            for item in items {
                collect_anchor_refs(item, refs);
            }
        }
        _ => {}
    }
}

pub(crate) fn expand_aliases(
    node: &YamlNode,
    refs: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    crate::stack::guard(|| expand_aliases_inner(node, refs, depth, budget))
}

fn expand_aliases_inner(
    node: &YamlNode,
    refs: &HashMap<String, &YamlNode>,
    depth: usize,
    budget: &mut usize,
) -> YamlNode {
    // `depth` (alias hops) fast-fails a pure cycle; `budget` (a total node count)
    // bounds an alias bomb. See [`MAX_ALIAS_DEPTH`] and [`MAX_ALIAS_NODES`].
    if depth > MAX_ALIAS_DEPTH || *budget == 0 {
        return YamlNode::new(YamlNodeKind::Null, node.span);
    }
    *budget -= 1;
    match &node.kind {
        YamlNodeKind::Alias(name) => match refs.get(name) {
            Some(target) => expand_aliases(target, refs, depth + 1, budget),
            None => YamlNode::new(YamlNodeKind::Null, node.span),
        },
        YamlNodeKind::Mapping(pairs) => {
            let resolved = pairs
                .iter()
                .map(|(k, v)| {
                    (
                        expand_aliases(k, refs, depth, budget),
                        expand_aliases(v, refs, depth, budget),
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
                .map(|item| expand_aliases(item, refs, depth, budget))
                .collect();
            let mut out = YamlNode::new(YamlNodeKind::Sequence(resolved), node.span);
            out.tag = node.tag.clone();
            out
        }
        _ => node.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        alias_target_path, build_anchor_map, collect_alias_paths, collect_anchor_paths,
        collect_anchor_refs, detached_clone, expand_aliases, find_anchor_path, path_precedes,
        PathSeg, MAX_ALIAS_NODES,
    };
    use crate::roundtrip::ast::{YamlNode, YamlNodeKind};
    use crate::roundtrip::composer::compose;
    use crate::scanner::Span;
    use std::collections::HashMap;

    fn roots(src: &str) -> Vec<YamlNode> {
        compose(src).expect("valid YAML")
    }
    fn key(name: &str) -> PathSeg {
        PathSeg::Key(name.to_owned())
    }

    #[test]
    fn build_anchor_map_expands_anchored_values() {
        let map = build_anchor_map(&roots("base: &x\n  k: 1\nuse: *x\n"));
        assert!(map.contains_key("x"));
        assert!(matches!(map["x"].kind, YamlNodeKind::Mapping(_)));
    }

    #[test]
    fn find_anchor_path_locates_and_misses() {
        let r = roots("a: 1\nb: &x 2\n");
        assert!(find_anchor_path(&r, "x").is_some());
        assert!(find_anchor_path(&r, "absent").is_none());
    }

    #[test]
    fn collects_anchor_and_alias_paths() {
        let r = roots("base: &x 1\nuse: *x\nalso: *x\n");
        let mut anchors = Vec::new();
        collect_anchor_paths(&r[0], &mut Vec::new(), &mut anchors);
        assert!(anchors.iter().any(|(name, _)| name == "x"));

        let mut aliases = Vec::new();
        collect_alias_paths(&r[0], "x", &mut Vec::new(), &mut aliases);
        assert_eq!(aliases.len(), 2); // `use` and `also`
    }

    #[test]
    fn collects_an_anchor_on_a_mapping_key() {
        // An anchor on a key (`&k key: value`) is discovered and addressable,
        // mirroring `collect_anchor_refs`, which already sees key anchors.
        let r = roots("&kanchor key: value\nother: &vanchor 1\n");
        let mut anchors = Vec::new();
        collect_anchor_paths(&r[0], &mut Vec::new(), &mut anchors);
        let names: Vec<&str> = anchors.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"kanchor"));
        assert!(names.contains(&"vanchor"));
        // The key anchor's path resolves to the key node (the `key` scalar).
        assert!(find_anchor_path(&r, "kanchor").is_some());
    }

    #[test]
    fn alias_target_path_points_at_the_anchor() {
        let r = roots("base: &x 1\nuse: *x\n");
        assert!(alias_target_path(&r, &[key("use")]).is_some());
        // A non-alias path has no target.
        assert!(alias_target_path(&r, &[key("base")]).is_none());
    }

    #[test]
    fn path_precedes_follows_document_order() {
        let r = roots("first: 1\nsecond: 2\n");
        assert!(path_precedes(&r, &[key("first")], &[key("second")]));
        assert!(!path_precedes(&r, &[key("second")], &[key("first")]));
    }

    #[test]
    fn detached_clone_resolves_inner_aliases() {
        let r = roots("base: &x 1\nuse: *x\n");
        let mut refs = HashMap::new();
        for node in &r {
            collect_anchor_refs(node, &mut refs);
        }
        let YamlNodeKind::Mapping(pairs) = &r[0].kind else {
            panic!("mapping");
        };
        let use_val = &pairs
            .iter()
            .find(|(k, _)| matches!(&k.kind, YamlNodeKind::Scalar(s, _) if s == "use"))
            .unwrap()
            .1;
        assert!(matches!(use_val.kind, YamlNodeKind::Alias(_)));
        let cloned = detached_clone(use_val, &refs);
        assert!(matches!(cloned.kind, YamlNodeKind::Scalar(..))); // resolved to `1`
    }

    #[test]
    fn cyclic_anchor_terminates_via_depth_cap() {
        // A self-referential anchor (`&x` whose value contains `*x`) would recurse
        // forever without the alias-hop depth cap. This is cheap (the cap stops it
        // after MAX_ALIAS_DEPTH hops); the larger alias-bomb class is bounded by
        // the shared node budget in `build_anchor_map`, not exercised here because
        // doing so would allocate up to that budget.
        let map = build_anchor_map(&roots("a: &x\n  self: *x\n"));
        assert!(map.contains_key("x"));
    }

    #[test]
    fn expand_aliases_maps_a_missing_alias_to_null() {
        let alias = YamlNode::new(
            YamlNodeKind::Alias("missing".to_owned()),
            Span {
                file_id: 0,
                line: 0,
                column: 0,
                offset: 0,
            },
        );
        let refs: HashMap<String, &YamlNode> = HashMap::new();
        let mut budget = MAX_ALIAS_NODES;
        assert!(matches!(
            expand_aliases(&alias, &refs, 0, &mut budget).kind,
            YamlNodeKind::Null
        ));
    }
}
