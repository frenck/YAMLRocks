---
title: Projects using YAMLRocks
description: Where YAMLRocks fits, and who is using it.
---

YAMLRocks is young. This page tracks projects that use it and, just as usefully,
the ecosystems it is built to serve. If you adopt YAMLRocks, please
[open a pull request](https://github.com/frenck/yamlrocks) to add yourself here.

This page lists actual adopters only. For the public repositories YAMLRocks is
tested against as a compatibility corpus, see
[real-world verification](/verification/real-world-corpus/).

## Using YAMLRocks

_Be the first!_ There are no public adopters yet. If your project uses YAMLRocks,
we would love to list it.

## Where YAMLRocks fits

YAMLRocks was designed for YAML-heavy Python projects that need speed, correctness,
and round-trip fidelity at the same time. The following ecosystems are the
primary motivation for its feature set.

### Home Assistant

Home Assistant has a large, split YAML configuration with `!include`, `!secret`,
and `!env_var`, and it tracks source lines for friendly error messages. YAMLRocks
implements that entire tag set with matching semantics, parses faster, and adds
**writable includes**: load the config, edit one automation, and save only the
changed file. That unlocks reliable UI-driven config editing.

See the [includes guide](/guides/includes/) and
[annotated mode](/guides/annotated/).

### ESPHome

ESPHome compiles YAML device configurations with `!include`, `!secret`,
`!lambda`, `!extend`, and a substitutions system. YAMLRocks covers the include and
secret tags directly, and the ESPHome-specific tags map onto a `tag_handler`.
The native include resolver is dramatically faster for configs split across many
files.

### Ansible

Ansible parses playbooks and inventories with source-position tracking and
custom tags like `!vault` and `!unsafe`. YAMLRocks offers source locations,
YAML 1.1 mode, and tag handling, though Ansible's heavy use of annotated string
subclasses is an area still being explored.

## Why choose YAMLRocks

- **Fast**: Rust-backed; competitive with PyYAML's C loader and far faster than
  pure-Python round-trip libraries. See [Performance](/guides/performance/).
- **Correct**: validated against the official YAML test suite plus snapshot and
  fuzz corpora, and a public real-world compatibility corpus.
- **Round-trip**: edit a value and re-emit with the rest of the document preserved.
- **Safe**: never executes arbitrary code from tags; bounded against alias bombs,
  deep nesting, and include cycles.
