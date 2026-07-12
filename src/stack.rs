//! On-demand native stack growth for recursive descent.
//!
//! The decoder, composer, emitter, and their tree-to-Python conversions all
//! recurse once per level of nesting. The [`MAX_DEPTH`](crate::decode) cap bounds
//! how deep that goes, but the cap alone assumes a generous (~8 MB) stack: a
//! thread with a smaller stack (a thread-pool worker, or musl's ~128 KB default)
//! can overflow *before* the cap fires, and under the release profile's
//! `panic = "abort"` a stack overflow aborts the whole interpreter rather than
//! raising a catchable error.
//!
//! [`guard`] wraps a recursive step so that, whenever the remaining stack runs
//! low, a fresh segment is allocated and the recursion continues on it. The depth
//! cap still bounds total work; this only removes the dependency on the initial
//! stack size, so deeply nested untrusted input fails with a clean
//! "maximum nesting depth" error on every thread instead of crashing on some.

/// Stack headroom below which [`guard`] allocates a new segment. Kept small so
/// shallow work on a small-stack worker thread (a thread-pool or musl thread)
/// does not allocate a segment, yet comfortably larger than a single recursion
/// step so the frames between two checks never exhaust the remaining stack.
const RED_ZONE: usize = 100 * 1024;

/// Size of each freshly allocated stack segment. Sized to hold the whole of a
/// `MAX_DEPTH` (1000) recursion in one segment: guarding every level adds a few
/// frames per level, so the worst recursive paths (the comment-attachment walk,
/// the AST-to-Python conversion) use tens of kilobytes per level. A segment is
/// allocated only when a guard actually fires (deeply nested input on a thread
/// whose remaining stack has fallen below [`RED_ZONE`]) and is freed when the
/// operation returns, so ordinary documents never pay for it.
const STACK_SIZE: usize = 32 * 1024 * 1024;

/// Run `f` on a stack guaranteed to have at least [`RED_ZONE`] bytes free,
/// growing onto a new [`STACK_SIZE`] segment first if it does not. Call at the
/// top of a recursive function so every level is covered. When ample stack
/// remains (the common, shallow case) this is a single cheap pointer comparison.
#[inline]
pub(crate) fn guard<R>(f: impl FnOnce() -> R) -> R {
    stacker::maybe_grow(RED_ZONE, STACK_SIZE, f)
}

/// Whether enough native stack remains to recurse one more level of a descent
/// that re-enters Python (the `represent` lowering, where every level may call
/// the callback, `default`, or a serializer).
///
/// Such a descent must *not* grow onto a new segment the way [`guard`] does:
/// CPython validates its C stack against the thread's original stack bounds,
/// and Python code running on a detached segment trips a fatal "Unrecoverable
/// stack overflow" abort instead of raising. The caller checks headroom per
/// level and raises a clean depth error when it runs low. An unknown remaining
/// size (no platform stack bounds) counts as headroom: that is the pre-guard
/// status quo, and every tier-one platform reports bounds.
#[inline]
pub(crate) fn python_call_headroom() -> bool {
    stacker::remaining_stack().map_or(true, |remaining| remaining > RED_ZONE)
}

use crate::decode::Value;
use crate::roundtrip::ast::{YamlNode, YamlNodeKind};

/// Drop a [`Value`] tree iteratively, never recursing.
///
/// The derived drop recurses once per level of nesting, so freeing a deeply
/// nested tree (bounded only by `MAX_DEPTH`) can overflow a small thread stack
/// and abort the interpreter under `panic = "abort"`. This moves every child
/// onto an explicit heap work-list instead: each popped node's children are
/// taken out before the node's shell drops, so the drop stays shallow regardless
/// of depth. Call it wherever an owned deep tree would otherwise drop on the
/// stack (the build paths grow the stack on demand via [`guard`], but a drop
/// happens after those return).
pub(crate) fn drop_value_tree(value: Value<'_>) {
    // Pre-sized so small trees drop with a single work-list allocation instead
    // of doubling through the first pushes.
    let mut work = Vec::with_capacity(32);
    work.push(value);
    while let Some(node) = work.pop() {
        match node {
            Value::Sequence(items) => work.extend(items),
            Value::Mapping(pairs) => {
                for (key, val) in pairs {
                    work.push(key);
                    work.push(val);
                }
            }
            Value::Tagged(_, inner) => work.push(*inner),
            // A leaf (scalar/null/bool/int/float): its shell drops shallowly here.
            _ => {}
        }
    }
}

/// Drop a [`YamlNode`] tree iteratively, never recursing. The round-trip
/// counterpart to [`drop_value_tree`]; see its documentation. Matching on
/// `node.kind` by value moves the children out, leaving the node's remaining
/// (non-recursive) fields to drop shallowly.
pub(crate) fn drop_node_tree(node: YamlNode) {
    let mut work = vec![node];
    while let Some(node) = work.pop() {
        match node.kind {
            YamlNodeKind::Sequence(items) => work.extend(items),
            YamlNodeKind::Mapping(pairs) => {
                for (key, val) in pairs {
                    work.push(key);
                    work.push(val);
                }
            }
            _ => {}
        }
    }
}
