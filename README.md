# Web Architecture Engine (WAE)

WAE enforces the architecture of JavaScript and TypeScript codebases: layers, feature boundaries,
public APIs, workspace packages, dependency cycles and Next.js client/server/edge boundaries. It
builds the real module and package graphs (with Node and TypeScript resolution), evaluates
deterministic rules, and reports every violation with the dependency path that causes it, in the
terminal, in CI, in VS Code and WebStorm, and to AI agents over MCP.

```text
RUNTIME-001 Browser module transitively depends on server-only code
src/app/profile/profile-card.tsx:4:29
src/app/profile/profile-card.tsx
→ src/features/user/index.ts
→ src/features/user/api.ts
→ src/server/db.ts
Suggestion: Move the server operation behind a Server Action or another explicit RPC boundary.
```

WAE leaves style and per-function linting to ESLint/Biome and type checking to TypeScript; it
checks what those tools cannot see: how modules and packages depend on each other.

## Quick start

```bash
npm install --save-dev @don-erfan/wae
npx wae check                     # zero-config: cycles, unresolved imports, parse errors
npx wae discover --write          # or: npx wae init --preset next
npx wae check                     # every rule enabled once wae.yaml exists
npx wae explain ARCH-004          # why a rule exists, bad/good examples, fix, configuration
npx wae baseline                  # accept existing violations…
npx wae check --changed           # …and fail only on new ones
```

Read the [getting-started guide](docs/GETTING_STARTED.md) for the full walkthrough.

## What it checks

| Family | Rules |
|---|---|
| Architecture | cycles, forbidden dependencies, layers, feature public APIs, private modules, depth, coupling, orphans, layer ownership and coverage (`ARCH-001..011`) |
| Packages | package cycles, forbidden package dependencies, undeclared workspace imports, cross-package relative imports (`PACKAGE-001..004`) |
| Runtime | browser → server, browser → Node, browser-incompatible packages, Edge → Node, ambiguous universal modules, cross-runtime cycles (`RUNTIME-001..006`) |

Every rule is documented with rationale, examples, configuration and known false positives in the
[rule reference](docs/RULES.md) (`wae rules`, `wae explain <RULE_ID>`).

## Install

- **npm** (recommended): `npm install --save-dev @don-erfan/wae`. One package; it downloads the
  native `wae`, `wae-lsp` and `wae-mcp` binaries for Linux x64/arm64, macOS x64/arm64 or Windows x64
  from the matching GitHub Release, verifies them with SHA-256 hashes embedded in the package, and
  works with install scripts disabled (first-run download) or offline (`WAE_BINARY_DIR`).
- **GitHub Releases**: signed binaries, VS Code VSIX and JetBrains plugin ZIP.
- **From source**: `cargo build --release -p wae-cli -p wae-lsp -p wae-mcp`.

## Integrations

- **CI**: `uses: Don-Erfan/wae@v1.0.0` adds inline pull-request annotations and a job summary;
  stable exit codes (`0` passed, `1` violations, `2` config/project error, `3` internal error,
  `130` cancelled) and versioned JSON/JSONL/SARIF output work in any CI. [CI guide](docs/guides/CI.md)
- **VS Code**: live diagnostics, dependency paths, rule explanations, module inspection and an
  Architecture explorer. [VS Code guide](docs/guides/VSCODE.md)
- **WebStorm / IntelliJ IDEA**: the same diagnostics and Alt+Enter actions through the platform LSP
  API. [WebStorm guide](docs/guides/WEBSTORM.md)
- **AI agents**: `wae-mcp` answers "can module X depend on Y?" without parsing terminal output.
  [MCP server](docs/INTEGRATIONS.md#mcp-server)

The CLI, editors, CI and MCP all run the same engine and are verified against the same golden
diagnostics, so they always agree.

## Documentation

- [Getting started](docs/GETTING_STARTED.md) · [Configuration](docs/CONFIGURATION.md) ·
  [Rules](docs/RULES.md) · [Troubleshooting](docs/TROUBLESHOOTING.md)
- Guides: [Next.js](docs/guides/NEXTJS.md) · [Monorepo](docs/guides/MONOREPO.md) ·
  [CI](docs/guides/CI.md) · [VS Code](docs/guides/VSCODE.md) · [WebStorm](docs/guides/WEBSTORM.md)
- Reference: [Debugging](docs/DEBUGGING.md) · [IDE protocol](docs/IDE.md) ·
  [MCP/Explorer/Action](docs/INTEGRATIONS.md) · [Compatibility](docs/COMPATIBILITY.md) ·
  [Migration guide](docs/MIGRATIONS.md) · [Changelog](CHANGELOG.md)
- Engineering: [Product](docs/PRODUCT.md) · [Architecture](docs/ARCHITECTURE.md) ·
  [Roadmap](docs/ROADMAP.md) · [v1 release gate](docs/V1_RELEASE_GATE.md) ·
  [Acceptance](docs/ACCEPTANCE.md) · [Real-world validation](docs/REAL_WORLD_VALIDATION.md) ·
  [Performance](docs/PERFORMANCE.md) · [Reliability](docs/RELIABILITY.md) ·
  [Releasing](docs/RELEASING.md) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md)

## Verify release assets

Release binaries, the SPDX asset inventory, the CycloneDX dependency SBOM, and their aggregate
manifest are covered by `SHA256SUMS`. Verify the files first, then verify the manifest's keyless
Sigstore identity:

```bash
sha256sum --check SHA256SUMS
cosign verify-blob \
  --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp 'https://github\.com/Don-Erfan/wae/\.github/workflows/release-binaries\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
gh attestation verify wae-x86_64-unknown-linux-gnu --repo Don-Erfan/wae
jq '.packages | length' wae-v1.0.0-assets.spdx.json
jq '.components | length' wae-v1.0.0-dependencies.cdx.json
```

The checksum proves both downloaded inventories are the files signed by the release workflow; use
SPDX tooling for assets and CycloneDX tooling for the Cargo dependency tree.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo bench --workspace --no-run
npm --prefix npm/wae run test:installer
node --test action/report.test.js
```

Run from source with `cargo run -p wae-cli -- check` (the source-built CLI binary is `wae-cli`).

## License

MIT
