---
title: Credits
description: The open-source projects YAMLRocks is built on, tested with, and inspired by.
---

YAMLRocks stands on a great deal of excellent open-source work. This page is our
thank-you to the projects and people that make it possible.

## Built on

- **[Rust](https://www.rust-lang.org/)** and **[PyO3](https://pyo3.rs/)** give
  YAMLRocks a safe, fast native core and its bridge to Python.
- **[maturin](https://www.maturin.rs/)** builds and packages the extension into
  wheels for every platform.

### Bundled Rust crates

The compiled extension links the Rust crates below. The complete list, including
transitive crates and their full license texts, is generated into
[`THIRD_PARTY_LICENSES.md`](https://github.com/frenck/yamlrocks/blob/main/THIRD_PARTY_LICENSES.md)
and shipped inside every wheel under `.dist-info/licenses/`.

| Crate                                         | Purpose                                      | License           |
| --------------------------------------------- | -------------------------------------------- | ----------------- |
| [pyo3](https://crates.io/crates/pyo3)         | Python bindings for the native core          | MIT OR Apache-2.0 |
| [smallvec](https://crates.io/crates/smallvec) | Small-buffer optimization on hot paths       | MIT OR Apache-2.0 |
| [stacker](https://crates.io/crates/stacker)   | On-demand native stack growth for deep input | MIT OR Apache-2.0 |

Each crate brings its own transitive dependencies (the `pyo3` macro crates,
`libc`, `once_cell`, `unicode-ident`, and a few others); all are permissive
(MIT, Apache-2.0, Unicode-3.0) and all are credited in full in the file linked
above.

## Tested with

- The official **[YAML test suite](https://github.com/yaml/yaml-test-suite)** is
  our correctness oracle for spec compliance.
- A corpus of real-world configurations from across the ecosystem (Home
  Assistant, ESPHome, Ansible, Kubernetes, Docker Compose, and more) keeps
  YAMLRocks honest against YAML that people actually ship. Thank you to every
  author whose public configuration we parse; the
  [corpus list](https://github.com/frenck/yamlrocks/blob/main/tests/realworld/README.md)
  names each one.

## Inspired by

- **[orjson](https://github.com/ijl/orjson)** set the bar for what a fast,
  Rust-backed Python serialization library can be.
- **[PyYAML](https://pyyaml.org/)**,
  **[ruamel.yaml](https://pypi.org/project/ruamel.yaml/)**, and
  **[yamlium](https://pypi.org/project/yamlium/)** are the shoulders the Python
  YAML world stands on; our comparison benchmarks measure against them.

## Tooling

Day-to-day development leans on **[uv](https://docs.astral.sh/uv/)**,
**[Ruff](https://docs.astral.sh/ruff/)**,
**[mypy](https://mypy-lang.org/)** and **[ty](https://github.com/astral-sh/ty)**,
**[zizmor](https://docs.zizmor.sh/)**, and the Rust toolchain (Clippy, rustfmt).
This documentation site is built with
**[Astro Starlight](https://starlight.astro.build/)**.

## License

YAMLRocks is released under the
[MIT license](https://github.com/frenck/yamlrocks/blob/main/LICENSE). The bundled
Rust crates are licensed as noted above, and their full license texts travel with
every wheel.
