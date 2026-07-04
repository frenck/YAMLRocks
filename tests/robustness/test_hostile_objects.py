"""Property-based robustness tests for hostile Python objects at the dump boundary.

`dumps`, `to_json`, and the round-trip emitter walk arbitrary Python objects
across the PyO3 boundary. A malicious or buggy object, a lying container, a
dunder that mutates or raises, a reference cycle, or a recursive ``default``
hook, must never crash the interpreter or hang. The contract for every case is
the same: the call returns ``bytes``, or it raises a catchable ``Exception``.

The Rust fuzz targets (``fuzz/``) cannot reach this layer: a libFuzzer binary has
no Python interpreter, so it never exercises the object-to-node conversion or the
user callbacks. Hypothesis drives that surface here, generating nested structures
that mix well-behaved values with adversarial objects.
"""

from __future__ import annotations

from typing import Any

import pytest
from hypothesis import HealthCheck, given, settings
from hypothesis import strategies as st

import yamlrocks

# Hostile objects can be slow to reject and vary wildly in size, so the timing
# and size health checks would flake without telling us anything. The property
# is about crashing, not speed.
HOSTILE = settings(
    deadline=None,
    max_examples=300,
    suppress_health_check=[HealthCheck.too_slow, HealthCheck.data_too_large],
)


class Opaque:
    """A plain object no serializer knows how to represent."""


class RaisingHash:
    """``__hash__`` raises; unserializable as a value, explosive as a key."""

    def __hash__(self) -> int:
        raise ValueError("hostile __hash__")


class RaisingKeys(dict):
    """A mapping whose ``keys()`` raises mid-walk."""

    def keys(self):  # type: ignore[override]
        raise RuntimeError("hostile keys()")


class LyingKeys(dict):
    """A mapping whose ``keys()`` advertises keys that are not present."""

    def keys(self):  # type: ignore[override]
        return ["ghost", *dict.keys(self)]


class LyingLen(list):
    """A sequence whose ``__len__`` disagrees with its real length."""

    def __len__(self) -> int:
        return 9999


class RaisingLen(list):
    """A sequence whose ``__len__`` raises."""

    def __len__(self) -> int:
        raise RuntimeError("hostile __len__")


class RaisingIter:
    """An object that claims to be iterable but explodes on iteration."""

    def __iter__(self):
        raise RuntimeError("hostile __iter__")


# Each builder returns a *fresh* instance per example so no mutable state leaks
# between Hypothesis runs.
hostile_objects = st.one_of(
    st.builds(Opaque),
    st.builds(RaisingHash),
    st.builds(lambda: RaisingKeys({"real": 1})),
    st.builds(lambda: LyingKeys({"real": 1})),
    st.builds(lambda: LyingLen([1, 2])),
    st.builds(lambda: RaisingLen([1, 2])),
    st.builds(RaisingIter),
    st.builds(lambda: (n for n in range(3))),  # a bare generator
)

scalars = (
    st.none()
    | st.booleans()
    | st.integers()
    | st.floats(allow_nan=True, allow_infinity=True)
    | st.text(max_size=8)
)

# Nested structures whose leaves may be well-behaved scalars or hostile objects.
# Mapping keys stay as text (a hostile key is covered separately) so the
# generator itself does not raise while building the example.
structures = st.recursive(
    scalars | hostile_objects,
    lambda children: (
        st.lists(children, max_size=4)
        | st.dictionaries(st.text(max_size=5), children, max_size=4)
    ),
    max_leaves=25,
)


def _assert_bytes_or_raises(fn, value: Any) -> None:
    """The dump contract: return ``bytes``/``bytearray`` or raise ``Exception``."""
    try:
        result = fn(value)
    except Exception:
        return
    assert isinstance(result, (bytes, bytearray))


@HOSTILE
@given(value=structures)
def test_dumps_never_crashes_on_hostile_objects(value):
    """dumps returns bytes or raises on any structure of hostile objects."""
    _assert_bytes_or_raises(yamlrocks.dumps, value)


@HOSTILE
@given(value=structures)
def test_to_json_never_crashes_on_hostile_objects(value):
    """to_json returns bytes or raises on any structure of hostile objects."""
    _assert_bytes_or_raises(yamlrocks.to_json, value)


@HOSTILE
@given(value=structures)
def test_round_trip_emit_never_crashes_on_hostile_objects(value):
    """A round-trip document built from hostile objects emits or raises cleanly."""
    try:
        doc = yamlrocks.dumps(value)
    except Exception:
        return
    # Whatever dumped must reload and re-emit without crashing.
    reloaded = yamlrocks.loads(doc, option=yamlrocks.OPT_ROUND_TRIP)
    assert isinstance(reloaded.to_yaml(), (bytes, bytearray))


@settings(deadline=None, max_examples=60)
@given(depth=st.integers(min_value=0, max_value=6))
def test_reference_cycle_is_rejected_not_hung(depth):
    """A self-referential structure raises rather than looping forever."""
    root: dict[str, Any] = {}
    cursor = root
    for i in range(depth):
        child: dict[str, Any] = {}
        cursor[f"k{i}"] = child
        cursor = child
    cursor["loop"] = root  # close the cycle

    with pytest.raises(Exception):
        yamlrocks.dumps(root)
    with pytest.raises(Exception):
        yamlrocks.to_json(root)


def _default_returns_self(obj):
    return obj


def _default_returns_unserializable(obj):
    return Opaque()


def _default_raises(obj):
    raise ValueError("hostile default")


def _default_returns_cycle(obj):
    cycle: dict[str, Any] = {}
    cycle["self"] = cycle
    return cycle


@pytest.mark.parametrize(
    "hook",
    [
        _default_returns_self,
        _default_returns_unserializable,
        _default_raises,
        _default_returns_cycle,
    ],
)
def test_hostile_default_hook_raises_not_crashes(hook):
    """A default= hook that misbehaves surfaces an exception, never a crash."""
    with pytest.raises(Exception):
        yamlrocks.dumps(Opaque(), default=hook)


def test_default_hook_mutating_parent_during_walk_does_not_crash():
    """A default= callback that mutates the mapping being serialized is safe.

    Rehashing a dict while Rust iterates its items is the classic use-after-free
    shape (see the FFI hardening for `__bool__`/`__index__`); the callback here
    clears and repopulates the very dict under the walk.
    """
    parent: dict[str, Any] = {"a": Opaque(), "b": Opaque()}

    def hook(obj):
        parent["injected"] = "x"
        parent.clear()
        return "safe"

    try:
        yamlrocks.dumps(parent, default=hook)
    except Exception:
        pass


def test_hostile_object_as_mapping_key_raises():
    """A key whose __hash__ raises propagates that error, not a crash."""
    with pytest.raises(Exception):
        yamlrocks.dumps({RaisingHash(): 1})
