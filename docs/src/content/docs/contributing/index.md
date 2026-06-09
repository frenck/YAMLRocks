---
title: Contributing
description: How to set up a development environment and contribute to YAMLRocks.
---

YAMLRocks is an open-source project, and contributions are welcome, whether
that is a bug report, a documentation fix, or a pull request. This page covers
the local development workflow. The full guidelines also live in
[`CONTRIBUTING.md`](https://github.com/frenck/yamlrocks/blob/main/CONTRIBUTING.md)
in the repository.

:::caution[Using AI tools]
AI tools are welcome as an aid, but you must review and understand everything you
submit and be able to explain it in your own words. Autonomous agents are not
allowed, and unreviewed AI output (in pull requests, issues, or review threads)
will be closed. Read the
[AI Policy](https://github.com/frenck/yamlrocks/blob/main/AI_POLICY.md) before
contributing.
:::

## Development setup

The fastest way to start is the [dev container](https://containers.dev/): open
the repository in a GitHub Codespace or in VS Code with the Dev Containers
extension, and the toolchain is set up for you.

To set things up by hand you need a [Rust toolchain](https://rustup.rs/) and
[uv](https://docs.astral.sh/uv/). uv manages the Python version (3.11 or newer),
the virtual environment, and every dependency from `pyproject.toml`:

```bash
# Install all development dependencies into a managed virtual environment.
uv sync

# Activate it so `just` (shipped as a dev dependency) is on PATH.
source .venv/bin/activate

# Build and install the extension (release mode; rerun after Rust changes).
just develop
```

`just develop` compiles the Rust crate and installs `yamlrocks` into the managed
environment as an editable build. Re-run it after changing any Rust code. For a
faster, less-optimized build during tight iteration loops, use `just develop-debug`.

### Task runner

`just` is the primary interface for everyday work: every common task (build, test,
lint, type-check, docs) is a recipe that wraps the exact command CI runs, so you
rarely need to remember the underlying invocation. It ships with the dev
dependencies, so there is nothing extra to install.

```bash
just                # list every recipe
just develop        # build the extension (rerun after Rust changes)
just test           # run the Python suite (just test -k anchors to filter)
just check          # the full local gate: build, lint, types, clippy, tests, docs
```

If you would rather not activate the venv, prefix any recipe with
`uv run --no-sync`, for example `uv run --no-sync just test`. Every recipe maps to
the raw commands shown throughout this page, so either style works.

## Running the tests

The suite is grouped by capability under `tests/` and always runs under a memory
cap (configured in `tests/conftest.py`). `just test` runs it; during development
it is good practice to add a shell-level guard as well, so a parser bug can never
exhaust the host:

```bash
just test                    # or: uv run --no-sync pytest
just test -k anchors         # extra args pass straight through to pytest
# with an explicit shell guard:
timeout 120 bash -c 'ulimit -v 3000000; uv run --no-sync pytest'
```

It includes the
[YAML test suite](https://github.com/yaml/yaml-test-suite) (a git submodule),
golden-file snapshot
tests, fuzz tests, security tests, and memory/refcount checks. See
[`tests/README.md`](https://github.com/frenck/yamlrocks/blob/main/tests/README.md)
for the layout.

### Rust unit tests

The pure-Rust engine (scanner, parser, resolver, decode, encode, and the
include/schema helpers) carries direct unit tests alongside the Python suite,
which covers the PyO3 boundary on top. They need no Python and run fast:

```bash
just test-rust               # or: cargo test --lib
```

### Real-world configs

A dedicated `realworld` category parses large public configurations across
ecosystems (Home Assistant, ESPHome, Ansible, Kubernetes, Docker Compose) and
asserts every file parses and round-trips byte-for-byte. The configs are git
submodules, so they are opt-in and the category auto-skips when they are absent:

```bash
git submodule update --init                 # fetch the config repos once
uv run --no-sync pytest tests/realworld -m realworld  # run just this category
```

## Coverage

The Rust core is exercised both by its own Rust unit tests and, end-to-end,
through the Python suite. Coverage is measured by instrumenting the build with
[`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) and running both
`cargo test` and `pytest` against it. The whole flow is wrapped in one recipe:

```bash
rustup component add llvm-tools-preview   # one-time: the llvm coverage tools
cargo install cargo-llvm-cov              # one-time

just coverage                             # instrument, run both suites, print a summary
```

`just coverage` runs the instrumented `cargo test --lib` and `pytest` under a
fresh profile and prints a line-coverage summary. Check out the real-world
submodules first (`git submodule update --init`) for a representative number; CI
runs the same flow and fails when line coverage drops below 90%
(`cargo llvm-cov report --fail-under-lines 90`).

## Benchmarks

There are two layers of performance tooling. `just bench` runs a one-off report
comparing YAMLRocks against PyYAML, ruamel.yaml, and yamlium, while `just codspeed`
runs YAMLRocks's own operations through [CodSpeed](https://codspeed.io), which also
runs on every pull request and comments on any regression. Build in release mode
(`just develop`) before measuring; debug builds are far slower.

```bash
just bench           # comparison report vs other libraries
just codspeed        # YAMLRocks's own operations (local walltime; CI instruments)
```

See the [performance guide](/guides/performance/) for the headline numbers and
how to read them.

## Fuzzing

`tests/robustness/test_fuzz.py` is a fast, always-on property check. For deeper,
coverage-guided fuzzing of the Rust parser there are four
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) targets under `fuzz/`:

| Target         | What it drives                                             | Contract                    |
| -------------- | ---------------------------------------------------------- | --------------------------- |
| `parse`        | scanner → parser → composer (the round-trip AST)           | never panic or hang         |
| `decode`       | the fast `loads` path and fast `dumps`, under both schemas | never panic or hang         |
| `roundtrip`    | compose → emit → re-compose (the round-trip emitter)       | never panic or hang         |
| `differential` | `loads(dumps(loads(x)))` must equal `loads(x)`             | never silently corrupt data |

```bash
cargo install cargo-fuzz          # once; needs a nightly toolchain
just fuzz 60                       # fuzz `parse` for 60s (the default target)
just fuzz 60 differential          # fuzz any target by name
```

The first three targets check that no input _crashes_ the parser. `differential`
checks something a crash-only target cannot: that `dumps` never emits YAML which
`loads` reads back as _different_ data (a mis-quoted string re-resolving to a
bool, a float losing precision). That shows up as wrong values, not a panic, so
only comparing the two trees surfaces it.
[ClusterFuzzLite](https://google.github.io/clusterfuzzlite/) builds and runs
every target for a short batch on each pull request. A `parse`/`decode`/`roundtrip`
crash is a parser bug; a `differential` failure is a correctness bug.

## Code quality

All checks are wired into [prek](https://prek.j178.dev) (a drop-in pre-commit
runner). `just precommit` runs the exact hook set CI runs, and `just check` runs
the full local gate (build, every hook, the Rust and Python suites, and the
documented examples) in one go:

```bash
just precommit       # every pre-commit hook (the set CI runs)
just check           # the full gate: build, hooks, tests, examples
```

You can also run each linter on its own:

```bash
just lint            # ruff check + ruff format --check (Python lint + format)
just typecheck       # mypy and ty (Python typing)
just clippy          # cargo clippy --all-targets -D warnings (Rust lint)
just fmt             # auto-format Python and Rust in place
just spellcheck      # codespell
```

CI enforces all of the above, so make sure they pass before opening a pull
request.

### Dependency audits

Advisory and supply-chain checks run on a schedule and whenever a lockfile
changes. Run them locally with `just audit` (all ecosystems at once), or one at a
time with `just audit-rust`, `just audit-python`, and `just audit-docs`.

## Conventions

- Match the style of the surrounding code; the codebase favors small, well-named
  functions and explicit error handling.
- Add tests for new behavior. Round-trip changes must preserve byte-for-byte
  fidelity for unmodified documents (the YAML test suite enforces this).
- Update the documentation under `docs/` when adding or changing a user-facing
  feature, and keep its examples runnable: `just examples` runs every documented
  example and verifies its output, and `just docs-dev` serves the site locally
  with live reload.
- There is no changelog to edit: release notes are drafted automatically by
  [Release Drafter](https://github.com/release-drafter/release-drafter) from
  merged pull requests, grouped by label. A clear pull request title becomes the
  release-note line.

## See also

- [Architecture](/contributing/architecture/): how the parser is put together.
- [Security](/reference/security/): the threat model and how to report issues.
- [License](/license/): the terms YAMLRocks is distributed under.
