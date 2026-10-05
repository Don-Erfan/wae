# v1.0.0 release gate

This is the phase-by-phase audit of the [engineering roadmap](ROADMAP.md) against its Definition of
Done, with the evidence that proves each item. "CI" means a job that the `v1 readiness` aggregate
requires before a signed tag may be released.

## Phase audit

| # | Phase | Status | Evidence |
|---|---|---|---|
| 0 | Product contract | Done | [PRODUCT](PRODUCT.md), [ARCHITECTURE](ARCHITECTURE.md), [ROADMAP](ROADMAP.md), [COMPATIBILITY](COMPATIBILITY.md) |
| 1 | Rust workspace foundation | Done | CI `quality` (fmt, clippy `-D warnings`), `tests` (Linux/macOS/Windows, stable + MSRV 1.85), `audit` |
| 2 | Core domain / IR | Done | `crates/core`; rules consume `RuleContext` only; synthetic-graph rule tests in `crates/rules` |
| 3 | Error and diagnostic model | Done | typed `EngineError` (parse/resolution/config/internal); determinism asserted by running the synthetic app twice (`crates/cli/tests/synthetic_app.rs`) |
| 4 | Project discovery | Done | npm/Yarn/pnpm workspaces, Nx/Turborepo fixtures, `wae discover` |
| 5 | JS/TS parser | Done | import/export/type/dynamic/require matrix, iterative AST walk, `cargo-fuzz` parser target |
| 6 | Module resolver | Done | `resolution-matrix` fixture, Node10/16/NodeNext/Bundler, exports/imports, aliases; `virtual_modules` for generated/virtual specifiers; `wae resolve` trace |
| 7 | Module graph | Done | SCC, reachability, shortest path, 100k-module gate, property tests against a reference closure |
| 8 | Package graph | Done | `PACKAGE-001..004`, `policies` and `monorepo-12` fixtures |
| 9 | Architecture model | Done | validated layer/feature ownership index, overlap = config error, coverage |
| 10 | Configuration engine | Done | strict versioned schema (`schemas/wae.schema.json`, registry-synchronized), aggregated readable errors, `extends`, `overrides`, YAML fuzzing |
| 11 | Core architecture rules | Done | 21 configurable rules; per-rule golden and false-positive tests; full documentation per rule in [RULES](RULES.md) (generated from code, staleness test) |
| 12 | Rule engine | Done | enable/disable/`off`, severity overrides, path overrides, deterministic ordering, parallel evaluation, suppression |
| 13 | Suppression and baseline | Done | structural fingerprints, baseline v3 with migration, ratchet test on real Git history |
| 14 | CLI v1 | Done | `init`, `discover`, `scan`, `check`, `baseline`, `explain`, `rules`, `graph [--module]`, `resolve`, `explore`, `config validate`, `suppressions`, `doctor`; per-command `--help`; stable exit codes 0/1/2/3/130 |
| 15 | Git regression analysis | Done | `check --changed` with existing/introduced/fixed counts and importer closure |
| 16 | Monorepo production support | Done | 12-package fixture, per-package framework detection |
| 17 | Framework adapter system | Done | `FrameworkAdapter` port; no `if nextjs` in core; `rpcBoundary` capability |
| 18 | Next.js adapter | Done | CI `next-compatibility`: real install + build of Next.js 13.5/14.2/15.5/16.3 |
| 19 | Runtime model | Done | runtime + evidence source on every module; propagation path explained by `wae graph --module` and LSP hover |
| 20 | Runtime rules | Done | `RUNTIME-001..006`, direct and transitive, shortest path evidence |
| 21 | Architecture discovery | Done | `wae discover` with evidence, confidence and explicit unknowns; never writes without `--write` |
| 22 | Cache and incremental engine | Done | content-addressed module shards, scoped rule partitions, 500-module single-edit test |
| 23 | Performance and benchmarking | Done | CI `performance`: 10k and 50k cold/warm/edit/RSS budgets plus relative release baseline; scheduled 100k |
| 24 | Observability and debugging | Done | `check --verbose` phase/per-rule timings, `doctor`, `resolve`, `graph --module`, [DEBUGGING](DEBUGGING.md) |
| 25 | Language server | Done | `wae-lsp`: diagnostics with dependency paths and doc links, hover, code actions, explain/path/inspect/overview commands, multi-root, UTF-16 |
| 26 | VS Code extension | Done | live diagnostics, quick fixes, explain, dependency-path picker, module inspection, Architecture explorer; CI Extension Host e2e |
| 27 | WebStorm / JetBrains plugin | Done* | same LSP actions via Alt+Enter, hover, diagnostics; CI `buildPlugin` + `verifyPlugin`. *No dedicated tool window yet; `wae explore` covers the overview |
| 28 | Architecture explorer | Done | VS Code tree (Violations/Packages/Layers/Features/Runtimes) from `wae_engine::projection`; self-contained HTML `wae explore` |
| 29 | GitHub Action / CI | Done | annotations + job summary from the JSON contract, optional SARIF, exit status propagation; [CI guide](guides/CI.md) |
| 30 | Machine-readable API | Done | JSON/JSONL `schemaVersion: 1`, `schemas/diagnostics.schema.json`, SARIF 2.1.0 with rule help |
| 31 | MCP / AI agents | Done | `architecture_check`, `architecture_explain`, `dependency_path`, `architecture_model`, `dependency_policy`; CI protocol contract |
| 32 | Reliability hardening | Done | fuzz targets, cancellation (exit 130), malformed input/symlink/unicode/deleted-file tests; known failures are typed errors |
| 33 | Compatibility matrix | Done | [COMPATIBILITY](COMPATIBILITY.md) matches CI (OS, Rust, Node 20/22/24, Next.js versions, resolution modes) |
| 34 | Documentation | Done | [Getting started](GETTING_STARTED.md), [Configuration](CONFIGURATION.md), [Rules](RULES.md), [Next.js](guides/NEXTJS.md), [Monorepo](guides/MONOREPO.md), [CI](guides/CI.md), [VS Code](guides/VSCODE.md), [WebStorm](guides/WEBSTORM.md), [Troubleshooting](TROUBLESHOOTING.md) |
| 35 | Dogfooding | Done | WAE checks its own JS/TS tooling at 100% layer coverage (CI); Rust crate dependency direction enforced by `crates/cli/tests/workspace_architecture.rs` |
| 36 | Synthetic test application | Done | `fixtures/synthetic-app` golden, asserted by CLI, MCP and LSP tests |
| 37 | Real-world test project | Done | [Real-world validation](REAL_WORLD_VALIDATION.md); vercel/commerce audited in CI |
| 38 | False-positive audit | Done | three false-positive classes found and fixed with regression tests |
| 39/40 | v1 release gate | Ready | this checklist; release steps below |

