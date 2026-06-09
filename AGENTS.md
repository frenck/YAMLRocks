# AGENTS.md

Guidance for AI coding agents (and humans) working in this repository. This file
follows the [agents.md](https://agents.md) convention.

## What this project is

YAMLRocks is a fast, correct YAML library for Python, implemented in Rust with PyO3
and built by maturin. The Rust crate lives in `src/`; the Python package and
type stubs live in `pysrc/yamlrocks/`.

## Project layout

| Path               | Purpose                                                                               |
| ------------------ | ------------------------------------------------------------------------------------- |
| `src/scanner/`     | Byte reader, tokenizer, comment extraction                                            |
| `src/parser/`      | Tokens to events                                                                      |
| `src/resolver/`    | Scalar typing (YAML 1.1 and 1.2 schemas)                                              |
| `src/decode/`      | Fast path: events to a `Value` tree (+ merge keys)                                    |
| `src/encode/`      | Fast path: `Value` tree to YAML bytes                                                 |
| `src/roundtrip/`   | Rich AST, composer, emitter, `YAMLRocksDocument`, upgrade                             |
| `src/include/`     | `!include`/`!secret`/`!env_var`/`!input` resolution                                   |
| `src/schema/`      | JSON Schema validation against the AST                                                |
| `src/ffi/`         | PyO3 module: functions, types, conversions                                            |
| `pysrc/yamlrocks/` | Python package, stubs, PyYAML compat shim                                             |
| `tests/`           | pytest suite, grouped by capability (see `tests/README.md`); data under `tests/data/` |
| `docs/`            | Astro Starlight documentation site                                                    |
| `adr/`             | Architecture decision records (the why behind major choices)                          |

## The `just` task runner

This project uses [uv](https://docs.astral.sh/uv/) for the Python side and
[just](https://github.com/casey/just) as the task runner. **`just` is the primary
interface: every common workflow (build, test, lint, type-check, docs) has a
recipe, and the recipes wrap the exact `uv`/`cargo` commands CI runs.** Run
`just --list` to see them all, or `just` on its own for the same list.

Run a recipe either by activating the venv once (`uv sync && source
.venv/bin/activate`, then `just <recipe>`) or without activating by prefixing
`uv run --no-sync just <recipe>`. Recipes assume `just setup` (or `just develop`)
has run at least once.

| Recipe                        | What it does                                                                    |
| ----------------------------- | ------------------------------------------------------------------------------- |
| `just setup`                  | Install all dev dependencies into the uv-managed venv                           |
| `just develop`                | Build + install the extension in **release** mode (rerun after Rust changes)    |
| `just develop-debug`          | Faster **debug** build for tight iteration loops                                |
| `just test [args]`            | Run the Python suite (`pytest`); args pass through, e.g. `just test -k anchors` |
| `just test-rust [args]`       | Run the Rust unit tests (`cargo test --lib`)                                    |
| `just lint`                   | Lint and format-check Python (read-only)                                        |
| `just fmt`                    | Auto-format Python **and** Rust in place                                        |
| `just typecheck`              | Type-check with both `mypy` and `ty`                                            |
| `just clippy`                 | Lint the Rust crate (`-D warnings`)                                             |
| `just spellcheck`             | Spell-check with `codespell`                                                    |
| `just precommit [hook]`       | Run every pre-commit hook via `prek` (the exact set CI runs)                    |
| `just check`                  | **Full local gate**: build, all hooks, Rust + Python suites, doc examples       |
| `just examples`               | Run every documented Python example and verify its output comments              |
| `just docs` / `just docs-dev` | Build the docs site / serve it with live reload                                 |
| `just bench [args]`           | Comparison benchmarks vs PyYAML, ruamel.yaml, and yamlium                       |
| `just audit`                  | Audit every dependency ecosystem for advisories                                 |
| `just licenses`               | Regenerate `THIRD_PARTY_LICENSES.md` from the crate graph                       |
| `just coverage`               | Rust line coverage measured through the Python suite                            |
| `just fuzz [seconds]`         | Fuzz the parser, optionally time-boxed                                          |
| `just clean`                  | Remove build and fuzzing artifacts                                              |

The sections below give the raw commands behind the most important recipes, for
when you need to run a variant by hand.

## Building

```bash
uv sync --no-install-project              # create the venv, install dev deps (just setup)
uv run --no-sync maturin develop          # build + install the extension, debug (just develop-debug)
# `just develop` builds the same thing with --release.
```

Prefix any tool with `uv run --no-sync` to use the project venv without
re-syncing, or activate it once with `source .venv/bin/activate`.

## Running tests (IMPORTANT)

`just test` runs the Python suite and `just test-rust` runs the Rust one. When
invoking `pytest` directly, always run it under a memory and time guard: a parser
bug can otherwise exhaust memory and take down the host. `tests/conftest.py` caps
`RLIMIT_AS`, but add a shell guard too:

```bash
timeout 200 bash -c 'ulimit -v 4000000; uv run --no-sync pytest -q'   # raw form of: just test
cargo test --lib                                                      # raw form of: just test-rust
```

When working on the round-trip emitter, regenerate snapshots intentionally:

```bash
UPDATE_SNAPSHOTS=1 uv run --no-sync pytest tests/compliance/test_snapshots.py
```

Regenerate the YAML test suite baseline after parser changes:

```bash
uv run --no-sync python tests/data/yaml_test_suite/generate_expectations.py
```

## Quality gates (all enforced in CI)

`just check` runs the complete local gate, and `just precommit` runs the exact
hook set CI runs. The individual gates, for running one at a time:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings        # just clippy
uv run --no-sync ruff check . && uv run --no-sync ruff format --check .   # just lint
uv run --no-sync mypy pysrc/yamlrocks            # \
uv run --no-sync ty check pysrc/yamlrocks        #  > just typecheck
uv run --no-sync codespell                       # just spellcheck
```

## Conventions

- Keep public Rust items documented with `///` rustdoc.
- Every Python test has a one-line docstring describing what it verifies.
- Round-trip fidelity is sacred: an unmodified document must re-emit
  byte-for-byte. Do not regress this (the YAML test suite enforces it).
- Match the surrounding code style; prefer small, well-named functions.
- External test corpora (the YAML test suite and the real-world config corpus)
  live as git submodules under `tests/data/` (ADR-017); they auto-skip when the
  submodule is absent, so the default suite stays green without them.

## Gotchas

- `Date.now()` / randomness are fine in tests but seed any generators.
- The fast path (`loads`/`dumps`) must stay allocation-light; the round-trip
  path may be heavier.
- PyO3 (Rust) cannot natively subclass `str`/`int`/`float`, so the annotated
  string and number types are thin pure-Python subclasses while annotated
  `dict`/`list` are Rust pyclasses (see
  [`adr/010-...`](adr/010-annotated-mode-subclasses-dict-list-and-str.md)).

## Where to read next

- `adr/` (start at `adr/README.md`): architecture decision records.
- `docs/`: the full user-facing documentation (Astro Starlight).
- `CONTRIBUTING.md`: contributor workflow.
