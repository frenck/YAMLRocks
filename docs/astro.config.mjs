// @ts-check
import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightLlmsTxt from "starlight-llms-txt";

import rehypeYamlrocks from "./src/plugins/rehype-yamlrocks.mjs";

// https://astro.build/config
export default defineConfig({
  site: "https://yaml.rocks",
  markdown: {
    rehypePlugins: [rehypeYamlrocks],
  },
  integrations: [
    starlight({
      title: "YAMLRocks",
      description: "Rock-solid YAML for Python, written in Rust.",
      plugins: [
        starlightLlmsTxt({
          projectName: "YAMLRocks",
          description:
            "YAMLRocks is a fast, correct YAML library for Python, implemented in Rust with PyO3. It is safe by default (tags never construct arbitrary objects), defaults to the YAML 1.2 core schema, reads and writes bytes without an extra encode step, and adds a comment-preserving round-trip mode, native `!include` resolution, JSON Schema validation, and source-tracked annotated mode.",
          details: [
            "Important notes for working with YAMLRocks:",
            "",
            "- Install with `pip install yamlrocks`; the import name is `yamlrocks`. Prebuilt wheels ship for CPython (including free-threaded builds); no C toolchain is needed.",
            "- The core API is `loads`/`load` (parse), `dumps`/`dump` (serialize), `to_json` (JSON output), and the `loads_all`/`load_all` multi-document variants. `async_loads`/`async_load`/`async_load_all` run the parse off the event loop.",
            "- `loads` takes and `dumps` returns `bytes` (a `str` is also accepted on input). There is no `yamlrocks.load` that executes code; the safe behavior is the only behavior.",
            "- Behavior is controlled by `option=` flags combined with `|`, such as `OPT_ROUND_TRIP`, `OPT_INCLUDES`, `OPT_SECRETS`, `OPT_ENV_VAR`, `OPT_ANNOTATED`, `OPT_YAML_1_1`, `OPT_SORT_KEYS`, and the `OPT_INDENT_2`/`OPT_INDENT_4` pair.",
            "- Round-trip mode returns a `YAMLRocksDocument`: edit it like a `dict`/`list`, then `to_yaml()` re-emits with comments and layout intact, byte-for-byte for an unmodified document.",
            "- For a near drop-in migration from PyYAML, `import yamlrocks.compat as yaml` exposes `safe_load`/`safe_dump` and friends returning `str` like PyYAML.",
          ].join("\n"),
          customSets: [
            {
              label: "Guides and recipes",
              paths: ["guides/**", "recipes/**"],
              description:
                "Task-oriented documentation: loading, dumping, JSON, round-trip editing, includes, annotated mode, schema validation, custom tags, and worked recipes.",
            },
            {
              label: "API reference",
              paths: ["reference/**", "getting-started/**"],
              description:
                "The full function and option reference, exceptions, the security model, and migration guides.",
            },
          ],
        }),
      ],
      head: [
        {
          tag: "meta",
          attrs: {
            property: "og:image",
            content: "https://yaml.rocks/social.png",
          },
        },
        {
          tag: "meta",
          attrs: {
            property: "og:image:width",
            content: "640",
          },
        },
        {
          tag: "meta",
          attrs: {
            property: "og:image:height",
            content: "320",
          },
        },
        {
          tag: "meta",
          attrs: {
            property: "og:image:alt",
            content: "YAMLRocks - Rock-solid YAML for Python, written in Rust.",
          },
        },
        {
          tag: "meta",
          attrs: {
            name: "twitter:image",
            content: "https://yaml.rocks/social.png",
          },
        },
        {
          tag: "meta",
          attrs: {
            name: "twitter:image:alt",
            content: "YAMLRocks - Rock-solid YAML for Python, written in Rust.",
          },
        },
      ],
      customCss: ["@fontsource-variable/inter", "./src/styles/custom.css"],
      editLink: {
        baseUrl: "https://github.com/frenck/yamlrocks/edit/main/docs/",
      },
      components: {
        Footer: "./src/components/Footer.astro",
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/frenck/yamlrocks",
        },
      ],
      sidebar: [
        {
          label: "Getting started",
          items: [
            { label: "Introduction", slug: "index" },
            { label: "Installation", slug: "getting-started/installation" },
            { label: "Quick start", slug: "getting-started/quick-start" },
            {
              label: "Migrating from PyYAML",
              slug: "getting-started/migrating-from-pyyaml",
            },
            {
              label: "Migrating from ruamel.yaml",
              slug: "getting-started/migrating-from-ruamel",
            },
            {
              label: "Migration compatibility",
              slug: "getting-started/compatibility",
            },
          ],
        },
        {
          label: "Guides",
          items: [
            { label: "Loading YAML", slug: "guides/loading" },
            { label: "Dumping YAML", slug: "guides/dumping" },
            { label: "JSON import and export", slug: "guides/json" },
            { label: "YAML 1.1 vs 1.2", slug: "guides/yaml-11-vs-12" },
            { label: "Round-trip editing", slug: "guides/round-trip" },
            { label: "Includes", slug: "guides/includes" },
            { label: "Annotated mode", slug: "guides/annotated" },
            { label: "Schema validation", slug: "guides/schema-validation" },
            { label: "Custom tags", slug: "guides/tags" },
          ],
        },
        {
          label: "Comparisons",
          items: [
            { label: "How YAMLRocks compares", slug: "comparisons" },
            { label: "Performance", slug: "guides/performance" },
            { label: "vs PyYAML", slug: "comparisons/vs-pyyaml" },
            { label: "vs ruamel.yaml", slug: "comparisons/vs-ruamel" },
            { label: "vs yaml-rs", slug: "comparisons/vs-yaml-rs" },
            { label: "vs fast-yaml", slug: "comparisons/vs-fast-yaml" },
            { label: "vs ryaml", slug: "comparisons/vs-ryaml" },
            { label: "vs py-yaml12", slug: "comparisons/vs-py-yaml12" },
            { label: "vs yamlium", slug: "comparisons/vs-yamlium" },
            { label: "vs strictyaml", slug: "comparisons/vs-strictyaml" },
            { label: "vs oyaml", slug: "comparisons/vs-oyaml" },
          ],
        },
        {
          label: "Recipes",
          items: [
            { label: "Home Assistant", slug: "recipes/home-assistant" },
            { label: "Config editor", slug: "recipes/config-editor" },
            { label: "YAML and JSON", slug: "recipes/yaml-to-json" },
          ],
        },
        {
          label: "Reference",
          items: [
            { label: "API", slug: "reference/api" },
            { label: "Options", slug: "reference/options" },
            { label: "Exceptions", slug: "reference/exceptions" },
            { label: "YAML style guide", slug: "guides/yaml-style" },
          ],
        },
        {
          label: "Project",
          items: [
            { label: "About", slug: "about" },
            { label: "Stability and roadmap", slug: "stability-roadmap" },
            { label: "Projects using YAMLRocks", slug: "projects" },
            {
              label: "Real-world verification",
              slug: "verification/real-world-corpus",
            },
            { label: "Credits", slug: "credits" },
            { label: "Architecture", slug: "contributing/architecture" },
            { label: "Security", slug: "reference/security" },
            { label: "Contributing", slug: "contributing" },
            { label: "License", slug: "license" },
          ],
        },
      ],
    }),
  ],
});
