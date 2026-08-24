use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use wae_config::Config;
use wae_core::domain::{
    Dependency, DependencyTarget, Diagnostic, FrameworkMetadata, LayerId, Module, ModuleId,
    ModuleKind, ModulePath, Package, PackageName, Project, ResolvedDependency, Runtime, Severity,
    SourceLocation,
};
use wae_graph::ModuleGraph;
use wae_parser::{JsTsParser, ParserAdapter};
use wae_resolver::{
    ModuleFormat, ModuleResolver, PackageScopeIndex, Resolution, ResolutionKind, ResolutionRequest,
    ResolverPipeline, TsConfigIndex, WorkspacePackage, WorkspacePackageIndex,
};
use wae_rules::{CompiledRulePolicies, RuleContext, RuleSet};

mod architecture_index;
mod cache;
mod diagnostic_arbitrator;
mod discovery;
mod resolution_context;
mod suppression;
use architecture_index::CompiledArchitectureModel;
use cache::{AnalysisCache, stable_hash};
use diagnostic_arbitrator::DiagnosticArbitrator;
use discovery::discover_modules;
use resolution_context::ModuleFormatResolver;

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

/// Single failure policy shared by exit codes and machine-readable reporting.
pub struct FailurePolicy;

impl FailurePolicy {
    pub fn is_failure(diagnostic: &Diagnostic) -> bool {
        !diagnostic.suppressed && matches!(diagnostic.severity, Severity::Error | Severity::Warning)
    }

    pub fn count(diagnostics: &[Diagnostic]) -> usize {
        diagnostics.iter().filter(|diagnostic| Self::is_failure(diagnostic)).count()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub changed: HashSet<String>,
    pub deleted: HashSet<String>,
}

/// Port implemented by Git (or another VCS) outside the analysis engine.
pub trait VcsPort {
    fn changes(&self, base: Option<&str>) -> Result<ChangeSet, String>;
}

pub struct ImpactAnalyzer;

impl ImpactAnalyzer {
    pub fn affected(analysis: &Analysis, changes: &ChangeSet) -> HashSet<String> {
        let mut affected = changes.changed.clone();
        for diagnostic in &analysis.diagnostics {
            if diagnostic.rule_id.0 != "RESOLVE-001" {
                continue;
            }
            let candidate_deleted = diagnostic
                .metadata
                .get("candidatePaths")
                .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
                .is_some_and(|candidates| {
                    candidates.iter().any(|candidate| changes.deleted.contains(candidate))
                });
            if candidate_deleted {
                if let Some(location) = &diagnostic.primary_location {
                    affected.insert(location.file.clone());
                }
            }
        }
        let mut queue = VecDeque::from_iter(affected.iter().cloned());
        while let Some(module) = queue.pop_front() {
            for importer in analysis.graph.incoming(&ModuleId(module)) {
                if affected.insert(importer.0.clone()) {
                    queue.push_back(importer.0);
                }
            }
        }
        affected
    }
}

/// Mutable membership index kept in lockstep with `Project` while the graph is built.
/// It prevents repeated linear scans for every resolved edge in large repositories.
#[derive(Default)]
struct ProjectIndex {
    modules: HashSet<ModuleId>,
    packages: HashSet<PackageName>,
}

impl ProjectIndex {
    fn from_project(project: &Project) -> Self {
        Self {
            modules: project.modules.iter().map(|module| module.id.clone()).collect(),
            packages: project.packages.iter().map(|package| package.name.clone()).collect(),
        }
    }

    fn insert_module(&mut self, id: ModuleId) -> bool {
        self.modules.insert(id)
    }

    fn insert_package(&mut self, name: PackageName) -> bool {
        self.packages.insert(name)
    }
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
        let architecture = CompiledArchitectureModel::compile(&config)?;
        let files = discover_modules(&root, &config)?;
        let tsconfigs = TsConfigIndex::discover(&root).map_err(AnalysisError::Project)?;
        let workspace_resolver =
            WorkspacePackageIndex::discover(&root).map_err(AnalysisError::Project)?;
        let package_scopes = PackageScopeIndex::discover(&root).map_err(AnalysisError::Project)?;
        let workspace_packages = workspace_resolver.packages().to_vec();
        let module_formats = ModuleFormatResolver::new(&package_scopes);
        let resolver = ResolverPipeline::indexed_node_with_workspaces(
            tsconfigs,
            workspace_resolver,
            config.resolution.mode,
        );
        let default_package =
            Package { name: PackageName(project_name(&root)), root_path: normalize(&root) };
        let mut project = Project::default();
        let mut discovered_packages = HashMap::<PackageName, Package>::new();
        let mut layers = HashMap::new();
        let mut features = HashMap::new();
        let mut feature_roots = HashMap::new();
        let mut cache = AnalysisCache::load(&root, &config)?;
        let mut suppressions = Vec::new();

