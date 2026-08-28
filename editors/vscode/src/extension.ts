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
    vscode.commands.registerCommand("wae.showSuggestion", (suggestion: string) => {
      void vscode.window.showInformationMessage(suggestion);
    })
  );
  await start();
}

function terminalCommand(command: string): void {
  const terminal = vscode.window.createTerminal("WAE");
  terminal.show();
  terminal.sendText(command);
}

export async function deactivate(): Promise<void> {
  if (client) await client.stop();
}
