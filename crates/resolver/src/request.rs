use wae_config::ResolutionMode;
use wae_core::domain::{DependencyKind, ModulePath};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Module(ModulePath),
    External(String),
    Redirect(String),
    Invalid(String),
    Unresolved,
}

pub struct ResolutionRequest<'a> {
    pub importer: &'a ModulePath,
    pub specifier: &'a str,
    pub dependency_kind: DependencyKind,
    pub mode: ResolutionMode,
    pub custom_conditions: &'a [String],
}

pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, request: &ResolutionRequest<'_>) -> Resolution;
}

/// A resolution handler is one link in the Node/TypeScript resolution chain.
pub trait ResolutionHandler: Send + Sync {
    fn try_resolve(&self, request: &ResolutionRequest<'_>) -> Option<Resolution>;

    fn candidate_paths(&self, _request: &ResolutionRequest<'_>) -> Vec<ModulePath> {
        Vec::new()
    }
}
