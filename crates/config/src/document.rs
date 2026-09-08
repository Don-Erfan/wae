use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    ConfigError, ConfigErrorKind, FingerprintSuppression, PathSuppression, config_error,
    expiration_day,
};

/// A source-preserving editor for governance entries owned by one configuration file.
///
/// This deliberately does not deserialize and serialize the complete [`crate::Config`]: doing
/// so would flatten `extends`, materialize defaults, and discard comments. Only block-style
/// suppression sequences are edited, and unsupported layouts fail without touching the file.
#[derive(Debug)]
pub struct EditableConfigDocument {
    path: PathBuf,
    source: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SuppressionPruneResult {
    pub removed_paths: usize,
    pub removed_fingerprints: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuppressionProvenance {
    pub kind: String,
    pub identity: String,
    pub reason: String,
    pub owner: Option<String>,
    pub ticket: Option<String>,
    pub expires_at: Option<String>,
    pub defined_in: String,
    pub yaml_path: String,
    pub inherited: bool,
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Default)]
struct ProvenanceSet {
    paths: Option<Vec<SuppressionProvenance>>,
    fingerprints: Option<Vec<SuppressionProvenance>>,
}

pub fn suppression_provenance(path: &Path) -> Result<Vec<SuppressionProvenance>, ConfigError> {
    let leaf = path.canonicalize().map_err(|error| {
        config_error(
            ConfigErrorKind::Io,
            Some(path.display().to_string()),
            format!("cannot resolve configuration: {error}"),
        )
    })?;
    let entries = provenance_set(&leaf, &leaf, &mut Vec::new())?;
    let mut output = entries.paths.unwrap_or_default();
    output.extend(entries.fingerprints.unwrap_or_default());
    Ok(output)
}

fn provenance_set(
    path: &Path,
    leaf: &Path,
    loading: &mut Vec<PathBuf>,
) -> Result<ProvenanceSet, ConfigError> {
    let path = path.canonicalize().map_err(|error| {
        config_error(
            ConfigErrorKind::Io,
            Some(path.display().to_string()),
            format!("cannot resolve inherited configuration: {error}"),
        )
    })?;
    if let Some(cycle_start) = loading.iter().position(|candidate| candidate == &path) {
        let mut cycle =
            loading[cycle_start..].iter().map(|path| portable_path(path)).collect::<Vec<_>>();
        cycle.push(portable_path(&path));
        return Err(config_error(
            ConfigErrorKind::ConflictingConfig,
            Some("extends".into()),
            format!("configuration extends cycle: {}", cycle.join(" -> ")),
        ));
    }
    loading.push(path.clone());
    let source = fs::read_to_string(&path).map_err(|error| {
        config_error(
            ConfigErrorKind::Io,
            Some(path.display().to_string()),
            format!("cannot read configuration: {error}"),
        )
    })?;
    let mut value: yaml_serde::Value = yaml_serde::from_str(&source).map_err(|error| {
        config_error(
            ConfigErrorKind::InvalidYaml,
            Some(path.display().to_string()),
            error.to_string(),
        )
    })?;
    let local: LeafSuppressions = yaml_serde::from_value(value.clone()).map_err(|error| {
        config_error(
            ConfigErrorKind::InvalidYaml,
            Some(path.display().to_string()),
            error.to_string(),
        )
    })?;
    let local_keys = value
        .as_mapping()
        .and_then(|mapping| mapping.get("suppressions"))
        .and_then(yaml_serde::Value::as_mapping)
        .map(|mapping| (mapping.contains_key("paths"), mapping.contains_key("fingerprints")))
        .unwrap_or_default();
    let extends = crate::take_extends(&mut value)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut resolved = ProvenanceSet::default();
    for extension in extends {
        let extension = Path::new(&extension);
        if extension.is_absolute() {
            return Err(config_error(
                ConfigErrorKind::ConflictingConfig,
                Some("extends".into()),
                "absolute extends paths are not allowed".into(),
            ));
        }
        resolved =
            merge_provenance(resolved, provenance_set(&parent.join(extension), leaf, loading)?);
    }
    if local_keys.0 {
        resolved.paths = Some(
            local
                .suppressions
                .paths
                .into_iter()
                .enumerate()
                .map(|(index, entry)| SuppressionProvenance {
                    kind: "path".into(),
                    identity: entry.pattern,
                    reason: entry.reason,
                    owner: entry.owner,
                    ticket: entry.ticket,
                    expires_at: entry.expires_at,
                    defined_in: portable_path(&path),
                    yaml_path: format!("suppressions.paths[{index}]"),
                    inherited: path != leaf,
                })
                .collect(),
        );
    }
    if local_keys.1 {
        resolved.fingerprints = Some(
            local
                .suppressions
                .fingerprints
                .into_iter()
                .enumerate()
                .map(|(index, entry)| SuppressionProvenance {
                    kind: "fingerprint".into(),
                    identity: entry.fingerprint,
                    reason: entry.reason,
                    owner: entry.owner,
                    ticket: entry.ticket,
                    expires_at: entry.expires_at,
                    defined_in: portable_path(&path),
                    yaml_path: format!("suppressions.fingerprints[{index}]"),
                    inherited: path != leaf,
                })
                .collect(),
        );
    }
    loading.pop();
    Ok(resolved)
}

fn merge_provenance(mut base: ProvenanceSet, overlay: ProvenanceSet) -> ProvenanceSet {
    if overlay.paths.is_some() {
        base.paths = overlay.paths;
    }
    if overlay.fingerprints.is_some() {
        base.fingerprints = overlay.fingerprints;
    }
    base
}

impl SuppressionPruneResult {
    pub fn removed(self) -> usize {
        self.removed_paths + self.removed_fingerprints
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LeafSuppressions {
    suppressions: LeafSuppressionConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct LeafSuppressionConfig {
    paths: Vec<PathSuppression>,
    fingerprints: Vec<FingerprintSuppression>,
}

impl EditableConfigDocument {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = fs::read_to_string(path).map_err(|error| {
            config_error(
                ConfigErrorKind::Io,
                Some(path.display().to_string()),
                format!("cannot read configuration: {error}"),
            )
        })?;
        // Validate syntax up front. The narrow leaf view intentionally ignores unrelated keys.
        yaml_serde::from_str::<yaml_serde::Value>(&source).map_err(|error| {
            config_error(
                ConfigErrorKind::InvalidYaml,
                Some(path.display().to_string()),
                error.to_string(),
            )
        })?;
        Ok(Self { path: path.to_path_buf(), source })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn prune_expired_suppressions(
        &mut self,
        today: u64,
    ) -> Result<SuppressionPruneResult, ConfigError> {
        let leaf: LeafSuppressions = yaml_serde::from_str(&self.source).map_err(|error| {
            config_error(
                ConfigErrorKind::InvalidYaml,
                Some(self.path.display().to_string()),
                error.to_string(),
            )
        })?;
        let expired_paths = expired_indexes(
            leaf.suppressions.paths.iter().map(|entry| entry.expires_at.as_deref()),
            today,
        );
        let expired_fingerprints = expired_indexes(
            leaf.suppressions.fingerprints.iter().map(|entry| entry.expires_at.as_deref()),
            today,
        );
        if expired_paths.is_empty() && expired_fingerprints.is_empty() {
            return Ok(SuppressionPruneResult::default());
        }

        let mut lines = self.source.split_inclusive('\n').map(str::to_owned).collect::<Vec<_>>();
        if !self.source.is_empty() && !self.source.ends_with('\n') && lines.is_empty() {
            lines.push(self.source.clone());
        }
        let suppressions = mapping_key(&lines, 0, "suppressions")?.ok_or_else(|| {
            edit_error(&self.path, "cannot locate the local `suppressions` mapping")
        })?;
        let section_end = mapping_end(&lines, suppressions, indentation(&lines[suppressions]));
        prune_sequence(
            &mut lines,
            suppressions,
            section_end,
            "fingerprints",
            &expired_fingerprints,
            &self.path,
        )?;
        // Recalculate bounds because the first edit may have removed lines.
        let suppressions = mapping_key(&lines, 0, "suppressions")?.ok_or_else(|| {
            edit_error(&self.path, "cannot locate the local `suppressions` mapping")
        })?;
        let section_end = mapping_end(&lines, suppressions, indentation(&lines[suppressions]));
        prune_sequence(&mut lines, suppressions, section_end, "paths", &expired_paths, &self.path)?;
        self.source = lines.concat();
        Ok(SuppressionPruneResult {
            removed_paths: expired_paths.len(),
            removed_fingerprints: expired_fingerprints.len(),
        })
    }
}

fn expired_indexes<'a>(dates: impl Iterator<Item = Option<&'a str>>, today: u64) -> Vec<usize> {
    dates
        .enumerate()
        .filter_map(|(index, date)| {
            date.is_some_and(|date| expiration_day(date) <= today).then_some(index)
        })
        .collect()
}

fn prune_sequence(
    lines: &mut Vec<String>,
    section_start: usize,
    section_end: usize,
    key: &str,
    expired: &[usize],
    path: &Path,
) -> Result<(), ConfigError> {
    if expired.is_empty() {
        return Ok(());
    }
    let section_indent = indentation(&lines[section_start]);
    let Some(key_line) =
        mapping_key_in_range(lines, section_start + 1, section_end, section_indent, key)?
    else {
        return Err(edit_error(path, &format!("cannot locate local `suppressions.{key}`")));
    };
    let key_indent = indentation(&lines[key_line]);
    let value = content_after_key(&lines[key_line], key);
    if !value.is_empty() && value != "[]" && !value.starts_with('#') {
        return Err(edit_error(
            path,
            &format!("`suppressions.{key}` must use block-list YAML syntax to be pruned safely"),
        ));
    }
    let sequence_end = mapping_end(lines, key_line, key_indent).min(section_end);
    let item_starts = (key_line + 1..sequence_end)
        .filter(|&line| {
            indentation(&lines[line]) > key_indent && lines[line].trim_start().starts_with("- ")
        })
        .collect::<Vec<_>>();
    if item_starts.len() <= expired.iter().copied().max().unwrap_or(0) {
        return Err(edit_error(
            path,
            &format!("could not map parsed `suppressions.{key}` entries back to source lines"),
        ));
    }
    if expired.len() == item_starts.len() {
        let newline = if lines[key_line].ends_with("\r\n") {
            "\r\n"
        } else if lines[key_line].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let prefix = " ".repeat(key_indent);
        let comment = lines[key_line]
            .trim_end_matches(['\r', '\n'])
            .split_once('#')
            .map(|(_, comment)| format!(" #{}", comment.trim_end()))
            .unwrap_or_default();
        lines[key_line] = format!("{prefix}{key}: []{comment}{newline}");
    }
    let mut ranges = expired
        .iter()
        .map(|&index| {
            let start = item_starts[index];
            let mut end = item_starts.get(index + 1).copied().unwrap_or(sequence_end);
            while end > start + 1 {
                let candidate = lines[end - 1].trim();
                if candidate.is_empty() || candidate.starts_with('#') {
                    end -= 1;
                } else {
                    break;
                }
            }
            start..end
        })
        .collect::<Vec<_>>();
    ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
    for range in ranges {
        lines.drain(range);
    }
    Ok(())
}

fn mapping_key(
    lines: &[String],
    expected_indent: usize,
    key: &str,
) -> Result<Option<usize>, ConfigError> {
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if indentation(line) == expected_indent
            && trimmed.starts_with(&format!("{key}:"))
            && found.replace(index).is_some()
        {
            return Err(edit_error(Path::new("wae.yaml"), &format!("duplicate `{key}` key")));
        }
    }
    Ok(found)
}

fn mapping_key_in_range(
    lines: &[String],
    start: usize,
    end: usize,
    parent_indent: usize,
    key: &str,
) -> Result<Option<usize>, ConfigError> {
    let mut found = None;
    for (index, line) in lines.iter().enumerate().take(end.min(lines.len())).skip(start) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || indentation(line) <= parent_indent {
            continue;
        }
        if trimmed.starts_with(&format!("{key}:")) && found.replace(index).is_some() {
            return Err(edit_error(Path::new("wae.yaml"), &format!("duplicate `{key}` key")));
        }
    }
    Ok(found)
}

fn mapping_end(lines: &[String], start: usize, indent: usize) -> usize {
    (start + 1..lines.len())
        .find(|&line| {
            let trimmed = lines[line].trim();
            !trimmed.is_empty() && !trimmed.starts_with('#') && indentation(&lines[line]) <= indent
        })
        .unwrap_or(lines.len())
}

fn indentation(line: &str) -> usize {
    line.bytes().take_while(|byte| *byte == b' ').count()
}

fn content_after_key<'a>(line: &'a str, key: &str) -> &'a str {
    line.trim_start().strip_prefix(key).and_then(|line| line.strip_prefix(':')).unwrap_or("").trim()
}

