//! `setting:SCHEMA/KEY @value=…` — desktop configuration that lives in a settings store
//! rather than a file (X.4).
//!
//! GNOME and KDE keep configuration in dconf and kconfig, not in files `link:` can write, so a
//! tiling-WM config is `link:` and a GNOME toggle is not. The adapter is chosen by the desktop
//! that is running, not by what the user typed — which is why this is a statement, not a
//! backend named after one desktop.
//!
//! **Read before write is what makes it declarative.** A `setting:` that shells out every sync
//! is a command; one that reads the current value and writes only on a difference is a
//! declaration, and only the second belongs in this model.
//!
//! **Removal resets the key to its schema default** (owner ruling, 2026-07-20), rather than
//! restoring whatever value preceded LiNix. There is no per-key store of prior values to keep,
//! and "undeclared means the desktop's own default" is the same shape every other statement's
//! removal follows.

use crate::core::{BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package,
    PackageSpec, Queryable, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

/// The settings store driving the running desktop. One `setting` backend speaks each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingStore {
    /// GNOME's schema-backed store, via `gsettings`.
    GSettings,
    /// No adapter for the running desktop (or no desktop). A `setting:` line then refuses
    /// rather than writing something a desktop does not read.
    None,
}

/// Pure: the command that reads `schema/key`'s current value. Split out so the read-before-
/// write decision is testable without a desktop.
pub fn read_command(store: SettingStore, schema: &str, key: &str) -> Option<(String, Vec<String>)> {
    match store {
        SettingStore::GSettings => Some((
            "gsettings".into(),
            vec!["get".into(), schema.into(), key.into()],
        )),
        SettingStore::None => None,
    }
}

/// Pure: the command that writes `value` to `schema/key`.
pub fn write_command(
    store: SettingStore,
    schema: &str,
    key: &str,
    value: &str,
) -> Option<(String, Vec<String>)> {
    match store {
        SettingStore::GSettings => Some((
            "gsettings".into(),
            vec!["set".into(), schema.into(), key.into(), value.into()],
        )),
        SettingStore::None => None,
    }
}

/// Pure: the command that returns `schema/key` to its schema default (the removal path).
pub fn reset_command(store: SettingStore, schema: &str, key: &str) -> Option<(String, Vec<String>)> {
    match store {
        SettingStore::GSettings => Some((
            "gsettings".into(),
            vec!["reset".into(), schema.into(), key.into()],
        )),
        SettingStore::None => None,
    }
}

/// Pure: whether the store already holds `want`, given what `gsettings get` printed.
///
/// `gsettings` quotes string values (`'prefer-dark'`) and prints bare booleans/enums, and a
/// user writes the bare form. So the comparison holds if the trimmed reading equals the value
/// either bare or single-quoted — the difference is `gsettings`' presentation, not a real one.
pub fn already_set(current: &str, want: &str) -> bool {
    let cur = current.trim();
    cur == want || cur.trim_matches('\'') == want
}

pub struct SettingBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
}

impl SettingBackendCore {
    pub fn new(executor: CommandExecutor) -> Self {
        Self {
            executor,
            name: "setting".to_string(),
        }
    }

    /// Which store the running desktop uses. `gsettings` on `PATH` is the test for GNOME's;
    /// KDE's `kwriteconfig` has no schema to read a current value cleanly and is deliberately
    /// not adapted yet (K7).
    pub fn detect_store(&self) -> SettingStore {
        if cfg!(target_os = "linux") && self.executor.command_exists_sync("gsettings") {
            SettingStore::GSettings
        } else {
            SettingStore::None
        }
    }

    fn split(spec_name: &str) -> Result<(&str, &str)> {
        crate::config::grammar::statement::split_setting(spec_name).ok_or_else(|| {
            Error::Validation(format!("`{}` is not `SCHEMA/KEY`", spec_name))
        })
    }

    fn no_adapter(&self, name: &str) -> Error {
        Error::Validation(format!(
            "`setting:{}` — no settings adapter for this desktop. \
             `setting:` writes GNOME's store via `gsettings`; nothing here provides one, and a \
             key silently unapplied is worse than an error.",
            name
        ))
    }
}

#[async_trait]
impl BackendCore for SettingBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.detect_store() != SettingStore::None
    }
    fn needs_root(&self) -> bool {
        // A desktop setting is per-user, written as the user, never with sudo.
        false
    }
}

