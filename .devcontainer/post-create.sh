#!/usr/bin/env bash
# Set up the yamlrocks development environment inside the dev container.
set -euo pipefail

echo "Installing development dependencies..."
# uv is provided by the devcontainer `uv` feature (see devcontainer.json).
# uv.lock is fully hash-pinned, so this is a reproducible, pinned install of the
# whole dev toolchain (maturin, pytest, ruff, mypy, codespell, just, ...). This
# mirrors `just setup`; pyproject.toml/uv.lock is the single source of truth, so
# nothing is re-listed here.
uv sync --no-install-project

echo "Adding Rust components..."
rustup component add rustfmt clippy

echo "Building yamlrocks (debug)..."
uv run --no-sync maturin develop

echo
echo "Dev container ready. Activate with: source .venv/bin/activate"
echo "Then use the task runner, e.g.:  just test"
