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
wae explore
wae-lsp
wae-mcp
```

## How it works

- During installation, `postinstall` downloads the CLI, LSP and MCP platform binaries from GitHub Releases.
- Binary assets are resolved from:
  - `https://github.com/<repo>/releases/download/v<version>/<wae|wae-lsp|wae-mcp>-<target>[.exe]`
- Every component is installed only after its adjacent SHA-256 file matches.
- Repository can be configured by:
  - `wae.githubRepo` in `package.json`
  - `WAE_GITHUB_REPOSITORY` environment variable (overrides config)
- GitHub Releases include an aggregate checksum manifest, keyless Sigstore bundle, SPDX SBOM and
  provenance attestations. See the repository README for verification commands.

## Maintainer note

- This package is preconfigured for `Don-Erfan/wae` releases.
- To use another repository, change `wae.githubRepo` or set `WAE_GITHUB_REPOSITORY`.
