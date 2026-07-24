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
//!
//! **An adapter is a row in a table, and the built-ins are rows in it** (K17, ruled
//! 2026-07-23). A closed enum cannot mean *everywhere* — every new desktop would be a LiNix
//! release, and the machine running the unusual store is the one that cannot wait for one. So
//! `gsettings` is not special: it is `setting_stores.toml`, shipped, parsed by the loader a
//! user's own row goes through. An adapter mechanism the built-ins bypass is one nobody has
//! tested.

use crate::core::{BackendCore, CommandExecutor, Error, Installable, MetadataProvider, Package,
    PackageSpec, Queryable, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use tracing::{info, warn};

/// One settings store: how to tell it is here, and the three commands that read, write and
/// reset a key. `{schema}`, `{key}` and `{value}` are substituted into the argv.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingAdapter {
    /// What this store is called, in errors and in `doctor`.
    pub name: String,
    /// The command whose presence on PATH means this machine runs this store.
    pub detect: String,
    /// Restrict the row to one OS (`std::env::consts::OS`). Absent means any.
    #[serde(default)]
    pub os: Option<String>,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub reset: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingStoreFile {
    #[serde(default)]
    pub setting_store: Vec<SettingAdapter>,
}

/// The rows LiNix ships. Parsed rather than constructed, so the shipped adapters exercise the
/// same loader a user's row does.
const BUILTIN_STORES: &str = include_str!("setting_stores.toml");

impl SettingAdapter {
    fn fill(args: &[String], schema: &str, key: &str, value: &str) -> Vec<String> {
        args.iter()
            .map(|a| {
                a.replace("{schema}", schema)
                    .replace("{key}", key)
                    .replace("{value}", value)
            })
            .collect()
    }

    /// Split filled argv into the program and its arguments. A row with an empty command is
    /// refused at load, so this never has to invent one.
    fn command(args: &[String], schema: &str, key: &str, value: &str) -> (String, Vec<String>) {
        let filled = Self::fill(args, schema, key, value);
        let (prog, rest) = filled.split_first().expect("an empty adapter command was loaded");
        (prog.clone(), rest.to_vec())
    }

    pub fn read_command(&self, schema: &str, key: &str) -> (String, Vec<String>) {
        Self::command(&self.read, schema, key, "")
    }

    pub fn write_command(&self, schema: &str, key: &str, value: &str) -> (String, Vec<String>) {
        Self::command(&self.write, schema, key, value)
    }

    pub fn reset_command(&self, schema: &str, key: &str) -> (String, Vec<String>) {
        Self::command(&self.reset, schema, key, "")
    }

    /// A row LiNix will act on: it names itself, it can be detected, and all three commands
    /// are present. A store that can be written but not read cannot answer X.4's
    /// read-before-write question, so it is not an adapter — it is a command that runs every
    /// sync, which is the thing `setting:` exists not to be.
    fn is_usable(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("it has no `name`");
        }
        if self.detect.trim().is_empty() {
            return Some("it has no `detect` command");
        }
        for (label, args) in [("read", &self.read), ("write", &self.write), ("reset", &self.reset)] {
            if args.is_empty() {
                return Some(match label {
                    "read" => "its `read` command is empty — a store LiNix cannot read is one it \
                               would write on every sync",
                    "write" => "its `write` command is empty",
                    _ => "its `reset` command is empty — removing a declaration would do nothing",
                });
            }
        }
        None
    }

    fn applies_to_this_os(&self) -> bool {
        match &self.os {
            Some(os) => os.eq_ignore_ascii_case(std::env::consts::OS),
            None => true,
        }
    }
}

/// Every adapter this machine knows: the shipped rows, then the user's. A user row cannot take
/// a shipped name — that is the same rule custom backends follow, and for the same reason.
pub fn adapters(user_rows: Vec<SettingAdapter>) -> Vec<SettingAdapter> {
    let shipped: SettingStoreFile = toml::from_str(BUILTIN_STORES)
        .expect("the shipped setting_stores.toml must parse");
    let mut out: Vec<SettingAdapter> = Vec::new();
    for row in shipped.setting_store.into_iter().chain(user_rows) {
        if let Some(why) = row.is_usable() {
            warn!("ignoring the `{}` settings adapter: {}.", row.name, why);
            continue;
        }
        if out.iter().any(|a| a.name.eq_ignore_ascii_case(&row.name)) {
            warn!(
                "ignoring a second settings adapter named `{}`: the first one wins.",
                row.name
            );
            continue;
        }
        out.push(row);
    }
    out
}

/// Read the user's `[[setting_store]]` rows out of the config repo's `custom_backends.toml`.
///
/// The same file, because it is the same question — *what have you taught this LiNix?* — and
/// so an adapter inherits the approval the definitions in that file already carry (7a/II.12)
/// rather than needing a second ledger, a second loader and a second thing to forget.
pub fn user_adapters(cfg: &crate::config::Config) -> Vec<SettingAdapter> {
    let layout = cfg.layout();
    match crate::backends::onboarder::read_approved_definitions(
        &layout.custom_backends_file(),
        &layout.locks_dir(),
    ) {
        Some(body) => match toml::from_str::<SettingStoreFile>(&body) {
            Ok(f) => f.setting_store,
            Err(e) => {
                warn!("ignoring the settings adapters in custom_backends.toml: {}", e);
                Vec::new()
            }
        },
        None => Vec::new(),
    }
}

