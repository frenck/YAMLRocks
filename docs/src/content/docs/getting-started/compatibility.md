---
title: Migration compatibility
description: What YAMLRocks matches today, what intentionally differs, and what still needs battle testing before 1.0.
---

YAMLRocks is meant to be easy to adopt, but YAML compatibility is not one thing.
Projects usually depend on a mix of library API, schema behavior, parser
leniency, custom tags, error reporting, and formatting. This page makes those
layers explicit so a migration can be planned without surprises.

The short version: the safe PyYAML surface has a compatibility shim, ruamel-style
round-trip editing has a direct YAMLRocks workflow, and YAML 1.1-era booleans can
be read deliberately. The work before 1.0 is mostly real downstream testing:
finding which PyYAML quirks projects accidentally rely on, then deciding whether
YAMLRocks should support them, warn about them, or reject them with clear docs.

## Strict defaults, explicit options

YAMLRocks tries to keep the default path as pure and predictable as possible:
YAML 1.2 semantics, safe loading, spec-compliant parsing, and round-trip
preservation when requested. That does not mean every project has to adopt those
defaults all at once.

The `OPT_*` flags are the compatibility surface. They let a project opt into the
behavior it needs for migration, legacy configuration, or domain-specific tags
without weakening the default behavior for everyone else. A project can keep
using PyYAML's practical behavior, even where it differs from strict YAML 1.1,
by enabling `OPT_PYYAML_COMPAT`. Another project can choose strict YAML 1.1 with
`OPT_YAML_1_1`, or read legacy 1.1 spellings while writing canonical 1.2 with
`OPT_UPGRADE_1_1`.

In other words: YAMLRocks is opinionated by default, but configurable on purpose.
See the [options reference](/reference/options/) for the full flag surface.

## Compatibility matrix

| Area                                | Status                                | Migration path                                                                           |
| ----------------------------------- | ------------------------------------- | ---------------------------------------------------------------------------------------- |
| PyYAML safe loading                 | Compatible for common `safe_load` use | Use `yamlrocks.compat.safe_load` or native `yamlrocks.loads`                             |
| PyYAML safe dumping                 | Compatible through shim               | Use `yamlrocks.compat.safe_dump` for `str` output and PyYAML-style sorting               |
| PyYAML unsafe object tags           | Intentionally not supported           | Replace unsafe constructors with explicit `tags` or `tag_handler` callbacks              |
| PyYAML 1.1 booleans                 | Supported with options                | Use `OPT_PYYAML_COMPAT` for PyYAML's boolean set, or `OPT_YAML_1_1` for spec 1.1         |
| PyYAML parser leniency              | Case by case before 1.0               | Test real configs; use documented compatibility paths where they exist                   |
| PyYAML alias object identity        | Supported on rich paths               | Use `OPT_ANNOTATED`, `OPT_ROUND_TRIP`, or custom-tag paths when identity matters         |
| ruamel safe load/dump               | Compatible conceptually               | Use native `loads`, `load`, `dumps`, and `dump`                                          |
| ruamel round-trip editing           | Supported with different API          | Use `OPT_ROUND_TRIP`, `YAMLRocksDocument`, `YAMLRocksDocumentView`, and `YAMLRocksNode`  |
| ruamel fine-grained comment editing | Not equivalent yet                    | YAMLRocks preserves comments, but does not expose a full `.ca`-style authoring API       |
| Application tags                    | Supported explicitly                  | Use `tags`, `tag_handler`, `OPT_PASSTHROUGH_TAG`, or domain flags such as `OPT_INCLUDES` |
| Source locations                    | Supported                             | Use `OPT_ANNOTATED`, round-trip `YAMLRocksNode` handles, and structured exceptions       |
| Includes and secrets                | Supported with trust-boundary flags   | Enable only the tags a document is trusted to use                                        |

## PyYAML migration modes

PyYAML compatibility has three useful levels.

| Need                   | Use                                   | Notes                                                                                                  |
| ---------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| Drop-in safe API       | `import yamlrocks.compat as yaml`     | Keeps `safe_dump` returning `str` and accepts common PyYAML keyword arguments.                         |
| Native fast API        | `yamlrocks.loads` / `yamlrocks.dumps` | Faster and smaller, but `dumps` returns `bytes` and YAML 1.2 is the default.                           |
| Legacy scalar behavior | `OPT_PYYAML_COMPAT`                   | Reads PyYAML's off-spec boolean set, useful for Home Assistant, ESPHome, and Ansible style migrations. |

