use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Sender, unbounded};
use lsp_server::{Connection, Message, Notification, Request, Response};
use serde_json::{Value, json};
use url::Url;
use wae_core::domain::{Diagnostic, ModuleKind, Severity};
use wae_engine::{Analysis, AnalysisError, AnalysisTicket, WorkspaceSession};

fn main() {
    if let Err(error) = run() {
        eprintln!("wae-lsp: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialize_params) = connection.initialize_start().map_err(err)?;
    let root = workspace_root(&initialize_params)?;
    connection.initialize_finish(initialize_id, capabilities()).map_err(err)?;
    let mut state = ServerState::new(root);
    let (analysis_sender, analysis_receiver) = unbounded();
    state.schedule_analysis(analysis_sender.clone());

    loop {
        crossbeam_channel::select! {
            recv(connection.receiver) -> message => {
                let message = message.map_err(err)?;
                match message {
                    Message::Request(request) => {
                        if connection.handle_shutdown(&request).map_err(err)? {
                            break;
                        }
                        handle_request(&connection, &state, request)?;
                    }
                    Message::Notification(notification) => {
                        state.handle_notification(notification, analysis_sender.clone());
                    }
                    Message::Response(_) => {}
                }
            }
            recv(analysis_receiver) -> result => {
                state.publish_result(&connection, result.map_err(err)?)?;
            }
        }
    }
    drop(connection);
    io_threads.join().map_err(err)
}

struct ServerState {
    root: PathBuf,
    session: Arc<WorkspaceSession>,
    analysis: Option<Analysis>,
    published: HashSet<String>,
    documents: BTreeMap<String, String>,
}

struct BackgroundAnalysis {
    ticket: AnalysisTicket,
    result: Result<Analysis, AnalysisError>,
}

impl ServerState {
    fn new(root: PathBuf) -> Self {
        Self {
            session: Arc::new(WorkspaceSession::new(&root)),
            root,
            analysis: None,
            published: HashSet::new(),
            documents: BTreeMap::new(),
        }
    }

    fn handle_notification(
        &mut self,
        notification: Notification,
        sender: Sender<BackgroundAnalysis>,
    ) {
        let should_analyze = match notification.method.as_str() {
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                    notification.params.pointer("/textDocument/text").and_then(Value::as_str),
                ) {
                    if let Some(path) = uri_path(&self.root, uri) {
                        self.documents.insert(path, text.into());
                    }
                }
                true
            }
            "textDocument/didChange" => {
                if let (Some(uri), Some(text)) = (
                    notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                    notification
                        .params
                        .pointer("/contentChanges")
                        .and_then(Value::as_array)
                        .and_then(|changes| changes.last())
                        .and_then(|change| change.get("text"))
                        .and_then(Value::as_str),
                ) {
                    if let Some(path) = uri_path(&self.root, uri) {
                        self.documents.insert(path, text.into());
                    }
                }
                true
            }
            "textDocument/didClose" => {
                if let Some(uri) =
                    notification.params.pointer("/textDocument/uri").and_then(Value::as_str)
                {
                    if let Some(path) = uri_path(&self.root, uri) {
                        self.documents.remove(&path);
                    }
                }
                true
            }
            "textDocument/didSave"
            | "workspace/didChangeConfiguration"
            | "workspace/didChangeWatchedFiles" => true,
            _ => false,
        };
        if should_analyze {
            self.schedule_analysis(sender);
        }
    }

    fn schedule_analysis(&self, sender: Sender<BackgroundAnalysis>) {
        let ticket = self.session.begin_analysis();
        let worker_ticket = ticket.clone();
        let session = Arc::clone(&self.session);
        let overlays = self.documents.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(75));
            if !session.is_current(&worker_ticket) {
                return;
            }
            let result = session.analyze(&worker_ticket, &overlays);
            let _ = sender.send(BackgroundAnalysis { ticket, result });
        });
    }

    fn publish_result(
        &mut self,
        connection: &Connection,
        completed: BackgroundAnalysis,
    ) -> Result<(), String> {
        if !self.session.is_current(&completed.ticket) {
            return Ok(());
        }
        match completed.result {
            Ok(analysis) => {
                let mut by_file = std::collections::BTreeMap::<String, Vec<Value>>::new();
                for diagnostic in &analysis.diagnostics {
                    let Some(location) = &diagnostic.primary_location else { continue };
                    by_file
                        .entry(location.file.clone())
                        .or_default()
                        .push(lsp_diagnostic(&self.root, diagnostic));
                }
                let current = analysis
                    .project
                    .modules
                    .iter()
                    .filter(|module| module.kind == ModuleKind::Source)
                    .map(|module| module.id.0.clone())
                    .collect::<HashSet<_>>();
                for file in self.published.union(&current) {
                    let Some(uri) = file_uri(&self.root, file) else { continue };
                    send_notification(
                        connection,
                        "textDocument/publishDiagnostics",
                        json!({ "uri": uri, "diagnostics": by_file.remove(file).unwrap_or_default() }),
                    )?;
                }
                self.published = current;
                self.analysis = Some(analysis);
            }
            Err(AnalysisError::Cancelled) => {}
            Err(error) => send_notification(
                connection,
                "window/showMessage",
                json!({ "type": 1, "message": format!("WAE analysis failed: {error:?}") }),
            )?,
        }
        Ok(())
    }
}

