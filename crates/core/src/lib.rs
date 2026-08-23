pub const ENGINE_NAME: &str = "Web Architecture Engine";
pub const ENGINE_VERSION: &str = "v0.0.1";

pub fn banner_lines() -> [&'static str; 2] {
    [ENGINE_NAME, ENGINE_VERSION]
}

pub mod domain {
    use std::collections::{BTreeMap, HashSet};

    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct WorkspaceId(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct PackageName(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ModuleId(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ModulePath(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RuleId(pub String);

    impl Default for RuleId {
        fn default() -> Self {
            Self(String::from("ARCH-000"))
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Severity {
        Error,
        Warning,
        Info,
    }

    impl Default for Severity {
        fn default() -> Self {
            Self::Error
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ParseErrorKind {
        UnsupportedSyntax,
        MalformedSource,
        ProviderFailure,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ParseError {
        pub kind: ParseErrorKind,
        pub message: String,
        pub location: Option<SourceLocation>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ResolutionErrorKind {
        UnresolvedImport,
        InvalidSpecifier,
        AliasConflict,
        PackageNotFound,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ResolutionError {
        pub kind: ResolutionErrorKind,
        pub importer: Option<ModuleId>,
        pub specifier: Option<String>,
        pub message: String,
        pub location: Option<SourceLocation>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ConfigErrorKind {
        UnknownRule,
        DuplicateLayer,
        InvalidPattern,
        ConflictingConfig,
        InvalidDependency,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ConfigError {
        pub kind: ConfigErrorKind,
        pub message: String,
        pub path: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum InternalErrorKind {
        InvariantViolation,
        UnexpectedState,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct InternalError {
        pub kind: InternalErrorKind,
        pub message: String,
        pub component: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum EngineError {
        Parse(ParseError),
        Resolution(ResolutionError),
        Config(ConfigError),
        Internal(InternalError),
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct SourceLocation {
        pub file: String,
        pub line: usize,
        pub column: usize,
    }

    impl SourceLocation {
        pub fn unknown() -> Self {
            Self::default()
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum Runtime {
        Browser,
        Server,
        Edge,
        Node,
        Universal,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum Framework {
        NextJs,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum NextRouterKind {
        App,
        Pages,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub enum NextModuleKind {
        ServerComponent,
        ClientComponent,
        ServerAction,
        RouteHandler,
        Middleware,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct NextMetadata {
        pub router: Option<NextRouterKind>,
        pub kind: Option<NextModuleKind>,
        pub edge_runtime: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FrameworkMetadata {
        pub framework: Option<Framework>,
        pub next: Option<NextMetadata>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum Layer {
        App,
        Features,
        Entities,
        Shared,
        Infrastructure,
        Unknown,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct LayerPolicy {
        pub name: String,
        pub patterns: Vec<String>,
        pub can_import: Vec<String>,
    }

    impl LayerPolicy {
        pub fn new(name: impl Into<String>) -> Self {
            Self { name: name.into(), ..Self::default() }
        }

        pub fn with_patterns(mut self, patterns: Vec<String>) -> Self {
            self.patterns = patterns;
            self
        }

        pub fn with_can_import(mut self, can_import: Vec<String>) -> Self {
            self.can_import = can_import;
            self
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ArchitectureModel {
        pub layers: Vec<LayerPolicy>,
    }

    impl ArchitectureModel {
        pub fn new(layers: Vec<LayerPolicy>) -> Self {
            Self { layers }
        }

        pub fn layer(&self, name: &str) -> Option<&LayerPolicy> {
            self.layers.iter().find(|layer| layer.name == name)
        }

        pub fn can_import(&self, from_layer: &str, to_layer: &str) -> bool {
            if from_layer == to_layer {
                return true;
            }

            self.layer(from_layer)
                .map(|layer| layer.can_import.iter().any(|candidate| candidate == to_layer))
                .unwrap_or(false)
        }

        pub fn invalid_references(&self) -> Vec<(String, String)> {
            let known: HashSet<&str> =
                self.layers.iter().map(|layer| layer.name.as_str()).collect();

            self.layers
                .iter()
                .flat_map(|layer| {
                    layer
                        .can_import
                        .iter()
                        .filter(|target| !known.contains(target.as_str()))
                        .map(|target| (layer.name.clone(), target.clone()))
                })
                .collect()
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ModuleKind {
        Source,
        External,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyKind {
        Static,
        Dynamic,
        TypeOnly,
        ReExport,
        Require,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ImportKind {
        Static,
        Dynamic,
        Require,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ExportKind {
        Named,
        ReExport,
        Default,
        All,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Workspace {
        pub id: WorkspaceId,
        pub root_path: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Package {
        pub name: PackageName,
        pub root_path: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Module {
        pub id: ModuleId,
        pub path: ModulePath,
        pub package: PackageName,
        pub kind: ModuleKind,
        pub runtime: Runtime,
        pub layer: Layer,
        pub framework_metadata: FrameworkMetadata,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Import {
        pub module_id: ModuleId,
        pub specifier: String,
        pub kind: ImportKind,
        pub location: SourceLocation,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Export {
        pub module_id: ModuleId,
        pub specifier: Option<String>,
        pub kind: ExportKind,
        pub location: SourceLocation,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct DependencyCandidate {
        pub module_id: ModuleId,
        pub specifier: String,
        pub kind: DependencyKind,
        pub location: SourceLocation,
    }

    impl From<Import> for DependencyCandidate {
        fn from(value: Import) -> Self {
            let kind = match value.kind {
                ImportKind::Static => DependencyKind::Static,
                ImportKind::Dynamic => DependencyKind::Dynamic,
                ImportKind::Require => DependencyKind::Require,
            };

            Self {
                module_id: value.module_id,
                specifier: value.specifier,
                kind,
                location: value.location,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Dependency {
        pub from: ModuleId,
        pub to: ModuleId,
        pub kind: DependencyKind,
        pub location: SourceLocation,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Project {
        pub workspace: Option<Workspace>,
        pub packages: Vec<Package>,
        pub modules: Vec<Module>,
        pub imports: Vec<Import>,
        pub exports: Vec<Export>,
        pub dependency_candidates: Vec<DependencyCandidate>,
        pub dependencies: Vec<Dependency>,
        pub diagnostics: Vec<Diagnostic>,
    }

    #[derive(Clone, Debug, Default)]
    pub struct ProjectBuilder {
        workspace: Option<Workspace>,
        packages: Vec<Package>,
        modules: Vec<Module>,
        imports: Vec<Import>,
        exports: Vec<Export>,
        dependency_candidates: Vec<DependencyCandidate>,
        dependencies: Vec<Dependency>,
        diagnostics: Vec<Diagnostic>,
    }

    impl ProjectBuilder {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn workspace(mut self, workspace: Workspace) -> Self {
            self.workspace = Some(workspace);
            self
        }

        pub fn add_package(mut self, package: Package) -> Self {
            self.packages.push(package);
            self
        }

        pub fn add_module(mut self, module: Module) -> Self {
            self.modules.push(module);
            self
        }

        pub fn add_import(mut self, import: Import) -> Self {
            self.dependency_candidates.push(import.clone().into());
            self.imports.push(import);
            self
        }

        pub fn add_export(mut self, export: Export) -> Self {
            self.exports.push(export);
            self
        }

        pub fn add_dependency_candidate(
            mut self,
            dependency_candidate: DependencyCandidate,
        ) -> Self {
            self.dependency_candidates.push(dependency_candidate);
            self
        }

        pub fn add_dependency(mut self, dependency: Dependency) -> Self {
            self.dependencies.push(dependency);
            self
        }

        pub fn add_diagnostic(mut self, diagnostic: Diagnostic) -> Self {
            self.diagnostics.push(diagnostic);
            self
        }

        pub fn build(self) -> Project {
            Project {
                workspace: self.workspace,
                packages: self.packages,
                modules: self.modules,
                imports: self.imports,
                exports: self.exports,
                dependency_candidates: self.dependency_candidates,
                dependencies: self.dependencies,
                diagnostics: self.diagnostics,
            }
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Diagnostic {
        pub rule_id: RuleId,
        pub severity: Severity,
        pub message: String,
        pub primary_location: Option<SourceLocation>,
        pub secondary_locations: Vec<SourceLocation>,
        pub dependency_path: Vec<ModuleId>,
        pub suggestion: Option<String>,
        pub metadata: BTreeMap<String, String>,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnalyzeRequest {
    pub project_root: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AnalyzeOutput {
    pub module_count: usize,
    pub diagnostics: Vec<domain::Diagnostic>,
}

pub trait Analyzer {
    fn analyze(&self, request: &AnalyzeRequest) -> AnalyzeOutput;
}

#[cfg(test)]
mod tests {
    use super::banner_lines;
    use crate::domain::{
        ArchitectureModel, Dependency, DependencyKind, FrameworkMetadata, Layer, LayerPolicy,
        Module, ModuleId, ModuleKind, ModulePath, Package, PackageName, ProjectBuilder, Runtime,
    };

    #[test]
    fn banner_format_is_stable() {
        assert_eq!(banner_lines(), ["Web Architecture Engine", "v0.0.1"]);
    }

    #[test]
    fn can_build_project_graph_without_parser() {
        let package =
            Package { name: PackageName(String::from("web")), root_path: String::from("/app") };

        let module_a = Module {
            id: ModuleId(String::from("A")),
            path: ModulePath(String::from("/app/src/a.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Layer::Features,
            framework_metadata: FrameworkMetadata::default(),
        };

        let module_b = Module {
            id: ModuleId(String::from("B")),
            path: ModulePath(String::from("/app/src/b.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Layer::Features,
            framework_metadata: FrameworkMetadata::default(),
        };

        let module_c = Module {
            id: ModuleId(String::from("C")),
            path: ModulePath(String::from("/app/src/c.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Layer::Features,
            framework_metadata: FrameworkMetadata::default(),
        };

        let dependency_ab = Dependency {
            from: module_a.id.clone(),
            to: module_b.id.clone(),
            kind: DependencyKind::Static,
            location: crate::domain::SourceLocation::unknown(),
        };

        let dependency_bc = Dependency {
            from: module_b.id.clone(),
            to: module_c.id.clone(),
            kind: DependencyKind::Static,
            location: crate::domain::SourceLocation::unknown(),
        };

        let project = ProjectBuilder::new()
            .add_package(package)
            .add_module(module_a)
            .add_module(module_b)
            .add_module(module_c)
            .add_dependency(dependency_ab)
            .add_dependency(dependency_bc)
            .build();

        assert_eq!(project.modules.len(), 3);
        assert_eq!(project.dependencies.len(), 2);
        assert_eq!(project.dependencies[0].from.0, "A");
        assert_eq!(project.dependencies[0].to.0, "B");
        assert_eq!(project.dependencies[1].from.0, "B");
        assert_eq!(project.dependencies[1].to.0, "C");
    }

    #[test]
    fn architecture_model_tracks_import_permissions() {
        let model = ArchitectureModel::new(vec![
            LayerPolicy::new("app")
                .with_patterns(vec![String::from("src/app/**")])
                .with_can_import(vec![
                    String::from("features"),
                    String::from("entities"),
                    String::from("shared"),
                ]),
            LayerPolicy::new("features")
                .with_patterns(vec![String::from("src/features/**")])
                .with_can_import(vec![String::from("entities"), String::from("shared")]),
            LayerPolicy::new("entities")
                .with_patterns(vec![String::from("src/entities/**")])
                .with_can_import(vec![String::from("shared")]),
            LayerPolicy::new("shared")
                .with_patterns(vec![String::from("src/shared/**")])
                .with_can_import(vec![]),
        ]);

        assert!(model.can_import("app", "features"));
        assert!(model.can_import("entities", "shared"));
        assert!(model.can_import("shared", "shared"));
        assert!(!model.can_import("shared", "entities"));
    }
}
