# Web Architecture Engine for VS Code

Live architecture diagnostics for JavaScript and TypeScript: layer, feature, package and Next.js
runtime boundary violations, each with the dependency path that causes it.

- Diagnostics while you type, with the full dependency path as clickable related information and
  the rule code linked to its documentation.
- Quick fixes: review the suggested fix, show the dependency path, explain the rule, or suppress
  with a documented reason.
- Hover: package, layer, feature, runtime and why the module has that runtime.
- **WAE Architecture** explorer in the activity bar: violations by rule, modules by package, layer,
  feature and runtime.
- Commands: *WAE: Check Project*, *WAE: Explain Rule...*, *WAE: Inspect Module*.

## Requirements

Install `@don-erfan/wae` in the workspace (`npm install --save-dev @don-erfan/wae`). The extension
uses `node_modules/.bin/wae-lsp` automatically; set `wae.server.path` to use another executable.

All analysis runs in the shared `wae-lsp` server, so results match `npx wae check` and CI exactly.
See the [VS Code guide](https://github.com/Don-Erfan/wae/blob/master/docs/guides/VSCODE.md).