/// Pure: whether the store already holds `want`, given what the read command printed.
///
/// `gsettings` quotes string values (`'prefer-dark'`) and prints bare booleans/enums, and a
/// user writes the bare form. So the comparison holds if the trimmed reading equals the value
/// either bare or quoted — the difference is the store's presentation, not a real one.
pub fn already_set(current: &str, want: &str) -> bool {
    let cur = current.trim();
    cur == want || cur.trim_matches(['\'', '"']) == want
}

pub struct SettingBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    pub adapters: Vec<SettingAdapter>,
}

impl SettingBackendCore {
    pub fn new(executor: CommandExecutor, adapters: Vec<SettingAdapter>) -> Self {
        Self {
            executor,
            name: "setting".to_string(),
            adapters,
        }
    }

    /// The adapter for the store this machine is running: the first row whose `detect`
    /// command is here. `None` makes every `setting:` line an error naming the key — a key
    /// silently unapplied is worse than a refusal, because the whole point is that the file
    /// is the truth (X.4).
    pub fn adapter(&self) -> Option<&SettingAdapter> {
        self.adapters.iter().find(|a| {
            a.applies_to_this_os() && self.executor.command_exists_sync(&a.detect)
        })
    }

    fn split(spec_name: &str) -> Result<(&str, &str)> {
        crate::config::grammar::statement::split_setting(spec_name).ok_or_else(|| {
            Error::Validation(format!("`{}` is not `SCHEMA/KEY`", spec_name))
        })
    }

    fn no_adapter(&self, name: &str) -> Error {
        let known: Vec<&str> = self.adapters.iter().map(|a| a.name.as_str()).collect();
        Error::Validation(format!(
            "`setting:{}` — no settings adapter matches this machine. LiNix looked for {}, and \
             found none of them. Add a `[[setting_store]]` row to `custom_backends.toml` \
             naming the command your store is driven by; a key silently unapplied is worse \
             than an error.",
            name,
            known.join(", ")
        ))
    }
}

#[async_trait]
impl BackendCore for SettingBackendCore {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_available(&self) -> bool {
        self.adapter().is_some()
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
        for spec in specs {
            let (schema, key) = SettingBackendCore::split(&spec.name)?;
            let want = spec.options.get("value").map(String::as_str).ok_or_else(|| {
                Error::Validation(format!("`setting:{}` has no value", spec.name))
            })?;

            let Some(adapter) = self.core.adapter() else {
                return Err(self.core.no_adapter(&spec.name));
            };

            // Read before write: only touch the store when it does not already hold `want`,
            // so a settled sync runs no command at all.
            let (rprog, rargs) = adapter.read_command(schema, key);
            let refs: Vec<&str> = rargs.iter().map(String::as_str).collect();
            if let Ok(current) = self.core.executor.run_output(&rprog, &refs, false).await {
                if already_set(&current, want) {
                    continue;
                }
            }

            let (prog, args) = adapter.write_command(schema, key, want);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run(&prog, &refs, false).await?;
            info!("Setting {}/{} = {}", schema, key, want);
        }
        Ok(())
    }

