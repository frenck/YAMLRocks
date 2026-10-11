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
    remaining_stack().map_or(true, |remaining| remaining > RED_ZONE)
}

#[cfg(not(all(target_os = "linux", target_env = "musl")))]
#[inline]
fn remaining_stack() -> Option<usize> {
    stacker::remaining_stack()
}

/// The remaining native stack, correcting for musl's main thread.
///
/// musl's `pthread_getattr_np` reports the main thread's stack as only the pages
/// touched so far, not the `RLIMIT_STACK` the kernel lets it grow to, and
/// `stacker` caches that bound on first use. A later, deeper call then sees
/// almost no headroom and the `represent` lowering refuses even a shallow
/// document. A thread running on the kernel's grow-down `[stack]` mapping
/// therefore measures against the real rlimit bound; every other thread (and a
/// grown segment) keeps `stacker`'s answer, which musl reports correctly there.
///
/// Like `stacker`, the bound is computed once per thread, so a later change to
/// `RLIMIT_STACK` is not picked up.
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[inline]
fn remaining_stack() -> Option<usize> {
    thread_local! {
        static GROWABLE_STACK: Option<(usize, usize)> = musl_growable_stack();
    }

    let marker = 0u8;
    let stack_pointer = std::ptr::addr_of!(marker) as usize;
    let on_growable_stack = GROWABLE_STACK
        .with(|bounds| bounds.filter(|&(limit, top)| (limit..top).contains(&stack_pointer)));
    match on_growable_stack {
        Some((limit, _)) => Some(stack_pointer - limit),
        None => stacker::remaining_stack(),
    }
}

/// The `(limit, top)` range the calling thread's stack can grow to, when that
/// thread runs on the kernel's grow-down `[stack]` mapping; `None` otherwise, or
/// when anything cannot be established (the caller then keeps `stacker`'s
/// bound).
///
/// Being the thread-group leader is not enough: a process forked from a worker
/// thread has its leader on that worker's fixed-size pthread stack. So the stack
/// pointer must lie inside the `[stack]` mapping from `/proc/self/maps`. The
/// limit is that mapping's top minus `RLIMIT_STACK`, or minus Linux's 8 MB
/// default when the rlimit is unlimited.
#[cfg(all(target_os = "linux", target_env = "musl"))]
fn musl_growable_stack() -> Option<(usize, usize)> {
    const DEFAULT_MAIN_STACK: usize = 8 * 1024 * 1024;

    // Only the thread-group leader can own the `[stack]` mapping; checking that
    // first keeps every worker thread from reading `/proc`.
    // SAFETY: argument-less syscalls with no memory access.
    let is_leader = unsafe { libc::getpid() as libc::c_long == libc::syscall(libc::SYS_gettid) };
    if !is_leader {
        return None;
    }

    let marker = 0u8;
    let stack_pointer = std::ptr::addr_of!(marker) as usize;
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let (start, top) = maps
        .lines()
        .filter(|line| line.ends_with("[stack]"))
        .find_map(|line| {
            let (start, end) = line.split_whitespace().next()?.split_once('-')?;
            let start = usize::from_str_radix(start, 16).ok()?;
            let end = usize::from_str_radix(end, 16).ok()?;
            Some((start, end))
        })?;
    if !(start..top).contains(&stack_pointer) {
        return None;
    }

    // SAFETY: `getrlimit` writes only into the `rlimit` passed to it, and the
    // result is read only after it reports success.
    let rlimit = unsafe {
        let mut rlimit: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_STACK, &mut rlimit) != 0 {
            return None;
        }
        rlimit
    };
    let reserved = if rlimit.rlim_cur == libc::RLIM_INFINITY {
        DEFAULT_MAIN_STACK
    } else {
        rlimit.rlim_cur as usize
    };
    Some((top.saturating_sub(reserved), top))
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
