use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An actual package discovered on the system or found in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// Unique only within `backend` — two backends may each hold a different "ripgrep".
    pub name: String,
    pub backend: String,
    pub version: Option<String>,
    pub properties: HashMap<String, String>,
}

/// The intent to have a package installed with specific metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub backend: String,
    /// Parsed from the manifest entry's '@' tag.
    pub options: HashMap<String, String>,
    pub requires: Vec<String>,
}

impl Package {
    pub fn new(name: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            backend: backend.into(),
            version: None,
            properties: HashMap::new(),
        }
    }

    pub fn with_version(name: &str, version: &str, backend: &str) -> Self {
        let mut p = Self::new(name, backend);
        p.version = Some(version.to_string());
        p
    }

    pub fn display_name(&self) -> String {
        if let Some(version) = &self.version {
            format!("{} ({})", self.name, version)
        } else {
            self.name.clone()
        }
    }
}

impl std::fmt::Display for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.backend, self.display_name())
    }
}
