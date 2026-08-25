mod legacy_entrypoints;
mod target;
mod workspace_index;

pub use legacy_entrypoints::{LegacyEntrypoints, PackageRelativePath};
pub(crate) use target::{
    PackageTarget, lexically_within, manifest_entrypoints, manifest_imports, resolve_export,
    resolve_package_target,
};
pub use workspace_index::{PackageModuleType, WorkspacePackage, WorkspacePackageIndex};
