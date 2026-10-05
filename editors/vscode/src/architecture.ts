import * as vscode from "vscode";

// Mirrors `wae_engine::projection::ArchitectureOverview`; the extension only renders it.
export interface OverviewResponse {
  workspaces: { root: string; ready: boolean; overview: ArchitectureOverview | null }[];
}

interface ModuleGroup { name: string; modules: string[] }

interface OverviewDiagnostic {
  rule_id: string;
  message: string;
  primary_location: { file: string; line: number; column: number } | null;
  dependency_path: string[];
}

interface ArchitectureOverview {
  sourceModules: number;
  dependencies: number;
  packages: ModuleGroup[];
  layers: ModuleGroup[];
  features: ModuleGroup[];
  runtimes: ModuleGroup[];
  violations: { ruleId: string; title: string; category: string; diagnostics: OverviewDiagnostic[] }[];
}

type Node = vscode.TreeItem & { children?: () => Node[] };

const SECTIONS: [keyof ArchitectureOverview, string, string][] = [
  ["packages", "Packages", "package"],
  ["layers", "Layers", "layers"],
  ["features", "Features", "extensions"],
  ["runtimes", "Runtimes", "server-environment"],
];

export class ArchitectureTreeProvider implements vscode.TreeDataProvider<Node> {
  private readonly changed = new vscode.EventEmitter<Node | undefined>();
  readonly onDidChangeTreeData = this.changed.event;
  private overview: Promise<OverviewResponse | undefined> | undefined;

  constructor(private readonly load: () => Promise<OverviewResponse | undefined>) {}

  refresh(): void {
    this.overview = undefined;
    this.changed.fire(undefined);
  }

  getTreeItem(node: Node): vscode.TreeItem {
    return node;
  }

  async getChildren(node?: Node): Promise<Node[]> {
    if (node) return node.children?.() ?? [];
    this.overview ??= this.load().catch(() => undefined);
    const response = await this.overview;
    const workspaces = response?.workspaces ?? [];
    if (workspaces.length === 0) return [message("Waiting for the WAE language server…")];
    if (workspaces.length === 1) return workspaceNodes(workspaces[0]);
    return workspaces.map((workspace) =>
      branch(vscode.Uri.parse(workspace.root).path.split("/").filter(Boolean).pop() ?? workspace.root,
        "root-folder", () => workspaceNodes(workspace)));
  }
}

function workspaceNodes(workspace: OverviewResponse["workspaces"][number]): Node[] {
  const overview = workspace.overview;
  if (!workspace.ready || !overview) return [message("Analyzing…")];
  const root = vscode.Uri.parse(workspace.root);
  const violationCount = overview.violations.reduce((sum, group) => sum + group.diagnostics.length, 0);
  const violations = branch(`Violations (${violationCount})`, violationCount ? "warning" : "pass",
    () => overview.violations.map((group) => branch(
      `${group.ruleId} ${group.title} (${group.diagnostics.length})`, "symbol-ruler",
      () => group.diagnostics.map((diagnostic) => diagnosticNode(root, diagnostic)),
      group.category
    )), `${overview.sourceModules} modules · ${overview.dependencies} dependencies`);
  violations.collapsibleState = violationCount
    ? vscode.TreeItemCollapsibleState.Expanded
    : vscode.TreeItemCollapsibleState.None;
  return [
    violations,
    ...SECTIONS.map(([key, title, icon]) => {
      const groups = overview[key] as ModuleGroup[];
      return branch(`${title} (${groups.length})`, icon, () => groups.map((group) =>
        branch(`${group.name} (${group.modules.length})`, icon,
          () => group.modules.map((module) => moduleNode(root, module)))));
    }),
  ];
}

function diagnosticNode(root: vscode.Uri, diagnostic: OverviewDiagnostic): Node {
  const location = diagnostic.primary_location;
  const item: Node = new vscode.TreeItem(diagnostic.message, vscode.TreeItemCollapsibleState.None);
  item.description = location ? `${location.file}:${location.line}` : undefined;
  item.iconPath = new vscode.ThemeIcon("error");
  item.tooltip = diagnostic.dependency_path.length > 1
    ? `${diagnostic.rule_id}\n${diagnostic.dependency_path.join("\n→ ")}`
    : diagnostic.rule_id;
  if (location) {
    item.command = {
      command: "wae.openLocation",
      title: "Open",
      arguments: [vscode.Uri.joinPath(root, location.file).toString(), location.line, location.column],
    };
  }
  return item;
}

function moduleNode(root: vscode.Uri, module: string): Node {
  const item: Node = new vscode.TreeItem(module, vscode.TreeItemCollapsibleState.None);
  item.iconPath = vscode.ThemeIcon.File;
  item.resourceUri = vscode.Uri.joinPath(root, module);
  item.contextValue = "waeModule";
  item.command = { command: "wae.openLocation", title: "Open", arguments: [item.resourceUri.toString()] };
  return item;
}

function branch(label: string, icon: string, children: () => Node[], description?: string): Node {
  const item: Node = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.Collapsed);
  item.iconPath = new vscode.ThemeIcon(icon);
  item.description = description;
  item.children = children;
  return item;
}

function message(text: string): Node {
  const item: Node = new vscode.TreeItem(text, vscode.TreeItemCollapsibleState.None);
  item.iconPath = new vscode.ThemeIcon("info");
  return item;
}
