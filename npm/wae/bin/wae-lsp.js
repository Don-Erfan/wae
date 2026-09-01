#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const binaryPath = path.join(__dirname, os.platform() === "win32" ? "wae-lsp.exe" : "wae-lsp");
if (!fs.existsSync(binaryPath)) {
  console.error("WAE language server was not found. Reinstall without --ignore-scripts or rebuild @don-erfan/wae.");
  process.exit(1);
}
const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`Failed to launch WAE language server: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
