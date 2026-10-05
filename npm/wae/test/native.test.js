const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const { EventEmitter } = require("node:events");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { PassThrough } = require("node:stream");
const test = require("node:test");

const {
  assetName,
  cacheDirectory,
  downloadFile,
  ensureBinary,
  findBinary,
  installAll,
  installVerifiedBinary,
  resolveTarget,
  validateDownloadUrl,
} = require("../lib/native.js");

function temporaryPackage(t, { version = "1.2.3", checksums = {} } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-npm-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const packageRoot = path.join(root, "package");
  fs.mkdirSync(packageRoot);
  fs.writeFileSync(
    path.join(packageRoot, "package.json"),
    JSON.stringify({ version, wae: { githubRepo: "owner/repo" } })
  );
  fs.writeFileSync(path.join(packageRoot, "checksums.json"), JSON.stringify(checksums));
  return { root, packageRoot, cacheDir: path.join(root, "cache") };
}

function payloadFor(url) {
  const assetUrl = url.endsWith(".sha256") ? url.slice(0, -".sha256".length) : url;
  return `binary:${path.basename(assetUrl)}`;
}

function releaseDownload(requested = []) {
  return async (url, destination) => {
    requested.push(url);
    const payload = payloadFor(url);
    fs.writeFileSync(
      destination,
      url.endsWith(".sha256") ? crypto.createHash("sha256").update(payload).digest("hex") : payload
    );
  };
}

function fakeClient(handler) {
  return {
    get(url, _options, callback) {
      const request = new EventEmitter();
      request.setTimeout = () => {};
      request.destroy = (error) => request.emit("error", error);
      process.nextTick(() => {
        const response = new PassThrough();
        response.statusCode = 200;
        response.headers = {};
        const start = handler(response, url);
        callback(response);
        process.nextTick(() => start && start());
      });
      return request;
    },
  };
}

const quiet = () => {};

test("maps every supported platform to its Rust release target", () => {
  assert.equal(resolveTarget("linux", "x64"), "x86_64-unknown-linux-gnu");
  assert.equal(resolveTarget("linux", "arm64"), "aarch64-unknown-linux-gnu");
  assert.equal(resolveTarget("darwin", "x64"), "x86_64-apple-darwin");
  assert.equal(resolveTarget("darwin", "arm64"), "aarch64-apple-darwin");
  assert.equal(resolveTarget("win32", "x64"), "x86_64-pc-windows-msvc");
  assert.throws(() => resolveTarget("freebsd", "x64"), /Unsupported platform/);
  assert.equal(assetName("wae-lsp", "win32", "x64"), "wae-lsp-x86_64-pc-windows-msvc.exe");
  assert.equal(assetName("wae", "linux", "arm64"), "wae-aarch64-unknown-linux-gnu");
});

test("uses conventional per-user cache directories and honors WAE_CACHE_DIR", () => {
  const home = path.join(path.sep, "home", "dev");
  assert.equal(cacheDirectory({}, "linux", home), path.join(home, ".cache", "wae"));
  assert.equal(
    cacheDirectory({ XDG_CACHE_HOME: path.join(path.sep, "xdg") }, "linux", home),
    path.join(path.sep, "xdg", "wae")
  );
  assert.equal(cacheDirectory({}, "darwin", home), path.join(home, "Library", "Caches", "wae"));
  assert.equal(
    cacheDirectory({ LOCALAPPDATA: path.join(path.sep, "local") }, "win32", home),
    path.join(path.sep, "local", "wae", "Cache")
  );
  assert.equal(cacheDirectory({ WAE_CACHE_DIR: "custom" }, "linux", home), path.resolve("custom"));
});

test("first use installs a verified binary into the package vendor directory", async (t) => {
  const { packageRoot, cacheDir } = temporaryPackage(t);
  const requested = [];
  const options = {
    packageRoot,
    cacheDir,
    env: {},
    platform: "linux",
    arch: "x64",
    download: releaseDownload(requested),
    log: quiet,
  };
  assert.equal(findBinary("wae", options), undefined);
  const installed = await ensureBinary("wae", options);
  assert.equal(installed, path.join(packageRoot, "vendor", "wae"));
  assert.equal(fs.readFileSync(installed, "utf8"), "binary:wae-x86_64-unknown-linux-gnu");
  assert.deepEqual(requested, [
    "https://github.com/owner/repo/releases/download/v1.2.3/wae-x86_64-unknown-linux-gnu",
    "https://github.com/owner/repo/releases/download/v1.2.3/wae-x86_64-unknown-linux-gnu.sha256",
  ]);
  assert.deepEqual(fs.readdirSync(path.dirname(installed)), ["wae"]);
  // A second launch reuses the installed binary without any network access.
  const again = await ensureBinary("wae", { ...options, download: () => assert.fail("downloaded") });
  assert.equal(again, installed);
});

test("embedded release checksums are the trust root when present", async (t) => {
  const asset = "wae-mcp-aarch64-apple-darwin";
  const checksum = crypto.createHash("sha256").update(`binary:${asset}`).digest("hex");
  const { packageRoot, cacheDir } = temporaryPackage(t, { checksums: { [asset]: checksum } });
  const requested = [];
  await ensureBinary("wae-mcp", {
    packageRoot,
    cacheDir,
    env: {},
    platform: "darwin",
    arch: "arm64",
    download: releaseDownload(requested),
    log: quiet,
  });
  assert.equal(requested.length, 1, "embedded checksum must avoid the GitHub .sha256 lookup");
});

