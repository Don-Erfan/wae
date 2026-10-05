# WebStorm and IntelliJ IDEA guide

The WAE plugin brings the same diagnostics as the CLI into JetBrains IDEs through the platform's
Language Server Protocol support. All analysis runs in the shared `wae-lsp` server; the Kotlin
plugin only starts it.

## Requirements

- WebStorm, IntelliJ IDEA Ultimate or another commercial JetBrains IDE, version 2024.2 or newer
  (the platform LSP API is required).
- `@don-erfan/wae` installed in the project (`npm install --save-dev @don-erfan/wae`), or `wae-lsp`
  on `PATH`.

## Install

1. Download `wae-jetbrains.zip` from the GitHub Release.
2. **Settings → Plugins → ⚙ → Install Plugin from Disk…**, choose the zip, restart the IDE.
3. Open a `.js`, `.jsx`, `.ts`, `.tsx`, `.mjs`, `.cjs`, `.mts` or `.cts` file.

The plugin starts the server from, in order: the `WAE_LSP_PATH` environment variable, the project's
`node_modules/.bin/wae-lsp`, then `wae-lsp` on `PATH`. IDEs launched from a desktop launcher may not
inherit your shell's `PATH`, which is why the project-local launcher is preferred.

## Features

- **Editor diagnostics** for open files, including unsaved changes. Transitive violations include
  the full dependency path in the message (`Path: a.tsx → b.ts → c.ts`).
- **Alt+Enter actions** on a violation:
  - *Review suggested fix for RULE* — WAE's remediation;
  - *Show dependency path for RULE* — the full path as a notification;
  - *Explain RULE* — summary, fix and a link to the full rule documentation;
  - *Insert RULE suppression template* — inserts `// wae-ignore RULE -- ` for you to complete with a
    concrete reason (WAE rejects suppressions without one).
- **Hover**: package, layer, feature, runtime and why the module has that runtime; on a violating
  line, the rule, message, dependency path and rationale.

Behavior is identical to VS Code because both editors consume the same server; the CI test suite
verifies the server against the same golden diagnostics as the CLI and MCP server.

## Architecture overview

JetBrains does not yet have a dedicated WAE tool window. Use either:

```bash
npx wae explore            # writes .wae/explorer.html: searchable modules, packages, layers, runtimes, violations
npx wae graph --module src/app/page.tsx
```

## Troubleshooting

- *No diagnostics*: check **Help → Show Log** for `wae-lsp` errors and run `npx wae check` in the
  terminal; the IDE shows exactly what the CLI reports.
- *Plugin incompatible*: the platform LSP API requires a 2024.2+ commercial IDE; Community editions
  are not supported.
- See [Troubleshooting](../TROUBLESHOOTING.md).
