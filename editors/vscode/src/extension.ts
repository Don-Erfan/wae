import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions } from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const start = async (): Promise<void> => {
    const command = vscode.workspace.getConfiguration("wae").get<string>("server.path", "wae-lsp");
    const serverOptions: ServerOptions = { command, args: [] };
    const clientOptions: LanguageClientOptions = {
      documentSelector: [
        { scheme: "file", language: "javascript" },
        { scheme: "file", language: "javascriptreact" },
        { scheme: "file", language: "typescript" },
        { scheme: "file", language: "typescriptreact" }
      ],
      synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/{wae.yaml,package.json,tsconfig.json,jsconfig.json}") }
    };
    client = new LanguageClient("wae", "Web Architecture Engine", serverOptions, clientOptions);
    await client.start();
  };

  context.subscriptions.push(
    vscode.commands.registerCommand("wae.check", () => terminalCommand("wae check --verbose")),
    vscode.commands.registerCommand("wae.graph", () => terminalCommand("wae graph")),
    vscode.commands.registerCommand("wae.reload", async () => {
      if (client) await client.restart(); else await start();
    }),
    vscode.commands.registerCommand("wae.showSuggestion", (payload: string | { suggestion?: string }) => {
      const suggestion = typeof payload === "string" ? payload : payload?.suggestion;
      void vscode.window.showInformationMessage(suggestion ?? "No WAE suggestion was provided.");
    }),
    vscode.commands.registerCommand("wae.suppressWithReason", addDocumentedSuppression)
  );
  await start();
}

interface SuppressionPayload {
  uri: string;
  line: number;
  ruleId: string;
  reason?: string;
}

async function addDocumentedSuppression(payload: SuppressionPayload): Promise<boolean> {
  if (!payload?.uri || !payload.ruleId || !Number.isInteger(payload.line) || payload.line < 0) {
    throw new Error("WAE suppression command received an invalid location");
  }
  const suppliedReason = payload.reason?.trim();
  const reason = suppliedReason || await vscode.window.showInputBox({
    prompt: `Why is suppressing ${payload.ruleId} safe?`,
    placeHolder: "Reference an architecture decision, migration ticket, or concrete safety reason",
    ignoreFocusOut: true,
    validateInput: validateSuppressionReason
  });
  if (reason === undefined) return false;
  const validationError = validateSuppressionReason(reason);
  if (validationError) {
    void vscode.window.showErrorMessage(validationError);
    return false;
  }
  const uri = vscode.Uri.parse(payload.uri);
  const document = await vscode.workspace.openTextDocument(uri);
  if (payload.line >= document.lineCount) {
    throw new Error(`WAE suppression line ${payload.line} is outside ${uri.fsPath}`);
  }
  const indentation = document.lineAt(payload.line).text.match(/^\s*/)?.[0] ?? "";
  const edit = new vscode.WorkspaceEdit();
  edit.insert(uri, new vscode.Position(payload.line, 0),
    `${indentation}// wae-ignore ${payload.ruleId} -- ${reason.trim()}\n`);
  const applied = await vscode.workspace.applyEdit(edit);
  if (!applied) void vscode.window.showErrorMessage("VS Code could not apply the WAE suppression edit.");
  return applied;
}

function validateSuppressionReason(value: string): string | undefined {
  const reason = value.trim();
  if (reason.length < 8) return "Enter a specific reason of at least 8 characters.";
  if (/\r|\n/.test(reason)) return "The reason must be a single line.";
  if (/^(todo|fixme|reason|explain why|temporary|n\/a)$/i.test(reason)) {
    return "Replace the placeholder with a concrete architecture or migration reason.";
  }
  return undefined;
}

function terminalCommand(command: string): void {
  const terminal = vscode.window.createTerminal("WAE");
  terminal.show();
  terminal.sendText(command);
}

export async function deactivate(): Promise<void> {
  if (client) await client.stop();
}
