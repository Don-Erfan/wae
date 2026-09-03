use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use wae_core::domain::{ModuleId, ModuleKind};
use wae_engine::{Analysis, WorkspaceSession};

#[derive(Clone, Debug)]
pub struct ServerPolicy {
    allowed_roots: Vec<PathBuf>,
    allow_any_root: bool,
    max_request_bytes: usize,
}

pub struct McpServer {
    default_root: PathBuf,
    policy: ServerPolicy,
    sessions: Mutex<HashMap<PathBuf, Arc<WorkspaceSession>>>,
}

impl McpServer {
    pub fn new(default_root: impl Into<PathBuf>, policy: ServerPolicy) -> Self {
        Self { default_root: default_root.into(), policy, sessions: Mutex::new(HashMap::new()) }
    }

    pub fn handle_line(&self, line: &str) -> Option<Value> {
        if line.len() > self.policy.max_request_bytes {
            return Some(error(
                Value::Null,
                -32001,
                "request exceeds configured byte quota".into(),
            ));
        }
        match serde_json::from_str(line) {
            Ok(message) => self.handle_message(message),
            Err(parse_error) => {
                Some(error(Value::Null, -32700, format!("parse error: {parse_error}")))
            }
        }
    }

    pub fn handle_message(&self, message: Value) -> Option<Value> {
        handle_message_with_server(message, self)
    }

    fn analyze(&self, root: &Path, refresh: bool) -> Result<Arc<Analysis>, String> {
        let session = {
            let mut sessions =
                self.sessions.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                sessions
                    .entry(root.to_path_buf())
                    .or_insert_with(|| Arc::new(WorkspaceSession::new(root))),
            )
        };
        let force = refresh || session.snapshot().is_none();
        session
            .analyze_changes(&session.begin_analysis(), &BTreeMap::new(), force)
            .map_err(|error| format!("{error:?}"))
    }
}

impl ServerPolicy {
    pub fn confined(default_root: &Path) -> Self {
        Self {
            allowed_roots: vec![default_root.to_path_buf()],
            allow_any_root: false,
            max_request_bytes: 1024 * 1024,
        }
    }

    pub fn max_request_bytes(&self) -> usize {
        self.max_request_bytes
    }

    pub fn with_allowed_root(mut self, root: PathBuf) -> Self {
        self.allowed_roots.push(root);
        self
    }

    pub fn allow_any_root(mut self) -> Self {
        self.allow_any_root = true;
        self
    }

    pub fn with_max_request_bytes(mut self, bytes: usize) -> Self {
        self.max_request_bytes = bytes.max(1);
        self
    }
}

pub fn handle_line(line: &str, default_root: &Path, policy: &ServerPolicy) -> Option<Value> {
    McpServer::new(default_root, policy.clone()).handle_line(line)
}

pub fn handle_message(message: Value, default_root: &Path) -> Option<Value> {
    McpServer::new(default_root, ServerPolicy::confined(default_root)).handle_message(message)
}

pub fn handle_message_with_policy(
    message: Value,
    default_root: &Path,
    policy: &ServerPolicy,
) -> Option<Value> {
    McpServer::new(default_root, policy.clone()).handle_message(message)
}

