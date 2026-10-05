#!/usr/bin/env node
"use strict";

// Renders a WAE JSON report (schemaVersion 1) as GitHub workflow annotations and a job summary.
// It only formats the stable machine contract; it never re-evaluates architecture rules.

const fs = require("node:fs");

const MAX_ANNOTATIONS = 50; // GitHub shows at most 50 annotations per step.
const MAX_SUMMARY_ROWS = 100;
const LEVELS = { error: "error", warning: "warning", info: "notice" };

function escapeData(value) {
  return String(value).replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
}

function escapeProperty(value) {
  return escapeData(value).replace(/:/g, "%3A").replace(/,/g, "%2C");
}

function readReport(file) {
  try {
    const text = fs.readFileSync(file, "utf8").trim();
    if (!text) return undefined;
    const report = JSON.parse(text);
    return report && Array.isArray(report.diagnostics) ? report : undefined;
  } catch {
    return undefined;
  }
}

function activeDiagnostics(report) {
  return report.diagnostics.filter((diagnostic) => !diagnostic.suppressed);
}

function annotation(diagnostic, prefix = "") {
  const level = LEVELS[diagnostic.severity] || "warning";
  const location = diagnostic.primary_location;
  const properties = [`title=${escapeProperty(`WAE ${diagnostic.rule_id}`)}`];
  if (location && location.file) {
    properties.unshift(
      `file=${escapeProperty(prefix + location.file)}`,
      `line=${Math.max(1, location.line || 1)}`,
      `col=${Math.max(1, location.column || 1)}`
    );
  }
  const lines = [diagnostic.message];
  const path = diagnostic.dependency_path || [];
  if (path.length > 1) lines.push(`Path: ${path.join(" → ")}`);
  if (diagnostic.suggestion) lines.push(`Suggestion: ${diagnostic.suggestion}`);
  lines.push(`Run \`wae explain ${diagnostic.rule_id}\` for details.`);
  return `::${level} ${properties.join(",")}::${escapeData(lines.join("\n"))}`;
}

function markdownCell(value) {
  return String(value).replace(/\|/g, "\\|").replace(/\r?\n/g, " ");
}

function summary(report, exitCode) {
  if (!report) {
    return `### ❌ WAE architecture check failed\n\nWAE exited with code ${exitCode} before producing a report (2 = configuration/project error, 3 = internal error). See the step log for details.\n`;
  }
  const active = activeDiagnostics(report);
  const failed = exitCode === 1 || report.failureCount > 0;
  const lines = [
    `### ${failed ? "❌ Architecture violations found" : "✅ No architecture violations"}`,
    "",
    `| Modules | Dependencies | Failures | Warnings | Suppressed | Layer coverage |`,
    `|---:|---:|---:|---:|---:|---:|`,
    `| ${report.sourceModules ?? report.modules ?? 0} | ${report.dependencies ?? 0} | ${report.failureCount ?? 0} | ${report.warningCount ?? 0} | ${report.diagnostics.length - active.length} | ${report.architectureCoverage ? `${report.architectureCoverage.percent}%` : "n/a"} |`,
  ];
  if (report.regression) {
    const regression = report.regression;
    lines.push(
      "",
      `**Changed mode:** ${regression.affectedModules} affected modules · ${regression.existing} existing (baseline) · **${regression.introduced} introduced** · ${regression.fixed} fixed`
    );
  }
  if (active.length > 0) {
    lines.push("", "| Rule | Location | Message | Dependency path |", "|---|---|---|---|");
    for (const diagnostic of active.slice(0, MAX_SUMMARY_ROWS)) {
      const location = diagnostic.primary_location;
      const where = location ? `\`${location.file}:${location.line}\`` : "";
      const path = (diagnostic.dependency_path || []).length > 1
        ? diagnostic.dependency_path.map((step) => `\`${step}\``).join(" → ")
        : "";
      lines.push(
        `| [${diagnostic.rule_id}](https://github.com/Don-Erfan/wae/blob/master/docs/RULES.md#${diagnostic.rule_id.toLowerCase()}) | ${markdownCell(where)} | ${markdownCell(diagnostic.message)} | ${markdownCell(path)} |`
      );
    }
    if (active.length > MAX_SUMMARY_ROWS) {
      lines.push("", `_${active.length - MAX_SUMMARY_ROWS} more diagnostics omitted; download the JSON report for the full list._`);
    }
  }
  return `${lines.join("\n")}\n`;
}

function main(argv = process.argv.slice(2), env = process.env, out = process.stdout) {
  const [file, exitCodeText = "0"] = argv;
  const exitCode = Number(exitCodeText);
  const report = readReport(file);
  if (report && env.WAE_ACTION_ANNOTATIONS !== "false") {
    const prefix = env.WAE_ACTION_PATH_PREFIX || "";
    const active = activeDiagnostics(report);
    for (const diagnostic of active.slice(0, MAX_ANNOTATIONS)) out.write(`${annotation(diagnostic, prefix)}\n`);
    if (active.length > MAX_ANNOTATIONS) {
      out.write(`::notice title=WAE::${active.length - MAX_ANNOTATIONS} additional diagnostics are listed in the job summary.\n`);
    }
  }
  if (env.GITHUB_STEP_SUMMARY && env.WAE_ACTION_SUMMARY !== "false") {
    fs.appendFileSync(env.GITHUB_STEP_SUMMARY, summary(report, exitCode));
  }
  if (env.GITHUB_OUTPUT) {
    fs.appendFileSync(
      env.GITHUB_OUTPUT,
      `failure-count=${report ? report.failureCount ?? 0 : ""}\nwarning-count=${report ? report.warningCount ?? 0 : ""}\n`
    );
  }
}

if (require.main === module) main();

module.exports = { annotation, escapeData, escapeProperty, main, readReport, summary };