test("falls back to the user cache when the package directory is not writable", async (t) => {
  const { root, packageRoot, cacheDir } = temporaryPackage(t);
  const blocked = path.join(root, "read-only-file");
  fs.writeFileSync(blocked, "not a directory");
  const installed = await ensureBinary("wae-lsp", {
    packageRoot,
    cacheDir,
    vendorDir: path.join(blocked, "vendor"),
    env: {},
    platform: "linux",
    arch: "arm64",
    download: releaseDownload(),
    log: quiet,
  });
  assert.equal(installed, path.join(cacheDir, "wae-lsp"));
});

test("WAE_BINARY_DIR and WAE_SKIP_DOWNLOAD never trigger a download", async (t) => {
  const { root, packageRoot, cacheDir } = temporaryPackage(t);
  const provisioned = path.join(root, "provisioned");
  fs.mkdirSync(provisioned);
  fs.writeFileSync(path.join(provisioned, "wae.exe"), "air-gapped");
  const noDownload = () => assert.fail("must not download");
  const base = { packageRoot, cacheDir, platform: "win32", arch: "x64", download: noDownload };
  assert.equal(
    await ensureBinary("wae", { ...base, env: { WAE_BINARY_DIR: provisioned } }),
    path.join(provisioned, "wae.exe")
  );
  await assert.rejects(
    ensureBinary("wae-lsp", { ...base, env: { WAE_BINARY_DIR: provisioned } }),
    /does not contain wae-lsp\.exe/
  );
  await assert.rejects(
    ensureBinary("wae", { ...base, env: { WAE_SKIP_DOWNLOAD: "1" } }),
    /WAE_SKIP_DOWNLOAD=1 disables downloads/
  );
});

test("installs the CLI, language server and MCP server from version-matched assets", async (t) => {
  const { packageRoot, cacheDir } = temporaryPackage(t);
  const requested = [];
  await installAll({
    packageRoot,
    cacheDir,
    env: {},
    platform: "linux",
    arch: "x64",
    download: releaseDownload(requested),
    log: quiet,
  });
  assert.deepEqual(fs.readdirSync(path.join(packageRoot, "vendor")).sort(), [
    "wae",
    "wae-lsp",
    "wae-mcp",
  ]);
  for (const component of ["wae", "wae-lsp", "wae-mcp"]) {
    assert(requested.some((url) => url.endsWith(`/v1.2.3/${component}-x86_64-unknown-linux-gnu`)));
  }
});

test("checksum failure preserves the installed binary and removes temporary files", async (t) => {
  const { packageRoot } = temporaryPackage(t, { version: "9.9.9" });
  const binDir = path.join(packageRoot, "vendor");
  fs.mkdirSync(binDir);
  fs.writeFileSync(path.join(binDir, "wae"), "known-good");
  const download = async (url, destination) => {
    fs.writeFileSync(destination, url.endsWith(".sha256") ? "0".repeat(64) : "new-binary");
  };
  await assert.rejects(
    installVerifiedBinary({
      packageRoot,
      binDir,
      env: {},
      platform: "linux",
      arch: "x64",
      download,
      log: quiet,
    }),
    /SHA-256 verification failed/
  );
  assert.equal(fs.readFileSync(path.join(binDir, "wae"), "utf8"), "known-good");
  assert.deepEqual(fs.readdirSync(binDir), ["wae"]);
});

test("accepts only HTTPS GitHub release hosts", () => {
  assert.equal(validateDownloadUrl("https://github.com/owner/repo").hostname, "github.com");
  assert.equal(
    validateDownloadUrl("https://release-assets.githubusercontent.com/file").hostname,
    "release-assets.githubusercontent.com"
  );
  assert.throws(() => validateDownloadUrl("http://github.com/owner/repo"), /untrusted/);
  assert.throws(() => validateDownloadUrl("https://github.com.example.org/file"), /untrusted/);
});

test("rejects redirect loops before creating an install file", async (t) => {
  const { root } = temporaryPackage(t);
  const destination = path.join(root, "wae.tmp");
  const client = fakeClient((response) => {
    response.statusCode = 302;
    response.headers.location = "https://github.com/owner/repo/file";
    return () => response.end();
  });
  await assert.rejects(
    downloadFile("https://github.com/owner/repo/file", destination, { client }),
    /Too many redirects/
  );
  assert.equal(fs.existsSync(destination), false);
});

test("cleans up oversized and interrupted downloads", async (t) => {
  const { root } = temporaryPackage(t);
  const oversized = path.join(root, "oversized.tmp");
  const oversizedClient = fakeClient((response) => {
    response.headers["content-length"] = "11";
    return () => response.end("too large");
  });
  await assert.rejects(
    downloadFile("https://github.com/owner/repo/file", oversized, {
      client: oversizedClient,
      maxBytes: 10,
    }),
    /exceeds 10 bytes/
  );
  assert.equal(fs.existsSync(oversized), false);

  const interrupted = path.join(root, "interrupted.tmp");
  const interruptedClient = fakeClient((response) => () => {
    response.write("partial");
    response.destroy(new Error("connection interrupted"));
  });
  await assert.rejects(
    downloadFile("https://github.com/owner/repo/file", interrupted, { client: interruptedClient }),
    /connection interrupted/
  );
  assert.equal(fs.existsSync(interrupted), false);
});
