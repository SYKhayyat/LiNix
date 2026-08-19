use crate::core::{
    BackendCore, CommandExecutor, Installable, MetadataProvider, Package, PackageSpec, Queryable,
    Result, Searchable, Upgradable,
};
use crate::model::scope::Scope;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info};

/// The name every flatpak verb takes the exclusive lock under.
///
/// Asked of `stale_lock`, which owns the table of which programs share one package
/// database, rather than spelled as a literal here — a second copy of that table is
/// exactly what its own doc says goes stale. A verb that changes the manager takes
/// the manager's lock; install and remove already did, and `update` and the cache
/// cleaners did not.
fn lock_key() -> &'static str {
    crate::app::stale_lock::lock_key("flatpak")
}

pub struct FlatpakBackendCore {
    pub executor: CommandExecutor,
    pub name: String,
    /// Whose installation flatpak acts on. Parsed once at registration: two call sites reading
    /// the raw map is two chances to disagree about what the string meant.
    pub scope: Scope,
}

impl FlatpakBackendCore {
    pub fn new(executor: CommandExecutor, scope: Scope) -> Self {
        Self {
            executor,
            name: "flatpak".to_string(),
            scope,
        }
    }

    /// **These go AFTER the subcommand, and every caller here does that.**
    ///
    /// `flatpak(1)` is `flatpak [OPTION…] COMMAND`, and its top-level options are `--help`,
    /// `--version`, `--verbose` and friends. `--user`/`--system` belong to the *commands* —
    /// `flatpak-install(1)` is `flatpak install [OPTION…] REF…` — so `flatpak --system install`
    /// is rejected before flatpak decides what it was being asked to do.
    ///
    /// Every argv this backend built had the flag in front for as long as the backend has
    /// existed, and nothing noticed, because flatpak is one of the managers no harness has ever
    /// driven: it needs a session bus, so the container matrix names it as argv-tested-only. An
    /// argv test proves a command line was constructed, not that the tool accepts it. What found
    /// it was `argv_drift_tests`, which asks the real flatpak on the tools image whether it
    /// documents the flags Shall passes, and got back
    ///
    /// ```text
    /// `flatpak  --system` — the tool says: error: unknown option --system
    /// ```
    ///
    /// — with the subcommand slot empty, which is exactly where the flag was going.
    pub fn scope_args(&self) -> Vec<&str> {
        match self.scope {
            Scope::User => vec!["--user"],
            Scope::System => vec!["--system"],
        }
    }

    /// Every installed application, with the branch it is on.
    ///
    /// One reader, because the install path needs the same answer the query path does: it has to
    /// know which branch an app is on *before* it adds another one, and a second listing here
    /// would be a second chance to disagree with the one the planner read.
    async fn installed_refs(&self) -> Result<Vec<Package>> {
        let out = self
            .executor
            .run_output(
                "flatpak",
                &["list", "--app", "--columns=application,version,branch"],
                false,
            )
            .await?;
        Ok(parse_flatpak_list(&out)?)
    }

    /// The `[backend_settings.flatpak]` block as a scope, or the message that says what to
    /// write instead.
    ///
    /// `--user` and `--system` are a value, not a flag: the argv needs the word itself, which
    /// is why the key is `scope` and not the boolean it used to be.
    pub fn scope_from_settings(settings: Option<&HashMap<String, String>>) -> Result<Scope> {
        let Some(settings) = settings else {
            return Ok(FLATPAK_DEFAULT_SCOPE);
        };
        if settings.contains_key("user") {
            return Err(crate::core::Error::Config(format!(
                "`[backend_settings.flatpak]` sets `user`, which flatpak no longer reads. Write \
                 `scope = \"user\"` or `scope = \"system\"` instead — the flag needs the word, \
                 not a boolean. Default is `{FLATPAK_DEFAULT_SCOPE}`."
            )));
        }
        let Some(written) = settings.get("scope") else {
            return Ok(FLATPAK_DEFAULT_SCOPE);
        };
        Scope::parse(written).ok_or_else(|| {
            crate::core::Error::Config(format!(
                "`[backend_settings.flatpak]` has `scope = \"{written}\"`. Scope is {}. \
                 Omitting it means `{FLATPAK_DEFAULT_SCOPE}`.",
                Scope::vocabulary()
            ))
        })
    }
}

