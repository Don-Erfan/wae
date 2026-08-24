mod baseline;

use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
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

pub fn check(
    root: &Path,
    changed: bool,
    format: Option<Format>,
    base: Option<String>,
) -> CliOutput {
    let mut analysis = match analyze(root) {
        Ok(result) => result,
        Err(output) => return output,
    };
    if changed {
        let signatures = match baseline::load(root) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        let affected = match affected_modules(root, &analysis, base.as_deref()) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        analysis.diagnostics.retain(|diagnostic| {
            diagnostic_affects(diagnostic, &affected)
                && !signatures.contains(&diagnostic.fingerprint)
        });
    }
    let format = match format {
        Some(format) => format,
        None => match Config::load(root).map_err(|error| error.message).and_then(|config| {
            Format::parse(&config.output.format)
                .ok_or_else(|| format!("unsupported output format `{}`", config.output.format))
        }) {
            Ok(format) => format,
            Err(error) => return CliOutput::project_error(error),
        },
    };
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

fn affected_modules(
    root: &Path,
    analysis: &Analysis,
    explicit_base: Option<&str>,
) -> Result<HashSet<String>, String> {
    let base = select_base(root, explicit_base)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["diff", "--name-status", "-M", &format!("{base}...HEAD")])
        .output()
        .map_err(|e| format!("cannot run git diff: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff failed for base `{base}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let mut changed = HashSet::new();
    let mut deleted = HashSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        let status = fields.first().copied().unwrap_or_default();
        if status.starts_with('R') || status.starts_with('C') {
            if let Some(old) = fields.get(1) {
                changed.insert(old.replace('\\', "/"));
            }
            if let Some(new) = fields.get(2) {
                changed.insert(new.replace('\\', "/"));
            }
        } else if let Some(path) = fields.get(1) {
            let normalized = path.replace('\\', "/");
            if status.starts_with('D') {
                deleted.insert(normalized.clone());
            }
            changed.insert(normalized);
        }
    }
    for diagnostic in &analysis.diagnostics {
        if diagnostic.rule_id.0 == "RESOLVE-001" {
            if let (Some(location), Some(specifier)) =
                (diagnostic.primary_location.as_ref(), diagnostic.metadata.get("specifier"))
            {
                if unresolved_target_was_deleted(&location.file, specifier, &deleted) {
                    changed.insert(location.file.clone());
                }
            }
        }
    }
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

fn select_base(root: &Path, explicit: Option<&str>) -> Result<String, String> {
    if let Some(base) = explicit {
        return Ok(base.into());
    }
    if let Ok(base) = std::env::var("WAE_BASE_REF") {
        if !base.trim().is_empty() {
            return Ok(base);
        }
    }
    let upstream =
        git_output(root, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"])
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                git_output(root, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]).ok()
            });
    if let Some(upstream) = upstream {
        if let Ok(base) = git_output(root, &["merge-base", "HEAD", &upstream]) {
            return Ok(base);
        }
    }
    Ok("HEAD~1".into())
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn unresolved_target_was_deleted(
    importer: &str,
    specifier: &str,
    deleted: &HashSet<String>,
) -> bool {
    if !specifier.starts_with('.') {
        return false;
    }
    let importer = Path::new(importer);
    let candidate =
        normalize_relative(&importer.parent().unwrap_or(Path::new(".")).join(specifier));
    deleted.iter().any(|path| {
        let deleted_path = Path::new(path);
        normalize_relative(deleted_path) == candidate
            || deleted_path.with_extension("").to_string_lossy().replace('\\', "/") == candidate
            || deleted_path.file_stem().is_some_and(|stem| stem == "index")
                && deleted_path
                    .parent()
                    .is_some_and(|parent| normalize_relative(parent) == candidate)
    })
}

fn normalize_relative(path: &Path) -> String {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().replace('\\', "/")
}

fn diagnostic_affects(diagnostic: &Diagnostic, affected: &HashSet<String>) -> bool {
    diagnostic.primary_location.as_ref().is_some_and(|l| affected.contains(&l.file))
        || diagnostic.dependency_path.iter().any(|m| affected.contains(&m.0))
}