fn handle_request(
    connection: &Connection,
    state: &ServerState,
    request: Request,
) -> Result<(), String> {
    let result = match request.method.as_str() {
        "textDocument/hover" => hover(state, &request.params),
        "textDocument/codeAction" => code_actions(&request.params),
        "workspace/executeCommand" => Value::Null,
        _ => {
            connection
                .sender
                .send(Message::Response(Response::new_err(
                    request.id,
                    -32601,
                    format!("unsupported request `{}`", request.method),
                )))
                .map_err(err)?;
            return Ok(());
        }
    };
    connection.sender.send(Message::Response(Response::new_ok(request.id, result))).map_err(err)
}

fn hover(state: &ServerState, params: &Value) -> Value {
    let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
        return Value::Null;
    };
    let Some(path) = uri_path(&state.root, uri) else { return Value::Null };
    let Some(module) = state
        .analysis
        .as_ref()
        .and_then(|analysis| analysis.project.modules.iter().find(|module| module.id.0 == path))
    else {
        return Value::Null;
    };
    json!({
        "contents": {
            "kind": "markdown",
            "value": format!(
                "**WAE architecture**\n\n- Package: `{}`\n- Layer: `{}`\n- Runtime: `{:?}`\n- Framework: `{}`",
                module.package.0,
                module.layer.as_ref().map_or("unassigned", |layer| layer.0.as_str()),
                module.runtime,
                module.framework_metadata.adapter_id.as_deref().unwrap_or("none")
            )
        }
    })
}

fn code_actions(params: &Value) -> Value {
    let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) else {
        return Value::Array(Vec::new());
    };
    let actions = params
        .pointer("/context/diagnostics")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|diagnostic| {
            let data = diagnostic.get("data")?;
            let suggestion = data.get("suggestion")?.as_str()?;
            let rule = data.get("ruleId")?.as_str()?;
            let line = diagnostic.pointer("/range/start/line").and_then(Value::as_u64)?;
            Some(json!({
                "title": format!("Suppress {rule} with a documented reason"),
                "kind": "quickfix",
                "diagnostics": [diagnostic],
                "isPreferred": false,
                "edit": { "changes": {
                    (uri): [{
                        "range": {
                            "start": { "line": line, "character": 0 },
                            "end": { "line": line, "character": 0 }
                        },
                        "newText": format!("// wae-ignore {rule} -- explain why this exception is safe\n")
                    }]
                }},
                "data": { "suggestion": suggestion }
            }))
        })
        .collect::<Vec<_>>();
    Value::Array(actions)
}