/// What `flatpak` itself does when neither flag is passed.
pub const FLATPAK_DEFAULT_SCOPE: Scope = Scope::System;

/// `flatpak list --columns=application,version,branch` — TAB-separated, like `flatpak search`.
///
/// **Tabs, not whitespace.** Most flathub apps carry no version, and flatpak keeps an empty
/// *middle* column as an empty field (`org.gimp.GIMP\t\tstable`) while dropping trailing ones.
/// Split on whitespace and the branch of every versionless app is read as its version — measured
/// against flathub, `--columns=application,version,branch` beside `application,branch,version`.
///
/// **A second row for an application erases its branch.** flatpak installs branches side by side
/// and the listing has no column saying which one is current — `--columns=help` offers none, and
/// the binary carries no such word. An app on two branches is therefore a branch Shall cannot
/// read, and D13's rule is that an unreadable value is left alone: guessing one of the two would
/// schedule a switch on every sync for ever.
fn parse_flatpak_list(output: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates = crate::parsers::data_lines(&clean);
    let mut order: Vec<String> = Vec::new();
    let mut apps: HashMap<String, Package> = HashMap::new();
    for line in &candidates {
        let cols: Vec<&str> = line.split('\t').map(str::trim).collect();
        let Some(name) = cols.first().copied().filter(|s| !s.is_empty()) else {
            continue;
        };
        if let Some(seen) = apps.get_mut(name) {
            seen.properties.remove("channel");
            continue;
        }
        let mut p = Package::new(name, "flatpak");
        p.version = cols
            .get(1)
            .filter(|s| !s.is_empty())
            .map(|s| (*s).to_string());
        if let Some(branch) = cols.get(2).filter(|s| !s.is_empty()) {
            p.properties
                .insert("channel".to_string(), (*branch).to_string());
        }
        order.push(name.to_string());
        apps.insert(name.to_string(), p);
    }
    let found = order.into_iter().filter_map(|n| apps.remove(&n)).collect();
    or_unrecognised("flatpak", found, &candidates)
}

#[async_trait]
impl BackendCore for FlatpakBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        // No per-backend cache: the executor memoises every PATH lookup now, which dedupes
        // across the backends that probe the same program too. One backend having its own
        // `OnceCell` while the other forty-four re-probed is exactly the "two of everything"
        // this repo removes.
        self.executor.command_exists_sync("flatpak")
    }
    fn probes(&self) -> Vec<String> {
        vec!["flatpak".into()]
    }

    fn needs_root(&self) -> bool {
        // A user-scoped install writes under `$HOME` and must not be run through sudo.
        self.scope == Scope::System
    }
}

#[async_trait]
impl MetadataProvider for FlatpakBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let mut final_args: Vec<String> = vec!["info".to_string()];
        final_args.extend(self.scope_args().into_iter().map(str::to_string));
        final_args.push("--show-metadata".to_string());
        crate::core::argv::push_names(&mut final_args, "flatpak", [name]);
        let arg_refs: Vec<&str> = final_args.iter().map(String::as_str).collect();

        // Flatpak metadata contains a [Extension] or [Runtime] section.
        // We look for 'runtime=' which is the primary transitive dependency.
        let output = self
            .executor
            .run_output("flatpak", &arg_refs, false)
            .await?;
        let mut deps = Vec::new();

        for line in output.lines() {
            if let Some(runtime) = line.strip_prefix("runtime=") {
                deps.push(runtime.trim().to_string());
            }
        }

        Ok(deps)
    }
}

pub struct FlatpakInstallable {
    pub core: Arc<FlatpakBackendCore>,
}

