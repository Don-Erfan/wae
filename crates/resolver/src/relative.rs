use std::path::{Path, PathBuf};

use wae_config::ResolutionMode;
use wae_core::domain::ModulePath;

use super::{ModuleFormat, ResolutionKind, ResolutionRequest, normalize};

pub fn resolve_file(base: &Path) -> Option<ModulePath> {
    resolve_file_with_mode(base, ResolutionMode::NodeNext)
}

pub fn resolve_file_with_mode(base: &Path, mode: ResolutionMode) -> Option<ModulePath> {
    const EXTENSIONS: [&str; 8] = ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"];
    if base.is_file() {
        return Some(ModulePath(normalize(base)));
    }
    let mapped: &[&str] = match (mode, base.extension().and_then(|value| value.to_str())) {
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("js"),
        ) => &["ts", "tsx", "js", "jsx"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("mjs"),
        ) => &["mts", "mjs"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("cjs"),
        ) => &["cts", "cjs"],
        _ => &[],
    };
    for extension in mapped {
        let candidate = base.with_extension(extension);
        if candidate.is_file() {
            return Some(ModulePath(normalize(&candidate)));
        }
    }
    if mapped.is_empty() {
        for extension in EXTENSIONS {
            let candidate = PathBuf::from(format!("{}.{}", base.to_string_lossy(), extension));
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

pub(super) fn resolve_relative(base: &Path, request: &ResolutionRequest<'_>) -> Option<ModulePath> {
    let node_esm = matches!(request.mode, ResolutionMode::Node16 | ResolutionMode::NodeNext)
        && request.importer_format == ModuleFormat::Esm
        && request.resolution_kind == ResolutionKind::Import;
    if !node_esm {
        return resolve_file_with_mode(base, request.mode);
    }
    let extension = base.extension().and_then(|value| value.to_str())?;
    let mapped = match extension {
        "js" => &["ts", "tsx", "js", "jsx"][..],
        "mjs" => &["mts", "mjs"][..],
        "cjs" => &["cts", "cjs"][..],
        _ => return base.is_file().then(|| ModulePath(normalize(base))),
    };
    mapped
        .iter()
        .map(|extension| base.with_extension(extension))
        .find(|candidate| candidate.is_file())
        .map(|candidate| ModulePath(normalize(&candidate)))
}

pub(super) fn resolution_candidates(base: &Path, mode: ResolutionMode) -> Vec<ModulePath> {
    let mut candidates = vec![ModulePath(normalize(base))];
    let extensions: &[&str] = match (mode, base.extension().and_then(|value| value.to_str())) {
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("js"),
        ) => &["ts", "tsx", "js", "jsx"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("mjs"),
        ) => &["mts", "mjs"],
        (
            ResolutionMode::Node16 | ResolutionMode::NodeNext | ResolutionMode::Bundler,
            Some("cjs"),
        ) => &["cts", "cjs"],
        _ => &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"],
    };
    for extension in extensions {
        candidates.push(ModulePath(normalize(&base.with_extension(extension))));
        candidates.push(ModulePath(normalize(&base.join(format!("index.{extension}")))));
    }
    candidates
}
