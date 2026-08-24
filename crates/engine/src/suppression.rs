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
    used: bool,
}

pub(crate) fn collect(
    file: &str,
    source: &str,
    require_reason: bool,
    directives: &mut Vec<SuppressionDirective>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (index, line) in source.lines().enumerate() {
        let Some((_, declaration)) = line.split_once("wae-ignore") else { continue };
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
                used: false,
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
            !directive.used
                && directive.rule_id == diagnostic.rule_id.0
                && directive.file == location.file
                && (directive.line == location.line || directive.line + 1 == location.line)
        }) {
            directive.used = true;
            diagnostic.suppressed = true;
            diagnostic.suppression_reason = Some(directive.reason.clone());
        }
    }
    if report_unused {
        diagnostics.extend(directives.iter().filter(|directive| !directive.used).map(
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
