use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use wae_config::Config;
use wae_core::domain::{
    Dependency, DependencyTarget, Diagnostic, FeatureId, FrameworkMetadata, LayerId, Module,
    ModuleId, ModuleKind, ModulePath, Package, PackageName, Project, ResolvedDependency, Runtime,
    Severity, SourceLocation,
};
use wae_framework::{FrameworkAdapter, FrameworkRegistry, ModuleEvidence, ProjectEvidence};
use wae_graph::{ModuleGraph, PackageGraph, RuntimeGraph};
use wae_parser::{JsTsParser, ParserAdapter};
use wae_resolver::{
    BundlerConditions, ConditionSetProvider, ModuleResolver, Node16Conditions, NodeNextConditions,
    PackageScopeIndex, Resolution, ResolutionRequest, ResolverPipeline, TsConfigIndex,
    WorkspacePackage, WorkspacePackageIndex, resolution_kind_for,
};
use wae_rules::{CompiledRulePolicies, RuleContext, RuleSet};

mod architecture_index;
mod cache;
mod diagnostic_arbitrator;
mod discovery;
mod resolution_context;
mod suppression;
use architecture_index::CompiledArchitectureModel;
use cache::{AnalysisCache, CachedModuleAnalysis, stable_hash};
use diagnostic_arbitrator::DiagnosticArbitrator;
use discovery::discover_modules;
use resolution_context::ModuleFormatResolver;

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct AnalyzeRequest {
    pub root: PathBuf,
    pub config_path: Option<PathBuf>,
    pub cache_enabled: Option<bool>,
    pub overlays: BTreeMap<String, String>,
    pub cancellation: CancellationToken,
}
impl AnalyzeRequest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            config_path: None,
            cache_enabled: None,
            overlays: BTreeMap::new(),
            cancellation: CancellationToken::default(),
        }
    }
    pub fn with_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }
    pub fn without_cache(mut self) -> Self {
        self.cache_enabled = Some(false);
        self
    }
    pub fn with_overlay(mut self, module: impl Into<String>, source: impl Into<String>) -> Self {
        self.overlays.insert(module.into(), source.into());
        self.cache_enabled = Some(false);
        self
    }
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub schema_version: u32,
    pub project: Project,
    pub graph: ModuleGraph,
    pub diagnostics: Vec<Diagnostic>,
    pub incremental: IncrementalStats,
    pub timings: AnalysisTimings,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnalysisTimings {
    pub discovery_ms: u128,
    pub module_analysis_ms: u128,
    pub graph_ms: u128,
    pub rules_ms: u128,
    pub total_ms: u128,
}

