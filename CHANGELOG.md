# Changelog

All notable changes to WAE are documented here. GitHub Releases automatically adds the complete
pull-request and contributor list for every signed version tag.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/Don-Erfan/wae/compare/v0.0.10...HEAD
[0.0.10]: https://github.com/Don-Erfan/wae/compare/v0.0.9...v0.0.10
