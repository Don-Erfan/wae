# Getting started

This guide takes a JavaScript or TypeScript repository from zero to an enforced architecture in
about ten minutes. Every command shown is safe to run: nothing is written until you ask.

## 1. Install

WAE ships as a single npm package that installs native binaries for Linux x64/arm64, macOS
x64/arm64 and Windows x64.

```bash
npm install --save-dev @don-erfan/wae
# pnpm add -D @don-erfan/wae      yarn add -D @don-erfan/wae
npx wae --version
```

The package downloads the CLI (`wae`), language server (`wae-lsp`) and MCP server (`wae-mcp`) from
the matching GitHub Release and verifies each file against SHA-256 hashes embedded in the package.
If install scripts are disabled (pnpm 10 defaults, `--ignore-scripts`), the same verified download
happens on first use. Offline or air-gapped environments set `WAE_BINARY_DIR`; see
[Troubleshooting](TROUBLESHOOTING.md#native-binary-download-fails).

Other options:

- download a binary from the GitHub Release and verify it (see the [README](../README.md#verify-release-assets));
- build from source with `cargo build --release -p wae-cli -p wae-lsp -p wae-mcp` (binaries land in
  `target/release/`; the CLI binary is named `wae-cli`).

Requirements: Node.js 18+ for the npm launcher. WAE never executes your project's code.

## 2. See what WAE finds with zero configuration

```bash
npx wae check
```

Without a `wae.yaml`, WAE parses and resolves every module and reports only unambiguous problems:
runtime circular dependencies (`ARCH-001`), unresolved relative/aliased imports (`RESOLVE-001`) and
unparseable files (`PARSE-001`). Exit code `0` means clean, `1` means violations.

## 3. Describe your architecture

Ask WAE for an evidence-based proposal, review it, then write it:

```bash
npx wae discover            # read-only proposal with evidence and open questions
npx wae discover --write    # writes wae.yaml (refuses to overwrite)
```

Or start from a preset and edit it:

```bash
npx wae init --preset next  # also: blank (default), fsd, nx
```

A minimal layered Next.js configuration:

```yaml
version: 1
resolution:
  mode: bundler
architecture:
  layers:
    app:      { patterns: ["src/app/**"],      canImport: [features, entities, shared] }
    features: { patterns: ["src/features/**"], canImport: [entities, shared] }
    entities: { patterns: ["src/entities/**"], canImport: [shared] }
    shared:   { patterns: ["src/shared/**"],   canImport: [] }
  features:
    roots: ["src/features"]
```

Once a `wae.yaml` exists, every rule is enabled at `error` severity. Validate ownership before
enforcing it:

```bash
npx wae config validate --show-overlaps --show-coverage --show-unassigned
```

See the [configuration reference](CONFIGURATION.md) and the [rule reference](RULES.md).

## 4. Understand a violation

```bash
npx wae check
npx wae explain ARCH-004                      # why, bad/good example, fix, configuration
npx wae graph --module src/features/cart/ui.tsx   # dependencies, dependents, runtime and why
npx wae resolve src/app/page.tsx '@/features/cart' # how one import resolved
```

Transitive violations (for example a client component reaching server-only code) always include
the shortest dependency path, so you can see exactly which import to cut.

## 5. Adopt on an existing codebase

Large codebases rarely start clean. Record today's violations as accepted debt, commit the
baseline, and fail CI only on new violations:

```bash
npx wae baseline                 # writes .wae/baseline.json
git add wae.yaml .wae/baseline.json && git commit -m "Adopt WAE"
npx wae check --changed --base origin/main
```

`--changed` analyzes changed files plus everything that imports them and reports how many
violations are existing, introduced and fixed. Shrink the baseline over time with
`wae baseline prune`.

## 6. Enforce in CI

```yaml
- uses: actions/checkout@v4
  with:
    fetch-depth: 0
- uses: Don-Erfan/wae@v1.0.0
  with:
    version: 1.0.0
    changed: "true"
    base: origin/${{ github.base_ref }}
```

Violations appear as inline pull-request annotations with their dependency path, plus a job
summary. See the [CI guide](guides/CI.md) for other CI systems.

## 7. See violations while you type

- VS Code: [VS Code guide](guides/VSCODE.md)
- WebStorm / IntelliJ IDEA: [WebStorm guide](guides/WEBSTORM.md)
- AI agents: [MCP server](INTEGRATIONS.md#mcp-server)

Both editors use the same `wae-lsp` server as the CLI, so the editor, the CLI, CI and agents always
report the same violations.

## Next

- [Next.js guide](guides/NEXTJS.md): client/server/edge boundaries and Server Actions.
- [Monorepo guide](guides/MONOREPO.md): package graphs and workspace rules.
- [Troubleshooting](TROUBLESHOOTING.md)
