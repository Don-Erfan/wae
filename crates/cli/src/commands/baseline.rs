use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use wae_config::Config;
use wae_core::domain::Diagnostic;

pub fn save(root: &Path, diagnostics: &[Diagnostic]) -> Result<PathBuf, String> {
    let config = Config::load(root).map_err(|e| e.message)?;
    let path = root.join(config.baseline.file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let fingerprints = diagnostics.iter().map(|d| d.fingerprint.as_str()).collect::<BTreeSet<_>>();
    let contents =
        serde_json::to_string_pretty(&json!({ "schemaVersion": 1, "fingerprints": fingerprints }))
            .map_err(|e| e.to_string())?;
    fs::write(&path, contents).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn load(root: &Path) -> Result<HashSet<String>, String> {
    let config = Config::load(root).map_err(|e| e.message)?;
    let path = root.join(config.baseline.file);
    if !path.exists() {
        return Err(format!(
            "baseline is missing at {}; run `wae baseline create` explicitly",
            path.display()
        ));
    }
    let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let value: serde_json::Value =
        serde_json::from_str(&source).map_err(|e| format!("invalid baseline: {e}"))?;
    if value["schemaVersion"].as_u64() != Some(1) {
        return Err("unsupported baseline schema version".into());
    }
    Ok(value["fingerprints"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}
