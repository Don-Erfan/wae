# Releasing WAE

## Preconditions

1. Protect `master` and require the `quality`, `tests`, `audit`, `npm-installer`, and
   `v1 readiness` CI jobs.
2. Configure npm Trusted Publishing for `@don-erfan/wae` only, repository `Don-Erfan/wae`, and
   workflow `release-binaries.yml`. Since 1.0.0 WAE publishes a single npm package; the former
   `@don-erfan/wae-<platform>` packages are no longer built or published.
3. Keep Cargo, npm, and tag versions identical.
4. Enable GitHub's immutable releases setting before publishing the first immutable release.

The complete readiness workflow runs before tagging and includes performance, the real Next.js
compatibility matrix, IDE, MCP, fuzz and installer gates. The release workflow resolves the signed
tag to its exact commit and refuses to build unless that commit already has successful `quality`,
`tests`, `audit`, `performance` and `v1 readiness` checks. This keeps failed gates from consuming an
immutable version number.

## Publish

```bash
git switch master
git pull --ff-only
# Wait for the exact HEAD commit's `v1 readiness` check to be green.
gh run list --commit "$(git rev-parse HEAD)" --workflow CI
git tag -s vX.Y.Z -m "WAE vX.Y.Z"
git push origin master vX.Y.Z
```

GitHub Actions builds the CLI, LSP and MCP server for every supported native target, verifies and publishes an aggregate
`SHA256SUMS` manifest, signs that manifest through keyless Sigstore, generates a separate SPDX
asset inventory and CycloneDX dependency SBOM from the repository/Cargo.lock,
and records GitHub SLSA build-provenance attestations for every binary. The curated section for the
version in `CHANGELOG.md` is prepended to GitHub's generated pull-request notes. npm publication
uses OIDC; no long-lived `NPM_TOKEN` or interactive OTP belongs in CI. The npm job verifies the
Sigstore-signed `SHA256SUMS`, embeds the 15 component/platform hashes into
`npm/wae/checksums.json`, checks that the tarball contains only the JavaScript launchers, publishes
`@don-erfan/wae`, and then installs the published version twice (with and without lifecycle
scripts) to prove both the `postinstall` download and the first-run download produce a working
`wae --version` and `wae-mcp`.

Verify a downloaded release exactly as a consumer should:

```bash
sha256sum --check SHA256SUMS
cosign verify-blob \
  --bundle SHA256SUMS.sigstore.json \
  --certificate-identity-regexp 'https://github\.com/Don-Erfan/wae/\.github/workflows/release-binaries\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  SHA256SUMS
gh attestation verify wae-x86_64-unknown-linux-gnu --repo Don-Erfan/wae
```

Both `wae-vX-assets.spdx.json` and `wae-vX-dependencies.cdx.json` are listed in the signed checksum
manifest, binding the release inventory and dependency tree to the same workflow identity.

## Recovery

- Never move a published tag. A tag counts as published once its run has created a GitHub Release
  or an npm version. A tag whose release run stopped before either (for example `v1.0.0`, first
  pushed on the pre-release commit and stopped at release-notes extraction) may be deleted and
  re-created, signed, on the correct commit after that commit's readiness checks are green.
- Editor packages are attached to the GitHub Release; publishing them to the VS Code Marketplace or
  JetBrains Marketplace is a separate, manual step with the maintainer's publisher accounts.
- If every GitHub Release asset was published successfully but the downstream npm job failed,
  fix the release workflow on `master`, wait for CI, and recover only npm from the existing signed
  assets with:

  ```bash
  gh workflow run release-binaries.yml -f tag=vX.Y.Z -f npm_only=true
  ```

  This mode verifies the tag's readiness checks, the Sigstore-signed `SHA256SUMS`, and every asset
  checksum. It skips npm package versions that are already immutable, so a partially completed npm
  publish can be resumed safely. It does not run builds or update the GitHub Release.
- For a source, binary, metadata, or already-published package defect, fix the source, bump the
  version, and create a new tag.
- npm versions and release assets are immutable release records.
- If immutable releases are enabled, never attempt to edit assets or notes after publication;
  publish every correction under a new version and signed tag.
