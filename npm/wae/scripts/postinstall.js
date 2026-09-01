#!/usr/bin/env node

const fs = require("node:fs");
const crypto = require("node:crypto");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const MAX_REDIRECTS = 5;
const MAX_DOWNLOAD_BYTES = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_MS = 30_000;

function resolveTarget(platform = os.platform(), arch = os.arch()) {

  const targets = {
    linux: {
      x64: "x86_64-unknown-linux-gnu",
      arm64: "aarch64-unknown-linux-gnu",
    },
    darwin: {
      x64: "x86_64-apple-darwin",
      arm64: "aarch64-apple-darwin",
    },
    win32: {
      x64: "x86_64-pc-windows-msvc",
    },
  };

  if (!targets[platform] || !targets[platform][arch]) {
    throw new Error(`Unsupported platform/arch combination: ${platform}/${arch}`);
  }

  return targets[platform][arch];
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
    let parsedUrl;
    try {
      parsedUrl = validateDownloadUrl(url);
    } catch (error) {
      fail(error);
      return;
    }
    const request = client.get(
      url,
      {
        headers: {
          "User-Agent": "wae-npm-installer",
        },
      },
      (response) => {
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
          if (received > maxBytes) {
            response.destroy(new Error(`Download exceeds ${maxBytes} bytes`));
          }
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

        stream.on("error", (error) => {
          fail(error);
        });
      }
    );

    request.on("error", fail);
    request.setTimeout(timeoutMs, () => {
      request.destroy(new Error(`Download timed out after ${timeoutMs}ms`));
    });
  });
}

function validateDownloadUrl(url) {
  const parsedUrl = new URL(url);
  const allowedHost =
    parsedUrl.hostname === "github.com" ||
    parsedUrl.hostname.endsWith(".githubusercontent.com");
  if (parsedUrl.protocol !== "https:" || !allowedHost) {
    throw new Error(`Refusing download from untrusted URL host: ${parsedUrl.hostname}`);
  }
  return parsedUrl;
}

function sha256(filePath) {
  return crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

async function installVerifiedBinary(options = {}) {
  if (process.env.WAE_SKIP_DOWNLOAD === "1") {
    console.log("Skipping WAE binary download because WAE_SKIP_DOWNLOAD=1");
    return;
  }

  const packageJsonPath = options.packageJsonPath || path.join(__dirname, "..", "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

  const repository =
    options.repository || process.env.WAE_GITHUB_REPOSITORY || packageJson.wae?.githubRepo;
  if (!repository) {
    throw new Error(
      "`wae.githubRepo` is not configured. Set it in package.json or export WAE_GITHUB_REPOSITORY."
    );
  }

  const platform = options.platform || os.platform();
  const arch = options.arch || os.arch();
  const target = resolveTarget(platform, arch);
  const version = packageJson.version;
  const extension = platform === "win32" ? ".exe" : "";
  const component = options.component || "wae";
  const assetName = `${component}-${target}${extension}`;
  const releaseTag = `v${version}`;

  const downloadUrl = `https://github.com/${repository}/releases/download/${releaseTag}/${assetName}`;
  const binDir = options.binDir || path.join(__dirname, "..", "bin");
  const binaryPath = path.join(binDir, `${component}${extension}`);
  const nonce = `${process.pid}-${crypto.randomBytes(8).toString("hex")}`;
  const temporaryBinaryPath = path.join(binDir, `.wae-${nonce}.tmp`);
  const temporaryChecksumPath = path.join(binDir, `.wae-${nonce}.sha256.tmp`);
  const embeddedChecksumsPath = path.join(__dirname, "..", "checksums.json");
  const embeddedChecksums = JSON.parse(fs.readFileSync(embeddedChecksumsPath, "utf8"));

  fs.mkdirSync(binDir, { recursive: true });

  try {
    console.log(`Downloading ${assetName} from ${downloadUrl}`);
    const download = options.download || downloadFile;
    await download(downloadUrl, temporaryBinaryPath);
    let expected = embeddedChecksums[assetName];
    if (!expected) {
      // Source checkouts keep an empty manifest. Published packages embed release hashes so the
      // trust root is npm package integrity, independent of the GitHub release origin.
      await download(`${downloadUrl}.sha256`, temporaryChecksumPath);
      expected = fs.readFileSync(temporaryChecksumPath, "utf8").trim().split(/\s+/)[0];
    }
    const actual = sha256(temporaryBinaryPath);
    if (!/^[a-f0-9]{64}$/i.test(expected) || actual !== expected.toLowerCase()) {
      throw new Error(`SHA-256 verification failed for ${assetName}`);
    }

    if (platform !== "win32") {
      fs.chmodSync(temporaryBinaryPath, 0o755);
    }

    try {
      fs.renameSync(temporaryBinaryPath, binaryPath);
    } catch (error) {
      if (!['EEXIST', 'EPERM'].includes(error.code)) throw error;
      fs.rmSync(binaryPath, { force: true });
      fs.renameSync(temporaryBinaryPath, binaryPath);
    }
    console.log(`WAE binary installed at ${binaryPath}`);
  } finally {
    fs.rmSync(temporaryBinaryPath, { force: true });
    fs.rmSync(temporaryChecksumPath, { force: true });
  }
}

async function installVerifiedBinaries(options = {}) {
  for (const component of ["wae", "wae-lsp", "wae-mcp"]) {
    await installVerifiedBinary({ ...options, component });
  }
}

async function main() {
  await installVerifiedBinaries();
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`Could not install the verified WAE binary: ${error.message}`);
    process.exitCode = 1;
  });
}

module.exports = {
  downloadFile,
  installVerifiedBinary,
  installVerifiedBinaries,
  resolveTarget,
  sha256,
  validateDownloadUrl,
};
