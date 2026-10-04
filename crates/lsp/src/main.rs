use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Sender, bounded, unbounded};
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
    let roots = workspace_roots(&initialize_params)?;
    connection.initialize_finish(initialize_id, capabilities()).map_err(err)?;
    let (analysis_sender, analysis_receiver) = unbounded();
    let mut state = ServerState::new(roots, analysis_sender);
    state.schedule_all(true);

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
                        state.handle_notification(&connection, notification)?;
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
    workspaces: BTreeMap<PathBuf, WorkspaceState>,
    results: Sender<BackgroundAnalysis>,
}

struct WorkspaceState {
    session: Arc<WorkspaceSession>,
    analysis: Option<Arc<Analysis>>,
    published: HashSet<String>,
    documents: BTreeMap<String, String>,
    scheduler: AnalysisScheduler,
}

struct BackgroundAnalysis {
    root: PathBuf,
    ticket: AnalysisTicket,
    result: Result<Arc<Analysis>, AnalysisError>,
}

struct PendingAnalysis {
    ticket: AnalysisTicket,
    overlays: BTreeMap<String, String>,
    force: bool,
}

/// A single bounded worker coalesces typing bursts into the latest immutable overlay snapshot.
/// New generations cancel in-flight engine work; no notification creates a new sleeping thread.
struct AnalysisScheduler {
    session: Arc<WorkspaceSession>,
    pending: Arc<Mutex<Option<PendingAnalysis>>>,
    wake: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl AnalysisScheduler {
    fn new(
        root: PathBuf,
        session: Arc<WorkspaceSession>,
        results: Sender<BackgroundAnalysis>,
    ) -> Self {
        let pending = Arc::new(Mutex::new(None::<PendingAnalysis>));
        let worker_pending = Arc::clone(&pending);
        let worker_session = Arc::clone(&session);
        let worker_root = root.clone();
        let (wake, notifications) = bounded::<()>(1);
        let worker = std::thread::spawn(move || {
            while notifications.recv().is_ok() {
                while notifications.recv_timeout(Duration::from_millis(75)).is_ok() {}
                let job =
                    worker_pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
                let Some(job) = job else { continue };
                let result = worker_session.analyze_changes(&job.ticket, &job.overlays, job.force);
                let _ = results.send(BackgroundAnalysis {
                    root: worker_root.clone(),
                    ticket: job.ticket,
                    result,
                });
            }
        });
        Self { session, pending, wake: Some(wake), worker: Some(worker) }
    }

    fn schedule(&self, ticket: AnalysisTicket, overlays: BTreeMap<String, String>, force: bool) {
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let force = force || pending.as_ref().is_some_and(|job| job.force);
        *pending = Some(PendingAnalysis { ticket, overlays, force });
        drop(pending);
        if let Some(wake) = &self.wake {
            let _ = wake.try_send(());
        }
    }
}

impl Drop for AnalysisScheduler {
    fn drop(&mut self) {
        self.session.cancel_active();
        self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        self.wake.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl ServerState {
    fn new(roots: Vec<PathBuf>, results: Sender<BackgroundAnalysis>) -> Self {
        let mut state = Self { workspaces: BTreeMap::new(), results };
        for root in roots {
            state.add_workspace(root);
        }
        state
    }

    fn add_workspace(&mut self, root: PathBuf) {
        if self.workspaces.contains_key(&root) {
            return;
        }
        let session = Arc::new(WorkspaceSession::new(&root));
        let scheduler =
            AnalysisScheduler::new(root.clone(), Arc::clone(&session), self.results.clone());
        self.workspaces.insert(
            root,
            WorkspaceState {
                session,
                analysis: None,
                published: HashSet::new(),
                documents: BTreeMap::new(),
                scheduler,
            },
        );
    }

    fn handle_notification(
        &mut self,
        connection: &Connection,
        notification: Notification,
    ) -> Result<(), String> {
        if notification.method == "workspace/didChangeWorkspaceFolders" {
            for removed in notification
                .params
                .pointer("/event/removed")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(root) = removed.get("uri").and_then(Value::as_str).and_then(uri_file_path)
                else {
                    continue;
                };
                if let Some(workspace) = self.workspaces.remove(&root) {
                    for file in workspace.published {
                        if let Some(uri) = file_uri(&root, &file) {
                            send_notification(
                                connection,
                                "textDocument/publishDiagnostics",
                                json!({ "uri": uri, "diagnostics": [] }),
                            )?;
                        }
                    }
                }
            }
            for added in notification
                .params
                .pointer("/event/added")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(root) = added.get("uri").and_then(Value::as_str).and_then(uri_file_path)
                {
                    self.add_workspace(root.clone());
                    self.schedule_root(&root, true);
                }
            }
            return Ok(());
        }

        let document_uri = notification.params.pointer("/textDocument/uri").and_then(Value::as_str);
        let root = document_uri.and_then(|uri| self.root_for_uri(uri));
        let analysis_force = match notification.method.as_str() {
            "textDocument/didOpen" => {
                if let (Some(uri), Some(text)) = (
                    notification.params.pointer("/textDocument/uri").and_then(Value::as_str),
                    notification.params.pointer("/textDocument/text").and_then(Value::as_str),
                ) {
                    if let Some(root) = &root {
                        if let Some(path) = uri_path(root, uri) {
                            self.workspaces
                                .get_mut(root)
                                .unwrap()
                                .documents
                                .insert(path, text.into());
                        }
                    }
                }
                Some(false)
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
                    if let Some(root) = &root {
                        if let Some(path) = uri_path(root, uri) {
                            self.workspaces
                                .get_mut(root)
                                .unwrap()
                                .documents
                                .insert(path, text.into());
                        }
                    }
                }
                Some(false)
            }
            "textDocument/didClose" => {
                if let Some(uri) =
                    notification.params.pointer("/textDocument/uri").and_then(Value::as_str)
                {
                    if let Some(root) = &root {
                        if let Some(path) = uri_path(root, uri) {
                            self.workspaces.get_mut(root).unwrap().documents.remove(&path);
                        }
                    }
                }
                Some(false)
            }
            "textDocument/didSave" => Some(true),
            "workspace/didChangeConfiguration" | "workspace/didChangeWatchedFiles" => {
                self.schedule_all(true);
                None
            }
            _ => None,
        };
        if let Some(force) = analysis_force {
            if let Some(root) = root {
                self.schedule_root(&root, force);
            }
        }
        Ok(())
    }

