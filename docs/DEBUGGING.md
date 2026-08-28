# Observability and resolution debugging

`wae check --verbose` keeps the selected reporter on stdout and writes operational details to
stderr, so JSON/SARIF pipelines remain valid. The report separates discovery, module
parse/resolution, graph construction and rules, followed by total latency and cache hit counts.

```bash
wae check --verbose
wae check --format json --no-cache --verbose > result.json
```

Use `resolve` to explain one import without running all rules:

```bash
wae resolve src/app/page.tsx '@/features/cart'
wae resolve src/server.ts '#internal/db' --kind require
wae resolve src/types.ts '@acme/contracts' --kind type --config architecture.yml
```

The JSON response records importer format, resolution mode, import/require branch, active package
conditions, every candidate path, each Chain-of-Responsibility handler attempt (including misses
and redirects), and the normalized final outcome. Paths inside the project are project-relative.
An importer outside the project root is rejected.
