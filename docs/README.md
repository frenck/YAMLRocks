# YAMLRocks documentation

The YAMLRocks documentation site, built with [Astro
Starlight](https://starlight.astro.build/).

## Local development

```bash
cd docs
npm install
npm run dev      # serve at http://localhost:4321
npm run build    # build static site to ./dist
```

## Structure

```
docs/
├── astro.config.mjs              # site + sidebar configuration
├── src/
│   ├── content.config.ts         # Starlight content collection
│   └── content/docs/
│       ├── index.mdx             # landing page
│       ├── getting-started/      # installation, quick-start, migrating
│       ├── guides/               # loading, dumping, round-trip, includes, …
│       └── reference/            # api, options, exceptions
```

Every feature page includes copy-pasteable examples that match the behavior in
the test suite.
