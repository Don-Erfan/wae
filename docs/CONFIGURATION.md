# WAE configuration

`wae.yaml` is a versioned, strict schema. Unknown keys and invalid glob patterns are errors.

```yaml
version: 1

project:
  include: ["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx"]
  exclude: ["**/*.test.*", "**/node_modules/**", "**/.next/**", "**/dist/**"]
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
    public_entrypoints: ["index.ts", "index.tsx"]

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
targets. Conditions are selected from the dependency kind: static/dynamic imports and re-exports
use `import`, `require()` uses `require`, and type-only imports use `types`. `node` or `browser` is
then selected by `resolution.mode`; additional explicit conditions belong in `custom_conditions`.

Absolute module specifiers are rejected because they escape the project analysis boundary.
Duplicate workspace package names are configuration errors.

## Source suppressions

A suppression is rule-scoped, applies to its own line or the following line, and should explain why
the exception exists:

```ts
// wae-ignore ARCH-003 -- legacy adapter; remove after ARC-142
import { legacyClient } from "../legacy/client";
```

Suppressed diagnostics stay visible in human, JSON, JSONL, and SARIF output, but do not fail the
check. SARIF records them as accepted in-source suppressions. Missing reasons, unknown rule IDs,
and unused directives produce `SUPPRESS-001` warnings when the corresponding options are enabled.

## Changed mode

Create the baseline explicitly and commit it:

```bash
wae baseline create
wae check --changed --base origin/main
```

Changed mode evaluates changed/deleted files plus their transitive importer closure. Resolver
candidate hints ensure that deleting a relative, TypeScript-alias, or workspace-export target also
marks its importer as affected.
