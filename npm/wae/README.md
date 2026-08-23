# @don-erfan/wae

NPM wrapper for the WAE CLI.

## Install

```bash
yarn add -D @don-erfan/wae
```

## Usage

```bash
wae help
wae init
wae scan
wae check
wae check --changed
```

## How it works

- During installation, `postinstall` downloads the platform binary from GitHub Releases.
- Binary assets are resolved from:
  - `https://github.com/<repo>/releases/download/v<version>/wae-<target>[.exe]`
- Repository can be configured by:
  - `wae.githubRepo` in `package.json`
  - `WAE_GITHUB_REPOSITORY` environment variable (overrides config)

## Maintainer note

- This package is preconfigured for `Don-Erfan/wae` releases.
- To use another repository, change `wae.githubRepo` or set `WAE_GITHUB_REPOSITORY`.