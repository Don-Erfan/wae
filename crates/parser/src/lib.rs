use wae_core::domain::{Import, ModulePath};

pub trait ParserAdapter {
    fn parse_imports(&self, _module_path: &ModulePath, _source: &str) -> Vec<Import>;
}

#[derive(Debug, Default)]
pub struct NoopParser;

impl ParserAdapter for NoopParser {
    fn parse_imports(&self, _module_path: &ModulePath, _source: &str) -> Vec<Import> {
        Vec::new()
    }
}
