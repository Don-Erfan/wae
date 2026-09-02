pub const ENGINE_NAME: &str = "Web Architecture Engine";
pub const ENGINE_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

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

    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub struct ModuleId(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct FeatureId {
        pub package: PackageName,
        pub name: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ModulePath(pub String);

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct RuleId(pub String);

    impl Default for RuleId {
        fn default() -> Self {
            Self(String::from("ARCH-000"))
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
    #[serde(rename_all = "lowercase")]
    pub enum Severity {
        #[default]
        Error,
        Warning,
        Info,
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
        Io,
        InvalidYaml,
        UnsupportedVersion,
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
    pub enum Runtime {
        Browser,
        Server,
        Edge,
        Node,
        Universal,
        Unknown,
    }

    /// Framework-neutral metadata emitted by optional framework adapters. Core intentionally
    /// treats adapter identifiers and attributes as open values so adding a framework does not
    /// require changing the IR crate.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct FrameworkMetadata {
        pub adapter_id: Option<String>,
        pub attributes: BTreeMap<String, String>,
    }

    /// Framework-neutral facts extracted from syntax by a parser adapter. Framework adapters
    /// consume this IR instead of rescanning source text with regular expressions.
    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct ModuleSemantics {
        pub directives: Vec<String>,
        pub exported_runtime: Option<String>,
        /// Framework marker packages imported for their side effects, such as Next.js
        /// `server-only` and `client-only`. The parser records syntax facts; adapters own meaning.
        pub marker_imports: Vec<String>,
    }

    /// Open layer identity; configured layer names remain first-class in the IR.
    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct LayerId(pub String);

    /// The canonical ownership decision for a discovered source module. All architecture
    /// consumers use this value instead of independently re-evaluating layer and exemption globs.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "status", content = "value", rename_all = "camelCase")]
    pub enum LayerOwnership {
        Assigned(LayerId),
        Exempt(String),
        Unassigned,
        Overlap(Vec<LayerId>),
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ArchitectureOwnershipIndex {
        entries: BTreeMap<ModuleId, LayerOwnership>,
    }

    impl ArchitectureOwnershipIndex {
        pub fn insert(&mut self, module: ModuleId, ownership: LayerOwnership) {
            self.entries.insert(module, ownership);
        }

        pub fn get(&self, module: &ModuleId) -> Option<&LayerOwnership> {
            self.entries.get(module)
        }

        pub fn iter(&self) -> impl Iterator<Item = (&ModuleId, &LayerOwnership)> {
            self.entries.iter()
        }

        pub fn source_modules(&self) -> usize {
            self.entries.len()
        }

        pub fn assigned_modules(&self) -> usize {
            self.entries
                .values()
                .filter(|ownership| matches!(ownership, LayerOwnership::Assigned(_)))
                .count()
        }

        pub fn exempted_modules(&self) -> usize {
            self.entries
                .values()
                .filter(|ownership| matches!(ownership, LayerOwnership::Exempt(_)))
                .count()
        }

        pub fn unassigned_modules(&self) -> Vec<&ModuleId> {
            self.entries
                .iter()
                .filter_map(|(module, ownership)| {
                    matches!(ownership, LayerOwnership::Unassigned).then_some(module)
                })
                .collect()
        }

        pub fn coverage_percent(&self) -> u8 {
            let enforceable = self.source_modules().saturating_sub(self.exempted_modules());
            self.assigned_modules()
                .saturating_mul(100)
                .checked_div(enforceable)
                .unwrap_or(100)
                .min(100) as u8
        }
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
        Excluded,
        External,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
        TypeOnly,
        ReExport,
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
        pub layer: Option<LayerId>,
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
                ImportKind::TypeOnly => DependencyKind::TypeOnly,
                ImportKind::ReExport => DependencyKind::ReExport,
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

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyTarget {
        Internal(ModuleId),
        WorkspacePackage { package: PackageName, module: ModuleId },
        Builtin(String),
        ExternalPackage(PackageName),
        Unresolved { specifier: String, reason: String },
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ResolvedDependency {
        pub from: ModuleId,
        pub specifier: String,
        pub kind: DependencyKind,
        pub target: DependencyTarget,
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
        pub resolved_dependencies: Vec<ResolvedDependency>,
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
        resolved_dependencies: Vec<ResolvedDependency>,
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

        pub fn add_resolved_dependency(mut self, dependency: ResolvedDependency) -> Self {
            self.resolved_dependencies.push(dependency);
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
                resolved_dependencies: self.resolved_dependencies,
                diagnostics: self.diagnostics,
            }
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub struct Diagnostic {
        /// Stable semantic identity derived from the rule and involved modules.
        pub fingerprint: String,
        pub rule_id: RuleId,
        pub severity: Severity,
        pub message: String,
        pub primary_location: Option<SourceLocation>,
        pub secondary_locations: Vec<SourceLocation>,
        pub dependency_path: Vec<ModuleId>,
        pub suggestion: Option<String>,
        pub metadata: BTreeMap<String, String>,
        /// Optional semantic target used when no resolved dependency target exists (for example,
        /// the requested specifier of an unresolved import). It is intentionally not serialized:
        /// machine output exposes diagnostics, not the internal fingerprint implementation.
        #[serde(skip)]
        pub identity_target: Option<ModuleId>,
        #[serde(default)]
        pub suppressed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub suppression_reason: Option<String>,
    }

    impl Diagnostic {
        pub fn new(rule_id: impl Into<String>, message: impl Into<String>) -> Self {
            Self { rule_id: RuleId(rule_id.into()), message: message.into(), ..Self::default() }
        }

        pub fn identity(&self) -> DiagnosticIdentity {
            let source = self.dependency_path.first().cloned().or_else(|| {
                self.primary_location.as_ref().map(|location| ModuleId(location.file.clone()))
            });
            let target =
                self.identity_target.clone().or_else(|| self.dependency_path.get(1).cloned());
            let stable_location = self
                .primary_location
                .as_ref()
                .map(|location| StableLocation { file: location.file.replace('\\', "/") });
            DiagnosticIdentity { rule_id: self.rule_id.clone(), source, target, stable_location }
        }

        /// FNV-1a is intentionally used instead of `DefaultHasher`: its output is stable across
        /// processes and Rust releases. Presentation fields such as metadata, message, severity,
        /// suggestions and source line numbers never participate in identity.
        pub fn refresh_fingerprint(&mut self) {
            self.fingerprint = stable_fingerprint(&self.identity().canonical());
        }

        /// Fingerprints emitted by 0.0.10 and 0.0.11. Baseline matching uses these aliases during
        /// migration, while newly-created baselines only store the stable identity fingerprint.
        pub fn legacy_fingerprint_aliases(&self) -> Vec<String> {
            let with_all_metadata = legacy_identity(self, false);
            let without_presentation_metadata = legacy_identity(self, true);
            let mut aliases = vec![
                stable_fingerprint(&with_all_metadata),
                stable_fingerprint(&without_presentation_metadata),
            ];
            aliases.sort();
            aliases.dedup();
            aliases
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct DiagnosticIdentity {
        pub rule_id: RuleId,
        pub source: Option<ModuleId>,
        pub target: Option<ModuleId>,
        pub stable_location: Option<StableLocation>,
    }

    impl DiagnosticIdentity {
        fn canonical(&self) -> String {
            let mut value = self.rule_id.0.clone();
            if let Some(source) = &self.source {
                value.push_str("|source=");
                value.push_str(&source.0.replace('\\', "/"));
            }
            if let Some(target) = &self.target {
                value.push_str("|target=");
                value.push_str(&target.0.replace('\\', "/"));
            }
            if self.source.is_none() {
                if let Some(location) = &self.stable_location {
                    value.push_str("|file=");
                    value.push_str(&location.file);
                }
            }
            value
        }
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
    pub struct StableLocation {
        pub file: String,
    }

    fn legacy_identity(diagnostic: &Diagnostic, ignore_presentation_metadata: bool) -> String {
        let mut identity = diagnostic.rule_id.0.clone();
        if diagnostic.dependency_path.is_empty() {
            if let Some(location) = &diagnostic.primary_location {
                identity.push('|');
                identity.push_str(&location.file.replace('\\', "/"));
            }
        }
        for module in &diagnostic.dependency_path {
            identity.push('|');
            identity.push_str(&module.0.replace('\\', "/"));
        }
        for (key, value) in &diagnostic.metadata {
            if ignore_presentation_metadata && key == "related_rules" {
                continue;
            }
            identity.push('|');
            identity.push_str(key);
            identity.push('=');
            identity.push_str(value);
        }
        identity
    }

    fn stable_fingerprint(identity: &str) -> String {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in identity.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }
}

pub mod rule_registry {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RuleScope {
        Edge,
        Closure,
        Global,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RuleDescriptor {
        pub id: &'static str,
        pub title: &'static str,
        pub description: &'static str,
        pub category: &'static str,
        pub configurable: bool,
    }

    impl RuleDescriptor {
        pub fn scope(&self) -> RuleScope {
            match self.id {
                "ARCH-002" | "ARCH-003" | "ARCH-004" | "ARCH-005" | "ARCH-007" | "ARCH-008"
                | "ARCH-010" | "PACKAGE-002" | "PACKAGE-003" | "PACKAGE-004" => RuleScope::Edge,
                "RUNTIME-001" | "RUNTIME-002" | "RUNTIME-003" | "RUNTIME-004" | "RUNTIME-005" => {
                    RuleScope::Closure
                }
                _ => RuleScope::Global,
            }
        }
    }

    pub static RULES: &[RuleDescriptor] = &[
        RuleDescriptor {
            id: "ARCH-001",
            title: "Circular dependency",
            description: "Detects strongly connected module components.",
            category: "dependency-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-002",
            title: "Forbidden dependency",
            description: "Enforces configured dependency policies and architecture presets.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-003",
            title: "Layer boundary",
            description: "Enforces configured layer import direction while allowing same-layer imports.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-004",
            title: "Feature boundary",
            description: "Requires cross-feature dependencies to use public entrypoints.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-005",
            title: "Private import",
            description: "Prevents access to private modules from outside their owner.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-006",
            title: "Dependency depth",
            description: "Limits transitive dependency depth from configured architecture entrypoints.",
            category: "maintainability",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-007",
            title: "Outgoing coupling",
            description: "Limits the number of modules directly imported by one module.",
            category: "maintainability",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-008",
            title: "Incoming coupling",
            description: "Limits the number of modules directly depending on one module.",
            category: "maintainability",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-009",
            title: "Orphan module",
            description: "Finds source modules unreachable from configured architecture entrypoints.",
            category: "maintainability",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-010",
            title: "Unassigned architecture module",
            description: "Requires every source module to belong to a configured architecture layer.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "ARCH-011",
            title: "Architecture coverage threshold",
            description: "Enforces the configured aggregate minimum for architecture layer ownership.",
            category: "architecture",
            configurable: true,
        },
        RuleDescriptor {
            id: "PACKAGE-001",
            title: "Package cycle",
            description: "Detects circular dependencies between workspace packages.",
            category: "package-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "PACKAGE-002",
            title: "Forbidden package dependency",
            description: "Enforces configured package-to-package dependency policies.",
            category: "package-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "PACKAGE-003",
            title: "Undeclared workspace dependency",
            description: "Requires cross-workspace imports to be declared in the importer manifest.",
            category: "package-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "PACKAGE-004",
            title: "Cross-package relative import",
            description: "Prevents relative paths from bypassing workspace package entrypoints.",
            category: "package-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-001",
            title: "Browser to server dependency",
            description: "Prevents browser modules from reaching server-only modules transitively.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-002",
            title: "Browser to Node dependency",
            description: "Prevents browser modules from reaching Node-only modules transitively.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-003",
            title: "Browser-incompatible package",
            description: "Prevents browser modules from reaching configured incompatible packages.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-004",
            title: "Edge-incompatible dependency",
            description: "Prevents Edge modules from reaching Node-only modules or incompatible packages.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-005",
            title: "Ambiguous universal runtime",
            description: "Detects universal modules that transitively require incompatible runtime capabilities.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "RUNTIME-006",
            title: "Incompatible runtime cycle",
            description: "Detects dependency cycles that join incompatible runtime domains.",
            category: "runtime-graph",
            configurable: true,
        },
        RuleDescriptor {
            id: "PARSE-001",
            title: "Parse failure",
            description: "Reports malformed or unreadable source files.",
            category: "correctness",
            configurable: false,
        },
        RuleDescriptor {
            id: "RESOLVE-001",
            title: "Unresolved import",
            description: "Reports relative or aliased imports that cannot be resolved.",
            category: "correctness",
            configurable: false,
        },
        RuleDescriptor {
            id: "RESOLVE-002",
            title: "Invalid module specifier",
            description: "Reports module specifiers that violate the analysis boundary.",
            category: "correctness",
            configurable: false,
        },
        RuleDescriptor {
            id: "SUPPRESS-001",
            title: "Unused suppression",
            description: "Reports suppression directives that did not match a diagnostic.",
            category: "maintainability",
            configurable: false,
        },
    ];

    pub fn descriptor(id: &str) -> Option<&'static RuleDescriptor> {
        RULES.iter().find(|rule| rule.id == id)
    }

    pub fn configurable_ids() -> impl Iterator<Item = &'static str> {
        RULES.iter().filter(|rule| rule.configurable).map(|rule| rule.id)
    }
}

#[cfg(test)]
mod tests {
    use super::banner_lines;
    use crate::domain::{
        ArchitectureModel, Dependency, DependencyKind, FrameworkMetadata, LayerId, LayerPolicy,
        Module, ModuleId, ModuleKind, ModulePath, Package, PackageName, ProjectBuilder, Runtime,
    };

    #[test]
    fn banner_format_is_stable() {
        assert_eq!(banner_lines(), ["Web Architecture Engine", "v0.0.26"]);
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
            layer: Some(LayerId("features".into())),
            framework_metadata: FrameworkMetadata::default(),
        };

        let module_b = Module {
            id: ModuleId(String::from("B")),
            path: ModulePath(String::from("/app/src/b.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("features".into())),
            framework_metadata: FrameworkMetadata::default(),
        };

        let module_c = Module {
            id: ModuleId(String::from("C")),
            path: ModulePath(String::from("/app/src/c.ts")),
            package: package.name.clone(),
            kind: ModuleKind::Source,
            runtime: Runtime::Universal,
            layer: Some(LayerId("features".into())),
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

    #[test]
    fn diagnostic_fingerprint_does_not_change_when_lines_move() {
        use crate::domain::{Diagnostic, ModuleId, SourceLocation};
        let mut first = Diagnostic::new("ARCH-003", "Layer violation");
        first.primary_location =
            Some(SourceLocation { file: "src/app.ts".into(), line: 10, column: 3 });
        first.dependency_path =
            vec![ModuleId("src/app.ts".into()), ModuleId("src/shared.ts".into())];
        first.refresh_fingerprint();
        let mut moved = first.clone();
        moved.primary_location.as_mut().unwrap().line = 200;
        moved.refresh_fingerprint();
        assert_eq!(first.fingerprint, moved.fingerprint);
    }

    #[test]
    fn diagnostic_identity_ignores_presentation_fields_and_keeps_legacy_aliases() {
        use crate::domain::{Diagnostic, ModuleId, Severity, SourceLocation};
        let mut diagnostic = Diagnostic::new("ARCH-005", "Private feature import");
        diagnostic.severity = Severity::Warning;
        diagnostic.primary_location =
            Some(SourceLocation { file: "src/app.ts".into(), line: 3, column: 7 });
        diagnostic.dependency_path =
            vec![ModuleId("src/app.ts".into()), ModuleId("src/features/a/private.ts".into())];
        diagnostic.metadata.insert("owner".into(), "a".into());
        let legacy_before_arbitration = diagnostic.legacy_fingerprint_aliases();
        diagnostic.refresh_fingerprint();
        let stable = diagnostic.fingerprint.clone();

        diagnostic.message = "Reworded message".into();
        diagnostic.severity = Severity::Error;
        diagnostic.suggestion = Some("Use the public entrypoint".into());
        diagnostic.metadata.insert("related_rules".into(), "ARCH-004,ARCH-005".into());
        diagnostic.refresh_fingerprint();

        assert_eq!(diagnostic.fingerprint, stable);
        assert!(
            diagnostic
                .legacy_fingerprint_aliases()
                .iter()
                .any(|alias| legacy_before_arbitration.contains(alias))
        );
    }

    #[test]
    fn unresolved_specifiers_have_distinct_stable_identities() {
        use crate::domain::{Diagnostic, ModuleId, SourceLocation};
        let diagnostic = |specifier: &str| {
            let mut diagnostic = Diagnostic::new("RESOLVE-001", "unresolved");
            diagnostic.primary_location =
                Some(SourceLocation { file: "src/app.ts".into(), line: 1, column: 1 });
            diagnostic.dependency_path = vec![ModuleId("src/app.ts".into())];
            diagnostic.identity_target = Some(ModuleId(format!("unresolved:{specifier}")));
            diagnostic.refresh_fingerprint();
            diagnostic
        };
        assert_ne!(diagnostic("alpha").fingerprint, diagnostic("beta").fingerprint);
    }

    #[test]
    fn every_configurable_rule_declares_an_incremental_scope() {
        use crate::rule_registry::{RuleScope, configurable_ids, descriptor};
        let scopes =
            configurable_ids().map(|id| descriptor(id).unwrap().scope()).collect::<Vec<_>>();
        assert_eq!(scopes.len(), 21);
        assert!(scopes.contains(&RuleScope::Edge));
        assert!(scopes.contains(&RuleScope::Closure));
        assert!(scopes.contains(&RuleScope::Global));
    }
}