fn lsp_diagnostic(root: &Path, diagnostic: &Diagnostic) -> Value {
    let location = diagnostic.primary_location.as_ref();
    let line = location.map_or(0, |location| location.line.saturating_sub(1));
    let column = location.map_or(0, |location| location.column.saturating_sub(1));
    json!({
        "range": {
            "start": { "line": line, "character": column },
            "end": { "line": line, "character": column.saturating_add(1) }
        },
        "severity": match diagnostic.severity { Severity::Error => 1, Severity::Warning => 2, Severity::Info => 3 },
        "code": diagnostic.rule_id.0,
        "source": "wae",
        "message": diagnostic.message,
        "relatedInformation": diagnostic.secondary_locations.iter().filter_map(|location| {
            Some(json!({
                "location": {
                    "uri": file_uri(root, &location.file)?,
                    "range": {
                        "start": { "line": location.line.saturating_sub(1), "character": location.column.saturating_sub(1) },
                        "end": { "line": location.line.saturating_sub(1), "character": location.column }
                    }
                },
                "message": "Related architecture location"
            }))
        }).collect::<Vec<_>>(),
        "data": { "ruleId": diagnostic.rule_id.0, "suggestion": diagnostic.suggestion }
    })
}

fn capabilities() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
            "hoverProvider": true,
            "codeActionProvider": true,
            "executeCommandProvider": { "commands": ["wae.showSuggestion", "wae.reload"] },
            "workspace": { "workspaceFolders": { "supported": true, "changeNotifications": true } }
        },
        "serverInfo": { "name": "wae-lsp", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn workspace_root(params: &Value) -> Result<PathBuf, String> {
    let uri = params
        .pointer("/workspaceFolders/0/uri")
        .or_else(|| params.get("rootUri"))
        .and_then(Value::as_str)
        .ok_or("LSP initialization requires a workspace folder")?;
    Url::parse(uri).map_err(err)?.to_file_path().map_err(|_| "invalid workspace file URI".into())
}

fn file_uri(root: &Path, file: &str) -> Option<String> {
    Url::from_file_path(root.join(file)).ok().map(|url| url.to_string())
}

fn uri_path(root: &Path, uri: &str) -> Option<String> {
    let path = Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(path.strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/"))
}

fn send_notification(connection: &Connection, method: &str, params: Value) -> Result<(), String> {
    connection
        .sender
        .send(Message::Notification(Notification::new(method.into(), params)))
        .map_err(err)
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wae_core::domain::{RuleId, SourceLocation};

    #[test]
    fn diagnostics_are_zero_based_and_keep_quick_fix_data() {
        let diagnostic = Diagnostic {
            rule_id: RuleId("ARCH-003".into()),
            severity: Severity::Warning,
            message: "Layer violation".into(),
            primary_location: Some(SourceLocation { file: "src/a.ts".into(), line: 3, column: 5 }),
            suggestion: Some("Move the dependency".into()),
            ..Diagnostic::default()
        };
        let value = lsp_diagnostic(Path::new("/project"), &diagnostic);
        assert_eq!(value["range"]["start"]["line"], 2);
        assert_eq!(value["range"]["start"]["character"], 4);
        assert_eq!(value["data"]["ruleId"], "ARCH-003");
        let actions = code_actions(&json!({
            "textDocument": { "uri": "file:///project/src/a.ts" },
            "context": { "diagnostics": [value] }
        }));
        assert_eq!(actions.as_array().unwrap().len(), 1);
        assert_eq!(
            actions[0]["edit"]["changes"]["file:///project/src/a.ts"][0]["range"]["start"]["line"],
            2
        );
        assert!(actions[0]["command"].is_null());
    }

    #[test]
    fn file_uris_round_trip_to_project_relative_module_ids() {
        let root = std::env::temp_dir().join("wae-lsp-test");
        let uri = file_uri(&root, "src/a file.ts").unwrap();
        assert_eq!(uri_path(&root, &uri).unwrap(), "src/a file.ts");
    }
}
