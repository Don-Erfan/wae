# Web Architecture Engine (WAE)

WAE analyzes JavaScript and TypeScript dependency architecture. Its production path is:

```text
source discovery → import parsing → Node/TypeScript resolution → module graph → rules → diagnostics
```

The current engine traverses the Tree-sitter AST for static imports, type-only imports, re-exports, literal dynamic imports, and CommonJS `require` calls. It resolves relative extension/index imports, JSONC `tsconfig` aliases and `extends`, workspace packages, `exports`, and `imports`, then evaluates `ARCH-001` through `ARCH-005` against a shared deterministic graph.

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
cargo run -p wae-cli -- check --changed --base origin/main
```

Supported reporters are `human`, `json`, `jsonl`, and `sarif`. Exit codes are stable: `0` passed, `1` violations, `2` project/config error, and `3` internal error.

## npm wrapper

```bash
npm install --save-dev @don-erfan/wae
npx wae check
```

The installer supports Linux x64/arm64, macOS x64/arm64, and Windows x64. It allows only HTTPS GitHub hosts, enforces redirect, timeout, and size limits, downloads to temporary files, verifies SHA-256, and atomically installs the verified binary.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

See [Configuration](docs/CONFIGURATION.md), [Architecture](docs/ARCHITECTURE.md), [Compatibility](docs/COMPATIBILITY.md), [Contributing](CONTRIBUTING.md), and [Security](SECURITY.md).
Release maintainers should also read [Releasing](docs/RELEASING.md).
