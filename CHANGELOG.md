# Changelog

All notable changes to WAE are documented here. GitHub Releases automatically adds the complete
pull-request and contributor list for every signed version tag.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.19] - 2026-08-30

### Added

- A shared reverse multi-source runtime reachability index with deterministic shortest-path
  reconstruction, plus an engine-level 10,000-module cold/warm/single-edit performance gate that
  records wall time and peak RSS.
- A long-lived, cancellable `WorkspaceSession` for LSP analysis, debounced background diagnostics,
  real suppression `WorkspaceEdit` code actions, and a framed stdio protocol integration test.
- Semantic parser facts for directive prologues and exported runtime literals, architecture
  coverage thresholds and reports, MCP dependency-policy queries, and workspace-root confinement.
- Release-built VS Code and JetBrains artifacts, SARIF upload support in the composite action, and
  separate SPDX asset and CycloneDX dependency SBOMs.

### Changed

- Split engine orchestration into an explicit analysis pipeline and replaced overlapping timing
  buckets with reconcilable discovery, classification, parsing, resolution, graph, rule, cache,
  reporting, and orchestration phases.
- Made CI reusable at an exact commit or tag and made binary releases depend on the same complete
  quality, compatibility, performance, fuzz, integration, installer, and IDE readiness workflow.

### Fixed

- Reduced `RUNTIME-005` from repeated per-module graph traversals to linear indexed reachability and
  skip evaluation when either required runtime target set is absent.
- Prevented concurrent cache writers from restoring stale entries by committing only dirty updates;
  warm no-op analyses no longer rewrite the cache and Windows replacement is atomic.
- Fixed the MCP contract artifact directory, pinned `cargo-fuzz` installation, corrected the
  JetBrains WebStorm/LSP target, and downgraded missing Git in `doctor` to a changed-mode warning.
- Kept fuzz execution on nightly even when the repository toolchain file pins the MSRV toolchain.
- Updated architecture coverage arithmetic for the stricter Rust 1.98 Clippy release gate.

## [0.0.18] - 2026-08-29

### Release status

- The signed source tag was retained for auditability, but the new readiness gate correctly
  blocked binary and npm publication after detecting quality and fuzz environment failures.

## [0.0.17] - 2026-08-29

### Fixed

- Made the LSP file-URI round-trip contract use the operating system's real
  temporary root, including a drive-qualified path on Windows.

## [0.0.16] - 2026-08-29

### Fixed

- Matched TypeScript configuration scopes using normalized path boundaries so
  canonical Windows directories and normalized importer IDs resolve identically.

## [0.0.15] - 2026-08-29

### Fixed

- Normalized Windows verbatim canonical paths before resolver matching, keeping
  TypeScript aliases functional on Windows runners.
- Compared jsconfig targets against canonical temporary directories so macOS
  `/var` and `/private/var` aliases are handled consistently.

## [0.0.14] - 2026-08-29

### Fixed

- Rewrote Next.js framework detection scoring as explicit conditionals so the
  release gate remains warning-free on Rust/Clippy 1.98 and newer.

## [0.0.13] - 2026-08-29

### Added

- `ARCH-006` through `ARCH-010`, `PACKAGE-001` through `PACKAGE-004`, and `RUNTIME-001`
  through `RUNTIME-006` over shared module/package/runtime graphs.
- A semantic Next.js adapter, module-level incremental cache, resolver traces, Criterion suites,
  100k-node performance gates, and a 500-module fault-injection acceptance scenario.
- `wae-lsp`, VS Code and JetBrains clients; `wae-mcp` with four architecture tools; and a
  self-contained `wae explore` report.
- Evidence-based `wae discover` proposals for Next.js, FSD, Nx and Turborepo layouts, including
  feature clusters and explicit approve/overwrite behavior.
- A synchronized JSON Schema, reusable composite GitHub Action, fuzz/property tests, Ctrl+C
  cancellation and an aggregate `v1 readiness` CI gate.

### Changed

- The npm installer now downloads and checksum-verifies the CLI, LSP and MCP binaries for every
  supported platform.
- Large configured projects evaluate independent rules concurrently and merge results in stable
  registry order.
- Alias resolution accepts nearest `jsconfig.json` projects while preferring `tsconfig.json` in
  the same directory, and the parser recognizes literal `require.resolve()` dependencies.

