use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use wae_config::Config;
use wae_core::domain::{
    Dependency, Diagnostic, FrameworkMetadata, Layer, Module, ModuleId, ModuleKind, ModulePath,
    Package, PackageName, Project, Runtime, Severity, SourceLocation,
};
use wae_graph::ModuleGraph;
use wae_parser::{JsTsParser, ParserAdapter};
use wae_resolver::{ModuleResolver, PathAlias, Resolution, ResolverPipeline};
use wae_rules::{RuleContext, RuleSet};

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct AnalyzeRequest {
    pub root: PathBuf,
}
impl AnalyzeRequest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub schema_version: u32,
    pub project: Project,
    pub graph: ModuleGraph,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug)]
pub enum AnalysisError {
    Config(wae_core::domain::ConfigError),
    Project(String),
    Internal(String),
}

/// Facade for the complete architecture-analysis subsystem.
pub struct Engine<P = JsTsParser> {
    parser: P,
    rules: RuleSet,
}

impl Default for Engine<JsTsParser> {
    fn default() -> Self {
        Self { parser: JsTsParser, rules: RuleSet::defaults() }
    }
}

impl<P: ParserAdapter> Engine<P> {
    pub fn with_parser(parser: P) -> Self {
        Self { parser, rules: RuleSet::defaults() }
    }

    pub fn analyze(&self, request: AnalyzeRequest) -> Result<Analysis, AnalysisError> {
        let root = request
            .root
            .canonicalize()
            .map_err(|e| AnalysisError::Project(format!("cannot open project root: {e}")))?;
        let config = Config::load(&root).map_err(AnalysisError::Config)?;
        let files = discover_modules(&root, &config)?;
        let aliases = load_aliases(&root);
        let resolver = ResolverPipeline::node_defaults(aliases.0, aliases.1);
        let default_package =
            Package { name: PackageName(project_name(&root)), root_path: normalize(&root) };
        let mut project = Project::default();
        let mut discovered_packages = HashMap::<PackageName, Package>::new();
        let mut layers = HashMap::new();
        let mut features = HashMap::new();

        for path in &files {
            let relative = relative_path(&root, path);
            let id = ModuleId(relative.clone());
            let package = infer_package(&root, &relative, &default_package);
            discovered_packages.entry(package.name.clone()).or_insert_with(|| package.clone());
            let layer_name = infer_layer(&relative, &config);
            if let Some(value) = &layer_name {
                layers.insert(id.clone(), value.clone());
            }
            if let Some(value) = infer_feature(&relative) {
                features.insert(id.clone(), value);
            }
            project.modules.push(Module {
                id: id.clone(),
                path: ModulePath(id.0.clone()),
                package: package.name.clone(),
                kind: ModuleKind::Source,
                runtime: Runtime::Universal,
                layer: compatibility_layer(layer_name.as_deref()),
                framework_metadata: FrameworkMetadata::default(),
            });
        }

        project.packages = discovered_packages.into_values().collect();
        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));

        for path in &files {
            let module_path = ModulePath(normalize(path));
            let module_id = ModuleId(relative_path(&root, path));
            let source = match fs::read_to_string(path) {
                Ok(source) => source,
                Err(error) => {
                    project.diagnostics.push(simple_diagnostic(
                        "PARSE-001",
                        format!("Cannot read source: {error}"),
                        &module_id.0,
                    ));
                    continue;
                }
            };
            match self.parser.parse_imports(&module_path, &source) {
                Ok(imports) => {
                    for mut import in imports {
                        import.module_id = module_id.clone();
                        import.location.file = module_id.0.clone();
                        match resolver.resolve(&module_path, &import.specifier) {
                            Resolution::Module(target) => {
                                let candidate =
                                    wae_core::domain::DependencyCandidate::from(import.clone());
                                project.dependencies.push(Dependency {
                                    from: module_id.clone(),
                                    to: ModuleId(relative_path(&root, Path::new(&target.0))),
                                    kind: candidate.kind,
                                    location: import.location.clone(),
                                });
                            }
                            Resolution::External(name) => {
                                let external_id = ModuleId(format!("external:{name}"));
                                if !project.modules.iter().any(|module| module.id == external_id) {
                                    let external_package = PackageName(name.clone());
                                    project.modules.push(Module {
                                        id: external_id.clone(),
                                        path: ModulePath(format!("external:{name}")),
                                        package: external_package.clone(),
                                        kind: ModuleKind::External,
                                        runtime: Runtime::Unknown,
                                        layer: Layer::Unknown,
                                        framework_metadata: FrameworkMetadata::default(),
                                    });
                                    if !project
                                        .packages
                                        .iter()
                                        .any(|package| package.name == external_package)
                                    {
                                        project.packages.push(Package {
                                            name: external_package,
                                            root_path: String::new(),
                                        });
                                    }
                                }
                                let candidate =
                                    wae_core::domain::DependencyCandidate::from(import.clone());
                                project.dependencies.push(Dependency {
                                    from: module_id.clone(),
                                    to: external_id,
                                    kind: candidate.kind,
                                    location: import.location.clone(),
                                });
                            }
                            Resolution::Unresolved => {
                                project.diagnostics.push(unresolved_diagnostic(&import))
                            }
                        }
                        project.dependency_candidates.push(import.clone().into());
                        project.imports.push(import);
                    }
                }
                Err(error) => {
                    let mut diagnostic =
                        simple_diagnostic("PARSE-001", error.message, &module_id.0);
                    diagnostic.primary_location = error.location.or_else(|| {
                        Some(SourceLocation { file: module_id.0.clone(), line: 1, column: 1 })
                    });
                    diagnostic.refresh_fingerprint();
                    project.diagnostics.push(diagnostic);
                }
            }
        }

        project.modules.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        project.dependencies.sort_by(|a, b| (&a.from.0, &a.to.0).cmp(&(&b.from.0, &b.to.0)));
        let graph = ModuleGraph::from_project(&project);
        let context = RuleContext {
            project: &project,
            graph: &graph,
            config: &config,
            module_layers: &layers,
            module_features: &features,
        };
        let mut diagnostics = project.diagnostics.clone();
        diagnostics.extend(self.rules.evaluate(&context).map_err(AnalysisError::Internal)?);
        diagnostics.sort_by(|a, b| diagnostic_key(a).cmp(&diagnostic_key(b)));
        Ok(Analysis { schema_version: OUTPUT_SCHEMA_VERSION, project, graph, diagnostics })
    }
}

