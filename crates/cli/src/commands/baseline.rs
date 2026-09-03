use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use wae_config::Config;
use wae_core::domain::{Diagnostic, Severity};
use wae_engine::{AtomicJsonRepository, FailurePolicy};

const BASELINE_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BaselineFile {
    schema_version: u32,
    created_at_unix: u64,
    entries: Vec<BaselineEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineEntry {
    pub fingerprint: String,
    #[serde(default)]
    pub rule_id: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBaseline {
    schema_version: u32,
    #[serde(default)]
    created_at_unix: u64,
    #[serde(default)]
    fingerprints: Vec<String>,
    #[serde(default)]
    entries: Vec<BaselineEntry>,
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
    expired: usize,
}

impl BaselineMatcher {
    pub fn matches(&self, diagnostic: &Diagnostic) -> bool {
        self.fingerprints.contains(&diagnostic.fingerprint)
            || diagnostic
                .legacy_fingerprint_aliases()
                .iter()
                .any(|fingerprint| self.fingerprints.contains(fingerprint))
    }

    pub fn len(&self) -> usize {
        self.fingerprints.len()
    }

    pub fn matched_count(&self, diagnostics: &[Diagnostic]) -> usize {
        let mut live = HashSet::new();
        for diagnostic in diagnostics {
            live.insert(diagnostic.fingerprint.clone());
            live.extend(diagnostic.legacy_fingerprint_aliases());
        }
        self.fingerprints.intersection(&live).count()
    }

    pub fn expired_count(&self) -> usize {
        self.expired
    }

    #[cfg(test)]
    fn contains(&self, fingerprint: &str) -> bool {
        self.fingerprints.contains(fingerprint)
    }
}

pub fn save(root: &Path, diagnostics: &[Diagnostic]) -> Result<SaveResult, String> {
    let config = Config::load(root).map_err(|error| error.message)?;
    let failure_policy = FailurePolicy::from_output(&config.output);
    let path = root.join(config.baseline.file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let suppressed = diagnostics.iter().filter(|diagnostic| diagnostic.suppressed).count();
    let informational = diagnostics
        .iter()
        .filter(|diagnostic| !diagnostic.suppressed && diagnostic.severity == Severity::Info)
        .count();
    if failure_policy.fail_on() == wae_config::FailOn::Error
        && failure_policy
            .max_warnings()
            .is_some_and(|maximum| failure_policy.warning_count(diagnostics) > maximum)
    {
        return Err(
            "the aggregate warning budget is exceeded and cannot be baselined; reduce warnings, increase output.max_warnings, or set output.fail_on: warning to baseline individual warnings"
                .into(),
        );
    }
    let mut sorted = diagnostics
        .iter()
        .filter(|diagnostic| failure_policy.is_failure(diagnostic))
        .collect::<Vec<_>>();
    sorted.sort_by(|left, right| left.fingerprint.cmp(&right.fingerprint));
    sorted.dedup_by(|left, right| left.fingerprint == right.fingerprint);
    let entries: Vec<_> = sorted
        .into_iter()
        .map(|diagnostic| BaselineEntry {
            fingerprint: diagnostic.fingerprint.clone(),
            rule_id: diagnostic.rule_id.0.clone(),
            source: diagnostic.dependency_path.first().map(|module| module.0.clone()),
            target: diagnostic.dependency_path.get(1).map(|module| module.0.clone()),
            reason: None,
            expires_at: None,
        })
        .collect();
    let recorded = entries.len();
    let created_at_unix =
        SystemTime::now().duration_since(UNIX_EPOCH).map_err(|error| error.to_string())?.as_secs();
    let baseline =
        BaselineFile { schema_version: BASELINE_SCHEMA_VERSION, created_at_unix, entries };
    AtomicJsonRepository::write(&path, &baseline)?;
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
    let stored = read_stored(&path)?;
    let now = now_unix()?;
    match stored.schema_version {
        1 => Ok(BaselineMatcher {
            fingerprints: stored.fingerprints.into_iter().collect(),
            expired: 0,
        }),
        2 | BASELINE_SCHEMA_VERSION => {
            let expired = stored
                .entries
                .iter()
                .filter(|entry| entry.expires_at.is_some_and(|expires| expires <= now))
                .count();
            Ok(BaselineMatcher {
                fingerprints: stored
                    .entries
                    .into_iter()
                    .filter(|entry| entry.expires_at.is_none_or(|expires| expires > now))
                    .map(|entry| entry.fingerprint)
                    .collect(),
                expired,
            })
        }
        version => Err(format!("unsupported baseline schema version `{version}`")),
    }
}

fn now_unix() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())
        .map(|d| d.as_secs())
}

fn read_stored(path: &Path) -> Result<StoredBaseline, String> {
    let source = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&source).map_err(|error| format!("invalid baseline: {error}"))
}

