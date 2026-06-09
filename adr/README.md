# Architecture Decision Records

Each record captures what was chosen, the alternatives considered, and why.

| ADR                                                                             | Title                                                                   |
| ------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| [ADR-001](001-build-a-custom-yaml-parser-instead-of-using-saphyr-parser.md)     | Build a custom YAML parser instead of using saphyr-parser               |
| [ADR-002](002-use-pyo3-ffi-low-level-instead-of-high-level-pyo3.md)             | Use pyo3-ffi (low-level) instead of high-level PyO3                     |
| [ADR-003](003-return-bytes-from-dumps-not-str.md)                               | Return bytes from dumps(), not str                                      |
| [ADR-004](004-single-parser-with-dual-resolver-for-yaml-1-1-1-2.md)             | Single parser with dual resolver for YAML 1.1/1.2                       |
| [ADR-005](005-bitflag-options-pattern-opt.md)                                   | Bitflag options pattern (OPT\_\*)                                       |
| [ADR-006](006-astro-starlight-for-documentation.md)                             | Astro Starlight for documentation                                       |
| [ADR-007](007-maturin-as-build-system.md)                                       | maturin as build system                                                 |
| [ADR-008](008-mit-license.md)                                                   | MIT license                                                             |
| [ADR-009](009-start-on-high-level-pyo3-defer-pyo3-ffi-to-a-performance-pass.md) | Start on high-level PyO3, defer pyo3-ffi to a performance pass          |
| [ADR-010](010-annotated-mode-subclasses-dict-list-and-str.md)                   | Annotated mode subclasses dict, list, and str                           |
| [ADR-011](011-reattach-comments-by-source-position-in-a-post-pass.md)           | Reattach comments by source position in a post-pass                     |
| [ADR-012](012-pyo3-ffi-materialization-pass-for-the-fast-paths.md)              | pyo3-ffi materialization pass for the fast paths                        |
| [ADR-013](013-zero-copy-scalar-scanning-with-cow-input-str.md)                  | Zero-copy scalar scanning with `Cow<'input, str>`                       |
| [ADR-014](014-json-export-via-to-json-no-from-json-no-async-serializer.md)      | JSON export via `to_json`; no `from_json`, no async serializer          |
| [ADR-015](015-a-uniform-node-cursor-for-round-trip-metadata.md)                 | A uniform `YAMLRocksNode` cursor for round-trip metadata                |
| [ADR-016](016-anchor-discovery-and-detach-on-the-node-cursor-defer-creation.md) | Anchor discovery and detach on the YAMLRocksNode cursor; defer creation |
| [ADR-017](017-git-submodules-for-external-test-corpora.md)                      | Git submodules for external test corpora                                |
| [ADR-018](018-anchor-alias-creation-with-validity-guards.md)                    | Anchor/alias creation with validity guards                              |
| [ADR-019](019-in-file-schema-references-yaml-language-server-directive.md)      | In-file schema references (yaml-language-server directive)              |
| [ADR-020](020-annotated-mode-and-the-node-cursor-stay-separate-apis.md)         | Annotated mode and the `YAMLRocksNode` cursor stay separate APIs        |
