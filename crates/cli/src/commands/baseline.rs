use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::{Diagnostic, Severity};
use wae_engine::FailurePolicy;

const BASELINE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BaselineFile<'a> {
    schema_version: u32,
    created_at_unix: u64,
    entries: Vec<BaselineEntry<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BaselineEntry<'a> {
    fingerprint: &'a str,
    rule_id: &'a str,
    source: Option<&'a str>,
    target: Option<&'a str>,
    reason: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBaseline {
    schema_version: u32,
    #[serde(default)]
    fingerprints: Vec<String>,
    #[serde(default)]
    entries: Vec<StoredEntry>,
}

#[derive(Debug, Deserialize)]
struct StoredEntry {
    fingerprint: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct SaveResult {
    pub path: PathBuf,
    pub recorded: usize,
    pub suppressed: usize,
    pub informational: usize,
}

#[derive(Debug, Default)]
pub struct BaselineMatcher {
    fingerprints: HashSet<String>,
}

impl BaselineMatcher {
    pub fn matches(&self, diagnostic: &Diagnostic) -> bool {
        self.fingerprints.contains(&diagnostic.fingerprint)
            || diagnostic
                .legacy_fingerprint_aliases()
                .iter()
                .any(|fingerprint| self.fingerprints.contains(fingerprint))
    }

    #[cfg(test)]
    fn contains(&self, fingerprint: &str) -> bool {
        self.fingerprints.contains(fingerprint)
    }
}

pub fn save(root: &Path, diagnostics: &[Diagnostic]) -> Result<SaveResult, String> {
    let config = Config::load(root).map_err(|error| error.message)?;
    let path = root.join(config.baseline.file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let suppressed = diagnostics.iter().filter(|diagnostic| diagnostic.suppressed).count();
    let informational = diagnostics
        .iter()
        .filter(|diagnostic| !diagnostic.suppressed && diagnostic.severity == Severity::Info)
        .count();
    let mut sorted = diagnostics
        .iter()
        .filter(|diagnostic| FailurePolicy::is_failure(diagnostic))
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
    sorted.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    let entries: Vec<_> = sorted
        .into_iter()
        .map(|diagnostic| BaselineEntry {
            fingerprint: &diagnostic.fingerprint,
            rule_id: &diagnostic.rule_id.0,
            source: diagnostic.dependency_path.first().map(|module| module.0.as_str()),
            target: diagnostic.dependency_path.get(1).map(|module| module.0.as_str()),
            reason: None,
        })
        .collect();
    let recorded = entries.len();
    let created_at_unix =
        SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_secs();
    let contents = serde_json::to_string_pretty(&BaselineFile {
        schema_version: BASELINE_SCHEMA_VERSION,
        created_at_unix,
        entries,
    })
    .map_err(|error| error.to_string())?;
    fs::write(&path, format!("{contents}\n")).map_err(|error| error.to_string())?;
    Ok(SaveResult { path, recorded, suppressed, informational })
}

pub fn load(root: &Path) -> Result<BaselineMatcher, String> {
    let config = Config::load(root).map_err(|error| error.message)?;
    let path = root.join(config.baseline.file);
    if !path.exists() {
        return Err(format!(
            "baseline is missing at {}; run `wae baseline create` explicitly",
            path.display()
        ));
    }
    let source = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let stored: StoredBaseline =
        serde_json::from_str(&source).map_err(|error| format!("invalid baseline: {error}"))?;
    match stored.schema_version {
        1 => Ok(BaselineMatcher { fingerprints: stored.fingerprints.into_iter().collect() }),
        BASELINE_SCHEMA_VERSION => Ok(BaselineMatcher {
            fingerprints: stored.entries.into_iter().map(|entry| entry.fingerprint).collect(),
        }),
        version => Err(format!("unsupported baseline schema version `{version}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{ModuleId, SourceLocation};

    #[test]
    fn saves_auditable_v2_entries_and_loads_them() {
        let root = std::env::temp_dir().join(format!("wae-baseline-v2-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut diagnostic = Diagnostic::new("ARCH-003", "layer");
        diagnostic.primary_location =
            Some(SourceLocation { file: "src/app.ts".into(), line: 1, column: 1 });
        diagnostic.dependency_path =
            vec![ModuleId("src/app.ts".into()), ModuleId("src/shared.ts".into())];
        diagnostic.refresh_fingerprint();
        let saved = save(&root, &[diagnostic.clone()]).unwrap();
        assert_eq!(saved.recorded, 1);
        let source = fs::read_to_string(root.join(".wae/baseline.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&source).unwrap();
        assert_eq!(value["schemaVersion"], 2);
        assert_eq!(value["entries"][0]["ruleId"], "ARCH-003");
        assert!(load(&root).unwrap().contains(&diagnostic.fingerprint));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn excludes_suppressed_and_informational_diagnostics() {
        let root = std::env::temp_dir().join(format!("wae-baseline-filter-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let kept = Diagnostic::new("ARCH-001", "kept");
        let mut suppressed = Diagnostic::new("ARCH-002", "suppressed");
        suppressed.suppressed = true;
        let mut info = Diagnostic::new("ARCH-003", "info");
        info.severity = Severity::Info;
        let saved = save(&root, &[kept.clone(), suppressed, info]).unwrap();
        assert_eq!(saved.recorded, 1);
        assert_eq!(saved.suppressed, 1);
        assert_eq!(saved.informational, 1);
        let baseline = load(&root).unwrap();
        assert!(baseline.contains(&kept.fingerprint));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migrates_v1_fingerprint_arrays_on_read() {
        let root = std::env::temp_dir().join(format!("wae-baseline-v1-{}", std::process::id()));
        fs::create_dir_all(root.join(".wae")).unwrap();
        fs::write(
            root.join(".wae/baseline.json"),
            r#"{"schemaVersion":1,"fingerprints":["legacy"]}"#,
        )
        .unwrap();
        assert!(load(&root).unwrap().contains("legacy"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn matches_pre_arbitration_0_0_10_fingerprints() {
        let root = std::env::temp_dir().join(format!("wae-baseline-legacy-{}", std::process::id()));
        fs::create_dir_all(root.join(".wae")).unwrap();
        let mut diagnostic = Diagnostic::new("ARCH-005", "private import");
        diagnostic.dependency_path =
            vec![ModuleId("src/app.ts".into()), ModuleId("src/features/a/private.ts".into())];
        diagnostic.metadata.insert("owner".into(), "a".into());
        let legacy = diagnostic.legacy_fingerprint_aliases().into_iter().next().unwrap();
        diagnostic.metadata.insert("related_rules".into(), "ARCH-004,ARCH-005".into());
        diagnostic.refresh_fingerprint();
        fs::write(
            root.join(".wae/baseline.json"),
            format!(r#"{{"schemaVersion":1,"fingerprints":["{legacy}"]}}"#),
        )
        .unwrap();
        assert!(load(&root).unwrap().matches(&diagnostic));
        fs::remove_dir_all(root).unwrap();
    }
}
