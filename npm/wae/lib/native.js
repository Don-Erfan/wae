"use strict";

// Native binary provisioning for the single `@don-erfan/wae` package.
//
// The package ships only JavaScript. The CLI, language server and MCP server for the host
// platform are downloaded from the matching GitHub Release, verified against SHA-256 hashes that
// the release workflow embeds in `checksums.json` (so npm package integrity is the trust root),
// and atomically installed. `postinstall` pre-installs them; when lifecycle scripts are disabled
// (pnpm 10 defaults, `--ignore-scripts`) the launcher performs the same verified install on first
// use. All progress output goes to stderr because `wae-lsp` and `wae-mcp` own stdout.

const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const PACKAGE_ROOT = path.join(__dirname, "..");
const COMPONENTS = Object.freeze(["wae", "wae-lsp", "wae-mcp"]);
const TARGETS = Object.freeze({
  "linux-x64": "x86_64-unknown-linux-gnu",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "darwin-x64": "x86_64-apple-darwin",
  "darwin-arm64": "aarch64-apple-darwin",
  "win32-x64": "x86_64-pc-windows-msvc",
});
const MAX_REDIRECTS = 5;
const MAX_DOWNLOAD_BYTES = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_MS = 30_000;
// Errors that mean "this directory cannot hold binaries", so the next location is tried.
const UNWRITABLE_CODES = new Set(["EACCES", "EPERM", "EROFS", "ENOTDIR", "EEXIST"]);

function resolveTarget(platform = process.platform, arch = process.arch) {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target) {
    throw new Error(
      `Unsupported platform/arch combination: ${platform}/${arch}. ` +
        `Supported: ${Object.keys(TARGETS).join(", ")}.`
    );
  }
  return target;
}

function executableName(component, platform = process.platform) {
  return `${component}${platform === "win32" ? ".exe" : ""}`;
}

function assetName(component, platform = process.platform, arch = process.arch) {
  return `${component}-${resolveTarget(platform, arch)}${platform === "win32" ? ".exe" : ""}`;
}

