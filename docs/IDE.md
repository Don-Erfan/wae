# IDE integrations

All editor clients use the same `wae-lsp` binary and therefore share engine, configuration,
fingerprint and suppression behavior with the CLI. The server supports:

- full-project diagnostics on initialization, save, watched config changes and explicit reload;
- live diagnostics for open unsaved JS/TS documents through in-memory overlays (the source tree and
  persistent cache are never modified by editor buffers);
- architecture hover with package, layer, runtime and framework ownership;
- rule-scoped quick actions exposing the diagnostic suggestion;
- configuration reload without restarting the editor.

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

Both clients treat `wae-lsp` as the single source of diagnostics; neither reimplements rules or
resolution logic.
