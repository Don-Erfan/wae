# WAE migration guide

## Configuration composition and governance

Existing version-1 files remain valid. Teams may extract shared policy into relative parent files
without changing the resolved rule model:

```yaml
extends: config/company.yaml
version: 1
```

Mappings are deep-merged, while arrays and scalar values are replaced by the child. Put local
exceptions in the leaf file if the local team owns their lifecycle. `wae suppressions prune`
removes only expired leaf entries and preserves source formatting. When a parent owns an expired
entry, the command reports that parent and leaves it unchanged for an explicit owner review.

Path-specific rollout belongs in ordered `overrides`. Later matches win:

```yaml
overrides:
  - files: ["src/legacy/**"]
    rules:
      ARCH-003: warning
```

Config suppressions can add `owner`, `ticket`, and ISO `expires_at`. These fields now accompany the
diagnostic in human, JSON, JSONL, and SARIF output. `wae suppressions prune` intentionally leaves
expired inherited entries untouched and reports their defining file; edit or prune that source
document explicitly rather than flattening it into a child. Consumers should ignore unknown
diagnostic metadata keys as required by the compatibility policy.

## Upgrading to 1.0

### npm: one package

`@don-erfan/wae` is now the only npm package. It no longer depends on the
`@don-erfan/wae-<platform>` optional packages; it downloads and verifies the native binaries for
the current platform itself (at `postinstall`, or on first use when install scripts are disabled).

- Run `npm install --save-dev @don-erfan/wae@1` and refresh the lockfile. Remove any
  `@don-erfan/wae-linux-x64` (or other platform) entries you added explicitly.
- Private registries only need to mirror `@don-erfan/wae`; runners need HTTPS access to
  `github.com` release assets, or set `WAE_BINARY_DIR` to pre-provisioned, verified binaries.
- `npm run recover:binaries` was removed; running any `wae` command repairs a missing binary.

### ARCH-001 ignores type-only cycles by default

Cycles that exist only through `import type` / `export type` are no longer reported, because
TypeScript erases those imports. A component that previously hid several runtime cycles behind one
type-only path now reports each runtime cycle separately. In changed mode, previously baselined
type-only cycles count as fixed. To keep the old behavior:

```yaml
rules:
  ARCH-001:
    include_type_only: true
```

Cycles in components without type-only edges keep their fingerprints. Components that were
joined only by a type-only edge are now reported as separate runtime cycles, which can carry new
fingerprints; if you use `check --changed`, run `wae baseline` once after upgrading and commit the
refreshed `.wae/baseline.json`.

### RUNTIME-005 requires a declared browser boundary

A universal module is reported only when its closure reaches a declared browser module
(`"use client"`, `client-only`, explicit runtime) and server/Node code. Modules that are bundled for
the browser merely because some other client component imports them no longer make their universal
importers ambiguous; that client leak is still reported once as `RUNTIME-001`.

### GitHub Action defaults

- `format` now defaults to `human` (console output); annotations and the job summary are produced
  from a separate JSON run and are enabled by default.
- `upload-sarif` now defaults to `false`, so private repositories without code scanning work out of
  the box. Set `upload-sarif: "true"` and grant `security-events: write` to keep SARIF uploads.
- New inputs: `working-directory`, `annotations`, `summary`. New outputs: `exit-code`,
  `failure-count`, `warning-count`, `report-file`.

### New, backward-compatible additions

- Rule settings accept `off`; `ARCH-001` accepts `include_type_only`.
- `resolution.virtual_modules` declares build-generated or bundler-virtual specifiers.
- CLI: `wae <command> --help`, `wae rules`, a full `wae explain`, `wae graph --module`, and bare
  `wae baseline` (same as `wae baseline create`).
- LSP: dependency paths as related information, rule documentation links, and the
  `wae.explainRule`, `wae.showDependencyPath`, `wae.inspectModule`, `wae.architectureOverview` and
  `wae.reanalyze` commands.

Config files (`version: 1`), JSON output (`schemaVersion: 1`) and baselines remain compatible;
no file needs to be rewritten.