        for path in &files {
            let relative = relative_path(&root, path);
            let id = ModuleId(relative.clone());
            let package = infer_package(&root, path, &workspace_packages, &default_package);
            discovered_packages.entry(package.name.clone()).or_insert_with(|| package.clone());
            let layer_name = architecture.layer(&relative)?;
            if let Some(value) = &layer_name {
                layers.insert(id.clone(), value.clone());
            }
            let package_root = relative_resolved_path(&root, &package.root_path);
            if let Some((feature, feature_root)) =
                architecture.feature(&relative, &package, &package_root)
            {
                features.insert(id.clone(), feature);
                feature_roots.insert(id.clone(), feature_root);
            }
            project.modules.push(Module {
                id: id.clone(),
                path: ModulePath(id.0.clone()),
                package: package.name.clone(),
                kind: ModuleKind::Source,
                runtime: Runtime::Universal,
                layer: layer_name.map(LayerId),
                framework_metadata: FrameworkMetadata::default(),
            });
        }

        project.packages = discovered_packages.into_values().collect();
        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        let mut project_index = ProjectIndex::from_project(&project);

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
            let source_hash = stable_hash(source.as_bytes());
            suppression::collect(
                &module_id.0,
                &source,
                config.suppressions.require_reason,
                &mut suppressions,
                &mut project.diagnostics,
            );
            let parsed = cache
                .get(&module_id.0, source_hash)
                .map(Ok)
                .unwrap_or_else(|| self.parser.parse_imports(&module_path, &source));
            if let Ok(imports) = &parsed {
                cache.insert(module_id.0.clone(), source_hash, imports.clone());
            }
            match parsed {
                Ok(imports) => {
                    for mut import in imports {
                        import.module_id = module_id.clone();
                        import.location.file = module_id.0.clone();
                        let candidate = wae_core::domain::DependencyCandidate::from(import.clone());
                        let importer_format = module_formats.resolve(&module_path);
                        let resolution_kind = match candidate.kind {
                            wae_core::domain::DependencyKind::Dynamic => ResolutionKind::Import,
                            wae_core::domain::DependencyKind::Require => ResolutionKind::Require,
                            _ if importer_format == ModuleFormat::CommonJs => {
                                ResolutionKind::Require
                            }
                            _ => ResolutionKind::Import,
                        };
                        let resolution_request = ResolutionRequest {
                            importer: &module_path,
                            specifier: &import.specifier,
                            dependency_kind: candidate.kind.clone(),
                            resolution_kind,
                            importer_format,
                            mode: config.resolution.mode,
                            custom_conditions: &config.resolution.custom_conditions,
                        };
                        let candidate_paths = resolver
                            .candidate_paths(&resolution_request)
                            .into_iter()
                            .map(|path| relative_resolved_path(&root, &path.0))
                            .collect::<Vec<_>>();
                        match resolver.resolve(&resolution_request) {
                            Resolution::Module(target) => {
                                let target_id = ModuleId(relative_resolved_path(&root, &target.0));
                                let target_kind = workspace_packages
                                    .iter()
                                    .filter(|package| {
                                        normalized_path_is_within(&target.0, &package.root)
                                    })
                                    .max_by_key(|package| package.root.components().count())
                                    .map_or_else(
                                        || DependencyTarget::Internal(target_id.clone()),
                                        |package| DependencyTarget::WorkspacePackage {
                                            package: PackageName(package.name.clone()),
                                            module: target_id.clone(),
                                        },
                                    );
                                if project_index.insert_module(target_id.clone()) {
                                    let target_path = root.join(&target_id.0);
                                    let package = infer_package(
                                        &root,
                                        &target_path,
                                        &workspace_packages,
                                        &default_package,
                                    );
                                    if project_index.insert_package(package.name.clone()) {
                                        project.packages.push(package.clone());
                                    }
                                    let layer = architecture.layer(&target_id.0)?;
                                    if let Some(value) = &layer {
                                        layers.insert(target_id.clone(), value.clone());
                                    }
                                    let package_root =
                                        relative_resolved_path(&root, &package.root_path);
                                    if let Some((feature, feature_root)) =
                                        architecture.feature(&target_id.0, &package, &package_root)
                                    {
                                        features.insert(target_id.clone(), feature);
                                        feature_roots.insert(target_id.clone(), feature_root);
                                    }
                                    project.modules.push(Module {
                                        id: target_id.clone(),
                                        path: ModulePath(target_id.0.clone()),
                                        package: package.name,
                                        kind: ModuleKind::Excluded,
                                        runtime: Runtime::Unknown,
                                        layer: layer.map(LayerId),
                                        framework_metadata: FrameworkMetadata::default(),
                                    });
                                }
                                project.resolved_dependencies.push(ResolvedDependency {
                                    from: module_id.clone(),
                                    specifier: import.specifier.clone(),
                                    kind: candidate.kind.clone(),
                                    target: target_kind,
                                    location: import.location.clone(),
                                });
                                project.dependencies.push(Dependency {
                                    from: module_id.clone(),
                                    to: target_id,
                                    kind: candidate.kind,
                                    location: import.location.clone(),
                                });
                            }
                            Resolution::External(name) => {
                                let external_id = ModuleId(format!("external:{name}"));
                                if project_index.insert_module(external_id.clone()) {
                                    let external_package = PackageName(name.clone());
                                    project.modules.push(Module {
                                        id: external_id.clone(),
                                        path: ModulePath(format!("external:{name}")),
                                        package: external_package.clone(),
                                        kind: ModuleKind::External,
                                        runtime: Runtime::Unknown,
                                        layer: None,
                                        framework_metadata: FrameworkMetadata::default(),
                                    });
                                    if project_index.insert_package(external_package.clone()) {
                                        project.packages.push(Package {
                                            name: external_package,
                                            root_path: String::new(),
                                        });
                                    }
                                }
                                project.resolved_dependencies.push(ResolvedDependency {
                                    from: module_id.clone(),
                                    specifier: import.specifier.clone(),
                                    kind: candidate.kind.clone(),
                                    target: DependencyTarget::ExternalPackage(PackageName(name)),
                                    location: import.location.clone(),
                                });
                                project.dependencies.push(Dependency {
                                    from: module_id.clone(),
                                    to: external_id,
                                    kind: candidate.kind,
                                    location: import.location.clone(),
                                });
                            }
                            Resolution::Unresolved => {
                                project.resolved_dependencies.push(ResolvedDependency {
                                    from: module_id.clone(),
                                    specifier: import.specifier.clone(),
                                    kind: candidate.kind,
                                    target: DependencyTarget::Unresolved {
                                        specifier: import.specifier.clone(),
                                        reason:
                                            "no resolver in the configured chain produced a module"
                                                .into(),
                                    },
                                    location: import.location.clone(),
                                });
                                let mut diagnostic = unresolved_diagnostic(&import);
                                if !candidate_paths.is_empty() {
                                    diagnostic.metadata.insert(
                                        "candidatePaths".into(),
                                        serde_json::to_string(&candidate_paths).map_err(
                                            |error| AnalysisError::Internal(error.to_string()),
                                        )?,
                                    );
                                    diagnostic.refresh_fingerprint();
                                }
                                project.diagnostics.push(diagnostic)
                            }
                            Resolution::Invalid(reason) => {
                                project.resolved_dependencies.push(ResolvedDependency {
                                    from: module_id.clone(),
                                    specifier: import.specifier.clone(),
                                    kind: candidate.kind.clone(),
                                    target: DependencyTarget::Unresolved {
                                        specifier: import.specifier.clone(),
                                        reason: reason.clone(),
                                    },
                                    location: import.location.clone(),
                                });
                                let mut diagnostic =
                                    simple_diagnostic("RESOLVE-002", reason, &module_id.0);
                                diagnostic.primary_location = Some(import.location.clone());
                                diagnostic.refresh_fingerprint();
                                project.diagnostics.push(diagnostic);
                            }
                            Resolution::Redirect(target) => {
                                return Err(AnalysisError::Internal(format!(
                                    "resolver leaked redirect `{target}` out of its pipeline"
                                )));
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

        cache.save()?;

        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        project.modules.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        project.dependencies.sort_by(|a, b| (&a.from.0, &a.to.0).cmp(&(&b.from.0, &b.to.0)));
        let graph = ModuleGraph::from_project(&project);
        let rule_policies =
            CompiledRulePolicies::compile(&config).map_err(AnalysisError::Internal)?;
        let context = RuleContext {
            project: &project,
            graph: &graph,
            config: &config,
            module_layers: &layers,
            module_features: &features,
            module_feature_roots: &feature_roots,
            policies: &rule_policies,
        };
        let mut diagnostics = project.diagnostics.clone();
        diagnostics.extend(self.rules.evaluate(&context).map_err(AnalysisError::Internal)?);
        diagnostics = DiagnosticArbitrator::arbitrate(diagnostics);
        suppression::apply(&mut diagnostics, &mut suppressions, config.suppressions.report_unused);
        diagnostics.sort_by(|a, b| diagnostic_key(a).cmp(&diagnostic_key(b)));
        Ok(Analysis { schema_version: OUTPUT_SCHEMA_VERSION, project, graph, diagnostics })
    }
}

fn project_name(root: &Path) -> String {
    root.file_name().and_then(|v| v.to_str()).unwrap_or("project").to_string()
}
fn infer_package(
    root: &Path,
    path: &Path,
    packages: &[WorkspacePackage],
    fallback: &Package,
) -> Package {
    if let Some(package) = packages
        .iter()
        .filter(|package| path.starts_with(&package.root))
        .max_by_key(|package| package.root.components().count())
    {
        return Package {
            name: PackageName(package.name.clone()),
            root_path: normalize(&package.root),
        };
    }
    let relative = relative_path(root, path);
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

fn relative_resolved_path(root: &Path, resolved: &str) -> String {
    let root = normalize(root);
    let resolved = resolved.replace('\\', "/");
    if resolved == root {
        return String::new();
    }
    resolved
        .strip_prefix(root.trim_end_matches('/'))
        .and_then(|relative| relative.strip_prefix('/'))
        .unwrap_or(&resolved)
        .to_string()
}

fn normalized_path_is_within(resolved: &str, directory: &Path) -> bool {
    let directory = normalize(directory);
    let resolved = resolved.replace('\\', "/");
    resolved == directory
        || resolved
            .strip_prefix(directory.trim_end_matches('/'))
            .is_some_and(|relative| relative.starts_with('/'))
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
    diagnostic.metadata.insert("specifier".into(), import.specifier.clone());
    diagnostic.refresh_fingerprint();
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_parser::PARSER_CACHE_VERSION;

    #[test]
    fn source_suppressions_require_reasons_and_report_unused_directives() {
        let mut directives = Vec::new();
        let mut diagnostics = Vec::new();
        suppression::collect(
            "src/app.ts",
            "// wae-ignore ARCH-003 -- migration ticket ARC-12\nimport './feature';\n// wae-ignore ARCH-004 -- temporary",
            true,
            &mut directives,
            &mut diagnostics,
        );
        let mut violation = Diagnostic::new("ARCH-003", "layer");
        violation.primary_location =
            Some(SourceLocation { file: "src/app.ts".into(), line: 2, column: 1 });
        diagnostics.push(violation);
        suppression::apply(&mut diagnostics, &mut directives, true);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id.0 == "ARCH-003"
                && diagnostic.suppressed
                && diagnostic.suppression_reason.as_deref() == Some("migration ticket ARC-12")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.rule_id.0 == "SUPPRESS-001" && diagnostic.message.contains("Unused")
        }));
    }

    #[test]
    fn source_suppression_without_required_reason_is_invalid() {
        let mut directives = Vec::new();
        let mut diagnostics = Vec::new();
        suppression::collect(
            "src/app.ts",
            "// wae-ignore ARCH-003",
            true,
            &mut directives,
            &mut diagnostics,
        );
        assert!(directives.is_empty());
        assert_eq!(diagnostics[0].rule_id.0, "SUPPRESS-001");
        assert!(diagnostics[0].message.contains("requires a reason"));
    }

    #[test]
    fn resolved_windows_verbatim_paths_become_project_relative_ids() {
        let root = Path::new(r"\\?\D:\a\wae\wae");
        let resolved = "//?/D:/a/wae/wae/src/a.ts";
        assert_eq!(relative_resolved_path(root, "//?/D:/a/wae/wae"), "");
        assert_eq!(relative_resolved_path(root, resolved), "src/a.ts");
        assert!(normalized_path_is_within(resolved, Path::new(r"\\?\D:\a\wae\wae")));
    }

    #[test]
    fn real_twelve_package_monorepo_fixture_resolves_end_to_end() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/monorepo-12");
        let analysis = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        assert_eq!(analysis.project.modules.len(), 12);
        assert_eq!(analysis.project.dependencies.len(), 11);
        assert_eq!(analysis.graph.node_count(), 12);
        assert!(analysis.diagnostics.is_empty(), "{:?}", analysis.diagnostics);
        assert_eq!(
            analysis
                .project
                .resolved_dependencies
                .iter()
                .filter(|dependency| matches!(
                    dependency.target,
                    DependencyTarget::WorkspacePackage { .. }
                ))
                .count(),
            11
        );
    }

    #[test]
    fn realistic_resolution_matrix_covers_esm_cjs_exports_tsconfigs_and_cycles() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/resolution-matrix");
        let analysis = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        assert!(analysis.project.modules.iter().any(|module| module.id.0.ends_with("index.mts")));
        assert!(analysis.project.modules.iter().any(|module| module.id.0.ends_with("index.cts")));
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.rule_id.0 == "ARCH-001")
                .count(),
            1
        );
        assert!(
            !analysis.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RESOLVE-001")
        );
        let esm_type_edges = analysis
            .project
            .resolved_dependencies
            .iter()
            .filter(|dependency| {
                dependency.from.0.ends_with("apps/esm/src/index.mts")
                    && dependency.specifier == "@fixture/domain"
                    && dependency.kind == wae_core::domain::DependencyKind::TypeOnly
            })
            .collect::<Vec<_>>();
        assert_eq!(esm_type_edges.len(), 2);
        assert!(esm_type_edges.iter().all(|dependency| matches!(
            &dependency.target,
            DependencyTarget::WorkspacePackage { module, .. }
                if module.0.ends_with("packages/domain/src/import.d.ts")
        )));
    }

    #[test]
    fn unnamed_root_and_nested_package_scopes_control_static_resolution_format() {
        let root = std::env::temp_dir().join(format!("wae-engine-scopes-{}", std::process::id()));
        let package = root.join("packages/lib");
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"private":true,"type":"module","workspaces":["packages/*"]}"#,
        )
        .unwrap();
        fs::write(root.join("src/nested/package.json"), r#"{"type":"commonjs"}"#).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"pkg","exports":{".":{"import":"./src/import.js","require":"./src/require.cjs"}}}"#,
        )
        .unwrap();
        fs::write(root.join("src/root.ts"), "import value from 'pkg';").unwrap();
        fs::write(root.join("src/nested/app.ts"), "import value from 'pkg';").unwrap();
        fs::write(package.join("src/import.ts"), "export default 'esm';").unwrap();
        fs::write(package.join("src/require.cts"), "export default 'cjs';").unwrap();
        fs::write(root.join("wae.yaml"), "version: 1\narchitecture:\n  layers: {}\n").unwrap();

        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        let target_for = |importer: &str| {
            analysis
                .project
                .resolved_dependencies
                .iter()
                .find(|dependency| dependency.from.0 == importer)
                .and_then(|dependency| match &dependency.target {
                    DependencyTarget::WorkspacePackage { module, .. } => Some(module.0.as_str()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(target_for("src/root.ts"), "packages/lib/src/import.ts");
        assert_eq!(target_for("src/nested/app.ts"), "packages/lib/src/require.cts");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn feature_roots_are_relative_to_each_workspace_package() {
        let root =
            std::env::temp_dir().join(format!("wae-feature-workspace-{}", std::process::id()));
        let app = root.join("apps/web");
        fs::create_dir_all(app.join("src/features/a")).unwrap();
        fs::create_dir_all(app.join("src/features/b")).unwrap();
        fs::write(
            root.join("package.json"),
            r#"{"name":"repo","private":true,"workspaces":["apps/*"]}"#,
        )
        .unwrap();
        fs::write(app.join("package.json"), r#"{"name":"@acme/web"}"#).unwrap();
        fs::write(
            app.join("src/features/a/service.ts"),
            "import { value } from '../b/model'; export { value };",
        )
        .unwrap();
        fs::write(app.join("src/features/b/model.ts"), "export const value = true;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\narchitecture:\n  layers: {}\n  features:\n    root: src/features\nrules:\n  ARCH-004: error\n",
        )
        .unwrap();
        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(analysis.diagnostics.iter().filter(|d| d.rule_id.0 == "ARCH-004").count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn application_code_cannot_import_a_non_public_feature_module() {
        let root = std::env::temp_dir().join(format!("wae-feature-owner-{}", std::process::id()));
        fs::create_dir_all(root.join("src/app")).unwrap();
        fs::create_dir_all(root.join("src/features/user")).unwrap();
        fs::write(
            root.join("src/app/page.ts"),
            "import { user } from '../features/user/model'; export { user };",
        )
        .unwrap();
        fs::write(root.join("src/features/user/model.ts"), "export const user = true;").unwrap();
        fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(analysis.diagnostics.iter().filter(|d| d.rule_id.0 == "ARCH-004").count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolved_targets_outside_discovery_are_explicit_excluded_modules() {
        let root = std::env::temp_dir().join(format!("wae-excluded-target-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "import { value } from './excluded'; export { value };")
            .unwrap();
        fs::write(root.join("src/excluded.ts"), "export const value = true;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nproject:\n  include: ['**/*.ts']\n  exclude: ['**/excluded.ts']\narchitecture:\n  layers: {}\n",
        )
        .unwrap();
        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(analysis.project.modules.len(), 2);
        assert_eq!(analysis.graph.node_count(), 2);
        assert!(
            analysis
                .project
                .modules
                .iter()
                .any(|module| module.id.0 == "src/excluded.ts"
                    && module.kind == ModuleKind::Excluded)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn overlapping_layer_matches_are_configuration_errors() {
        let root = std::env::temp_dir().join(format!("wae-layer-overlap-{}", std::process::id()));
        fs::create_dir_all(root.join("src/app")).unwrap();
        fs::write(root.join("src/app/page.ts"), "export const page = true;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\narchitecture:\n  layers:\n    broad:\n      patterns: ['src/**']\n    app:\n      patterns: ['**/app/**']\n",
        )
        .unwrap();
        let error = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap_err();
        assert!(matches!(error, AnalysisError::Config(_)));
        fs::remove_dir_all(root).unwrap();
    }

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

    #[test]
    fn configured_roots_limit_discovery() {
        let root = std::env::temp_dir().join(format!("wae-roots-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("ignored")).unwrap();
        fs::write(root.join("src/good.ts"), "export const good = true;").unwrap();
        fs::write(root.join("ignored/broken.ts"), "export const broken = ;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nproject:\n  roots: [src]\n  include: ['**/*.ts']\n",
        )
        .unwrap();
        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(analysis.project.modules.len(), 1);
        assert!(analysis.diagnostics.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enabled_cache_is_persisted_and_reusable() {
        let root = std::env::temp_dir().join(format!("wae-cache-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\ncache:\n  enabled: true\n  directory: .wae/cache\n",
        )
        .unwrap();
        Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        let cache = root.join(".wae/cache/imports-v1.json");
        assert!(cache.is_file());
        let lock = root.join(".wae/cache/imports-v1.lock");
        assert!(lock.is_file());
        fs::write(&lock, "stale lock file content from a killed process").unwrap();
        let mut stale: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
        stale["parser_version"] = serde_json::Value::String("stale-parser".into());
        fs::write(&cache, serde_json::to_vec(&stale).unwrap()).unwrap();
        Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        let refreshed: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
        assert_eq!(refreshed["parser_version"], PARSER_CACHE_VERSION);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_cache_writers_leave_a_valid_atomic_cache_file() {
        let root = std::env::temp_dir().join(format!("wae-cache-race-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\ncache:\n  enabled: true\n  directory: .wae/cache\n",
        )
        .unwrap();
        let handles = (0..4)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || Engine::default().analyze(AnalyzeRequest::new(root)))
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        let cache = fs::read_to_string(root.join(".wae/cache/imports-v1.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cache).unwrap();
        assert_eq!(parsed["parser_version"], PARSER_CACHE_VERSION);
        fs::remove_dir_all(root).unwrap();
    }
}
