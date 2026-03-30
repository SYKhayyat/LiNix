// src/core/package.rs
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: Option<String>,
    pub backend: String,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub backend: String,
    pub options: HashMap<String, String>,
}

impl Package {
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