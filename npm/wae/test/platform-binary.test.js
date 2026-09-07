const assert = require("node:assert/strict");
const fs = require("node:fs");
const Module = require("node:module");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { packageForPlatform, resolveBinary } = require("../bin/platform-binary.js");

test("maps every supported npm platform to a stable optional package", () => {
  assert.equal(packageForPlatform("linux", "x64"), "@don-erfan/wae-linux-x64");
  assert.equal(packageForPlatform("linux", "arm64"), "@don-erfan/wae-linux-arm64");
  assert.equal(packageForPlatform("darwin", "x64"), "@don-erfan/wae-darwin-x64");
  assert.equal(packageForPlatform("darwin", "arm64"), "@don-erfan/wae-darwin-arm64");
  assert.equal(packageForPlatform("win32", "x64"), "@don-erfan/wae-win32-x64");
  assert.throws(() => packageForPlatform("freebsd", "x64"), /Unsupported platform/);
});

test("wrapper and native manifests stay exact-versioned and script-free", () => {
  const wrapper = require("../package.json");
  assert.equal(wrapper.scripts.postinstall, undefined);
  assert.deepEqual(
    Object.keys(wrapper.optionalDependencies).sort(),
    Object.values(require("../bin/platform-binary.js").PLATFORM_PACKAGES).sort()
  );
  for (const [platformKey, packageName] of Object.entries(
    require("../bin/platform-binary.js").PLATFORM_PACKAGES
  )) {
    const manifest = require(path.join("..", "..", "platform", platformKey, "package.json"));
    assert.equal(manifest.name, packageName);
    assert.equal(manifest.version, wrapper.version);
    assert.equal(wrapper.optionalDependencies[packageName], wrapper.version);
    assert.equal(manifest.preferUnplugged, true);
    assert.equal(manifest.scripts, undefined);
  }
});

test("resolves a binary from a script-free optional platform package", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-platform-package-"));
  const binary = path.join(root, "@don-erfan", "wae-linux-x64", "bin", "wae");
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(binary, "native-binary");
  const previous = process.env.NODE_PATH;
  process.env.NODE_PATH = previous ? `${root}${path.delimiter}${previous}` : root;
  Module._initPaths();
  try {
    assert.equal(resolveBinary("wae", { platform: "linux", arch: "x64" }), binary);
  } finally {
    if (previous === undefined) delete process.env.NODE_PATH;
    else process.env.NODE_PATH = previous;
    Module._initPaths();
    fs.rmSync(root, { recursive: true, force: true });
  }
});
