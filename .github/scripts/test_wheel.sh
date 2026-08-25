#!/usr/bin/env bash
# Install the wheel that was just built into dist/, plus the pinned test
# dependencies, into the interpreter that built it, then run the full pytest
# suite against that wheel. Used by the native legs of build-wheels.yaml so the
# shipped artifact itself is what gets tested. Run from the repository root.
set -euo pipefail

# The interpreter set up for this job is the one the wheel was built for, so its
# ABI matches. Use it for both installing and running, rather than a uv-managed
# venv, because uv cannot always provide the right architecture (e.g. 32-bit
# Windows) while setup-python can.
pyexe="$(python -c 'import sys; print(sys.executable)')"

# Pinned test dependencies come from pyproject's [dependency-groups] so the
# versions live in exactly one place. numpy is best-effort: it has no wheel on
# every target (e.g. 32-bit Windows) and its tests skip cleanly when absent.
uv pip install --python "${pyexe}" --group test
uv pip install --python "${pyexe}" --group numpy \
  || echo "::notice::numpy has no wheel on this target; its tests will skip"
# Hypothesis ships ABI-specific wheels and has none for some targets here
# (prerelease CPython, 32-bit/ARM Windows); best-effort, and the property tests
# skip when it is absent, exactly like numpy above.
uv pip install --python "${pyexe}" --group proptest \
  || echo "::notice::hypothesis has no wheel on this target; property tests will skip"

# Install the just-built wheel, offline (never PyPI), no deps (yamlrocks has
# none of its own). `--refresh-package` is what makes this test the *built*
# wheel: every build produces the same name and version, so a wheel cached by an
# earlier run (uv's cache is restored across jobs) satisfies the requirement and
# is installed instead, silently smoke-testing yesterday's binary.
uv pip install --python "${pyexe}" --no-index --no-deps --refresh-package yamlrocks \
  --find-links dist yamlrocks

# On a free-threaded (no-GIL) build, fail loudly if the GIL is actually enabled,
# i.e. the build silently fell back to a GIL build or the import re-enabled it.
# On a regular build Py_GIL_DISABLED is unset and this is a no-op.
"${pyexe}" -c "import sys, sysconfig
if sysconfig.get_config_var('Py_GIL_DISABLED'):
    assert not sys._is_gil_enabled(), 'GIL is enabled on a free-threaded build'
    print('free-threaded build: GIL is disabled')"

# Run against the installed wheel via the exact interpreter the wheel was built
# for and installed into, not whatever "python" happens to resolve to on PATH.
"${pyexe}" -m pytest -q
