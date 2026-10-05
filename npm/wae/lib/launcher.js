"use strict";

const { spawnSync } = require("node:child_process");
const { ensureBinary } = require("./native.js");

const LABELS = Object.freeze({
  wae: "WAE CLI",
  "wae-lsp": "WAE language server",
  "wae-mcp": "WAE MCP server",
});

/**
 * Runs a native WAE component with inherited stdio and mirrors its exit status. Diagnostics from
 * the launcher itself are written to stderr only, keeping LSP/MCP stdout protocol-clean.
 */
async function launch(component, args = process.argv.slice(2), options = {}) {
  const label = LABELS[component] || component;
  let binaryPath;
  try {
    binaryPath = await ensureBinary(component, options);
  } catch (error) {
    console.error(`Could not prepare the ${label}: ${error.message}`);
    return 1;
  }
  const result = spawnSync(binaryPath, args, { stdio: "inherit", windowsHide: true });
  if (result.error) {
    console.error(`Failed to launch the ${label}: ${result.error.message}`);
    return 1;
  }
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return 1;
  }
  return result.status ?? 1;
}

function main(component) {
  launch(component).then(
    (code) => {
      process.exitCode = code;
    },
    (error) => {
      console.error(error && error.stack ? error.stack : String(error));
      process.exitCode = 1;
    }
  );
}

module.exports = { LABELS, launch, main };