impl FlatpakInstallable {
    /// Which branch each `@channel` app is on right now, read before the install so the switch
    /// after it can tell *added a second branch* from *installed the first*. Empty — and the
    /// listing unread — when no line asks for a channel.
    async fn branches_now(&self, specs: &[PackageSpec]) -> Result<HashMap<String, String>> {
        if !specs.iter().any(|s| s.options.one("channel").is_some()) {
            return Ok(HashMap::new());
        }
        Ok(self
            .core
            .installed_refs()
            .await?
            .into_iter()
            .filter_map(|p| {
                p.properties
                    .get("channel")
                    .map(|c| (p.name.clone(), c.clone()))
            })
            .collect())
    }
}

/// A flatpak ref is `name/arch/branch`. The arch slot stays empty so flatpak keeps choosing it
/// from the machine; writing `name/branch` would be read as an architecture, not a branch.
fn install_ref(spec: &PackageSpec) -> String {
    match spec.options.one("channel") {
        Some(channel) => format!("{}//{}", spec.name, channel),
        None => spec.name.clone(),
    }
}

/// `--or-update`, because a ref that is already there is not an error to Shall.
///
/// `flatpak install` answers `Error: <ref> already installed` and exits non-zero. Every other
/// path here can hand it a ref it already has — a `@channel` whose drift is repaired by the
/// same sync that read it, a package adopted between the plan and the apply — and each one of
/// those used to fail the whole transaction over a machine that was already in the declared
/// state. `--or-update` is flatpak's own answer to that: *"Update install if already
/// installed."*
fn install_argv(scope: &[&str], specs: &[PackageSpec]) -> Vec<String> {
    let mut args: Vec<String> = vec!["install".to_string()];
    args.extend(scope.iter().map(|s| s.to_string()));
    args.extend([
        "-y".to_string(),
        "--noninteractive".to_string(),
        "--or-update".to_string(),
    ]);
    let names: Vec<String> = specs.iter().map(install_ref).collect();
    crate::core::argv::push_names(&mut args, "flatpak", names);
    args
}

/// `flatpak make-current <app> <branch>`, the only thing that moves an app from one branch to
/// another.
fn make_current_argv(scope: &[&str], name: &str, branch: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["make-current".to_string()];
    args.extend(scope.iter().map(|s| s.to_string()));
    crate::core::argv::push_names(&mut args, "flatpak", [name, branch]);
    args
}

#[async_trait]
impl Installable for FlatpakInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        let before = self.branches_now(specs).await?;

        let args = install_argv(&self.core.scope_args(), specs);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

        info!("Flatpak: Installing {} package(s)...", specs.len());
        self.core
            .executor
            .run_exclusive(lock_key(), "flatpak", &arg_refs, sudo)
            .await?;

        // **flatpak has no channel switch.** `snap refresh --channel=` moves a snap; installing
        // `app//beta` next to `app//stable` moves nothing — flatpak keeps both and the launcher
        // still runs the old one. `make-current` is what points it at the declared branch, and
        // it has to happen here: the listing carries no current-branch column, so no later sync
        // can see that this step was skipped.
        for spec in specs {
            let Some(want) = spec.options.one("channel") else {
                continue;
            };
            let Some(had) = before.get(&spec.name) else {
                continue;
            };
            // Compared the way the planner compares it, through the same function. Two answers
            // to "is this the branch the line asked for" is how a switch fires on a machine the
            // planner called converged.
            use crate::backends::capability::channel_risk;
            if channel_risk(had) == channel_risk(want) {
                continue;
            }
            info!(
                "Flatpak: Switching {} from branch {} to {}...",
                spec.name, had, want
            );
            let args = make_current_argv(&self.core.scope_args(), &spec.name, want);
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.core
                .executor
                .run_exclusive(lock_key(), "flatpak", &arg_refs, sudo)
                .await?;
        }
        Ok(())
    }

    async fn remove(
        &self,
        names: &[String],
        sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }

        let mut args: Vec<String> = vec!["uninstall".to_string()];
        args.extend(self.core.scope_args().into_iter().map(str::to_string));
        args.extend(["-y".to_string(), "--noninteractive".to_string()]);
        crate::core::argv::push_names(&mut args, "flatpak", names);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

        info!("Flatpak: Removing {} package(s)...", names.len());
        self.core
            .executor
            .run_exclusive(lock_key(), "flatpak", &arg_refs, sudo)
            .await?;
        Ok(())
    }
}

