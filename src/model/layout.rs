use std::path::{Path, PathBuf};

/// A validated module name (II.5): lowercase letters, digits, `-` and `_`.
///
/// `module_file` joins the name into a path and cannot fail, so the check has to happen
/// before the name reaches it — that is what this type is. Excluding `.` rules out `..`
/// and a second extension together, rather than enumerating separators and hoping the
/// list is complete.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleName(String);

impl ModuleName {
    pub fn new(name: &str) -> Result<Self, String> {
        let lowered = name.to_lowercase();
        if lowered.is_empty()
            || !lowered
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "`{}` is not a module name — modules are lowercase letters, digits, `-` and `_`.",
                name
            ));
        }
        Ok(Self(lowered))
    }

    /// For names fixed in the source. Panics on an invalid one, which is a bug in this crate,
    /// not a user error.
    pub fn literal(name: &'static str) -> Self {
        Self::new(name).expect("a module name written into Shall must satisfy II.5")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModuleName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where everything lives (SPEC II.1).
///
/// Two roots, and the split is load-bearing. **Your repo** holds what you wrote and is a
/// git repo. **Shall's data** holds what Shall worked out, never goes in git, and never
/// goes in a folder Shall scans.
///
/// Keeping them apart is the fix for the shape of Monday's bug: `registry.json` (what
/// Shall owns) lived somewhere `-g` could not move while the wish list moved, so ownership
/// and intent disagreed and everything owned-but-unwished read as drift (V.1). They can no
/// longer be pointed at each other because they are no longer the same kind of thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    config_root: PathBuf,
    data_root: PathBuf,
}

