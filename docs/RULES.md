# WAE rule reference

## ARCH-001

Detects circular dependencies in the analyzed module graph.

## ARCH-002

Enforces configured forbidden-dependency policies and optional architecture presets.

## ARCH-003

Enforces configured layer import permissions. A module matching multiple layers is a configuration error.

## ARCH-004

Requires every importer outside a feature's owning package/feature to use that feature's public entrypoint.

## ARCH-005

Rejects imports of explicitly private path segments from outside their owning package/feature.

## ARCH-006

Limits shortest transitive dependency depth from explicitly configured `entrypoints`. Configure
`max_depth`; without a threshold the rule remains enabled but intentionally emits nothing.

## ARCH-007 / ARCH-008

Limit direct unique outgoing (`max_fan_out`) and incoming (`max_fan_in`) module coupling. External
and excluded nodes remain visible in the graph, so the reported count matches the analyzed model.

## ARCH-009

Reports source modules that cannot be reached from any configured `entrypoints`. The rule requires
at least one explicit entrypoint to avoid guessing framework roots.

## ARCH-010

When layers are configured, reports every discovered source module not owned by exactly one layer.
Multiple ownership remains a configuration error; zero ownership is this diagnostic. Paths matched
by `architecture.coverage.allow_unassigned` are explicitly exempt and never reported by this rule.

## ARCH-011

Enforces `architecture.coverage.minimum` during every normal analysis. This aggregate rule reports
the actual and required percentages plus assigned, exempt and unassigned counts. Keep `ARCH-010`
enabled for strict per-module ownership, or disable it when a threshold-only adoption policy is
intentional.

## PACKAGE-001

Detects strongly connected components in the workspace package graph and reports one stable cycle
path per component.

## PACKAGE-002

Enforces `architecture.forbidden_package_dependencies` using package-name glob pairs.

## PACKAGE-003

Requires cross-workspace imports to appear in the importing package's `dependencies`,
`devDependencies`, `peerDependencies`, or `optionalDependencies` manifest section.

## PACKAGE-004

Rejects relative imports whose resolved source and target belong to different workspace packages;
consumers must use the target package name and its resolver-enforced public entrypoint.

## RUNTIME-001 / RUNTIME-002

Reject the shortest transitive path from a browser-classified module to server-only or Node-only
code. Next.js Server Action modules are explicit RPC boundaries: their implementation closure is
not treated as part of a browser caller's bundle.

## RUNTIME-003

Rejects a browser dependency path ending in a package matching
`runtime.browser_incompatible_packages`. Package policies use validated glob syntax and diagnostics
identify both the package and the complete shortest path.

## RUNTIME-004

Rejects an Edge dependency path ending in a Node-classified module or a package matching
`runtime.edge_incompatible_packages`.

## RUNTIME-005

Reports a universal module whose transitive closure requires both browser and server/Node
capabilities. The diagnostic includes separate browser and server path evidence so the module can
be split at the correct boundary.

## RUNTIME-006

Reports dependency cycles containing incompatible runtime domains: browser with server/Node, or
Edge with Node. RPC-boundary cycles are excluded from the runtime graph.

## Metric configuration example

```yaml
architecture:
  forbidden_package_dependencies:
    - from: "@acme/ui-*"
      to: "@acme/app-*"

runtime:
  browser_incompatible_packages: ["node:*", "@acme/server-*"]
  edge_incompatible_packages: ["node:*", "*-native"]

rules:
  ARCH-006:
    severity: warning
    max_depth: 8
    entrypoints: ["src/app/**/page.tsx", "src/pages/**/*.tsx"]
  ARCH-007:
    severity: warning
    max_fan_out: 20
  ARCH-008:
    severity: warning
    max_fan_in: 50
  ARCH-009:
    severity: warning
    entrypoints: ["src/app/**/page.tsx", "src/pages/**/*.tsx"]
```