#[async_trait]
impl MetadataProvider for SettingBackendCore {
    async fn get_dependencies(&self, _name: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub struct SettingInstallable {
    pub core: Arc<SettingBackendCore>,
}

#[async_trait]
impl Installable for SettingInstallable {
    async fn install(&self, specs: &[PackageSpec], _sudo: bool) -> Result<()> {
        let store = self.core.detect_store();
        for spec in specs {
            let (schema, key) = SettingBackendCore::split(&spec.name)?;
            let want = spec.options.get("value").map(String::as_str).ok_or_else(|| {
                Error::Validation(format!("`setting:{}` has no value", spec.name))
            })?;

            let Some((prog, args)) = write_command(store, schema, key, want) else {
                return Err(self.core.no_adapter(&spec.name));
            };

            // Read before write: only touch the store when it does not already hold `want`,
            // so a settled sync runs no command at all.
            if let Some((rprog, rargs)) = read_command(store, schema, key) {
                let refs: Vec<&str> = rargs.iter().map(String::as_str).collect();
                if let Ok(current) = self.core.executor.run_output(&rprog, &refs, false).await {
                    if already_set(&current, want) {
                        continue;
                    }
                }
            }

            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run(&prog, &refs, false).await?;
            info!("Setting {}/{} = {}", schema, key, want);
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        let store = self.core.detect_store();
        for name in names {
            let (schema, key) = SettingBackendCore::split(name)?;
            // A store with no adapter never held the value, so there is nothing to reset and
            // nothing to fail on.
            if let Some((prog, args)) = reset_command(store, schema, key) {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                self.core.executor.run(&prog, &refs, false).await?;
                info!("Setting {}/{} reset to its default", schema, key);
            }
        }
        Ok(())
    }
}

pub struct SettingQueryable {
    pub core: Arc<SettingBackendCore>,
}

#[async_trait]
impl Queryable for SettingQueryable {
    async fn list_installed(&self) -> Result<Vec<Package>> {
        // A setting is not software with an inventory: LiNix knows the keys it declares, not
        // every key the store holds. Nothing to enumerate.
        Ok(vec![])
    }
    async fn list_manual(&self) -> Result<Vec<Package>> {
        Ok(vec![])
    }
    async fn info(&self, _name: &str) -> Result<Option<Package>> {
        Ok(None)
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    _cfg: &crate::config::Config,
) {
    let core = Arc::new(SettingBackendCore::new(exec.duplicate()));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(SettingInstallable { core: core.clone() }))
            .with_queryable(Arc::new(SettingQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsettings_reads_writes_and_resets_a_key() {
        let (p, a) = read_command(SettingStore::GSettings, "org.gnome.x", "k").unwrap();
        assert_eq!(p, "gsettings");
        assert_eq!(a, vec!["get", "org.gnome.x", "k"]);

        let (_, a) = write_command(SettingStore::GSettings, "org.gnome.x", "k", "v").unwrap();
        assert_eq!(a, vec!["set", "org.gnome.x", "k", "v"]);

        let (_, a) = reset_command(SettingStore::GSettings, "org.gnome.x", "k").unwrap();
        assert_eq!(a, vec!["reset", "org.gnome.x", "k"]);
    }

    /// `gsettings` dispatches on argv[1] by hand, so a `--` is read as the command name and
    /// the call fails before it reaches the schema.
    #[test]
    fn gsettings_deliberately_gets_no_option_terminator() {
        assert!(!crate::core::argv::terminates_options("gsettings"));
        for (_, args) in [
            read_command(SettingStore::GSettings, "org.gnome.x", "k").unwrap(),
            write_command(SettingStore::GSettings, "org.gnome.x", "k", "v").unwrap(),
            reset_command(SettingStore::GSettings, "org.gnome.x", "k").unwrap(),
        ] {
            assert!(!args.iter().any(|a| a == "--"), "{:?}", args);
        }
    }

    #[test]
    fn no_adapter_plans_no_command() {
        assert!(read_command(SettingStore::None, "s", "k").is_none());
        assert!(write_command(SettingStore::None, "s", "k", "v").is_none());
        assert!(reset_command(SettingStore::None, "s", "k").is_none());
    }

    #[test]
    fn a_quoted_reading_equals_the_bare_value() {
        // `gsettings get` prints `'prefer-dark'`; the user wrote `prefer-dark`. Same value.
        assert!(already_set("'prefer-dark'", "prefer-dark"));
        assert!(already_set("prefer-dark\n", "prefer-dark"));
        assert!(already_set("true", "true"));
    }

    #[test]
    fn a_different_value_is_not_already_set() {
        assert!(!already_set("'prefer-light'", "prefer-dark"));
        assert!(!already_set("false", "true"));
    }
}
