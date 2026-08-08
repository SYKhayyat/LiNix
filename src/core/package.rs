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

/// One declaration: a package, and whether it must exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSpec {
    pub name: String,
    pub backend: String,
    /// Parsed from the manifest entry's `@` tag — **the grammar's own type**.
    ///
    /// It was `HashMap<String, String>`, and the seam that forced was not small:
    /// `to_spec` joined every option value the grammar had kept as a `Vec` with `;` because
    /// "that is what the planner already splits on", and `ArtifactOptions::read` split it back
    /// on a `LIST_SEPARATOR` nothing validated. The comment above the join said *"requires is a
    /// list; the rest are single values"* — which is not true, because II.2 makes any repeated
    /// key a list and `validate_setting` refuses two values by counting them. `requires` was
    /// the one list somebody remembered, and it got a real `Vec<String>` three lines below.
    /// Every other one got a delimiter.
    pub options: crate::config::grammar::Options,
    pub requires: Vec<String>,
    /// `false` for an `absent:` line — declare it must NOT exist (SPEC II.2).
    ///
    /// It lives here rather than in a second map beside the desired state because the map
    /// type IS the seam: everything upstream produces `HashMap<backend, Vec<PackageSpec>>`
    /// and everything downstream consumes it, so `absent:` arriving as a field means most
    /// of the codebase never notices the model changed.
    ///
    /// `absent:` is the one exception to "LiNix only removes what it manages" — because
    /// you named it (V.7).
    pub present: bool,
}

impl Default for PackageSpec {
    /// **A bare line already means present. There is no `present:`** (II.3).
    fn default() -> Self {
        Self {
            name: String::new(),
            backend: String::new(),
            options: Default::default(),
            requires: Vec::new(),
            present: true,
        }
    }
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
