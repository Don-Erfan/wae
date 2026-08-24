use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use wae_config::Config;

use crate::AnalysisError;

pub(crate) fn discover_modules(
    root: &Path,
    config: &Config,
) -> Result<Vec<PathBuf>, AnalysisError> {
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
            let entry = entry.map_err(|error| AnalysisError::Project(error.to_string()))?;
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            if !include.is_match(&relative) || exclude.is_match(&relative) {
                continue;
            }
            let length =
                entry.metadata().map_err(|error| AnalysisError::Project(error.to_string()))?.len();
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

pub(crate) fn build_globs(patterns: &[String]) -> Result<GlobSet, AnalysisError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(GlobBuilder::new(pattern).literal_separator(true).build().map_err(
            |error| {
                AnalysisError::Config(wae_core::domain::ConfigError {
                    kind: wae_core::domain::ConfigErrorKind::InvalidPattern,
                    message: error.to_string(),
                    path: Some(pattern.clone()),
                })
            },
        )?);
    }
    builder.build().map_err(|error| AnalysisError::Internal(error.to_string()))
}