## Stability contract for 1.x

Frozen for every 1.x release (breaking changes require 2.0 and a migration guide):

- rule IDs and their meaning (`ARCH-001..011`, `PACKAGE-001..004`, `RUNTIME-001..006`, `PARSE-001`,
  `RESOLVE-001..002`, `SUPPRESS-001`);
- configuration schema `version: 1` (new optional keys may be added);
- JSON/JSONL output `schemaVersion: 1` (new fields may be added; consumers ignore unknown fields)
  and SARIF 2.1.0 output;
- diagnostic fingerprints for unchanged code and configuration;
- CLI commands, flags and exit codes `0/1/2/3/130`;
- LSP server command names and MCP tool names and their required arguments;
- the `@don-erfan/wae` package name and its `wae`, `wae-lsp` and `wae-mcp` executables.

## Release checklist

1. All versions read `1.0.0`: `Cargo.toml`, `npm/wae/package.json`, `editors/vscode/package.json`,
   `editors/jetbrains/build.gradle.kts`, the `action.yml` default (enforced by
   `npm/wae/test/package.test.js`).
2. `CHANGELOG.md` has a `[1.0.0]` section.
3. Push to `master`; wait for the exact commit's `v1 readiness` check to be green.
4. Configure npm Trusted Publishing for `@don-erfan/wae` (single package).
5. `git tag -s v1.0.0 -m "WAE v1.0.0" && git push origin v1.0.0`.
6. The release workflow builds and signs the binaries, publishes the GitHub Release (CLI/LSP/MCP for
   five platforms, VSIX, JetBrains ZIP, SBOMs, Sigstore bundle) and publishes `@don-erfan/wae`,
   then verifies installs with and without lifecycle scripts.
7. Optional, manual: publish the VSIX and JetBrains ZIP to their marketplaces.
