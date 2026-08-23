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
          downloadFile(response.headers.location, destination, redirects + 1)
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

        const tempPath = `${destination}.tmp`;
        const stream = fs.createWriteStream(tempPath);
        let received = 0;

        response.on("data", (chunk) => {
          received += chunk.length;
          if (received > MAX_DOWNLOAD_BYTES) {
            response.destroy(new Error(`Download exceeds ${MAX_DOWNLOAD_BYTES} bytes`));
          }
        });

        response.pipe(stream);

        stream.on("finish", () => {
          stream.close(() => {
            fs.rename(tempPath, destination, (error) => {
              if (error) {
                reject(error);
                return;
              }
              resolve();
            });
          });
        });

        stream.on("error", (error) => {
          fs.rmSync(tempPath, { force: true });
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
  const checksumPath = `${binaryPath}.sha256`;

  fs.mkdirSync(binDir, { recursive: true });

  console.log(`Downloading ${assetName} from ${downloadUrl}`);
  await downloadFile(downloadUrl, binaryPath);
  await downloadFile(`${downloadUrl}.sha256`, checksumPath);

  const expected = fs.readFileSync(checksumPath, "utf8").trim().split(/\s+/)[0];
  const actual = sha256(binaryPath);
  fs.rmSync(checksumPath, { force: true });
  if (!/^[a-f0-9]{64}$/i.test(expected) || actual !== expected.toLowerCase()) {
    fs.rmSync(binaryPath, { force: true });
    throw new Error(`SHA-256 verification failed for ${assetName}`);
  }

  if (os.platform() !== "win32") {
    fs.chmodSync(binaryPath, 0o755);
  }

  console.log(`WAE binary installed at ${binaryPath}`);
}

main().catch((error) => {
  console.error(`Could not install the verified WAE binary: ${error.message}`);
  process.exitCode = 1;
});
