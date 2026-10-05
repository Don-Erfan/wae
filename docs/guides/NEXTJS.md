# Next.js guide

WAE understands the Next.js App Router and Pages Router well enough to catch client/server/edge
mistakes before `next build` or production does, including when the problem is several imports
away from the file you are editing.

Supported and tested in CI with real installs and builds: Next.js `13.5`, `14.2`, `15.5` and
`16.3` (App, Pages and hybrid routers; root and `src/` layouts; monorepo packages).

## How WAE detects Next.js

The Next.js adapter is enabled for a package when `next` appears in that package's dependencies or
the package has a `next.config.{js,mjs,cjs,ts}`. In a monorepo each package is detected
independently, so a Next.js app and a plain library can live side by side. Directory names alone
never enable the adapter. To force it:

```yaml
framework:
  auto_detect: false
  enabled: [nextjs]
```

## How each module gets its runtime

Every module is classified as `browser`, `server`, `edge`, `node`, `universal` or `unknown`. The
first matching rule wins:

| Evidence | Runtime | Source |
|---|---|---|
| `import "server-only"` | server | `marker-package` |
| `import "client-only"` | browser | `marker-package` |
| `export const runtime = "edge"` / `"nodejs"` | edge / node | `explicit` |
| `"use client"` directive | browser | `directive` |
| `middleware.ts` at the package root or `src/` | edge | `convention` |
| App Router files, Pages API routes, `_document`, `"use server"` modules | server | `convention` |
| Node builtins (`fs`, `node:crypto`, …) | node | `builtin` |
| anything else | universal | `default` |

Then browser-ness propagates: a universal module imported (transitively) by a `"use client"` file
is bundled for the browser, so it becomes `browser` with source `propagated`. Propagation stops at
Server Action modules (`"use server"`), because importing an action creates an RPC reference, not a
bundle dependency on its implementation.

Ask WAE why any module has its runtime:

```bash
npx wae graph --module src/lib/format.ts
#   runtime:   browser (propagated: imported (transitively) by a browser boundary, …)
#   via:       src/app/profile/card.tsx → src/lib/user.ts → src/lib/format.ts
```

Editors show the same explanation on hover.

## Runtime rules

| Rule | Catches |
|---|---|
| `RUNTIME-001` | a browser module reaching server-only code |
| `RUNTIME-002` | a browser module reaching Node-only code (`fs`, `node:*`) |
| `RUNTIME-003` | a browser module reaching a package you listed as browser-incompatible |
| `RUNTIME-004` | an Edge module (middleware, `runtime = "edge"`) reaching Node-only code or listed packages |
| `RUNTIME-005` | a universal module that requires both a declared browser boundary and server code |
| `RUNTIME-006` | a cycle that joins browser and server (or Edge and Node) code |

Each runtime diagnostic reports the shortest offending path, and type-only imports are ignored
because TypeScript erases them:

```text
RUNTIME-001 Browser module transitively depends on server-only code
src/app/profile/profile-card.tsx:4:29
src/app/profile/profile-card.tsx
→ src/features/user/index.ts
→ src/features/user/api.ts
→ src/server/db.ts
```

Fix by cutting the first edge that crosses the boundary: call the server through a Server Action
or route handler, split shared modules into `*.client.ts` / `*.server.ts`, and keep re-export
barrels free of server-only code.

## Recommended configuration

```yaml
version: 1
resolution:
  mode: bundler            # matches Next.js "moduleResolution": "bundler"
runtime:
  browser_incompatible_packages: ["pg", "@prisma/client", "@acme/server-*"]
  edge_incompatible_packages: ["node:*", "bcrypt", "sharp"]
architecture:
  layers:
    app:      { patterns: ["src/app/**", "src/middleware.ts"], canImport: [features, entities, shared, server] }
    features: { patterns: ["src/features/**"], canImport: [entities, shared, server] }
    server:   { patterns: ["src/server/**"],   canImport: [entities, shared] }
    entities: { patterns: ["src/entities/**"], canImport: [shared] }
    shared:   { patterns: ["src/shared/**"],   canImport: [] }
  features:
    roots: ["src/features"]
  forbidden_dependencies:
    - from: "src/app/**"
      to: "src/server/db/**"     # pages go through features, not straight to the database
```

`npx wae init --preset next` writes a starting point. Mark files with `import "server-only"` to make
server boundaries explicit; WAE then reports every client path that reaches them.

## Generated modules

Imports of build-generated code (for example `contentlayer/generated`) cannot be resolved before
the build runs. Either run WAE after the generation step or declare them:

```yaml
resolution:
  virtual_modules: ["contentlayer/generated"]
```

## What WAE does not do

WAE does not run Next.js, evaluate `next.config.js` or check React rules; it complements
`next build`, ESLint and TypeScript. Dynamic `import()` with a non-literal argument and runtime
`require()` of computed paths cannot be followed statically.
