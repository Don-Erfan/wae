use serde::{Deserialize, Serialize};
use wae_core::domain::PackageName;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub workspace_name: String,
    pub packages: Vec<PackageName>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            workspace_name: String::from("default"),
            packages: Vec::new(),
        }
    }
}
