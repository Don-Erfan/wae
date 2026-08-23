# Web Architecture Engine (WAE)

WAE analyzes JavaScript and TypeScript dependency architecture. Its production path is:

```text
source discovery → import parsing → Node/TypeScript resolution → module graph → rules → diagnostics
```

The current engine extracts static imports, type-only imports, re-exports, dynamic imports, and CommonJS `require` calls; resolves relative extension/index imports and `tsconfig` path aliases; and evaluates `ARCH-001` through `ARCH-005` against a shared graph.

## Use from source

```bash
cargo run -p wae-cli -- check
cargo run -p wae-cli -- check --format json
cargo run -p wae-cli -- graph
cargo run -p wae-cli -- doctor
```

Create a typed, versioned configuration:

```bash
cargo run -p wae-cli -- init
```

Ratchet mode never creates state implicitly. Review the current diagnostics, explicitly create and commit the baseline, then compare affected files and their importer closure against Git:

```bash
cargo run -p wae-cli -- baseline create
WAE_BASE_REF=origin/master cargo run -p wae-cli -- check --changed
```

Supported reporters are `human`, `json`, `jsonl`, and `sarif`. Exit codes are stable: `0` passed, `1` violations, `2` project/config error, and `3` internal error.

## npm wrapper

```bash
npm install --save-dev @don-erfan/wae
npx wae check
```

The installer supports Linux x64/arm64, macOS x64/arm64, and Windows x64. It enforces redirect, timeout, and size limits and verifies the downloaded binary against the release SHA-256 asset before installation.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

See [Architecture](docs/ARCHITECTURE.md), [Compatibility](docs/COMPATIBILITY.md), [Contributing](CONTRIBUTING.md), and [Security](SECURITY.md).
