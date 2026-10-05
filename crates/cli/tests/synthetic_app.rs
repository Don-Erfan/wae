//! Phase 36 acceptance: the synthetic product fixture violates every rule family on purpose and
//! its diagnostics must match `expected-diagnostics.json` exactly. The MCP and LSP test suites
//! assert the same golden file, so all surfaces report identical violations.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/synthetic-app")
}

fn check_json() -> (i32, String) {
    let output = wae_cli::run(
        &["check".into(), "--format".into(), "json".into(), "--no-cache".into()],
        &fixture(),
    );
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    (output.exit_code, output.stdout)
}

/// The stable, surface-independent part of each diagnostic.
fn golden_view(report: &Value) -> Value {
    let diagnostics = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            json!({
                "ruleId": diagnostic["rule_id"],
                "fingerprint": diagnostic["fingerprint"],
                "severity": diagnostic["severity"],
                "file": diagnostic["primary_location"]["file"],
                "line": diagnostic["primary_location"]["line"],
                "column": diagnostic["primary_location"]["column"],
                "message": diagnostic["message"],
                "dependencyPath": diagnostic["dependency_path"],
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schemaVersion": report["schemaVersion"],
        "sourceModules": report["sourceModules"],
        "dependencies": report["dependencies"],
        "failureCount": report["failureCount"],
        "diagnostics": diagnostics,
    })
}

#[test]
fn synthetic_app_matches_the_golden_diagnostics_deterministically() {
    let (exit_code, first) = check_json();
    let (_, second) = check_json();
    assert_eq!(exit_code, wae_cli::EXIT_VIOLATIONS);
    assert_eq!(first, second, "two runs over the same project must produce identical output");

    let actual = golden_view(&serde_json::from_str(&first).unwrap());
    let golden_path = fixture().join("expected-diagnostics.json");
    if std::env::var_os("WAE_UPDATE_GOLDEN").is_some() {
        std::fs::write(&golden_path, serde_json::to_string_pretty(&actual).unwrap() + "\n")
            .unwrap();
    }
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).unwrap()).unwrap();
    assert_eq!(
        actual, expected,
        "synthetic-app diagnostics changed; review and run `WAE_UPDATE_GOLDEN=1 cargo test -p wae-cli --test synthetic_app`"
    );

    // Every intentional violation family is present.
    let rules = actual["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["ruleId"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    for rule in [
        "ARCH-001",
        "ARCH-002",
        "ARCH-003",
        "ARCH-004",
        "ARCH-005",
        "PACKAGE-001",
        "PACKAGE-002",
        "PACKAGE-003",
        "PACKAGE-004",
        "RUNTIME-001",
        "RUNTIME-002",
        "RUNTIME-003",
        "RUNTIME-004",
    ] {
        assert!(rules.contains(rule), "synthetic-app no longer exercises {rule}");
    }
}

#[test]
fn synthetic_app_baseline_ratchet_accepts_existing_and_rejects_new_violations() {
    let root = std::env::temp_dir().join(format!("wae-synthetic-ratchet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    copy_dir(&fixture(), &root);
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .env("GIT_AUTHOR_NAME", "wae")
            .env("GIT_AUTHOR_EMAIL", "wae@example.invalid")
            .env("GIT_COMMITTER_NAME", "wae")
            .env("GIT_COMMITTER_EMAIL", "wae@example.invalid")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "--quiet"]);
    let run = |args: &[&str]| {
        wae_cli::run(&args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(), &root)
    };
    assert_eq!(run(&["baseline"]).exit_code, wae_cli::EXIT_PASSED);
    git(&["add", "."]);
    git(&["commit", "--quiet", "-m", "baseline"]);
    // Existing violations are accepted by the committed baseline.
    let unchanged = run(&["check", "--changed", "--base", "HEAD", "--no-cache"]);
    assert_eq!(
        unchanged.exit_code,
        wae_cli::EXIT_PASSED,
        "{}{}",
        unchanged.stdout,
        unchanged.stderr
    );
    // A newly introduced cross-feature import fails the changed check.
    std::fs::write(
        root.join("apps/web/src/features/checkout/summary.ts"),
        "import { getSession } from \"../user/model/session\";\nexport const summary = getSession;\n",
    )
    .unwrap();
    let regressed =
        run(&["check", "--changed", "--base", "HEAD", "--no-cache", "--format", "json"]);
    assert_eq!(regressed.exit_code, wae_cli::EXIT_VIOLATIONS, "{}", regressed.stderr);
    let report: Value = serde_json::from_str(&regressed.stdout).unwrap();
    assert_eq!(report["failureCount"], 1);
    assert!(report["diagnostics"].as_array().unwrap().iter().any(|diagnostic| {
        diagnostic["rule_id"] == "ARCH-004"
            && diagnostic["primary_location"]["file"] == "apps/web/src/features/checkout/summary.ts"
    }));
    std::fs::remove_dir_all(root).unwrap();
}

fn copy_dir(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            if entry.file_name() != ".wae" {
                copy_dir(&entry.path(), &target);
            }
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
