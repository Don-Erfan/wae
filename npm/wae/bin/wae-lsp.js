#!/usr/bin/env node

const { spawnSync } = require("node:child_process");
const { resolveBinary } = require("./platform-binary.js");

let binaryPath;
try {
  binaryPath = resolveBinary("wae-lsp");
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`Failed to launch WAE language server: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
