use serde::{Deserialize, Serialize};

/// Represents a package in the system
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Package {
    /// Package name
    pub name: String,

    /// Package version (if available)
    pub version: Option<String>,

    /// Backend managing this package
    pub backend: String,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl Package {
    /// Create a new package
    pub fn new(name: impl Into<String>, backend: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            backend: backend.into(),
            description: None,
            repository: None,
            size: None,
        }
    }

    /// Create a package with version
    pub fn with_version(
        name: impl Into<String>,
        version: impl Into<String>,
        backend: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            version: Some(version.into()),
            backend: backend.into(),
            description: None,
            repository: None,
            size: None,
        }
    }

    /// Get a display name for the package
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
        write!(f, "{}/{}", self.backend, self.display_name())
    }
}
