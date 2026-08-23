use serde_json::json;
use wae_core::domain::{Diagnostic, Severity};
use wae_engine::Analysis;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
    Jsonl,
    Sarif,
}

impl Format {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "json" => Some(Self::Json),
            "jsonl" => Some(Self::Jsonl),
            "sarif" => Some(Self::Sarif),
            _ => None,
        }
    }
}

pub fn render(analysis: &Analysis, format: Format) -> Result<String, serde_json::Error> {
    match format {
        Format::Human => Ok(human(analysis)),
        Format::Json => json_report(analysis),
        Format::Jsonl => jsonl(analysis),
        Format::Sarif => sarif(analysis),
    }
}

fn human(analysis: &Analysis) -> String {
    let errors = analysis.diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = analysis.diagnostics.iter().filter(|d| d.severity == Severity::Warning).count();
    let mut output = format!(
        "Analyzing {} modules...\n\nArchitecture\n\n✖ {errors} errors\n⚠ {warnings} warnings\n",
        analysis.project.modules.len()
    );
    for diagnostic in &analysis.diagnostics {
        output.push_str(&format!(
            "\n{} [{}]\n{}",
            diagnostic.rule_id.0, diagnostic.fingerprint, diagnostic.message
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
        output.push('\n');
    }
    if analysis.diagnostics.is_empty() {
        output.push_str("\n✓ Passed\n");
    }
    output.trim_end().to_string()
}

fn json_report(analysis: &Analysis) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(
        &json!({ "schemaVersion": analysis.schema_version, "modules": analysis.project.modules.len(), "diagnostics": analysis.diagnostics }),
    )
}

fn jsonl(analysis: &Analysis) -> Result<String, serde_json::Error> {
    analysis
        .diagnostics
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
}

fn sarif(analysis: &Analysis) -> Result<String, serde_json::Error> {
    let results = analysis.diagnostics.iter().map(sarif_result).collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json", "version": "2.1.0",
        "runs": [{ "tool": { "driver": { "name": "WAE", "informationUri": "https://github.com/Don-Erfan/wae" } }, "results": results }]
    }))
}

fn sarif_result(diagnostic: &Diagnostic) -> serde_json::Value {
    let level = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "note",
    };
    let locations = diagnostic.primary_location.as_ref().map(|location| vec![json!({ "physicalLocation": { "artifactLocation": { "uri": location.file }, "region": { "startLine": location.line.max(1), "startColumn": location.column.max(1) } } })]).unwrap_or_default();
    json!({ "ruleId": diagnostic.rule_id.0, "level": level, "message": { "text": diagnostic.message }, "partialFingerprints": { "waeViolationId": diagnostic.fingerprint }, "locations": locations })
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
}
