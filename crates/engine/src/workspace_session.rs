use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    Analysis, AnalysisError, AnalysisExecution, AnalyzeRequest, CancellationToken, ChangeSet,
    Engine,
};

/// Generation-scoped work issued by a long-lived editor session.
#[derive(Clone, Debug)]
pub struct AnalysisTicket {
    generation: u64,
    cancellation: CancellationToken,
}

impl AnalysisTicket {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Stateful facade for LSP/IDE clients. Starting a newer generation cancels the previous one and
/// results can be checked for staleness before publication.
pub struct WorkspaceSession {
    root: PathBuf,
    engine: Engine,
    generation: AtomicU64,
    active: Mutex<Option<CancellationToken>>,
    snapshot: Mutex<Option<WorkspaceSnapshot>>,
    last_execution: Mutex<AnalysisExecution>,
}

#[derive(Clone, Debug)]
struct WorkspaceSnapshot {
    overlays: BTreeMap<String, String>,
    analysis: Arc<Analysis>,
}

impl WorkspaceSession {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        // Keep every path produced by a long-lived session in the same namespace as the engine.
        // This matters on macOS (`/var` aliases `/private/var`) and on Windows where
        // canonicalization may add a verbatim prefix. Without it, a known module and an overlay
        // for that module can compare as different paths and the edit is parsed twice.
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        Self {
            root,
            engine: Engine::default(),
            generation: AtomicU64::new(0),
            active: Mutex::new(None),
            snapshot: Mutex::new(None),
            last_execution: Mutex::new(AnalysisExecution::default()),
        }
    }

    pub fn begin_analysis(&self) -> AnalysisTicket {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let cancellation = CancellationToken::default();
        let mut active = self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous) = active.replace(cancellation.clone()) {
            previous.cancel();
        }
        AnalysisTicket { generation, cancellation }
    }

    pub fn is_current(&self, ticket: &AnalysisTicket) -> bool {
        self.generation.load(Ordering::Acquire) == ticket.generation
            && !ticket.cancellation.is_cancelled()
    }

    pub fn cancel_active(&self) {
        if let Some(token) =
            self.active.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref()
        {
            token.cancel();
        }
    }

    pub fn changes_since_snapshot(&self, overlays: &BTreeMap<String, String>) -> ChangeSet {
        let snapshot = self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = snapshot.as_ref().map(|snapshot| &snapshot.overlays);
        let changed = overlays
            .iter()
            .filter(|(module, source)| previous.and_then(|old| old.get(*module)) != Some(*source))
            .map(|(module, _)| module.clone())
            .collect();
        let deleted = previous
            .into_iter()
            .flat_map(|old| old.keys())
            .filter(|module| !overlays.contains_key(*module))
            .cloned()
            .collect();
        ChangeSet { changed, deleted }
    }

    pub fn snapshot(&self) -> Option<Arc<Analysis>> {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|snapshot| Arc::clone(&snapshot.analysis))
    }

    pub fn last_execution(&self) -> AnalysisExecution {
        self.last_execution.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    pub fn analyze(
        &self,
        ticket: &AnalysisTicket,
        overlays: &BTreeMap<String, String>,
    ) -> Result<Arc<Analysis>, AnalysisError> {
        self.analyze_changes(ticket, overlays, true)
    }

    pub fn analyze_changes(
        &self,
        ticket: &AnalysisTicket,
        overlays: &BTreeMap<String, String>,
        force: bool,
    ) -> Result<Arc<Analysis>, AnalysisError> {
        if !self.is_current(ticket) {
            return Err(AnalysisError::Cancelled);
        }
        if !force && self.changes_since_snapshot(overlays) == ChangeSet::default() {
            if let Some(analysis) = self.snapshot() {
                let restored_modules = analysis
                    .project
                    .modules
                    .iter()
                    .filter(|module| module.kind == wae_core::domain::ModuleKind::Source)
                    .count();
                *self.last_execution.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                    AnalysisExecution {
                        incremental: crate::IncrementalStats {
                            cache_enabled: analysis.incremental.cache_enabled,
                            restored_modules,
                            analyzed_modules: 0,
                            rule_snapshot_reused: true,
                            restored_rules: analysis.incremental.restored_rules
                                + analysis.incremental.evaluated_rules,
                            evaluated_rules: 0,
                            environment_hash: analysis.incremental.environment_hash,
                        },
                        timings: Default::default(),
                        reused_snapshot: true,
                    };
                return Ok(analysis);
            }
        }
        let mut request = overlays.iter().fold(
            AnalyzeRequest::new(&self.root).with_cancellation(ticket.cancellation.clone()),
            |request, (module, source)| request.with_overlay(module.clone(), source.clone()),
        );
        if !force {
            let known_files =
                self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().map(
                    |snapshot| {
                        snapshot
                            .analysis
                            .project
                            .modules
                            .iter()
                            .filter(|module| module.kind == wae_core::domain::ModuleKind::Source)
                            .map(|module| self.root.join(&module.id.0))
                            .collect::<Vec<_>>()
                    },
                );
            if let Some(known_files) = known_files {
                let environment_hash = self
                    .snapshot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .map(|snapshot| snapshot.analysis.incremental.environment_hash);
                request = request.with_known_files(known_files);
                if let Some(environment_hash) = environment_hash {
                    request = request.with_known_environment_hash(environment_hash);
                }
            }
        }
        let analysis = Arc::new(self.engine.analyze(request)?);
        *self.last_execution.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
            AnalysisExecution {
                incremental: analysis.incremental.clone(),
                timings: analysis.timings.clone(),
                reused_snapshot: false,
            };
        if self.is_current(ticket) {
            *self.snapshot.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(WorkspaceSnapshot {
                    overlays: overlays.clone(),
                    analysis: Arc::clone(&analysis),
                });
        }
        Ok(analysis)
    }
}

