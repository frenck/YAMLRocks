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

# Install the just-built wheel, offline (never PyPI), no deps (yamlrocks has
# none of its own).
uv pip install --python "${pyexe}" --no-index --no-deps --find-links dist yamlrocks

# Run against the installed wheel via the same interpreter.
python -m pytest -q
