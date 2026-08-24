# Releasing WAE

## Preconditions

1. Protect `master` and require the `quality`, `tests`, `audit`, and `npm-installer` CI jobs.
2. Configure npm Trusted Publishing for package `@don-erfan/wae`, repository `Don-Erfan/wae`, and workflow `release-binaries.yml`.
3. Keep Cargo, npm, and tag versions identical.

The release workflow repeats formatting, Clippy, the complete test suite, dependency audit, installer tests, and package inspection before any platform binary is built. A failing verification job prevents GitHub Release and npm publication.

## Publish

```bash
git switch master
git pull --ff-only
git tag -s vX.Y.Z -m "WAE vX.Y.Z"
git push origin master vX.Y.Z
```

GitHub Actions builds every supported native target, publishes checksums with the binaries, creates the GitHub Release, and publishes to npm through OIDC. No long-lived `NPM_TOKEN` or interactive OTP belongs in CI.

## Recovery

- Never move a published tag.
- Fix the source, bump the version, and create a new tag.
- npm versions and release assets are immutable release records.
