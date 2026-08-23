# Web Architecture Engine (WAE)

Workspace  `WebLint / ArchLint`.

## Run from source

```bash
cargo run -p wae-cli
```

## Install as npm package (frontend teams)

The npm wrapper lives at `npm/wae` and exposes a `wae` binary for frontend projects.

### Frontend developer usage

```bash
yarn add -D @don-erfan/wae
```

Add scripts to your frontend `package.json`:

```json
{
  "scripts": {
    "arch:init": "wae init",
    "arch:scan": "wae scan",
    "arch:check": "wae check",
    "arch:changed": "wae check --changed"
  }
}
```

Run checks:

```bash
yarn arch:init
yarn arch:check
```

### Maintainer release flow

1. Ensure repository settings/secrets are configured:
   - GitHub repo: `https://github.com/Don-Erfan/wae`
   - npm token in repository secret: `NPM_TOKEN`
2. Create and push a tag like `v0.1.0`.
3. `release-binaries.yml` builds Linux/macOS/Windows binaries and uploads them to the GitHub Release.
4. `publish-npm.yml` publishes `npm/wae` to npmjs.com using the same tag version.

## CI checks

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`