#[derive(Clone, Debug)]
pub struct TraceResolutionRequest {
    pub root: PathBuf,
    pub importer: PathBuf,
    pub specifier: String,
    pub dependency_kind: wae_core::domain::DependencyKind,
    pub config_path: Option<PathBuf>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionTrace {
    pub importer: String,
    pub specifier: String,
    pub dependency_kind: String,
    pub mode: String,
    pub importer_format: String,
    pub resolution_kind: String,
    pub active_conditions: Vec<String>,
    pub candidate_paths: Vec<String>,
    pub attempts: Vec<ResolutionTraceAttempt>,
    pub outcome: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionTraceAttempt {
    pub specifier: String,
    pub handler: String,
    pub outcome: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IncrementalStats {
    pub cache_enabled: bool,
    pub restored_modules: usize,
    pub analyzed_modules: usize,
    pub rule_snapshot_reused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerOverlap {
    pub module: String,
    pub layers: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigValidation {
    pub source_modules: usize,
    pub layer_overlaps: Vec<LayerOverlap>,
}

pub fn validate_project_config(root: impl AsRef<Path>) -> Result<ConfigValidation, AnalysisError> {
    let root = root
        .as_ref()
        .canonicalize()
        .map_err(|error| AnalysisError::Project(format!("cannot open project root: {error}")))?;
    let config = Config::load(&root).map_err(AnalysisError::Config)?;
    let architecture = CompiledArchitectureModel::compile(&config)?;
    let files = discover_modules(&root, &config)?;
    let mut layer_overlaps = files
        .iter()
        .filter_map(|path| {
            let module = relative_path(&root, path);
            let layers = architecture.matching_layers(&module);
            (layers.len() > 1).then_some(LayerOverlap { module, layers })
        })
        .collect::<Vec<_>>();
    layer_overlaps.sort_by(|left, right| left.module.cmp(&right.module));
    Ok(ConfigValidation { source_modules: files.len(), layer_overlaps })
}

pub fn trace_resolution(request: TraceResolutionRequest) -> Result<ResolutionTrace, AnalysisError> {
    let root = request
        .root
        .canonicalize()
        .map_err(|error| AnalysisError::Project(format!("cannot open project root: {error}")))?;
    let importer =
        if request.importer.is_absolute() { request.importer } else { root.join(request.importer) }
            .canonicalize()
            .map_err(|error| AnalysisError::Project(format!("cannot open importer: {error}")))?;
    if !importer.starts_with(&root) {
        return Err(AnalysisError::Project("importer escapes the project root".into()));
    }
    let config = match request.config_path {
        Some(path) => {
            let path = if path.is_absolute() { path } else { root.join(path) };
            Config::load_file(&path).map_err(AnalysisError::Config)?
        }
        None => Config::load(&root).map_err(AnalysisError::Config)?,
    };
    let tsconfigs = TsConfigIndex::discover(&root).map_err(AnalysisError::Project)?;
    let workspaces = WorkspacePackageIndex::discover(&root).map_err(AnalysisError::Project)?;
    let scopes = PackageScopeIndex::from_importers(&root, std::slice::from_ref(&importer))
        .map_err(AnalysisError::Project)?;
    let formats = ModuleFormatResolver::new(&scopes);
    let importer_format = formats.resolve(&importer);
    let resolution_kind =
        resolution_kind_for(config.resolution.mode, &request.dependency_kind, importer_format);
    let importer_path = ModulePath(normalize(&importer));
    let resolution_request = ResolutionRequest {
        importer: &importer_path,
        specifier: &request.specifier,
        dependency_kind: request.dependency_kind.clone(),
        resolution_kind,
        importer_format,
        mode: config.resolution.mode,
        custom_conditions: &config.resolution.custom_conditions,
    };
    let resolver = ResolverPipeline::indexed_node_with_workspaces(
        tsconfigs,
        workspaces,
        config.resolution.mode,
    );
    let candidate_paths = resolver
        .candidate_paths(&resolution_request)
        .into_iter()
        .map(|candidate| relative_resolved_path(&root, &candidate.0))
        .collect();
    let active_conditions = match config.resolution.mode {
        wae_config::ResolutionMode::Node10 | wae_config::ResolutionMode::Node16 => {
            Node16Conditions.active_conditions(&resolution_request)
        }
        wae_config::ResolutionMode::NodeNext => {
            NodeNextConditions.active_conditions(&resolution_request)
        }
        wae_config::ResolutionMode::Bundler => {
            BundlerConditions.active_conditions(&resolution_request)
        }
    }
    .iter()
    .map(str::to_owned)
    .collect();
    let (outcome, attempts) = resolver.resolve_with_trace(&resolution_request);
    Ok(ResolutionTrace {
        importer: relative_path(&root, &importer),
        specifier: request.specifier,
        dependency_kind: format!("{:?}", request.dependency_kind),
        mode: format!("{:?}", config.resolution.mode),
        importer_format: format!("{importer_format:?}"),
        resolution_kind: format!("{resolution_kind:?}"),
        active_conditions,
        candidate_paths,
        attempts: attempts
            .into_iter()
            .map(|attempt| ResolutionTraceAttempt {
                specifier: attempt.specifier,
                handler: attempt.handler.into(),
                outcome: attempt.outcome.map(|outcome| resolution_text(&root, &outcome)),
            })
            .collect(),
        outcome: resolution_text(&root, &outcome),
    })
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
    Cancelled,
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
        let total_started = std::time::Instant::now();
        let AnalyzeRequest {
            root: requested_root,
            config_path,
            cache_enabled,
            overlays,
            cancellation,
        } = request;
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        let root = requested_root
            .canonicalize()
            .map_err(|e| AnalysisError::Project(format!("cannot open project root: {e}")))?;
        let mut config = match config_path {
            Some(path) => {
                let path = if path.is_absolute() { path } else { root.join(path) };
                Config::load_file(&path).map_err(AnalysisError::Config)?
            }
            None => Config::load(&root).map_err(AnalysisError::Config)?,
        };
        if let Some(enabled) = cache_enabled {
            config.cache.enabled = enabled;
        }
        let discovery_started = std::time::Instant::now();
        let architecture = CompiledArchitectureModel::compile(&config)?;
        let files = discover_modules(&root, &config)?;
        let framework_registry = FrameworkRegistry::default();
        let framework_evidence = framework_project_evidence(&root)?;
        let framework_adapter = framework_registry.select(
            &framework_evidence,
            &config.framework.enabled,
            config.framework.auto_detect,
        );
        let tsconfigs = TsConfigIndex::discover(&root).map_err(AnalysisError::Project)?;
        let workspace_resolver =
            WorkspacePackageIndex::discover(&root).map_err(AnalysisError::Project)?;
        let package_scopes =
            PackageScopeIndex::from_importers(&root, &files).map_err(AnalysisError::Project)?;
        let workspace_packages = workspace_resolver.packages().to_vec();
        let declared_package_dependencies = workspace_packages
            .iter()
            .map(|package| {
                (
                    PackageName(package.name.clone()),
                    package
                        .declared_dependencies
                        .iter()
                        .cloned()
                        .map(PackageName)
                        .collect::<HashSet<_>>(),
                )
            })
            .collect::<HashMap<_, _>>();
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
        let live_cache_files = files.iter().map(|path| relative_path(&root, path)).collect();
        let mut cache = AnalysisCache::load(&root, &config, live_cache_files)?;
        let environment_hash = analysis_environment_hash(&root, &config)?;
        let mut incremental =
            IncrementalStats { cache_enabled: cache.enabled(), ..Default::default() };
        let mut suppressions = Vec::new();
        let discovery_ms = discovery_started.elapsed().as_millis();
        let module_analysis_started = std::time::Instant::now();

        for path in &files {
            if cancellation.is_cancelled() {
                return Err(AnalysisError::Cancelled);
            }
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
        let module_analysis_ms = module_analysis_started.elapsed().as_millis();
        let graph_started = std::time::Instant::now();
        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        let mut project_index = ProjectIndex::from_project(&project);

        for path in &files {
            if cancellation.is_cancelled() {
                return Err(AnalysisError::Cancelled);
            }
            let module_path = ModulePath(normalize(path));
            let module_id = ModuleId(relative_path(&root, path));
            let source = match overlays
                .get(&module_id.0)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| fs::read_to_string(path))
            {
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
            if let Some(adapter) = framework_adapter {
                let classification =
                    adapter.classify(ModuleEvidence { path: &module_id.0, source: &source });
                if let Some(module) =
                    project.modules.iter_mut().find(|module| module.id == module_id)
                {
                    module.framework_metadata = classification.metadata;
                    module.runtime = classification.runtime;
                }
            }
            suppression::collect(
                &module_id.0,
                &source,
                config.suppressions.require_reason,
                &mut suppressions,
                &mut project.diagnostics,
            );
            if let Some(cached) = cache.get(&module_id.0, source_hash, environment_hash) {
                incremental.restored_modules += 1;
                restore_cached_module(
                    cached,
                    &root,
                    &workspace_packages,
                    &default_package,
                    framework_adapter,
                    &architecture,
                    &mut project,
                    &mut project_index,
                    &mut layers,
                    &mut features,
                    &mut feature_roots,
                )?;
                continue;
            }
            incremental.analyzed_modules += 1;
            let imports_start = project.imports.len();
            let dependencies_start = project.dependencies.len();
            let resolved_start = project.resolved_dependencies.len();
            let diagnostics_start = project.diagnostics.len();
            let parsed = self.parser.parse_imports(&module_path, &source);
            match parsed {
                Ok(imports) => {
                    for mut import in imports {
                        if cancellation.is_cancelled() {
                            return Err(AnalysisError::Cancelled);
                        }
                        import.module_id = module_id.clone();
                        import.location.file = module_id.0.clone();
                        let candidate = wae_core::domain::DependencyCandidate::from(import.clone());
                        let importer_format = module_formats.resolve(path);
                        let resolution_kind = resolution_kind_for(
                            config.resolution.mode,
                            &candidate.kind,
                            importer_format,
                        );
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
                                        runtime: framework_adapter.map_or(
                                            Runtime::Unknown,
                                            |adapter| {
                                                adapter
                                                    .classify(ModuleEvidence {
                                                        path: &target_id.0,
                                                        source: "",
                                                    })
                                                    .runtime
                                            },
                                        ),
                                        layer: layer.map(LayerId),
                                        framework_metadata: framework_adapter.map_or_else(
                                            FrameworkMetadata::default,
                                            |adapter| {
                                                adapter
                                                    .classify(ModuleEvidence {
                                                        path: &target_id.0,
                                                        source: "",
                                                    })
                                                    .metadata
                                            },
                                        ),
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
            cache.insert(
                module_id.0.clone(),
                source_hash,
                environment_hash,
                CachedModuleAnalysis {
                    hash: source_hash,
                    environment_hash,
                    imports: project.imports[imports_start..].to_vec(),
                    dependencies: project.dependencies[dependencies_start..].to_vec(),
                    resolved_dependencies: project.resolved_dependencies[resolved_start..].to_vec(),
                    diagnostics: project.diagnostics[diagnostics_start..].to_vec(),
                    resolved_paths: Vec::new(),
                },
            );
        }

        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        project.modules.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        project.dependencies.sort_by(|a, b| (&a.from.0, &a.to.0).cmp(&(&b.from.0, &b.to.0)));
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        let graph = ModuleGraph::from_project(&project);
        let package_graph = PackageGraph::from_project(&project);
        let runtime_graph = RuntimeGraph::from_project(&project);
        let graph_ms = graph_started.elapsed().as_millis();
        let rules_started = std::time::Instant::now();
        let rule_policies =
            CompiledRulePolicies::compile(&config).map_err(AnalysisError::Internal)?;
        let context = RuleContext {
            project: &project,
            graph: &graph,
            package_graph: &package_graph,
            runtime_graph: &runtime_graph,
            config: &config,
            module_layers: &layers,
            module_features: &features,
            module_feature_roots: &feature_roots,
            policies: &rule_policies,
            declared_package_dependencies: &declared_package_dependencies,
        };
        let mut diagnostics = project.diagnostics.clone();
        let graph_hash = analysis_graph_hash(&project, environment_hash)?;
        let rule_diagnostics = if let Some(diagnostics) = cache.rule_diagnostics(graph_hash) {
            incremental.rule_snapshot_reused = true;
            diagnostics
        } else {
            let diagnostics = self.rules.evaluate(&context).map_err(AnalysisError::Internal)?;
            cache.set_rule_diagnostics(graph_hash, diagnostics.clone());
            diagnostics
        };
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        diagnostics.extend(rule_diagnostics);
        diagnostics = DiagnosticArbitrator::arbitrate(diagnostics);
        suppression::apply(&mut diagnostics, &mut suppressions, config.suppressions.report_unused);
        diagnostics.sort_by(|a, b| diagnostic_key(a).cmp(&diagnostic_key(b)));
        if cancellation.is_cancelled() {
            return Err(AnalysisError::Cancelled);
        }
        cache.save()?;
        let rules_ms = rules_started.elapsed().as_millis();
        let timings = AnalysisTimings {
            discovery_ms,
            module_analysis_ms,
            graph_ms,
            rules_ms,
            total_ms: total_started.elapsed().as_millis(),
        };
        Ok(Analysis {
            schema_version: OUTPUT_SCHEMA_VERSION,
            project,
            graph,
            diagnostics,
            incremental,
            timings,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn restore_cached_module(
    cached: CachedModuleAnalysis,
    root: &Path,
    workspace_packages: &[WorkspacePackage],
    default_package: &Package,
    framework_adapter: Option<&dyn FrameworkAdapter>,
    architecture: &CompiledArchitectureModel,
    project: &mut Project,
    project_index: &mut ProjectIndex,
    layers: &mut HashMap<ModuleId, String>,
    features: &mut HashMap<ModuleId, FeatureId>,
    feature_roots: &mut HashMap<ModuleId, String>,
) -> Result<(), AnalysisError> {
    for dependency in &cached.dependencies {
        if !project_index.insert_module(dependency.to.clone()) {
            continue;
        }
        let resolved = cached
            .resolved_dependencies
            .iter()
            .find(|resolved| {
                resolved.from == dependency.from
                    && resolved.kind == dependency.kind
                    && resolved.location == dependency.location
            })
            .ok_or_else(|| {
                AnalysisError::Internal(format!(
                    "cached edge `{}` -> `{}` has no resolution record",
                    dependency.from.0, dependency.to.0
                ))
            })?;
        if let DependencyTarget::ExternalPackage(package_name) = &resolved.target {
            let package = Package { name: package_name.clone(), root_path: String::new() };
            if project_index.insert_package(package.name.clone()) {
                project.packages.push(package.clone());
            }
            project.modules.push(Module {
                id: dependency.to.clone(),
                path: ModulePath(dependency.to.0.clone()),
                package: package.name,
                kind: ModuleKind::External,
                runtime: Runtime::Unknown,
                layer: None,
                framework_metadata: FrameworkMetadata::default(),
            });
            continue;
        }

        let target_path = root.join(&dependency.to.0);
        let package = infer_package(root, &target_path, workspace_packages, default_package);
        if project_index.insert_package(package.name.clone()) {
            project.packages.push(package.clone());
        }
        let layer = architecture.layer(&dependency.to.0)?;
        if let Some(value) = &layer {
            layers.insert(dependency.to.clone(), value.clone());
        }
        let package_root = relative_resolved_path(root, &package.root_path);
        if let Some((feature, feature_root)) =
            architecture.feature(&dependency.to.0, &package, &package_root)
        {
            features.insert(dependency.to.clone(), feature);
            feature_roots.insert(dependency.to.clone(), feature_root);
        }
        let classification = framework_adapter
            .map(|adapter| adapter.classify(ModuleEvidence { path: &dependency.to.0, source: "" }));
        project.modules.push(Module {
            id: dependency.to.clone(),
            path: ModulePath(dependency.to.0.clone()),
            package: package.name,
            kind: ModuleKind::Excluded,
            runtime: classification.as_ref().map_or(Runtime::Unknown, |value| value.runtime),
            layer: layer.map(LayerId),
            framework_metadata: classification
                .map_or_else(FrameworkMetadata::default, |value| value.metadata),
        });
    }
    project.dependency_candidates.extend(cached.imports.iter().cloned().map(Into::into));
    project.imports.extend(cached.imports);
    project.dependencies.extend(cached.dependencies);
    project.resolved_dependencies.extend(cached.resolved_dependencies);
    project.diagnostics.extend(cached.diagnostics);
    Ok(())
}

fn analysis_environment_hash(root: &Path, config: &Config) -> Result<u64, AnalysisError> {
    let mut inputs = Vec::<PathBuf>::new();
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true).filter_entry(
        |entry| {
            !entry.file_type().is_some_and(|kind| kind.is_dir())
                || !matches!(
                    entry.file_name().to_string_lossy().as_ref(),
                    "node_modules" | ".git" | ".wae" | ".next" | "dist" | "build" | "target"
                )
        },
    );
    for entry in builder.build() {
        let entry = entry.map_err(|error| AnalysisError::Project(error.to_string()))?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if matches!(
            name.as_ref(),
            "package.json"
                | "tsconfig.json"
                | "jsconfig.json"
                | "next.config.js"
                | "next.config.mjs"
                | "next.config.cjs"
                | "next.config.ts"
        ) {
            inputs.push(entry.into_path());
        }
    }
    inputs.sort();
    let mut identity = format!("wae-analysis-v2\n{config:?}").into_bytes();
    for path in inputs {
        identity.extend_from_slice(relative_path(root, &path).as_bytes());
        identity.push(0);
        identity.extend_from_slice(&fs::read(&path).map_err(|error| {
            AnalysisError::Project(format!(
                "cannot fingerprint analysis input `{}`: {error}",
                path.display()
            ))
        })?);
        identity.push(0xff);
    }
    Ok(stable_hash(&identity))
}

fn analysis_graph_hash(project: &Project, environment_hash: u64) -> Result<u64, AnalysisError> {
    let identity = serde_json::to_vec(&(
        environment_hash,
        &project.modules,
        &project.dependencies,
        &project.resolved_dependencies,
    ))
    .map_err(|error| AnalysisError::Internal(error.to_string()))?;
    Ok(stable_hash(&identity))
}

fn framework_project_evidence(root: &Path) -> Result<ProjectEvidence, AnalysisError> {
    let manifest_path = root.join("package.json");
    let package_manifest = manifest_path
        .exists()
        .then(|| {
            fs::read_to_string(&manifest_path)
                .map_err(|error| {
                    AnalysisError::Project(format!(
                        "cannot read framework manifest `{}`: {error}",
                        manifest_path.display()
                    ))
                })
                .and_then(|source| {
                    serde_json::from_str(&source).map_err(|error| {
                        AnalysisError::Project(format!(
                            "invalid framework manifest `{}`: {error}",
                            manifest_path.display()
                        ))
                    })
                })
        })
        .transpose()?;
    let config_files = ["next.config.js", "next.config.mjs", "next.config.cjs", "next.config.ts"]
        .into_iter()
        .filter(|name| root.join(name).is_file())
        .map(str::to_owned)
        .collect();
    Ok(ProjectEvidence { package_manifest, config_files })
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
    let resolved = normalize_text_path(resolved);
    if resolved == root {
        return String::new();
    }
    resolved
        .strip_prefix(root.trim_end_matches('/'))
        .and_then(|relative| relative.strip_prefix('/'))
        .unwrap_or(&resolved)
        .to_string()
}

fn resolution_text(root: &Path, resolution: &Resolution) -> String {
    match resolution {
        Resolution::Module(module) => {
            format!("module:{}", relative_resolved_path(root, &module.0))
        }
        Resolution::External(package) => format!("external:{package}"),
        Resolution::Redirect(target) => format!("redirect:{target}"),
        Resolution::Invalid(reason) => format!("invalid:{reason}"),
        Resolution::Unresolved => "unresolved".into(),
    }
}

fn normalized_path_is_within(resolved: &str, directory: &Path) -> bool {
    let directory = normalize(directory);
    let resolved = normalize_text_path(resolved);
    resolved == directory
        || resolved
            .strip_prefix(directory.trim_end_matches('/'))
            .is_some_and(|relative| relative.starts_with('/'))
}

fn normalize(path: &Path) -> String {
    normalize_text_path(&path.to_string_lossy())
}

fn normalize_text_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized.strip_prefix("//?/").unwrap_or(&normalized).to_string()
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
    diagnostic.identity_target = Some(ModuleId(format!("unresolved:{}", import.specifier)));
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
    fn nx_and_turborepo_consumer_structures_resolve_without_false_positives() {
        for fixture_name in ["nx-workspace", "turbo-workspace"] {
            let root =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(fixture_name);
            let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
            assert_eq!(analysis.project.modules.len(), 2, "{fixture_name}");
            assert_eq!(analysis.project.packages.len(), 2, "{fixture_name}");
            assert_eq!(analysis.project.dependencies.len(), 1, "{fixture_name}");
            assert!(analysis.diagnostics.is_empty(), "{fixture_name}: {:?}", analysis.diagnostics);
        }
    }

    #[test]
    fn policy_fixture_exercises_every_architecture_and_package_rule_added_after_mvp() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/policies");
        let analysis = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        for rule in [
            "ARCH-006",
            "ARCH-007",
            "ARCH-008",
            "ARCH-009",
            "ARCH-010",
            "PACKAGE-001",
            "PACKAGE-002",
            "PACKAGE-003",
            "PACKAGE-004",
        ] {
            assert!(
                analysis.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == rule),
                "expected {rule}, got {:?}",
                analysis.diagnostics
            );
        }
    }

    #[test]
    fn next_consumer_is_classified_by_a_real_framework_adapter() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/consumer-next");
        let analysis = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        let module = |path: &str| {
            analysis.project.modules.iter().find(|module| module.id.0 == path).unwrap()
        };
        assert_eq!(module("src/app/page.tsx").runtime, Runtime::Server);
        assert_eq!(
            module("src/app/client-widget.tsx").framework_metadata.attributes["component"],
            "client"
        );
        assert_eq!(module("src/app/client-widget.tsx").runtime, Runtime::Browser);
        assert_eq!(module("src/app/api/route.ts").runtime, Runtime::Edge);
        assert_eq!(
            module("src/app/actions.ts").framework_metadata.attributes["role"],
            "server-action-module"
        );
        assert_eq!(module("src/middleware.ts").runtime, Runtime::Edge);
        assert_eq!(
            module("src/pages/api/health.ts").framework_metadata.attributes["role"],
            "api-route"
        );
        assert!(analysis.diagnostics.is_empty(), "{:?}", analysis.diagnostics);
    }

    #[test]
    fn runtime_fixture_exercises_transitive_runtime_graph_rules_end_to_end() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/runtime");
        let analysis = Engine::default().analyze(AnalyzeRequest::new(root)).unwrap();
        for rule in [
            "RUNTIME-001",
            "RUNTIME-002",
            "RUNTIME-003",
            "RUNTIME-004",
            "RUNTIME-005",
            "RUNTIME-006",
        ] {
            let diagnostic = analysis
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.rule_id.0 == rule)
                .unwrap_or_else(|| panic!("expected {rule}, got {:?}", analysis.diagnostics));
            assert!(diagnostic.dependency_path.len() >= 2, "{rule} must explain its path");
            assert!(diagnostic.metadata.contains_key("runtimePath"));
        }
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
    fn bundler_resolution_uses_dependency_syntax_without_package_type() {
        let root = std::env::temp_dir().join(format!("wae-engine-bundler-{}", std::process::id()));
        let package = root.join("packages/dual");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(package.join("src")).unwrap();
        fs::write(root.join("package.json"), r#"{"private":true,"workspaces":["packages/*"]}"#)
            .unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@fixture/dual","exports":{".":{"browser":"./src/browser.js","import":"./src/import.js","require":"./src/require.cjs","default":"./src/default.js"}}}"#,
        )
        .unwrap();
        fs::write(
            root.join("src/app.ts"),
            "import value from '@fixture/dual'; const legacy = require('@fixture/dual'); export { value, legacy };",
        )
        .unwrap();
        for file in ["browser.js", "import.ts", "require.cts", "default.ts"] {
            fs::write(package.join("src").join(file), "export default true;").unwrap();
        }
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nresolution:\n  mode: bundler\narchitecture:\n  layers: {}\n",
        )
        .unwrap();

        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        let targets = analysis
            .project
            .resolved_dependencies
            .iter()
            .filter(|dependency| dependency.from.0 == "src/app.ts")
            .map(|dependency| (&dependency.kind, &dependency.target))
            .collect::<Vec<_>>();
        assert!(targets.iter().any(|(kind, target)| {
            **kind == wae_core::domain::DependencyKind::Static
                && matches!(target, DependencyTarget::WorkspacePackage { module, .. }
                    if module.0.ends_with("packages/dual/src/import.ts"))
        }));
        assert!(targets.iter().any(|(kind, target)| {
            **kind == wae_core::domain::DependencyKind::Require
                && matches!(target, DependencyTarget::WorkspacePackage { module, .. }
                    if module.0.ends_with("packages/dual/src/require.cts"))
        }));
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
    fn clean_fixture_false_positive_budget_is_zero() {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let mut source_modules = 0;
        for name in
            ["basic", "aliases", "monorepo-12", "consumer-next", "nx-workspace", "turbo-workspace"]
        {
            let analysis =
                Engine::default().analyze(AnalyzeRequest::new(fixtures.join(name))).unwrap();
            source_modules += analysis
                .project
                .modules
                .iter()
                .filter(|module| module.kind == ModuleKind::Source)
                .count();
            assert!(
                analysis.diagnostics.is_empty(),
                "false positive in {name}: {:?}",
                analysis.diagnostics
            );
        }
        assert_eq!(source_modules, 30);
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
    fn invalid_package_manifest_outside_roots_and_excludes_is_ignored() {
        let root = std::env::temp_dir().join(format!("wae-scoped-manifest-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();
        fs::write(root.join("src/good.ts"), "export const good = true;").unwrap();
        fs::write(root.join("dist/package.json"), "not-json").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nproject:\n  roots: [src]\n  include: ['**/*.ts']\n  exclude: ['dist/**']\n",
        )
        .unwrap();
        let analysis = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(analysis.project.modules.len(), 1);
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
        let cold = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(cold.incremental.analyzed_modules, 1);
        assert_eq!(cold.incremental.restored_modules, 0);
        assert!(!cold.incremental.rule_snapshot_reused);
        let warm = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(warm.incremental.analyzed_modules, 0);
        assert_eq!(warm.incremental.restored_modules, 1);
        assert!(warm.incremental.rule_snapshot_reused);
        assert_eq!(warm.diagnostics, cold.diagnostics);
        let cache = root.join(".wae/cache/analysis-v2.json");
        assert!(cache.is_file());
        let snapshot: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache).unwrap()).unwrap();
        assert_eq!(snapshot["schema_version"], 2);
        assert!(snapshot["files"]["src/a.ts"]["resolved_dependencies"].is_array());
        assert!(snapshot["rules"]["diagnostics"].is_array());
        let lock = root.join(".wae/cache/analysis-v2.lock");
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
    fn incremental_resolution_invalidates_deleted_and_newly_resolvable_targets() {
        let root = std::env::temp_dir()
            .join(format!("wae-cache-resolution-invalidation-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "import { value } from './b'; export { value };").unwrap();
        fs::write(root.join("src/b.ts"), "export const value = 1;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nresolution:\n  mode: bundler\ncache:\n  enabled: true\n  directory: .wae/cache\n",
        )
        .unwrap();

        let cold = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(cold.incremental.analyzed_modules, 2);
        let warm = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(warm.incremental.restored_modules, 2);
        assert!(warm.incremental.rule_snapshot_reused);

        fs::remove_file(root.join("src/b.ts")).unwrap();
        let deleted = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(deleted.incremental.analyzed_modules, 1);
        assert!(deleted.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RESOLVE-001"));

        fs::write(root.join("src/b.ts"), "export const value = 2;").unwrap();
        let restored = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert_eq!(restored.incremental.analyzed_modules, 2);
        assert!(
            !restored.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RESOLVE-001")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_memory_overlay_supports_live_editor_analysis_without_writing_source() {
        let root = std::env::temp_dir().join(format!("wae-editor-overlay-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "import './missing';").unwrap();
        fs::write(root.join("wae.yaml"), "version: 1\nresolution:\n  mode: bundler\n").unwrap();
        let disk = Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        assert!(disk.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RESOLVE-001"));
        let live = Engine::default()
            .analyze(
                AnalyzeRequest::new(&root).with_overlay("src/a.ts", "export const fixed = true;"),
            )
            .unwrap();
        assert!(!live.diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "RESOLVE-001"));
        assert!(!live.incremental.cache_enabled);
        assert_eq!(fs::read_to_string(root.join("src/a.ts")).unwrap(), "import './missing';");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn pre_cancelled_analysis_stops_before_project_work() {
        let token = CancellationToken::default();
        token.cancel();
        let error = Engine::default()
            .analyze(AnalyzeRequest::new("path-that-must-not-be-opened").with_cancellation(token))
            .unwrap_err();
        assert!(matches!(error, AnalysisError::Cancelled));
    }

    #[test]
    fn cache_save_prunes_deleted_and_renamed_modules() {
        let root = std::env::temp_dir().join(format!("wae-cache-prune-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/old.ts"), "export const value = 1;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\ncache:\n  enabled: true\n  directory: .wae/cache\n",
        )
        .unwrap();
        Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        fs::rename(root.join("src/old.ts"), root.join("src/new.ts")).unwrap();
        Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        let cache: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(root.join(".wae/cache/analysis-v2.json")).unwrap(),
        )
        .unwrap();
        assert!(cache["files"]["src/old.ts"].is_null());
        assert!(cache["files"]["src/new.ts"].is_object());
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
        let cache = fs::read_to_string(root.join(".wae/cache/analysis-v2.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&cache).unwrap();
        assert_eq!(parsed["parser_version"], PARSER_CACHE_VERSION);
        fs::remove_dir_all(root).unwrap();
    }
}
