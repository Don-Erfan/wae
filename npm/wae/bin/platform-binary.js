#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const PLATFORM_PACKAGES = Object.freeze({
  "linux-x64": "@don-erfan/wae-linux-x64",
  "linux-arm64": "@don-erfan/wae-linux-arm64",
  "darwin-x64": "@don-erfan/wae-darwin-x64",
  "darwin-arm64": "@don-erfan/wae-darwin-arm64",
  "win32-x64": "@don-erfan/wae-win32-x64",
});

function packageForPlatform(platform = os.platform(), arch = os.arch()) {
  const packageName = PLATFORM_PACKAGES[`${platform}-${arch}`];
  if (!packageName) {
    throw new Error(`Unsupported platform/arch combination: ${platform}/${arch}`);
  }
  return packageName;
}

function resolveBinary(component, options = {}) {
  const platform = options.platform || os.platform();
  const arch = options.arch || os.arch();
  const extension = platform === "win32" ? ".exe" : "";
  const packageName = packageForPlatform(platform, arch);
  const request = `${packageName}/bin/${component}${extension}`;
  try {
    return require.resolve(request);
  } catch (packageError) {
    // Source checkouts and packages installed by the legacy recovery command may still keep
    // binaries beside the JS launchers. This fallback is deliberately local and never downloads.
    const local = path.join(__dirname, `${component}${extension}`);
    if (fs.existsSync(local)) return local;
    const error = new Error(
      `WAE ${component} binary is unavailable for ${platform}/${arch}. ` +
        `Reinstall optional dependency ${packageName}@the-same-version; package lifecycle scripts are not required.`
    );
    error.cause = packageError;
    throw error;
  }
}

module.exports = { PLATFORM_PACKAGES, packageForPlatform, resolveBinary };
