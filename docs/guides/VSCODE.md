# VS Code guide

The WAE extension shows architecture violations while you type, explains them, and adds an
Architecture explorer to the activity bar. It is a thin client: all analysis runs in the shared
`wae-lsp` server, so results match the CLI and CI exactly.

## Install

1. Add WAE to the project: `npm install --save-dev @don-erfan/wae` (this provides `wae-lsp`).
2. Install the extension: download `wae-vscode.vsix` from the GitHub Release, then run
   **Extensions: Install from VSIX…** (or `code --install-extension wae-vscode.vsix`).
3. Open a JavaScript or TypeScript file. The server starts automatically.

The extension finds the server in this order: the `wae.server.path` setting, the workspace's
`node_modules/.bin/wae-lsp`, then `wae-lsp` on `PATH`.

## Features

- **Live diagnostics** for open, unsaved files (debounced, cancelled when you keep typing). The
  whole import specifier is underlined, and the rule code links to its documentation.
- **Dependency paths**: transitive violations list every hop in the Problems panel as related
  information; click a hop to open the import that causes it.
- **Quick fixes** (`Ctrl+.` / `Cmd+.`):
  - *Review suggested fix* shows WAE's remediation;
  - *Show dependency path* opens a picker to jump to any module on the path;
  - *Explain RULE* opens the rule's documentation (why, bad/good examples, configuration);
  - *Suppress … after documenting a reason* inserts `// wae-ignore RULE -- reason` after asking for
    a concrete reason.
- **Hover** on any line: package, layer, feature, runtime and *why* the module has that runtime;
  on a violating line, the rule, message, path and rationale.
- **Architecture explorer** (activity bar → WAE Architecture): Violations grouped by rule, and
  modules grouped by Packages, Layers, Features and Runtimes. It refreshes after every analysis.

## Commands

| Command | Action |
|---|---|
| WAE: Check Project | Re-analyze the whole workspace and open the Problems panel |
| WAE: Explain Rule... | Show documentation for a rule ID |
| WAE: Inspect Module | Dependencies, dependents, runtime explanation and diagnostics of the current file |
| WAE: Refresh Architecture Explorer | Reload the explorer tree |
| WAE: Restart Language Server | Restart `wae-lsp` (for example after upgrading WAE) |
| WAE: Print Dependency Graph (terminal) | Run `wae graph` in a terminal |

## Settings

| Setting | Default | Description |
|---|---|---|
| `wae.server.path` | `wae-lsp` | Explicit server executable; the default auto-detects `node_modules/.bin/wae-lsp` |

Configuration changes to `wae.yaml`, `package.json`, `tsconfig.json` and `jsconfig.json` are
picked up automatically. Multi-root workspaces are supported; each folder gets its own analysis.

## Troubleshooting

- *No diagnostics*: check **Output → Web Architecture Engine** for server errors and run
  `npx wae check` in the terminal; the editor shows exactly what the CLI reports.
- *Server not found*: install `@don-erfan/wae` in the workspace or set `wae.server.path`.
- See [Troubleshooting](../TROUBLESHOOTING.md) for resolution and configuration problems.
