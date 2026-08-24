#!/usr/bin/env node

const fs = require("node:fs");
const crypto = require("node:crypto");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const MAX_REDIRECTS = 5;
const MAX_DOWNLOAD_BYTES = 100 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_MS = 30_000;

function resolveTarget() {
  const platform = os.platform();
  const arch = os.arch();

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

function downloadFile(url, destination, redirects = 0) {
  return new Promise((resolve, reject) => {
    let parsedUrl;
    try {
      parsedUrl = validateDownloadUrl(url);
    } catch (error) {
      reject(error);
      return;
    }
    const request = https.get(
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
            reject(new Error(`Too many redirects while downloading ${url}`));
            return;
          }
          const redirectUrl = new URL(response.headers.location, url).toString();
          downloadFile(redirectUrl, destination, redirects + 1)
            .then(resolve)
            .catch(reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`Download failed (${response.statusCode}) for ${url}`));
          return;
        }

        const declaredSize = Number(response.headers["content-length"] || 0);
        if (declaredSize > MAX_DOWNLOAD_BYTES) {
          response.resume();
          reject(new Error(`Download exceeds ${MAX_DOWNLOAD_BYTES} bytes`));
          return;
        }

        const stream = fs.createWriteStream(destination, { flags: "wx" });
        let received = 0;

        response.on("data", (chunk) => {
          received += chunk.length;
          if (received > MAX_DOWNLOAD_BYTES) {
            response.destroy(new Error(`Download exceeds ${MAX_DOWNLOAD_BYTES} bytes`));
          }
        });

        response.pipe(stream);

        response.on("error", (error) => {
          stream.destroy();
          reject(error);
        });

        stream.on("finish", () => {
          stream.close(resolve);
        });

        stream.on("error", (error) => {
          reject(error);
        });
      }
    );

    request.on("error", reject);
    request.setTimeout(DOWNLOAD_TIMEOUT_MS, () => {
      request.destroy(new Error(`Download timed out after ${DOWNLOAD_TIMEOUT_MS}ms`));
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

async function main() {
  if (process.env.WAE_SKIP_DOWNLOAD === "1") {
    console.log("Skipping WAE binary download because WAE_SKIP_DOWNLOAD=1");
    return;
  }

  const packageJsonPath = path.join(__dirname, "..", "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

  const repository = process.env.WAE_GITHUB_REPOSITORY || packageJson.wae?.githubRepo;
  if (!repository) {
    throw new Error(
      "`wae.githubRepo` is not configured. Set it in package.json or export WAE_GITHUB_REPOSITORY."
    );
  }

  const target = resolveTarget();
  const version = packageJson.version;
  const extension = os.platform() === "win32" ? ".exe" : "";
  const assetName = `wae-${target}${extension}`;
  const releaseTag = `v${version}`;

  const downloadUrl = `https://github.com/${repository}/releases/download/${releaseTag}/${assetName}`;
  const binDir = path.join(__dirname, "..", "bin");
  const binaryPath = path.join(binDir, `wae${extension}`);
  const nonce = `${process.pid}-${crypto.randomBytes(8).toString("hex")}`;
  const temporaryBinaryPath = path.join(binDir, `.wae-${nonce}.tmp`);
  const temporaryChecksumPath = path.join(binDir, `.wae-${nonce}.sha256.tmp`);

  fs.mkdirSync(binDir, { recursive: true });

  try {
    console.log(`Downloading ${assetName} from ${downloadUrl}`);
    await downloadFile(downloadUrl, temporaryBinaryPath);
    await downloadFile(`${downloadUrl}.sha256`, temporaryChecksumPath);

    const expected = fs.readFileSync(temporaryChecksumPath, "utf8").trim().split(/\s+/)[0];
    const actual = sha256(temporaryBinaryPath);
    if (!/^[a-f0-9]{64}$/i.test(expected) || actual !== expected.toLowerCase()) {
      throw new Error(`SHA-256 verification failed for ${assetName}`);
    }

    if (os.platform() !== "win32") {
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

if (require.main === module) {
  main().catch((error) => {
    console.error(`Could not install the verified WAE binary: ${error.message}`);
    process.exitCode = 1;
  });
}

module.exports = { downloadFile, resolveTarget, sha256, validateDownloadUrl };
