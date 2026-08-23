#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const binaryName = os.platform() === "win32" ? "wae.exe" : "wae";
const binaryPath = path.join(__dirname, binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    "WAE binary was not found. Reinstall the package or run `npm rebuild @don-erfan/wae`."
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  console.error(`Failed to launch WAE binary: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status ?? 1);