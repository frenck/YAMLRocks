#!/usr/bin/env bash
# Set up the yamlrocks development environment inside the dev container.
set -euo pipefail

echo "Creating virtual environment and installing tooling..."
python3 -m venv .venv
# shellcheck disable=SC1091
source .venv/bin/activate

pip install --upgrade pip
pip install maturin pytest pyyaml ruff mypy codespell

echo "Adding Rust components..."
rustup component add rustfmt clippy

echo "Building yamlrocks (debug)..."
maturin develop

echo
echo "Dev container ready. Activate with: source .venv/bin/activate"
echo "Run the tests with:  pytest -q"
