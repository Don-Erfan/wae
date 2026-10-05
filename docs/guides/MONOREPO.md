# Monorepo guide

WAE keeps two graphs: a **module graph** (files and imports) and a **package graph** (workspace
packages and the dependencies between them). Module rules (`ARCH-*`, `RUNTIME-*`) and package rules
(`PACKAGE-*`) run on the same analysis, so one `wae check` at the repository root covers every app
and library.

Tested layouts: npm and Yarn `workspaces`, `pnpm-workspace.yaml`, Turborepo and Nx-style
`apps/` + `libs/` + `packages/` trees, including a 12-package workspace fixture.

## Setup

Run WAE from the repository root, next to the root `package.json`:

```bash
npx wae discover        # detects workspaces, Nx/Turborepo, frameworks per package
npx wae check
```

Workspace packages are discovered from the declarations only (no guessing from folder names).
Imports of a workspace package by name resolve through that package's `exports`/`imports` (or
legacy `main`/`module`/`types`), exactly like Node and TypeScript do, and TypeScript path aliases
are taken from the nearest `tsconfig.json`/`jsconfig.json` of each importing file.

Framework detection is per package: a Next.js app gets client/server/edge analysis, while a plain
library next to it does not.

## Package rules

| Rule | Catches |
|---|---|
| `PACKAGE-001` | a cycle between workspace packages |
| `PACKAGE-002` | a package dependency forbidden by your policy (for example `ui` → `app`) |
| `PACKAGE-003` | importing a workspace package that is not declared in `package.json` |
| `PACKAGE-004` | a relative import (`../../packages/ui/src/x`) that crosses into another package |

```yaml
version: 1
architecture:
  forbidden_package_dependencies:
    - from: "@acme/ui-*"
      to: "@acme/app-*"
    - from: "@acme/*"
      to: "@acme/admin"
  presets:
    monorepo_boundaries: true     # also forbid any packages/** module importing apps/**
```

## Layers across packages

Layer patterns are project-relative globs, so they can span packages:

```yaml
architecture:
  layers:
    app:      { patterns: ["apps/*/src/app/**"],      canImport: [features, packages] }
    features: { patterns: ["apps/*/src/features/**"], canImport: [packages] }
    packages: { patterns: ["packages/*/src/**"],      canImport: [packages] }
  features:
    roots: ["src/features"]       # relative to each package root
```

A module matching two layers is a configuration error. Check ownership with
`npx wae config validate --show-overlaps --show-coverage --show-unassigned`.

## Per-package rollout

Use `overrides` to roll a rule out package by package:

```yaml
overrides:
  - files: ["apps/legacy/**"]
    rules:
      ARCH-003: warning
      PACKAGE-004: off
```

Or adopt with a baseline at the root and `wae check --changed` in CI so only new violations fail.

## Performance

Analysis is incremental: unchanged modules are restored from `.wae/cache` and only changed files
and their importers are re-analyzed. CI gates keep 10,000 and 50,000-module workspaces within
fixed latency and memory budgets (see [Performance](../PERFORMANCE.md)). Add `.wae/cache/` to
`.gitignore`; commit `wae.yaml` and `.wae/baseline.json`.