    fn root_for_uri(&self, uri: &str) -> Option<PathBuf> {
        let path = uri_file_path(uri)?;
        self.workspaces
            .keys()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
    }

    fn schedule_root(&self, root: &Path, force: bool) {
        let Some(workspace) = self.workspaces.get(root) else { return };
        let ticket = workspace.session.begin_analysis();
        workspace.scheduler.schedule(ticket, workspace.documents.clone(), force);
    }

    fn schedule_all(&self, force: bool) {
        for root in self.workspaces.keys() {
            self.schedule_root(root, force);
        }
    }

    fn publish_result(
        &mut self,
        connection: &Connection,
        completed: BackgroundAnalysis,
    ) -> Result<(), String> {
        let Some(workspace) = self.workspaces.get_mut(&completed.root) else { return Ok(()) };
        if !workspace.session.is_current(&completed.ticket) {
            return Ok(());
        }
        match completed.result {
            Ok(analysis) => {
                let mut by_file = std::collections::BTreeMap::<String, Vec<Value>>::new();
                for diagnostic in
                    analysis.diagnostics.iter().filter(|diagnostic| !diagnostic.suppressed)
                {
                    let Some(location) = &diagnostic.primary_location else { continue };
                    let source = workspace.documents.get(&location.file).cloned().or_else(|| {
                        std::fs::read_to_string(completed.root.join(&location.file)).ok()
                    });
                    by_file.entry(location.file.clone()).or_default().push(lsp_diagnostic(
                        &completed.root,
                        diagnostic,
                        source.as_deref(),
                    ));
                }
                let current = analysis
                    .project
                    .modules
                    .iter()
                    .filter(|module| module.kind == ModuleKind::Source)
                    .map(|module| module.id.0.clone())
                    .chain(by_file.keys().cloned())
                    .collect::<HashSet<_>>();
                for file in workspace.published.union(&current) {
                    let Some(uri) = file_uri(&completed.root, file) else { continue };
                    send_notification(
                        connection,
                        "textDocument/publishDiagnostics",
                        json!({ "uri": uri, "diagnostics": by_file.remove(file).unwrap_or_default() }),
                    )?;
                }
                workspace.published = current;
                workspace.analysis = Some(analysis);
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
        "workspace/executeCommand" => match execute_command(connection, &request.params) {
            Ok(value) => value,
            Err(message) => {
                connection
                    .sender
                    .send(Message::Response(Response::new_err(request.id, -32602, message)))
                    .map_err(err)?;
                return Ok(());
            }
        },
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
    let Some(root) = state.root_for_uri(uri) else { return Value::Null };
    let Some(path) = uri_path(&root, uri) else { return Value::Null };
    let Some(module) = state
        .workspaces
        .get(&root)
        .and_then(|workspace| workspace.analysis.as_ref())
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
            let rule = data.get("ruleId")?.as_str()?;
            let line = diagnostic.pointer("/range/start/line").and_then(Value::as_u64)?;
            let mut changes = serde_json::Map::new();
            changes.insert(
                uri.into(),
                json!([{
                    "range": {
                        "start": { "line": line, "character": 0 },
                        "end": { "line": line, "character": 0 }
                    },
                    // Deliberately leave the reason blank. The engine refuses to suppress until
                    // the maintainer documents a concrete reason.
                    "newText": format!("// wae-ignore {rule} -- \n")
                }]),
            );
            let mut actions = Vec::new();
            if let Some(suggestion) = data.get("suggestion").and_then(Value::as_str) {
                actions.push(json!({
                    "title": format!("Review suggested fix for {rule}"),
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": true,
                    "command": {
                        "title": "Show WAE architecture suggestion",
                        "command": "wae.showSuggestion",
                        "arguments": [{ "uri": uri, "line": line, "ruleId": rule, "suggestion": suggestion }]
                    }
                }));
            }
            actions.push(json!({
                    "title": format!("Suppress {rule} after documenting a reason"),
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": false,
                    "command": {
                        "title": "Add documented WAE suppression",
                        "command": "wae.suppressWithReason",
                        "arguments": [{ "uri": uri, "line": line, "ruleId": rule }]
                    }
                }));
            actions.push(json!({
                    "title": format!("Insert {rule} suppression template (complete the reason)"),
                    "kind": "quickfix",
                    "diagnostics": [diagnostic],
                    "isPreferred": false,
                    "edit": {
                        "changes": changes
                    }
                }));
            Some(actions)
        })
        .flatten()
        .collect::<Vec<_>>();
    Value::Array(actions)
}

fn execute_command(connection: &Connection, params: &Value) -> Result<Value, String> {
    match params.get("command").and_then(Value::as_str) {
        Some("wae.showSuggestion") => {
            let suggestion = params
                .pointer("/arguments/0/suggestion")
                .and_then(Value::as_str)
                .unwrap_or("No WAE suggestion was provided.");
            send_notification(
                connection,
                "window/showMessage",
                json!({ "type": 3, "message": suggestion }),
            )?;
        }
        Some("wae.suppressWithReason") => {
            // VS Code supplies a reason through its richer prompt. Generic LSP clients (including
            // JetBrains) receive a standard WorkspaceEdit action next to this command instead.
            send_notification(
                connection,
                "window/showMessage",
                json!({
                    "type": 2,
                    "message": "Use the WAE suppression-template quick fix, then enter a concrete reason after `--`."
                }),
            )?;
        }
        Some(command) => {
            return Err(format!("unsupported WAE command `{command}`"));
        }
        None => return Err("workspace/executeCommand requires `command`".into()),
    }
    Ok(Value::Null)
}

fn lsp_diagnostic(root: &Path, diagnostic: &Diagnostic, source: Option<&str>) -> Value {
    let location = diagnostic.primary_location.as_ref();
    let line = location.map_or(0, |location| location.line.saturating_sub(1));
    let column =
        location.map_or(0, |location| utf16_column(source, location.line, location.column));
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
            let related_source = std::fs::read_to_string(root.join(&location.file)).ok();
            let related_column = utf16_column(
                related_source.as_deref(),
                location.line,
                location.column,
            );
            Some(json!({
                "location": {
                    "uri": file_uri(root, &location.file)?,
                    "range": {
                        "start": { "line": location.line.saturating_sub(1), "character": related_column },
                        "end": { "line": location.line.saturating_sub(1), "character": related_column.saturating_add(1) }
                    }
                },
                "message": "Related architecture location"
            }))
        }).collect::<Vec<_>>(),
        "data": { "ruleId": diagnostic.rule_id.0, "suggestion": diagnostic.suggestion }
    })
}