## [0.0.12] - 2026-08-25

### Fixed

- Make diagnostic identity structural and independent of messages, severity, suggestions,
  positions, arbitration metadata, and presentation changes while accepting both `0.0.10` and
  `0.0.11` legacy baseline fingerprints during migration.
- Select Bundler `import`/`require` conditions from dependency syntax instead of the importer's
  package `type`, including TypeScript `import = require()` syntax.
- Keep modern package `exports` targets separate from legacy `main`, `module`, `types`, and
  `typings` entrypoints; support package-relative legacy paths and prioritize declaration targets
  for type-only imports.
- Limit package-scope discovery to source-importer ancestors so malformed manifests in excluded
  output directories cannot break analysis.
- Shorten cache lock duration to the reload/merge/prune/atomic-save transaction and prune entries
  for renamed or deleted source files.
- Generate a non-opinionated blank configuration by default and replace repository-wide layer
  globs with explicit, anchored `fsd`, `next`, and `nx` presets.

### Added

- `wae config validate --show-overlaps` and actionable `wae doctor` details for ambiguous layer
  ownership.
- Explicit `node10` resolution mode with a backward-compatible `node` configuration alias.
- Dedicated resolution-kind strategies and package-target/legacy-entrypoint value objects.
- A realistic Next.js/TypeScript consumer contract in Architecture Check CI.
- Documented checksum, keyless Sigstore, provenance-attestation, and SPDX SBOM verification.

## [0.0.11] - 2026-08-24

### Fixed

- Separate Node package scopes from named workspace packages so unnamed private apps and nested
  package boundaries select the correct ESM/CommonJS conditional export.
- Stop activating the `browser` export condition implicitly in Bundler mode.
- Classify `export type` and all-type inline import/export clauses as type-only dependencies.
- Replace crash-sensitive ownership lock files with cross-platform advisory cache locking held
  across the complete read/merge/write transaction.
- Arbitrate overlapping feature-visibility diagnostics deterministically without discarding the
  strongest severity or the identities of related rules.
- Report configured diagnostic severity in SARIF rule defaults and reuse one failure policy in CLI,
  JSON and Architecture Check summaries.

### Added

- Mode-specific condition strategies, a dedicated `PackageScopeIndex`, and a named
  `WorkspacePackageIndex`.
- Real multi-process cache concurrency coverage and package-scope regression fixtures.
- SPDX SBOM generation, GitHub build-provenance attestations, aggregate checksums, and keyless
  Sigstore signatures for binary releases.
- Curated changelog content in automatically generated GitHub release notes.

## [0.0.10] - 2026-08-24

### Fixed

- Preserve overlapping architecture diagnostics so a stricter rule cannot be bypassed by
  deduplication.
- Resolve NodeNext package conditions using both importer module format and resolution kind,
  including nested type-only conditions.
- Reject extensionless relative ESM imports in NodeNext/Node16 mode.
- Restrict source suppressions to standalone line comments and allow a directive to suppress every
  matching diagnostic in its line scope.
- Exclude suppressed and informational diagnostics from new baselines.

### Added

- JavaScript/TypeScript module extensions (`mts`, `cts`, `mjs`, `cjs`) in default discovery.
- Hoisted package-based `tsconfig extends` lookup.
- Detailed module/dependency counts and standard CLI version flags.
- Dependabot coverage and workflow concurrency controls.

[Unreleased]: https://github.com/Don-Erfan/wae/compare/v0.0.19...HEAD
[0.0.19]: https://github.com/Don-Erfan/wae/compare/v0.0.18...v0.0.19
[0.0.18]: https://github.com/Don-Erfan/wae/compare/v0.0.17...v0.0.18
[0.0.17]: https://github.com/Don-Erfan/wae/compare/v0.0.16...v0.0.17
[0.0.16]: https://github.com/Don-Erfan/wae/compare/v0.0.15...v0.0.16
[0.0.15]: https://github.com/Don-Erfan/wae/compare/v0.0.14...v0.0.15
[0.0.14]: https://github.com/Don-Erfan/wae/compare/v0.0.13...v0.0.14
[0.0.13]: https://github.com/Don-Erfan/wae/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/Don-Erfan/wae/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/Don-Erfan/wae/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/Don-Erfan/wae/compare/v0.0.9...v0.0.10
