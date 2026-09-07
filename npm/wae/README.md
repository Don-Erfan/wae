# @don-erfan/wae

NPM wrapper for the WAE CLI.

## Install

```bash
yarn add -D @don-erfan/wae
```

## Usage

```bash
wae help
wae init --preset next
wae discover
wae config validate --show-overlaps
wae scan
wae check
wae check --changed
wae check --fail-on error --max-warnings 20
wae baseline list --rule ARCH-003
wae baseline prune
wae explore
wae-lsp
wae-mcp
```

## How it works

- npm/yarn selects one exact-version optional package for Linux x64/arm64, macOS x64/arm64, or
  Windows x64. Each package contains the CLI, LSP and MCP native binaries.
- Installation works with lifecycle scripts disabled and uses the package manager's integrity,
  offline cache and lockfile normally.
- The release workflow verifies every native binary against the checksum embedded from the signed
  GitHub release before packaging it.
- `npm run recover:binaries --prefix node_modules/@don-erfan/wae` retains the verified GitHub
  downloader as an explicit recovery tool; it is never run automatically.
- GitHub Releases include an aggregate checksum manifest, keyless Sigstore bundle, SPDX SBOM and
  provenance attestations. See the repository README for verification commands.

## Maintainer note

- This package is preconfigured for `Don-Erfan/wae` releases.
- Every platform package must use the same version as this wrapper and be published first.
- npm Trusted Publishing must be configured for the wrapper and all five platform package names;
  authorizing only `@don-erfan/wae` is not sufficient for the release workflow.
