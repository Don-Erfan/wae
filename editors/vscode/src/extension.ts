import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions } from "vscode-languageclient/node";
import { ArchitectureTreeProvider, OverviewResponse } from "./architecture";

let client: LanguageClient | undefined;

// Server-advertised commands are registered by the language client itself. The extension must
// never register these IDs again; it customizes their UI through middleware instead.
const SERVER = {
  explainRule: "wae.explainRule",
  showDependencyPath: "wae.showDependencyPath",
  inspectModule: "wae.inspectModule",
  architectureOverview: "wae.architectureOverview",
  reanalyze: "wae.reanalyze",
} as const;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const tree = new ArchitectureTreeProvider(() => fetchOverview());
  const start = async (): Promise<void> => {
    const serverOptions = resolveServer();
    const clientOptions: LanguageClientOptions = {
      documentSelector: [
        { scheme: "file", language: "javascript" },
        { scheme: "file", language: "javascriptreact" },
        { scheme: "file", language: "typescript" },
        { scheme: "file", language: "typescriptreact" }
      ],
      synchronize: { fileEvents: vscode.workspace.createFileSystemWatcher("**/{wae.yaml,package.json,tsconfig.json,jsconfig.json}") },
      middleware: {
        executeCommand: async (command, args, next) => {
          if (command === "wae.showSuggestion") {
            const payload = args?.[0] as string | { suggestion?: string } | undefined;
            const suggestion = typeof payload === "string" ? payload : payload?.suggestion;
            await vscode.window.showInformationMessage(
              suggestion ?? "No WAE suggestion was provided."
            );
            return;
          }
          if (command === "wae.suppressWithReason") {
            return addDocumentedSuppression(args?.[0] as SuppressionPayload);
          }
          if (command === SERVER.explainRule) {
            const result = await next(command, [{ ...(args?.[0] ?? {}), notify: false }]) as
              { markdown?: string } | undefined;
            if (result?.markdown) await showMarkdown(result.markdown);
            return result;
          }
          if (command === SERVER.showDependencyPath) {
            const payload = (args?.[0] ?? {}) as DependencyPathPayload;
            await pickDependencyPathStep(payload);
            return { path: payload.path ?? [] };
          }
          return next(command, args);
        }
      }
    };
    client = new LanguageClient("wae", "Web Architecture Engine", serverOptions, clientOptions);
    await client.start();
    tree.refresh();
  };

  let refreshTimer: NodeJS.Timeout | undefined;
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("wae.architecture", tree),
    vscode.languages.onDidChangeDiagnostics(() => {
      if (refreshTimer) clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => tree.refresh(), 400);
    }),
    vscode.commands.registerCommand("wae.check", async () => {
      await vscode.commands.executeCommand(SERVER.reanalyze);
      await vscode.commands.executeCommand("workbench.actions.view.problems");
    }),
    vscode.commands.registerCommand("wae.graph", () => terminalCommand("wae graph")),
    vscode.commands.registerCommand("wae.explainRulePrompt", async (ruleId?: string) => {
      const rule = ruleId ?? await vscode.window.showInputBox({
        prompt: "WAE rule ID",
        placeHolder: "ARCH-004",
        validateInput: (value) => /^[A-Za-z]+-\d{3}$/.test(value.trim()) ? undefined : "Use an ID such as ARCH-004"
      });
      if (rule) await vscode.commands.executeCommand(SERVER.explainRule, { ruleId: rule.trim().toUpperCase() });
    }),
    vscode.commands.registerCommand("wae.inspectCurrentModule", async (target?: vscode.Uri) => {
      const uri = target ?? vscode.window.activeTextEditor?.document.uri;
      if (!uri) {
        void vscode.window.showWarningMessage("Open a JavaScript or TypeScript file to inspect.");
        return;
      }
      const inspection = await vscode.commands.executeCommand<ModuleInspection | null>(
        SERVER.inspectModule, { uri: uri.toString() }
      );
      if (!inspection) {
        void vscode.window.showWarningMessage("WAE has not analyzed this file yet.");
        return;
      }
      await showMarkdown(renderInspection(inspection));
    }),
    vscode.commands.registerCommand("wae.refreshArchitecture", () => tree.refresh()),
    vscode.commands.registerCommand("wae.openLocation", openLocation),
    vscode.commands.registerCommand("wae.reload", async () => {
      if (client) await client.restart(); else await start();
      tree.refresh();
    })
  );
  await start();
}

async function fetchOverview(): Promise<OverviewResponse | undefined> {
  if (!client?.isRunning()) return undefined;
  return vscode.commands.executeCommand<OverviewResponse>(SERVER.architectureOverview);
}