fn utf16_column(source: Option<&str>, one_based_line: usize, one_based_column: usize) -> usize {
    let code_point_column = one_based_column.saturating_sub(1);
    let Some(line) = source.and_then(|source| source.lines().nth(one_based_line.saturating_sub(1)))
    else {
        return code_point_column;
    };
    line.chars().take(code_point_column).map(char::len_utf16).sum()
}

fn capabilities() -> Value {
    json!({
        "capabilities": {
            "positionEncoding": "utf-16",
            "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": false } },
            "hoverProvider": true,
            "codeActionProvider": true,
            "executeCommandProvider": {
                "commands": ["wae.showSuggestion", "wae.suppressWithReason"]
            },
            "workspace": { "workspaceFolders": { "supported": true, "changeNotifications": true } }
        },
        "serverInfo": { "name": "wae-lsp", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn workspace_roots(params: &Value) -> Result<Vec<PathBuf>, String> {
    let mut roots = params
        .get("workspaceFolders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|folder| folder.get("uri").and_then(Value::as_str))
        .map(|uri| uri_file_path(uri).ok_or_else(|| format!("invalid workspace file URI `{uri}`")))
        .collect::<Result<Vec<_>, _>>()?;
    if roots.is_empty() {
        let uri = params
            .get("rootUri")
            .and_then(Value::as_str)
            .ok_or("LSP initialization requires a workspace folder")?;
        roots.push(uri_file_path(uri).ok_or("invalid workspace file URI")?);
    }
    roots.sort();
    roots.dedup();
    Ok(roots)
}

fn uri_file_path(uri: &str) -> Option<PathBuf> {
    let path = Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(path.canonicalize().unwrap_or(path))
}

fn file_uri(root: &Path, file: &str) -> Option<String> {
    Url::from_file_path(root.join(file)).ok().map(|url| url.to_string())
}

fn uri_path(root: &Path, uri: &str) -> Option<String> {
    let path = uri_file_path(uri)?;
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

    #[test]
    fn pending_force_is_monotonic_while_latest_ticket_and_overlays_win() {
        let session = Arc::new(WorkspaceSession::new(std::env::temp_dir()));
        let pending = Arc::new(Mutex::new(Some(PendingAnalysis {
            ticket: session.begin_analysis(),
            overlays: BTreeMap::from([("old.ts".into(), "old".into())]),
            force: true,
        })));
        let (wake, _notifications) = bounded(1);
        let scheduler = AnalysisScheduler {
            session: Arc::clone(&session),
            pending: Arc::clone(&pending),
            wake: Some(wake),
            worker: None,
        };
        let latest = session.begin_analysis();
        let latest_generation = latest.generation();
        scheduler.schedule(
            latest.clone(),
            BTreeMap::from([("latest.ts".into(), "latest".into())]),
            false,
        );
        let merged = pending.lock().unwrap();
        let merged = merged.as_ref().unwrap();
        assert!(merged.force);
        assert_eq!(merged.ticket.generation(), latest_generation);
        assert_eq!(merged.overlays.get("latest.ts").map(String::as_str), Some("latest"));
        assert!(!merged.overlays.contains_key("old.ts"));
    }
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
        let value = lsp_diagnostic(Path::new("/project"), &diagnostic, None);
        assert_eq!(value["range"]["start"]["line"], 2);
        assert_eq!(value["range"]["start"]["character"], 4);
        assert_eq!(value["data"]["ruleId"], "ARCH-003");
        let actions = code_actions(&json!({
            "textDocument": { "uri": "file:///project/src/a.ts" },
            "context": { "diagnostics": [value] }
        }));
        assert_eq!(actions.as_array().unwrap().len(), 3);
        assert_eq!(actions[0]["command"]["command"], "wae.showSuggestion");
        assert!(actions[0]["edit"].is_null());
        assert_eq!(actions[1]["command"]["command"], "wae.suppressWithReason");
        assert!(actions[1]["edit"].is_null());
        assert_eq!(
            actions[2]["edit"]["changes"]["file:///project/src/a.ts"][0]["newText"],
            "// wae-ignore ARCH-003 -- \n"
        );
        assert!(actions[2]["command"].is_null());
    }

    #[test]
    fn file_uris_round_trip_to_project_relative_module_ids() {
        let root = std::env::temp_dir().join("wae-lsp-test");
        let uri = file_uri(&root, "src/a file.ts").unwrap();
        assert_eq!(uri_path(&root, &uri).unwrap(), "src/a file.ts");
    }

    #[test]
    fn diagnostic_columns_are_converted_from_code_points_to_utf16_units() {
        let source = "const emoji = \"😀\"; import './missing';";
        let specifier_column = source.find("./missing").unwrap();
        let code_point_column = source[..specifier_column].chars().count() + 1;
        let mut diagnostic = Diagnostic::new("RESOLVE-001", "missing");
        diagnostic.primary_location =
            Some(SourceLocation { file: "src/a.ts".into(), line: 1, column: code_point_column });
        let value = lsp_diagnostic(Path::new("/project"), &diagnostic, Some(source));
        assert_eq!(
            value["range"]["start"]["character"],
            source[..specifier_column].encode_utf16().count()
        );
    }

    #[test]
    fn initialization_keeps_every_workspace_folder() {
        let first = std::env::temp_dir().join("wae-lsp-roots-a");
        let second = std::env::temp_dir().join("wae-lsp-roots-b");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let roots = workspace_roots(&json!({
            "workspaceFolders": [
                {"uri": Url::from_directory_path(&first).unwrap().to_string()},
                {"uri": Url::from_directory_path(&second).unwrap().to_string()}
            ]
        }))
        .unwrap();
        assert_eq!(roots.len(), 2);
        std::fs::remove_dir_all(first).unwrap();
        std::fs::remove_dir_all(second).unwrap();
    }
}
