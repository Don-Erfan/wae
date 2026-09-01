use std::collections::HashMap;

use globset::Glob;
use wae_config::SuppressionConfig;
use wae_core::{
    domain::{Diagnostic, Severity, SourceLocation},
    rule_registry,
};

#[derive(Clone, Debug)]
pub(crate) struct SuppressionDirective {
    file: String,
    line: usize,
    rule_id: String,
    reason: String,
    matched_count: usize,
    file_scope: bool,
}

pub(crate) fn collect(
    file: &str,
    source: &str,
    require_reason: bool,
    directives: &mut Vec<SuppressionDirective>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, line) in source.lines().enumerate() {
        // Suppressions are deliberately restricted to standalone line comments. This avoids
        // interpreting examples in string literals, templates and documentation as directives.
        let Some(comment) = line.trim_start().strip_prefix("//") else { continue };
        let comment = comment.trim_start();
        let (declaration, file_scope) =
            if let Some(declaration) = comment.strip_prefix("wae-ignore-file") {
                (declaration, true)
            } else if let Some(declaration) = comment.strip_prefix("wae-ignore") {
                (declaration, false)
            } else {
                continue;
            };
        if declaration.chars().next().is_some_and(|character| !character.is_whitespace()) {
            continue;
        }
        let declaration = declaration.trim();
        let (rule_id, reason) = declaration
            .split_once("--")
            .map_or((declaration, ""), |(rule, reason)| (rule.trim(), reason.trim()));
        let line_number = index + 1;
        let error = if rule_registry::descriptor(rule_id).is_none() {
            Some(format!("Suppression references unknown rule `{rule_id}`"))
        } else if require_reason && reason.is_empty() {
            Some(format!("Suppression for `{rule_id}` requires a reason after `--`"))
        } else {
            None
        };
        if let Some(message) = error {
            diagnostics.push(warning(file, line_number, message));
        } else {
            directives.push(SuppressionDirective {
                file: file.into(),
                line: line_number,
                rule_id: rule_id.into(),
                reason: reason.into(),
                matched_count: 0,
                file_scope,
            });
        }
    }
}

pub(crate) fn apply(
    diagnostics: &mut Vec<Diagnostic>,
    directives: &mut [SuppressionDirective],
    report_unused: bool,
) {
    let mut exact = HashMap::new();
    let mut files = HashMap::new();
    for (index, directive) in directives.iter().enumerate() {
        let key = (directive.file.clone(), directive.rule_id.clone());
        if directive.file_scope {
            files.entry(key).or_insert(index);
        } else {
            exact.entry((key.0, key.1, directive.line)).or_insert(index);
        }
    }
    for diagnostic in diagnostics.iter_mut() {
        let Some(location) = &diagnostic.primary_location else { continue };
        let key = (location.file.clone(), diagnostic.rule_id.0.clone());
        let index = files.get(&key).copied().or_else(|| {
            exact
                .get(&(key.0.clone(), key.1.clone(), location.line))
                .or_else(|| {
                    location.line.checked_sub(1).and_then(|line| exact.get(&(key.0, key.1, line)))
                })
                .copied()
        });
        if let Some(directive) = index.and_then(|index| directives.get_mut(index)) {
            directive.matched_count += 1;
            diagnostic.suppressed = true;
            diagnostic.suppression_reason = Some(directive.reason.clone());
        }
    }
    if report_unused {
        diagnostics.extend(directives.iter().filter(|directive| directive.matched_count == 0).map(
            |directive| {
                warning(
                    &directive.file,
                    directive.line,
                    format!("Unused suppression for `{}`", directive.rule_id),
                )
            },
        ));
    }
}

