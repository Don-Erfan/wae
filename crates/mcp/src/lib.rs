use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wae_core::domain::{ModuleId, ModuleKind};
use wae_engine::{Analysis, WorkspaceSession};

#[derive(Clone, Debug)]
pub struct ServerPolicy {
    allowed_roots: Vec<PathBuf>,
    allow_any_root: bool,
    max_request_bytes: usize,
    max_sessions: usize,
    session_ttl: Duration,
}

pub struct McpServer {
    default_root: PathBuf,
    policy: ServerPolicy,
    sessions: Mutex<HashMap<PathBuf, SessionEntry>>,
    evicted_sessions: AtomicU64,
    access_clock: AtomicU64,
}

struct SessionEntry {
    session: Arc<WorkspaceSession>,
    last_used: Instant,
    access_order: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionMetrics {
    pub active: usize,
    pub capacity: usize,
    pub evicted: u64,
}

impl McpServer {
    pub fn new(default_root: impl Into<PathBuf>, policy: ServerPolicy) -> Self {
        Self {
            default_root: default_root.into(),
            policy,
            sessions: Mutex::new(HashMap::new()),
            evicted_sessions: AtomicU64::new(0),
            access_clock: AtomicU64::new(0),
        }
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
            let now = Instant::now();
            let access_order = self.access_clock.fetch_add(1, Ordering::Relaxed);
            let before = sessions.len();
            // Expiry is evaluated before touching the requested entry. Exempting the current root
            // made a zero-TTL session live forever whenever the same workspace was queried again.
            sessions
                .retain(|_, entry| now.duration_since(entry.last_used) < self.policy.session_ttl);
            self.evicted_sessions
                .fetch_add(before.saturating_sub(sessions.len()) as u64, Ordering::Relaxed);
            if let Some(entry) = sessions.get_mut(root) {
                entry.last_used = now;
                entry.access_order = access_order;
                Arc::clone(&entry.session)
            } else {
                if sessions.len() >= self.policy.max_sessions {
                    let oldest = sessions
                        .iter()
                        .min_by(|(left_path, left), (right_path, right)| {
                            left.access_order
                                .cmp(&right.access_order)
                                .then_with(|| left_path.cmp(right_path))
                        })
                        .map(|(path, _)| path.clone());
                    if let Some(oldest) = oldest {
                        sessions.remove(&oldest);
                        self.evicted_sessions.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let session = Arc::new(WorkspaceSession::new(root));
                sessions.insert(
                    root.to_path_buf(),
                    SessionEntry { session: Arc::clone(&session), last_used: now, access_order },
                );
                session
            }
        };
        let force = refresh || session.snapshot().is_none();
        session
            .analyze_changes(&session.begin_analysis(), &BTreeMap::new(), force)
            .map_err(|error| format!("{error:?}"))
    }

    pub fn session_metrics(&self) -> SessionMetrics {
        let active = self.sessions.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).len();
        SessionMetrics {
            active,
            capacity: self.policy.max_sessions,
            evicted: self.evicted_sessions.load(Ordering::Relaxed),
        }
    }
}

impl ServerPolicy {
    pub fn confined(default_root: &Path) -> Self {
        Self {
            allowed_roots: vec![default_root.to_path_buf()],
            allow_any_root: false,
            max_request_bytes: 1024 * 1024,
            max_sessions: 16,
            session_ttl: Duration::from_secs(30 * 60),
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

    pub fn with_max_sessions(mut self, sessions: usize) -> Self {
        self.max_sessions = sessions.max(1);
        self
    }

    pub fn with_session_ttl(mut self, ttl: Duration) -> Self {
        self.session_ttl = ttl;
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
            "description": "Explain a WAE rule by stable rule id: rationale, bad and good examples, fix, configuration and known false positives.",
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
            let session_metrics = server.session_metrics();
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
                },
                "incremental": {
                    "restoredModules": analysis.incremental.restored_modules,
                    "analyzedModules": analysis.incremental.analyzed_modules,
                    "affectedModules": analysis.incremental.affected_modules,
                    "inspectedEdges": analysis.incremental.inspected_edges,
                    "retainedDiagnostics": analysis.incremental.retained_diagnostics,
                    "addedDiagnostics": analysis.incremental.added_diagnostics,
                    "removedDiagnostics": analysis.incremental.removed_diagnostics,
                    "restoredRules": analysis.incremental.restored_rules,
                    "evaluatedRules": analysis.incremental.evaluated_rules,
                    "fullRuleEvaluations": analysis.incremental.full_rule_evaluations,
                    "scopedRuleEvaluations": analysis.incremental.scoped_rule_evaluations,
                    "ruleSnapshotReused": analysis.incremental.rule_snapshot_reused
                },
                "sessionMetrics": {
                    "active": session_metrics.active,
                    "capacity": session_metrics.capacity,
                    "evicted": session_metrics.evicted
                }
            })
        }
        "architecture_explain" => {
            let rule =
                arguments.get("ruleId").and_then(Value::as_str).ok_or("ruleId is required")?;
            let descriptor = wae_core::rule_registry::descriptor(rule)
                .ok_or_else(|| format!("unknown rule `{rule}`"))?;
            let docs = descriptor.documentation();
            json!({
                "id": descriptor.id,
                "title": descriptor.title,
                "description": descriptor.description,
                "category": descriptor.category,
                "configurable": descriptor.configurable,
                "defaultBehavior": wae_core::rule_docs::default_behavior(descriptor),
                "rationale": docs.map(|docs| docs.rationale),
                "badExample": docs.map(|docs| docs.bad_example),
                "goodExample": docs.map(|docs| docs.good_example),
                "fix": docs.map(|docs| docs.fix),
                "configuration": docs.map(|docs| docs.configuration),
                "falsePositives": docs.map(|docs| docs.false_positives),
                "helpUri": wae_core::rule_docs::help_uri(descriptor.id)
            })
        }
        "dependency_path" => {
            let from = arguments.get("from").and_then(Value::as_str).ok_or("from is required")?;
            let to = arguments.get("to").and_then(Value::as_str).ok_or("to is required")?;
            let analysis = server.analyze(&root, true)?;
            let path = analysis
                .graph
                .shortest_path(&ModuleId(from.into()), &ModuleId(to.into()))
                .map(|path| path.into_iter().map(|module| module.0).collect::<Vec<_>>());
            json!({ "from": from, "to": to, "path": path })
        }
        "architecture_model" => {
            let analysis = server.analyze(&root, true)?;
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
            let analysis = server.analyze(&root, true)?;
            let edge_exists = analysis
                .project
                .dependencies
                .iter()
                .any(|edge| edge.from.0 == from && edge.to.0 == to);
            let diagnostics = analysis
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic_governs_edge(diagnostic, &analysis, from, to))
                .collect::<Vec<_>>();
            let allowed = edge_exists.then(|| {
                diagnostics.iter().all(|diagnostic| !analysis.failure_policy.is_failure(diagnostic))
            });
            json!({
                "from": from,
                "to": to,
                "edgeExists": edge_exists,
                "allowed": allowed,
                "decision": match allowed {
                    Some(true) => "allowed",
                    Some(false) => "denied",
                    None => "indeterminate",
                },
                "suppressedDiagnostics": diagnostics.iter().filter(|diagnostic| diagnostic.suppressed).count(),
                "diagnostics": diagnostics
            })
        }
        _ => return Err(format!("unknown tool `{name}`")),
    };
    Ok(structured)
}

