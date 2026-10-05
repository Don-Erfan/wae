import * as assert from "node:assert/strict";
import * as path from "node:path";
import * as vscode from "vscode";

// With `codeDescription`, VS Code exposes the rule ID as `{ value, target }` (a docs link).
function ruleOf(diagnostic: vscode.Diagnostic): string | number | undefined {
  return typeof diagnostic.code === "object" ? diagnostic.code.value : diagnostic.code;
}

export async function run(): Promise<void> {
    const serverPath = process.env.WAE_LSP_PATH;
    assert.ok(serverPath, "WAE_LSP_PATH must point to the test language server");
    await vscode.workspace
      .getConfiguration("wae")
      .update("server.path", serverPath, vscode.ConfigurationTarget.Global);
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
      if (diagnostics.some((diagnostic) => ruleOf(diagnostic) === "ARCH-001")) break;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    assert.ok(
      diagnostics.some((diagnostic) => ruleOf(diagnostic) === "ARCH-001"),
      `expected ARCH-001, got ${diagnostics.map(ruleOf).join(", ")}`,
    );
    const commands = await vscode.commands.getCommands(true);
    for (const command of [
      "wae.check", "wae.showSuggestion", "wae.suppressWithReason", "wae.explainRule",
      "wae.explainRulePrompt", "wae.showDependencyPath", "wae.inspectCurrentModule",
      "wae.architectureOverview", "wae.refreshArchitecture",
    ]) {
      assert.ok(commands.includes(command), `missing command ${command}`);
    }

    const arch001 = diagnostics.find((diagnostic) => ruleOf(diagnostic) === "ARCH-001");
    assert.ok(typeof arch001?.code === "object" && arch001.code.target.toString().includes("RULES.md#arch-001"),
      "the rule code links to its documentation");
    assert.ok(arch001?.relatedInformation?.some((related) => related.message.startsWith("Path 1/")),
      "ARCH-001 must expose its dependency path as related information");

    const explained = await vscode.commands.executeCommand<{ markdown: string }>(
      "wae.explainRule", { ruleId: "ARCH-001" });
    assert.match(explained.markdown, /### How to fix/);

    const overview = await vscode.commands.executeCommand<{
      workspaces: { ready: boolean; overview: { violations: { ruleId: string }[] } | null }[]
    }>("wae.architectureOverview");
    assert.ok(overview.workspaces[0].ready);
    assert.ok(overview.workspaces[0].overview?.violations.some((group) => group.ruleId === "ARCH-001"));

    const inspection = await vscode.commands.executeCommand<{ id: string; dependents: unknown[] }>(
      "wae.inspectModule", { uri: uri.toString() });
    assert.equal(inspection.id, "src/a.ts");
    assert.ok(inspection.dependents.length > 0);

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
