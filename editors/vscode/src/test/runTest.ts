import * as path from "node:path";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  const extensionDevelopmentPath = path.resolve(__dirname, "../../..");
  const extensionTestsPath = path.resolve(__dirname, "suite/index.js");
  const workspace = path.resolve(extensionDevelopmentPath, "../../fixtures/circular");
  await runTests({
    extensionDevelopmentPath,
    extensionTestsPath,
    launchArgs: [workspace, "--disable-extensions"],
    vscodeExecutablePath: process.env.WAE_VSCODE_EXECUTABLE,
    version: process.env.WAE_VSCODE_VERSION ?? "1.90.0",
  });
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
