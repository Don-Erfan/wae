use wae_core::domain::{ModulePath, PackageName};

pub trait ModuleResolver {
    fn resolve(&self, importer: &ModulePath, specifier: &str) -> ModulePath;
}

#[derive(Debug, Clone)]
pub struct IdentityResolver {
    package: PackageName,
}

impl IdentityResolver {
    pub fn new(package: PackageName) -> Self {
        Self { package }
    }

    pub fn package(&self) -> &PackageName {
        &self.package
    }
}

impl ModuleResolver for IdentityResolver {
    fn resolve(&self, importer: &ModulePath, _specifier: &str) -> ModulePath {
        importer.clone()
    }
}
