use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::{
    Dependency, DependencyTarget, Diagnostic, FeatureId, FrameworkMetadata, LayerId, Module,
    ModuleId, ModuleKind, ModulePath, Package, PackageName, Project, ResolvedDependency, Runtime,
    Severity, SourceLocation,
};
use wae_graph::ModuleGraph;
use wae_parser::{JsTsParser, ParserAdapter};
use wae_resolver::{
    ModuleResolver, Resolution, ResolverPipeline, TsConfigLoader, WorkspacePackage,
    WorkspaceResolver,
};
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
        let architecture = CompiledArchitectureModel::compile(&config)?;
        let files = discover_modules(&root, &config)?;
        let tsconfig = TsConfigLoader::load(&root).map_err(AnalysisError::Project)?;
        let workspace_resolver =
            WorkspaceResolver::discover(&root).map_err(AnalysisError::Project)?;
        let workspace_packages = workspace_resolver.packages().to_vec();
        let resolver = ResolverPipeline::node_with_workspaces(
            tsconfig.base_url,
            tsconfig.aliases,
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

        for path in &files {
            let relative = relative_path(&root, path);
            let id = ModuleId(relative.clone());
            let package = infer_package(&root, path, &workspace_packages, &default_package);
            discovered_packages.entry(package.name.clone()).or_insert_with(|| package.clone());
            let layer_name = architecture.layer(&relative)?;
            if let Some(value) = &layer_name {
                layers.insert(id.clone(), value.clone());
            }
            let package_root = relative_path(&root, Path::new(&package.root_path));
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
                        match resolver.resolve(&module_path, &import.specifier) {
                            Resolution::Module(target) => {
                                let candidate =
                                    wae_core::domain::DependencyCandidate::from(import.clone());
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
                                if !project.modules.iter().any(|module| module.id == target_id) {
                                    let target_path = root.join(&target_id.0);
                                    let package = infer_package(
                                        &root,
                                        &target_path,
                                        &workspace_packages,
                                        &default_package,
                                    );
                                    if !project
                                        .packages
                                        .iter()
                                        .any(|known| known.name == package.name)
                                    {
                                        project.packages.push(package.clone());
                                    }
                                    let layer = architecture.layer(&target_id.0)?;
                                    if let Some(value) = &layer {
                                        layers.insert(target_id.clone(), value.clone());
                                    }
                                    let package_root =
                                        relative_path(&root, Path::new(&package.root_path));
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
                                if !project.modules.iter().any(|module| module.id == external_id) {
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
                                let candidate =
                                    wae_core::domain::DependencyCandidate::from(import.clone());
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

        cache.save()?;

        project.packages.sort_by(|a, b| a.name.0.cmp(&b.name.0));
        project.modules.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        project.dependencies.sort_by(|a, b| (&a.from.0, &a.to.0).cmp(&(&b.from.0, &b.to.0)));
        let graph = ModuleGraph::from_project(&project);
        let context = RuleContext {
            project: &project,
            graph: &graph,
            config: &config,
            module_layers: &layers,
            module_features: &features,
            module_feature_roots: &feature_roots,
        };
        let mut diagnostics = project.diagnostics.clone();
        diagnostics.extend(self.rules.evaluate(&context).map_err(AnalysisError::Internal)?);
        diagnostics.sort_by(|a, b| diagnostic_key(a).cmp(&diagnostic_key(b)));
        Ok(Analysis { schema_version: OUTPUT_SCHEMA_VERSION, project, graph, diagnostics })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedImports {
    hash: u64,
    imports: Vec<wae_core::domain::Import>,
}

#[derive(Default, Serialize, Deserialize)]
struct CacheFile {
    schema_version: u32,
    files: std::collections::BTreeMap<String, CachedImports>,
}

struct AnalysisCache {
    enabled: bool,
    path: PathBuf,
    file: CacheFile,
}

impl AnalysisCache {
    fn load(root: &Path, config: &Config) -> Result<Self, AnalysisError> {
        let path = root.join(&config.cache.directory).join("imports-v1.json");
        let file = if config.cache.enabled && path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|source| serde_json::from_str::<CacheFile>(&source).ok())
                .filter(|cache| cache.schema_version == 1)
                .unwrap_or_else(|| CacheFile { schema_version: 1, ..CacheFile::default() })
        } else {
            CacheFile { schema_version: 1, ..CacheFile::default() }
        };
        Ok(Self { enabled: config.cache.enabled, path, file })
    }

    fn get(&self, module: &str, hash: u64) -> Option<Vec<wae_core::domain::Import>> {
        self.enabled
            .then(|| self.file.files.get(module))
            .flatten()
            .filter(|cached| cached.hash == hash)
            .map(|cached| cached.imports.clone())
    }

    fn insert(&mut self, module: String, hash: u64, imports: Vec<wae_core::domain::Import>) {
        if self.enabled {
            self.file.files.insert(module, CachedImports { hash, imports });
        }
    }

    fn save(&self) -> Result<(), AnalysisError> {
        if !self.enabled {
            return Ok(());
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AnalysisError::Project("cache path has no parent directory".into()))?;
        fs::create_dir_all(parent).map_err(|error| {
            AnalysisError::Project(format!("cannot create cache directory: {error}"))
        })?;
        let temporary = self.path.with_extension(format!("tmp-{}", std::process::id()));
        let contents = serde_json::to_vec(&self.file)
            .map_err(|error| AnalysisError::Internal(error.to_string()))?;
        fs::write(&temporary, contents)
            .map_err(|error| AnalysisError::Project(format!("cannot write cache: {error}")))?;
        if let Err(error) = fs::rename(&temporary, &self.path) {
            if self.path.exists() {
                fs::remove_file(&self.path).map_err(|remove_error| {
                    AnalysisError::Project(format!("cannot replace cache: {remove_error}"))
                })?;
                fs::rename(&temporary, &self.path).map_err(|rename_error| {
                    AnalysisError::Project(format!("cannot install cache: {rename_error}"))
                })?;
            } else {
                return Err(AnalysisError::Project(format!("cannot install cache: {error}")));
            }
        }
        Ok(())
    }
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn discover_modules(root: &Path, config: &Config) -> Result<Vec<PathBuf>, AnalysisError> {
    let include = build_globs(&config.project.include)?;
    let exclude = build_globs(&config.project.exclude)?;
    let mut files = Vec::new();
    for configured_root in &config.project.roots {
        let scan_root = root.join(configured_root).canonicalize().map_err(|error| {
            AnalysisError::Project(format!(
                "cannot open configured project root `{configured_root}`: {error}"
            ))
        })?;
        if !scan_root.starts_with(root) {
            return Err(AnalysisError::Project(format!(
                "configured project root `{configured_root}` escapes the project"
            )));
        }
        let mut builder = WalkBuilder::new(&scan_root);
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
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn build_globs(patterns: &[String]) -> Result<GlobSet, AnalysisError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(GlobBuilder::new(pattern).literal_separator(true).build().map_err(|e| {
            AnalysisError::Config(wae_core::domain::ConfigError {
                kind: wae_core::domain::ConfigErrorKind::InvalidPattern,
                message: e.to_string(),
                path: Some(pattern.clone()),
            })
        })?);
    }
    builder.build().map_err(|e| AnalysisError::Internal(e.to_string()))
}

struct CompiledArchitectureModel {
    layers: Vec<(String, GlobSet)>,
    feature_roots: Vec<String>,
}

impl CompiledArchitectureModel {
    fn compile(config: &Config) -> Result<Self, AnalysisError> {
        let layers = config
            .architecture
            .layers
            .iter()
            .map(|(name, layer)| Ok((name.clone(), build_globs(&layer.patterns)?)))
            .collect::<Result<Vec<_>, AnalysisError>>()?;
        Ok(Self {
            layers,
            feature_roots: config
                .architecture
                .features
                .effective_roots()
                .into_iter()
                .map(|root| root.replace('\\', "/").trim_matches('/').to_string())
                .collect(),
        })
    }

    fn layer(&self, path: &str) -> Result<Option<String>, AnalysisError> {
        let matches = self
            .layers
            .iter()
            .filter(|(_, matcher)| matcher.is_match(path))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            return Err(AnalysisError::Config(wae_core::domain::ConfigError {
                kind: wae_core::domain::ConfigErrorKind::ConflictingConfig,
                message: format!(
                    "module `{path}` matches multiple architecture layers: {}",
                    matches.join(", ")
                ),
                path: Some("architecture.layers".into()),
            }));
        }
        Ok(matches.into_iter().next())
    }

    fn feature(
        &self,
        path: &str,
        package: &Package,
        package_root: &str,
    ) -> Option<(FeatureId, String)> {
        let path = path.replace('\\', "/");
        self.feature_roots.iter().find_map(|configured_root| {
            let feature_root = if package_root.is_empty() {
                configured_root.clone()
            } else {
                format!("{}/{configured_root}", package_root.trim_matches('/'))
            };
            let prefix = format!("{feature_root}/");
            path.strip_prefix(&prefix)
                .and_then(|relative| relative.split('/').next())
                .filter(|feature| !feature.is_empty())
                .map(|feature| {
                    (
                        FeatureId { package: package.name.clone(), name: feature.to_owned() },
                        feature_root,
                    )
                })
        })
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

    #[test]
    fn resolved_windows_verbatim_paths_become_project_relative_ids() {
        let root = Path::new(r"\\?\D:\a\wae\wae");
        let resolved = "//?/D:/a/wae/wae/src/a.ts";
        assert_eq!(relative_resolved_path(root, resolved), "src/a.ts");
        assert!(normalized_path_is_within(resolved, Path::new(r"\\?\D:\a\wae\wae")));
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
        Engine::default().analyze(AnalyzeRequest::new(&root)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