pub struct FlatpakQueryable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Queryable for FlatpakQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        self.core.installed_refs().await
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        self.list_installed().await
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        Ok(all.iter().find(|p| p.name == name).cloned())
    }
}

pub struct FlatpakSearchable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Searchable for FlatpakSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        let mut args = vec!["search".to_string()];
        crate::core::argv::push_names(&mut args, "flatpak", [query]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self
            .core
            .executor
            .search_output("flatpak", &arg_refs, false)
            .await?;
        Ok(parse_flatpak_search(&output))
    }
}

/// Parse `flatpak search <q>` => TAB-separated columns:
/// Name \t Description \t Application ID \t Version \t Branch \t Remotes.
/// The Application ID is the installable identifier, so prefer it as the name.
fn parse_flatpak_search(output: &str) -> Vec<Package> {
    let mut results = Vec::new();
    for line in sanitize(output).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').map(|c| c.trim()).collect();
        let display_name = cols.first().copied().unwrap_or("").trim();
        let app_id = cols
            .get(2)
            .copied()
            .filter(|s| !s.is_empty())
            .unwrap_or(display_name);
        if app_id.is_empty() {
            continue;
        }
        let mut p = Package::new(app_id, "flatpak");
        if let Some(desc) = cols.get(1).filter(|s| !s.is_empty()) {
            p.properties
                .insert("description".to_string(), desc.to_string());
        }
        if let Some(ver) = cols.get(3).filter(|s| !s.is_empty()) {
            p.version = Some(ver.to_string());
        }
        results.push(p);
    }
    results
}

pub struct FlatpakUpgradable {
    pub core: Arc<FlatpakBackendCore>,
}

#[async_trait]
impl Upgradable for FlatpakUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        // Must pass -y --noninteractive (like install/upgrade/clean_orphans), otherwise an
        // automated run blocks on flatpak's interactive confirmation prompt.
        let mut args = vec!["update"];
        args.extend(self.core.scope_args());
        args.extend(["-y", "--noninteractive"]);
        debug!("Flatpak: Refreshing remotes...");
        self.core
            .executor
            .run_exclusive(lock_key(), "flatpak", &args, sudo)
            .await?;
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        let mut args = vec!["update"];
        args.extend(self.core.scope_args());
        args.extend(["-y", "--noninteractive"]);
        info!("Flatpak: Upgrading all applications...");
        self.core
            .executor
            .run_exclusive(lock_key(), "flatpak", &args, sudo)
            .await?;
        Ok(())
    }
}

