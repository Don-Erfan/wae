# CI guide

WAE is built for CI: output is deterministic, exit codes are stable, and machine output is
versioned.

| Exit code | Meaning |
|---|---|
| `0` | passed (no fail-level violations) |
| `1` | violations found |
| `2` | configuration or project error (fix `wae.yaml` or the repository) |
| `3` | internal error (please report it) |
| `130` | cancelled (Ctrl+C) |

## GitHub Actions

```yaml
name: Architecture
on: [pull_request]
permissions:
  contents: read
jobs:
  wae:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0              # needed for --changed
      - uses: Don-Erfan/wae@v1.0.0
        with:
          version: 1.0.0
          changed: "true"             # fail only on violations introduced by this PR
          base: origin/${{ github.base_ref }}
```

Every active diagnostic becomes an inline annotation on the pull request (rule, message,
dependency path and suggestion), and the job summary shows totals, layer coverage, the
existing/introduced/fixed regression counts and a table of violations. This works on public and
private repositories without GitHub Advanced Security.

| Input | Default | Description |
|---|---|---|
| `version` | action release | Exact `@don-erfan/wae` version |
| `working-directory` | `.` | Directory containing `wae.yaml` |
| `changed` | `false` | Check only the Git affected closure against the committed baseline |
| `base` | | Git base ref for `changed` |
| `config` | | Alternate config path |
| `format` | `human` | Console format: `human`, `json`, `jsonl`, `sarif` |
| `annotations` | `true` | Inline pull-request annotations |
| `summary` | `true` | Job summary |
| `upload-sarif` | `false` | Also upload SARIF to code scanning (needs `security-events: write`) |
| `sarif-file` | `wae-results.sarif` | SARIF path |

Outputs: `exit-code`, `failure-count`, `warning-count`, `report-file` (the JSON report path).

To use GitHub code scanning instead of (or in addition to) annotations:

```yaml
permissions:
  contents: read
  security-events: write
steps:
  - uses: Don-Erfan/wae@v1.0.0
    with:
      version: 1.0.0
      upload-sarif: "true"
```

SARIF rules include the full rule documentation, which code scanning displays next to each alert.

## Ratchet mode (`--changed`)

1. `npx wae baseline` records current fail-level violations in `.wae/baseline.json`.
2. Commit the baseline with `wae.yaml`.
3. CI runs `wae check --changed --base <ref>`: only changed files and their importers are checked,
   and violations already in the baseline pass.

The base ref defaults to `WAE_BASE_REF`, then the merge base with the upstream/`origin/HEAD`.
Checkouts must include history (`fetch-depth: 0`). WAE never creates a baseline implicitly; an
expired baseline must be pruned with `wae baseline prune`.

## Other CI systems

Any CI that runs Node can use the npm package:

```bash
npx --yes @don-erfan/wae@1.0.0 check --format json > wae.json
```

- GitLab: `--format sarif` or JSON; fail the job on exit code `1`.
- Jenkins, Buildkite, CircleCI: rely on the exit code; archive `wae.json` as an artifact.
- Air-gapped runners: pre-provision the verified binaries and set `WAE_BINARY_DIR`.

Machine output (`json`, `jsonl`) carries `schemaVersion: 1` and follows
[`schemas/diagnostics.schema.json`](../../schemas/diagnostics.schema.json). Diagnostics carry a
stable `fingerprint` that does not change when code moves between lines, so dashboards can track
individual violations over time. Consumers must ignore unknown fields.
