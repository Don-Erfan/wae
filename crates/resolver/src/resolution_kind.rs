use wae_config::ResolutionMode;
use wae_core::domain::DependencyKind;

use crate::{ModuleFormat, ResolutionKind};

/// Syntax-level dependency form. Package format may influence Node16/NodeNext, but it never
/// changes what syntax the parser observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DependencySyntax {
    Import,
    DynamicImport,
    Require,
}

impl From<&DependencyKind> for DependencySyntax {
    fn from(kind: &DependencyKind) -> Self {
        match kind {
            DependencyKind::Dynamic => Self::DynamicImport,
            DependencyKind::Require => Self::Require,
            DependencyKind::Static | DependencyKind::TypeOnly | DependencyKind::ReExport => {
                Self::Import
            }
        }
    }
}

/// Strategy port for selecting the `import` or `require` condition independently of the package
/// condition-selection strategy.
pub trait ResolutionKindProvider {
    fn resolution_kind(&self, syntax: DependencySyntax, importer: ModuleFormat) -> ResolutionKind;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BundlerResolutionKindProvider;

impl ResolutionKindProvider for BundlerResolutionKindProvider {
    fn resolution_kind(&self, syntax: DependencySyntax, _importer: ModuleFormat) -> ResolutionKind {
        match syntax {
            DependencySyntax::Require => ResolutionKind::Require,
            DependencySyntax::Import | DependencySyntax::DynamicImport => ResolutionKind::Import,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Node16ResolutionKindProvider;

impl ResolutionKindProvider for Node16ResolutionKindProvider {
    fn resolution_kind(&self, syntax: DependencySyntax, importer: ModuleFormat) -> ResolutionKind {
        node_modern_resolution_kind(syntax, importer)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NodeNextResolutionKindProvider;

impl ResolutionKindProvider for NodeNextResolutionKindProvider {
    fn resolution_kind(&self, syntax: DependencySyntax, importer: ModuleFormat) -> ResolutionKind {
        node_modern_resolution_kind(syntax, importer)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Node10ResolutionKindProvider;

impl ResolutionKindProvider for Node10ResolutionKindProvider {
    fn resolution_kind(&self, syntax: DependencySyntax, _importer: ModuleFormat) -> ResolutionKind {
        match syntax {
            DependencySyntax::Require => ResolutionKind::Require,
            DependencySyntax::Import | DependencySyntax::DynamicImport => ResolutionKind::Import,
        }
    }
}

fn node_modern_resolution_kind(syntax: DependencySyntax, importer: ModuleFormat) -> ResolutionKind {
    match syntax {
        DependencySyntax::DynamicImport => ResolutionKind::Import,
        DependencySyntax::Require => ResolutionKind::Require,
        DependencySyntax::Import if importer == ModuleFormat::CommonJs => ResolutionKind::Require,
        DependencySyntax::Import => ResolutionKind::Import,
    }
}

pub fn resolution_kind_for(
    mode: ResolutionMode,
    dependency_kind: &DependencyKind,
    importer: ModuleFormat,
) -> ResolutionKind {
    let syntax = DependencySyntax::from(dependency_kind);
    match mode {
        ResolutionMode::Node10 => Node10ResolutionKindProvider.resolution_kind(syntax, importer),
        ResolutionMode::Node16 => Node16ResolutionKindProvider.resolution_kind(syntax, importer),
        ResolutionMode::NodeNext => {
            NodeNextResolutionKindProvider.resolution_kind(syntax, importer)
        }
        ResolutionMode::Bundler => BundlerResolutionKindProvider.resolution_kind(syntax, importer),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundler_uses_syntax_in_packages_without_a_module_type() {
        for kind in [
            DependencyKind::Static,
            DependencyKind::ReExport,
            DependencyKind::TypeOnly,
            DependencyKind::Dynamic,
        ] {
            assert_eq!(
                resolution_kind_for(ResolutionMode::Bundler, &kind, ModuleFormat::CommonJs),
                ResolutionKind::Import
            );
        }
        assert_eq!(
            resolution_kind_for(
                ResolutionMode::Bundler,
                &DependencyKind::Require,
                ModuleFormat::Esm
            ),
            ResolutionKind::Require
        );
    }
}
