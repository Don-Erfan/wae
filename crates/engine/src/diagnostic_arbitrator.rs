use std::collections::HashMap;

use wae_core::domain::{Diagnostic, ModuleId, Severity};

/// Resolves intentional semantic overlap after independent rule evaluation. Arbitration is
/// deterministic and cannot depend on the order in which rules happen to run.
pub(crate) struct DiagnosticArbitrator;

impl DiagnosticArbitrator {
    pub(crate) fn arbitrate(diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
        let mut output = Vec::new();
        let mut groups = HashMap::<OverlapKey, Vec<Diagnostic>>::new();
        for diagnostic in diagnostics {
            if let Some(family) = overlap_family(&diagnostic.rule_id.0) {
                if !diagnostic.dependency_path.is_empty() {
                    groups
                        .entry(OverlapKey { family, path: diagnostic.dependency_path.clone() })
                        .or_default()
                        .push(diagnostic);
                    continue;
                }
            }
            output.push(diagnostic);
        }
        for (_, mut candidates) in groups {
            candidates.sort_by(|left, right| {
                severity_rank(&right.severity)
                    .cmp(&severity_rank(&left.severity))
                    .then_with(|| specificity(&right.rule_id.0).cmp(&specificity(&left.rule_id.0)))
                    .then_with(|| left.rule_id.0.cmp(&right.rule_id.0))
            });
            let mut selected = candidates.remove(0);
            let mut related_rules = candidates
                .iter()
                .map(|diagnostic| diagnostic.rule_id.0.clone())
                .chain(std::iter::once(selected.rule_id.0.clone()))
                .collect::<Vec<_>>();
            related_rules.sort();
            related_rules.dedup();
            if related_rules.len() > 1 {
                selected.metadata.insert("related_rules".into(), related_rules.join(","));
            }
            output.push(selected);
        }
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OverlapKey {
    family: &'static str,
    path: Vec<ModuleId>,
}

fn overlap_family(rule_id: &str) -> Option<&'static str> {
    matches!(rule_id, "ARCH-004" | "ARCH-005").then_some("feature-visibility")
}

fn severity_rank(severity: &Severity) -> u8 {
    match severity {
        Severity::Error => 3,
        Severity::Warning => 2,
        Severity::Info => 1,
    }
}

fn specificity(rule_id: &str) -> u8 {
    match rule_id {
        "ARCH-005" => 2,
        "ARCH-004" => 1,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic(rule: &str, severity: Severity) -> Diagnostic {
        let mut diagnostic = Diagnostic::new(rule, rule);
        diagnostic.severity = severity;
        diagnostic.dependency_path =
            vec![ModuleId("feature-a.ts".into()), ModuleId("feature-b/private.ts".into())];
        diagnostic.refresh_fingerprint();
        diagnostic
    }

    #[test]
    fn preserves_the_highest_severity_and_records_every_related_rule() {
        let result = DiagnosticArbitrator::arbitrate(vec![
            diagnostic("ARCH-005", Severity::Warning),
            diagnostic("ARCH-004", Severity::Error),
        ]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].rule_id.0, "ARCH-004");
        assert_eq!(result[0].severity, Severity::Error);
        assert_eq!(result[0].metadata["related_rules"], "ARCH-004,ARCH-005");
    }

    #[test]
    fn specificity_breaks_equal_severity_ties_independently_of_input_order() {
        for diagnostics in [
            vec![diagnostic("ARCH-004", Severity::Error), diagnostic("ARCH-005", Severity::Error)],
            vec![diagnostic("ARCH-005", Severity::Error), diagnostic("ARCH-004", Severity::Error)],
        ] {
            let result = DiagnosticArbitrator::arbitrate(diagnostics);
            assert_eq!(result[0].rule_id.0, "ARCH-005");
        }
    }

    #[test]
    fn presentation_metadata_does_not_change_the_selected_fingerprint() {
        let original = diagnostic("ARCH-005", Severity::Error);
        let expected = original.fingerprint.clone();
        let result = DiagnosticArbitrator::arbitrate(vec![
            diagnostic("ARCH-004", Severity::Error),
            original,
        ]);
        assert_eq!(result[0].rule_id.0, "ARCH-005");
        assert_eq!(result[0].fingerprint, expected);
        assert_eq!(result[0].metadata["related_rules"], "ARCH-004,ARCH-005");
    }
}
