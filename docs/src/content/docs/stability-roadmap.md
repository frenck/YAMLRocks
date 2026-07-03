---
title: Stability and roadmap
description: What YAMLRocks treats as stable, what may change before 1.0, and what blocks a 1.0 release.
---

YAMLRocks is currently **pre-1.0 alpha software**. It is built with a serious
stability bar, but the project is still young enough that some advanced APIs may
change before the first stable release.

This page is the contract: what users can rely on now, what is still allowed to
move, and what needs to be true before YAMLRocks calls itself 1.0.

## Stable now

These behaviors are treated as core. Regressions here are bugs.

- **Safe by default**: loading YAML never executes arbitrary Python objects from
  tags. There is no unsafe loader equivalent to PyYAML's historical `yaml.load`
  footgun.
- **YAML 1.2 by default**: `yes`, `no`, `on`, and `off` are strings unless YAML
  1.1 mode is explicitly requested.
- **Round-trip preservation**: a document loaded with `OPT_ROUND_TRIP` preserves
  comments, anchors, and formatting; an unmodified UTF-8 document re-emits
  byte-for-byte. (UTF-16 and UTF-32 are read per the spec but re-emit as UTF-8,
  so byte fidelity is a UTF-8 guarantee.)
- **Real-world verification**: standalone YAML files in the public compatibility
  corpus parse and round-trip without loss.
- **Structured errors**: YAMLRocks errors carry machine-readable location
  details when a location is known.
- **No silent secret write-back**: resolved `!secret` and `!env_var` values are
  not written back into configuration files in round-trip mode.
- **Typed Python package**: the public Python API ships with type stubs.

## Stable enough for early adopters

These APIs are intended to last, but may still receive small compatibility fixes
before 1.0 if production usage shows a better shape.

- `loads`, `loads_all`, `dumps`, `load`, `load_all`, and `dump`.
- The PyYAML-compatible `yamlrocks.compat` safe API.
- YAML 1.1 compatibility and upgrade options.
- Annotated mode source locations.
- Native include resolution and write-back.
- JSON export with `to_json`.
- JSON Schema validation and in-file schema reference discovery.
- The `YAMLRocksTags` registry, `tag_handler`, and `OPT_PASSTHROUGH_TAG` model.

Changes in this category should be rare, documented in the changelog, and shaped
to keep migration straightforward.

## May change before 1.0

The project may still adjust these surfaces before declaring a stable 1.0 API.

- Exact option flag names for newer or advanced features.
- The detailed `YAMLRocksDocument`, `YAMLRocksDocumentView`, and `YAMLRocksNode` editing API.
- Edge-case formatting choices for edited nodes, especially where preserving
  surrounding layout conflicts with producing normalized YAML.
- Custom tag callback signatures if real-world integrations reveal missing
  context.
- Include graph diagnostics and save-report details.
- Performance numbers and benchmark payloads as the harness grows.

Any breaking change should have a clear reason: correctness, safety, a better
long-term API, or alignment with real production use.

## What blocks 1.0

YAMLRocks should not call itself 1.0 just because its own test suite is green.
The real blocker is confidence from downstream use: projects loading, validating,
editing, and writing their actual configuration without hidden compatibility
traps.

The main 1.0 gates are:

- **Real-world battle testing**: YAMLRocks needs feedback from real integrations,
  not only synthetic tests and pinned corpus runs. Home Assistant, ESPHome,
  config editors, CI tooling, and other YAML-heavy projects should exercise the
  paths they would actually depend on.
- **Migration compatibility**: projects moving from PyYAML or ruamel.yaml need a
  clear migration path. Initial testing with ESPHome and Home Assistant showed
  that PyYAML sometimes accepts behavior outside strict YAML 1.1. Where projects
  rely on those quirks, YAMLRocks needs either a deliberate compatibility path or
  a documented migration story.
- **Round-trip editing under real changes**: an unmodified file already re-emits
  byte-for-byte, but 1.0 also needs confidence that common edits preserve
  comments, includes, anchors, and layout well enough for real configuration
  workflows.
- **Operational feedback**: error messages, exception classes, source locations,
  and include diagnostics need to be understandable to people debugging real
  config files, not only parser developers.
- **Release confidence**: wheels, provenance, SBOMs, smoke tests, and CI gates
  need to survive real release cycles, but those are supporting requirements.
  They are not a substitute for downstream adoption feedback.

## Versioning policy

Before 1.0, minor releases may include breaking changes when needed, but they
should be called out clearly in the changelog and release notes.

After 1.0, YAMLRocks follows semantic versioning:

- patch releases fix bugs without intentional API breaks;
- minor releases add backwards-compatible features;
- major releases are reserved for incompatible API or behavior changes.

Parser strictness is special: a patch or minor release may reject YAML that was
previously accepted if the input is invalid according to the YAML specification,
or if accepting it creates a safety risk. Such changes are documented because
they can surface in user configuration.

## See also

- [Real-world verification](/verification/real-world-corpus/): the public corpus
  used to catch parser and round-trip regressions.
- [Security](/reference/security/): the safety model and trust boundaries.
- [Migration compatibility](/getting-started/compatibility/): what matches,
  what differs, and what still needs battle testing.
- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/): compatibility
  notes for the most common migration path.
- [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/): what to
  expect when moving round-trip workflows.
