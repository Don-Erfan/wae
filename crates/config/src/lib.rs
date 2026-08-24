use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use globset::GlobBuilder;
use serde::{Deserialize, Serialize};
use wae_core::domain::{ConfigError, ConfigErrorKind, LayerPolicy, Severity};

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
            include: vec!["**/*.ts".into(), "**/*.tsx".into(), "**/*.js".into(), "**/*.jsx".into()],
            exclude: vec![
                "**/*.test.*".into(),
                "**/*.spec.*".into(),
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
        Self { layers, features: FeatureConfig::default(), forbidden_dependencies: Vec::new() }
    }
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
    pub public_entrypoints: Vec<String>,
    pub private_segments: Vec<String>,
}

impl Default for FeatureConfig {
    fn default() -> Self {
        Self {
            root: "src/features".into(),
            public_entrypoints: vec!["index.ts".into(), "index.tsx".into(), "index.js".into()],
            private_segments: vec!["internal".into(), "private".into()],
        }
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
    pub format: String,
}
impl Default for OutputConfig {
    fn default() -> Self {
        Self { format: "human".into() }
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
        let config: Self = yaml_serde::from_str(&source).map_err(|e| {
            config_error(
                ConfigErrorKind::InvalidYaml,
                Some(path.display().to_string()),
                e.to_string(),
            )
        })?;
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
        let known: BTreeSet<_> =
            ["ARCH-001", "ARCH-002", "ARCH-003", "ARCH-004", "ARCH-005"].into_iter().collect();
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
        validate_relative_path(&self.architecture.features.root, "architecture.features.root")?;
        validate_relative_path(&self.cache.directory, "cache.directory")?;
        if !matches!(self.output.format.as_str(), "human" | "json" | "jsonl" | "sarif") {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("output.format".into()),
                format!("unsupported output format `{}`", self.output.format),
            ));
        }
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
}
