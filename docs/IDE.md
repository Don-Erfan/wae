# IDE integrations

User guides: [VS Code](guides/VSCODE.md) · [WebStorm / IntelliJ IDEA](guides/WEBSTORM.md).

All editor clients use the same `wae-lsp` binary and therefore share engine, configuration,
fingerprint and suppression behavior with the CLI. Neither client reimplements rules, resolution
or architecture classification. The server supports:

- full-project diagnostics on initialization, save, watched config changes and explicit reload;
- live diagnostics for open unsaved JS/TS documents through content-hashed overlays (the source
  tree is never modified and stale cache entries cannot match different disk content);
- diagnostics whose range covers the whole import specifier, a `codeDescription` link to the rule
  documentation, the dependency path as `relatedInformation` (one clickable entry per import on the
  path) and, for paths longer than one edge, in the message text for clients that do not render
  related information;
- hover with package, layer, feature, runtime and the reason for that runtime (including the
  browser propagation path), plus rule, path and rationale for violations on the hovered line;
- code actions: review suggested fix, show dependency path, explain rule, interactive documented
  suppression (VS Code) and a portable standard `WorkspaceEdit` suppression template whose blank
  reason remains invalid until the maintainer documents the exception;
- configuration reload without restarting the editor.

## Server commands

`workspace/executeCommand` is advertised and handled by the server, so actions work in any LSP
client. Clients with richer UI pass `"notify": false` and render the result themselves; generic
clients receive a `window/showMessage` summary.

| Command | Arguments | Result |
|---|---|---|
| `wae.explainRule` | `{ ruleId, notify? }` | `{ ruleId, title, markdown, helpUri }` |
| `wae.showDependencyPath` | `{ ruleId, path, notify? }` | `{ path }` |
| `wae.inspectModule` | `{ uri }` | module inspection: package, layer, feature, runtime explanation, dependencies, dependents, diagnostics |
| `wae.architectureOverview` | none | `{ workspaces: [{ root, ready, overview }] }` with modules grouped by package, layer, feature and runtime, and active violations grouped by rule |
| `wae.reanalyze` | none | schedules a forced full analysis of every workspace |
| `wae.showSuggestion` | `{ suggestion }` | shows the remediation |
| `wae.suppressWithReason` | `{ uri, line, ruleId, reason? }` | client-side in VS Code; generic clients are pointed to the template action |

Inspection and overview are rendered from `wae_engine::projection`, the same read-only projection
the CLI uses for `wae graph --module`, so editors and the CLI explain modules identically.

## Session model

The server owns one long-lived `WorkspaceSession` per canonical workspace folder and routes a
document to the longest matching root. Folder add/remove notifications create or dispose sessions,
and removal clears previously published diagnostics. Document bursts are debounced for 75ms per
workspace, each analysis has a generation ID, starting newer work cancels the previous token,
analysis runs off the protocol event loop, and stale results are never published. Positions are
advertised and emitted as UTF-16 code units as required by the default LSP encoding. Suppressed
diagnostics are not published as active IDE errors.

Build the server with `cargo build -p wae-lsp --release`, or install `@don-erfan/wae` to receive
the checksum-verified `wae-lsp` executable. It communicates over stdio.

## Clients

- `editors/vscode`: `npm ci && npm run compile`, package with `npx @vscode/vsce package`. Adds the
  Architecture explorer tree view, markdown rule explanations and a dependency-path picker on top
  of the server commands. Server discovery: `wae.server.path`, then
  `node_modules/.bin/wae-lsp`, then `PATH`.
- `editors/jetbrains`: IntelliJ Platform plugin using the platform LSP API; build with
  `gradle buildPlugin`. Server discovery: `WAE_LSP_PATH`, then `node_modules/.bin/wae-lsp`, then
  `PATH`.

## Verification

CI uses JDK 21 and Gradle 8.10.2 to run both `buildPlugin` and JetBrains `verifyPlugin`; a Kotlin
API drift or incompatible plugin descriptor blocks the aggregate readiness gate. CI also executes
framed stdio LSP tests (including the synthetic-app golden diagnostics shared with the CLI and MCP
tests) and a real VS Code Extension Host test that validates diagnostics, related dependency-path
information, rule explanation, module inspection, the architecture overview, commands and
suppression `WorkspaceEdit` application. It packages installable VSIX/JetBrains ZIP artifacts;
release checksums and the keyless Sigstore bundle cover both editor packages.