`OPT_YAML_1_1` follows the YAML 1.1 specification. `OPT_PYYAML_COMPAT` follows
PyYAML's practical behavior where words like `yes`, `no`, `on`, and `off` are
booleans, but single-letter `y` and `n` stay strings. That difference matters for
real configurations that use `y` as a coordinate or ordinary key.

```python
import yamlrocks

source = """
y: 2
on: 5
"""

yamlrocks.loads(source, option=yamlrocks.OPT_YAML_1_1)
# {True: 5}

yamlrocks.loads(source, option=yamlrocks.OPT_PYYAML_COMPAT)
# {'y': 2, True: 5}
```

For a gradual migration, combine compatibility reading with the upgrade path:

- use `OPT_YAML_1_1_WARN` to discover values that behave differently between
  schemas;
- use `OPT_UPGRADE_1_1` when you want to accept old spellings while writing back
  canonical YAML 1.2;
- use `OPT_PYYAML_COMPAT` when the project is migrating from PyYAML behavior, not
  strict YAML 1.1 behavior.

See [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/) for the scalar details.

## Known PyYAML leniency edges

PyYAML accepts some inputs that are not valid YAML according to the spec. Some
projects accidentally rely on this because PyYAML has been the default library
for a long time. These cases are the migration edges that need real-world battle
testing before 1.0.

| Pattern                                                         | YAMLRocks position                    | Migration note                                                          |
| --------------------------------------------------------------- | ------------------------------------- | ----------------------------------------------------------------------- |
| Comments not preceded by whitespace                             | Rejects as invalid YAML               | Add whitespace before `#` or quote the value when `#` is data.          |
| Multi-line quoted scalar continuations at the block indent      | Rejects as invalid YAML               | Indent continuation lines past the parent block.                        |
| Flow collection content not indented past the surrounding block | Rejects as invalid YAML               | Re-indent the flow collection or use block style.                       |
| Template files with `.yaml` extension                           | Not standalone YAML                   | Render with the owning tool first, or exclude from parser-level checks. |
| Unknown application tags                                        | Preserved or passed through by opt-in | Register handlers only for tags the application wants to interpret.     |

These are not all permanent decisions. For each real downstream project, the
question is whether a compatibility mode would make migration safer without
weakening the parser's default correctness and security.

## ruamel.yaml migration notes

ruamel.yaml users usually migrate for speed while keeping comment-preserving
edits. YAMLRocks is closest when the workflow starts from an existing file,
changes values, and writes the same document back.

| Need                                           | YAMLRocks support                               | Notes                                                         |
| ---------------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------- |
| Preserve comments and formatting               | `OPT_ROUND_TRIP`                                | Unmodified documents re-emit byte-for-byte.                   |
| Edit mapping and sequence values               | `YAMLRocksDocument` and `YAMLRocksDocumentView` | Normal indexing and assignment write through to the AST.      |
| Inspect anchors, tags, comments, and locations | `YAMLRocksNode` handles                         | Use `YAMLRocksDocument.node` or `YAMLRocksDocumentView.node`. |
| Build commented documents from scratch         | Limited before 1.0                              | Preserve-and-edit is the primary target today.                |
| Move or author individual comments             | Limited before 1.0                              | ruamel's `.ca` API is still more complete here.               |

See [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/) and
[Round-trip editing](/guides/round-trip/) for the editing workflow.

## How to test a migration

For a project currently using PyYAML or ruamel.yaml, start with a shadow run
instead of replacing the parser outright.

1. Load the same files with the current library and YAMLRocks.
2. Compare the resulting native values for the paths that matter to the
   application.
3. For config editors, load with `OPT_ROUND_TRIP` and confirm unmodified files
   write back byte-for-byte.
4. Enable `OPT_YAML_1_1_WARN` or `OPT_PYYAML_COMPAT` when migrating from PyYAML
   and inspect scalar warnings.
5. Record any PyYAML leniency cases separately from real parser bugs.

When a migration depends on behavior outside strict YAML, please open an issue
with the smallest file that demonstrates it. Those reports are exactly what
should shape the 1.0 compatibility contract.

## See also

- [Migrating from PyYAML](/getting-started/migrating-from-pyyaml/): drop-in shim
  and behavior differences.
- [Migrating from ruamel.yaml](/getting-started/migrating-from-ruamel/):
  round-trip editing migration.
- [YAML 1.1 vs 1.2](/guides/yaml-11-vs-12/): schema modes and upgrade warnings.
- [Stability and roadmap](/stability-roadmap/): what still blocks 1.0.