impl Layout {
    /// `$SHALL_CONFIG_DIR` / `$SHALL_DATA_DIR`, else the platform dirs.
    pub fn discover() -> Self {
        let config_root = std::env::var_os("SHALL_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(crate::utils::safe_config_dir);
        let data_root = std::env::var_os("SHALL_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(crate::utils::safe_data_dir);
        Self {
            config_root,
            data_root,
        }
    }

    pub fn new(config_root: impl Into<PathBuf>, data_root: impl Into<PathBuf>) -> Self {
        Self {
            config_root: config_root.into(),
            data_root: data_root.into(),
        }
    }

    /// Your repo. This is a git repo; every path below it is yours and travels with it.
    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    /// Your lists. Lowercase names, `*.txt`. **The folder decides** — anything else in here
    /// is ignored, so a `README.md` costs nothing (II.3).
    pub fn modules_dir(&self) -> PathBuf {
        self.config_root.join("modules")
    }

    /// Your choices. Capitalized names.
    pub fn profiles_dir(&self) -> PathBuf {
        self.config_root.join("profiles")
    }

    /// Which profiles are on. Answers exactly one question and nothing else goes in it.
    pub fn active_file(&self) -> PathBuf {
        self.config_root.join("active")
    }

    /// Which backends, in order. Listed = available to Shall; not listed = Shall does not
    /// use it at all (V.15).
    pub fn priority_file(&self) -> PathBuf {
        self.config_root.join("priority")
    }

    /// The `groups` file (U18): named backend chains, `tools = apt, dnf, cargo`.
    pub fn groups_file(&self) -> PathBuf {
        self.config_root.join("groups")
    }

    /// When Shall runs itself. Being in the file means it is on — no active-list (V.28).
    pub fn schedules_file(&self) -> PathBuf {
        self.config_root.join("schedules")
    }

    /// Your own names for conditions (IX.2). Absent means no variables, not an error.
    pub fn vars_file(&self) -> PathBuf {
        self.config_root.join("vars")
    }

    /// What you have taught Shall: managers it does not ship, settings stores it has no
    /// adapter for, and how to obtain a manager that is missing (U10, ruled 2026-07-24).
    ///
    /// One folder, **in the repo** — a definition that cannot travel makes every line that uses
    /// it fail on every machine but the one where somebody hand-wrote it, including the fresh
    /// machine the repo exists to set up. One file per surface rather than one file, because
    /// each answers a different question and each is approved separately.
    ///
    /// This doc said *three files* and named them for as long as there have been eight, which is
    /// the small version of the same problem `app::adapters::SURFACES` exists to fix: nowhere in
    /// the program listed the extension points, so nothing noticed when the list grew.
    pub fn adapters_dir(&self) -> PathBuf {
        self.config_root.join("adapters")
    }

    /// One extension surface's file, by the name `app::adapters::SURFACES` gives it.
    ///
    /// The named accessors below are this, spelled out, and they stay because a caller reading
    /// `adapter_secret_file()` should not have to know a string. What does not stay is the ninth
    /// surface written as `adapters_dir().join("firewall.toml")` inline — `firewall:` was read
    /// that way, so it had no accessor, and a table of surfaces built from the accessors would
    /// have been a table with seven rows and no way to notice.
    pub fn adapter_file(&self, surface: &str) -> PathBuf {
        self.adapters_dir().join(format!("{surface}.toml"))
    }

    /// `[[backend]]` — how to drive a package manager Shall does not ship (XIII.2).
    pub fn adapter_backends_file(&self) -> PathBuf {
        self.adapter_file("backends")
    }

    /// `[[firewall]]` — how to drive a firewall Shall does not ship (N3).
    pub fn adapter_firewall_file(&self) -> PathBuf {
        self.adapter_file("firewall")
    }

    /// `[[setting_store]]` — how to read and write a settings store (K17).
    pub fn adapter_settings_file(&self) -> PathBuf {
        self.adapter_file("settings")
    }

    /// `[[bootstrap]]` — how to obtain a manager this machine does not have (7c).
    pub fn adapter_bootstrap_file(&self) -> PathBuf {
        self.adapter_file("bootstrap")
    }

    /// `[[prereq]]` — the setup a manager needs before it can install anything (Q10/Q11/Q13).
    /// Shall ships rows for the three that were measured; this is where a user adds a fourth.
    pub fn adapter_prereq_file(&self) -> PathBuf {
        self.adapter_file("prereq")
    }

    /// `[[init]]` — how to drive an init system Shall does not ship a built-in for (U36).
    pub fn adapter_init_file(&self) -> PathBuf {
        self.adapter_file("init")
    }

    /// `[[snapshot]]` — how to drive a snapshot/rollback provider from data (U27). A row that
    /// does not declare it can restore a running system is create-only, never `Live` (V.60).
    pub fn adapter_snapshot_file(&self) -> PathBuf {
        self.adapter_file("snapshot")
    }

    /// `[[secret]]` — how to decrypt with a provider Shall does not ship (U38). A row that does
    /// not promise the plaintext reaches stdout only is refused, never trusted with a secret.
    pub fn adapter_secret_file(&self) -> PathBuf {
        self.adapter_file("secret")
    }

    /// What everything resolved to. Generated, in git, yours. One file per backend.
    pub fn locks_dir(&self) -> PathBuf {
        self.config_root.join("locks")
    }

    pub fn lock_file(&self, backend: &str) -> PathBuf {
        self.locks_dir().join(format!("{}.toml", backend))
    }

    /// Refusals and behaviour. Nothing writes to it but you (II.6).
    pub fn preferences_file(&self) -> PathBuf {
        self.config_root.join(crate::config::PREFERENCES_FILE_NAME)
    }

    /// What Shall currently owns. **Never in git. Never in a folder Shall scans.**
    pub fn registry_file(&self) -> PathBuf {
        self.data_root.join("registry.json")
    }

    /// Snapshot metadata, tagged with commit hashes.
    pub fn snapshots_dir(&self) -> PathBuf {
        self.data_root.join("snapshots")
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    /// The module file for `name`. II.3: the filename is the module name, lowercased.
    pub fn module_file(&self, name: &ModuleName) -> PathBuf {
        self.modules_dir().join(format!("{}.txt", name))
    }

    pub fn profile_file(&self, name: &str) -> PathBuf {
        self.profiles_dir().join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::new("/cfg", "/data")
    }

    #[test]
    fn your_repo_holds_what_you_wrote() {
        let l = layout();
        assert_eq!(l.modules_dir(), PathBuf::from("/cfg/modules"));
        assert_eq!(l.profiles_dir(), PathBuf::from("/cfg/profiles"));
        assert_eq!(l.active_file(), PathBuf::from("/cfg/active"));
        assert_eq!(l.priority_file(), PathBuf::from("/cfg/priority"));
        assert_eq!(l.schedules_file(), PathBuf::from("/cfg/schedules"));
        assert_eq!(l.locks_dir(), PathBuf::from("/cfg/locks"));
        assert_eq!(l.preferences_file(), PathBuf::from("/cfg/preferences.toml"));
    }

    #[test]
    fn shall_data_is_never_inside_your_repo() {
        // II.1. The registry is what Shall OWNS; the modules are what you WANT. When those
        // two can be pointed at each other, owned-but-unwished reads as drift and drift
        // gets removed — which is Monday's bug (V.1).
        let l = layout();
        assert!(!l.registry_file().starts_with(l.config_root()));
        assert!(!l.snapshots_dir().starts_with(l.config_root()));
    }

    #[test]
    fn a_module_file_is_its_name_lowercased() {
        assert_eq!(
            layout().module_file(&ModuleName::new("Editors").unwrap()),
            PathBuf::from("/cfg/modules/editors.txt")
        );
    }

    #[test]
    fn a_module_name_cannot_climb_out_of_the_modules_folder() {
        // SEC6. `module add --name ../../foo` wrote outside `modules/`; the type is what
        // stops it, so no call site can lose the check.
        let err = ModuleName::new("../../foo").unwrap_err();
        assert!(err.contains("is not a module name"), "{}", err);
        assert!(
            err.contains("lowercase letters, digits"),
            "teach the rule: {}",
            err
        );

        for bad in ["", "a/b", "a\\b", "a.b", "..", "a b", "Ünicode"] {
            assert!(ModuleName::new(bad).is_err(), "`{}` must be refused", bad);
        }
        for good in ["editors", "Editors", "web-dev", "py_3", "x9"] {
            assert!(ModuleName::new(good).is_ok(), "`{}` must be accepted", good);
        }
    }

    #[test]
    fn a_module_name_is_lowercased_once_at_construction() {
        assert_eq!(ModuleName::new("Editors").unwrap().as_str(), "editors");
    }
}
