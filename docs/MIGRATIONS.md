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

## npm native package layout

The portable `@don-erfan/wae` wrapper now uses exact-version optional native packages. Remove any
previous binary copied into `node_modules/@don-erfan/wae/bin`, refresh the lockfile, and reinstall.
Lifecycle scripts are not required. Private mirrors and air-gapped caches must mirror the wrapper
and the one native package for each supported deployment platform.

The old verified downloader remains an explicit `recover:binaries` command for emergency recovery;
it is no longer a `postinstall` hook.