function cacheDirectory(env = process.env, platform = process.platform, home = os.homedir()) {
  if (env.WAE_CACHE_DIR) return path.resolve(env.WAE_CACHE_DIR);
  if (platform === "win32") {
    return path.join(env.LOCALAPPDATA || path.join(home, "AppData", "Local"), "wae", "Cache");
  }
  if (platform === "darwin") return path.join(home, "Library", "Caches", "wae");
  return path.join(env.XDG_CACHE_HOME || path.join(home, ".cache"), "wae");
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function context(options = {}) {
  const packageRoot = options.packageRoot || PACKAGE_ROOT;
  const env = options.env || process.env;
  const platform = options.platform || process.platform;
  const arch = options.arch || process.arch;
  const manifest = readJson(path.join(packageRoot, "package.json"));
  const target = resolveTarget(platform, arch);
  return {
    env,
    platform,
    arch,
    manifest,
    packageRoot,
    version: manifest.version,
    target,
    explicitDir: env.WAE_BINARY_DIR ? path.resolve(env.WAE_BINARY_DIR) : undefined,
    vendorDir: options.vendorDir || path.join(packageRoot, "vendor"),
    cacheDir:
      options.cacheDir ||
      path.join(cacheDirectory(env, platform, options.home), manifest.version, target),
  };
}

/** Returns the first installed binary, without downloading. */
function findBinary(component, options = {}) {
  const ctx = context(options);
  const file = executableName(component, ctx.platform);
  if (ctx.explicitDir) {
    const explicit = path.join(ctx.explicitDir, file);
    return fs.existsSync(explicit) ? explicit : undefined;
  }
  for (const directory of [ctx.vendorDir, ctx.cacheDir]) {
    const candidate = path.join(directory, file);
    if (fs.existsSync(candidate)) return candidate;
  }
  return undefined;
}

/** Returns an installed binary, performing a verified download into the first writable location. */
async function ensureBinary(component, options = {}) {
  if (!COMPONENTS.includes(component)) throw new Error(`Unknown WAE component: ${component}`);
  const ctx = context(options);
  const existing = findBinary(component, options);
  if (existing) return existing;
  const file = executableName(component, ctx.platform);
  if (ctx.explicitDir) {
    throw new Error(
      `WAE_BINARY_DIR is set to ${ctx.explicitDir}, but it does not contain ${file}. ` +
        `Place the ${assetName(component, ctx.platform, ctx.arch)} release asset there as ${file}.`
    );
  }
  if (ctx.env.WAE_SKIP_DOWNLOAD === "1") {
    throw new Error(
      `${file} is not installed and WAE_SKIP_DOWNLOAD=1 disables downloads. ` +
        "Unset it, or set WAE_BINARY_DIR to a directory containing verified WAE release binaries."
    );
  }
  let lastError;
  for (const binDir of [ctx.vendorDir, ctx.cacheDir]) {
    try {
      return await installVerifiedBinary({ ...options, component, binDir });
    } catch (error) {
      if (!UNWRITABLE_CODES.has(error.code)) throw error;
      lastError = error;
    }
  }
  throw new Error(
    `No writable location for ${file} (tried ${ctx.vendorDir} and ${ctx.cacheDir}): ` +
      `${lastError.message}. Set WAE_CACHE_DIR to a writable directory.`
  );
}

function downloadFile(url, destination, options = {}) {
  return new Promise((resolve, reject) => {
    const redirects = options.redirects || 0;
    const client = options.client || https;
    const maxBytes = options.maxBytes || MAX_DOWNLOAD_BYTES;
    const timeoutMs = options.timeoutMs || DOWNLOAD_TIMEOUT_MS;
    let stream;
    let settled = false;
    const fail = (error) => {
      if (settled) return;
      settled = true;
      if (stream) stream.destroy();
      fs.rmSync(destination, { force: true });
      reject(error);
    };
    try {
      validateDownloadUrl(url);
    } catch (error) {
      fail(error);
      return;
    }
    const request = client.get(url, { headers: { "User-Agent": "wae-npm-installer" } }, (response) => {
      if (
        response.statusCode &&
        response.statusCode >= 300 &&
        response.statusCode < 400 &&
        response.headers.location
      ) {
        response.resume();
        if (redirects >= MAX_REDIRECTS) {
          fail(new Error(`Too many redirects while downloading ${url}`));
          return;
        }
        const redirectUrl = new URL(response.headers.location, url).toString();
        downloadFile(redirectUrl, destination, { ...options, redirects: redirects + 1 })
          .then(resolve)
          .catch(fail);
        return;
      }
      if (response.statusCode !== 200) {
        response.resume();
        fail(new Error(`Download failed (${response.statusCode}) for ${url}`));
        return;
      }
      const declaredSize = Number(response.headers["content-length"] || 0);
      if (declaredSize > maxBytes) {
        response.resume();
        fail(new Error(`Download exceeds ${maxBytes} bytes`));
        return;
      }
      stream = fs.createWriteStream(destination, { flags: "wx" });
      let received = 0;
      response.on("data", (chunk) => {
        received += chunk.length;
        if (received > maxBytes) response.destroy(new Error(`Download exceeds ${maxBytes} bytes`));
      });
      response.pipe(stream);
      response.on("error", (error) => {
        stream.destroy();
        fail(error);
      });
      stream.on("finish", () => {
        stream.close(() => {
          if (settled) return;
          settled = true;
          resolve();
        });
      });
      stream.on("error", fail);
    });
    request.on("error", fail);
    request.setTimeout(timeoutMs, () => {
      request.destroy(new Error(`Download timed out after ${timeoutMs}ms`));
    });
  });
}

function validateDownloadUrl(url) {
  const parsedUrl = new URL(url);
  const allowedHost =
    parsedUrl.hostname === "github.com" || parsedUrl.hostname.endsWith(".githubusercontent.com");
  if (parsedUrl.protocol !== "https:" || !allowedHost) {
    throw new Error(`Refusing download from untrusted URL host: ${parsedUrl.hostname}`);
  }
  return parsedUrl;
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function installVerifiedBinary(options = {}) {
  const ctx = context(options);
  const log = options.log || ((message) => console.error(message));
  const repository =
    options.repository || ctx.env.WAE_GITHUB_REPOSITORY || ctx.manifest.wae?.githubRepo;
  if (!repository) {
    throw new Error(
      "`wae.githubRepo` is not configured. Set it in package.json or export WAE_GITHUB_REPOSITORY."
    );
  }
  const component = options.component || "wae";
  const asset = assetName(component, ctx.platform, ctx.arch);
  const downloadUrl = `https://github.com/${repository}/releases/download/v${ctx.version}/${asset}`;
  const binDir = options.binDir || ctx.vendorDir;
  const binaryPath = path.join(binDir, executableName(component, ctx.platform));
  const nonce = `${process.pid}-${crypto.randomBytes(8).toString("hex")}`;
  const temporaryBinaryPath = path.join(binDir, `.wae-${nonce}.tmp`);
  const temporaryChecksumPath = path.join(binDir, `.wae-${nonce}.sha256.tmp`);
  const embeddedChecksums = readJson(path.join(ctx.packageRoot, "checksums.json"));

  fs.mkdirSync(binDir, { recursive: true });
  try {
    log(`Downloading ${asset} for WAE ${ctx.version}`);
    const download = options.download || downloadFile;
    await download(downloadUrl, temporaryBinaryPath);
    let expected = embeddedChecksums[asset];
    if (!expected) {
      // Source checkouts keep an empty manifest. Published packages embed release hashes so the
      // trust root is npm package integrity, independent of the GitHub release origin.
      await download(`${downloadUrl}.sha256`, temporaryChecksumPath);
      expected = fs.readFileSync(temporaryChecksumPath, "utf8").trim().split(/\s+/)[0];
    }
    const actual = sha256(temporaryBinaryPath);
    if (!/^[a-f0-9]{64}$/i.test(expected) || actual !== expected.toLowerCase()) {
      throw new Error(`SHA-256 verification failed for ${asset}`);
    }
    if (ctx.platform !== "win32") fs.chmodSync(temporaryBinaryPath, 0o755);
    try {
      fs.renameSync(temporaryBinaryPath, binaryPath);
    } catch (error) {
      if (!["EEXIST", "EPERM"].includes(error.code)) throw error;
      // A concurrent first run may have installed the same verified file already.
      if (fs.existsSync(binaryPath) && sha256(binaryPath) === actual) return binaryPath;
      fs.rmSync(binaryPath, { force: true });
      fs.renameSync(temporaryBinaryPath, binaryPath);
    }
    log(`Installed verified ${path.basename(binaryPath)} at ${binaryPath}`);
    return binaryPath;
  } finally {
    fs.rmSync(temporaryBinaryPath, { force: true });
    fs.rmSync(temporaryChecksumPath, { force: true });
  }
}

/** Installs every component, reusing binaries that are already present. */
async function installAll(options = {}) {
  const installed = [];
  for (const component of COMPONENTS) installed.push(await ensureBinary(component, options));
  return installed;
}

module.exports = {
  COMPONENTS,
  TARGETS,
  assetName,
  cacheDirectory,
  downloadFile,
  ensureBinary,
  executableName,
  findBinary,
  installAll,
  installVerifiedBinary,
  resolveTarget,
  sha256,
  validateDownloadUrl,
};
