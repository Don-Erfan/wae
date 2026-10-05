<!-- Generated from crates/core/src/rule_docs.rs. Do not edit by hand; run
     `WAE_UPDATE_DOCS=1 cargo test -p wae-core rules_reference_is_generated` to refresh. -->

# WAE rule reference

Every rule has a stable ID that never changes meaning within a major version. `wae explain
<RULE_ID>` prints the same text in the terminal, and editors show it from the language server.

Without a `wae.yaml`, only `ARCH-001` and the correctness diagnostics run. Once a configuration
exists every configurable rule defaults to `error`; set a rule to `warning`, `info` or `off`, or
use `overrides` for path-specific rollout. Suppress an accepted exception with a documented
reason (`// wae-ignore RULE-ID -- reason`) rather than disabling the rule.

| Rule | Title | Category |
|---|---|---|
| [ARCH-001](#arch-001) | Circular dependency | dependency-graph |
| [ARCH-002](#arch-002) | Forbidden dependency | architecture |
| [ARCH-003](#arch-003) | Layer boundary | architecture |
| [ARCH-004](#arch-004) | Feature boundary | architecture |
| [ARCH-005](#arch-005) | Private import | architecture |
| [ARCH-006](#arch-006) | Dependency depth | maintainability |
| [ARCH-007](#arch-007) | Outgoing coupling | maintainability |
| [ARCH-008](#arch-008) | Incoming coupling | maintainability |
| [ARCH-009](#arch-009) | Orphan module | maintainability |
| [ARCH-010](#arch-010) | Unassigned architecture module | architecture |
| [ARCH-011](#arch-011) | Architecture coverage threshold | architecture |
| [PACKAGE-001](#package-001) | Package cycle | package-graph |
| [PACKAGE-002](#package-002) | Forbidden package dependency | package-graph |
| [PACKAGE-003](#package-003) | Undeclared workspace dependency | package-graph |
| [PACKAGE-004](#package-004) | Cross-package relative import | package-graph |
| [RUNTIME-001](#runtime-001) | Browser to server dependency | runtime-graph |
| [RUNTIME-002](#runtime-002) | Browser to Node dependency | runtime-graph |
| [RUNTIME-003](#runtime-003) | Browser-incompatible package | runtime-graph |
| [RUNTIME-004](#runtime-004) | Edge-incompatible dependency | runtime-graph |
| [RUNTIME-005](#runtime-005) | Ambiguous universal runtime | runtime-graph |
| [RUNTIME-006](#runtime-006) | Incompatible runtime cycle | runtime-graph |
| [PARSE-001](#parse-001) | Parse failure | correctness |
| [RESOLVE-001](#resolve-001) | Unresolved import | correctness |
| [RESOLVE-002](#resolve-002) | Invalid module specifier | correctness |
| [SUPPRESS-001](#suppress-001) | Unused suppression | maintainability |

## ARCH-001

**Circular dependency** — Detects strongly connected module components.

- Category: `dependency-graph`
- Default: error; enabled even without a wae.yaml

### Why

A dependency cycle means no module in it can be understood, tested, reused or extracted on its own.
In ES modules and CommonJS, cycles also cause initialization-order bugs: a binding may still be
`undefined` (or in its temporal dead zone) when the other module reads it. Cycles tend to grow
because every new edge inside the strongly connected component is "free". WAE reports one
diagnostic per strongly connected component with a closed, shortest representative path.

### Bad

```ts
// src/cart/cart.ts
import { formatPrice } from "../pricing/price";
export const total = (items: Item[]) => items.map(formatPrice);

// src/pricing/price.ts
import { total } from "../cart/cart"; // closes the cycle
export const formatPrice = (item: Item) => `${item.price} of ${total([item])}`;
```

### Good

```ts
// src/pricing/price.ts — depends on nothing in cart
export const formatPrice = (item: Item) => `${item.price}`;

// src/cart/cart.ts
import { formatPrice } from "../pricing/price";
export const total = (items: Item[]) => items.map(formatPrice);
```

### How to fix

Read the reported path and find the edge that points "upward" (from a lower-level module to a
higher-level one). Remove it by extracting the shared code into a new lower-level module, by
passing the dependency in as a parameter, or by introducing a small interface (port) owned by
the lower module. `wae graph --module <file>` lists both directions for any module on the path.

### Configuration

```yaml
rules:
  ARCH-001: error   # or warning / off
  # Also report cycles closed only by `import type` (design-time coupling):
  # ARCH-001:
  #   include_type_only: true
```

### False positives

By default a cycle closed only by `import type` / `export type` is not reported, because
TypeScript erases those imports and the cycle never executes; set `include_type_only: true` to
report design-time cycles too. Barrel files (`index.ts` re-exporting everything) often create
cycles that disappear when importers use the concrete module. Accept an intentional cycle with
a fingerprint suppression in `wae.yaml`.

## ARCH-002

**Forbidden dependency** — Enforces configured dependency policies and architecture presets.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

Some dependencies are wrong regardless of layers: UI importing a database client, a shared
library importing an application, test utilities leaking into production code. `ARCH-002`
turns those decisions into explicit, reviewable policy instead of tribal knowledge.

### Bad

```ts
// src/components/user-card.tsx
import { db } from "../server/db"; // forbidden: components -> server
```

### Good

```ts
// src/components/user-card.tsx
import type { User } from "../entities/user";
export function UserCard({ user }: { user: User }) { /* ... */ }
```

### How to fix

Route the dependency through an allowed module (for example a Server Action, API client or
application service), or, if the policy is outdated, change the policy in `wae.yaml` in the same
reviewed pull request.

### Configuration

```yaml
architecture:
  forbidden_dependencies:
    - from: "src/components/**"
      to: "src/server/**"
  presets:
    monorepo_boundaries: true   # also forbid packages/** -> apps/**
```

### False positives

Globs match project-relative module paths with literal `/` separators, so `src/*` does not
match nested files; use `src/**`. The rule never fires without a configured policy or preset.

## ARCH-003

**Layer boundary** — Enforces configured layer import direction while allowing same-layer imports.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

Layered architectures (for example app → features → entities → shared) only stay layered if
the import direction is enforced. Without it, lower layers start importing higher ones and the
layers collapse into one tangled module.

### Bad

```ts
# wae.yaml: shared may not import features
// src/shared/ui/button.tsx
import { useCart } from "../../features/cart"; // shared -> features
```

### Good

```ts
// src/features/cart/ui/add-button.tsx
import { Button } from "../../../shared/ui/button"; // features -> shared is allowed
import { useCart } from "../model";
```

### How to fix

Move the code to the layer that is allowed to depend on both sides, invert the dependency with
a callback or prop, or add the target layer to `canImport` if the architecture genuinely
changed. Imports within the same layer are always allowed.

### Configuration

```yaml
architecture:
  layers:
    app:      { patterns: ["src/app/**"],      canImport: [features, entities, shared] }
    features: { patterns: ["src/features/**"], canImport: [entities, shared] }
    entities: { patterns: ["src/entities/**"], canImport: [shared] }
    shared:   { patterns: ["src/shared/**"],   canImport: [] }
```

### False positives

Modules outside every layer are ignored by this rule (see `ARCH-010`). A module matching two
layers is a configuration error, not a diagnostic; run `wae config validate --show-overlaps`
and anchor broad patterns such as `**/shared/**` to `src/shared/**`.

## ARCH-004

**Feature boundary** — Requires cross-feature dependencies to use public entrypoints.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

A feature's public entrypoint (`index.ts`) is its contract. Importing files behind it couples
the importer to implementation details, so the owning team cannot refactor without breaking
other features.

### Bad

```ts
// src/features/checkout/pay.ts
import { sessionStore } from "../user/model/session"; // reaches into feature `user`
```

### Good

```ts
// src/features/user/index.ts
export { getSession } from "./model/session";

// src/features/checkout/pay.ts
import { getSession } from "../user";
```

### How to fix

Export what the importer needs from the target feature's public entrypoint and import that
entrypoint instead. If the importer needs a lot of internals, the code probably belongs in the
target feature or in a lower shared layer.

### Configuration

```yaml
architecture:
  features:
    roots: ["src/features"]
    public_entrypoints: ["index.ts", "index.tsx"]
rules:
  ARCH-004: error
```

### False positives

Files inside the same feature may import each other freely. Code outside every feature (for
example `src/app`) is still an importer and must use the entrypoint. Add extra public files such
as `public-api.ts` to `public_entrypoints` instead of suppressing.

## ARCH-005

**Private import** — Prevents access to private modules from outside their owner.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

Directories named `internal` or `private` declare code that is not part of any contract.
`ARCH-005` makes that naming convention enforceable, even outside configured features.

### Bad

```ts
// src/features/checkout/pay.ts
import { tokenCache } from "../user/internal/token-cache";
```

### Good

```ts
// src/features/user/index.ts
export { refreshToken } from "./internal/token-cache";

// src/features/checkout/pay.ts
import { refreshToken } from "../user";
```

### How to fix

Import the owner's public entrypoint, or move the shared helper out of the private directory
into a module that is meant to be shared.

### Configuration

```yaml
architecture:
  features:
    private_segments: [internal, private]
rules:
  ARCH-005: error
```

### False positives

A path segment must equal a configured private segment exactly; `internals` or `private-api`
do not match. Imports from inside the owning feature are allowed.

## ARCH-006

**Dependency depth** — Limits transitive dependency depth from configured architecture entrypoints.

- Category: `maintainability`
- Default: error once a wae.yaml exists

### Why

Long dependency chains from an entrypoint (page, route, CLI) make start-up cost, bundle size
and change impact hard to reason about. A depth budget surfaces chains that keep growing.

### Bad

```ts
# max_depth: 3
// src/app/page.tsx -> features/a -> features/b -> entities/c -> shared/d (depth 4)
```

### Good

```ts
// src/app/page.tsx -> features/a -> shared/d (depth 2)
```

### How to fix

Flatten the chain: let the entrypoint compose features directly, remove pass-through modules,
or move deep helpers closer to their callers.

### Configuration

```yaml
rules:
  ARCH-006:
    severity: warning
    max_depth: 8
    entrypoints: ["src/app/**/page.tsx", "src/pages/**/*.tsx"]
```

### False positives

Depth is the shortest path from any configured entrypoint, so a module reachable through both
a short and a long chain is measured by the short one. Without `max_depth` the rule emits nothing.

## ARCH-007

**Outgoing coupling** — Limits the number of modules directly imported by one module.

- Category: `maintainability`
- Default: error once a wae.yaml exists

### Why

A module that imports many distinct modules (high fan-out) usually mixes responsibilities and
changes whenever any of its dependencies change.

### Bad

```ts
# max_fan_out: 3
// src/app/dashboard.tsx imports 12 different feature and entity modules
```

### Good

```ts
// src/app/dashboard.tsx imports one composed widget per area
import { SalesWidget } from "../features/sales";
import { UsersWidget } from "../features/users";
```

### How to fix

Split the module by responsibility or introduce a composition module per area.

### Configuration

```yaml
rules:
  ARCH-007:
    severity: warning
    max_fan_out: 20
```

### False positives

Multiple imports of the same module count once. Barrel and composition-root files are
expected to have high fan-out; exempt them with a path override set to `off`.

## ARCH-008

**Incoming coupling** — Limits the number of modules directly depending on one module.

- Category: `maintainability`
- Default: error once a wae.yaml exists

### Why

A module that very many modules depend on (high fan-in) is a change hotspot: every edit risks
breaking a large part of the codebase.

### Bad

```ts
# max_fan_in: 2
// src/utils/index.ts is imported by 140 modules and keeps growing
```

### Good

```ts
// small, focused modules: src/shared/lib/format-date.ts, src/shared/lib/money.ts
```

### How to fix

Split the hotspot into smaller cohesive modules, or keep it deliberately stable and exempt it.

### Configuration

```yaml
rules:
  ARCH-008:
    severity: warning
    max_fan_in: 50
```

### False positives

Stable foundations (design-system primitives, type definitions) legitimately have high fan-in;
exempt them with `overrides` rather than raising the global threshold.

## ARCH-009

**Orphan module** — Finds source modules unreachable from configured architecture entrypoints.

- Category: `maintainability`
- Default: error once a wae.yaml exists

### Why

Source modules that no entrypoint can reach are dead code or forgotten migrations. They still
cost review, type-checking and maintenance time.

### Bad

```ts
# entrypoints: ["src/app/**/page.tsx"]
// src/features/legacy-banner/index.ts — nothing imports it
```

### Good

```ts
// delete the module, or import it from a page/route that should use it
```

### How to fix

Delete unreachable code, or add the missing entrypoint (for example a script or route) to
`entrypoints`.

### Configuration

```yaml
rules:
  ARCH-009:
    severity: warning
    entrypoints: ["src/app/**/page.tsx", "src/pages/**/*.tsx", "scripts/*.ts"]
```

### False positives

Framework files loaded by convention (middleware, instrumentation, config files) and files
loaded through non-static mechanisms are unreachable unless listed as entrypoints. The rule never
guesses entrypoints and emits nothing until at least one is configured.

## ARCH-010

**Unassigned architecture module** — Requires every source module to belong to a configured architecture layer.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

Layer rules only protect modules that belong to a layer. `ARCH-010` makes sure new directories
do not silently escape the architecture.

### Bad

```ts
# layers cover src/app, src/features and src/shared
// src/helpers/date.ts — belongs to no layer
```

### Good

```ts
// src/shared/lib/date.ts — owned by the shared layer
```

### How to fix

Move the module under an existing layer, add a pattern to the right layer, or list the path in
`architecture.coverage.allow_unassigned` when it is intentionally outside the architecture.

### Configuration

```yaml
architecture:
  coverage:
    allow_unassigned: ["scripts/**", "generated/**"]
rules:
  ARCH-010: warning
```

### False positives

The rule is silent when no layers are configured. Use `wae config validate --show-coverage
--show-unassigned` to review the full list before enabling it as an error.

## ARCH-011

**Architecture coverage threshold** — Enforces the configured aggregate minimum for architecture layer ownership.

- Category: `architecture`
- Default: error once a wae.yaml exists

### Why

Strict per-module ownership (`ARCH-010`) can be too much on day one. `ARCH-011` enforces an
aggregate coverage floor so adoption can ratchet upward.

### Bad

```ts
# coverage.minimum: 90
# 70% of non-exempt source modules belong to exactly one layer
```

### Good

```ts
# 92% assigned: passes, and the minimum can be raised over time
```

### How to fix

Assign more modules to layers or exempt intentional areas; then raise `minimum`.

### Configuration

```yaml
architecture:
  coverage:
    minimum: 80
rules:
  ARCH-010: off      # threshold-only adoption
  ARCH-011: error
```

### False positives

Exempt (`allow_unassigned`) modules are excluded from the percentage. The rule emits nothing
without `coverage.minimum`.

## PACKAGE-001

**Package cycle** — Detects circular dependencies between workspace packages.

- Category: `package-graph`
- Default: error once a wae.yaml exists

### Why

A cycle between workspace packages makes independent versioning, building and publishing
impossible and usually breaks task-graph tools such as Turborepo and Nx.

### Bad

```ts
// packages/ui/src/button.tsx
import { track } from "@acme/analytics";
// packages/analytics/src/index.ts
import { Button } from "@acme/ui"; // @acme/ui <-> @acme/analytics
```

### Good

```ts
// move the shared contract into a lower package both depend on
import { type TrackEvent } from "@acme/contracts";
```

### How to fix

Move the shared contract into a lower-level package, or invert one dependency through a port
that the higher package implements.

### Configuration

```yaml
rules:
  PACKAGE-001: error
```

### False positives

Only declared workspace packages participate; external npm packages never form package cycles.

## PACKAGE-002

**Forbidden package dependency** — Enforces configured package-to-package dependency policies.

- Category: `package-graph`
- Default: error once a wae.yaml exists

### Why

Package direction policies (shared packages never import apps, UI never imports server
packages) keep a monorepo layered at the package level.

### Bad

```ts
# from: "@acme/ui-*" to: "@acme/app-*"
// packages/ui-kit/src/header.tsx
import { routes } from "@acme/app-web";
```

### Good

```ts
// pass routes in from the app instead
export function Header({ routes }: { routes: Route[] }) { /* ... */ }
```

### How to fix

Invert the dependency (props, callbacks, dependency injection) or move the code into a package
that is allowed to depend on both.

### Configuration

```yaml
architecture:
  forbidden_package_dependencies:
    - from: "@acme/ui-*"
      to: "@acme/app-*"
```

### False positives

Patterns match package names, not paths. The rule never fires without a configured policy.

## PACKAGE-003

**Undeclared workspace dependency** — Requires cross-workspace imports to be declared in the importer manifest.

- Category: `package-graph`
- Default: error once a wae.yaml exists

### Why

Importing a workspace package that is not declared in `package.json` works locally through
hoisting but breaks isolated installs, pruned Docker builds and publishing.

### Bad

```ts
// apps/web/src/page.tsx
import { Button } from "@acme/ui";
// apps/web/package.json has no "@acme/ui" entry
```

### Good

```ts
// apps/web/package.json
{ "dependencies": { "@acme/ui": "workspace:*" } }
```

### How to fix

Add the package to `dependencies`, `devDependencies`, `peerDependencies` or
`optionalDependencies` of the importing package.

### Configuration

```yaml
rules:
  PACKAGE-003: error
```

### False positives

Only cross-workspace imports are checked; third-party packages are left to your package
manager and tools such as `depcheck`.

## PACKAGE-004

**Cross-package relative import** — Prevents relative paths from bypassing workspace package entrypoints.

- Category: `package-graph`
- Default: error once a wae.yaml exists

### Why

A relative import such as `../../packages/ui/src/button` bypasses the target package's
`exports`, its build and its dependency declaration.

### Bad

```ts
// apps/web/src/page.tsx
import { Button } from "../../../packages/ui/src/button";
```

### Good

```ts
// apps/web/src/page.tsx
import { Button } from "@acme/ui";
```

### How to fix

Import the package by name and expose what you need through its `exports` entrypoints.

### Configuration

```yaml
rules:
  PACKAGE-004: error
```

### False positives

Relative imports within one package are always allowed.

## RUNTIME-001

**Browser to server dependency** — Prevents browser modules from reaching server-only modules transitively.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

Code that reaches the browser must never pull in server-only modules: it leaks secrets and
server logic into the client bundle or fails the build. The violation is often indirect, so WAE
reports the shortest transitive path.

### Bad

```ts
// src/app/profile/client.tsx
"use client";
import { getUser } from "../../lib/user"; // lib/user imports "server-only" db code
```

### Good

```ts
// src/app/profile/actions.ts
"use server";
export async function getUser() { /* db access */ }

// src/app/profile/client.tsx
"use client";
import { getUser } from "./actions"; // Server Action = explicit RPC boundary
```

### How to fix

Follow the reported path to the first server-only module and cut the edge before it: call the
server through a Server Action, a route handler or an API client, and keep shared modules free
of server imports.

### Configuration

```yaml
rules:
  RUNTIME-001: error
```

### False positives

Type-only imports are ignored. Modules are server-only through `import "server-only"`, Next.js
conventions or an explicit runtime export; `wae graph --module <file>` shows why a module got its
runtime. Server Action modules (`"use server"`) stop propagation by design.

## RUNTIME-002

**Browser to Node dependency** — Prevents browser modules from reaching Node-only modules transitively.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

Browser bundles cannot use Node APIs (`fs`, `child_process`, `node:*`). Reaching them through a
shared helper fails at build time or ships broken polyfills.

### Bad

```ts
"use client";
import { readConfig } from "../lib/config"; // lib/config imports "node:fs"
```

### Good

```ts
"use client";
import type { Config } from "../lib/config-types"; // read the file on the server and pass data down
```

### How to fix

Split the Node implementation from its browser-safe contract, and load the data on the server.

### Configuration

```yaml
rules:
  RUNTIME-002: error
```

### False positives

Node classification comes from Node builtins and explicit `runtime = "nodejs"` exports. Type-only
imports never count.

## RUNTIME-003

**Browser-incompatible package** — Prevents browser modules from reaching configured incompatible packages.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

Some npm packages only work on the server (native addons, database drivers, SDKs holding
secrets). Listing them makes accidental client imports a build-time error.

### Bad

```ts
# browser_incompatible_packages: ["pg", "@acme/server-*"]
"use client";
import { query } from "../lib/db"; // lib/db imports "pg"
```

### Good

```ts
// keep pg behind a Server Action or route handler and call that from the client
```

### How to fix

Replace the package with a browser-safe client, or keep it behind a server boundary.

### Configuration

```yaml
runtime:
  browser_incompatible_packages: ["node:*", "pg", "@acme/server-*"]
rules:
  RUNTIME-003: error
```

### False positives

The list is opt-in; an empty list produces no diagnostics. Keep entries specific to packages that
really require server APIs.

## RUNTIME-004

**Edge-incompatible dependency** — Prevents Edge modules from reaching Node-only modules or incompatible packages.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

Edge runtimes (Next.js middleware, `runtime = "edge"` routes) do not provide Node APIs. A
transitive Node import fails only at deploy time unless it is caught statically.

### Bad

```ts
// src/middleware.ts (Edge)
import { verify } from "./lib/auth"; // lib/auth imports "node:crypto"
```

### Good

```ts
// src/middleware.ts
import { verify } from "./lib/auth-edge"; // uses Web Crypto (crypto.subtle)
```

### How to fix

Use an Edge-compatible implementation (Web APIs), or move the work to a Node runtime route.

### Configuration

```yaml
runtime:
  edge_incompatible_packages: ["node:*", "*-native"]
rules:
  RUNTIME-004: error
```

### False positives

Middleware is Edge by convention; a route is Edge only with an explicit `runtime = "edge"` export.

## RUNTIME-005

**Ambiguous universal runtime** — Detects universal modules that transitively require incompatible runtime capabilities.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

A "universal" module that transitively needs both browser and server capabilities cannot run
correctly anywhere. It is usually a shared file that grew imports from both sides.

### Bad

```ts
// src/lib/session.ts (no runtime marker)
import { useSyncExternalStore } from "./client-store"; // browser
import { cookies } from "./server-cookies";             // server
```

### Good

```ts
// src/lib/session-types.ts      — shared contract
// src/lib/session.client.ts     — browser implementation
// src/lib/session.server.ts     — server implementation
```

### How to fix

Split the module at the boundary the diagnostic reports: a runtime-neutral contract plus one
implementation per runtime.

### Configuration

```yaml
rules:
  RUNTIME-005: error
```

### False positives

Only declared browser boundaries (`"use client"`, `client-only`, an explicit runtime) count as
browser requirements; modules that are merely bundled for the browser because some client
component imports them do not. The diagnostic includes both paths as evidence; if a module was
misclassified, fix its marker rather than suppressing.

## RUNTIME-006

**Incompatible runtime cycle** — Detects dependency cycles that join incompatible runtime domains.

- Category: `runtime-graph`
- Default: error once a wae.yaml exists

### Why

A cycle that joins browser and server (or Edge and Node) code cannot be split into valid
bundles; every module in it inherits both runtimes.

### Bad

```ts
// client.tsx ("use client") -> server.ts -> client.tsx
```

### Good

```ts
// client.tsx -> contract.ts <- server.ts (no cycle across the boundary)
```

### How to fix

Break the cycle at a runtime-neutral contract or an explicit transport boundary such as a
Server Action.

### Configuration

```yaml
rules:
  RUNTIME-006: error
```

### False positives

Cycles through Server Action (RPC) boundaries are excluded. A plain cycle is also reported as
`ARCH-001`; fixing the cycle resolves both.

## PARSE-001

**Parse failure** — Reports malformed or unreadable source files.

- Category: `correctness`
- Default: error; always enabled

### Why

WAE cannot see the imports of a file it cannot parse, so the architecture model would be
incomplete. The file is reported instead of silently skipped.

### Bad

```ts
// src/broken.ts
export const value = ;
```

### Good

```ts
// src/broken.ts
export const value = 1;
```

### How to fix

Fix the syntax error (your compiler reports the details), or exclude generated or intentionally
invalid files with `project.exclude`.

### Configuration

```yaml
project:
  exclude: ["**/fixtures/invalid/**"]
```

### False positives

Tree-sitter accepts all standard JS/TS/JSX/TSX syntax. Experimental syntax not yet supported by
the grammar may be reported; exclude those files and open an issue.

## RESOLVE-001

**Unresolved import** — Reports relative or aliased imports that cannot be resolved.

- Category: `correctness`
- Default: error; always enabled

### Why

An unresolved relative or aliased import is a broken edge in the architecture graph: rules
cannot evaluate what they cannot see, and the import usually fails at build time too.

### Bad

```ts
import { Button } from "@/components/buton"; // typo
```

### Good

```ts
import { Button } from "@/components/button";
```

### How to fix

Fix the path, or teach WAE the alias through `tsconfig.json`/`jsconfig.json` `paths`. Run
`wae resolve <importer> <specifier>` to see every resolver step that was tried.

### Configuration

```yaml
# aliases come from tsconfig.json / jsconfig.json; conditions and generated modules from:
resolution:
  mode: bundler
  custom_conditions: [browser]
  virtual_modules: ["contentlayer/generated"]   # created by a build step
```

### False positives

Bare npm package imports are treated as external and are never reported. Bundler-only aliases
(webpack/Vite `resolve.alias`) must be mirrored in tsconfig `paths`. Imports of code generated by
a build step (contentlayer, codegen output) fail before that step runs: run WAE after generation,
or list the specifiers in `resolution.virtual_modules` to treat them as opaque modules.

## RESOLVE-002

**Invalid module specifier** — Reports module specifiers that violate the analysis boundary.

- Category: `correctness`
- Default: error; always enabled

### Why

Absolute filesystem specifiers and similar forms escape the project analysis boundary and make
results depend on the machine running the analysis.

### Bad

```ts
import config from "/Users/alice/project/src/config";
```

### Good

```ts
import config from "@/config"; // or a relative path
```

### How to fix

Use a relative path, a tsconfig alias or a package name.

### Configuration

```yaml
# not configurable
```

### False positives

None known; URL-style and absolute specifiers are intentionally rejected.

## SUPPRESS-001

**Unused suppression** — Reports suppression directives that did not match a diagnostic.

- Category: `maintainability`
- Default: warning; controlled by `suppressions.require_reason` and `report_unused`

### Why

Suppressions without a reason, for unknown rules, or that no longer match anything hide real
problems and rot over time.

### Bad

```ts
// wae-ignore ARCH-003
import { legacy } from "../legacy"; // no reason given
```

### Good

```ts
// wae-ignore ARCH-003 -- legacy adapter; remove after ARC-142
import { legacy } from "../legacy";
```

### How to fix

Add a concrete reason after `--`, correct the rule ID, or delete the unused directive. `wae
suppressions validate` and `wae suppressions prune` audit config-level suppressions.

### Configuration

```yaml
suppressions:
  require_reason: true
  report_unused: true
```

### False positives

A directive applies to its own line or the next line only; move it directly above the import.
