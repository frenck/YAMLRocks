"""Memory safety and liveness at the PyO3 boundary.

Each test pins down one way Python code (a tag callback, a key's ``__hash__``,
another thread) could reach into the extension while it is mid-way through
building an object, or hold a lock it needs. The scenarios that crash or hang
the interpreter on a regression run in a subprocess, so a failure is reported
cleanly instead of taking the whole pytest run down with it.
"""

from __future__ import annotations

import gc
import importlib.util
import subprocess
import sys
import textwrap
import weakref

import pytest

import yamlrocks


def _run_script(script: str, *, timeout: float = 60) -> subprocess.CompletedProcess:
    """Run ``script`` in a fresh interpreter and return the finished process."""
    return subprocess.run(
        [sys.executable, "-c", textwrap.dedent(script)],
        capture_output=True,
        timeout=timeout,
        check=False,
    )


def _assert_ok(proc: subprocess.CompletedProcess) -> None:
    """Assert a subprocess exited cleanly and printed ``OK`` as its last line."""
    assert proc.returncode == 0, (
        f"rc={proc.returncode} stderr={proc.stderr.decode(errors='replace')[-800:]}"
    )
    assert proc.stdout.strip().splitlines()[-1] == b"OK"


# A tag callback runs while the enclosing sequence is being converted. It hunts
# for that sequence's list through the garbage collector, the way any Python
# code can, and touches every slot. The list must not exist yet: a half-built
# list with NULL slots segfaults on read, and clearing it made the extension
# write through a stale index.
_PARTIAL_LIST_SCRIPT = """
    import gc, yamlrocks

    N = 37
    source = "[" + ", ".join(["a"] * (N - 1) + ["!t x"]) + "]"
    seen = []

    def handler(tag, value):
        for obj in gc.get_objects():
            if type(obj) is list and len(obj) == N and obj[:1] == ["a"]:
                seen.append([repr(item) for item in obj])
                obj.clear()
        return value

    result = yamlrocks.loads(source, tag_handler=handler)
    assert result == ["a"] * (N - 1) + ["x"], result
    assert seen == [], "the half-built list was reachable from a callback"
    print("OK")
"""


def test_tag_callback_cannot_reach_a_half_built_list():
    """A tag callback cannot find (and so cannot read or clear) a list still being filled."""
    _assert_ok(_run_script(_PARTIAL_LIST_SCRIPT))


# On a free-threaded build no callback is needed: another thread walking the
# garbage collector could find a half-built list on its own. On a regular build
# this is a plain stress test of the same conversion.
_CONCURRENT_GC_SCRIPT = """
    import gc, threading, yamlrocks

    source = "[" + ", ".join("[" + ", ".join(["v"] * 50) + "]" for _ in range(200)) + "]"
    stop = threading.Event()

    def walk_gc():
        while not stop.is_set():
            for obj in gc.get_objects():
                if type(obj) is list and obj[:1] == ["v"]:
                    for item in obj:
                        repr(item)

    walker = threading.Thread(target=walk_gc)
    walker.start()
    try:
        for _ in range(200):
            assert yamlrocks.loads(source) == [["v"] * 50] * 200
    finally:
        stop.set()
        walker.join()
    print("OK")
"""


def test_lists_are_never_half_built_while_another_thread_walks_the_gc():
    """Loading lists while another thread walks gc.get_objects() never crashes."""
    _assert_ok(_run_script(_CONCURRENT_GC_SCRIPT, timeout=120))


def test_failing_child_in_a_sequence_releases_converted_items():
    """A child conversion that fails midway leaks no reference to the items before it."""
    sentinel = object()
    baseline = sys.getrefcount(sentinel)
    source = "[" + ", ".join(["!ok a"] * 50 + ["!boom b"]) + "]"

    def handler(tag, value):
        if tag == "!boom":
            raise ValueError("boom")
        return sentinel

    for _ in range(200):
        with pytest.raises(ValueError, match="boom"):
            yamlrocks.loads(source, tag_handler=handler)

    assert sys.getrefcount(sentinel) == baseline


def test_nested_sequences_with_tag_callbacks_keep_their_order():
    """Nested sequences converted through tag callbacks keep every item in place."""
    source = "[!t 1, [!t 2, [!t 3, 4], 5], !t 6, [], [[!t 7]]]"
    result = yamlrocks.loads(source, tag_handler=lambda tag, value: int(value) * 10)
    assert result == [10, [20, [30, 4], 5], 60, [], [[70]]]


# -- Mapping keys --------------------------------------------------------------


class _HashRaises:
    """A key whose ``__hash__`` raises the exception it was built with."""

    def __init__(self, error: BaseException) -> None:
        self.error = error

    def __hash__(self) -> int:
        raise self.error


class _CustomError(Exception):
    """An exception type yamlrocks knows nothing about."""


