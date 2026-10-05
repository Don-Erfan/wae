const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { annotation, main, summary } = require("./report.js");

const report = {
  schemaVersion: 1,
  sourceModules: 3,
  dependencies: 4,
  failureCount: 1,
  warningCount: 1,
  architectureCoverage: { percent: 100 },
  diagnostics: [
    {
      rule_id: "RUNTIME-001",
      severity: "error",
      message: "Browser module transitively depends on server-only code",
      primary_location: { file: "src/app/a,b.tsx", line: 4, column: 29 },
      dependency_path: ["src/app/a,b.tsx", "src/lib/user.ts", "src/server/db.ts"],
      suggestion: "Use a Server Action: 100% safer",
      suppressed: false,
    },
    {
      rule_id: "ARCH-007",
      severity: "warning",
      message: "Outgoing coupling 30 exceeds configured maximum 20",
      primary_location: { file: "src/app/page.tsx", line: 1, column: 1 },
      dependency_path: [],
      suppressed: false,
    },
    {
      rule_id: "ARCH-003",
      severity: "error",
      message: "suppressed one",
      primary_location: { file: "src/legacy.ts", line: 1, column: 1 },
      dependency_path: [],
      suppressed: true,
    },
  ],
};

test("annotations escape workflow-command syntax and carry the dependency path", () => {
  const line = annotation(report.diagnostics[0]);
  assert.match(line, /^::error file=src\/app\/a%2Cb\.tsx,line=4,col=29,title=WAE RUNTIME-001::/);
  assert.match(line, /%0APath: src\/app\/a,b\.tsx → src\/lib\/user\.ts → src\/server\/db\.ts/);
  assert.match(line, /100%25 safer/);
  assert.ok(!line.includes("\n"));
  assert.match(annotation(report.diagnostics[1]), /^::warning /);
});

test("main writes annotations, a summary and step outputs; suppressed diagnostics stay silent", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wae-action-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const file = path.join(dir, "report.json");
  fs.writeFileSync(file, JSON.stringify(report));
  const env = {
    GITHUB_STEP_SUMMARY: path.join(dir, "summary.md"),
    GITHUB_OUTPUT: path.join(dir, "output.txt"),
    WAE_ACTION_PATH_PREFIX: "web/",
  };
  let written = "";
  main([file, "1"], env, { write: (chunk) => { written += chunk; } });
  const lines = written.trim().split("\n");
  assert.equal(lines.length, 2);
  assert.match(lines[0], /file=web\/src\/app\/a%2Cb\.tsx/);
  const markdown = fs.readFileSync(env.GITHUB_STEP_SUMMARY, "utf8");
  assert.match(markdown, /❌ Architecture violations found/);
  assert.match(markdown, /\| 3 \| 4 \| 1 \| 1 \| 1 \| 100% \|/);
  assert.match(markdown, /RULES\.md#runtime-001/);
  assert.ok(!markdown.includes("suppressed one"));
  assert.equal(fs.readFileSync(env.GITHUB_OUTPUT, "utf8"), "failure-count=1\nwarning-count=1\n");
});

test("a missing report produces a failure summary instead of crashing", () => {
  assert.match(summary(undefined, 2), /exited with code 2 before producing a report/);
  const clean = summary({ ...report, failureCount: 0, warningCount: 0, diagnostics: [] }, 0);
  assert.match(clean, /✅ No architecture violations/);
  const changed = summary({ ...report, regression: { affectedModules: 2, existing: 5, introduced: 1, fixed: 3 } }, 1);
  assert.match(changed, /\*\*1 introduced\*\*/);
});
