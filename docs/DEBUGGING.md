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

## Explain a module

```bash
wae graph --module src/features/cart/ui.tsx
wae graph --module src/features/cart/ui.tsx --format json
```

The report lists the module's package, layer, feature, runtime and the evidence for that runtime
(`directive`, `marker-package`, `explicit`, `convention`, `propagated`, `default`, `builtin`). For a
browser module that inherited its runtime, `via` shows the import path from the declaring
`"use client"` file. It also lists every dependency and dependent with the import location and
every diagnostic that involves the module, including its dependency path. Editors render the same
projection on hover and through the `wae.inspectModule` server command.

## Explain a rule

```bash
wae rules
wae explain RUNTIME-001
```

`explain` prints why the rule exists, a violating and a passing example, how to fix it, the
configuration that enables or tunes it and its known false positives. The same text is the
[rule reference](RULES.md), the MCP `architecture_explain` result and the SARIF rule help.

## Bug report checklist

1. `wae --version` and the operating system.
2. `wae doctor` output.
3. For a wrong diagnostic: `wae check --format json`, `wae graph --module <file> --format json`
   and `wae resolve <importer> <specifier>` for the import involved.
4. For slowness: `wae check --verbose --no-cache` (phase and per-rule timings).
