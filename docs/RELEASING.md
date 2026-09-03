# Releasing WAE

## Preconditions

1. Protect `master` and require the `quality`, `tests`, `audit`, `npm-installer`, and
   `v1 readiness` CI jobs.
2. Configure npm Trusted Publishing for package `@don-erfan/wae`, repository `Don-Erfan/wae`, and workflow `release-binaries.yml`.
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
uses OIDC; no long-lived `NPM_TOKEN` or interactive OTP belongs in CI.

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

- Never move a published tag.
- Fix the source, bump the version, and create a new tag.
- npm versions and release assets are immutable release records.
- If immutable releases are enabled, never attempt to edit assets or notes after publication;
  publish every correction under a new version and signed tag.
