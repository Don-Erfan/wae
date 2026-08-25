# Releasing WAE

## Preconditions

1. Protect `master` and require the `quality`, `tests`, `audit`, and `npm-installer` CI jobs.
2. Configure npm Trusted Publishing for package `@don-erfan/wae`, repository `Don-Erfan/wae`, and workflow `release-binaries.yml`.
3. Keep Cargo, npm, and tag versions identical.
4. Enable GitHub's immutable releases setting before publishing the first immutable release.

The release workflow repeats formatting, Clippy, the complete test suite, dependency audit, installer tests, and package inspection before any platform binary is built. A failing verification job prevents GitHub Release and npm publication.

## Publish

```bash
git switch master
git pull --ff-only
git tag -s vX.Y.Z -m "WAE vX.Y.Z"
git push origin master vX.Y.Z
```

GitHub Actions builds every supported native target, verifies and publishes an aggregate
`SHA256SUMS` manifest, signs that manifest through keyless Sigstore, generates an SPDX JSON SBOM,
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

Because the SPDX JSON file is listed in the signed checksum manifest, its verified digest binds
the dependency inventory to the same workflow identity as the binaries.

## Recovery

- Never move a published tag.
- Fix the source, bump the version, and create a new tag.
- npm versions and release assets are immutable release records.
- If immutable releases are enabled, never attempt to edit assets or notes after publication;
  publish every correction under a new version and signed tag.
