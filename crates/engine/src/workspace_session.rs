use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Analysis, AnalysisError, AnalyzeRequest, CancellationToken, Engine};

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
}

impl WorkspaceSession {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            engine: Engine::default(),
            generation: AtomicU64::new(0),
            active: Mutex::new(None),
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

    pub fn analyze(
        &self,
        ticket: &AnalysisTicket,
        overlays: &BTreeMap<String, String>,
    ) -> Result<Analysis, AnalysisError> {
        if !self.is_current(ticket) {
            return Err(AnalysisError::Cancelled);
        }
        let request = overlays.iter().fold(
            AnalyzeRequest::new(&self.root).with_cancellation(ticket.cancellation.clone()),
            |request, (module, source)| request.with_overlay(module.clone(), source.clone()),
        );
        self.engine.analyze(request)
    }
}

impl Drop for WorkspaceSession {
    fn drop(&mut self) {
        if let Ok(active) = self.active.lock() {
            if let Some(token) = active.as_ref() {
                token.cancel();
            }
        }
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
        let cold = session.analyze(&session.begin_analysis(), &BTreeMap::new()).unwrap();
        assert_eq!(cold.incremental.analyzed_modules, 2);

        let overlays = BTreeMap::from([(
            "src/a.ts".into(),
            "import './b'; export const edited = true;".into(),
        )]);
        let edited = session.analyze(&session.begin_analysis(), &overlays).unwrap();
        assert_eq!(edited.incremental.analyzed_modules, 1);
        assert_eq!(edited.incremental.restored_modules, 1);
        fs::remove_dir_all(root).unwrap();
    }
}
