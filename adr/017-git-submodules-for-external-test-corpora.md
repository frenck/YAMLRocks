# ADR-017: Git submodules for external test corpora

**Date**: 2026-06-08
**Status**: Accepted

**Context**: Two test corpora come from outside this repository: the official
YAML test suite (the correctness oracle) and a set of large, real public
configurations from many ecosystems (Home Assistant, ESPHome, Ansible,
Kubernetes, Docker Compose, and more), whose byte-for-byte round-trip is the
strongest real-world signal for the parser. The project's first instinct was to
copy the YAML test suite into the repo and ban submodules outright. That was fine
for one small, rarely changing corpus, but it stopped scaling the moment the
real-world corpus arrived, and the two corpora pull in opposite directions under
a single rule.

**Decision**: Reference both corpora as git submodules, kept apart from the test
code under a dedicated data tree:

- the YAML test suite at `tests/data/yaml_test_suite/cases`, tracking its `data`
  branch;
- the real-world configs at `tests/data/realworld/<ecosystem>/<repo>`, pinned by
  commit and organized by ecosystem.

The test code stays under `tests/compliance/` and `tests/realworld/`; only the
data lives in `tests/data/`, so data and tests are cleanly separated.

**Rationale**:

- **Licensing**: the real-world configs are other people's repositories, mostly
  with no license or an all-rights-reserved default. Copying them into this MIT
  repo would be a redistribution we have no right to make. A submodule
  _references_ a repo by URL and commit; it never copies their files into ours.
- **Size and churn**: the corpus is megabytes of files their authors keep
  changing. Vendoring would bloat the repo and demand manual re-syncs; pinned
  commits keep it reproducible, and `--remote` bumps stay deliberate.
- **One mechanism, one place**: once submodules are the tool for the real-world
  corpus, holding the YAML test suite to a separate vendoring rule earns nothing.
  The suite is small and changes rarely, so a pinned submodule is just as
  reproducible while dropping the bespoke vendoring step, and both corpora now
  sit side by side under `tests/data/`.

**Alternatives considered**: vendoring a curated subset (rejected: redistribution
we cannot license for the real-world configs, and a second mechanism for the test
suite); a pinned fetch script cloning into a gitignored directory (rejected: a
bespoke reimplementation of what submodules already do natively).

**Consequences**:

- Contributors run `git submodule update --init <path>` to exercise either
  category; both **auto-skip** when their submodule is absent, so the default
  suite stays green without them. CI checks out only the small YAML-suite
  submodule in the main test matrix (keeping it fast) and the full set in the
  coverage and real-world jobs.
- The compliance harness reads the suite's `data`-branch layout directly,
  exercising the single-document cases (those with a top-level `in.yaml`) and
  skipping the multi-document variants; `expectations.json` baselines the cases
  the pragmatic parser does not yet handle.
- The real-world category surfaced and fixed a real parser bug on day one: a
  plain scalar's continuation line beginning with a quote (`text\n  'more'`) was
  wrongly rejected; in block context quotes are ordinary plain-scalar characters,
  so it now folds. Two files that are genuinely not valid standalone YAML (a
  Jinja template; a config with a spec-invalid `'#x'# comment`) are recorded as
  strict xfails rather than silenced.
- First multi-ecosystem run: 8 repos across 5 ecosystems, 592 YAML files, **590
  parse + round-trip byte-identical** with 0 round-trip diffs (the 2 xfails are
  the genuinely-invalid HA files). The 477 non-HA files (ESPHome, Ansible,
  Kubernetes, Docker Compose, including heavy multi-document streams) all passed
  unchanged, strong evidence the parser generalizes beyond Home Assistant.
