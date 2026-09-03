use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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
#[non_exhaustive]
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
    pub overrides: Vec<ConfigOverride>,
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
            overrides: Vec::new(),
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

/// A path-scoped policy overlay. Entries are applied in declaration order; the last matching
/// override wins for a rule. Detailed rule entries may change `enabled` and `severity`, while
/// structural rule options remain project-wide so graph evaluation stays deterministic.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigOverride {
    pub files: Vec<String>,
    pub excluded_files: Vec<String>,
    pub rules: BTreeMap<String, RuleConfig>,
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
    pub paths: Vec<PathSuppression>,
    pub fingerprints: Vec<FingerprintSuppression>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PathSuppression {
    pub pattern: String,
    pub rules: Vec<String>,
    pub reason: String,
    pub owner: Option<String>,
    pub ticket: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FingerprintSuppression {
    pub fingerprint: String,
    pub reason: String,
    pub owner: Option<String>,
    pub ticket: Option<String>,
    pub expires_at: Option<String>,
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
        Self {
            require_reason: true,
            report_unused: true,
            paths: Vec::new(),
            fingerprints: Vec::new(),
        }
    }
}

impl SuppressionConfig {
    pub fn prune_expired(&mut self, today: u64) -> usize {
        let before = self.paths.len() + self.fingerprints.len();
        self.paths.retain(|entry| {
            entry.expires_at.as_deref().is_none_or(|date| expiration_day(date) > today)
        });
        self.fingerprints.retain(|entry| {
            entry.expires_at.as_deref().is_none_or(|date| expiration_day(date) > today)
        });
        before - self.paths.len() - self.fingerprints.len()
    }
}

pub fn current_epoch_day() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() / 86_400)
}

pub fn expiration_day(date: &str) -> u64 {
    let mut parts = date.split('-').filter_map(|part| part.parse::<i64>().ok());
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return 0;
    };
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era - 719_468).max(0) as u64
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
    pub fail_on: FailOn,
    pub max_warnings: Option<usize>,
}
impl Default for OutputConfig {
    fn default() -> Self {
        Self { format: OutputFormat::Human, fail_on: FailOn::Error, max_warnings: None }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailOn {
    #[default]
    Error,
    Warning,
}

impl FailOn {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            _ => None,
        }
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
        let value = load_config_value(path, &mut Vec::new())?;
        let mut config = Self::from_value(value).map_err(|mut error| {
            error.path = Some(path.display().to_string());
            error
        })?;
        config.configured = true;
        Ok(config)
    }