fn handle_message_with_server(message: Value, server: &McpServer) -> Option<Value> {
    let id = message.get("id").cloned()?;
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error(id, -32600, "invalid JSON-RPC version".into()));
    }
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
            &server.default_root,
            &server.policy,
            server,
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
        },
        {
            "name": "dependency_policy",
            "description": "Report whether an existing dependency is allowed and return every policy diagnostic that governs it.",
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

fn call_tool(
    name: &str,
    arguments: Value,
    default_root: &Path,
    policy: &ServerPolicy,
    server: &McpServer,
) -> Value {
    match execute_tool(name, arguments, default_root, policy, server) {
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

fn execute_tool(
    name: &str,
    arguments: Value,
    default_root: &Path,
    policy: &ServerPolicy,
    server: &McpServer,
) -> Result<Value, String> {
    let requested = arguments
        .get("root")
        .and_then(Value::as_str)
        .map(|root| {
            let path = PathBuf::from(root);
            if path.is_absolute() { path } else { default_root.join(path) }
        })
        .unwrap_or_else(|| default_root.to_path_buf());
    let root = confined_root(&requested, policy)?;
    let structured = match name {
        "architecture_check" => {
            let analysis = server.analyze(&root, true)?;
            json!({
                "schemaVersion": analysis.schema_version,
                "sourceModules": analysis.project.modules.iter().filter(|module| module.kind == ModuleKind::Source).count(),
                "dependencies": analysis.project.dependencies.len(),
                "diagnostics": analysis.diagnostics,
                "timings": {
                    "discoveryMs": analysis.timings.discovery_ms,
                    "classificationMs": analysis.timings.classification_ms,
                    "parsingMs": analysis.timings.parsing_ms,
                    "resolutionMs": analysis.timings.resolution_ms,
                    "graphBuildMs": analysis.timings.graph_build_ms,
                    "ruleEvaluationMs": analysis.timings.rule_evaluation_ms,
                    "cacheMs": analysis.timings.cache_ms,
                    "reportingMs": analysis.timings.reporting_ms,
                    "orchestrationMs": analysis.timings.orchestration_ms,
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
            let analysis = server.analyze(&root, false)?;
            let path = analysis
                .graph
                .shortest_path(&ModuleId(from.into()), &ModuleId(to.into()))
                .map(|path| path.into_iter().map(|module| module.0).collect::<Vec<_>>());
            json!({ "from": from, "to": to, "path": path })
        }
        "architecture_model" => {
            let analysis = server.analyze(&root, false)?;
            let modules = analysis
                .project
                .modules
                .iter()
                .map(|module| {
                    let ownership = analysis.ownership.get(&module.id);
                    json!({
                        "id": module.id.0,
                        "package": module.package.0,
                        "kind": format!("{:?}", module.kind),
                        "layer": module.layer.as_ref().map(|layer| &layer.0),
                        "ownership": ownership,
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
        "dependency_policy" => {
            let from = arguments.get("from").and_then(Value::as_str).ok_or("from is required")?;
            let to = arguments.get("to").and_then(Value::as_str).ok_or("to is required")?;
            let analysis = server.analyze(&root, false)?;
            let edge_exists = analysis
                .project
                .dependencies
                .iter()
                .any(|edge| edge.from.0 == from && edge.to.0 == to);
            let diagnostics = analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.dependency_path.first().is_some_and(|module| module.0 == from)
                        && diagnostic.dependency_path.last().is_some_and(|module| module.0 == to)
                })
                .collect::<Vec<_>>();
            let allowed = edge_exists.then(|| {
                diagnostics.iter().all(|diagnostic| !analysis.failure_policy.is_failure(diagnostic))
            });
            json!({
                "from": from,
                "to": to,
                "edgeExists": edge_exists,
                "allowed": allowed,
                "diagnostics": diagnostics
            })
        }
        _ => return Err(format!("unknown tool `{name}`")),
    };
    Ok(structured)
}

fn confined_root(requested: &Path, policy: &ServerPolicy) -> Result<PathBuf, String> {
    let requested = requested.canonicalize().map_err(|error| {
        format!("cannot open requested root `{}`: {error}", requested.display())
    })?;
    if policy.allow_any_root {
        return Ok(requested);
    }
    let allowed = policy.allowed_roots.iter().filter_map(|root| root.canonicalize().ok());
    if allowed.into_iter().any(|root| requested.starts_with(root)) {
        Ok(requested)
    } else {
        Err(format!(
            "requested root `{}` is outside the MCP server allowed roots",
            requested.display()
        ))
    }
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
        assert_eq!(listed["result"]["tools"].as_array().unwrap().len(), 5);
    }

    #[test]
    fn malformed_oversized_and_wrong_version_requests_are_bounded_protocol_errors() {
        let root = std::env::current_dir().unwrap();
        let policy = ServerPolicy::confined(&root).with_max_request_bytes(32);
        assert_eq!(handle_line("{", &root, &policy).unwrap()["error"]["code"], -32700);
        assert_eq!(handle_line(&"x".repeat(33), &root, &policy).unwrap()["error"]["code"], -32001);
        let wrong = handle_message_with_policy(
            json!({"jsonrpc":"1.0","id":7,"method":"ping"}),
            &root,
            &policy,
        )
        .unwrap();
        assert_eq!(wrong["error"]["code"], -32600);
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
    fn persistent_server_reuses_the_last_checked_workspace_snapshot_for_queries() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/basic");
        let canonical = root.canonicalize().unwrap();
        let server = McpServer::new(&canonical, ServerPolicy::confined(&canonical));
        let check = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "architecture_check", "arguments": {} }
        });
        assert_eq!(server.handle_message(check).unwrap()["result"]["isError"], false);
        let model = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "architecture_model", "arguments": {} }
        });
        assert_eq!(server.handle_message(model).unwrap()["result"]["isError"], false);
        let session =
            Arc::clone(server.sessions.lock().unwrap().get(&canonical).expect("workspace session"));
        assert!(session.last_execution().reused_snapshot);
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

    #[test]
    fn requested_roots_are_confined_by_default() {
        let allowed = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/basic");
        let outside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/circular");
        let response = handle_message(
            json!({
                "jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {
                    "name": "architecture_check",
                    "arguments": { "root": outside }
                }
            }),
            &allowed,
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("outside the MCP server allowed roots")
        );
    }

    #[test]
    fn dependency_policy_returns_governing_diagnostics() {
        let root = std::env::temp_dir().join(format!("wae-mcp-policy-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "import './b';").unwrap();
        std::fs::write(root.join("src/b.ts"), "export const value = true;").unwrap();
        std::fs::write(
            root.join("wae.yaml"),
            "version: 1\nresolution:\n  mode: bundler\narchitecture:\n  forbidden_dependencies:\n    - from: 'src/a.ts'\n      to: 'src/b.ts'\n",
        )
        .unwrap();
        let response = handle_message(
            json!({
                "jsonrpc": "2.0", "id": 6, "method": "tools/call",
                "params": { "name": "dependency_policy", "arguments": {
                    "from": "src/a.ts", "to": "src/b.ts"
                }}
            }),
            &root,
        )
        .unwrap();
        let policy = &response["result"]["structuredContent"];
        assert_eq!(policy["edgeExists"], true);
        assert_eq!(policy["allowed"], false);
        assert!(
            policy["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| { diagnostic["rule_id"] == "ARCH-002" })
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