def test_keyboard_interrupt_from_a_key_hash_propagates():
    """A KeyboardInterrupt raised in a key's __hash__ is not masked as a decode error."""
    key = _HashRaises(KeyboardInterrupt("stop"))
    with pytest.raises(KeyboardInterrupt, match="stop"):
        yamlrocks.loads("{!k a: 1}", tag_handler=lambda tag, value: key)


def test_non_type_error_from_a_key_hash_propagates():
    """Any non-TypeError raised in a key's __hash__ reaches the caller unchanged."""
    key = _HashRaises(_CustomError("custom"))
    with pytest.raises(_CustomError, match="custom"):
        yamlrocks.loads("{!k a: 1}", tag_handler=lambda tag, value: key)


def test_unhashable_key_is_still_a_decode_error():
    """An unhashable key from a tag callback still raises the clean decode error."""
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not hashable") as info:
        yamlrocks.loads("{!k a: 1}", tag_handler=lambda tag, value: [value])
    assert isinstance(info.value.__cause__, TypeError)


def test_type_error_from_a_key_hash_is_a_decode_error():
    """A TypeError raised in a key's __hash__ is translated into the decode error."""
    key = _HashRaises(TypeError("no hashing"))
    with pytest.raises(yamlrocks.YAMLRocksDecodeError, match="not hashable"):
        yamlrocks.loads("{!k a: 1}", tag_handler=lambda tag, value: key)


# -- Reference cycles ----------------------------------------------------------


class _Probe:
    """A weak-referenceable object to watch for collection."""


def test_tag_reference_cycle_is_collected():
    """A reference cycle through a YAMLRocksTag value is freed by the garbage collector."""
    probe = _Probe()
    watcher = weakref.ref(probe)
    holder: list[object] = [probe]
    holder.append(yamlrocks.YAMLRocksTag("!a", holder))
    del probe, holder

    gc.collect()

    assert watcher() is None


def test_tag_reports_its_value_to_the_garbage_collector():
    """gc.get_referents() of a YAMLRocksTag includes the value it holds."""
    value = ["held"]
    tag = yamlrocks.YAMLRocksTag("!a", value)
    assert any(referent is value for referent in gc.get_referents(tag))


# -- First use of numpy --------------------------------------------------------


# Two threads make their very first numpy-aware call at the same time, with
# numpy installed but not yet imported. Resolving the numpy types imports numpy,
# which runs Python and hands the GIL to the other thread; that thread must not
# then block on the cache while holding the GIL.
_NUMPY_FIRST_USE_SCRIPT = """
    import sys, threading, yamlrocks

    assert "numpy" not in sys.modules

    class Opaque:
        pass

    barrier = threading.Barrier(2)
    errors = []

    def work():
        try:
            barrier.wait()
            yamlrocks.dumps(
                [Opaque()], default=lambda obj: "x", option=yamlrocks.OPT_SERIALIZE_NUMPY
            )
        except BaseException as error:
            errors.append(error)

    threads = [threading.Thread(target=work) for _ in range(2)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert not errors, errors
    print("OK")
"""


@pytest.mark.skipif(
    importlib.util.find_spec("numpy") is None, reason="numpy is not installed"
)
def test_concurrent_first_numpy_use_does_not_deadlock():
    """Two threads resolving the numpy types for the first time at once both finish."""
    for _ in range(3):
        try:
            proc = _run_script(_NUMPY_FIRST_USE_SCRIPT, timeout=30)
        except subprocess.TimeoutExpired:
            pytest.fail("concurrent first numpy use deadlocked")
        _assert_ok(proc)


# -- Shared bytearray input ----------------------------------------------------


# On a free-threaded build the bytearray copy must hold the object's critical
# section, or a concurrent resize can free the buffer mid-copy. On a regular
# build the GIL already serializes this, so here it is a plain stress test.
_SHARED_BYTEARRAY_SCRIPT = """
    import threading, yamlrocks

    data = bytearray(b"key: value\\n" * 200)
    stop = threading.Event()

    def mutate():
        while not stop.is_set():
            data.extend(b"more: data\\n" * 50)
            del data[2200:]

    mutator = threading.Thread(target=mutate)
    mutator.start()
    try:
        for _ in range(2000):
            try:
                yamlrocks.loads(data)
            except yamlrocks.YAMLRocksDecodeError:
                pass  # a torn snapshot may be invalid YAML; it must not crash
    finally:
        stop.set()
        mutator.join()
    print("OK")
"""


def test_loading_a_bytearray_another_thread_resizes_never_crashes():
    """Loading a bytearray while another thread resizes it never crashes."""
    _assert_ok(_run_script(_SHARED_BYTEARRAY_SCRIPT, timeout=120))


def test_bytearray_and_memoryview_input_decode():
    """bytearray and memoryview input decode the same as bytes, including UTF-16."""
    text = "name: café\nitems: [1, 2]\n"
    expected = yamlrocks.loads(text)

    assert yamlrocks.loads(bytearray(text.encode())) == expected
    assert yamlrocks.loads(memoryview(text.encode())) == expected
    assert yamlrocks.loads(bytearray(text.encode("utf-16"))) == expected
