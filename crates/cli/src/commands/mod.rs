mod baseline;

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::process::Command;

use serde_json::json;
use wae_config::{CONFIG_FILE, Config};
use wae_core::domain::{Diagnostic, Severity};
use wae_engine::{Analysis, AnalysisError, AnalyzeRequest, Engine};
use wae_reporters::{Format, render};

use crate::CliOutput;

pub fn init(root: &Path) -> CliOutput {
    let path = root.join(CONFIG_FILE);
    if path.exists() {
        return CliOutput::success(format!("Configuration already exists: {}", path.display()));
    }
    let config = Config::default();
    match config.to_yaml().and_then(|yaml| {
        std::fs::write(&path, yaml).map_err(|e| wae_core::domain::ConfigError {
            kind: wae_core::domain::ConfigErrorKind::Io,
            message: e.to_string(),
            path: Some(path.display().to_string()),
        })
    }) {
        Ok(()) => CliOutput::success(format!("Created {}", path.display())),
        Err(error) => CliOutput::project_error(config_error(&error)),
    }
}

pub fn scan(root: &Path) -> CliOutput {
    match analyze(root) {
        Ok(result) => CliOutput::success(format!(
            "Analyzed {} modules and {} dependencies.",
            result.project.modules.len(),
            result.project.dependencies.len()
        )),
        Err(output) => output,
    }
}

pub fn check(root: &Path, changed: bool, format: Format) -> CliOutput {
    let mut analysis = match analyze(root) {
        Ok(result) => result,
        Err(output) => return output,
    };
    if changed {
        let signatures = match baseline::load(root) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        let affected = match affected_modules(root, &analysis) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        analysis.diagnostics.retain(|diagnostic| {
            diagnostic_affects(diagnostic, &affected)
                && !signatures.contains(&diagnostic.fingerprint)
        });
    }
    let has_failures = analysis
        .diagnostics
        .iter()
        .any(|d| matches!(d.severity, Severity::Error | Severity::Warning));
    match render(&analysis, format) {
        Ok(output) if has_failures => CliOutput::violations(output),
        Ok(output) => CliOutput::success(output),
        Err(error) => CliOutput::internal_error(error.to_string()),
    }
}

pub fn baseline_create(root: &Path) -> CliOutput {
    let analysis = match analyze(root) {
        Ok(result) => result,
        Err(output) => return output,
    };
    match baseline::save(root, &analysis.diagnostics) {
        Ok(path) => CliOutput::success(format!(
            "Recorded {} violations in {}",
            analysis.diagnostics.len(),
            path.display()
        )),
        Err(error) => CliOutput::project_error(error),
    }
}

pub fn graph(root: &Path) -> CliOutput {
    let analysis = match analyze(root) {
        Ok(result) => result,
        Err(output) => return output,
    };
    let edges = analysis.graph.edges().iter().map(|edge| json!({ "from": edge.from.0, "to": edge.to.0, "kind": format!("{:?}", edge.kind) })).collect::<Vec<_>>();
    match serde_json::to_string_pretty(
        &json!({ "schemaVersion": analysis.schema_version, "nodes": analysis.graph.nodes().iter().map(|n| &n.0).collect::<Vec<_>>(), "edges": edges }),
    ) {
        Ok(value) => CliOutput::success(value),
        Err(error) => CliOutput::internal_error(error.to_string()),
    }
}

pub fn doctor(root: &Path) -> CliOutput {
    let checks = vec![
        ("project root", root.is_dir(), root.display().to_string()),
        ("configuration", Config::load(root).is_ok(), root.join(CONFIG_FILE).display().to_string()),
        (
            "git",
            Command::new("git")
                .arg("-C")
                .arg(root)
                .arg("rev-parse")
                .arg("--is-inside-work-tree")
                .output()
                .is_ok_and(|o| o.status.success()),
            "required for --changed".into(),
        ),
        ("analysis", analyze(root).is_ok(), "parser → resolver → graph → rules".into()),
    ];
    let ok = checks.iter().all(|(_, passed, _)| *passed);
    let report = checks
        .into_iter()
        .map(|(name, passed, detail)| {
            format!("{} {name}: {detail}", if passed { "✓" } else { "✖" })
        })
        .collect::<Vec<_>>()
        .join("\n");
    if ok { CliOutput::success(report) } else { CliOutput::project_error(report) }
}

pub fn explain(rule: &str) -> CliOutput {
    let explanation = match rule {
        "ARCH-001" => "Circular dependency: detects strongly connected module components.",
        "ARCH-002" => {
            "Forbidden dependency: enforces configured and package-to-application boundaries."
        }
        "ARCH-003" => {
            "Layer boundary: enforces canImport while allowing imports within the same layer."
        }
        "ARCH-004" => "Feature boundary: cross-feature dependencies must use public entrypoints.",
        "ARCH-005" => "Private import: internal/private segments cannot be consumed directly.",
        _ => return CliOutput::project_error(format!("Unknown rule id: {rule}")),
    };
    CliOutput::success(format!("{rule}\n{explanation}"))
}

fn analyze(root: &Path) -> Result<Analysis, CliOutput> {
    Engine::default().analyze(AnalyzeRequest::new(root)).map_err(map_analysis_error)
}
fn map_analysis_error(error: AnalysisError) -> CliOutput {
    match error {
        AnalysisError::Config(error) => CliOutput::project_error(config_error(&error)),
        AnalysisError::Project(error) => CliOutput::project_error(error),
        AnalysisError::Internal(error) => CliOutput::internal_error(error),
    }
}
fn config_error(error: &wae_core::domain::ConfigError) -> String {
    format!(
        "Configuration error{}: {}",
        error.path.as_ref().map(|p| format!(" at {p}")).unwrap_or_default(),
        error.message
    )
}

fn affected_modules(root: &Path, analysis: &Analysis) -> Result<HashSet<String>, String> {
    let base = std::env::var("WAE_BASE_REF").unwrap_or_else(|_| "HEAD~1".into());
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-only", &format!("{base}...HEAD")])
        .output()
        .map_err(|e| format!("cannot run git diff: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff failed for base `{base}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let changed = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.replace('\\', "/"))
        .collect::<HashSet<_>>();
    let mut affected = changed.clone();
    let mut queue = VecDeque::from_iter(changed);
    while let Some(module) = queue.pop_front() {
        for importer in analysis.graph.incoming(&wae_core::domain::ModuleId(module)) {
            if affected.insert(importer.0.clone()) {
                queue.push_back(importer.0);
            }
        }
    }
    Ok(affected)
}

fn diagnostic_affects(diagnostic: &Diagnostic, affected: &HashSet<String>) -> bool {
    diagnostic.primary_location.as_ref().is_some_and(|l| affected.contains(&l.file))
        || diagnostic.dependency_path.iter().any(|m| affected.contains(&m.0))
}
