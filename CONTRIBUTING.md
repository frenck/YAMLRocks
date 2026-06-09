# Contributing to YAMLRocks

Thanks for your interest in improving YAMLRocks! This guide covers the local
development workflow.

> [!IMPORTANT]
> Using AI tools to help with a contribution is fine, but you must review and
> understand everything you submit and be able to explain it in your own words.
> Read the [AI Policy](AI_POLICY.md) before opening a pull request or issue:
> autonomous agents are not allowed, and unreviewed AI output will be closed.

## Development setup

You need a [Rust toolchain](https://rustup.rs/) and [uv](https://docs.astral.sh/uv/).
uv manages the Python version, the virtual environment, and every dependency from
`pyproject.toml` (locked in `uv.lock`), so there is nothing to install by hand.

```bash
# Install all development dependencies into a managed virtual environment.
uv sync

# Build and install the extension in development mode (rerun after Rust changes).
uv run maturin develop
```

Prefix project commands with `uv run` so they use the managed environment, e.g.
`uv run pytest`. Dependency versions live only in `pyproject.toml`; CI installs
the same groups with `uv sync`, so there is no second place to keep in step.

### Task runner

Common tasks are wrapped in a [`just`](https://github.com/casey/just) `justfile`,
so you rarely need to remember the underlying commands. `just` ships with the dev
dependencies (the `rust-just` wheel), so no separate install is needed:

```bash
uv sync                      # installs deps, including just
source .venv/bin/activate    # so `just` is on PATH

just                         # list every task
just develop                 # build the extension (rerun after Rust changes)
just test                    # run the suite (just test -k anchors to filter)
just check                   # the full local gate: lint, types, clippy, tests, docs
```

Without activating the venv, prefix any recipe with `uv run --no-sync`, e.g.
`uv run --no-sync just test`. The recipes call the same commands documented below,
so the rest of this guide still applies if you prefer running them directly.

## Running the tests

The test suite always runs under a memory cap (see `tests/conftest.py`). During
development it is good practice to add a shell-level guard as well:

```bash
uv run pytest
# or, with explicit guards:
timeout 120 bash -c 'ulimit -v 3000000; uv run pytest'
```

The suite includes the [YAML test
suite](https://github.com/yaml/yaml-test-suite), snapshot tests, fuzz tests, and
memory-leak checks. The YAML test suite lives in a git submodule (under
`tests/data/yaml_test_suite/cases`), so the compliance category auto-skips unless
you check it out:

```bash
git submodule update --init tests/data/yaml_test_suite/cases
```

### Rust unit tests

The pure-Rust engine (scanner, parser, resolver, decode, encode, and the
include/schema helpers) carries direct unit tests alongside the Python suite,
which covers the PyO3 boundary on top. They need no Python and run fast:

```bash
cargo test --lib        # or: just test-rust
```

### Real-world configs

A dedicated `realworld` category parses large public configurations across
ecosystems (Home Assistant, ESPHome, Ansible, Kubernetes, Docker Compose) and
asserts every file parses and round-trips byte-for-byte. The configs are git
submodules, so they are opt-in:

```bash
# Fetch the config repos once.
git submodule update --init

# Run just this category.
uv run pytest tests/realworld -m realworld
```

Without the submodules checked out the category auto-skips, so a plain `pytest`
stays green. A parse or round-trip failure on valid YAML is a parser bug to fix:
see `tests/realworld/README.md` before adding a file to the xfail list.

### Coverage

The Rust core is exercised both by its own unit tests and, end-to-end, through
the Python test suite. Coverage is measured by instrumenting the build with
[`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) and running both
`cargo test` and `pytest` against the instrumented build:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov

# Apply the instrumentation environment to the current shell.
source <(cargo llvm-cov show-env --sh)
cargo llvm-cov clean --workspace

# Run the Rust unit tests, then build instrumented and run the Python suite.
cargo test --lib
uv run --no-sync maturin develop
uv run --no-sync python -m pytest

# Print a line-coverage summary.
cargo llvm-cov report --summary-only
```

Check out the real-world submodules first (`git submodule update --init`) for a
representative number; CI enforces a line-coverage floor of 90% via
`cargo llvm-cov report --fail-under-lines 90`, and uploads the Cobertura report
to Codecov (needs a `CODECOV_TOKEN` repository secret) and to GitHub code
quality.

## Benchmarks

There are two layers of performance tooling:

- `bench/bench.py` is a one-off report that compares YAMLRocks against PyYAML,
  ruamel.yaml, and yamlium. Run `uv run --group bench python bench/bench.py`, or
  add `--save` to refresh `bench/RESULTS.md`.
- `bench/test_benchmarks.py` measures YAMLRocks's _own_ operations through the
  [`pytest-codspeed`](https://github.com/CodSpeedHQ/pytest-codspeed) `benchmark`
  fixture. [CodSpeed](https://codspeed.io) runs these on every pull request under
  instrumentation and comments with any regression against the base branch.

```bash
uv run --group codspeed pytest bench --codspeed   # local walltime; CI instruments
```

These live under `bench/` (outside `testpaths`), so a normal `pytest` run does
not collect them. The CodSpeed CI job needs a `CODSPEED_TOKEN` repository secret.

## Fuzzing

`tests/robustness/test_fuzz.py` is a fast, always-on property check (random valid
structures and arbitrary bytes). For deeper, coverage-guided fuzzing of the Rust
parser, there are [`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) targets
under `fuzz/`:

| Target         | What it drives                                                      | Contract                    |
| -------------- | ------------------------------------------------------------------- | --------------------------- |
| `parse`        | scanner → parser → composer (the round-trip AST)                    | never panic/hang            |
| `decode`       | fast `loads` path → `Value` tree → fast `dumps`, under both schemas | never panic/hang            |
| `roundtrip`    | compose → emit → re-compose (the round-trip emitter)                | never panic/hang            |
| `differential` | `loads(dumps(loads(x)))` must equal `loads(x)`                      | never silently corrupt data |

```bash
cargo install cargo-fuzz             # once; needs a nightly toolchain
just fuzz 60                         # fuzz `parse` for 60s (the default target)
just fuzz 60 differential            # fuzz any target by name
cargo +nightly fuzz run parse        # or invoke cargo-fuzz directly, until a crash or Ctrl-C
```

The first three targets check that no input _crashes_ the parser. `differential`
checks something a crash-only target cannot: that `dumps` never emits YAML which
`loads` reads back as _different data_ (a mis-quoted string re-resolving to a
bool, a float losing precision). Such a bug produces wrong values, not a panic,
so only comparing the two trees surfaces it.

[ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) builds and runs
every target for a short batch on each pull request (see
`.github/workflows/clusterfuzzlite.yaml` and `.clusterfuzzlite/`). A `parse`,
`decode`, or `roundtrip` crash is a parser bug (the parser may reject malformed
input with an error, but must never panic or hang); a `differential` failure is a
correctness bug (a document that does not survive a `dumps`/`loads` round-trip).

## Code quality

All checks are wired into [prek](https://prek.j178.dev) /
[pre-commit](https://pre-commit.com):

```bash
prek run --all-files
```

Or run the linters directly:

```bash
uv run ruff check . && uv run ruff format --check .  # Python lint + format
uv run mypy pysrc/yamlrocks                          # Python typing
uv run ty check pysrc/yamlrocks                      # Python typing (ty)
uv run codespell                                     # Spell check
uv run zizmor .github/workflows/ --persona=pedantic  # Actions security
cargo fmt --check                                    # Rust format
cargo clippy --all-targets -- -D warnings            # Rust lint
```

CI enforces all of the above; please make sure they pass before opening a pull
request.

### Dependency audits

Known-vulnerability and supply-chain checks run on a weekly schedule and whenever
a lockfile changes (see `.github/workflows/security-audit.yaml`). Run them locally
with:

```bash
just audit            # all three ecosystems at once
just audit-rust       # cargo audit (RustSec advisory database; needs cargo-audit)
just audit-python     # pip-audit over the locked dependencies (PyPI advisories)
just audit-docs       # npm audit signatures (registry signatures + provenance)
```

Integrity is already pinned by the lockfiles (`Cargo.lock`, `uv.lock`,
`docs/package-lock.json`) and the SHA-pinned actions; these audits add advisory
scanning on top.

## Conventions

- Match the style of the surrounding code; the codebase favors small, well-named
  functions and explicit error handling.
- Add tests for new behavior. Round-trip changes should preserve byte-for-byte
  fidelity for unmodified documents.
- Update the documentation under `docs/` when adding or changing user-facing
  features.
- There is no changelog to edit: release notes are drafted automatically by
  [Release Drafter](https://github.com/release-drafter/release-drafter) from
  merged pull requests, grouped by label. Give your pull request a clear,
  descriptive title (it becomes the release-note line) and the right label.

## Architecture

See the [architecture decision records](adr/) for the major design choices and
[`docs/`](docs/) for user-facing documentation.
