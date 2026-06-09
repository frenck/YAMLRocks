---
title: About YAMLRocks
description: Why YAMLRocks exists, the state of YAML in Python, and who builds it.
---

YAMLRocks exists because Python deserves a YAML library that is fast, correct,
and able to preserve a document exactly as a human wrote it, all at once.

## YAML runs Home Assistant

[Home Assistant](https://www.home-assistant.io) is configured in YAML, and it
leans on it hard. A real installation is rarely a single file: a
`configuration.yaml` pulls in dozens or hundreds of others through `!include`
and the `!include_dir_*` family, organized into packages, dashboards,
automations, scripts, and scenes. All of it is parsed at startup and re-parsed on
every reload, so YAML parsing sits squarely on a hot path that millions of
installations hit every day.

Home Assistant cares about more than the values, too. It tracks where each
setting came from (which file, which line), so an error can point a user at the
exact spot, even when that spot is buried several includes deep. To get that, the
project maintains `annotatedyaml`, a wrapper around PyYAML that bolts source
locations onto the result. It works, but it is a patch over a parser that was
never built for it.

## The state of YAML in Python is rough

Anyone who has reached for YAML in Python ends up choosing between two
compromises:

- **PyYAML** is the default. With its C loader it is reasonably fast, but it
  speaks only YAML 1.1, discards comments, cannot round-trip a document, and its
  codebase has aged with little movement for years.
- **ruamel.yaml** is the capable one. It implements YAML 1.2, keeps comments, and
  round-trips faithfully, but it is pure Python and pays for all of that in
  speed, which is exactly the wrong trade-off for a startup-time hot path.

There simply was not a library that was _fast and modern and able to round-trip_.
`orjson` showed years ago what a Rust- or C-backed core does for JSON in Python.
Nothing had done the same for YAML.

## So this is that library

YAMLRocks is a YAML library with Rust at its core: fast like `orjson`,
feature-rich like ruamel.yaml, and faithful to the YAML 1.2 specification (with a
YAML 1.1 compatibility mode for the older spellings). It keeps comments, anchors,
and formatting through a round-trip, resolves and writes back `!include` trees
natively, tracks source locations, and is hardened against the YAML attack
classes by default. The whole engine (scanner, parser, emitter) is designed and
written from scratch, and exercised against the official YAML test suite plus
thousands of real-world configuration files from many ecosystems.

The name says the goal: rock-solid YAML, with Rust at the core (the R in Rock).

## Who builds it

YAMLRocks is created and written by **Franck Nijhof**, better known as
**Frenck**, a [GitHub Star](https://stars.github.com/profiles/frenck/) and the
Home Assistant lead, where he has spent years working with YAML at a scale and on
a hot path that most projects never see. That experience, and a long-running frustration
with the choices above, is where YAMLRocks comes from.

Find more of his work at [frenck.dev](https://frenck.dev) and on GitHub at
[@frenck](https://github.com/frenck).
