use std::path::Path;

use wae_config::CONFIG_FILE;
use wae_discovery::{DiscoveryReport, discover};

use crate::CliOutput;

pub(super) fn run(root: &Path, json: bool, write: bool, force: bool) -> CliOutput {
    let report = match discover(root) {
        Ok(report) => report,
        Err(error) => return CliOutput::project_error(error),
    };
    if write {
        let path = root.join(CONFIG_FILE);
        if path.exists() && !force {
            return CliOutput::project_error(format!(
                "configuration already exists: {}\nReview the proposal without --write, or explicitly use --write --force.",
                path.display()
            ));
        }
        let yaml = match report.suggested_config.to_yaml() {
            Ok(yaml) => yaml,
            Err(error) => return CliOutput::internal_error(error.message),
        };
        if let Err(error) = std::fs::write(&path, yaml) {
            return CliOutput::project_error(format!("cannot write `{}`: {error}", path.display()));
        }
    }
    if json {
        return match serde_json::to_string_pretty(&report) {
            Ok(output) => CliOutput::success(output),
            Err(error) => CliOutput::internal_error(error.to_string()),
        };
    }
    human(report, write, root)
}

fn human(report: DiscoveryReport, wrote: bool, root: &Path) -> CliOutput {
    let mut lines = vec![
        format!("Suggested architecture: {:?}", report.project_kind),
        format!("Confidence: {}%", report.confidence),
    ];
    if report.evidence.is_empty() {
        lines.push("Evidence: none; the safe blank architecture is proposed.".into());
    } else {
        lines.push("Evidence:".into());
        lines.extend(
            report
                .evidence
                .iter()
                .map(|item| format!("- {} at {} (weight {})", item.signal, item.path, item.weight)),
        );
    }
    if !report.feature_clusters.is_empty() {
        lines.push(format!("Feature clusters: {}", report.feature_clusters.join(", ")));
    }
    if wrote {
        lines.push(format!(
            "Approved configuration written to {}",
            root.join(CONFIG_FILE).display()
        ));
    } else {
        lines.push(
            "No files changed. Review this proposal, then run `wae discover --write`.".into(),
        );
        match report.suggested_config.to_yaml() {
            Ok(yaml) => lines.extend(["".into(), yaml]),
            Err(error) => return CliOutput::internal_error(error.message),
        }
    }
    CliOutput::success(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_is_read_only_and_write_requires_explicit_overwrite() {
        let root = std::env::temp_dir().join(format!("wae-discover-cli-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src/features/auth")).unwrap();
        std::fs::write(root.join("package.json"), r#"{"dependencies":{"next":"15"}}"#).unwrap();
        let preview = run(&root, false, false, false);
        assert_eq!(preview.exit_code, 0);
        assert!(!root.join(CONFIG_FILE).exists());
        assert!(preview.stdout.contains("NextJs"));
        assert_eq!(run(&root, false, true, false).exit_code, 0);
        assert_eq!(run(&root, false, true, false).exit_code, 2);
        assert_eq!(run(&root, true, true, true).exit_code, 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