pub fn list(root: &Path, rule: Option<&str>) -> Result<Vec<BaselineEntry>, String> {
    let config = Config::load(root).map_err(|error| error.message)?;
    let path = root.join(config.baseline.file);
    let stored = read_stored(&path)?;
    if !matches!(stored.schema_version, 2 | BASELINE_SCHEMA_VERSION) {
        return Err("baseline list requires schema v2 or newer; recreate the baseline".into());
    }
    Ok(stored
        .entries
        .into_iter()
        .filter(|entry| rule.is_none_or(|rule| entry.rule_id == rule))
        .collect())
}

pub fn prune(root: &Path, diagnostics: &[Diagnostic]) -> Result<(PathBuf, usize, usize), String> {
    let config = Config::load(root).map_err(|error| error.message)?;
    let path = root.join(config.baseline.file);
    let stored = read_stored(&path)?;
    if !matches!(stored.schema_version, 2 | BASELINE_SCHEMA_VERSION) {
        return Err("baseline prune requires schema v2 or newer; recreate the baseline".into());
    }
    let now = now_unix()?;
    let mut live = HashSet::new();
    for diagnostic in diagnostics {
        live.insert(diagnostic.fingerprint.clone());
        live.extend(diagnostic.legacy_fingerprint_aliases());
    }
    let before = stored.entries.len();
    let mut entries = stored.entries;
    entries.retain(|entry| {
        entry.expires_at.is_none_or(|expires| expires > now) && live.contains(&entry.fingerprint)
    });
    let removed = before - entries.len();
    let remaining = entries.len();
    let baseline = BaselineFile {
        schema_version: BASELINE_SCHEMA_VERSION,
        created_at_unix: stored.created_at_unix,
        entries,
    };
    AtomicJsonRepository::write(&path, &baseline)?;
    Ok((path, removed, remaining))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{ModuleId, SourceLocation};

    #[test]
    fn saves_auditable_v3_entries_and_loads_them() {
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
        assert_eq!(value["schemaVersion"], 3);
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
    fn rejects_aggregate_warning_budgets_that_a_fingerprint_baseline_cannot_represent() {
        let root = std::env::temp_dir()
            .join(format!("wae-baseline-warning-budget-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\noutput:\n  fail_on: error\n  max_warnings: 0\n",
        )
        .unwrap();
        let mut warning = Diagnostic::new("ARCH-001", "warning");
        warning.severity = Severity::Warning;
        let error = save(&root, &[warning]).unwrap_err();
        assert!(error.contains("aggregate warning budget"));
        assert!(!root.join(".wae/baseline.json").exists());
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

    #[test]
    fn expired_entries_are_reported_and_pruned_with_resolved_entries() {
        let root = std::env::temp_dir().join(format!("wae-baseline-expiry-{}", std::process::id()));
        fs::create_dir_all(root.join(".wae")).unwrap();
        fs::write(
            root.join(".wae/baseline.json"),
            r#"{
              "schemaVersion": 3,
              "createdAtUnix": 1,
              "entries": [
                {"fingerprint":"expired","ruleId":"ARCH-001","expiresAt":1},
                {"fingerprint":"resolved","ruleId":"ARCH-003"}
              ]
            }"#,
        )
        .unwrap();
        let matcher = load(&root).unwrap();
        assert_eq!(matcher.expired_count(), 1);
        assert_eq!(matcher.len(), 1);
        let (_, removed, remaining) = prune(&root, &[]).unwrap();
        assert_eq!((removed, remaining), (2, 0));
        assert!(list(&root, None).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