    pub fn from_yaml(source: &str) -> Result<Self, ConfigError> {
        let value: yaml_serde::Value = yaml_serde::from_str(source)
            .map_err(|e| config_error(ConfigErrorKind::InvalidYaml, None, e.to_string()))?;
        if value.as_mapping().is_some_and(|mapping| mapping.contains_key("extends")) {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("extends".into()),
                "`extends` requires a file-backed configuration so relative paths are defined"
                    .into(),
            ));
        }
        Self::from_value(value)
    }

    fn from_value(value: yaml_serde::Value) -> Result<Self, ConfigError> {
        let mut config: Self = yaml_serde::from_value(value)
            .map_err(|e| config_error(ConfigErrorKind::InvalidYaml, None, e.to_string()))?;
        for (id, rule) in Self::default().rules {
            config.rules.entry(id).or_insert(rule);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut errors = self.validation_errors();
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.remove(0)),
            count => Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                None,
                format!(
                    "configuration has {count} errors:\n{}",
                    errors
                        .iter()
                        .map(|error| format!(
                            "- {}: {}",
                            error.path.as_deref().unwrap_or("configuration"),
                            error.message
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            )),
        }
    }

    pub fn validation_errors(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        if self.version != CURRENT_CONFIG_VERSION {
            errors.push(config_error(
                ConfigErrorKind::UnsupportedVersion,
                Some("version".into()),
                format!(
                    "unsupported config version {}; expected {}",
                    self.version, CURRENT_CONFIG_VERSION
                ),
            ));
        }
        let known = rule_registry::configurable_ids().collect::<BTreeSet<_>>();
        errors.extend(self.rules.keys().filter(|rule| !known.contains(rule.as_str())).map(
            |rule| {
                config_error(
                    ConfigErrorKind::UnknownRule,
                    Some(format!("rules.{rule}")),
                    format!("unknown rule `{rule}`"),
                )
            },
        ));
        if self.project.roots.is_empty() {
            errors.push(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("project.roots".into()),
                "at least one project root is required".into(),
            ));
        }
        if self.architecture.coverage.minimum.is_some_and(|minimum| minimum > 100) {
            errors.push(config_error(
                ConfigErrorKind::InvalidDependency,
                Some("architecture.coverage.minimum".into()),
                "architecture coverage minimum must be between 0 and 100".into(),
            ));
        }
        for (index, condition) in self.resolution.custom_conditions.iter().enumerate() {
            if condition.trim().is_empty() {
                errors.push(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("resolution.custom_conditions.{index}")),
                    "custom resolution conditions cannot be empty".into(),
                ));
            }
        }
        for (index, framework) in self.framework.enabled.iter().enumerate() {
            if framework != "nextjs" {
                errors.push(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("framework.enabled.{index}")),
                    format!("unsupported framework adapter `{framework}`"),
                ));
            }
        }
        if let Err(error) = self.validate_first() {
            if !errors
                .iter()
                .any(|candidate| candidate.path == error.path && candidate.message == error.message)
            {
                errors.push(error);
            }
        }
        errors.sort_by(|left, right| {
            left.path
                .as_deref()
                .unwrap_or("")
                .cmp(right.path.as_deref().unwrap_or(""))
                .then_with(|| left.message.cmp(&right.message))
        });
        errors
    }

    fn validate_first(&self) -> Result<(), ConfigError> {
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
        for (index, suppression) in self.suppressions.paths.iter().enumerate() {
            validate_patterns(
                std::slice::from_ref(&suppression.pattern),
                &format!("suppressions.paths.{index}.pattern"),
            )?;
            if suppression.rules.is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("suppressions.paths.{index}.rules")),
                    "a path suppression must name at least one rule".into(),
                ));
            }
            if let Some(rule) = suppression.rules.iter().find(|rule| !known.contains(rule.as_str()))
            {
                return Err(config_error(
                    ConfigErrorKind::UnknownRule,
                    Some(format!("suppressions.paths.{index}.rules")),
                    format!("suppression references unknown rule `{rule}`"),
                ));
            }
            if self.suppressions.require_reason && suppression.reason.trim().is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("suppressions.paths.{index}.reason")),
                    "path suppression requires a reason".into(),
                ));
            }
            validate_suppression_metadata(
                suppression.owner.as_deref(),
                suppression.ticket.as_deref(),
                suppression.expires_at.as_deref(),
                &format!("suppressions.paths.{index}"),
            )?;
        }
        for (index, suppression) in self.suppressions.fingerprints.iter().enumerate() {
            if suppression.fingerprint.trim().is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("suppressions.fingerprints.{index}.fingerprint")),
                    "suppression fingerprint cannot be empty".into(),
                ));
            }
            if self.suppressions.require_reason && suppression.reason.trim().is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("suppressions.fingerprints.{index}.reason")),
                    "fingerprint suppression requires a reason".into(),
                ));
            }
            validate_suppression_metadata(
                suppression.owner.as_deref(),
                suppression.ticket.as_deref(),
                suppression.expires_at.as_deref(),
                &format!("suppressions.fingerprints.{index}"),
            )?;
        }
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
        for (index, policy) in self.overrides.iter().enumerate() {
            if policy.files.is_empty() {
                return Err(config_error(
                    ConfigErrorKind::ConflictingConfig,
                    Some(format!("overrides.{index}.files")),
                    "an override must include at least one file glob".into(),
                ));
            }
            validate_patterns(&policy.files, &format!("overrides.{index}.files"))?;
            validate_patterns(
                &policy.excluded_files,
                &format!("overrides.{index}.excluded_files"),
            )?;
            for (id, rule) in &policy.rules {
                if !known.contains(id.as_str()) {
                    return Err(config_error(
                        ConfigErrorKind::UnknownRule,
                        Some(format!("overrides.{index}.rules.{id}")),
                        format!("override references unknown rule `{id}`"),
                    ));
                }
                if rule.options().is_some_and(|options| {
                    options.max_depth.is_some()
                        || options.max_fan_out.is_some()
                        || options.max_fan_in.is_some()
                        || !options.entrypoints.is_empty()
                }) {
                    return Err(config_error(
                        ConfigErrorKind::ConflictingConfig,
                        Some(format!("overrides.{index}.rules.{id}")),
                        "path overrides may change only rule enablement and severity; structural options are project-wide".into(),
                    ));
                }
            }
        }
        for (id, rule) in &self.rules {
            let Some(options) = rule.options() else { continue };
            let descriptor = rule_registry::descriptor(id).expect("known rule was validated above");
            for (name, configured, supported) in [
                ("max_depth", options.max_depth.is_some(), descriptor.supports_option("max_depth")),
                (
                    "max_fan_out",
                    options.max_fan_out.is_some(),
                    descriptor.supports_option("max_fan_out"),
                ),
                (
                    "max_fan_in",
                    options.max_fan_in.is_some(),
                    descriptor.supports_option("max_fan_in"),
                ),
                (
                    "entrypoints",
                    !options.entrypoints.is_empty(),
                    descriptor.supports_option("entrypoints"),
                ),
            ] {
                if configured && !supported {
                    return Err(config_error(
                        ConfigErrorKind::ConflictingConfig,
                        Some(format!("rules.{id}.{name}")),
                        format!("rule `{id}` does not support option `{name}`"),
                    ));
                }
            }
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

    pub fn rule_severity_for_path(&self, rule_id: &str, path: &str) -> Option<Severity> {
        let mut severity = self.rules.get(rule_id).and_then(RuleConfig::severity);
        for policy in &self.overrides {
            let included = policy.files.iter().any(|pattern| pattern_matches(pattern, path));
            let excluded =
                policy.excluded_files.iter().any(|pattern| pattern_matches(pattern, path));
            if included && !excluded {
                if let Some(rule) = policy.rules.get(rule_id) {
                    severity = rule.severity();
                }
            }
        }
        severity
    }

    pub fn rule_enabled_anywhere(&self, rule_id: &str) -> bool {
        self.rules.get(rule_id).and_then(RuleConfig::severity).is_some()
            || self
                .overrides
                .iter()
                .filter_map(|policy| policy.rules.get(rule_id))
                .any(|rule| rule.severity().is_some())
    }
    pub fn to_yaml(&self) -> Result<String, ConfigError> {
        yaml_serde::to_string(self)
            .map_err(|e| config_error(ConfigErrorKind::InvalidYaml, None, e.to_string()))
    }
}