    async fn remove(&self, names: &[String], _sudo: bool) -> Result<()> {
        for name in names {
            let (schema, key) = SettingBackendCore::split(name)?;
            // A store with no adapter never held the value, so there is nothing to reset and
            // nothing to fail on.
            let Some(adapter) = self.core.adapter() else {
                continue;
            };
            let (prog, args) = adapter.reset_command(schema, key);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core.executor.run(&prog, &refs, false).await?;
            info!("Setting {}/{} reset to its default", schema, key);
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
    cfg: &crate::config::Config,
) {
    let core = Arc::new(SettingBackendCore::new(
        exec.duplicate(),
        adapters(user_adapters(cfg)),
    ));
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

    fn gsettings() -> SettingAdapter {
        adapters(vec![])
            .into_iter()
            .find(|a| a.name == "gsettings")
            .expect("the shipped table must carry gsettings")
    }

    /// The shipped rows go through the loader, not around it. If this fails, the built-in
    /// adapters have stopped being the thing a user's row is.
    #[test]
    fn the_shipped_table_parses_and_carries_gsettings() {
        let all = adapters(vec![]);
        assert_eq!(all.len(), 1, "{:?}", all.iter().map(|a| &a.name).collect::<Vec<_>>());
        assert_eq!(all[0].detect, "gsettings");
        assert_eq!(all[0].os.as_deref(), Some("linux"));
    }

    #[test]
    fn gsettings_reads_writes_and_resets_a_key() {
        let a = gsettings();
        let (p, args) = a.read_command("org.gnome.x", "k");
        assert_eq!(p, "gsettings");
        assert_eq!(args, vec!["get", "org.gnome.x", "k"]);

        let (_, args) = a.write_command("org.gnome.x", "k", "v");
        assert_eq!(args, vec!["set", "org.gnome.x", "k", "v"]);

        let (_, args) = a.reset_command("org.gnome.x", "k");
        assert_eq!(args, vec!["reset", "org.gnome.x", "k"]);
    }

    /// `gsettings` dispatches on argv[1] by hand, so a `--` is read as the command name and
    /// the call fails before it reaches the schema.
    #[test]
    fn gsettings_deliberately_gets_no_option_terminator() {
        assert!(!crate::core::argv::terminates_options("gsettings"));
        let a = gsettings();
        for (_, args) in [
            a.read_command("org.gnome.x", "k"),
            a.write_command("org.gnome.x", "k", "v"),
            a.reset_command("org.gnome.x", "k"),
        ] {
            assert!(!args.iter().any(|x| x == "--"), "{:?}", args);
        }
    }

    fn row(name: &str, detect: &str) -> SettingAdapter {
        SettingAdapter {
            name: name.into(),
            detect: detect.into(),
            os: None,
            read: vec![detect.into(), "read".into(), "{schema}".into(), "{key}".into()],
            write: vec![
                detect.into(),
                "write".into(),
                "{schema}".into(),
                "{key}".into(),
                "{value}".into(),
            ],
            reset: vec![detect.into(), "reset".into(), "{schema}".into(), "{key}".into()],
        }
    }

    /// K17: a store LiNix has never heard of is a row, not a release. This is the whole point
    /// of the ruling — the adapter below is driven from a definition, through the same code
    /// path the shipped one uses.
    #[test]
    fn a_store_with_no_compiled_in_support_is_driven_from_a_row() {
        let all = adapters(vec![row("kde", "kwriteconfig6")]);
        let kde = all.iter().find(|a| a.name == "kde").expect("the user row loaded");
        let (prog, args) = kde.write_command("kdeglobals/General", "ColorScheme", "BreezeDark");
        assert_eq!(prog, "kwriteconfig6");
        assert_eq!(
            args,
            vec!["write", "kdeglobals/General", "ColorScheme", "BreezeDark"]
        );
    }

    #[test]
    fn a_user_row_cannot_take_a_shipped_name() {
        let all = adapters(vec![row("gsettings", "impostor")]);
        let g = all.iter().find(|a| a.name == "gsettings").unwrap();
        assert_eq!(g.detect, "gsettings", "a user row shadowed the shipped one");
        assert_eq!(all.len(), 1);
    }

    /// A row that cannot be read is not an adapter: X.4's read-before-write is what makes
    /// `setting:` a declaration rather than a command that runs every sync.
    #[test]
    fn a_row_missing_a_command_is_refused_rather_than_half_used() {
        let mut unreadable = row("half", "halfctl");
        unreadable.read = vec![];
        assert!(!adapters(vec![unreadable]).iter().any(|a| a.name == "half"));

        let mut unresettable = row("half", "halfctl");
        unresettable.reset = vec![];
        assert!(!adapters(vec![unresettable]).iter().any(|a| a.name == "half"));

        let mut nameless = row("", "halfctl");
        nameless.name = String::new();
        assert_eq!(adapters(vec![nameless]).len(), 1);
    }

    #[test]
    fn a_row_for_another_os_is_not_this_machines_store() {
        let mut elsewhere = row("elsewhere", "cmd");
        elsewhere.os = Some("plan9".into());
        assert!(!elsewhere.applies_to_this_os());
        let mut here = row("here", "cmd");
        here.os = Some(std::env::consts::OS.to_string());
        assert!(here.applies_to_this_os());
    }

    #[test]
    fn a_quoted_reading_equals_the_bare_value() {
        // `gsettings get` prints `'prefer-dark'`; the user wrote `prefer-dark`. Same value.
        assert!(already_set("'prefer-dark'", "prefer-dark"));
        assert!(already_set("prefer-dark\n", "prefer-dark"));
        assert!(already_set("true", "true"));
        // A registry read prints `"Dark"`; the same rule covers it.
        assert!(already_set("\"Dark\"", "Dark"));
    }

    #[test]
    fn a_different_value_is_not_already_set() {
        assert!(!already_set("'prefer-light'", "prefer-dark"));
        assert!(!already_set("false", "true"));
    }

    /// The refusal names what LiNix looked for, so a machine running an unlisted store learns
    /// what to write a row about rather than only that it failed.
    #[test]
    fn the_refusal_names_the_stores_it_looked_for() {
        let core = SettingBackendCore::new(CommandExecutor::new(true, false), adapters(vec![]));
        let msg = core.no_adapter("org.gnome.desktop.interface/color-scheme").to_string();
        assert!(msg.contains("gsettings"), "{}", msg);
        assert!(msg.contains("setting_store"), "{}", msg);
    }
}
