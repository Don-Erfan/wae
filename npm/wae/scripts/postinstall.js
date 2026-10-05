#!/usr/bin/env node
"use strict";

// Pre-installs the verified native binaries. Failure is deliberately non-fatal: the launcher
// retries the identical verified download on first use and reports an actionable error there, so
// an offline `npm ci` of an unrelated project is never broken by this optional step.

const fs = require("node:fs");
const path = require("node:path");
const { installAll } = require("../lib/native.js");

const packageRoot = path.join(__dirname, "..");

function isSourceCheckout() {
  const checksums = JSON.parse(fs.readFileSync(path.join(packageRoot, "checksums.json"), "utf8"));
  return (
    Object.keys(checksums).length === 0 &&
    fs.existsSync(path.join(packageRoot, "..", "..", "Cargo.toml"))
  );
}

async function main(env = process.env) {
  if (env.WAE_SKIP_DOWNLOAD === "1") {
    console.error("Skipping WAE native binary download because WAE_SKIP_DOWNLOAD=1.");
    return;
  }
  if (env.WAE_BINARY_DIR) {
    console.error(`Using pre-provisioned WAE binaries from WAE_BINARY_DIR=${env.WAE_BINARY_DIR}.`);
    return;
  }
  if (isSourceCheckout()) {
    console.error("Skipping WAE native binary download inside the WAE source checkout.");
    return;
  }
  try {
    await installAll();
  } catch (error) {
    console.error(
      `WAE could not pre-install its native binaries: ${error.message}\n` +
        "They will be downloaded and verified on first use. For offline installs, set " +
        "WAE_BINARY_DIR to a directory containing the verified release binaries."
    );
  }
}

if (require.main === module) main();

module.exports = { main };
