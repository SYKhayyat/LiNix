use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An actual package discovered on the system or found in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    /// The unique name of the package within its backend (e.g. "ripgrep").
    pub name: String,
    /// The backend identifier that owns this package (e.g. "apt", "cargo").
    pub backend: String,
    /// The version string if known (e.g. "1.2.3").
    pub version: Option<String>,
    /// Production property map for extensible metadata.
    pub properties: HashMap<String, String>,
}

/// The intent to have a package installed with specific metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    /// The name of the package.
    pub name: String,
    /// The targeted backend.
    pub backend: String,
    /// Configuration options parsed from the '@' tag.
    pub options: HashMap<String, String>,
    /// A list of other package identifiers this package depends on.
    pub requires: Vec<String>,
}

impl Package {
    /// Creates a new Package instance with no version or properties.
    pub fn new(name: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            backend: backend.into(),
            version: None,
            properties: HashMap::new(),
        }
    }

    /// Creates a new Package instance with a specific version.
    pub fn with_version(name: &str, version: &str, backend: &str) -> Self {
        let mut p = Self::new(name, backend);
        p.version = Some(version.to_string());
        p
    }

    /// Returns a human-readable name including version if available.
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
