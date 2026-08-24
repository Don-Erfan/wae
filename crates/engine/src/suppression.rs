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
        let Some(declaration) = comment.trim_start().strip_prefix("wae-ignore") else {
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
            });
        }
    }
}

pub(crate) fn apply(
    diagnostics: &mut Vec<Diagnostic>,
    directives: &mut [SuppressionDirective],
    report_unused: bool,
) {
    for diagnostic in diagnostics.iter_mut() {
        let Some(location) = &diagnostic.primary_location else { continue };
        if let Some(directive) = directives.iter_mut().find(|directive| {
            directive.rule_id == diagnostic.rule_id.0
                && directive.file == location.file
                && (directive.line == location.line || directive.line + 1 == location.line)
        }) {
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
}
