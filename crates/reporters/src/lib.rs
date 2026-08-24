use serde_json::json;
pub use wae_config::OutputFormat as Format;
use wae_core::domain::{Diagnostic, ModuleKind, Severity};
use wae_engine::Analysis;

pub fn render(analysis: &Analysis, format: Format) -> Result<String, serde_json::Error> {
    match format {
        Format::Human => Ok(human(analysis)),
        Format::Json => json_report(analysis),
        Format::Jsonl => jsonl(analysis),
        Format::Sarif => sarif(analysis),
    }
}

fn human(analysis: &Analysis) -> String {
    let counts = counts(analysis);
    let errors = analysis
        .diagnostics
        .iter()
        .filter(|d| !d.suppressed && d.severity == Severity::Error)
        .count();
    let warnings = analysis
        .diagnostics
        .iter()
        .filter(|d| !d.suppressed && d.severity == Severity::Warning)
        .count();
    let mut output = format!(
        "Analyzing {} source modules, {} excluded modules, {} external packages and {} dependencies...\n\nArchitecture\n\n✖ {errors} errors\n⚠ {warnings} warnings\n",
        counts.source_modules,
        counts.excluded_modules,
        counts.external_packages,
        counts.dependencies,
    );
    for diagnostic in &analysis.diagnostics {
        output.push_str(&format!(
            "\n{}{} [{}]\n{}",
            diagnostic.rule_id.0,
            if diagnostic.suppressed { " (suppressed)" } else { "" },
            diagnostic.fingerprint,
            diagnostic.message
        ));
        if let Some(location) = &diagnostic.primary_location {
            output.push_str(&format!("\n{}:{}:{}", location.file, location.line, location.column));
        }
        if diagnostic.dependency_path.len() > 1 {
            output.push('\n');
            output.push_str(
                &diagnostic
                    .dependency_path
                    .iter()
                    .map(|m| m.0.as_str())
                    .collect::<Vec<_>>()
                    .join("\n→ "),
            );
        }
        if let Some(suggestion) = &diagnostic.suggestion {
            output.push_str(&format!("\nSuggestion: {suggestion}"));
        }
        if let Some(reason) = &diagnostic.suppression_reason {
            output.push_str(&format!("\nSuppression reason: {reason}"));
        }
        output.push('\n');
    }
    if analysis.diagnostics.is_empty() {
        output.push_str("\n✓ Passed\n");
    }
    output.trim_end().to_string()
}

fn json_report(analysis: &Analysis) -> Result<String, serde_json::Error> {
    let counts = counts(analysis);
    serde_json::to_string_pretty(&json!({
        "schemaVersion": analysis.schema_version,
        "modules": analysis.project.modules.len(),
        "sourceModules": counts.source_modules,
        "excludedModules": counts.excluded_modules,
        "externalPackages": counts.external_packages,
        "dependencies": counts.dependencies,
        "diagnostics": analysis.diagnostics
    }))
}

fn jsonl(analysis: &Analysis) -> Result<String, serde_json::Error> {
    let counts = counts(analysis);
    let mut events = vec![serde_json::to_string(&json!({
        "schemaVersion": analysis.schema_version,
        "type": "analysis",
        "modules": analysis.project.modules.len(),
        "sourceModules": counts.source_modules,
        "excludedModules": counts.excluded_modules,
        "externalPackages": counts.external_packages,
        "dependencies": counts.dependencies,
    }))?];
    for diagnostic in &analysis.diagnostics {
        events.push(serde_json::to_string(&json!({
            "schemaVersion": analysis.schema_version,
            "type": "diagnostic",
            "diagnostic": diagnostic,
        }))?);
    }
    Ok(events.join("\n"))
}

fn sarif(analysis: &Analysis) -> Result<String, serde_json::Error> {
    let results = analysis.diagnostics.iter().map(sarif_result).collect::<Vec<_>>();
    let mut rule_ids = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule_id.0.as_str())
        .collect::<Vec<_>>();
    rule_ids.sort_unstable();
    rule_ids.dedup();
    let rules = rule_ids.into_iter().map(sarif_rule).collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json", "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "WAE",
                "semanticVersion": env!("CARGO_PKG_VERSION"),
                "informationUri": "https://github.com/Don-Erfan/wae",
                "rules": rules
            } },
            "invocations": [{ "executionSuccessful": true }],
            "results": results
        }]
    }))
}

