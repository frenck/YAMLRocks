"""Shared pytest configuration and fixtures for the YAMLRocks test suite.

Two things live here, available to every test in every subpackage:

* a hard address-space limit, installed at import time, so a pathological
  parser bug (an unbounded allocation loop) can never exhaust system memory and
  take down the host. This complements the shell-level ``ulimit``/``timeout``
  guards used in development.
* the ``assert_snapshot`` fixture: a tiny golden-file helper backed by
  ``tests/data/snapshots/``.
"""

from __future__ import annotations

import os
import pathlib
import sys
from collections.abc import Callable

import pytest

try:
    import resource
except ImportError:  # Windows has no `resource` module; the memory guard is skipped.
    resource = None  # type: ignore[assignment]

# -- Memory guard ------------------------------------------------------------

# Cap the process address space. Parsing any reasonable document stays far below
# this; a runaway allocation hits MemoryError instead of swap. Free-threaded
# CPython reserves substantially more address space (per-thread arenas, deferred
# refcounting), so the cap is configurable via ``YAMLROCKS_AS_LIMIT_MIB`` and
# defaults higher when the GIL is disabled.
# ``_is_gil_enabled`` exists only on free-threaded builds; assume GIL elsewhere.
_gil_enabled = getattr(sys, "_is_gil_enabled", lambda: True)()
_default_limit_mib = 1536 if _gil_enabled else 3072
_MEMORY_LIMIT_BYTES = (
    int(os.environ.get("YAMLROCKS_AS_LIMIT_MIB", _default_limit_mib)) * 1024 * 1024
)

if resource is not None:
    try:
        _soft, _hard = resource.getrlimit(resource.RLIMIT_AS)
        _target = (
            _MEMORY_LIMIT_BYTES
            if _hard == resource.RLIM_INFINITY
            else min(_MEMORY_LIMIT_BYTES, _hard)
        )
        resource.setrlimit(resource.RLIMIT_AS, (_target, _hard))
    except (ValueError, OSError):
        # Some platforms disallow lowering RLIMIT_AS; tests still run, just unguarded.
        pass


# -- Shared paths ------------------------------------------------------------

#: Root of the vendored test data (corpus, snapshots, the YAML test suite).
DATA_DIR = pathlib.Path(__file__).parent / "data"


# -- Snapshot helper ---------------------------------------------------------


@pytest.fixture
def assert_snapshot() -> Callable[[str, str], None]:
    """Return a golden-file assertion helper.

    Snapshots live under ``tests/data/snapshots/`` as plain text. On the first
    run (or when ``UPDATE_SNAPSHOTS=1`` is set) the snapshot is written;
    afterwards the produced value is compared against the stored one. This
    deliberately avoids a third-party dependency so the suite stays
    self-contained and fast.
    """
    snapshot_dir = DATA_DIR / "snapshots"
    update = os.environ.get("UPDATE_SNAPSHOTS") == "1"

    def _assert(name: str, content: str) -> None:
        path = snapshot_dir / name
        if update or not path.exists():
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
            return
        expected = path.read_text(encoding="utf-8")
        assert content == expected, (
            f"snapshot mismatch for {name!r}\n"
            f"--- expected ---\n{expected}\n--- actual ---\n{content}\n"
            f"(run with UPDATE_SNAPSHOTS=1 to refresh if this change is intended)"
        )

    return _assert