fn load_config_value(
    path: &Path,
    loading: &mut Vec<PathBuf>,
) -> Result<yaml_serde::Value, ConfigError> {
    let canonical = path.canonicalize().map_err(|error| {
        config_error(ConfigErrorKind::Io, Some(path.display().to_string()), error.to_string())
    })?;
    if let Some(start) = loading.iter().position(|candidate| candidate == &canonical) {
        let mut cycle =
            loading[start..].iter().map(|path| path.display().to_string()).collect::<Vec<_>>();
        cycle.push(canonical.display().to_string());
        return Err(config_error(
            ConfigErrorKind::ConflictingConfig,
            Some("extends".into()),
            format!("configuration extends cycle: {}", cycle.join(" -> ")),
        ));
    }
    loading.push(canonical.clone());
    let source = fs::read_to_string(&canonical).map_err(|error| {
        config_error(ConfigErrorKind::Io, Some(canonical.display().to_string()), error.to_string())
    })?;
    let mut child: yaml_serde::Value = yaml_serde::from_str(&source).map_err(|error| {
        config_error(
            ConfigErrorKind::InvalidYaml,
            Some(canonical.display().to_string()),
            error.to_string(),
        )
    })?;
    let extends = take_extends(&mut child)?;
    let mut merged = yaml_serde::Value::Mapping(Default::default());
    let parent = canonical.parent().unwrap_or_else(|| Path::new("."));
    for extension in extends {
        if Path::new(&extension).is_absolute() {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("extends".into()),
                format!("extended config `{extension}` must be a relative path"),
            ));
        }
        let inherited = load_config_value(&parent.join(extension), loading)?;
        merge_yaml(&mut merged, inherited);
    }
    merge_yaml(&mut merged, child);
    loading.pop();
    Ok(merged)
}

