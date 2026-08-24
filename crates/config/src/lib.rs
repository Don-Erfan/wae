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
    #[serde(skip)]
    pub configured: bool,
}

impl Default for Config {
    fn default() -> Self {
        let rules = ["ARCH-001", "ARCH-002", "ARCH-003", "ARCH-004", "ARCH-005"]
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
            configured: false,
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureConfig {
    pub layers: BTreeMap<String, LayerConfig>,
    pub features: FeatureConfig,
    pub forbidden_dependencies: Vec<ForbiddenDependency>,
    pub presets: ArchitecturePresets,
}

impl Default for ArchitectureConfig {
    fn default() -> Self {
        let mut layers = BTreeMap::new();
        layers.insert(
            "app".into(),
            LayerConfig {
                patterns: vec!["**/app/**".into()],
                can_import: vec!["features".into(), "entities".into(), "shared".into()],
            },
        );
        layers.insert(
            "features".into(),
            LayerConfig {
                patterns: vec!["**/features/**".into()],
                can_import: vec!["entities".into(), "shared".into()],
            },
        );
        layers.insert(
            "entities".into(),
            LayerConfig {
                patterns: vec!["**/entities/**".into()],
                can_import: vec!["shared".into()],
            },
        );
        layers.insert(
            "shared".into(),
            LayerConfig { patterns: vec!["**/shared/**".into()], can_import: vec![] },
        );
        Self {
            layers,
            features: FeatureConfig::default(),
            forbidden_dependencies: Vec::new(),
            presets: ArchitecturePresets::default(),
        }
    }
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
    Node,
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuleOptions {
    pub enabled: bool,
    pub severity: Severity,
}
impl Default for RuleOptions {
    fn default() -> Self {
        Self { enabled: true, severity: Severity::Error }
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
        let path = root.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let source = fs::read_to_string(&path).map_err(|e| {
            config_error(ConfigErrorKind::Io, Some(path.display().to_string()), e.to_string())
        })?;
        let mut config: Self = yaml_serde::from_str(&source).map_err(|e| {
            config_error(
                ConfigErrorKind::InvalidYaml,
                Some(path.display().to_string()),
                e.to_string(),
            )
        })?;
        for (id, rule) in Self::default().rules {
            config.rules.entry(id).or_insert(rule);
        }
        config.configured = true;
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
        validate_patterns(
            &self.architecture.features.public_entrypoints,
            "architecture.features.public_entrypoints",
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
        assert_eq!(config.rules.len(), 5);
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
}
