import * as assert from "node:assert/strict";
import * as path from "node:path";
import * as vscode from "vscode";

export async function run(): Promise<void> {
    const serverPath = process.env.WAE_LSP_PATH;
    assert.ok(serverPath, "WAE_LSP_PATH must point to the test language server");
    await vscode.workspace
      .getConfiguration("wae")
      .update("server.path", serverPath, vscode.ConfigurationTarget.Workspace);
    const extension = vscode.extensions.getExtension("don-erfan.wae-vscode");
    assert.ok(extension, "development extension is installed");
    await extension.activate();
    const workspace = vscode.workspace.workspaceFolders?.[0];
    assert.ok(workspace);
    const uri = vscode.Uri.file(path.join(workspace.uri.fsPath, "src/a.ts"));
    const document = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(document);

    const deadline = Date.now() + 15_000;
    let diagnostics: readonly vscode.Diagnostic[] = [];
    while (Date.now() < deadline) {
      diagnostics = vscode.languages.getDiagnostics(uri);
      if (diagnostics.some((diagnostic) => diagnostic.code === "ARCH-001")) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.ok(
      diagnostics.some((diagnostic) => diagnostic.code === "ARCH-001"),
      `expected ARCH-001, got ${diagnostics.map((diagnostic) => diagnostic.code).join(", ")}`,
    );
    const commands = await vscode.commands.getCommands(true);
    assert.ok(commands.includes("wae.check"));
    assert.ok(commands.includes("wae.showSuggestion"));
    assert.ok(commands.includes("wae.suppressWithReason"));

    const applied = await vscode.commands.executeCommand<boolean>("wae.suppressWithReason", {
      uri: uri.toString(),
      line: 0,
      ruleId: "ARCH-001",
      reason: "Extension Host regression test ARC-001",
    });
    assert.equal(applied, true);
    assert.equal(
      document.lineAt(0).text,
      "// wae-ignore ARCH-001 -- Extension Host regression test ARC-001",
    );
    const restore = new vscode.WorkspaceEdit();
    restore.delete(uri, new vscode.Range(0, 0, 1, 0));
    assert.equal(await vscode.workspace.applyEdit(restore), true);
    assert.ok(!document.getText().includes("Extension Host regression test ARC-001"));
}
