# yamlrocks task runner. See every recipe with `just --list`.
#
# First time:   uv sync && source .venv/bin/activate   (then `just <recipe>`)
# Or without activating, prefix any recipe: `uv run --no-sync just <recipe>`.
# After Rust changes, rebuild the extension with `just develop`.
#
# Recipes use the uv-managed environment and assume `just setup` (or `just
# develop`) has run once. Versions live only in pyproject.toml / uv.lock.

# Show the list of recipes.
default:
    @just --list

# Install all development dependencies into the uv-managed environment.
setup:
    uv sync --no-install-project

# Build the extension in release mode (rerun after Rust changes; bootstraps deps).
develop:
    uv sync --no-install-project
    uv run --no-sync maturin develop --release

# Faster debug build for tight iteration loops.
develop-debug:
    uv sync --no-install-project
    uv run --no-sync maturin develop

# Run the Python test suite. Extra args pass through, e.g. `just test -k anchors`.
test *args:
    uv run --no-sync pytest {{args}}

# Run the Rust unit tests (the pure engine: scanner, resolver, decode, encode, ...).
test-rust *args:
    cargo test --lib {{args}}

# Lint and format-check Python (read-only; use `just fmt` to fix).
lint:
    uv run --no-sync ruff check .
    uv run --no-sync ruff format --check .

# Auto-format Python and Rust in place.
fmt:
    uv run --no-sync ruff check --fix .
    uv run --no-sync ruff format .
    cargo fmt

# Type-check with both mypy and ty.
typecheck:
    uv run --no-sync mypy pysrc/yamlrocks
    uv run --no-sync ty check pysrc/yamlrocks

# Lint the Rust crate.
clippy:
    cargo clippy --all-targets -- -D warnings

# Spell-check the codebase (codespell; config in pyproject.toml).
spellcheck:
    uv run --no-sync codespell

# Audit the GitHub Actions workflows for security issues (zizmor, pedantic).
zizmor:
    uv run --no-sync zizmor .github/workflows/ --persona=pedantic

# Run every pre-commit hook via prek (the exact set CI runs): ruff, mypy, ty,
# codespell, zizmor, cargo fmt/clippy, actionlint, prettier, taplo, file hygiene.
# Pass a hook id to run just one, e.g. `just precommit actionlint`.
precommit *args:
    uvx prek run {{args}} --all-files

# Audit every dependency ecosystem for advisories and supply-chain integrity.
audit: audit-rust audit-python audit-docs

# Scan Cargo.lock against the RustSec advisory database (needs cargo-audit).
audit-rust:
    cargo audit

# Audit the locked Python dependencies against the PyPI advisory database.
audit-python:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'rm -f requirements-audit.txt' EXIT
    uv export --frozen --no-emit-project --all-groups --format requirements-txt > requirements-audit.txt
    uvx pip-audit --requirement requirements-audit.txt

# Verify npm registry signatures and provenance for the docs dependencies.
audit-docs:
    cd docs && npm ci && npm audit signatures

# Regenerate THIRD_PARTY_LICENSES.md from the crate graph (needs cargo-about).
# Fails if a dependency uses a license not allow-listed in about.toml.
licenses:
    cargo about generate about.hbs -o THIRD_PARTY_LICENSES.md

# Rust line coverage measured through the Python suite (needs cargo-llvm-cov).
coverage:
    #!/usr/bin/env bash
    set -euo pipefail
    uv sync --no-install-project
    source <(cargo llvm-cov show-env --sh)
    cargo llvm-cov clean --workspace
    cargo test --lib
    uv run --no-sync maturin develop
    uv run --no-sync python -m pytest
    cargo llvm-cov report --summary-only

# Comparison benchmark report vs PyYAML, ruamel.yaml, and yamlium.
bench *args:
    uv sync --inexact --no-install-project --group bench
    uv run --no-sync python bench/bench.py {{args}}

# Throughput over the real-world config corpus (tests/data/realworld), the honest
# signal for parser speed and profiling on the shapes real YAML has. Auto-skips
# when the corpus submodule is absent.
bench-corpus *args:
    uv sync --inexact --no-install-project --group bench
    uv run --no-sync python bench/corpus_bench.py {{args}}

# Builds a release extension first (a debug build is several times slower, so it
# would understate YAMLRocks against the other libraries' release wheels), then
# renders the cross-library comparison charts into the docs assets.
charts:
    uv sync --inexact --no-install-project --group charts
    uv run --no-sync maturin develop --release
    cd bench && uv run --no-sync python charts.py

# Run the CodSpeed benchmarks locally (walltime mode).
codspeed:
    uv sync --inexact --no-install-project --group codspeed
    uv run --no-sync pytest bench --codspeed

# Fuzz a target; optional seconds time-box and target, e.g. `just fuzz 60` or
# `just fuzz 60 differential` (targets: parse, decode, roundtrip, differential).
fuzz seconds="" target="parse":
    #!/usr/bin/env bash
    set -euo pipefail
    # libFuzzer's own -rss_limit_mb is the memory guard here; `ulimit -v` cannot
    # be used because AddressSanitizer reserves its shadow memory virtually.
    if [ -n "{{seconds}}" ]; then
        cargo +nightly fuzz run {{target}} -- -max_total_time={{seconds}} -rss_limit_mb=4096
    else
        cargo +nightly fuzz run {{target}} -- -rss_limit_mb=4096
    fi

# Run every documented Python example and verify its output comments.
examples:
    uv run --no-sync python docs/verify_examples.py

# Build the documentation site.
docs:
    cd docs && npm ci && npm run build

# Serve the docs with live reload (astro dev, http://localhost:4321).
docs-dev:
    cd docs && npm ci && npm run dev

# Run the full local gate, mirroring CI: build, every pre-commit hook (lint,
# format, types, spelling, workflows, actionlint, prettier, taplo, file hygiene),
# then the Rust and Python test suites and the documented examples.
check: develop precommit
    cargo test --lib
    uv run --no-sync pytest -q
    uv run --no-sync python docs/verify_examples.py

# Remove build and fuzzing artifacts.
clean:
    cargo clean
    rm -rf fuzz/target fuzz/corpus fuzz/artifacts
