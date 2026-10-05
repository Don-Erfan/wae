# @don-erfan/wae

Web Architecture Engine (WAE) for JavaScript and TypeScript: deterministic architecture checks
for layers, features, public APIs, workspace packages and Next.js runtime boundaries.

## Install

```bash
npm install --save-dev @don-erfan/wae
# or: pnpm add -D @don-erfan/wae / yarn add -D @don-erfan/wae
```

This is the only WAE npm package. It contains JavaScript launchers and installs the native CLI,
language server and MCP server for Linux x64/arm64, macOS x64/arm64 or Windows x64.

## Usage

```bash
npx wae init --preset next      # or: wae discover --write
npx wae check                   # exit 0 passed, 1 violations, 2 config/project, 3 internal
npx wae check --format json     # stable, versioned machine output (also jsonl and sarif)
npx wae baseline create         # adopt on an existing codebase, then:
npx wae check --changed         # fail only on newly introduced violations
npx wae explain ARCH-004        # why a rule exists, bad/good examples and configuration
npx wae graph --module src/app/page.tsx
npx wae doctor
```

`wae-lsp` (editor language server) and `wae-mcp` (AI agent MCP server) are installed as
additional executables.

## How the native binaries are installed

1. `postinstall` downloads the three binaries for the current platform from the GitHub Release
   whose tag equals this package version.
2. Each file is verified against the SHA-256 hash embedded in this package at publish time. The
   hashes come from the Sigstore-signed `SHA256SUMS` of that release, so the trust root is the npm
   package integrity recorded in your lockfile.
3. Verified files are installed atomically into the package's `vendor/` directory, or into the
   per-user cache when `node_modules` is read-only.

When lifecycle scripts are disabled (`--ignore-scripts`, pnpm 10 defaults), the first `wae`,
`wae-lsp` or `wae-mcp` invocation performs the same verified install. Progress is written to
stderr, so the LSP and MCP stdio protocols stay clean.

| Variable | Effect |
|---|---|
| `WAE_BINARY_DIR` | Use pre-provisioned binaries (`wae`, `wae-lsp`, `wae-mcp`) from this directory; never download. |
| `WAE_SKIP_DOWNLOAD=1` | Never download; fail with an actionable message if binaries are missing. |
| `WAE_CACHE_DIR` | Override the per-user cache (`~/.cache/wae`, `~/Library/Caches/wae`, `%LOCALAPPDATA%\wae\Cache`). |

For air-gapped CI, download the release assets once, verify them with `sha256sum --check
SHA256SUMS` and `cosign verify-blob` (see the repository README), rename them to `wae`,
`wae-lsp` and `wae-mcp` (`.exe` on Windows), and point `WAE_BINARY_DIR` at that directory.

## Documentation

See the [repository](https://github.com/Don-Erfan/wae) for the getting-started guide, rule
reference, configuration reference, CI, VS Code and WebStorm guides.
