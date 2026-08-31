mod baseline;
mod discover;
mod explorer;

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::json;
use wae_config::{CONFIG_FILE, Config, ConfigPreset};
use wae_core::domain::{DependencyKind, Diagnostic, ModuleKind};
use wae_engine::{
    Analysis, AnalysisError, AnalyzeRequest, CancellationToken, ChangeSet, Engine, FailurePolicy,
    ImpactAnalyzer, TraceResolutionRequest, VcsPort, trace_resolution, validate_project_config,
};
use wae_reporters::{Format, render};

use crate::CliOutput;

pub fn init(root: &Path, preset: ConfigPreset) -> CliOutput {
    let path = root.join(CONFIG_FILE);
    if path.exists() {
        return CliOutput::success(format!("Configuration already exists: {}", path.display()));
    }
    let config = Config::for_preset(preset);
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

pub fn scan(root: &Path, cancellation: &CancellationToken) -> CliOutput {
    match analyze(root, cancellation) {
        Ok(result) => {
            let source = result
                .project
                .modules
                .iter()
                .filter(|module| module.kind == ModuleKind::Source)
                .count();
            let excluded = result
                .project
                .modules
                .iter()
                .filter(|module| module.kind == ModuleKind::Excluded)
                .count();
            let external = result
                .project
                .modules
                .iter()
                .filter(|module| module.kind == ModuleKind::External)
                .count();
            CliOutput::success(format!(
                "Analyzed {source} source modules, {excluded} excluded modules, {external} external packages, and {} dependencies.",
                result.project.resolved_dependencies.len()
            ))
        }
        Err(output) => output,
    }
}

pub fn discover(root: &Path, json: bool, write: bool, force: bool) -> CliOutput {
    discover::run(root, json, write, force)
}

pub struct CheckOptions {
    pub changed: bool,
    pub format: Option<Format>,
    pub base: Option<String>,
    pub config_path: Option<PathBuf>,
    pub no_cache: bool,
    pub verbose: bool,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegressionSummary {
    affected_modules: usize,
    existing: usize,
    introduced: usize,
    fixed: usize,
}

pub fn check(root: &Path, options: CheckOptions) -> CliOutput {
    let CheckOptions { changed, format, base, config_path, no_cache, verbose, cancellation } =
        options;
    let mut request = AnalyzeRequest::new(root).with_cancellation(cancellation);
    if let Some(path) = &config_path {
        request = request.with_config(path);
    }
    if no_cache {
        request = request.without_cache();
    }
    let mut analysis = match Engine::default().analyze(request).map_err(map_analysis_error) {
        Ok(result) => result,
        Err(output) => return output,
    };
    let mut regression = None;
    if changed {
        let signatures = match baseline::load(root) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        let affected = match affected_modules(root, &analysis, base.as_deref()) {
            Ok(value) => value,
            Err(error) => return CliOutput::project_error(error),
        };
        let current_failures = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| FailurePolicy::is_failure(diagnostic))
            .cloned()
            .collect::<Vec<_>>();
        let existing = current_failures
            .iter()
            .filter(|diagnostic| {
                diagnostic_affects(diagnostic, &affected) && signatures.matches(diagnostic)
            })
            .count();
        analysis.diagnostics.retain(|diagnostic| {
            diagnostic_affects(diagnostic, &affected) && !signatures.matches(diagnostic)
        });
        regression = Some(RegressionSummary {
            affected_modules: affected.len(),
            existing,
            introduced: FailurePolicy::count(&analysis.diagnostics),
            fixed: signatures.len().saturating_sub(signatures.matched_count(&current_failures)),
        });
    }
    let format = match format {
        Some(format) => format,
        None => match config_path
            .as_ref()
            .map_or_else(
                || Config::load(root),
                |path| {
                    let path = if path.is_absolute() { path.clone() } else { root.join(path) };
                    Config::load_file(&path)
                },
            )
            .map_err(|error| error.message)
            .map(|config| config.output.format)
        {
            Ok(format) => format,
            Err(error) => return CliOutput::project_error(error),
        },
    };
    let has_failures = FailurePolicy::count(&analysis.diagnostics) > 0;
    let reporting_started = std::time::Instant::now();
    let rendered = render(&analysis, format)
        .and_then(|output| attach_regression_summary(output, format, regression.as_ref()));
    analysis.timings.record_reporting(reporting_started.elapsed());
    let verbose_report = verbose.then(|| verbose_analysis(&analysis));
    let mut output = match rendered {
        Ok(output) if has_failures => CliOutput::violations(output),
        Ok(output) => CliOutput::success(output),
        Err(error) => CliOutput::internal_error(error.to_string()),
    };
    if let Some(report) = verbose_report {
        output.stderr = report;
    }
    output
}

fn attach_regression_summary(
    output: String,
    format: Format,
    summary: Option<&RegressionSummary>,
) -> Result<String, serde_json::Error> {
    let Some(summary) = summary else { return Ok(output) };
    match format {
        Format::Human => Ok(format!(
            "Regression: {} affected, {} existing, {} introduced, {} fixed\n\n{output}",
            summary.affected_modules, summary.existing, summary.introduced, summary.fixed
        )),
        Format::Json => {
            let mut value: serde_json::Value = serde_json::from_str(&output)?;
            value["regression"] = serde_json::to_value(summary)?;
            serde_json::to_string_pretty(&value)
        }
        Format::Jsonl => {
            let mut lines = output.lines();
            let Some(first) = lines.next() else { return Ok(output) };
            let mut event: serde_json::Value = serde_json::from_str(first)?;
            event["regression"] = serde_json::to_value(summary)?;
            let mut result = vec![serde_json::to_string(&event)?];
            result.extend(lines.map(str::to_owned));
            Ok(result.join("\n"))
        }
        Format::Sarif => {
            let mut value: serde_json::Value = serde_json::from_str(&output)?;
            value["runs"][0]["properties"]["waeRegression"] = serde_json::to_value(summary)?;
            serde_json::to_string_pretty(&value)
        }
    }
}

fn verbose_analysis(analysis: &Analysis) -> String {
    format!(
        "WAE timing: discovery={}ms classification={}ms parsing={}ms resolution={}ms graph={}ms rules={}ms cache={}ms reporting={}ms orchestration={}ms total={}ms\nIncremental: enabled={} restored={} analyzed={} rule-snapshot-reused={}",
        analysis.timings.discovery_ms,
        analysis.timings.classification_ms,
        analysis.timings.parsing_ms,
        analysis.timings.resolution_ms,
        analysis.timings.graph_build_ms,
        analysis.timings.rule_evaluation_ms,
        analysis.timings.cache_ms,
        analysis.timings.reporting_ms,
        analysis.timings.orchestration_ms,
        analysis.timings.total_ms,
        analysis.incremental.cache_enabled,
        analysis.incremental.restored_modules,
        analysis.incremental.analyzed_modules,
        analysis.incremental.rule_snapshot_reused,
    )
}

pub fn baseline_create(root: &Path, cancellation: &CancellationToken) -> CliOutput {
    let analysis = match analyze(root, cancellation) {
        Ok(result) => result,
        Err(output) => return output,
    };
    match baseline::save(root, &analysis.diagnostics) {
        Ok(result) => CliOutput::success(format!(
            "Recorded {} fail-level violations in {} (excluded: {} suppressed, {} informational)",
            result.recorded,
            result.path.display(),
            result.suppressed,
            result.informational,
        )),
        Err(error) => CliOutput::project_error(error),
    }
}

pub fn graph(root: &Path, cancellation: &CancellationToken) -> CliOutput {
    let analysis = match analyze(root, cancellation) {
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

pub fn explore(root: &Path, output: PathBuf, cancellation: &CancellationToken) -> CliOutput {
    let analysis = match analyze(root, cancellation) {
        Ok(result) => result,
        Err(output) => return output,
    };
    explorer::write(root, &output, &analysis)
}

pub fn doctor(root: &Path, cancellation: &CancellationToken) -> CliOutput {
    let mut ok = true;
    let mut report = Vec::new();
    if root.is_dir() {
        report.push(format!("✓ project root: {}", root.display()));
    } else {
        ok = false;
        report.push(format!("✖ project root: {} is not a directory", root.display()));
    }
    match Config::load(root) {
        Ok(config) => {
            report.push(format!("✓ configuration: {}", root.join(CONFIG_FILE).display()));
            if config.architecture.layers.is_empty() {
                report.push(
                    "⚠ architecture: no layers configured; layer ownership is not enforced".into(),
                );
            }
        }
        Err(error) => {
            ok = false;
            report.push(format!("✖ configuration\n  {}", config_error(&error)));
        }
    }
    let git_ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
        .is_ok_and(|output| output.status.success());
    if git_ok {
        report.push("✓ git: available for --changed".into());
    } else {
        report.push("⚠ git: unavailable; only `wae check --changed` is disabled".into());
    }
    match Engine::default()
        .analyze(AnalyzeRequest::new(root).with_cancellation(cancellation.clone()))
    {
        Ok(_) => report.push("✓ analysis: parser → resolver → graph → rules".into()),
        Err(error) => {
            ok = false;
            report.push(format!(
                "✖ analysis\n  {}{}",
                analysis_error_detail(&error),
                analysis_error_suggestions(&error)
            ));
        }
    }
    let report = report.join("\n");
    if ok { CliOutput::success(report) } else { CliOutput::project_error(report) }
}

pub fn config_validate(
    root: &Path,
    show_overlaps: bool,
    show_coverage: bool,
    show_unassigned: bool,
) -> CliOutput {
    match validate_project_config(root) {
        Ok(validation) if !validation.layer_overlaps.is_empty() => {
            let overlaps = if show_overlaps {
                validation.layer_overlaps.as_slice()
            } else {
                &validation.layer_overlaps[..1]
            };
            let mut report = vec![format!(
                "✖ {} module(s) match multiple architecture layers:",
                validation.layer_overlaps.len()
            )];
            for overlap in overlaps {
                report.push(format!("  {}: {}", overlap.module, overlap.layers.join(", ")));
            }
            if !show_overlaps && validation.layer_overlaps.len() > 1 {
                report.push("  Run: wae config validate --show-overlaps".into());
            }
            report.push("\nSuggested fixes:".into());
            let mut layers = overlaps
                .iter()
                .flat_map(|overlap| overlap.layers.iter().cloned())
                .collect::<Vec<_>>();
            layers.sort();
            layers.dedup();
            for layer in layers {
                report.push(format!("- Anchor `{layer}` to `src/{layer}/**`"));
            }
            report.push("- Add an explicit exclude to the broader layer".into());
            CliOutput::project_error(report.join("\n"))
        }
        Ok(validation) => {
            let below_minimum = validation
                .minimum_coverage
                .is_some_and(|minimum| validation.coverage_percent < minimum);
            let mut report = vec![format!(
                "{} configuration valid: {} source modules, no layer overlaps",
                if below_minimum { "✖" } else { "✓" },
                validation.source_modules
            )];
            if validation.blank_architecture && validation.source_modules > 0 {
                report.push(
                    "⚠ no architecture layers are configured; checks cannot enforce layer ownership"
                        .into(),
                );
            }
            if show_coverage || below_minimum {
                report.push(format!(
                    "Coverage: {}% ({} assigned, {} unassigned, {} exempt)",
                    validation.coverage_percent,
                    validation.assigned_modules,
                    validation.unassigned_modules.len(),
                    validation.exempted_modules
                ));
                if let Some(minimum) = validation.minimum_coverage {
                    report.push(format!("Required minimum: {minimum}%"));
                }
            }
            if show_unassigned {
                if validation.unassigned_modules.is_empty() {
                    report.push("Unassigned modules: none".into());
                } else {
                    report.push("Unassigned modules:".into());
                    report.extend(
                        validation.unassigned_modules.iter().map(|module| format!("  {module}")),
                    );
                }
            }
            if below_minimum {
                report.push(
                    "Assign modules to layers or explicitly exempt support paths in `architecture.coverage.allow_unassigned`."
                        .into(),
                );
                CliOutput::project_error(report.join("\n"))
            } else {
                CliOutput::success(report.join("\n"))
            }
        }
        Err(error) => CliOutput::project_error(format!(
            "{}{}",
            analysis_error_detail(&error),
            analysis_error_suggestions(&error)
        )),
    }
}

pub fn explain(rule: &str) -> CliOutput {
    let Some(descriptor) = wae_core::rule_registry::descriptor(rule) else {
        return CliOutput::project_error(format!("Unknown rule id: {rule}"));
    };
    CliOutput::success(format!(
        "{}\n{}: {}",
        descriptor.id, descriptor.title, descriptor.description
    ))
}

pub fn resolve(
    root: &Path,
    importer: PathBuf,
    specifier: String,
    dependency_kind: DependencyKind,
    config_path: Option<PathBuf>,
) -> CliOutput {
    match trace_resolution(TraceResolutionRequest {
        root: root.to_path_buf(),
        importer,
        specifier,
        dependency_kind,
        config_path,
    })
    .map_err(map_analysis_error)
    .and_then(|trace| {
        serde_json::to_string_pretty(&trace)
            .map_err(|error| CliOutput::internal_error(error.to_string()))
    }) {
        Ok(output) => CliOutput::success(output),
        Err(output) => output,
    }
}

fn analyze(root: &Path, cancellation: &CancellationToken) -> Result<Analysis, CliOutput> {
    Engine::default()
        .analyze(AnalyzeRequest::new(root).with_cancellation(cancellation.clone()))
        .map_err(map_analysis_error)
}
fn map_analysis_error(error: AnalysisError) -> CliOutput {
    match error {
        AnalysisError::Config(error) => CliOutput::project_error(config_error(&error)),
        AnalysisError::Project(error) => CliOutput::project_error(error),
        AnalysisError::Internal(error) => CliOutput::internal_error(error),
        AnalysisError::Cancelled => CliOutput::cancelled(),
    }
}
fn config_error(error: &wae_core::domain::ConfigError) -> String {
    format!(
        "Configuration error{}: {}",
        error.path.as_ref().map(|p| format!(" at {p}")).unwrap_or_default(),
        error.message
    )
}

fn analysis_error_detail(error: &AnalysisError) -> String {
    match error {
        AnalysisError::Config(error) => config_error(error),
        AnalysisError::Project(message) => format!("Project error: {message}"),
        AnalysisError::Internal(message) => format!("Internal error: {message}"),
        AnalysisError::Cancelled => "Analysis cancelled".into(),
    }
}

fn analysis_error_suggestions(error: &AnalysisError) -> String {
    match error {
        AnalysisError::Config(error)
            if error.path.as_deref() == Some("architecture.layers") =>
        {
            "\n\n  Suggested fixes:\n  - Anchor layer patterns to project roots such as `src/shared/**`\n  - Add an exclude to the broader layer\n  - Run: wae config validate --show-overlaps".into()
        }
        _ => String::new(),
    }
}

fn affected_modules(
    root: &Path,
    analysis: &Analysis,
    explicit_base: Option<&str>,
) -> Result<HashSet<String>, String> {
    let mut changes = GitVcsAdapter { root }.changes(explicit_base)?;
    for diagnostic in &analysis.diagnostics {
        if diagnostic.rule_id.0 == "RESOLVE-001" {
            if let (Some(location), Some(specifier)) =
                (diagnostic.primary_location.as_ref(), diagnostic.metadata.get("specifier"))
            {
                let candidate_deleted = diagnostic
                    .metadata
                    .get("candidatePaths")
                    .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
                    .is_some_and(|candidates| {
                        candidates.iter().any(|candidate| changes.deleted.contains(candidate))
                    });
                if candidate_deleted
                    || unresolved_target_was_deleted(&location.file, specifier, &changes.deleted)
                {
                    changes.changed.insert(location.file.clone());
                }
            }
        }
    }
    Ok(ImpactAnalyzer::affected(analysis, &changes))
}

struct GitVcsAdapter<'a> {
    root: &'a Path,
}

impl VcsPort for GitVcsAdapter<'_> {
    fn changes(&self, explicit_base: Option<&str>) -> Result<ChangeSet, String> {
        let base = select_base(self.root, explicit_base)?;
        let mut changes = ChangeSet::default();
        let committed_range = format!("{base}...HEAD");
        for args in [
            vec!["diff", "--name-status", "-M", committed_range.as_str()],
            vec!["diff", "--name-status", "-M"],
            vec!["diff", "--cached", "--name-status", "-M"],
        ] {
            let output = git_output(self.root, &args)?;
            collect_name_status(&output, &mut changes.changed, &mut changes.deleted);
        }
        for path in git_output(self.root, &["ls-files", "--others", "--exclude-standard"])?.lines()
        {
            if !path.trim().is_empty() {
                changes.changed.insert(path.replace('\\', "/"));
            }
        }
        Ok(changes)
    }
}

fn collect_name_status(output: &str, changed: &mut HashSet<String>, deleted: &mut HashSet<String>) {
    for line in output.lines() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::Project;

    #[test]
    fn regression_summary_is_machine_readable_in_every_structured_report() {
        let summary =
            RegressionSummary { affected_modules: 4, existing: 2, introduced: 1, fixed: 3 };
        let json = attach_regression_summary("{}".into(), Format::Json, Some(&summary)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["regression"]["fixed"],
            3
        );
        let jsonl = attach_regression_summary(
            r#"{"schemaVersion":1,"type":"analysis"}"#.into(),
            Format::Jsonl,
            Some(&summary),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&jsonl).unwrap()["regression"]["introduced"],
            1
        );
        let sarif =
            attach_regression_summary(r#"{"runs":[{}]}"#.into(), Format::Sarif, Some(&summary))
                .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sarif).unwrap()["runs"][0]["properties"]["waeRegression"]
                ["existing"],
            2
        );
    }

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git").arg("-C").arg(root).args(args).status().unwrap();
        assert!(status.success(), "git command failed: {args:?}");
    }

    #[test]
    fn changed_mode_includes_unstaged_staged_and_untracked_files() {
        let root = std::env::temp_dir().join(format!("wae-changed-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-b", "master"]);
        git(&root, &["config", "user.email", "wae@example.invalid"]);
        git(&root, &["config", "user.name", "WAE Test"]);
        std::fs::write(root.join("tracked.ts"), "export const value = 1;").unwrap();
        git(&root, &["add", "tracked.ts"]);
        git(&root, &["commit", "-m", "initial"]);

        std::fs::write(root.join("tracked.ts"), "export const value = 2;").unwrap();
        std::fs::write(root.join("staged.ts"), "export const staged = true;").unwrap();
        git(&root, &["add", "staged.ts"]);
        std::fs::write(root.join("untracked.ts"), "export const fresh = true;").unwrap();

        let project = Project::default();
        let analysis = Analysis {
            schema_version: 1,
            graph: Default::default(),
            ownership: Default::default(),
            project,
            diagnostics: Vec::new(),
            incremental: Default::default(),
            timings: Default::default(),
        };
        let affected = affected_modules(&root, &analysis, Some("HEAD")).unwrap();
        assert!(affected.contains("tracked.ts"));
        assert!(affected.contains("staged.ts"));
        assert!(affected.contains("untracked.ts"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
