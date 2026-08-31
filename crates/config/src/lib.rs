use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use globset::GlobBuilder;
use serde::{Deserialize, Serialize};
use wae_core::{
    domain::{ConfigError, ConfigErrorKind, LayerPolicy, Severity},
    rule_registry,
};

pub const CONFIG_FILE: &str = "wae.yaml";
pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub project: ProjectConfig,
    pub architecture: ArchitectureConfig,
    pub rules: BTreeMap<String, RuleConfig>,
    pub baseline: BaselineConfig,
    pub output: OutputConfig,
    pub cache: CacheConfig,
    pub resolution: ResolutionConfig,
    pub suppressions: SuppressionConfig,
    pub framework: FrameworkConfig,
    pub runtime: RuntimeConfig,
    #[serde(skip)]
    pub configured: bool,
}

impl Default for Config {
    fn default() -> Self {
        let rules = [
            "ARCH-001",
            "ARCH-002",
            "ARCH-003",
            "ARCH-004",
            "ARCH-005",
            "ARCH-006",
            "ARCH-007",
            "ARCH-008",
            "ARCH-009",
            "ARCH-010",
            "ARCH-011",
            "PACKAGE-001",
            "PACKAGE-002",
            "PACKAGE-003",
            "PACKAGE-004",
            "RUNTIME-001",
            "RUNTIME-002",
            "RUNTIME-003",
            "RUNTIME-004",
            "RUNTIME-005",
            "RUNTIME-006",
        ]
        .into_iter()
        .map(|id| (id.into(), RuleConfig::Severity(Severity::Error)))
        .collect();
        Self {
            version: CURRENT_CONFIG_VERSION,
            project: ProjectConfig::default(),
            architecture: ArchitectureConfig::default(),
            rules,
            baseline: BaselineConfig::default(),
            output: OutputConfig::default(),
            cache: CacheConfig::default(),
            resolution: ResolutionConfig::default(),
            suppressions: SuppressionConfig::default(),
            framework: FrameworkConfig::default(),
            runtime: RuntimeConfig::default(),
            configured: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfigPreset {
    #[default]
    Blank,
    Fsd,
    Next,
    Nx,
}

impl Config {
    pub fn for_preset(preset: ConfigPreset) -> Self {
        let mut config = Self::default();
        config.architecture.layers = match preset {
            ConfigPreset::Blank => BTreeMap::new(),
            ConfigPreset::Fsd => fsd_layers("src/"),
            ConfigPreset::Next => {
                let mut layers = fsd_layers("src/");
                layers.get_mut("app").expect("FSD app layer").patterns =
                    vec!["app/**".into(), "src/app/**".into()];
                layers
            }
            ConfigPreset::Nx => BTreeMap::from([
                (
                    "apps".into(),
                    LayerConfig {
                        patterns: vec!["apps/*/src/**".into()],
                        can_import: vec!["libs".into()],
                    },
                ),
                (
                    "libs".into(),
                    LayerConfig { patterns: vec!["libs/*/src/**".into()], can_import: vec![] },
                ),
            ]),
        };
        config
    }
}

fn fsd_layers(prefix: &str) -> BTreeMap<String, LayerConfig> {
    let pattern = |segment: &str| vec![format!("{prefix}{segment}/**")];
    BTreeMap::from([
        (
            "app".into(),
            LayerConfig {
                patterns: pattern("app"),
                can_import: vec!["features".into(), "entities".into(), "shared".into()],
            },
        ),
        (
            "features".into(),
            LayerConfig {
                patterns: pattern("features"),
                can_import: vec!["entities".into(), "shared".into()],
            },
        ),
        (
            "entities".into(),
            LayerConfig { patterns: pattern("entities"), can_import: vec!["shared".into()] },
        ),
        ("shared".into(), LayerConfig { patterns: pattern("shared"), can_import: vec![] }),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub roots: Vec<String>,
    pub follow_symlinks: bool,
    pub max_file_size_kb: u64,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            include: ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"]
                .into_iter()
                .map(|extension| format!("**/*.{extension}"))
                .collect(),
            exclude: vec![
                "**/*.test.*".into(),
                "**/*.spec.*".into(),
                "**/*.d.ts".into(),
                "**/*.d.mts".into(),
                "**/*.d.cts".into(),
                "**/node_modules/**".into(),
                "**/.git/**".into(),
                "**/.idea/**".into(),
                "**/.wae/**".into(),
                "**/.next/**".into(),
                "**/dist/**".into(),
                "**/build/**".into(),
                "**/coverage/**".into(),
                "**/target/**".into(),
            ],
            roots: vec![".".into()],
            follow_symlinks: false,
            max_file_size_kb: 2_048,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureConfig {
    pub layers: BTreeMap<String, LayerConfig>,
    pub coverage: ArchitectureCoverageConfig,
    pub features: FeatureConfig,
    pub forbidden_dependencies: Vec<ForbiddenDependency>,
    pub forbidden_package_dependencies: Vec<ForbiddenDependency>,
    pub presets: ArchitecturePresets,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureCoverageConfig {
    /// Optional minimum percentage of non-exempt source modules assigned to exactly one layer.
    pub minimum: Option<u8>,
    pub allow_unassigned: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    /// Package-name globs that must never be bundled into a browser dependency closure.
    pub browser_incompatible_packages: Vec<String>,
    /// Package-name globs unavailable in an Edge isolate (for example native Node packages).
    pub edge_incompatible_packages: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitecturePresets {
    pub monorepo_boundaries: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LayerConfig {
    pub patterns: Vec<String>,
    #[serde(rename = "canImport", alias = "can_import")]
    pub can_import: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FeatureConfig {
    pub root: String,
    pub roots: Vec<String>,
    pub public_entrypoints: Vec<String>,
    pub private_segments: Vec<String>,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            root: "src/features".into(),
            roots: Vec::new(),
            public_entrypoints: [
                "index.ts",
                "index.tsx",
                "index.mts",
                "index.cts",
                "index.js",
                "index.jsx",
                "index.mjs",
                "index.cjs",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            private_segments: vec!["internal".into(), "private".into()],
        }
    }
}

impl FeatureConfig {
    pub fn effective_roots(&self) -> Vec<&str> {
        if self.roots.is_empty() {
            vec![self.root.as_str()]
        } else {
            self.roots.iter().map(String::as_str).collect()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolutionMode {
    #[serde(alias = "node")]
    Node10,
    Node16,
    #[default]
    NodeNext,
    Bundler,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResolutionConfig {
    pub mode: ResolutionMode,
    pub custom_conditions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SuppressionConfig {
    pub require_reason: bool,
    pub report_unused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FrameworkConfig {
    pub auto_detect: bool,
    pub enabled: Vec<String>,
}

impl Default for FrameworkConfig {
    fn default() -> Self {
        Self { auto_detect: true, enabled: Vec::new() }
    }
}

impl Default for SuppressionConfig {
    fn default() -> Self {
        Self { require_reason: true, report_unused: true }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ForbiddenDependency {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RuleConfig {
    Severity(Severity),
    Detailed(RuleOptions),
}

impl RuleConfig {
    pub fn severity(&self) -> Option<Severity> {
        match self {
            Self::Severity(value) => Some(value.clone()),
            Self::Detailed(options) if options.enabled => Some(options.severity.clone()),
            Self::Detailed(_) => None,
        }
    }

    pub fn options(&self) -> Option<&RuleOptions> {
        match self {
            Self::Detailed(options) => Some(options),
            Self::Severity(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuleOptions {
    pub enabled: bool,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fan_out: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fan_in: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub entrypoints: Vec<String>,
}
impl Default for RuleOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            severity: Severity::Error,
            max_depth: None,
            max_fan_out: None,
            max_fan_in: None,
            entrypoints: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BaselineConfig {
    pub file: String,
}
impl Default for BaselineConfig {
    fn default() -> Self {
        Self { file: ".wae/baseline.json".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    pub format: OutputFormat,
}
impl Default for OutputConfig {
    fn default() -> Self {
        Self { format: OutputFormat::Human }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
    Jsonl,
    Sarif,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "json" => Some(Self::Json),
            "jsonl" => Some(Self::Jsonl),
            "sarif" => Some(Self::Sarif),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Human => "human",
            Self::Json => "json",
            Self::Jsonl => "jsonl",
            Self::Sarif => "sarif",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheConfig {
    pub enabled: bool,
    pub directory: String,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self { enabled: false, directory: ".wae/cache".into() }
    }
}

impl Config {
    pub fn load(root: &Path) -> Result<Self, ConfigError> {
        Self::load_path(&root.join(CONFIG_FILE), true)
    }

    pub fn load_file(path: &Path) -> Result<Self, ConfigError> {
        Self::load_path(path, false)
    }

    fn load_path(path: &Path, missing_is_default: bool) -> Result<Self, ConfigError> {
        if !path.exists() {
            if missing_is_default {
                return Ok(Self::default());
            }
            return Err(config_error(
                ConfigErrorKind::Io,
                Some(path.display().to_string()),
                "configuration file does not exist".into(),
            ));
        }
        let source = fs::read_to_string(path).map_err(|e| {
            config_error(ConfigErrorKind::Io, Some(path.display().to_string()), e.to_string())
        })?;
        let mut config = Self::from_yaml(&source).map_err(|mut error| {
            error.path = Some(path.display().to_string());
            error
        })?;
        config.configured = true;
        Ok(config)
    }

    pub fn from_yaml(source: &str) -> Result<Self, ConfigError> {
        let mut config: Self = yaml_serde::from_str(source)
            .map_err(|e| config_error(ConfigErrorKind::InvalidYaml, None, e.to_string()))?;
        for (id, rule) in Self::default().rules {
            config.rules.entry(id).or_insert(rule);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CURRENT_CONFIG_VERSION {
            return Err(config_error(
                ConfigErrorKind::UnsupportedVersion,
                Some("version".into()),
                format!(
                    "unsupported config version {}; expected {}",
                    self.version, CURRENT_CONFIG_VERSION
                ),
            ));
        }
        let known: BTreeSet<_> = rule_registry::configurable_ids().collect();
        if let Some(rule) = self.rules.keys().find(|rule| !known.contains(rule.as_str())) {
            return Err(config_error(
                ConfigErrorKind::UnknownRule,
                Some(format!("rules.{rule}")),
                format!("unknown rule `{rule}`"),
            ));
        }
        if self.project.roots.is_empty() {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("project.roots".into()),
                "at least one project root is required".into(),
            ));
        }
        for root in &self.project.roots {
            validate_relative_path(root, "project.roots")?;
        }
        for root in self.architecture.features.effective_roots() {
            validate_relative_path(root, "architecture.features.roots")?;
        }
        validate_relative_path(&self.cache.directory, "cache.directory")?;
        for (name, layer) in &self.architecture.layers {
            validate_patterns(&layer.patterns, &format!("architecture.layers.{name}.patterns"))?;
            for target in &layer.can_import {
                if target != name && !self.architecture.layers.contains_key(target) {
                    return Err(config_error(
                        ConfigErrorKind::InvalidDependency,
                        Some(format!("architecture.layers.{name}.canImport")),
                        format!("unknown layer `{target}`"),
                    ));
                }
            }
        }
        if self.architecture.coverage.minimum.is_some_and(|minimum| minimum > 100) {
            return Err(config_error(
                ConfigErrorKind::InvalidDependency,
                Some("architecture.coverage.minimum".into()),
                "architecture coverage minimum must be between 0 and 100".into(),
            ));
        }
        validate_patterns(
            &self.architecture.coverage.allow_unassigned,
            "architecture.coverage.allow_unassigned",
        )?;
        validate_patterns(&self.project.include, "project.include")?;
        validate_patterns(&self.project.exclude, "project.exclude")?;
        for (index, condition) in self.resolution.custom_conditions.iter().enumerate() {
            if condition.trim().is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("resolution.custom_conditions.{index}")),
                    "custom resolution conditions cannot be empty".into(),
                ));
            }
        }
        let supported_frameworks = ["nextjs"];
        if let Some((index, framework)) = self
            .framework
            .enabled
            .iter()
            .enumerate()
            .find(|(_, framework)| !supported_frameworks.contains(&framework.as_str()))
        {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some(format!("framework.enabled.{index}")),
                format!("unsupported framework adapter `{framework}`"),
            ));
        }
        validate_patterns(
            &self.architecture.features.public_entrypoints,
            "architecture.features.public_entrypoints",
        )?;
        validate_patterns(
            &self.runtime.browser_incompatible_packages,
            "runtime.browser_incompatible_packages",
        )?;
        validate_patterns(
            &self.runtime.edge_incompatible_packages,
            "runtime.edge_incompatible_packages",
        )?;
        for (index, policy) in self.architecture.forbidden_dependencies.iter().enumerate() {
            validate_patterns(
                std::slice::from_ref(&policy.from),
                &format!("architecture.forbidden_dependencies.{index}.from"),
            )?;
            validate_patterns(
                std::slice::from_ref(&policy.to),
                &format!("architecture.forbidden_dependencies.{index}.to"),
            )?;
        }
        for (index, policy) in self.architecture.forbidden_package_dependencies.iter().enumerate() {
            validate_patterns(
                std::slice::from_ref(&policy.from),
                &format!("architecture.forbidden_package_dependencies.{index}.from"),
            )?;
            validate_patterns(
                std::slice::from_ref(&policy.to),
                &format!("architecture.forbidden_package_dependencies.{index}.to"),
            )?;
        }
        for (id, rule) in &self.rules {
            let Some(options) = rule.options() else { continue };
            for (name, value) in [
                ("max_depth", options.max_depth),
                ("max_fan_out", options.max_fan_out),
                ("max_fan_in", options.max_fan_in),
            ] {
                if value == Some(0) {
                    return Err(config_error(
                        ConfigErrorKind::ConflictingConfig,
                        Some(format!("rules.{id}.{name}")),
                        format!("`{name}` must be greater than zero"),
                    ));
                }
            }
            validate_patterns(&options.entrypoints, &format!("rules.{id}.entrypoints"))?;
        }
        Ok(())
    }

    pub fn layer_policies(&self) -> Vec<LayerPolicy> {
        self.architecture
            .layers
            .iter()
            .map(|(name, layer)| LayerPolicy {
                name: name.clone(),
                patterns: layer.patterns.clone(),
                can_import: layer.can_import.clone(),
            })
            .collect()
    }
    pub fn to_yaml(&self) -> Result<String, ConfigError> {
        yaml_serde::to_string(self)
            .map_err(|e| config_error(ConfigErrorKind::InvalidYaml, None, e.to_string()))
    }
}

fn validate_patterns(patterns: &[String], path: &str) -> Result<(), ConfigError> {
    for pattern in patterns {
        GlobBuilder::new(pattern).literal_separator(true).build().map_err(|error| {
            config_error(
                ConfigErrorKind::InvalidPattern,
                Some(path.into()),
                format!("invalid glob `{pattern}`: {error}"),
            )
        })?;
    }
    Ok(())
}

fn validate_relative_path(value: &str, path: &str) -> Result<(), ConfigError> {
    let candidate = Path::new(value);
    if value.trim().is_empty()
        || candidate.is_absolute()
        || candidate
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(config_error(
            ConfigErrorKind::ConflictingConfig,
            Some(path.into()),
            format!("`{value}` must be a non-empty relative path inside the project"),
        ));
    }
    Ok(())
}

fn config_error(kind: ConfigErrorKind, path: Option<String>, message: String) -> ConfigError {
    ConfigError { kind, message, path }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_json_schema_lists_every_configurable_rule() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/wae.schema.json")).unwrap();
        let properties = schema["properties"]["rules"]["properties"].as_object().unwrap();
        let schema_rules = properties.keys().map(String::as_str).collect::<BTreeSet<_>>();
        let registry_rules = rule_registry::configurable_ids().collect::<BTreeSet<_>>();
        assert_eq!(schema_rules, registry_rules);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(
            yaml_serde::from_str::<Config>("version: 1\nunknown: true\n")
                .unwrap_err()
                .to_string()
                .contains("unknown field")
        );
    }

    #[test]
    fn rejects_unknown_rules_at_their_config_path() {
        let mut config = Config::default();
        config.rules.insert("ARCH-999".into(), RuleConfig::Severity(Severity::Error));
        let error = config.validate().unwrap_err();
        assert_eq!(error.path.as_deref(), Some("rules.ARCH-999"));
    }

    #[test]
    fn partial_rule_configuration_merges_with_defaults() {
        let root = std::env::temp_dir().join(format!("wae-config-rules-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(CONFIG_FILE), "version: 1\nrules:\n  ARCH-001: warning\n").unwrap();
        let config = Config::load(&root).unwrap();
        assert_eq!(config.rules.len(), 21);
        assert_eq!(config.rules["ARCH-001"].severity(), Some(Severity::Warning));
        assert_eq!(config.rules["ARCH-005"].severity(), Some(Severity::Error));
        assert!(config.configured);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_configuration_is_marked_as_neutral_defaults() {
        let root = std::env::temp_dir().join(format!("wae-no-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let config = Config::load(&root).unwrap();
        assert!(!config.configured);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn output_format_is_a_closed_deserialized_enum() {
        let error = yaml_serde::from_str::<Config>("version: 1\noutput:\n  format: xml\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown variant") || error.contains("expected one of"));
    }

    #[test]
    fn defaults_cover_modern_node_module_extensions_and_exclude_declarations() {
        let config = Config::default();
        for extension in ["mts", "cts", "mjs", "cjs"] {
            assert!(config.project.include.contains(&format!("**/*.{extension}")));
            assert!(
                config
                    .architecture
                    .features
                    .public_entrypoints
                    .contains(&format!("index.{extension}"))
            );
        }
        for declaration in ["**/*.d.ts", "**/*.d.mts", "**/*.d.cts"] {
            assert!(config.project.exclude.iter().any(|pattern| pattern == declaration));
        }
    }

    #[test]
    fn blank_is_safe_and_fsd_patterns_are_anchored() {
        assert!(Config::for_preset(ConfigPreset::Blank).architecture.layers.is_empty());
        let fsd = Config::for_preset(ConfigPreset::Fsd);
        assert_eq!(fsd.architecture.layers["app"].patterns, ["src/app/**"]);
        assert_eq!(fsd.architecture.layers["shared"].patterns, ["src/shared/**"]);
        assert!(
            fsd.architecture
                .layers
                .values()
                .flat_map(|layer| &layer.patterns)
                .all(|pattern| !pattern.starts_with("**/"))
        );
    }

    #[test]
    fn architecture_coverage_is_strict_and_bounded() {
        let config = Config::from_yaml(
            "version: 1\narchitecture:\n  coverage:\n    minimum: 90\n    allow_unassigned: ['scripts/**']\n",
        )
        .unwrap();
        assert_eq!(config.architecture.coverage.minimum, Some(90));
        assert_eq!(config.architecture.coverage.allow_unassigned, ["scripts/**"]);
        let error = Config::from_yaml("version: 1\narchitecture:\n  coverage:\n    minimum: 101\n")
            .unwrap_err();
        assert_eq!(error.path.as_deref(), Some("architecture.coverage.minimum"));
    }

    #[test]
    fn legacy_node_mode_deserializes_as_explicit_node10() {
        let legacy: Config =
            yaml_serde::from_str("version: 1\nresolution:\n  mode: node\n").unwrap();
        assert_eq!(legacy.resolution.mode, ResolutionMode::Node10);
        assert!(
            Config::for_preset(ConfigPreset::Blank).to_yaml().unwrap().contains("mode: nodenext")
        );
    }
}