fn diagnostic_governs_edge(
    diagnostic: &wae_core::domain::Diagnostic,
    analysis: &Analysis,
    from: &str,
    to: &str,
) -> bool {
    if diagnostic.dependency_path.windows(2).any(|edge| edge[0].0 == from && edge[1].0 == to) {
        return true;
    }
    let packages = analysis
        .project
        .modules
        .iter()
        .map(|module| (module.id.0.as_str(), module.package.0.as_str()))
        .collect::<HashMap<_, _>>();
    let (Some(from_package), Some(to_package)) = (packages.get(from), packages.get(to)) else {
        return false;
    };
    diagnostic.dependency_path.windows(2).any(|edge| {
        edge[0].0.strip_prefix("package:") == Some(*from_package)
            && edge[1].0.strip_prefix("package:") == Some(*to_package)
    })
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
        assert_eq!(response["result"]["structuredContent"]["sessionMetrics"]["active"], 1);
        assert_eq!(response["result"]["structuredContent"]["sessionMetrics"]["capacity"], 16);
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn persistent_server_refreshes_queries_after_files_change_on_disk() {
        let root = std::env::temp_dir().join(format!("wae-mcp-fresh-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "import './b';").unwrap();
        std::fs::write(root.join("src/b.ts"), "import './a';").unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
        let canonical = root.canonicalize().unwrap();
        let server = McpServer::new(&canonical, ServerPolicy::confined(&canonical));
        let model = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "architecture_model", "arguments": {} }
        });
        let before = server.handle_message(model.clone()).unwrap();
        assert!(
            before["result"]["structuredContent"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["rule_id"] == "ARCH-001")
        );
        std::fs::write(root.join("src/b.ts"), "export const fixed = true;").unwrap();
        let after = server.handle_message(model).unwrap();
        assert!(
            !after["result"]["structuredContent"]["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["rule_id"] == "ARCH-001")
        );
        let session = Arc::clone(
            &server.sessions.lock().unwrap().get(&canonical).expect("workspace session").session,
        );
        assert!(!session.last_execution().reused_snapshot);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn architecture_check_matches_the_shared_synthetic_app_golden() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/synthetic-app")
            .canonicalize()
            .unwrap();
        let server = McpServer::new(&fixture, ServerPolicy::confined(&fixture));
        let response = server
            .handle_message(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "architecture_check", "arguments": {} }
            }))
            .unwrap();
        let actual = response["result"]["structuredContent"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic["rule_id"].clone(),
                    diagnostic["fingerprint"].clone(),
                    diagnostic["primary_location"]["file"].clone(),
                    diagnostic["primary_location"]["line"].clone(),
                    diagnostic["dependency_path"].clone(),
                )
            })
            .collect::<Vec<_>>();
        let golden: Value = serde_json::from_str(
            &std::fs::read_to_string(fixture.join("expected-diagnostics.json")).unwrap(),
        )
        .unwrap();
        let expected = golden["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic["ruleId"].clone(),
                    diagnostic["fingerprint"].clone(),
                    diagnostic["file"].clone(),
                    diagnostic["line"].clone(),
                    diagnostic["dependencyPath"].clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn session_store_evicts_the_least_recently_used_workspace_at_capacity() {
        let root = std::env::temp_dir().join(format!("wae-mcp-lru-{}", std::process::id()));
        for name in ["a", "b", "c"] {
            let workspace = root.join(name);
            std::fs::create_dir_all(workspace.join("src")).unwrap();
            std::fs::write(workspace.join("src/index.ts"), "export {};").unwrap();
        }
        let policy = ServerPolicy::confined(&root).allow_any_root().with_max_sessions(2);
        let server = McpServer::new(&root, policy);
        server.analyze(&root.join("a").canonicalize().unwrap(), true).unwrap();
        server.analyze(&root.join("b").canonicalize().unwrap(), true).unwrap();
        // Touch A, making B the least recently used entry.
        server.analyze(&root.join("a").canonicalize().unwrap(), false).unwrap();
        server.analyze(&root.join("c").canonicalize().unwrap(), true).unwrap();
        let sessions = server.sessions.lock().unwrap();
        assert!(sessions.contains_key(&root.join("a").canonicalize().unwrap()));
        assert!(!sessions.contains_key(&root.join("b").canonicalize().unwrap()));
        assert!(sessions.contains_key(&root.join("c").canonicalize().unwrap()));
        drop(sessions);
        assert_eq!(server.session_metrics(), SessionMetrics { active: 2, capacity: 2, evicted: 1 });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_store_expires_idle_workspaces_by_ttl() {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let first = fixtures.join("basic").canonicalize().unwrap();
        let second = fixtures.join("circular").canonicalize().unwrap();
        let policy =
            ServerPolicy::confined(&fixtures).allow_any_root().with_session_ttl(Duration::ZERO);
        let server = McpServer::new(&fixtures, policy);
        server.analyze(&first, true).unwrap();
        server.analyze(&second, true).unwrap();
        let sessions = server.sessions.lock().unwrap();
        assert!(!sessions.contains_key(&first));
        assert!(sessions.contains_key(&second));
        drop(sessions);
        assert_eq!(server.session_metrics().evicted, 1);
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

    #[test]
    fn dependency_policy_attributes_cycle_diagnostics_to_each_cycle_edge() {
        let root = std::env::temp_dir().join(format!("wae-mcp-cycle-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "import './b';").unwrap();
        std::fs::write(root.join("src/b.ts"), "import './a';").unwrap();
        std::fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
        let response = handle_message(
            json!({
                "jsonrpc": "2.0", "id": 7, "method": "tools/call",
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
        assert_eq!(policy["decision"], "denied");
        assert!(
            policy["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| { diagnostic["rule_id"] == "ARCH-001" })
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
