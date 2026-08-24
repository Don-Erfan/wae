use std::path::Path;

use wae_core::domain::ModulePath;
use wae_resolver::{ModuleFormat, PackageModuleType, PackageScopeIndex};

/// Classifies importers before resolution requests are built. Explicit module extensions take
/// precedence over the nearest Node package scope.
pub(crate) struct ModuleFormatResolver<'a> {
    scopes: &'a PackageScopeIndex,
}

impl<'a> ModuleFormatResolver<'a> {
    pub(crate) fn new(scopes: &'a PackageScopeIndex) -> Self {
        Self { scopes }
    }

    pub(crate) fn resolve(&self, importer: &ModulePath) -> ModuleFormat {
        match Path::new(&importer.0).extension().and_then(|extension| extension.to_str()) {
            Some("mjs" | "mts") => ModuleFormat::Esm,
            Some("cjs" | "cts") => ModuleFormat::CommonJs,
            _ => match self.scopes.nearest(Path::new(&importer.0)).map(|scope| scope.module_type) {
                Some(PackageModuleType::Module) => ModuleFormat::Esm,
                Some(PackageModuleType::CommonJs | PackageModuleType::Unspecified) | None => {
                    ModuleFormat::CommonJs
                }
            },
        }
    }
}
