const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const packageRoot = path.join(__dirname, "..");
const repositoryRoot = path.join(packageRoot, "..", "..");
const manifest = require("../package.json");

test("the npm wrapper is a single self-contained package", () => {
  assert.equal(manifest.name, "@don-erfan/wae");
  assert.equal(manifest.optionalDependencies, undefined);
  assert.equal(manifest.dependencies, undefined);
  assert.equal(manifest.scripts.postinstall, "node scripts/postinstall.js");
  assert.deepEqual(Object.keys(manifest.bin).sort(), ["wae", "wae-lsp", "wae-mcp"]);
  for (const entry of ["bin", "lib", "scripts", "checksums.json"]) {
    assert(manifest.files.includes(entry), `${entry} must be published`);
  }
  assert.equal(fs.existsSync(path.join(repositoryRoot, "npm", "platform")), false);
});

test("every published version reference stays synchronized", () => {
  const cargo = fs.readFileSync(path.join(repositoryRoot, "Cargo.toml"), "utf8");
  assert.equal(cargo.match(/^version = "([^"]+)"/m)[1], manifest.version);
  const action = fs.readFileSync(path.join(repositoryRoot, "action.yml"), "utf8");
  const versionInput = action.match(
    /inputs:\s*[\s\S]*?version:\s*[\s\S]*?default:\s*["']([^"']+)["']/
  );
  assert.ok(versionInput, "action.yml must declare an exact default engine version");
  assert.equal(versionInput[1], manifest.version);
  const vscode = require(path.join(repositoryRoot, "editors", "vscode", "package.json"));
  assert.equal(vscode.version, manifest.version);
  const jetbrains = fs.readFileSync(
    path.join(repositoryRoot, "editors", "jetbrains", "build.gradle.kts"),
    "utf8"
  );
  assert.equal(jetbrains.match(/^version = "([^"]+)"/m)[1], manifest.version);
});

test("postinstall never downloads inside the source checkout or when disabled", async () => {
  const { main } = require("../scripts/postinstall.js");
  const messages = [];
  const original = console.error;
  console.error = (message) => messages.push(String(message));
  try {
    await main({ WAE_SKIP_DOWNLOAD: "1" });
    await main({ WAE_BINARY_DIR: "/opt/wae" });
    await main({});
  } finally {
    console.error = original;
  }
  assert.match(messages[0], /WAE_SKIP_DOWNLOAD=1/);
  assert.match(messages[1], /WAE_BINARY_DIR=\/opt\/wae/);
  assert.match(messages[2], /source checkout/);
});

test(
  "launchers forward arguments, stdout and exit status without writing to stdout",
  { skip: process.platform === "win32" },
  (t) => {
    const provisioned = fs.mkdtempSync(path.join(os.tmpdir(), "wae-launcher-"));
    t.after(() => fs.rmSync(provisioned, { recursive: true, force: true }));
    for (const component of ["wae", "wae-lsp", "wae-mcp"]) {
      const fake = path.join(provisioned, component);
      fs.writeFileSync(
        fake,
        `#!/usr/bin/env node\nprocess.stdout.write(${JSON.stringify(component)} + ":" + process.argv.slice(2).join(","));\nprocess.exit(7);\n`
      );
      fs.chmodSync(fake, 0o755);
      const result = spawnSync(
        process.execPath,
        [path.join(packageRoot, "bin", `${component}.js`), "check", "--format", "json"],
        { env: { ...process.env, WAE_BINARY_DIR: provisioned }, encoding: "utf8" }
      );
      assert.equal(result.status, 7, result.stderr);
      assert.equal(result.stdout, `${component}:check,--format,json`);
      assert.equal(result.stderr, "");
    }
  }
);

test("a missing binary produces an actionable stderr error and exit code 1", () => {
  const empty = fs.mkdtempSync(path.join(os.tmpdir(), "wae-launcher-empty-"));
  try {
    const result = spawnSync(process.execPath, [path.join(packageRoot, "bin", "wae.js")], {
      env: { ...process.env, WAE_BINARY_DIR: empty },
      encoding: "utf8",
    });
    assert.equal(result.status, 1);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /Could not prepare the WAE CLI: WAE_BINARY_DIR is set/);
  } finally {
    fs.rmSync(empty, { recursive: true, force: true });
  }
});
