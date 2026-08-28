use std::collections::BTreeSet;

use wae_config::ResolutionMode;
use wae_core::domain::DependencyKind;

use super::{ResolutionKind, ResolutionRequest};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConditionSet(BTreeSet<String>);

impl ConditionSet {
    pub fn contains(&self, condition: &str) -> bool {
        self.0.contains(condition)
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }

    fn common(request: &ResolutionRequest<'_>) -> Self {
        let mut conditions = BTreeSet::from(["default".to_owned()]);
        conditions.insert(
            match request.resolution_kind {
                ResolutionKind::Import => "import",
                ResolutionKind::Require => "require",
            }
            .to_owned(),
        );
        if request.dependency_kind == DependencyKind::TypeOnly {
            conditions.insert("types".to_owned());
        }
        conditions.extend(request.custom_conditions.iter().cloned());
        Self(conditions)
    }

    fn with_node(mut self) -> Self {
        self.0.insert("node".to_owned());
        self
    }
}

/// Strategy port for mode-specific package condition selection.
pub trait ConditionSetProvider {
    fn active_conditions(&self, request: &ResolutionRequest<'_>) -> ConditionSet;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Node16Conditions;

impl ConditionSetProvider for Node16Conditions {
    fn active_conditions(&self, request: &ResolutionRequest<'_>) -> ConditionSet {
        ConditionSet::common(request).with_node()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NodeNextConditions;

impl ConditionSetProvider for NodeNextConditions {
    fn active_conditions(&self, request: &ResolutionRequest<'_>) -> ConditionSet {
        ConditionSet::common(request).with_node()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BundlerConditions;

impl ConditionSetProvider for BundlerConditions {
    fn active_conditions(&self, request: &ResolutionRequest<'_>) -> ConditionSet {
        // TypeScript's bundler resolver does not implicitly activate the ecosystem-specific
        // `browser` condition. Users can opt into it through custom_conditions.
        ConditionSet::common(request)
    }
}

pub(crate) fn active_conditions(request: &ResolutionRequest<'_>) -> ConditionSet {
    match request.mode {
        ResolutionMode::Node10 | ResolutionMode::Node16 => {
            Node16Conditions.active_conditions(request)
        }
        ResolutionMode::NodeNext => NodeNextConditions.active_conditions(request),
        ResolutionMode::Bundler => BundlerConditions.active_conditions(request),
    }
}
