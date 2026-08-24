# WAE configuration

`wae.yaml` is a versioned, strict schema. Unknown keys and invalid glob patterns are errors.

```yaml
version: 1

project:
  include: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"]
  exclude: ["**/*.d.ts", "**/*.d.mts", "**/*.d.cts", "**/*.test.*", "**/node_modules/**", "**/.next/**", "**/dist/**"]
  roots: ["."]

resolution:
  mode: nodenext
  custom_conditions: []

architecture:
  layers:
    app:
      patterns: ["**/app/**"]
      canImport: ["features", "entities", "shared"]
  features:
    roots: ["src/features"]
    public_entrypoints: ["index.ts", "index.tsx", "index.mts", "index.cts", "index.js", "index.jsx", "index.mjs", "index.cjs"]

rules:
  ARCH-001: error
  ARCH-004: warning

suppressions:
  require_reason: true
  report_unused: true

baseline:
  file: .wae/baseline.json
```

## Resolution

The resolver selects the nearest ancestor `tsconfig.json` for each importer. It supports JSONC,
`extends`, `baseUrl`, `paths`, workspace manifests, package `exports`/`imports`, and conditional
targets. In Node16/NodeNext mode, the importer extension and nearest package `type` determine
whether a static edge uses the `import` or `require` condition. Dynamic imports always use
`import`, and `require()` always uses `require`. Type-only imports activate `types` together with
their underlying `import`/`require` condition, including nested conditional-export objects.
`node` or `browser` is selected by `resolution.mode`; additional explicit conditions belong in
`custom_conditions`.

Package format and workspace identity use separate indexes. `PackageScopeIndex` records every
`package.json` under the project—including private manifests without `name` and nested manifests
that are not workspaces—and chooses the nearest ancestor for `type`. `WorkspacePackageIndex`
contains only named, declared workspace packages and owns package-name/exports/imports resolution.
Consequently, `name` never affects ESM/CommonJS classification.

Bundler mode activates `import` or `require`, `types` for type-only edges, `default`, and configured
custom conditions. It does **not** activate `browser` implicitly; add `browser` to
`resolution.custom_conditions` only when that matches the project's bundler profile.

Node16/NodeNext ESM relative imports require an explicit runtime extension (for example,
`./user.js`, which may map to `user.ts`). CommonJS and Bundler resolution retain extension and
directory-index probing.

Package-based `extends` values are searched through `node_modules` in the tsconfig directory and
each ancestor, so hoisted configurations work in monorepos. WAE currently accepts exactly one
string-valued `extends`; TypeScript's newer `extends` arrays are rejected with an explicit config
error. Project references, `rootDirs`, `moduleSuffixes`, Yarn Plug'n'Play and arbitrary TypeScript
plugins are not yet interpreted by the resolver.

Declaration files (`.d.ts`, `.d.mts`, `.d.cts`) are excluded from default source discovery. They
can still be resolved as type targets, and projects that intentionally analyze declarations may
override `project.exclude`.

Type-only dependency classification covers `import type`, `export type`, and named clauses whose
specifiers are all marked `type`. Mixed clauses remain runtime `Static`/`ReExport` edges.

Absolute module specifiers are rejected because they escape the project analysis boundary.
Duplicate workspace package names are configuration errors.

## Source suppressions

A suppression is a standalone `//` comment, is rule-scoped, applies to its own line or the
following line, and should explain why the exception exists. One directive suppresses all matching
diagnostics in that line scope; text inside strings, templates and block comments is ignored:

```ts
// wae-ignore ARCH-003 -- legacy adapter; remove after ARC-142
import { legacyClient } from "../legacy/client";
```

Suppressed diagnostics stay visible in human, JSON, JSONL, and SARIF output, but do not fail the
check. SARIF records them as accepted in-source suppressions. Missing reasons, unknown rule IDs,
and unused directives produce `SUPPRESS-001` warnings when the corresponding options are enabled.

## Baselines

`wae baseline create` records only unsuppressed error/warning diagnostics. Informational and
suppressed diagnostics remain visible in reports but are excluded from the ratchet file; the
command prints the recorded and excluded counts.

## Cache concurrency

The cache repository uses an operating-system advisory lock held across read, merge, atomic write,
and rename. The `.lock` path is a persistent coordination inode, not an ownership marker: its mere
existence is harmless, and the OS releases ownership automatically when a process crashes or is
killed. Cache payloads are also invalidated by parser behavior version.

## Changed mode

Create the baseline explicitly and commit it:

```bash
wae baseline create
wae check --changed --base origin/main
```

Changed mode evaluates changed/deleted files plus their transitive importer closure. Resolver
candidate hints ensure that deleting a relative, TypeScript-alias, or workspace-export target also
marks its importer as affected.
