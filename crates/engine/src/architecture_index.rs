use globset::GlobSet;
use wae_config::Config;
use wae_core::domain::{FeatureId, LayerId, LayerOwnership, Package};

use crate::AnalysisError;
use crate::discovery::build_globs;

pub(crate) struct CompiledArchitectureModel {
    layers: Vec<(String, GlobSet)>,
    feature_roots: Vec<String>,
    allow_unassigned: GlobSet,
}

impl CompiledArchitectureModel {
    pub(crate) fn compile(config: &Config) -> Result<Self, AnalysisError> {
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
            allow_unassigned: build_globs(&config.architecture.coverage.allow_unassigned)?,
        })
    }

    pub(crate) fn layer(&self, path: &str) -> Result<Option<String>, AnalysisError> {
        let matches = self.matching_layers(path);
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

    pub(crate) fn matching_layers(&self, path: &str) -> Vec<String> {
        self.layers
            .iter()
            .filter(|(_, matcher)| matcher.is_match(path))
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub(crate) fn ownership(&self, path: &str) -> LayerOwnership {
        let layers = self.matching_layers(path).into_iter().map(LayerId).collect::<Vec<_>>();
        match layers.as_slice() {
            [layer] => LayerOwnership::Assigned(layer.clone()),
            [] if self.allow_unassigned.is_match(path) => {
                LayerOwnership::Exempt("architecture.coverage.allow_unassigned".into())
            }
            [] => LayerOwnership::Unassigned,
            _ => LayerOwnership::Overlap(layers),
        }
    }

    pub(crate) fn feature(
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