fn take_extends(value: &mut yaml_serde::Value) -> Result<Vec<String>, ConfigError> {
    let Some(mapping) = value.as_mapping_mut() else { return Ok(Vec::new()) };
    let Some(value) = mapping.remove("extends") else { return Ok(Vec::new()) };
    match value {
        yaml_serde::Value::String(path) => Ok(vec![path]),
        yaml_serde::Value::Sequence(paths) => paths
            .into_iter()
            .map(|value| {
                value.as_str().map(str::to_owned).ok_or_else(|| {
                    config_error(
                        ConfigErrorKind::InvalidYaml,
                        Some("extends".into()),
                        "every `extends` entry must be a string path".into(),
                    )
                })
            })
            .collect(),
        _ => Err(config_error(
            ConfigErrorKind::InvalidYaml,
            Some("extends".into()),
            "`extends` must be a string or an array of string paths".into(),
        )),
    }
}

fn merge_yaml(base: &mut yaml_serde::Value, overlay: yaml_serde::Value) {
    match (base, overlay) {
        (yaml_serde::Value::Mapping(base), yaml_serde::Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge_yaml(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .is_ok_and(|glob| glob.compile_matcher().is_match(path))
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

fn validate_suppression_metadata(
    owner: Option<&str>,
    ticket: Option<&str>,
    expires_at: Option<&str>,
    path: &str,
) -> Result<(), ConfigError> {
    for (field, value) in [("owner", owner), ("ticket", ticket)] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some(format!("{path}.{field}")),
                format!("suppression {field} cannot be empty"),
            ));
        }
    }
    if let Some(date) = expires_at {
        let parts = date.split('-').map(str::parse::<u32>).collect::<Result<Vec<_>, _>>();
        let valid = parts.ok().filter(|parts| parts.len() == 3).is_some_and(|parts| {
            let (year, month, day) = (parts[0], parts[1], parts[2]);
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            let maximum = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if leap => 29,
                2 => 28,
                _ => 0,
            };
            year >= 1970 && day > 0 && day <= maximum
        });
        if !valid {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some(format!("{path}.expires_at")),
                "suppression expiration must be a valid YYYY-MM-DD date".into(),
            ));
        }
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
        let override_rules = schema["$defs"]["override"]["properties"]["rules"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(override_rules, registry_rules);
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
    fn reports_independent_configuration_errors_together() {
        let mut config = Config {
            version: 99,
            project: ProjectConfig { roots: Vec::new(), ..ProjectConfig::default() },
            ..Config::default()
        };
        config.rules.insert("ARCH-999".into(), RuleConfig::Severity(Severity::Error));
        config.resolution.custom_conditions = vec![String::new()];
        let errors = config.validation_errors();
        let paths =
            errors.iter().filter_map(|error| error.path.as_deref()).collect::<BTreeSet<_>>();
        assert!(paths.contains("version"));
        assert!(paths.contains("project.roots"));
        assert!(paths.contains("rules.ARCH-999"));
        assert!(paths.contains("resolution.custom_conditions.0"));
        let aggregate = config.validate().unwrap_err();
        assert!(aggregate.message.contains("configuration has 4 errors"));
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
    fn rejects_rule_options_that_the_selected_rule_cannot_consume() {
        let config: Config = yaml_serde::from_str(
            "version: 1\nrules:\n  ARCH-003:\n    severity: error\n    max_depth: 4\n",
        )
        .unwrap();
        let error = config.validate().unwrap_err();
        assert_eq!(error.path.as_deref(), Some("rules.ARCH-003.max_depth"));
        assert!(error.message.contains("does not support"));
    }

    #[test]
    fn validates_path_and_identity_suppressions_with_reasons() {
        let valid: Config = yaml_serde::from_str(
            "version: 1\nsuppressions:\n  paths:\n    - pattern: 'src/legacy/**'\n      rules: [ARCH-003]\n      reason: 'migration ARC-42'\n      owner: frontend-platform\n      ticket: ARC-42\n      expires_at: '2027-01-01'\n  fingerprints:\n    - fingerprint: abc123\n      reason: 'accepted ARC-99'\n",
        )
        .unwrap();
        valid.validate().unwrap();
        assert_eq!(valid.suppressions.paths[0].owner.as_deref(), Some("frontend-platform"));
        let invalid: Config = yaml_serde::from_str(
            "version: 1\nsuppressions:\n  paths:\n    - pattern: 'src/**'\n      rules: [ARCH-003]\n",
        )
        .unwrap();
        assert_eq!(
            invalid.validate().unwrap_err().path.as_deref(),
            Some("suppressions.paths.0.reason")
        );
        let invalid_date: Config = yaml_serde::from_str(
            "version: 1\nsuppressions:\n  fingerprints:\n    - fingerprint: abc\n      reason: migration\n      expires_at: '2026-02-30'\n",
        )
        .unwrap();
        assert_eq!(
            invalid_date.validate().unwrap_err().path.as_deref(),
            Some("suppressions.fingerprints.0.expires_at")
        );
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

    #[test]
    fn file_backed_config_extends_and_deep_merges_multiple_parents() {
        let root = std::env::temp_dir().join(format!("wae-config-extends-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/base.yaml"),
            "version: 1\nproject:\n  roots: [src]\nrules:\n  ARCH-001: warning\n",
        )
        .unwrap();
        fs::write(root.join("config/team.yaml"), "version: 1\noutput:\n  max_warnings: 4\n")
            .unwrap();
        fs::write(
            root.join(CONFIG_FILE),
            "extends: [config/base.yaml, config/team.yaml]\nversion: 1\nrules:\n  ARCH-001: error\n",
        )
        .unwrap();
        let config = Config::load(&root).unwrap();
        assert_eq!(config.project.roots, ["src"]);
        assert_eq!(config.output.max_warnings, Some(4));
        assert_eq!(config.rules["ARCH-001"].severity(), Some(Severity::Error));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn extends_cycles_are_reported_and_path_overrides_use_last_match() {
        let root = std::env::temp_dir().join(format!("wae-config-cycle-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.yaml"), "extends: b.yaml\nversion: 1\n").unwrap();
        fs::write(root.join("b.yaml"), "extends: a.yaml\nversion: 1\n").unwrap();
        let error = Config::load_file(&root.join("a.yaml")).unwrap_err();
        assert!(error.message.contains("extends cycle"));

        let config = Config::from_yaml(
            "version: 1\noverrides:\n  - files: ['src/legacy/**']\n    rules:\n      ARCH-003: warning\n  - files: ['src/legacy/generated/**']\n    rules:\n      ARCH-003:\n        enabled: false\n",
        )
        .unwrap();
        assert_eq!(
            config.rule_severity_for_path("ARCH-003", "src/legacy/a.ts"),
            Some(Severity::Warning)
        );
        assert_eq!(config.rule_severity_for_path("ARCH-003", "src/legacy/generated/a.ts"), None);
        fs::remove_dir_all(root).unwrap();
    }
}
