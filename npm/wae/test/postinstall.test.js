const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { PassThrough } = require("node:stream");
const test = require("node:test");

const {
  downloadFile,
  installVerifiedBinary,
  installVerifiedBinaries,
  validateDownloadUrl,
} = require("../scripts/postinstall.js");

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

test("accepts only HTTPS GitHub release hosts", () => {
  assert.equal(validateDownloadUrl("https://github.com/owner/repo").hostname, "github.com");
  assert.equal(
    validateDownloadUrl("https://release-assets.githubusercontent.com/file").hostname,
    "release-assets.githubusercontent.com"
  );
  assert.throws(() => validateDownloadUrl("http://github.com/owner/repo"), /untrusted/);
  assert.throws(() => validateDownloadUrl("https://github.com.example.org/file"), /untrusted/);
});

test("rejects redirect loops before creating an install file", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-redirect-"));
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
  fs.rmSync(root, { recursive: true, force: true });
});

test("cleans up oversized and interrupted downloads", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-stream-"));
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
  const interruptedClient = fakeClient((response) => {
    return () => {
      response.write("partial");
      response.destroy(new Error("connection interrupted"));
    };
  });
  await assert.rejects(
    downloadFile("https://github.com/owner/repo/file", interrupted, {
      client: interruptedClient,
    }),
    /connection interrupted/
  );
  assert.equal(fs.existsSync(interrupted), false);
  fs.rmSync(root, { recursive: true, force: true });
});

test("checksum failure preserves the installed binary and removes temporary files", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-checksum-"));
  const binDir = path.join(root, "bin");
  const packageJsonPath = path.join(root, "package.json");
  fs.mkdirSync(binDir);
  fs.writeFileSync(packageJsonPath, JSON.stringify({ version: "9.9.9" }));
  fs.writeFileSync(path.join(binDir, "wae"), "known-good");
  const download = async (url, destination) => {
    fs.writeFileSync(destination, url.endsWith(".sha256") ? "0".repeat(64) : "new-binary");
  };
  await assert.rejects(
    installVerifiedBinary({
      packageJsonPath,
      binDir,
      repository: "owner/repo",
      platform: "linux",
      arch: "x64",
      download,
    }),
    /SHA-256 verification failed/
  );
  assert.equal(fs.readFileSync(path.join(binDir, "wae"), "utf8"), "known-good");
  assert.deepEqual(fs.readdirSync(binDir), ["wae"]);
  fs.rmSync(root, { recursive: true, force: true });
});

test("installs CLI, language server and MCP server from version-matched assets", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wae-components-"));
  const binDir = path.join(root, "bin");
  const packageJsonPath = path.join(root, "package.json");
  fs.writeFileSync(packageJsonPath, JSON.stringify({ version: "1.2.3" }));
  const crypto = require("node:crypto");
  const requested = [];
  const download = async (url, destination) => {
    requested.push(url);
    const assetUrl = url.endsWith(".sha256") ? url.slice(0, -7) : url;
    const payload = `binary:${path.basename(assetUrl)}`;
    const content = url.endsWith(".sha256")
      ? crypto.createHash("sha256").update(payload).digest("hex")
      : payload;
    fs.writeFileSync(destination, content);
  };
  await installVerifiedBinaries({
    packageJsonPath,
    binDir,
    repository: "owner/repo",
    platform: "linux",
    arch: "x64",
    download,
  });
  assert.deepEqual(fs.readdirSync(binDir).sort(), ["wae", "wae-lsp", "wae-mcp"]);
  for (const component of ["wae", "wae-lsp", "wae-mcp"]) {
    assert(requested.some((url) => url.endsWith(`${component}-x86_64-unknown-linux-gnu`)));
  }
  fs.rmSync(root, { recursive: true, force: true });
});
