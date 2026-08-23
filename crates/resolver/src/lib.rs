use std::path::{Component, Path, PathBuf};

use wae_core::domain::ModulePath;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    Module(ModulePath),
    External(String),
    Unresolved,
}

pub trait ModuleResolver: Send + Sync {
    fn resolve(&self, importer: &ModulePath, specifier: &str) -> Resolution;
}

/// A resolution handler is one link in the Node/TypeScript resolution chain.
pub trait ResolutionHandler: Send + Sync {
    fn try_resolve(&self, importer: &Path, specifier: &str) -> Option<Resolution>;
}

#[derive(Default)]
pub struct ResolverPipeline {
    handlers: Vec<Box<dyn ResolutionHandler>>,
}

impl ResolverPipeline {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_handler<H: ResolutionHandler + 'static>(mut self, handler: H) -> Self {
        self.handlers.push(Box::new(handler));
        self
    }
    pub fn node_defaults(root: impl Into<PathBuf>, aliases: Vec<PathAlias>) -> Self {
        Self::new()
            .with_handler(RelativeResolver)
            .with_handler(AliasResolver { root: root.into(), aliases })
            .with_handler(PackageResolver)
    }
}

impl ModuleResolver for ResolverPipeline {
    fn resolve(&self, importer: &ModulePath, specifier: &str) -> Resolution {
        let importer = Path::new(&importer.0);
        self.handlers
            .iter()
            .find_map(|handler| handler.try_resolve(importer, specifier))
            .unwrap_or(Resolution::Unresolved)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathAlias {
    pub pattern: String,
    pub targets: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct RelativeResolver;
impl ResolutionHandler for RelativeResolver {
    fn try_resolve(&self, importer: &Path, specifier: &str) -> Option<Resolution> {
        if !specifier.starts_with('.') && !specifier.starts_with('/') {
            return None;
        }
        let base = if specifier.starts_with('/') {
            PathBuf::from(specifier)
        } else {
            importer.parent()?.join(specifier)
        };
        Some(resolve_file(&base).map_or(Resolution::Unresolved, Resolution::Module))
    }
}

#[derive(Clone, Debug)]
pub struct AliasResolver {
    pub root: PathBuf,
    pub aliases: Vec<PathAlias>,
}
impl ResolutionHandler for AliasResolver {
    fn try_resolve(&self, _importer: &Path, specifier: &str) -> Option<Resolution> {
        for alias in &self.aliases {
            let capture = match alias.pattern.split_once('*') {
                Some((prefix, suffix))
                    if specifier.starts_with(prefix) && specifier.ends_with(suffix) =>
                {
                    Some(&specifier[prefix.len()..specifier.len() - suffix.len()])
                }
                None if alias.pattern == specifier => Some(""),
                _ => None,
            };
            let Some(capture) = capture else { continue };
            for target in &alias.targets {
                let candidate = self.root.join(target.replace('*', capture));
                if let Some(path) = resolve_file(&candidate) {
                    return Some(Resolution::Module(path));
                }
            }
            return Some(Resolution::Unresolved);
        }
        None
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PackageResolver;
impl ResolutionHandler for PackageResolver {
    fn try_resolve(&self, _importer: &Path, specifier: &str) -> Option<Resolution> {
        (!specifier.starts_with('.') && !specifier.starts_with('/'))
            .then(|| Resolution::External(package_name(specifier)))
    }
}

pub fn resolve_file(base: &Path) -> Option<ModulePath> {
    const EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
    if base.is_file() {
        return Some(ModulePath(normalize(base)));
    }
    if base.extension().is_none() {
        for extension in EXTENSIONS {
            let candidate = base.with_extension(extension);
            if candidate.is_file() {
                return Some(ModulePath(normalize(&candidate)));
            }
        }
    }
    if base.is_dir() {
        for extension in EXTENSIONS {
            let candidate = base.join(format!("index.{extension}"));
            if candidate.is_file() {
                return Some(ModulePath(normalize(&candidate)));
            }
        }
    }
    None
}

fn normalize(path: &Path) -> String {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().replace('\\', "/")
}

fn package_name(specifier: &str) -> String {
    if specifier.starts_with('@') {
        specifier.split('/').take(2).collect::<Vec<_>>().join("/")
    } else {
        specifier.split('/').next().unwrap_or(specifier).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn resolves_extensions_and_index_modules() {
        let root = std::env::temp_dir().join(format!("wae-resolver-{}", std::process::id()));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("folder/index.ts"), "").unwrap();
        let importer = ModulePath(root.join("a.ts").to_string_lossy().into_owned());
        assert!(matches!(
            RelativeResolver.try_resolve(Path::new(&importer.0), "./folder"),
            Some(Resolution::Module(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn alias_handler_substitutes_wildcards() {
        let root = std::env::temp_dir().join(format!("wae-alias-{}", std::process::id()));
        fs::create_dir_all(root.join("src/shared")).unwrap();
        fs::write(root.join("src/shared/util.ts"), "").unwrap();
        let resolver = AliasResolver {
            root: root.clone(),
            aliases: vec![PathAlias { pattern: "@/*".into(), targets: vec!["src/*".into()] }],
        };
        assert!(matches!(
            resolver.try_resolve(Path::new("src/a.ts"), "@/shared/util"),
            Some(Resolution::Module(_))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