pub fn register(
    reg: &mut crate::backends::BackendRegistry,
    exec: &CommandExecutor,
    cfg: &crate::config::Config,
) {
    // A scope this backend cannot read is not a default to fall back on: `--system` where the
    // user asked for `--user` installs for every account and needs root to do it.
    let scope = match FlatpakBackendCore::scope_from_settings(cfg.backend_settings.get("flatpak")) {
        Ok(scope) => scope,
        Err(e) => {
            tracing::warn!("flatpak: {e}");
            return;
        }
    };
    let core = Arc::new(FlatpakBackendCore::new(exec.clone(), scope));
    reg.register(Arc::new(
        crate::core::BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(FlatpakInstallable { core: core.clone() }))
            .with_queryable(Arc::new(FlatpakQueryable { core: core.clone() }))
            .with_searchable(Arc::new(FlatpakSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(FlatpakUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatpak_search_prefers_app_id() {
        // Name \t Description \t AppID \t Version \t Branch \t Remotes
        let out = "Blender\tFree 3D suite\torg.blender.Blender\t4.0\tstable\tflathub\n\
                   GIMP\tImage editor\torg.gimp.GIMP\t2.10\tstable\tflathub\n";
        let pkgs = parse_flatpak_search(out);
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "org.blender.Blender");
        assert_eq!(pkgs[0].version.as_deref(), Some("4.0"));
        assert_eq!(
            pkgs[0].properties.get("description").map(String::as_str),
            Some("Free 3D suite")
        );
    }

    fn spec_with(name: &str, options: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: name.to_string(),
            backend: "flatpak".to_string(),
            options: options
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Both spellings, the absent case, and the absent *block* — the argv difference is which
    /// installation gets written, so a wrong answer here is a package installed for the wrong
    /// people.
    #[test]
    fn every_way_of_writing_the_scope_reaches_the_argv() {
        for (written, expect, flag) in [
            (Some("user"), Scope::User, "--user"),
            (Some("system"), Scope::System, "--system"),
            (None, FLATPAK_DEFAULT_SCOPE, "--system"),
        ] {
            let block = written.map(|w| settings(&[("scope", w)]));
            let scope = FlatpakBackendCore::scope_from_settings(block.as_ref()).unwrap();
            assert_eq!(scope, expect, "scope = {written:?}");
            let core = FlatpakBackendCore::new(CommandExecutor::new(false, false), scope);
            assert_eq!(core.scope_args(), vec![flag]);
            assert_eq!(
                core.needs_root(),
                scope == Scope::System,
                "a user-scoped install writes under $HOME and must not ask for root"
            );
        }
        assert_eq!(
            FlatpakBackendCore::scope_from_settings(None).unwrap(),
            FLATPAK_DEFAULT_SCOPE,
            "no `[backend_settings.flatpak]` block at all"
        );
    }

    /// The old boolean is refused by name rather than ignored: a `user = "true"` that silently
    /// stopped meaning anything would install system-wide under a line asking for the opposite.
    #[test]
    fn the_boolean_this_key_used_to_be_is_refused_and_names_its_replacement() {
        let err = FlatpakBackendCore::scope_from_settings(Some(&settings(&[("user", "true")])))
            .unwrap_err()
            .to_string();
        assert!(err.contains("scope = \"user\""), "{err}");

        // Refused whatever it says — `user = "false"` meant `--system`, which is the default,
        // and accepting it in silence would teach that the key still works.
        assert!(
            FlatpakBackendCore::scope_from_settings(Some(&settings(&[("user", "false")]))).is_err()
        );
    }

    /// A scope nobody can parse is not a default to fall back on.
    #[test]
    fn a_scope_that_is_neither_word_is_refused() {
        for bad in ["User", "SYSTEM", "machine", "global", "true", ""] {
            let err = FlatpakBackendCore::scope_from_settings(Some(&settings(&[("scope", bad)])));
            assert!(err.is_err(), "`scope = \"{bad}\"` was accepted");
        }
    }

    #[test]
    fn flatpak_channel_becomes_the_branch_of_the_ref() {
        let spec = spec_with("org.gimp.GIMP", &[("channel", "beta")]);
        assert_eq!(install_ref(&spec), "org.gimp.GIMP//beta");
    }

    #[test]
    fn flatpak_without_a_channel_installs_the_bare_name() {
        let spec = spec_with("org.gimp.GIMP", &[]);
        assert_eq!(install_ref(&spec), "org.gimp.GIMP");
    }

    #[test]
    fn flatpak_refs_come_after_the_terminator() {
        let argv = install_argv(
            &["--system"],
            &[spec_with("org.gimp.GIMP", &[]), spec_with("--user", &[])],
        );
        assert_eq!(
            argv,
            [
                "install",
                "--system",
                "-y",
                "--noninteractive",
                "--or-update",
                "--",
                "org.gimp.GIMP",
                "--user"
            ]
        );
    }

    /// `flatpak install` calls an already-installed ref an error and exits non-zero — the
    /// binary carries `Error: %s%s%s already installed`. Every sync that repairs a `@channel`
    /// hands it a ref it may already have, so without `--or-update` the repair fails the
    /// transaction over a machine that was already in the declared state.
    #[test]
    fn an_already_installed_ref_is_not_an_error_to_shall() {
        let argv = install_argv(&["--user"], &[spec_with("org.gimp.GIMP", &[])]);
        assert!(
            argv.contains(&"--or-update".to_string()),
            "install must tolerate a ref that is already there: {argv:?}"
        );
    }

    #[test]
    fn a_branch_switch_names_the_app_and_the_branch_behind_the_terminator() {
        assert_eq!(
            make_current_argv(&["--user"], "org.gimp.GIMP", "beta"),
            ["make-current", "--user", "--", "org.gimp.GIMP", "beta"]
        );
    }

    /// The listing is TAB-separated and most flathub apps carry no version, so flatpak emits an
    /// empty *middle* field. Split on whitespace — which is what the shared list parser did —
    /// and `stable` is read as GIMP's version while its branch goes unread entirely.
    #[test]
    fn an_empty_version_column_does_not_slide_the_branch_into_it() {
        let out = "org.gimp.GIMP\t\tstable\norg.blender.Blender\t4.0\tbeta\n";
        let pkgs = parse_flatpak_list(out).expect("this fixture parses");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "org.gimp.GIMP");
        assert_eq!(pkgs[0].version, None);
        assert_eq!(
            pkgs[0].properties.get("channel").map(String::as_str),
            Some("stable")
        );
        assert_eq!(pkgs[1].version.as_deref(), Some("4.0"));
        assert_eq!(
            pkgs[1].properties.get("channel").map(String::as_str),
            Some("beta")
        );
    }

    /// D13: an app installed on two branches has no readable current branch — the listing has
    /// no column for it — so it reports none and the planner leaves it alone. Reporting either
    /// row would schedule a switch on every sync for ever.
    #[test]
    fn an_app_on_two_branches_reports_no_branch_at_all() {
        let out = "org.gimp.GIMP\t2.10\tstable\norg.gimp.GIMP\t2.99\tbeta\norg.blender.Blender\t4.0\tstable\n";
        let pkgs = parse_flatpak_list(out).expect("this fixture parses");
        assert_eq!(pkgs.len(), 2, "one row per application");
        assert_eq!(pkgs[0].name, "org.gimp.GIMP");
        assert_eq!(
            pkgs[0].properties.get("channel"),
            None,
            "two branches installed, and nothing says which one runs"
        );
        assert_eq!(
            pkgs[1].properties.get("channel").map(String::as_str),
            Some("stable"),
            "the single-branch app beside it is still readable"
        );
    }

    /// A backend over a mock that has been told nothing.
    fn scripted_without_a_listing() -> (
        Arc<FlatpakBackendCore>,
        Arc<crate::core::executor::MockExecutor>,
    ) {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        (
            Arc::new(FlatpakBackendCore::new(exec, FLATPAK_DEFAULT_SCOPE)),
            mock,
        )
    }

    /// A backend wired to a scripted `flatpak list`.
    fn scripted(
        listing: &str,
    ) -> (
        Arc<FlatpakBackendCore>,
        Arc<crate::core::executor::MockExecutor>,
    ) {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        mock.set_response(
            "flatpak list --app --columns=application,version,branch",
            Ok(crate::core::executor::DryRunOutput {
                stdout: listing.as_bytes().to_vec(),
                ..Default::default()
            }
            .into()),
        );
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        (
            Arc::new(FlatpakBackendCore::new(exec, FLATPAK_DEFAULT_SCOPE)),
            mock,
        )
    }

    /// **flatpak has no channel switch.** Installing `app//beta` beside `app//stable` moves
    /// nothing — flatpak keeps both, and the launcher still runs the branch it ran yesterday.
    /// Without `make-current` the sync reports a repaired channel over a machine that changed
    /// nothing a user could see.
    #[tokio::test]
    async fn switching_branch_installs_the_ref_and_then_points_the_app_at_it() {
        let (core, mock) = scripted("org.gimp.GIMP\t2.10\tstable\n");
        FlatpakInstallable { core }
            .install(&[spec_with("org.gimp.GIMP", &[("channel", "beta")])], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "flatpak list --app --columns=application,version,branch",
                "flatpak install --system -y --noninteractive --or-update -- org.gimp.GIMP//beta",
                "flatpak make-current --system -- org.gimp.GIMP beta",
            ]
        );
    }

    /// The first install of an app is not a switch: flatpak has only the one branch to run, and
    /// a `make-current` on every install would be a second command doing nothing.
    #[tokio::test]
    async fn a_first_install_on_a_declared_branch_is_not_a_switch() {
        let (core, mock) = scripted("");
        FlatpakInstallable { core }
            .install(&[spec_with("org.gimp.GIMP", &[("channel", "beta")])], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "flatpak list --app --columns=application,version,branch",
                "flatpak install --system -y --noninteractive --or-update -- org.gimp.GIMP//beta",
            ]
        );
    }

    /// An app already on the declared branch is not switched, and — the reason the listing is
    /// read at all — an app on *two* branches is not either: nothing says which one runs, so
    /// nothing here may claim to know (D13).
    #[tokio::test]
    async fn an_unreadable_or_matching_branch_issues_no_switch() {
        for listing in [
            "org.gimp.GIMP\t2.10\tbeta\n",
            "org.gimp.GIMP\t2.10\tstable\norg.gimp.GIMP\t2.99\tbeta\n",
        ] {
            let (core, mock) = scripted(listing);
            FlatpakInstallable { core }
                .install(&[spec_with("org.gimp.GIMP", &[("channel", "beta")])], false)
                .await
                .unwrap();
            assert!(
                !mock
                    .get_calls()
                    .await
                    .iter()
                    .any(|c| c.contains("make-current")),
                "listing {listing:?} produced a switch"
            );
        }
    }

    /// A plan with no `@channel` in it does not read the listing at all — the branch question is
    /// not asked of packages that never raised it.
    #[tokio::test]
    async fn an_install_that_declares_no_channel_never_asks_for_the_listing() {
        // No listing stub on purpose: registering one and asserting it went unused is what the
        // mock calls a belief the product disagreed with, so the absence *is* the assertion.
        let (core, mock) = scripted_without_a_listing();
        FlatpakInstallable { core }
            .install(&[spec_with("org.blender.Blender", &[])], false)
            .await
            .unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec!["flatpak install --system -y --noninteractive --or-update -- org.blender.Blender"]
        );
    }

    /// Bytes off a real flathub listing (`debian:12` container, flatpak 1.14.10): the columns
    /// are tab-separated and a trailing empty column is dropped rather than emitted.
    #[test]
    fn a_row_with_no_trailing_columns_is_still_a_package() {
        let pkgs = parse_flatpak_list("ai.jan.Jan\n").expect("this fixture parses");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "ai.jan.Jan");
        assert_eq!(pkgs[0].version, None);
        assert_eq!(pkgs[0].properties.get("channel"), None);
    }

    #[tokio::test]
    async fn flatpaks_other_name_carrying_commands_terminate_too() {
        let vfs = Arc::new(dashmap::DashMap::new());
        let mock = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
        let exec = CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(dashmap::DashMap::new()),
        );
        let core = Arc::new(FlatpakBackendCore::new(exec, FLATPAK_DEFAULT_SCOPE));

        FlatpakInstallable { core: core.clone() }
            .remove(
                &["org.gimp.GIMP".to_string()],
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .unwrap();
        FlatpakSearchable { core: core.clone() }
            .search("gimp")
            .await
            .unwrap();
        core.get_dependencies("org.gimp.GIMP").await.unwrap();

        assert_eq!(
            mock.get_calls().await,
            vec![
                "flatpak uninstall --system -y --noninteractive -- org.gimp.GIMP",
                "flatpak search -- gimp",
                "flatpak info --system --show-metadata -- org.gimp.GIMP",
            ]
        );
    }
}
