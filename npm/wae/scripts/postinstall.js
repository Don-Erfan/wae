#!/usr/bin/env node

const fs = require("node:fs");
const https = require("node:https");
const os = require("node:os");
const path = require("node:path");

const MAX_REDIRECTS = 5;

function resolveTarget() {
  const platform = os.platform();
  const arch = os.arch();

  const targets = {
    linux: {
      x64: "x86_64-unknown-linux-gnu",
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

        const tempPath = `${destination}.tmp`;
        const stream = fs.createWriteStream(tempPath);

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
  });
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

  fs.mkdirSync(binDir, { recursive: true });

  console.log(`Downloading ${assetName} from ${downloadUrl}`);
  await downloadFile(downloadUrl, binaryPath);

  if (os.platform() !== "win32") {
    fs.chmodSync(binaryPath, 0o755);
  }

  console.log(`WAE binary installed at ${binaryPath}`);
}

main().catch((error) => {
  // Never fail the host project's install over this. WAE is a dev-time
  // architecture linter; a download outage, an offline CI box, or a missing
  // release asset must not break `npm/yarn install` for the whole app.
  // `bin/wae.js` reports the missing binary if someone actually runs the CLI.
  console.warn(`Warning: could not install the WAE binary: ${error.message}`);
  console.warn(
    "The `wae` command will be unavailable. Re-run `npm rebuild @don-erfan/wae` once the download works, " +
      "or set WAE_SKIP_DOWNLOAD=1 to silence this."
  );
});
