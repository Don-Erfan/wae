# Troubleshooting

Start with `npx wae doctor`: it checks the project root, configuration, layer overlaps and Git
availability and prints the next command to run. Add `--verbose` to `wae check` for phase timings
and cache statistics on stderr.

## Native binary download fails

The npm package downloads `wae`, `wae-lsp` and `wae-mcp` from the GitHub Release matching its
version and verifies them with SHA-256. Messages are printed to stderr.

- **Behind a proxy or firewall**: allow `github.com` and `*.githubusercontent.com`, or pre-provision
  the binaries.
- **Air-gapped / offline**: download the three release assets for your platform, verify them with
  `sha256sum --check SHA256SUMS` and `cosign verify-blob` (see the [README](../README.md#verify-release-assets)),
  rename them to `wae`, `wae-lsp` and `wae-mcp` (`.exe` on Windows), and set
  `WAE_BINARY_DIR=/path/to/dir`. WAE then never downloads.
- **Read-only `node_modules`** (Nix, Docker layers): binaries go to the per-user cache instead;
  override it with `WAE_CACHE_DIR`.
- **`SHA-256 verification failed`**: the download was corrupted or tampered with. Nothing was
  installed; retry, and report it if it persists.
- **Install scripts disabled** (pnpm 10, `--ignore-scripts`): expected. The first `wae` run performs
  the same verified download.

## `RESOLVE-001: Cannot resolve …`

Trace the resolver:

```bash
npx wae resolve src/app/page.tsx '@/components/button'
```

The trace lists every handler (relative, tsconfig alias, workspace, package), active conditions and
candidate paths.

- Aliases come from the nearest `tsconfig.json`/`jsconfig.json` `paths`. Bundler-only aliases
  (webpack/Vite `resolve.alias`) must be mirrored there.
- Next.js and most bundler projects should use `resolution.mode: bundler`. In `node16`/`nodenext`
  mode, relative ESM imports need explicit extensions (`./user.js`).
- Build-generated modules (for example `contentlayer/generated`) do not exist before the build:
  run WAE after generation, or list them in `resolution.virtual_modules`.

## "module matches multiple architecture layers"

Two layer patterns overlap. Run `npx wae config validate --show-overlaps`; anchor broad patterns
such as `**/shared/**` to `src/shared/**`.

## A module has an unexpected runtime

```bash
npx wae graph --module src/lib/format.ts
```

The output names the evidence (`directive`, `marker-package`, `explicit`, `convention`,
`propagated`, `default`) and, for propagated browser modules, the import path from the
`"use client"` file. Fix the marker (`import "server-only"`, `"use client"`) rather than
suppressing the rule. See the [Next.js guide](guides/NEXTJS.md).

## `check --changed` fails with "baseline is missing" or "expired"

Changed mode never creates state. Run `npx wae baseline`, commit `.wae/baseline.json`, and run
`npx wae baseline prune` when entries expire. In CI, check out full history (`fetch-depth: 0`) and
pass `--base origin/<branch>` or set `WAE_BASE_REF`.

## Too many violations on an existing project

1. Start with no layers and only the defaults; fix real cycles first.
2. Add layers with `wae discover`, then set noisy rules to `warning` or use `overrides` per path.
3. Record a baseline and gate CI with `--changed`.
4. Suppress a deliberate exception with a reason: `// wae-ignore ARCH-003 -- ADR-12 legacy adapter`.

If a diagnostic is wrong (not just unwanted), please open an issue with the output of
`wae graph --module <file>` and `wae resolve` for the import involved.

## Editor shows nothing

Run `npx wae check` in the same folder first: editors show exactly what the CLI reports. Then see
the [VS Code](guides/VSCODE.md#troubleshooting) or [WebStorm](guides/WEBSTORM.md#troubleshooting)
guide. **WAE: Restart Language Server** reloads after upgrading.

## Stale or inconsistent results

The cache is content-addressed and safe to delete. Run with `--no-cache` to bypass it, or remove
`.wae/cache/`. Results are deterministic: two runs on the same tree produce identical output.

## Exit code 3 (internal error)

This is a bug. Please report it with the command, `wae --version`, the stderr output and, if
possible, a minimal reproduction.
