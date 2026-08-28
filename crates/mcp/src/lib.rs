use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wae_core::domain::{ModuleId, ModuleKind};
use wae_engine::{AnalyzeRequest, Engine};

pub fn handle_message(message: Value, default_root: &Path) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message.get("method").and_then(Value::as_str)?;
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "wae-mcp", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call_tool(
            message.pointer("/params/name").and_then(Value::as_str).unwrap_or_default(),
            message.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({})),
            default_root,
        )),
        _ => return Some(error(id, -32601, format!("unknown method `{method}`"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(message) => error(id, -32000, message),
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "architecture_check",
            "description": "Analyze a JS/TS project and return versioned architecture diagnostics.",
            "inputSchema": root_schema()
        },
        {
            "name": "architecture_explain",
            "description": "Explain a WAE rule by stable rule id.",
            "inputSchema": {
                "type": "object",
                "properties": { "ruleId": { "type": "string" } },
                "required": ["ruleId"],
                "additionalProperties": false
            }
        },
        {
            "name": "dependency_path",
            "description": "Return the deterministic shortest resolved dependency path between two modules.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "from": { "type": "string" },
                    "to": { "type": "string" }
                },
                "required": ["from", "to"],
                "additionalProperties": false
            }
        },
        {
            "name": "architecture_model",
            "description": "Return modules, packages, layers, runtimes, framework metadata, edges and violation counts.",
            "inputSchema": root_schema()
        }
    ])
}

fn root_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "root": { "type": "string", "description": "Project root; defaults to the server working directory." } },
        "additionalProperties": false
    })
}

fn call_tool(name: &str, arguments: Value, default_root: &Path) -> Value {
    match execute_tool(name, arguments, default_root) {
        Ok(structured) => {
            let text = serde_json::to_string_pretty(&structured)
                .unwrap_or_else(|error| format!("could not serialize tool result: {error}"));
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": structured,
                "isError": false
            })
        }
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    }
}

fn execute_tool(name: &str, arguments: Value, default_root: &Path) -> Result<Value, String> {
    let root = arguments
        .get("root")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_root.to_path_buf());
    let structured = match name {
        "architecture_check" => {
            let analysis = analyze(&root)?;
            json!({
                "schemaVersion": analysis.schema_version,
                "sourceModules": analysis.project.modules.iter().filter(|module| module.kind == ModuleKind::Source).count(),
                "dependencies": analysis.project.dependencies.len(),
                "diagnostics": analysis.diagnostics,
                "timings": {
                    "discoveryMs": analysis.timings.discovery_ms,
                    "moduleAnalysisMs": analysis.timings.module_analysis_ms,
                    "graphMs": analysis.timings.graph_ms,
                    "rulesMs": analysis.timings.rules_ms,
                    "totalMs": analysis.timings.total_ms
                }
            })
        }
        "architecture_explain" => {
            let rule =
                arguments.get("ruleId").and_then(Value::as_str).ok_or("ruleId is required")?;
            let descriptor = wae_core::rule_registry::descriptor(rule)
                .ok_or_else(|| format!("unknown rule `{rule}`"))?;
            json!({
                "id": descriptor.id,
                "title": descriptor.title,
                "description": descriptor.description,
                "category": descriptor.category,
                "configurable": descriptor.configurable
            })
        }
        "dependency_path" => {
            let from = arguments.get("from").and_then(Value::as_str).ok_or("from is required")?;
            let to = arguments.get("to").and_then(Value::as_str).ok_or("to is required")?;
            let analysis = analyze(&root)?;
            let path = analysis
                .graph
                .shortest_path(&ModuleId(from.into()), &ModuleId(to.into()))
                .map(|path| path.into_iter().map(|module| module.0).collect::<Vec<_>>());
            json!({ "from": from, "to": to, "path": path })
        }
        "architecture_model" => {
            let analysis = analyze(&root)?;
            let modules = analysis
                .project
                .modules
                .iter()
                .map(|module| {
                    json!({
                        "id": module.id.0,
                        "package": module.package.0,
                        "kind": format!("{:?}", module.kind),
                        "layer": module.layer.as_ref().map(|layer| &layer.0),
                        "runtime": format!("{:?}", module.runtime),
                        "framework": module.framework_metadata
                    })
                })
                .collect::<Vec<_>>();
            let edges = analysis
                .project
                .dependencies
                .iter()
                .map(|edge| {
                    json!({
                        "from": edge.from.0, "to": edge.to.0, "kind": format!("{:?}", edge.kind)
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "schemaVersion": analysis.schema_version,
                "modules": modules,
                "edges": edges,
                "diagnostics": analysis.diagnostics
            })
        }
        _ => return Err(format!("unknown tool `{name}`")),
    };
    Ok(structured)
}

fn analyze(root: &Path) -> Result<wae_engine::Analysis, String> {
    Engine::default().analyze(AnalyzeRequest::new(root)).map_err(|error| format!("{error:?}"))
}

fn error(id: Value, code: i64, message: String) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_and_tool_list_follow_json_rpc_contract() {
        let root = Path::new(".");
        let initialized = handle_message(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
            root,
        )
        .unwrap();
        assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
        let listed = handle_message(
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
            root,
        )
        .unwrap();
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn check_tool_runs_the_real_engine() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/basic");
        let response = handle_message(
            json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "architecture_check", "arguments": {} }
            }),
            &root,
        )
        .unwrap();
        assert_eq!(response["result"]["structuredContent"]["sourceModules"], 1);
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn tool_failures_are_mcp_results_not_transport_errors() {
        let response = handle_message(
            json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": { "name": "architecture_explain", "arguments": {} }
            }),
            Path::new("."),
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response.get("error").is_none());
    }
}
