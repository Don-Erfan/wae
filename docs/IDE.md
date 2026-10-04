# IDE integrations

All editor clients use the same `wae-lsp` binary and therefore share engine, configuration,
fingerprint and suppression behavior with the CLI. The server supports:

- full-project diagnostics on initialization, save, watched config changes and explicit reload;
- live diagnostics for open unsaved JS/TS documents through content-hashed overlays (the source
  tree is never modified and stale cache entries cannot match different disk content);
- architecture hover with package, layer, runtime and framework ownership;
- a preferred refactoring suggestion, an interactive rule-scoped suppression command in clients
  that implement it, and a portable standard `WorkspaceEdit` template whose blank reason remains
  invalid until the maintainer documents the exception;
- configuration reload without restarting the editor.

The server owns one long-lived `WorkspaceSession` per canonical workspace folder and routes a
document to the longest matching root. Folder add/remove notifications create or dispose sessions,
and removal clears previously published diagnostics. Document bursts are debounced for 75ms per
workspace, each analysis has a generation ID, starting newer work cancels the previous token,
analysis runs off the protocol event loop, and stale results are never published. Positions are
advertised and emitted as UTF-16 code units as required by the default LSP encoding. Suppressed
diagnostics are not published as active IDE errors.

Build the server with `cargo build -p wae-lsp --release`, or install `@don-erfan/wae` to receive
the checksum-verified `wae-lsp` sidecar. It communicates over stdio.

## VS Code

The extension lives in `editors/vscode`. Run `npm ci && npm run compile`, then package it with
`npx @vscode/vsce package`. Set `wae.server.path` when `wae-lsp` is not on `PATH`. Commands are
available for check, graph and language-server reload.

## JetBrains / WebStorm

The IntelliJ Platform plugin lives in `editors/jetbrains` and uses the platform LSP API, keeping
Kotlin code intentionally thin. Build with `gradle buildPlugin`. It starts `wae-lsp` for supported
JS/TS extensions. Set `WAE_LSP_PATH` when the binary is not on `PATH`.

CI uses JDK 21 and Gradle 8.10.2 to run both `buildPlugin` and JetBrains `verifyPlugin`; a Kotlin API
drift or incompatible plugin descriptor therefore blocks the aggregate readiness gate.
CI also executes a framed stdio LSP test and a real VS Code Extension Host test that validates
diagnostics, commands, and suppression `WorkspaceEdit` application. It packages installable
VSIX/JetBrains ZIP artifacts; release checksums and the keyless Sigstore bundle cover both editor
packages.

Both clients treat `wae-lsp` as the single source of diagnostics; neither reimplements rules or
resolution logic. VS Code supplies the richer interactive suppression-reason prompt. JetBrains
uses the platform's standard CodeAction/WorkspaceEdit path: choose the suppression-template quick
fix and complete the mandatory reason after `--`. `workspace/executeCommand` is also advertised
and handled by the server, so suggestion commands no longer become silent no-ops in generic LSP
clients. Plugin verification proves binary/API compatibility; the framed protocol tests prove the
shared actions and diagnostics contract.
