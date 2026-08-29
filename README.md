# Web Architecture Engine (WAE)

WAE analyzes JavaScript and TypeScript dependency architecture. Its production path is:

```text
source discovery → import parsing → Node/TypeScript resolution → module graph → rules → diagnostics
```

The current engine traverses the Tree-sitter AST for static imports, type-only imports, re-exports, literal dynamic imports, and CommonJS `require` calls. It resolves relative extension/index imports, JSONC `tsconfig` aliases and `extends`, workspace packages, `exports`, and `imports`, then evaluates `ARCH-001` through `ARCH-010`, `PACKAGE-001` through `PACKAGE-004`, and `RUNTIME-001` through `RUNTIME-006` against shared deterministic graphs. A real framework adapter classifies Next.js App/Pages Router modules, client/server directives, routes, actions, middleware, and explicit runtime exports. Runtime diagnostics carry the shortest transitive dependency path, while Server Actions act as explicit RPC boundaries.

## Use from source

```bash
cargo run -p wae-cli -- check
cargo run -p wae-cli -- check --format json
cargo run -p wae-cli -- graph
cargo run -p wae-cli -- explore
cargo run -p wae-cli -- doctor
cargo run -p wae-cli -- resolve src/app/page.tsx '@/features/cart'
```

Create a typed, versioned configuration. The default is intentionally blank, so WAE never assigns
ownership from broad guesses. Choose an explicit preset when the repository follows a known
layout:

```bash
cargo run -p wae-cli -- init
cargo run -p wae-cli -- init --preset next
cargo run -p wae-cli -- discover
cargo run -p wae-cli -- config validate --show-overlaps
cargo run -p wae-cli -- config validate --show-coverage --show-unassigned
```

Ratchet mode never creates state implicitly. Review the current diagnostics, explicitly create and commit the baseline, then compare affected files and their importer closure against Git:

```bash
cargo run -p wae-cli -- baseline create
WAE_BASE_REF=origin/master cargo run -p wae-cli -- check --changed
cargo run -p wae-cli -- check --changed --base origin/main
```

Supported reporters are `human`, `json`, `jsonl`, and `sarif`. Exit codes are stable: `0` passed, `1` violations, `2` project/config error, and `3` internal error.

## npm wrapper

```bash
npm install --save-dev @don-erfan/wae
npx wae check
```

The installer supports Linux x64/arm64, macOS x64/arm64, and Windows x64. It installs the CLI,
language server and MCP server, allows only HTTPS GitHub hosts, enforces redirect, timeout, and size
limits, downloads to temporary files, verifies SHA-256, and atomically installs each verified
binary.

## Verify release assets

Release binaries, the SPDX asset inventory, the CycloneDX dependency SBOM, and their aggregate
manifest are covered by `SHA256SUMS`. Verify
the files first, then verify the manifest's keyless Sigstore identity:

```bash
sha256sum --check SHA256SUMS
cosign verify-blob \
  --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp 'https://github\.com/Don-Erfan/wae/\.github/workflows/release-binaries\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
gh attestation verify wae-x86_64-unknown-linux-gnu --repo Don-Erfan/wae
jq '.packages | length' wae-v0.0.18-assets.spdx.json
jq '.components | length' wae-v0.0.18-dependencies.cdx.json
```

The checksum proves both downloaded inventories are the files signed by the release workflow; use
SPDX tooling for assets and CycloneDX tooling for the Cargo dependency tree.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo bench --workspace --no-run
```

See [Configuration](docs/CONFIGURATION.md), [Architecture](docs/ARCHITECTURE.md), [Acceptance](docs/ACCEPTANCE.md), [Debugging](docs/DEBUGGING.md), [IDE integrations](docs/IDE.md), [MCP/Explorer/Action integrations](docs/INTEGRATIONS.md), [Performance](docs/PERFORMANCE.md), [Reliability](docs/RELIABILITY.md), [Compatibility](docs/COMPATIBILITY.md), [Contributing](CONTRIBUTING.md), and [Security](SECURITY.md).
Release maintainers should also read [Releasing](docs/RELEASING.md).