fn edit_error(path: &Path, message: &str) -> ConfigError {
    config_error(
        ConfigErrorKind::ConflictingConfig,
        Some(path.display().to_string()),
        message.to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_preserves_extends_comments_order_and_live_entries() {
        let source = "# team policy\nextends: ./base.yaml\nversion: 1\nsuppressions:\n  require_reason: true\n  # migration paths\n  paths:\n    - pattern: src/legacy/**\n      rules: [ARCH-003]\n      reason: migration\n      expires_at: 2020-01-01\n    # live owner note\n    - pattern: src/live/**\n      rules: [ARCH-004]\n      reason: active\n      expires_at: 2099-01-01\n  fingerprints: # graph exceptions\n    - fingerprint: deadbeef\n      reason: old\n      expires_at: 2020-01-01\n    # retained registry note\noutput:\n  format: human\n";
        let path = PathBuf::from("/tmp/wae.yaml");
        let mut document = EditableConfigDocument { path, source: source.into() };
        let result = document.prune_expired_suppressions(expiration_day("2026-01-01")).unwrap();
        assert_eq!(result.removed_paths, 1);
        assert_eq!(result.removed_fingerprints, 1);
        assert!(document.source().contains("extends: ./base.yaml"));
        assert!(document.source().contains("# team policy"));
        assert!(document.source().contains("# migration paths"));
        assert!(document.source().contains("# live owner note"));
        assert!(document.source().contains("pattern: src/live/**"));
        assert!(!document.source().contains("src/legacy/**"));
        assert!(document.source().contains("fingerprints: [] # graph exceptions"));
        assert!(document.source().contains("# retained registry note"));
        assert!(document.source().contains("output:\n  format: human"));
    }

    #[test]
    fn prune_is_a_byte_for_byte_noop_without_expired_local_entries() {
        let source = "extends: ./base.yaml\nversion: 1\n# retained\n";
        let mut document =
            EditableConfigDocument { path: PathBuf::from("/tmp/wae.yaml"), source: source.into() };
        assert_eq!(document.prune_expired_suppressions(u64::MAX).unwrap().removed(), 0);
        assert_eq!(document.source(), source);
    }

    #[test]
    fn prune_rejects_unsafe_flow_sequences_without_mutating_source() {
        let source = "version: 1\nsuppressions:\n  fingerprints: [{ fingerprint: old, reason: migration, expires_at: 2020-01-01 }]\n";
        let mut document =
            EditableConfigDocument { path: PathBuf::from("/tmp/wae.yaml"), source: source.into() };
        let error = document.prune_expired_suppressions(expiration_day("2026-01-01")).unwrap_err();
        assert!(error.message.contains("block-list YAML syntax"));
        assert_eq!(document.source(), source);
    }

    #[test]
    fn prune_preserves_crlf_line_endings() {
        let source = "version: 1\r\nsuppressions:\r\n  fingerprints: # owned\r\n    - fingerprint: old\r\n      reason: migration\r\n      expires_at: 2020-01-01\r\noutput:\r\n  format: human\r\n";
        let mut document = EditableConfigDocument {
            path: PathBuf::from("C:/project/wae.yaml"),
            source: source.into(),
        };
        document.prune_expired_suppressions(expiration_day("2026-01-01")).unwrap();
        assert!(document.source().contains("fingerprints: [] # owned\r\n"));
        assert!(!document.source().replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn provenance_paths_use_portable_separators() {
        assert_eq!(
            portable_path(Path::new(r"C:\repo\config\base.yaml")),
            "C:/repo/config/base.yaml"
        );
    }

    #[test]
    fn provenance_tracks_effective_inherited_and_replaced_suppression_sources() {
        let root = std::env::temp_dir().join(format!("wae-provenance-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/base.yaml"),
            "version: 1\nsuppressions:\n  paths:\n    - pattern: inherited/**\n      rules: [ARCH-003]\n      reason: base\n  fingerprints:\n    - fingerprint: inherited-id\n      reason: base graph\n",
        )
        .unwrap();
        fs::write(
            root.join("wae.yaml"),
            "extends: config/base.yaml\nversion: 1\nsuppressions:\n  paths:\n    - pattern: local/**\n      rules: [ARCH-004]\n      reason: leaf\n",
        )
        .unwrap();
        let entries = suppression_provenance(&root.join("wae.yaml")).unwrap();
        assert_eq!(entries.len(), 2);
        let path = entries.iter().find(|entry| entry.kind == "path").unwrap();
        assert_eq!(path.identity, "local/**");
        assert!(!path.inherited);
        let fingerprint = entries.iter().find(|entry| entry.kind == "fingerprint").unwrap();
        assert_eq!(fingerprint.identity, "inherited-id");
        assert!(fingerprint.inherited);
        assert!(fingerprint.defined_in.ends_with("config/base.yaml"));
        fs::remove_dir_all(root).unwrap();
    }
}
