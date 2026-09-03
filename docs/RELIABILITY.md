# Reliability, compatibility and noise budgets

## False-positive budget

The deterministic clean corpus currently contains 32 source modules across a minimal project,
TypeScript aliases, a 12-package workspace, Nx/Turborepo layouts and the semantic Next.js consumer. Every enabled rule
must produce zero diagnostics on that corpus. This is a blocking test, so the current measured
fixture false-positive rate is **0/32 (0%)**. The 500-module acceptance scenario adds a clean cold
and warm run before fault injection.

This number describes the maintained corpus, not an unsupported claim about all JavaScript code.
New framework conventions must first add a clean fixture and may not raise the zero-noise budget.

## Robustness gates

- parser and strict YAML config have `cargo-fuzz` targets;
- generated DAG property tests compare graph reachability with a reference transitive closure;
- cancellation is cooperative at discovery/module/import boundaries and the CLI maps Ctrl+C to
  exit code `130` without writing a partial cache result;
- malformed source/config, Unicode columns, cache races, deleted targets, symlinks and large graphs
  have deterministic tests;
- known analysis failures return typed errors rather than panicking.

## Compatibility matrix

| Axis | CI / fixture contract |
|---|---|
| Rust | MSRV 1.85 and stable |
| OS | Ubuntu, macOS, Windows |
| Node installer | Node 20, 22 and 24 |
| TypeScript syntax | TS/TSX plus `.mts`, `.cts`, declaration resolution |
| Next.js | Latest maintained patches of 13, 14, 15 and current stable; App, Pages and hybrid routers |
| Resolution | Node10, Node16, NodeNext and Bundler |

Version additions require a green matrix entry before documentation claims support.

CI installs each exact Next.js package version, builds the hybrid App/Pages consumer with the real
Next compiler, then runs WAE on that installed project. The maintained framework/runtime corpus
also covers package-root-relative routing, Edge and Node runtimes, Server Actions,
`server-only`/`client-only` markers, and browser propagation from a `'use client'` boundary.