pub(crate) fn apply_config(diagnostics: &mut [Diagnostic], config: &SuppressionConfig) {
    let paths = config
        .paths
        .iter()
        .filter_map(|entry| {
            Glob::new(&entry.pattern).ok().map(|glob| (glob.compile_matcher(), entry))
        })
        .collect::<Vec<_>>();
    for diagnostic in diagnostics {
        if diagnostic.suppressed {
            continue;
        }
        if let Some(entry) = config.fingerprints.iter().find(|entry| {
            entry.fingerprint == diagnostic.fingerprint
                || diagnostic
                    .legacy_fingerprint_aliases()
                    .iter()
                    .any(|alias| alias == &entry.fingerprint)
        }) {
            diagnostic.suppressed = true;
            diagnostic.suppression_reason = Some(entry.reason.clone());
            continue;
        }
        let files = diagnostic
            .primary_location
            .iter()
            .map(|location| location.file.as_str())
            .chain(diagnostic.dependency_path.iter().map(|module| module.0.as_str()));
        if let Some((_, entry)) = paths.iter().find(|(matcher, entry)| {
            entry.rules.iter().any(|rule| rule == &diagnostic.rule_id.0)
                && files.clone().any(|file| matcher.is_match(file))
        }) {
            diagnostic.suppressed = true;
            diagnostic.suppression_reason = Some(entry.reason.clone());
        }
    }
}

fn warning(file: &str, line: usize, message: String) -> Diagnostic {
    let mut diagnostic = Diagnostic::new("SUPPRESS-001", message);
    diagnostic.severity = Severity::Warning;
    diagnostic.primary_location = Some(SourceLocation { file: file.into(), line, column: 1 });
    diagnostic.refresh_fingerprint();
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(rule: &str, line: usize) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(rule, "test");
        diagnostic.primary_location =
            Some(SourceLocation { file: "src/app.ts".into(), line, column: 1 });
        diagnostic
    }

    #[test]
    fn ignores_directive_text_inside_source_strings() {
        let mut directives = Vec::new();
        let mut diagnostics = Vec::new();
        collect(
            "src/app.ts",
            r#"const example = "// wae-ignore ARCH-001 -- documentation";"#,
            true,
            &mut directives,
            &mut diagnostics,
        );
        assert!(directives.is_empty());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn one_directive_suppresses_every_matching_diagnostic_in_its_scope() {
        let mut directives = Vec::new();
        let mut collection_diagnostics = Vec::new();
        collect(
            "src/app.ts",
            "// wae-ignore ARCH-003 -- approved boundary\nimports();",
            true,
            &mut directives,
            &mut collection_diagnostics,
        );
        let mut diagnostics = vec![diagnostic("ARCH-003", 2), diagnostic("ARCH-003", 2)];
        apply(&mut diagnostics, &mut directives, true);
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.suppressed));
        assert_eq!(directives[0].matched_count, 2);
        assert!(!diagnostics.iter().any(|diagnostic| diagnostic.rule_id.0 == "SUPPRESS-001"));
    }

    #[test]
    fn file_directive_suppresses_diagnostics_anywhere_in_the_file() {
        let mut directives = Vec::new();
        let mut collection_diagnostics = Vec::new();
        collect(
            "src/app.ts",
            "// wae-ignore-file ARCH-003 -- legacy module\nimports();",
            true,
            &mut directives,
            &mut collection_diagnostics,
        );
        let mut diagnostics = vec![diagnostic("ARCH-003", 99)];
        apply(&mut diagnostics, &mut directives, true);
        assert!(diagnostics[0].suppressed);
        assert_eq!(diagnostics[0].suppression_reason.as_deref(), Some("legacy module"));
    }

    #[test]
    fn config_suppresses_by_path_and_stable_identity() {
        let by_path = wae_config::PathSuppression {
            pattern: "src/legacy/**".into(),
            rules: vec!["ARCH-003".into()],
            reason: "migration ARC-42".into(),
        };
        let mut path_diagnostic = diagnostic("ARCH-003", 3);
        path_diagnostic.primary_location.as_mut().unwrap().file = "src/legacy/app.ts".into();
        let mut identity_diagnostic = diagnostic("ARCH-001", 1);
        identity_diagnostic.refresh_fingerprint();
        let config = SuppressionConfig {
            paths: vec![by_path],
            fingerprints: vec![wae_config::FingerprintSuppression {
                fingerprint: identity_diagnostic.fingerprint.clone(),
                reason: "accepted cycle ARC-99".into(),
            }],
            ..Default::default()
        };
        let mut diagnostics = vec![path_diagnostic, identity_diagnostic];
        apply_config(&mut diagnostics, &config);
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.suppressed));
    }
}
