use std::path::Path;

use wae_core::domain::ModulePath;
use wae_resolver::{ModuleFormat, PackageModuleType, WorkspaceResolver};

/// Classifies an importer before constructing its resolution request. Explicit module extensions
/// take precedence over the nearest package manifest, matching Node's module-format rules.
pub(crate) fn module_format(importer: &ModulePath, packages: &WorkspaceResolver) -> ModuleFormat {
    match Path::new(&importer.0).extension().and_then(|extension| extension.to_str()) {
        Some("mjs" | "mts") => ModuleFormat::Esm,
        Some("cjs" | "cts") => ModuleFormat::CommonJs,
        _ => match packages
            .package_context(Path::new(&importer.0))
            .map(|context| context.module_type)
        {
            Some(PackageModuleType::Module) => ModuleFormat::Esm,
            Some(PackageModuleType::CommonJs | PackageModuleType::Unspecified) | None => {
                ModuleFormat::CommonJs
            }
        },
    }
}
