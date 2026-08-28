# Changelog

All notable changes to WAE are documented here. GitHub Releases automatically adds the complete
pull-request and contributor list for every signed version tag.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/Don-Erfan/wae/compare/v0.0.13...HEAD
[0.0.13]: https://github.com/Don-Erfan/wae/compare/v0.0.12...v0.0.13
[0.0.12]: https://github.com/Don-Erfan/wae/compare/v0.0.11...v0.0.12
[0.0.11]: https://github.com/Don-Erfan/wae/compare/v0.0.10...v0.0.11
[0.0.10]: https://github.com/Don-Erfan/wae/compare/v0.0.9...v0.0.10