/** Prefers an explicit setting, then the workspace's npm-installed `wae-lsp`, then PATH. */
function resolveServer(): ServerOptions {
  const configured = vscode.workspace.getConfiguration("wae").get<string>("server.path", "wae-lsp");
  if (configured && configured !== "wae-lsp") return { command: configured, args: [] };
  const shim = process.platform === "win32" ? "wae-lsp.cmd" : "wae-lsp";
  for (const folder of vscode.workspace.workspaceFolders ?? []) {
    const local = path.join(folder.uri.fsPath, "node_modules", ".bin", shim);
    if (fs.existsSync(local)) {
      return { command: local, args: [], options: { shell: process.platform === "win32" } };
    }
  }
  return { command: "wae-lsp", args: [] };
}

interface SuppressionPayload {
  uri: string;
  line: number;
  ruleId: string;
  reason?: string;
}

interface DependencyPathPayload {
  uri?: string;
  ruleId?: string;
  path?: string[];
}

interface EdgeView { module: string; kind: string; file: string; line: number; column: number }

interface ModuleInspection {
  id: string;
  kind: string;
  package: string;
  layer: string | null;
  feature: string | null;
  runtime: { runtime: string; source: string; reason: string; propagationPath: string[] | null };
  framework: string | null;
  frameworkAttributes: Record<string, string>;
  dependencies: EdgeView[];
  dependents: EdgeView[];
  diagnostics: { rule_id: string; message: string; dependency_path: string[]; suppressed: boolean }[];
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

async function pickDependencyPathStep(payload: DependencyPathPayload): Promise<void> {
  const steps = payload.path ?? [];
  if (steps.length === 0) return;
  const folder = payload.uri
    ? vscode.workspace.getWorkspaceFolder(vscode.Uri.parse(payload.uri))
    : vscode.workspace.workspaceFolders?.[0];
  const picked = await vscode.window.showQuickPick(
    steps.map((module, index) => ({
      label: `${index + 1}. ${module}`,
      description: index === 0 ? "importer" : index === steps.length - 1 ? "target" : undefined,
      module
    })),
    { title: `${payload.ruleId ?? "WAE"} dependency path`, placeHolder: "Open a module on the path" }
  );
  if (picked && folder && !picked.module.includes(":")) {
    await openLocation(vscode.Uri.joinPath(folder.uri, picked.module).toString());
  }
}

async function openLocation(uri: string, line = 1, column = 1): Promise<void> {
  const document = await vscode.workspace.openTextDocument(vscode.Uri.parse(uri));
  const position = new vscode.Position(Math.max(0, line - 1), Math.max(0, column - 1));
  await vscode.window.showTextDocument(document, { selection: new vscode.Range(position, position) });
}

async function showMarkdown(markdown: string): Promise<void> {
  const document = await vscode.workspace.openTextDocument({ content: markdown, language: "markdown" });
  try {
    await vscode.commands.executeCommand("markdown.showPreview", document.uri);
  } catch {
    // The built-in Markdown extension can be disabled; the raw document is still readable.
    await vscode.window.showTextDocument(document, { preview: true });
  }
}

export function renderInspection(module: ModuleInspection): string {
  const edges = (title: string, list: EdgeView[]) =>
    `\n## ${title} (${list.length})\n\n` +
    (list.length ? list.map((edge) => `- \`${edge.module}\` — ${edge.kind}, ${edge.file}:${edge.line}`).join("\n") : "_none_") + "\n";
  const via = module.runtime.propagationPath ? `\n- Browser via: ${module.runtime.propagationPath.map((step) => `\`${step}\``).join(" → ")}` : "";
  const diagnostics = module.diagnostics.length
    ? module.diagnostics.map((diagnostic) =>
      `- **${diagnostic.rule_id}** ${diagnostic.message}${diagnostic.suppressed ? " _(suppressed)_" : ""}` +
      (diagnostic.dependency_path.length > 1 ? `\n  - path: ${diagnostic.dependency_path.join(" → ")}` : "")).join("\n")
    : "_none_";
  return `# ${module.id}\n\n` +
    `- Package: \`${module.package}\`\n- Layer: \`${module.layer ?? "unassigned"}\`\n- Feature: \`${module.feature ?? "none"}\`\n` +
    `- Runtime: \`${module.runtime.runtime}\` — ${module.runtime.reason}${via}\n` +
    `- Framework: \`${module.framework ?? "none"}\` ${module.frameworkAttributes.role ? `(${module.frameworkAttributes.role})` : ""}\n` +
    edges("Dependencies", module.dependencies) + edges("Dependents", module.dependents) +
    `\n## Diagnostics (${module.diagnostics.length})\n\n${diagnostics}\n`;
}

function terminalCommand(command: string): void {
  const terminal = vscode.window.createTerminal("WAE");
  terminal.show();
  terminal.sendText(command);
}

export async function deactivate(): Promise<void> {
  if (client) await client.stop();
}