impl Drop for WorkspaceSession {
    fn drop(&mut self) {
        self.cancel_active();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn newer_generations_cancel_previous_work() {
        let session = WorkspaceSession::new(".");
        let old = session.begin_analysis();
        let current = session.begin_analysis();
        assert!(!session.is_current(&old));
        assert!(session.is_current(&current));
        assert!(matches!(session.analyze(&old, &BTreeMap::new()), Err(AnalysisError::Cancelled)));
    }

    #[test]
    fn editor_overlay_reanalyzes_only_the_changed_module() {
        let root =
            std::env::temp_dir().join(format!("wae-session-incremental-{}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "import './b';").unwrap();
        fs::write(root.join("src/b.ts"), "export const b = true;").unwrap();
        fs::write(
            root.join("wae.yaml"),
            "version: 1\nresolution:\n  mode: bundler\ncache:\n  enabled: true\n",
        )
        .unwrap();
        let session = WorkspaceSession::new(&root);
        assert_eq!(session.root, root.canonicalize().unwrap());
        let cold = session.analyze(&session.begin_analysis(), &BTreeMap::new()).unwrap();
        assert_eq!(cold.incremental.analyzed_modules, 2);

        let overlays = BTreeMap::from([(
            "src/a.ts".into(),
            "import './b'; export const edited = true;".into(),
        )]);
        let edited = session.analyze_changes(&session.begin_analysis(), &overlays, false).unwrap();
        assert_eq!(edited.incremental.analyzed_modules, 1);
        assert_eq!(edited.incremental.restored_modules, 1);
        assert_eq!(edited.incremental.environment_hash, cold.incremental.environment_hash);
        assert_eq!(session.changes_since_snapshot(&overlays), ChangeSet::default());
        assert_eq!(session.snapshot().unwrap().project.modules.len(), 2);
        let no_op = session.analyze_changes(&session.begin_analysis(), &overlays, false).unwrap();
        assert!(Arc::ptr_eq(&edited, &no_op));
        assert_eq!(session.last_execution().incremental.analyzed_modules, 0);
        assert_eq!(session.last_execution().incremental.restored_modules, 2);
        assert!(session.last_execution().reused_snapshot);

        fs::write(root.join("package.json"), r#"{"name":"changed-environment"}"#).unwrap();
        let forced = session.analyze_changes(&session.begin_analysis(), &overlays, true).unwrap();
        assert_ne!(forced.incremental.environment_hash, cold.incremental.environment_hash);
        fs::remove_dir_all(root).unwrap();
    }
}
