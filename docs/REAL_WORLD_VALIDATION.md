# Real-world validation and false-positive audit

Roadmap phases 36–38 require a synthetic project whose results are fully predictable, real
production-like Next.js projects, and an audit of every reported violation for noise. This page
records the method and results for the 1.0 release.

## Synthetic product fixture (phase 36)

`fixtures/synthetic-app` is a two-package workspace (a Next.js app and a UI library) that violates
every rule family on purpose: a module cycle, a forbidden dependency, a layer violation, a
cross-feature internal import, a private-module import, a package cycle, a forbidden package
dependency, two undeclared workspace imports, a cross-package relative import, a client component
reaching server-only code, Node-only code and a browser-incompatible package, and middleware
reaching a Node builtin. `expected-diagnostics.json` is the golden result (14 diagnostics with
rule, fingerprint, location and dependency path).

The same golden file is asserted by three independent surfaces:

| Surface | Test |
|---|---|
| CLI (`wae check --format json`), run twice for byte-identical output | `crates/cli/tests/synthetic_app.rs` |
| MCP `architecture_check` | `crates/mcp/src/lib.rs` (`architecture_check_matches_the_shared_synthetic_app_golden`) |
| LSP `publishDiagnostics` over stdio | `crates/lsp/tests/protocol.rs` (`stdio_server_reports_the_shared_synthetic_app_golden`) |

The VS Code and JetBrains clients render the LSP output without logic of their own, so they report
the same violations. A second test copies the fixture into a Git repository, records a baseline,
verifies that `wae check --changed` passes, introduces a new cross-feature import and verifies that
exactly that one violation fails the check.

## Real projects (phase 37)

Pinned, shallow clones of public Next.js App Router projects, analyzed with no configuration and
with every rule enabled (`version: 1`, `resolution.mode: bundler`):

| Project (commit) | Modules / edges | Before audit | After audit |
|---|---|---|---|
| vercel/commerce `3761e52` | 66 / 210 | 1 × ARCH-001 | 0 |
| shadcn-ui/taxonomy `298a885` | 129 / 515 | 7 × RESOLVE-001 | 0 (with `virtual_modules`) |
| vercel/ai-chatbot `c2f8235` | 150 / 780 | 1 × ARCH-001 | 4 × ARCH-001 |

vercel/commerce is also analyzed in CI on every change (`integration-contracts` job).

## Findings and changes (phase 38)

Every diagnostic was traced to its source imports and classified.

1. **Type-only cycles (false positive, fixed).** The only cycle in vercel/commerce, and the only
   cycle reported for vercel/ai-chatbot, were closed by `import type`. TypeScript erases those
   imports, so neither cycle executes. In ai-chatbot the type-only edge also merged four genuine
   runtime cycles into one strongly connected component, so three real problems were hidden behind
   one misleading path. `ARCH-001` now ignores type-only edges by default (opt back in with
   `include_type_only: true`); commerce is clean and ai-chatbot reports its four real runtime cycles
   (`artifact.tsx ↔ artifact-actions.tsx`, `lib/artifacts/server.ts ↔ artifacts/code/server.ts`,
   `lib/editor/config.ts ↔ functions.tsx`, and a sidebar/hook cycle), each with value imports.
2. **Build-generated modules (expected, now configurable).** taxonomy imports
   `contentlayer/generated`, a tsconfig alias to a gitignored directory that exists only after
   `contentlayer build`. The diagnostic is correct for a fresh checkout. `resolution.virtual_modules`
   now lets a project declare such specifiers; real files still win when they exist.
3. **Propagated browser modules in RUNTIME-005 (false positive, fixed).** While building the
   synthetic fixture, a universal module that imported a feature entrypoint also used by a client
   component was reported as "combining browser and server requirements", together with every
   universal module above it (3 diagnostics for one leak). The leak itself is already reported once
   as `RUNTIME-001`. `RUNTIME-005` now counts only declared browser boundaries.

Each fix has a regression test that fails on the previous code
(`type_only_cycles_are_ignored_by_default_and_reported_on_request`,
`configured_virtual_modules_replace_unresolved_generated_imports`,
`modules_bundled_for_the_browser_by_another_importer_do_not_make_universal_callers_ambiguous`).

## Noise budget

- Clean corpus (minimal project, aliases, 12-package workspace, Nx/Turborepo layouts, semantic
  Next.js consumer, three real projects above): zero false positives with every rule enabled.
- Every remaining real-world diagnostic is a true positive with an actionable dependency path.

New framework conventions or rules must keep this corpus clean before release.
