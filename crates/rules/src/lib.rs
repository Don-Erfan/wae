use wae_core::domain::{Diagnostic, Project};

pub trait Rule {
    fn check(&self, _project: &Project) -> Vec<Diagnostic>;
}

#[derive(Debug, Default)]
pub struct RuleSet {
    rules: Vec<Box<dyn Rule + Send + Sync>>,
}

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rule<R>(mut self, rule: R) -> Self
    where
        R: Rule + Send + Sync + 'static,
    {
        self.rules.push(Box::new(rule));
        self
    }

    pub fn check_all(&self, project: &Project) -> Vec<Diagnostic> {
        self.rules.iter().flat_map(|rule| rule.check(project)).collect()
    }
}