fn discover_modules(root: &Path, config: &Config) -> Result<Vec<PathBuf>, AnalysisError> {
    let include = build_globs(&config.project.include)?;
    let exclude = build_globs(&config.project.exclude)?;
    let mut files = Vec::new();
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(config.project.follow_symlinks)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);
    for entry in builder.build() {
        let entry = entry.map_err(|e| AnalysisError::Project(e.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = relative_path(root, entry.path());
        if !include.is_match(&relative) || exclude.is_match(&relative) {
            continue;
        }
        let length = entry.metadata().map_err(|e| AnalysisError::Project(e.to_string()))?.len();
        if length > config.project.max_file_size_kb.saturating_mul(1024) {
            continue;
        }
        files.push(entry.into_path());
    }
    files.sort();
    Ok(files)
}

fn build_globs(patterns: &[String]) -> Result<GlobSet, AnalysisError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|e| {
            AnalysisError::Config(wae_core::domain::ConfigError {
                kind: wae_core::domain::ConfigErrorKind::InvalidPattern,
                message: e.to_string(),
                path: Some(pattern.clone()),
            })
        })?);
    }
    builder.build().map_err(|e| AnalysisError::Internal(e.to_string()))
}

fn load_aliases(root: &Path) -> (PathBuf, Vec<PathAlias>) {
    let path = root.join("tsconfig.json");
    let Ok(source) = fs::read_to_string(path) else { return (root.to_path_buf(), Vec::new()) };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&source) else {
        return (root.to_path_buf(), Vec::new());
    };
    let compiler = &json["compilerOptions"];
    let base =
        compiler["baseUrl"].as_str().map_or_else(|| root.to_path_buf(), |value| root.join(value));
    let aliases = compiler["paths"]
        .as_object()
        .map(|paths| {
            paths
                .iter()
                .map(|(pattern, targets)| PathAlias {
                    pattern: pattern.clone(),
                    targets: targets
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    (base, aliases)
}

fn infer_layer(path: &str, config: &Config) -> Option<String> {
    config.architecture.layers.iter().find_map(|(name, layer)| {
        build_globs(&layer.patterns).ok().filter(|set| set.is_match(path)).map(|_| name.clone())
    })
}
fn infer_feature(path: &str) -> Option<String> {
    path.split('/')
        .collect::<Vec<_>>()
        .windows(2)
        .rev()
        .find(|pair| pair[0] == "features")
        .map(|pair| pair[1].to_string())
}
fn compatibility_layer(layer: Option<&str>) -> Layer {
    match layer {
        Some("app") => Layer::App,
        Some("features") => Layer::Features,
        Some("entities") => Layer::Entities,
        Some("shared") => Layer::Shared,
        Some("infrastructure") => Layer::Infrastructure,
        _ => Layer::Unknown,
    }
}
fn project_name(root: &Path) -> String {
    root.file_name().and_then(|v| v.to_str()).unwrap_or("project").to_string()
}
fn infer_package(root: &Path, relative: &str, fallback: &Package) -> Package {
    let parts = relative.split('/').collect::<Vec<_>>();
    if parts.len() >= 2 && matches!(parts[0], "apps" | "packages") {
        let name = format!("{}/{}", parts[0], parts[1]);
        return Package { name: PackageName(name.clone()), root_path: normalize(&root.join(name)) };
    }
    fallback.clone()
}
fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}
fn normalize(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn diagnostic_key(diagnostic: &Diagnostic) -> (&str, &str, usize, usize, &str) {
    let location = diagnostic.primary_location.as_ref();
    (
        &diagnostic.rule_id.0,
        location.map_or("", |l| l.file.as_str()),
        location.map_or(0, |l| l.line),
        location.map_or(0, |l| l.column),
        &diagnostic.fingerprint,
    )
}

fn simple_diagnostic(rule: &str, message: String, file: &str) -> Diagnostic {
    let mut diagnostic = Diagnostic::new(rule, message);
    diagnostic.severity = Severity::Error;
    diagnostic.primary_location = Some(SourceLocation { file: file.into(), line: 1, column: 1 });
    diagnostic.refresh_fingerprint();
    diagnostic
}
fn unresolved_diagnostic(import: &wae_core::domain::Import) -> Diagnostic {
    let mut diagnostic =
        Diagnostic::new("RESOLVE-001", format!("Cannot resolve `{}`", import.specifier));
    diagnostic.severity = Severity::Error;
    diagnostic.primary_location = Some(import.location.clone());
    diagnostic.dependency_path = vec![import.module_id.clone()];
    diagnostic.refresh_fingerprint();
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn circular_fixture_is_analyzed_from_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/circular");
        let result = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        assert_eq!(result.diagnostics.iter().filter(|d| d.rule_id.0 == "ARCH-001").count(), 1);
        assert_eq!(result.project.dependencies.len(), 3);
    }

    #[test]
    fn every_fixture_matches_its_golden_expectation_from_real_source() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        for name in ["basic", "circular", "layers", "features", "aliases", "monorepo", "broken"] {
            let root = fixtures.join(name);
            let expected: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(root.join("expected.json")).unwrap())
                    .unwrap();
            let rule = expected["rule"].as_str().unwrap();
            let violations = expected["violations"].as_u64().unwrap() as usize;
            let result = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
            let actual = if rule == "NONE" {
                result.diagnostics.len()
            } else {
                result.diagnostics.iter().filter(|diagnostic| diagnostic.rule_id.0 == rule).count()
            };
            assert_eq!(actual, violations, "fixture `{name}` did not match {rule}");
        }
    }
}
