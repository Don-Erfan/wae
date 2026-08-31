# WAE configuration

`wae.yaml` is a versioned, strict schema. Unknown keys and invalid glob patterns are errors.

For YAML autocomplete and inline validation, add this first line (or associate the schema in the
editor's YAML settings):

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/Don-Erfan/wae/master/schemas/wae.schema.json
```

The bundled schema is registry-synchronized by a test, so every configurable rule ID appears in
completion and removed/unknown keys are rejected before WAE runs.

Use an alternate file for one invocation with `wae check --config path/to/architecture.yml`.
`--no-cache` disables reads and writes without mutating that file.

```yaml
version: 1

project:
  include: ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"]
  exclude: ["**/*.d.ts", "**/*.d.mts", "**/*.d.cts", "**/*.test.*", "**/node_modules/**", "**/.next/**", "**/dist/**"]
  roots: ["."]

resolution:
  mode: nodenext
  custom_conditions: []

framework:
  auto_detect: true
  enabled: []

runtime:
  browser_incompatible_packages: ["node:*", "@acme/server-*"]
  edge_incompatible_packages: ["node:*", "*-native"]

architecture:
  coverage:
    minimum: 90
    allow_unassigned: ["scripts/**", "generated/**"]
  layers:
    app:
      patterns: ["src/app/**"]
      canImport: ["features", "entities", "shared"]
  forbidden_package_dependencies:
    - from: "@acme/shared-*"
      to: "@acme/app-*"
  features:
    roots: ["src/features"]
    public_entrypoints: ["index.ts", "index.tsx", "index.mts", "index.cts", "index.js", "index.jsx", "index.mjs", "index.cjs"]

rules:
  ARCH-001: error
  ARCH-004: warning
  ARCH-006:
    severity: warning
    max_depth: 8
    entrypoints: ["src/app/**/page.tsx"]

suppressions:
  require_reason: true
  report_unused: true

baseline:
  file: .wae/baseline.json
```

## Resolution

Supported modes are `node10`, `node16`, `nodenext`, and `bundler`. The legacy spelling `node` is
accepted as an input alias for `node10`, but generated configuration always uses the explicit
name. Node10 intentionally ignores package `exports`/`imports` and falls back to legacy package
entrypoints. Modern modes prefer `exports`; when it is absent, runtime imports use `module` then
`main`, while type-only imports prefer `types`/`typings`. Legacy values are package-relative paths
and do not need a `./` prefix.

The resolver selects the nearest ancestor `tsconfig.json` for each importer. It supports JSONC,
`extends`, `baseUrl`, `paths`, workspace manifests, package `exports`/`imports`, and conditional
targets. In Node16/NodeNext mode, the importer extension and nearest package `type` determine
whether a static edge uses the `import` or `require` condition. Dynamic imports always use
`import`, and `require()` always uses `require`. Type-only imports activate `types` together with
their underlying `import`/`require` condition, including nested conditional-export objects.
`node` or `browser` is selected by `resolution.mode`; additional explicit conditions belong in
`custom_conditions`.

Package format and workspace identity use separate indexes. `PackageScopeIndex` visits only the
ancestor directories of discovered source importers, records private or unnamed boundaries, and
chooses the nearest ancestor for `type`. Excluded build output is never crawled merely to classify
an importer. `WorkspacePackageIndex` contains only named, declared workspace packages and owns
package-name/exports/imports resolution. Consequently, `name` never affects ESM/CommonJS
classification.

Bundler mode derives `import` or `require` from source syntax, never from the nearest package
`type`. Static imports, re-exports, dynamic imports and type imports use the import branch;
`require()` and TypeScript `import x = require("x")` use the require branch. It also activates
`types` for type-only edges, `default`, and configured custom conditions. It does **not** activate `browser` implicitly; add `browser` to
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

## Framework adapters

`framework.auto_detect` is enabled by default. Next.js is selected only when `next` appears in a
root dependency section or a `next.config.js|mjs|cjs|ts` file exists; directory names alone are not
authoritative evidence. To disable all adapters use `auto_detect: false` with an empty `enabled`
list. To force the adapter for a nonstandard project use:

```yaml
framework:
  auto_detect: false
  enabled: [nextjs]
```

The parser emits framework-neutral `ModuleSemantics` from the JS/TS AST, including directive
prologues and literal runtime exports. The Next.js adapter consumes those facts to classify App
Router pages/layouts/loading/error/not-found/templates, route handlers, client/server directives,
middleware, Pages Router pages/API routes and custom `_app`/`_document`/`_error` files. It also
records explicit `edge`/`nodejs` runtime exports without regex rescanning source text.
Classification is stored as open `FrameworkMetadata`, so core and rule APIs do not depend on a
Next-specific enum.

## Runtime graph

`RuntimeGraph` propagates browser, server, Node and Edge requirements over the resolved module
graph and retains a deterministic shortest path for every requirement. `RUNTIME-001` through
`RUNTIME-006` consume this shared projection rather than repeating traversal per parser or
framework. Explicit runtime exports are classified before graph construction. Next.js Server
Actions terminate propagation because importing an action creates an RPC reference, not a client
bundle dependency on its server implementation.

The incompatible package lists are opt-in glob policies. Keep them specific to dependencies known
to require unavailable platform APIs; an empty list produces no package-compatibility diagnostic.

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

Fingerprint identity is structural: rule ID, canonical source/target and stable file identity.
Messages, severity, suggestions, line/column movement and `related_rules` presentation metadata do
not change it. The baseline reader accepts the old `0.0.10` and `0.0.11` fingerprint forms as
migration aliases, so existing committed baselines remain valid after upgrading to `0.0.12`.

## Safe initialization and ownership validation

`wae init` is equivalent to `--preset blank` and produces no inferred layers. The opt-in presets
use repository-anchored patterns:

```bash
wae init --preset blank
wae init --preset fsd
wae init --preset next
wae init --preset nx
wae config validate --show-overlaps
wae config validate --show-coverage --show-unassigned
```

The validation command lists every source file matching multiple layers, calculates layer coverage,
enforces the optional minimum and can list every non-exempt unassigned module. A non-empty project
with no configured layers emits an explicit warning. `wae doctor` includes the same root cause;
Git is advisory because only `check --changed` requires it.

For an existing repository, `wae discover` produces a read-only proposal with explicit evidence,
confidence, detected config files and feature clusters. It recognizes authoritative Next.js
dependencies/configs, `nx.json`, `turbo.json`, FSD directory segments, `tsconfig.json` and
`jsconfig.json`. No config is written until approval:

```bash
wae discover
wae discover --json > architecture-proposal.json
wae discover --write
# Overwriting an existing file requires both flags:
wae discover --write --force
```

The resolver selects `tsconfig.json` over `jsconfig.json` when both exist in one directory; otherwise
either file supplies the nearest configured-project `baseUrl` and `paths` aliases.

## Cache concurrency

`analysis-v3/` stores module-level parse results in content-addressed JSON shards and keeps rule
state in a small atomic manifest. A resolution-environment fingerprint covers configuration,
workspace manifests, ts/jsconfig files and framework configs. Unchanged modules restore those
fragments without parsing or resolving again; deleted targets and newly satisfiable unresolved
candidates invalidate the owning module. A graph-identity snapshot also reuses rule diagnostics
when the complete semantic graph is unchanged. Any graph change reevaluates rules for correctness.

Analysis reads an unlocked cache snapshot. Saving takes an operating-system advisory lock only for
the short dirty-shard merge, prune, manifest update and atomic replace transaction. A single-module
edit no longer serializes one project-wide JSON payload. Only entries produced by the current
analysis are merged, so a stale process cannot overwrite a newer module entry. Unix rename and
Windows `MoveFileExW` provide platform-native atomic replacement.
The `.lock` path is a persistent coordination inode, not an ownership marker: its mere existence is
harmless, and the OS releases ownership automatically after a crash. Payloads are also invalidated
by parser behavior version.

## Changed mode

Create the baseline explicitly and commit it:

```bash
wae baseline create
wae check --changed --base origin/main
```

Changed mode evaluates changed/deleted files plus their transitive importer closure. Resolver
candidate hints ensure that deleting a relative, TypeScript-alias, or workspace-export target also
marks its importer as affected. Human, JSON, JSONL and SARIF outputs include a regression summary
with affected-module, existing, introduced and fixed counts. Incremental cache fragments restore
unaffected modules, so the semantic project remains complete without reparsing/re-resolving the
entire source set.
