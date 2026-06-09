"""Memory-leak detection.

Two complementary strategies:

* **Object-count stability** (all platforms) - repeatedly run an operation and
  assert the number of live Python objects does not grow, which catches
  reference-count leaks in the objects yamlrocks hands back.
* **RSS growth** (Linux) - run an operation thousands of times and assert the
  resident set size does not grow meaningfully, which catches native (Rust)
  allocations that are never freed.

For deeper native analysis, build the extension and run the suite under
Valgrind or with the LeakSanitizer (e.g. ``RUSTFLAGS=-Zsanitizer=address``),
which is beyond the scope of these in-process checks.
"""

from __future__ import annotations

import gc
import os
import sys
import tempfile

import pytest

import yamlrocks

PAYLOAD = b"name: application\nversion: 1.2.3\nservers:\n" + b"".join(
    b"  - host: host-%d\n    port: %d\n    tags:\n      - a\n      - b\n"
    % (i, 8000 + i)
    for i in range(20)
)


def _rss_kb() -> int:
    """Current resident set size in KiB (Linux only)."""
    with open("/proc/self/statm") as handle:
        pages = int(handle.read().split()[1])
    return pages * (os.sysconf("SC_PAGE_SIZE") // 1024)


def _assert_no_object_growth(operation, *, warmup=200, iterations=2000):
    for _ in range(warmup):
        operation()
    gc.collect()
    before = len(gc.get_objects())
    for _ in range(iterations):
        operation()
    gc.collect()
    after = len(gc.get_objects())
    # Allow a tiny amount of slack for interpreter-internal caches.
    assert after - before < 100, f"object count grew by {after - before}"


def _assert_no_rss_growth(operation, *, warmup=500, iterations=10000, limit_kb=20000):
    if not sys.platform.startswith("linux"):
        pytest.skip("RSS check is Linux-only")
    for _ in range(warmup):
        operation()
    gc.collect()
    before = _rss_kb()
    for _ in range(iterations):
        operation()
    gc.collect()
    after = _rss_kb()
    assert after - before < limit_kb, f"RSS grew by {after - before} KiB"


# -- Object-count stability --------------------------------------------------


def test_loads_no_object_leak():
    """Repeated loads does not grow the live Python object count."""
    _assert_no_object_growth(lambda: yamlrocks.loads(PAYLOAD))


def test_dumps_no_object_leak():
    """Repeated dumps does not grow the live Python object count."""
    obj = yamlrocks.loads(PAYLOAD)
    _assert_no_object_growth(lambda: yamlrocks.dumps(obj))


def test_roundtrip_no_object_leak():
    """Repeated round-trip load and emit does not grow the object count."""

    def op():
        doc = yamlrocks.loads(PAYLOAD, option=yamlrocks.OPT_ROUND_TRIP)
        doc.to_yaml()

    _assert_no_object_growth(op)


def test_roundtrip_edit_no_object_leak():
    """Repeated round-trip edit and emit does not grow the object count."""

    def op():
        doc = yamlrocks.loads(b"a: 1\nb: 2\n", option=yamlrocks.OPT_ROUND_TRIP)
        doc["a"] = 99
        doc.to_yaml()

    _assert_no_object_growth(op)


def test_error_path_no_object_leak():
    """Repeatedly hitting the decode-error path does not grow the object count."""

    def op():
        try:
            yamlrocks.loads(b'x: "unterminated')
        except yamlrocks.YAMLRocksDecodeError:
            pass

    _assert_no_object_growth(op)


# -- RSS growth (Linux) ------------------------------------------------------


def test_loads_no_rss_leak():
    """Repeated loads does not grow the resident set size."""
    _assert_no_rss_growth(lambda: yamlrocks.loads(PAYLOAD))


def test_dumps_no_rss_leak():
    """Repeated dumps does not grow the resident set size."""
    obj = yamlrocks.loads(PAYLOAD)
    _assert_no_rss_growth(lambda: yamlrocks.dumps(obj))


def test_roundtrip_no_rss_leak():
    """Repeated round-trip load and emit does not grow the resident set size."""
    _assert_no_rss_growth(
        lambda: yamlrocks.loads(PAYLOAD, option=yamlrocks.OPT_ROUND_TRIP).to_yaml()
    )


def test_includes_no_rss_leak():
    """Repeated loads with !include directives does not grow the resident set size."""
    if not sys.platform.startswith("linux"):
        pytest.skip("RSS check is Linux-only")
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "sub.yaml"), "w") as f:
            f.write("nested:\n  value: 1\n")
        root = b"top: !include sub.yaml\n"
        _assert_no_rss_growth(
            lambda: yamlrocks.loads(
                root, option=yamlrocks.OPT_INCLUDES, include_dir=tmp
            ),
            iterations=5000,
        )