#[derive(Clone, Copy)]
struct Counts {
    source_modules: usize,
    excluded_modules: usize,
    external_packages: usize,
    dependencies: usize,
}

fn counts(analysis: &Analysis) -> Counts {
    Counts {
        source_modules: analysis
            .project
            .modules
            .iter()
            .filter(|module| module.kind == ModuleKind::Source)
            .count(),
        excluded_modules: analysis
            .project
            .modules
            .iter()
            .filter(|module| module.kind == ModuleKind::Excluded)
            .count(),
        external_packages: analysis
            .project
            .modules
            .iter()
            .filter(|module| module.kind == ModuleKind::External)
            .count(),
        dependencies: analysis.project.resolved_dependencies.len(),
    }
}

fn sarif_rule(rule_id: &str) -> serde_json::Value {
    let descriptor = wae_core::rule_registry::descriptor(rule_id);
    let title = descriptor.map_or("WAE diagnostic", |rule| rule.title);
    let description =
        descriptor.map_or("Reports a Web Architecture Engine diagnostic.", |rule| rule.description);
    let category = descriptor.map_or("architecture", |rule| rule.category);
    json!({
        "id": rule_id,
        "name": title.replace(' ', ""),
        "shortDescription": { "text": title },
        "fullDescription": { "text": description },
        "helpUri": format!("https://github.com/Don-Erfan/wae/blob/master/docs/RULES.md#{}", rule_id.to_ascii_lowercase()),
        "defaultConfiguration": { "level": "error" },
        "properties": { "tags": ["architecture", category] }
    })
}

fn sarif_result(diagnostic: &Diagnostic) -> serde_json::Value {
    let level = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    };
    let locations = diagnostic.primary_location.as_ref().map(|location| vec![json!({ "physicalLocation": { "artifactLocation": { "uri": location.file }, "region": { "startLine": location.line.max(1), "startColumn": location.column.max(1) } } })]).unwrap_or_default();
    let suppressions = if diagnostic.suppressed {
        vec![json!({
            "kind": "inSource", "status": "accepted",
            "justification": diagnostic.suppression_reason.as_deref().unwrap_or("WAE source suppression")
        })]
    } else {
        Vec::new()
    };
    json!({ "ruleId": diagnostic.rule_id.0, "level": level, "message": { "text": diagnostic.message }, "partialFingerprints": { "waeViolationId": diagnostic.fingerprint }, "locations": locations, "suppressions": suppressions })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_json_has_a_versioned_schema() {
        let analysis = Analysis {
            schema_version: 1,
            project: Default::default(),
            graph: Default::default(),
            diagnostics: vec![],
        };
        assert!(json_report(&analysis).unwrap().contains("\"schemaVersion\": 1"));
    }

    #[test]
    fn sarif_report_has_the_standard_envelope() {
        let analysis = Analysis {
            schema_version: 1,
            project: Default::default(),
            graph: Default::default(),
            diagnostics: vec![],
        };
        let output = sarif(&analysis).unwrap();
        assert!(output.contains("2.1.0"));
        assert!(output.contains("\"name\": \"WAE\""));
    }

    #[test]
    fn jsonl_events_are_individually_versioned() {
        let analysis = Analysis {
            schema_version: 1,
            project: Default::default(),
            graph: Default::default(),
            diagnostics: vec![Diagnostic::new("ARCH-001", "cycle")],
        };
        let output = jsonl(&analysis).unwrap();
        for line in output.lines() {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(value["schemaVersion"], 1);
        }
    }

    #[test]
    fn sarif_contains_rule_descriptors_and_fingerprints() {
        let mut diagnostic = Diagnostic::new("ARCH-003", "Layer violation");
        diagnostic.refresh_fingerprint();
        let analysis = Analysis {
            schema_version: 1,
            project: Default::default(),
            graph: Default::default(),
            diagnostics: vec![diagnostic],
        };
        let value: serde_json::Value = serde_json::from_str(&sarif(&analysis).unwrap()).unwrap();
        assert_eq!(value["runs"][0]["invocations"][0]["executionSuccessful"], true);
        assert_eq!(value["runs"][0]["tool"]["driver"]["rules"][0]["id"], "ARCH-003");
        assert!(
            value["runs"][0]["results"][0]["partialFingerprints"]["waeViolationId"].is_string()
        );
    }
}
