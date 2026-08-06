// src/backends/registry.rs

use crate::app::LuaHooks;
use crate::backends::generic::{ExportFormat, MachineListing, ManualFormat, OutdatedProbe};
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable, ManagerConfig, ManualListing, PropertyProbe,
    SearchSource, VersionPin,
};
use crate::backends::generic::{GenericEnumerable, OrphanDryRun};
use crate::backends::pip_search::PipSearchable;
use crate::config::Config;
use crate::core::{BackendCapabilities, CommandExecutor};
use crate::parsers::windows;
use crate::parsers::LambdaParser;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tracing::trace;

/// **Ordered, because everything downstream walks it and calls the result an order.**
///
/// This was a `HashMap`, whose iteration order Rust randomises per process — so `available()`
/// and `all()` handed back the backends in a different sequence on every run. Two `linix list`
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

    pub fn available(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends
            .values()
            .filter(|b| b.is_available())
            .cloned()
            .collect()
    }

    pub fn all(&self) -> Vec<Arc<BackendCapabilities>> {
        self.backends.values().cloned().collect()
    }

    pub fn get_filtered(&self, enabled: &[String]) -> Vec<Arc<BackendCapabilities>> {
        // `enabled.iter().any(|e| e == b.name())`, not `contains(&b.name().to_string())`: the
        // second allocates a `String` per backend per comparison, inside an O(n*m) scan.
        self.available()
            .into_iter()
            .filter(|b| enabled.iter().any(|e| e == b.name()))
            .collect()
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
        crate::backends::pacman::register(&mut reg, &executor, config);
        crate::backends::dnf::register(&mut reg, &executor, config);
        crate::backends::xbps::register(&mut reg, &executor, config);
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

    // --- Cross-platform & specialized backends (each module owns its registration) ---
    crate::backends::brew::register(&mut reg, &executor, config);
    register_cargo(&mut reg, &executor);
    register_pipx(&mut reg, &executor);
    register_uv(&mut reg, &executor);
    register_npm(&mut reg, &executor);
    register_pnpm(&mut reg, &executor);
    register_yarn(&mut reg, &executor);
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
    crate::backends::conda::register(&mut reg, &executor, config);
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

    // --- Ecosystem backends (generic, config-driven; cross-platform, runtime-gated) ---
    register_composer(&mut reg, &executor);
    register_opam(&mut reg, &executor);
    register_luarocks(&mut reg, &executor);
    register_nimble(&mut reg, &executor);
    register_pixi(&mut reg, &executor);
    register_spack(&mut reg, &executor);
    register_mix(&mut reg, &executor);
    register_helm(&mut reg, &executor);
    register_cabal(&mut reg, &executor);
    register_stack(&mut reg, &executor);
    register_asdf(&mut reg, &executor);

    // --- Ecosystem backends implemented as dedicated modules (subcommand binary / fs) ---
    crate::backends::go::register(&mut reg, &executor, config);
    register_pubdart(&mut reg, &executor);
    register_krew(&mut reg, &executor);

    // --- Linux-distro ecosystem backends (Gentoo, Guix, Solus, Slackware) ---
    if cfg!(target_os = "linux") {
        register_guix(&mut reg, &executor);
        register_emerge(&mut reg, &executor);
        register_eopkg(&mut reg, &executor);
        register_slackpkg(&mut reg, &executor);
    }

    // --- User-defined backends (the onboarder). Loaded last so a custom definition
    // can never silently shadow a built-in; collisions are skipped with a warning. ---
    crate::backends::onboarder::load_default_custom_backends(&mut reg, &executor, config);

    reg
}

// ============================================================================
// Generic (CLI-config-driven) backend registrations
// ============================================================================

fn register_apt(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apt".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "apt".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["remove".into(), "-y".into()],
            purge_args: Some(vec!["purge".into(), "-y".into()]),
            // apt lists installed packages via the SEPARATE `dpkg-query` binary, not
            // `apt dpkg-query`.
            list_binary: Some("dpkg-query".into()),
            list_args: vec!["-W".into(), "-f=${Package} ${Version}\\n".into()],
            // `dpkg-query -W` reports the entire dependency graph (579 packages on a stock
            // Ubuntu image, of which only 103 were user-chosen), so it cannot answer "what
            // did the user ask for?". `apt-mark` can — a third binary again, and one that
            // prints bare names with no versions, hence BareNames.
            manual: ManualListing::Command {
                binary: Some("apt-mark".into()),
                args: vec!["showmanual".into()],
                format: ManualFormat::BareNames,
            },
            // dpkg records which packages the system refuses to lose. Ask it, rather than
            // maintaining a per-release name list by hand.
            essential_args: Some(vec![
                "-W".into(),
                "-f=${Essential} ${Priority} ${Package}\\n".into(),
            ]),
            search_args: vec!["search".into()],
            search_binary: Some("apt-cache".into()),
            // `apt-cache search` matches descriptions and ranks results, so it cannot answer
            // "which names match this pattern". `pkgnames` prints the catalogue and nothing
            // else, which is what II.15's `re:` expands against. No root: it reads the index.
            enumerate_args: Some(vec!["pkgnames".into()]),
            enumerate_binary: Some("apt-cache".into()),
            upgrade_args: vec!["dist-upgrade".into(), "-y".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: Some(OrphanDryRun {
                binary: Some("apt-get".into()),
                args: vec!["autoremove".into(), "--dry-run".into()],
                removes_line_prefix: "Remv ".into(),
            }),
            repo_add_args: Some(vec!["-y".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["--remove".into(), "-y".into(), "{name}".into()]),
            repo_list_args: None,
            // `add-apt-repository` is its own program. Left as the first *argument* it ran as
            // `apt add-apt-repository -y <url>`, which apt refuses — so repo add and remove
            // could never have worked on apt at all.
            repo_binary: Some("add-apt-repository".into()),
            repo_list_binary: None,
            // No transitive dependency expansion for apt. apt resolves and installs a
            // package's full dependency closure itself at `apt-get install` time, so LiNix
            // re-deriving it is redundant. Worse, the planner's expansion is a recursive
            // BFS (walks jq -> libc6 -> libgcc-s1 -> …), and because apt's local cache lets
            // `apt depends` answer offline, that recursion fans out into hundreds of
            // subprocess calls and effectively hangs `status`/`sync`. It also wrongly tags
            // every transitive dependency as a LiNix-managed install. Leave dependency
            // resolution to apt. See the sync harness.
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            // `apt list --upgradable` also warns about an unstable CLI on stderr; the parser drops it.
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["list".into(), "--upgradable".into()],
                parse: std::sync::Arc::new(crate::parsers::apt::parse_apt_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(crate::parsers::apt::AptParser),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_enumerable(Arc::new(GenericEnumerable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// The two AUR helpers, each a two-argument registrar so the argv table can name it.
///
/// They are registered on Linux only, which makes them exactly the class
/// `every_os_native_backend_sends_the_argv_its_manager_expects` exists for — and until these
/// wrappers existed the five-argument `register_aur_helper` could not appear in that table, so
/// neither could they.
fn register_yay(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    register_aur_helper(
        reg,
        executor,
        "yay",
        |o| crate::parsers::pacman::parse_list_for(o, "yay"),
        |o| crate::parsers::pacman::parse_search_for(o, "yay"),
    );
}

fn register_paru(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    register_aur_helper(
        reg,
        executor,
        "paru",
        |o| crate::parsers::pacman::parse_list_for(o, "paru"),
        |o| crate::parsers::pacman::parse_search_for(o, "paru"),
    );
}

/// Register an AUR helper (`yay`, `paru`) as a generic backend. AUR helpers accept
/// pacman's flag syntax verbatim, so they reuse the pacman parsers — but with the
/// helper's own name stamped on results so state tracking stays per-backend correct.
/// Crucially `needs_root = false`: AUR helpers must run as an unprivileged user and
/// escalate internally; running them as root is unsupported and unsafe.
fn register_aur_helper(
    reg: &mut BackendRegistry,
    executor: &CommandExecutor,
    name: &'static str,
    installed_fn: fn(&str) -> Vec<crate::core::Package>,
    search_fn: fn(&str) -> Vec<crate::core::Package>,
) {
    let core = Arc::new(GenericBackendCore {
        name: name.into(),
        // AUR helpers speak pacman's flags, and they speak pacman's complaints too.
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: name.into(),
            binary: None,
            remove_binary: None,
            // AUR + Arch are rolling: no exact-version pin (mirrors pacman).
            version_pin: None,
            install_args: vec!["-S".into(), "--noconfirm".into(), "--needed".into()],
            remove_args: vec!["-Rs".into(), "--noconfirm".into()],
            purge_args: None,
            list_args: vec!["-Q".into()],
            // `-Qe` = explicitly installed only (11 of 173 on the arch test image).
            manual: ManualListing::Command {
                binary: None,
                args: vec!["-Qe".into()],
                format: ManualFormat::SameAsInstalled,
            },
            // pacman has no per-package essential flag: `base` is a convention and HoldPkg
            // is user config, so there is nothing authoritative to query.
            essential_args: None,
            search_args: vec!["-Ss".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["-Syu".into(), "--noconfirm".into()],
            update_args: Some(vec!["-Sy".into()]),
            // Orphan cleanup semantics differ per helper; leave it to the pacman backend
            // rather than guess, so we report Unsupported honestly instead of misfiring.
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn,
            search_fn,
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_apk(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apk".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "apk".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["add".into()],
            remove_args: vec!["del".into()],
            purge_args: None,
            list_args: vec!["info".into(), "-v".into()],
            // apk's explicit set IS the world file — `apk add`/`del` are edits to it. The
            // `apk world` subcommand only exists in apk 3.x (it errors on Alpine's 2.x, so
            // this silently reported nothing), but the file is stable and documented.
            // Entries may carry a version constraint or repo tag, which BareNames strips.
            manual: ManualListing::Command {
                binary: Some("cat".into()),
                args: vec!["/etc/apk/world".into()],
                format: ManualFormat::BareNames,
            },
            // apk has no essential concept; `alpine-base` is a meta-package convention.
            essential_args: None,
            search_args: vec!["search".into(), "-v".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "-c".into(),
                "echo '{url}' >> /etc/apk/repositories".into(),
            ]),
            repo_remove_args: Some(vec![
                "-c".into(),
                "sed -i '\\|{url}|d' /etc/apk/repositories".into(),
            ]),
            repo_list_args: Some(vec!["/etc/apk/repositories".into()]),
            // apk has no repo verb at all: its sources are a plain file. The shell writes
            // it and `cat` reads it — as arguments they ran as `apk sh -c …` and `apk cat
            // …`, which apk refuses.
            repo_binary: Some("sh".into()),
            repo_list_binary: Some("cat".into()),
            // No transitive dependency expansion for apk. `apk info -R <pkg>` emits a
            // header line ("<pkg>-<ver>-<rev> depends on:") plus virtual provider tokens
            // (`so:libc.musl…`, `pc:…`, `cmd:…`) — none of which are installable package
            // names. The generic label-parser would turn the header into a bogus target
            // (`jq-1.8.1-r0`) and the `so:` provides into non-existent packages, so `apk add`
            // would fail with "no such package". apk resolves its own dependency closure at
            // install time, so LiNix does not need to expand it. See the sync harness.
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["version".into(), "-l".into(), "<".into()],
                parse: std::sync::Arc::new(|o: &str| {
                    crate::parsers::common::parse_apk_outdated(o, "apk")
                }),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            // `apk info -v` emits `name-version-revision` as a single dash-joined token
            // per line (e.g. `tree-2.1.1-r0`); parse it so `info("tree")` matches by the
            // bare name. `parse_simple_list` would keep the whole token as the name, so
            // installed lookups (and therefore `remove`) never found the package.
            installed_fn: |o| crate::parsers::common::parse_dash_version_list(o, "apk"),
            // `apk search -v` answers with the same dash-joined token, followed by
            // ` - description`. Splitting on whitespace alone kept `jq-1.7.1-r0` as the name,
            // so a search result could never equal the name asked for — which made apk
            // invisible to every unpinned line, the way dnf was on Fedora.
            search_fn: |o| crate::parsers::common::parse_dash_version_list(o, "apk"),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_zypper(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "zypper".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "zypper".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}={version}".into())),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["remove".into(), "-y".into()],
            purge_args: None,
            list_args: vec!["search".into(), "--installed-only".into()],
            // zypper resolves dependencies, so its installed set is not the user's set.
            // `zypper packages --userinstalled` would answer this, but it emits a
            // pipe-delimited table no parser here handles and no test image covers it —
            // so decline to adopt rather than guess.
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["update".into(), "-y".into()],
            update_args: Some(vec!["refresh".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec!["addrepo".into(), "{url}".into(), "{name}".into()]),
            repo_remove_args: Some(vec!["removerepo".into(), "{name}".into()]),
            repo_list_args: Some(vec!["repos".into()]),
            repo_binary: None,
            repo_list_binary: None,
            // None, like apt, dnf and pacman: zypper resolves its own dependency closure at
            // install time, so LiNix re-deriving one adds nodes the planner then tries to
            // install by name. What `info --requires` reports are RPM capabilities
            // (`libjq.so.1()(64bit)`), not packages anyone declares — and until 2026-07-30 this
            // was the only system manager that set it, which is why it was the only one whose
            // first real run could not install anything.
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["--non-interactive".into(), "list-updates".into()],
                parse: std::sync::Arc::new(crate::parsers::dnf::parse_zypper_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dnf::parse_zypper_search,
            search_fn: crate::parsers::dnf::parse_zypper_search,
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_winget(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "winget".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "winget".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec![
                "install".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ],
            remove_args: vec!["uninstall".into(), "--silent".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // `winget list` is the machine, not the manifest. 186 of the 280 rows it reports on
            // a stock Windows box are `ARP\…`/`MSIX\…` identifiers winget synthesises from the
            // registry: `winget uninstall` takes them, `winget install` answers `No package
            // found matching input criteria` for every one, and a third of them carry their own
            // version so the name changes when the package updates. `winget export` is winget's
            // own answer to what it could put back, and adoption may write nothing else.
            manual: ManualListing::ExportFile {
                binary: None,
                args: vec![
                    "export".into(),
                    "-o".into(),
                    "{file}".into(),
                    // No `--include-versions`: adoption declares a package, not a moment.
                    "--accept-source-agreements".into(),
                    "--disable-interactivity".into(),
                ],
                format: ExportFormat::WingetJson,
            },
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "--all".into(), "--silent".into()],
            update_args: Some(vec!["source".into(), "update".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "source".into(),
                "add".into(),
                "--name".into(),
                "{name}".into(),
                "--arg".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec![
                "source".into(),
                "remove".into(),
                "--name".into(),
                "{name}".into(),
            ]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec![
                    "upgrade".into(),
                    "--disable-interactivity".into(),
                    "--accept-source-agreements".into(),
                ],
                parse: std::sync::Arc::new(windows::parse_winget_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("winget", o),
            search_fn: |o| windows::parse_search("winget", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_scoop(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "scoop".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "scoop".into(),
            binary: None,
            remove_binary: None,
            version_pin: None, // scoop pins via versioned manifests; not a simple flag

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // scoop apps are each installed on request; it tracks no dependency graph.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["update".into(), "*".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "bucket".into(),
                "add".into(),
                "{name}".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec!["bucket".into(), "rm".into(), "{name}".into()]),
            repo_list_args: Some(vec!["bucket".into(), "list".into()]),
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: Some(MachineListing {
                binary: None,
                // `scoop export` is JSON in current scoop and plain lines in older ones; the
                // negotiation below is what makes asking safe.
                args: vec!["export".into()],
                parse: std::sync::Arc::new(windows::parse_scoop_export),
            }),
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["status".into()],
                parse: std::sync::Arc::new(windows::parse_scoop_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("scoop", o),
            search_fn: |o| windows::parse_search("scoop", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_choco(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "choco".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "choco".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec!["install".into(), "-y".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            purge_args: None,
            // Chocolatey 2.x removed `-lo`: `list` is local-only now and the flag is an
            // error, so the command failed, the output was empty, and LiNix read that as
            // "nothing is installed" — the input to a mass removal, not a bad listing.
            list_args: vec!["list".into(), "-r".into()],
            // `choco list` reports locally-installed packages, all user-requested.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            // `-r` (--limit-output) makes search machine-readable `name|version` rows. Without
            // it choco prints its own `Chocolatey v2.7.3` banner and an `N packages found.`
            // summary, and both became packages in the results. `list` was given `-r` long
            // ago, for a related reason; its twin was left alone.
            search_args: vec!["search".into(), "-r".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "all".into(), "-y".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: Some(vec![
                "source".into(),
                "add".into(),
                "-n".into(),
                "{name}".into(),
                "-s".into(),
                "{url}".into(),
            ]),
            repo_remove_args: Some(vec![
                "source".into(),
                "remove".into(),
                "-n".into(),
                "{name}".into(),
            ]),
            repo_list_args: Some(vec!["source".into(), "list".into()]),
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["outdated".into(), "-r".into()],
                parse: std::sync::Arc::new(windows::parse_choco_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("choco", o),
            search_fn: |o| windows::parse_search("choco", o),
        }),
    });
    // Without this the manager runs on `ExitPolicy::default()` - no failure, permanent or
    // absent markers - so `scoop install <no-such-package>` is classified `unknown`, Q1 never
    // withdraws the line, and the dead declaration fails every later install.
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_mas(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "mas".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "mas".into(),
            binary: None,
            remove_binary: None,
            version_pin: None, // Mac App Store installs the current published version only

            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // Every App Store app was installed by a person clicking Get.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::macos::parse_mas_list,
            search_fn: crate::parsers::macos::parse_mas_search,
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_pip(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    // Generic for install/list; Searchable is a bespoke PyPI JSON lookup
    // (pip's own `search` was disabled upstream).
    let core = Arc::new(GenericBackendCore {
        name: "pip".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pip".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}=={version}".into())),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into(), "-y".into()],
            purge_args: None,
            list_args: vec!["list".into(), "--format=json".into()],
            // `pip list` includes every pulled-in dependency and pip keeps no record of
            // which distributions a person actually asked for. (`--not-required` reports
            // leaves, which is a different question: a leaf may still be a dependency.)
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["install".into(), "--upgrade".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["list".into(), "--outdated".into(), "--format=json".into()],
                parse: std::sync::Arc::new(crate::parsers::language::parse_pip_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("pip", o),
            search_fn: |_| vec![],
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(PipSearchable))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_gem(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "gem".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "gem".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec!["-v".into(), "{version}".into()])),
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["list".into(), "--local".into()],
            // `gem list --local` mixes user-installed gems with their dependencies, and
            // RubyGems records no explicit-install marker.
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["update".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: Some(vec!["sources".into(), "-a".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["sources".into(), "-r".into(), "{url}".into()]),
            repo_list_args: Some(vec!["sources".into()]),
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: Some(OutdatedProbe {
                binary: None,
                args: vec!["outdated".into()],
                parse: std::sync::Arc::new(crate::parsers::language::parse_gem_outdated),
            }),
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("gem", o),
            search_fn: |o| crate::parsers::language::parse_search("gem", o),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_repo_manager(Arc::new(GenericRepoManager { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_bun(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "bun".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "bun".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::Inline("{name}@{version}".into())),
            install_args: vec!["add".into(), "-g".into()],
            remove_args: vec!["remove".into(), "-g".into()],
            purge_args: None,
            list_args: vec!["pm".into(), "ls".into(), "-g".into()],
            // `bun pm ls -g` lists the top-level global installs (dependencies only appear
            // under `--all`), so what it reports is what was asked for.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("bun", o),
            search_fn: |_| vec![],
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

fn register_macports(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "macports".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "macports".into(),
            // The port collection is `macports`; the program it ships is `port`. Without this
            // the backend probed for a `macports` binary that exists on no Mac, so it never
            // came up READY and every command it would have run was `macports install …`.
            binary: Some("port".into()),
            remove_binary: None,
            // MacPorts pins via `install name @version`, but versions are entangled with
            // variants/revisions; skip automatic pinning rather than risk a wrong ref.
            version_pin: None,
            install_args: vec!["install".into()],
            remove_args: vec!["uninstall".into()],
            purge_args: None,
            list_args: vec!["installed".into()],
            // `port installed requested` = ports the user asked for, not pulled-in deps.
            manual: ManualListing::Command {
                binary: None,
                args: vec!["installed".into(), "requested".into()],
                format: ManualFormat::SameAsInstalled,
            },
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "outdated".into()],
            update_args: Some(vec!["selfupdate".into()]),
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::macos::parse_macports_installed,
            search_fn: crate::parsers::macos::parse_macports_search,
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// pkgsrc's binary package tool. Cross-platform (NetBSD/SmartOS/illumos, plus pkgsrc
/// on Linux/macOS); gated at runtime by the presence of the `pkgin` binary.
fn register_pkgin(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkgin".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pkgin".into(),
            binary: None,
            remove_binary: None,
            version_pin: None,
            install_args: vec!["-y".into(), "install".into()],
            remove_args: vec!["-y".into(), "remove".into()],
            purge_args: None,
            list_args: vec!["list".into()],
            // pkgin installs dependencies and `pkgin list` reports them all; its
            // automatic-install marker is not exposed through a stable listing command.
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["-y".into(), "full-upgrade".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o),
            search_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// FreeBSD's `pkg` (U26). One binary with subcommands, like apt — `pkg install`, `pkg delete`,
/// `pkg info`. Gated at runtime by the presence of `pkg`; on a Linux/mac box it simply is not
/// available. `when family == freebsd` already answers on a BSD (`d66730e`), so a module can
/// scope its `pkg:` lines to the platform.
fn register_pkg_freebsd(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkg".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pkg".into(),
            binary: None,
            remove_binary: None,
            version_pin: None,
            install_args: vec!["install".into(), "-y".into()],
            // `pkg delete` is the canonical uninstall; `-y` so a non-interactive sync does not hang.
            remove_args: vec!["delete".into(), "-y".into()],
            purge_args: None,
            list_args: vec!["info".into()],
            // FreeBSD marks automatically-installed packages; `%a = 0` selects the ones the
            // user asked for, `%n` prints just the name. That is exactly `adopt`'s manual set.
            manual: ManualListing::Command {
                binary: None,
                args: vec!["query".into(), "-e".into(), "%a = 0".into(), "%n".into()],
                format: crate::backends::generic::ManualFormat::BareNames,
            },
            essential_args: None,
            search_args: vec!["search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec!["upgrade".into(), "-y".into()],
            update_args: Some(vec!["update".into()]),
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg(o),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// OpenBSD's package tools (U26). Unlike FreeBSD there is no single frontend: install is
/// `pkg_add <name>` (no subcommand), remove is a SEPARATE binary `pkg_delete <name>`, and both
/// listing and search are `pkg_info`. The `remove_binary` field is what lets one backend drive
/// three tools. Gated by the presence of `pkg_add`.
fn register_pkg_add_openbsd(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkg_add".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "pkg_add".into(),
            binary: None,
            // The uninstaller is its own program; `pkg_delete <name>` takes no subcommand, so
            // remove_args stays empty and the separate-binary path in `remove` handles it.
            remove_binary: Some("pkg_delete".into()),
            version_pin: None,
            // `pkg_add <name>` — the binary itself is the verb, so no leading subcommand.
            install_args: vec![],
            remove_args: vec![],
            purge_args: None,
            // `pkg_info` with no args lists installed packages.
            list_args: vec![],
            list_binary: Some("pkg_info".into()),
            // OpenBSD does not expose a stable manual/automatic split through pkg_info, so
            // adoption skips it rather than risk adopting dependency packages.
            manual: ManualListing::Unsupported,
            essential_args: None,
            // `pkg_info -Q <query>` searches the remote package set.
            search_args: vec!["-Q".into()],
            search_binary: Some("pkg_info".into()),
            enumerate_args: None,
            enumerate_binary: None,
            // `pkg_add -u` updates every installed package to the newest build.
            upgrade_args: vec!["-u".into()],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg_add(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg_add(o),
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
}

/// .NET global tools (`dotnet tool ...`). Cross-platform; gated by the `dotnet` binary.
/// This is the system-inventory surface of the .NET ecosystem — plain NuGet packages
/// are project-scoped and deliberately out of scope.
fn register_dotnet(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "dotnet".into(),
        executor: executor.duplicate(),
        config: ManagerConfig {
            name: "dotnet".into(),
            binary: None,
            remove_binary: None,
            version_pin: Some(VersionPin::after(vec![
                "--version".into(),
                "{version}".into(),
            ])),
            install_args: vec!["tool".into(), "install".into(), "--global".into()],
            remove_args: vec!["tool".into(), "uninstall".into(), "--global".into()],
            purge_args: None,
            list_args: vec!["tool".into(), "list".into(), "--global".into()],
            // Global .NET tools are installed one by one, on request.
            manual: ManualListing::AllInstalled,
            essential_args: None,
            search_args: vec!["tool".into(), "search".into()],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec![
                "tool".into(),
                "update".into(),
                "--global".into(),
                "--all".into(),
            ],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            depends_args: None,
            needs_root: false,
            is_exclusive: false,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: Some(MachineListing {
                binary: None,
                // SDK 10 and later. Older SDKs reject the flag, which is what the
                // negotiation in `fetch_installed` is for.
                args: vec![
                    "tool".into(),
                    "list".into(),
                    "--global".into(),
                    "--format".into(),
                    "json".into(),
                ],
                parse: std::sync::Arc::new(crate::parsers::dotnet::parse_dotnet_list_json),
            }),
            outdated: None,
            search_source: SearchSource::Command,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dotnet::parse_dotnet_list,
            search_fn: crate::parsers::dotnet::parse_dotnet_search,
        }),
    });
    let core = with_manager_policy(core);
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .with_queryable(Arc::new(GenericQueryable { core: core.clone() }))
            .with_searchable(Arc::new(GenericSearchable { core: core.clone() }))
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .with_metadata_provider(core.clone())
            .build(),
    ));
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
        depends_args: None,
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
        flag_map: HashMap::new(),
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

/// PHP / Packagist (`composer global ...`). Cross-platform; gated by the `composer` binary.
fn register_composer(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("composer");
    // `composer global show` lists the whole solved tree. `--direct` would report just
    // the requested packages, but no test image covers composer, so decline to adopt
    // rather than ship an unverified guess.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}:{version}".into()));
    cfg.install_args = vec!["global".into(), "require".into()];
    cfg.remove_args = vec!["global".into(), "remove".into()];
    cfg.list_args = vec!["global".into(), "show".into(), "--format=json".into()];
    cfg.search_args = vec!["global".into(), "search".into(), "--format=json".into()];
    // composer prints `Changed current directory to ...` ahead of the JSON, which the
    // parser steps past — a strict parse would report nothing outdated on every machine.
    cfg.outdated = Some(OutdatedProbe {
        binary: None,
        args: vec!["global".into(), "outdated".into(), "--format=json".into()],
        parse: std::sync::Arc::new(crate::parsers::language::parse_composer_outdated),
    });
    cfg.upgrade_args = vec!["global".into(), "update".into()];
    let core = Arc::new(GenericBackendCore {
        name: "composer".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("composer", o),
            search_fn: |o| crate::parsers::language::parse_search("composer", o),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// OCaml (`opam`). Cross-platform; gated by the `opam` binary.
fn register_opam(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("opam");
    // opam installs dependencies as packages. `opam list --installed --roots` reports
    // the root (explicitly-installed) set and would be the right wiring; unverified here,
    // so adopt nothing.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}.{version}".into()));
    cfg.install_args = vec!["install".into(), "-y".into()];
    cfg.remove_args = vec!["remove".into(), "-y".into()];
    cfg.list_args = vec!["list".into(), "--installed".into(), "--short".into()];
    cfg.search_args = vec!["search".into(), "--short".into()];
    cfg.upgrade_args = vec!["upgrade".into(), "-y".into()];
    let core = Arc::new(GenericBackendCore {
        name: "opam".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::names_only(o, "opam"),
            search_fn: |o| crate::parsers::ecosystem::names_only(o, "opam"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Lua (`luarocks`). Cross-platform; gated by the `luarocks` binary. Version is a trailing
/// positional (`luarocks install <pkg> <version>`).
fn register_luarocks(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("luarocks");
    // luarocks installs a rock's dependencies alongside it and records no explicit
    // marker to tell them apart.
    cfg.manual = ManualListing::Unsupported;
    // The version is a bare operand, not an option: `luarocks install --` answers
    // `Error: missing argument 'rock'` over usage `<rock> [<version>]`, and
    // `luarocks install -- <rock> <version>` is identical to the same line without the
    // terminator (measured, `tools` image 2026-08-04). So the terminator stays on both the
    // pinned and the unpinned path, which is the whole point of saying `after` rather than
    // reaching for a variant named after flags.
    cfg.version_pin = Some(VersionPin::after(vec!["{version}".into()]));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["remove".into()];
    cfg.list_args = vec!["list".into(), "--porcelain".into()];
    cfg.search_args = vec!["search".into(), "--porcelain".into()];
    let core = Arc::new(GenericBackendCore {
        name: "luarocks".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "luarocks"),
            search_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "luarocks"),
        }),
    });
    register_generic(reg, core, true, true, false);
}

/// Nim (`nimble`). Cross-platform; gated by the `nimble` binary. No CLI search/upgrade-all.
fn register_nimble(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("nimble");
    // nimble installs dependencies and `list --installed` reports them all.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    cfg.install_args = vec!["install".into(), "-y".into()];
    cfg.remove_args = vec!["uninstall".into(), "-y".into()];
    cfg.list_args = vec!["list".into(), "--installed".into()];
    let core = Arc::new(GenericBackendCore {
        name: "nimble".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::nimble_list(o, "nimble"),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, false);
}

/// pixi global environments (`pixi global ...`). Cross-platform; gated by the `pixi` binary.
fn register_pixi(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("pixi");
    // `pixi global` installs one requested tool per entry; dependencies live inside each
    // tool's own environment and are never listed here.
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}={version}".into()));
    cfg.install_args = vec!["global".into(), "install".into()];
    // pixi removes a global TOOL with `global uninstall`; `global remove` deletes a package
    // from an environment and requires `--environment`, so it errors on a bare tool name.
    cfg.remove_args = vec!["global".into(), "uninstall".into()];
    cfg.list_args = vec!["global".into(), "list".into()];
    // pixi prints its listing as a box-drawing tree; `--json` is the same answer already
    // parsed. Recent pixi only, hence the negotiation rather than a straight swap.
    cfg.machine_list = Some(MachineListing {
        binary: None,
        args: vec!["global".into(), "list".into(), "--json".into()],
        parse: std::sync::Arc::new(|o: &str| crate::parsers::ecosystem::pixi_list_json(o, "pixi")),
    });
    cfg.search_args = vec!["search".into()];
    // `global upgrade-all` was removed upstream; pixi 0.73 answers it with "This command has
    // been removed, please use `pixi global update` instead". A plan-smoke passed it the whole
    // time, because constructing an argv proves nothing about whether the argv exists.
    cfg.upgrade_args = vec!["global".into(), "update".into()];
    let core = Arc::new(GenericBackendCore {
        name: "pixi".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::pixi_list(o, "pixi"),
            search_fn: |o| crate::parsers::ecosystem::pixi_search(o, "pixi"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Spack HPC package manager (`spack`). Cross-platform; gated by the `spack` binary.
fn register_spack(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("spack");
    // spack installs dependencies as first-class packages, so `spack find` is the whole
    // closure. `spack find --explicit` is the right answer; unverified here.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["uninstall".into(), "-y".into()];
    cfg.list_args = vec!["find".into(), "--format".into(), "{name} {version}".into()];
    cfg.search_args = vec!["list".into()];
    let core = Arc::new(GenericBackendCore {
        name: "spack".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "spack"),
            search_fn: |o| crate::parsers::ecosystem::names_only(o, "spack"),
        }),
    });
    register_generic(reg, core, true, true, false);
}

/// Elixir/Hex archives (`mix archive.*`). Cross-platform; gated by the `mix` binary. This is
/// the global-archive surface of the Elixir ecosystem; project-scoped hex deps are out of
/// scope. No CLI search/upgrade-all.
fn register_mix(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("mix");
    // Mix archives are installed one by one, on request.
    cfg.manual = ManualListing::AllInstalled;
    // The version is a bare positional after the name: `mix archive.install hex phx_new 1.6.16`.
    // Not optional in practice — an archive declares which Elixir it supports, so on Elixir
    // 1.14 the newest `phx_new` fetches, builds and then refuses to run, and pinning is the
    // only way to install one at all (measured, `tools` image 2026-07-29).
    cfg.version_pin = Some(VersionPin::after(vec!["{version}".into()]));
    cfg.install_args = vec!["archive.install".into(), "hex".into(), "--force".into()];
    // `--force` on the removal too, and this is not symmetry for its own sake: measured, a
    // bare `mix archive.uninstall phx_new` with no terminal prints `Are you sure…? [Yn]`,
    // takes the empty answer, **exits 0 and leaves the archive installed**. LiNix reported a
    // removal that did not happen — the scoop-exit-0 shape (E7), one manager over.
    cfg.remove_args = vec!["archive.uninstall".into(), "--force".into()];
    cfg.list_args = vec!["archive".into()];
    let core = Arc::new(GenericBackendCore {
        name: "mix".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::mix_archive(o, "mix"),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, false);
}

/// Helm plugins (`helm plugin ...`). Cross-platform; gated by the `helm` binary. (Chart
/// releases are a different concept and out of scope here.) No plugin search.
///
/// A declaration is `helm:NAME@url=SOURCE`: helm installs a plugin from a URL but lists and
/// uninstalls it by the name in its `plugin.yaml`, so naming the URL would install a plugin
/// LiNix could never remove or recognise again (U39).
fn register_helm(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("helm");
    // Helm plugins are installed individually and pull in no plugin dependencies.
    cfg.manual = ManualListing::AllInstalled;
    cfg.install_source_option =
        crate::backends::capability::install_source_key("helm").map(Into::into);
    cfg.install_args = vec!["plugin".into(), "install".into()];
    cfg.remove_args = vec!["plugin".into(), "uninstall".into()];
    cfg.list_args = vec!["plugin".into(), "list".into()];
    let core = Arc::new(GenericBackendCore {
        name: "helm".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "helm"),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, false);
}

/// Haskell (`cabal`). Cross-platform; gated by the `cabal` binary. cabal has no uninstall
/// verb, so `remove_args` is empty → removal reports Unsupported (see GenericInstallable).
fn register_cabal(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("cabal");
    // cabal installs a package's dependency closure into the store and lists it back.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}-{version}".into()));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec![]; // no uninstall verb
    cfg.list_args = vec![
        "list".into(),
        "--installed".into(),
        "--simple-output".into(),
    ];
    cfg.search_args = vec!["list".into(), "--simple-output".into()];
    let core = Arc::new(GenericBackendCore {
        name: "cabal".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "cabal"),
            search_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "cabal"),
        }),
    });
    register_generic(reg, core, true, true, false);
}

/// Haskell (`stack`). Cross-platform; gated by the `stack` binary. Like cabal it has no
/// uninstall verb (empty `remove_args` → Unsupported) and no reliable global install list.
fn register_stack(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("stack");
    // stack resolves and installs dependencies; nothing distinguishes them on listing.
    cfg.manual = ManualListing::Unsupported;
    cfg.version_pin = Some(VersionPin::Inline("{name}-{version}".into()));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec![]; // no uninstall verb
    let core = Arc::new(GenericBackendCore {
        name: "stack".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |_| vec![],
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, false, false, false);
}

/// asdf version manager (`asdf`). Cross-platform; gated by the `asdf` binary. A tool/plugin
/// is the "package"; installing pins a version via the trailing positional.
/// The three Node managers, which spell one shape three ways.
///
/// They were three modules totalling 757 non-test lines, ~85% identical once renamed, with
/// `global_argv` defined three separate times — npm's and pnpm's copies character-for-character
/// the same. What actually differs is four things, and all four are data: the global flag
/// (`-g` vs a `global` verb), the install verb (`install` vs `add`), the remove verb
/// (`uninstall` vs `remove`), and where the manager keeps what it installed.
///
/// None of them has a usable CLI search — npm's is slow and output-unstable, pnpm has none, and
/// yarn removed its own in Berry — and all three resolve from the npm registry, which is why
/// `node_registry.rs` already existed and why three backends reached into it. That is
/// `SearchSource::NpmRegistry` now.
///
/// Upgrading is re-installing each global package, for all three.
fn register_npm(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("npm");
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    cfg.install_args = vec!["install".into(), "-g".into()];
    cfg.remove_args = vec!["uninstall".into(), "-g".into()];
    cfg.list_args = vec![
        "list".into(),
        "-g".into(),
        "--depth=0".into(),
        "--json".into(),
    ];
    cfg.upgrade_reinstall_args = Some(cfg.install_args.clone());
    cfg.search_source = SearchSource::NpmRegistry;
    // `npm outdated` exits non-zero when it FINDS something, so this is read
    // through `run_output` rather than a status-checked reader.
    cfg.outdated = Some(OutdatedProbe {
        binary: None,
        args: vec!["outdated".into(), "-g".into(), "--json".into()],
        parse: std::sync::Arc::new(|o: &str| {
            crate::parsers::language::parse_npm_outdated(o, "npm")
        }),
    });
    // `npm prefix -g` reports the PREFIX, not the module directory, and the layout below it
    // differs by OS: POSIX puts modules under `lib/node_modules`, Windows directly under
    // `node_modules`. Getting this wrong yields a path that does not exist.
    cfg.property_probes = vec![PropertyProbe {
        property: "install_path".into(),
        args: vec!["prefix".into(), "-g".into()],
        template: if cfg!(windows) {
            "{base}/node_modules/{name}".into()
        } else {
            "{base}/lib/node_modules/{name}".into()
        },
    }];
    let core = Arc::new(GenericBackendCore {
        name: "npm".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("npm", o),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, true, true);
}

fn register_pnpm(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("pnpm");
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    cfg.install_args = vec!["add".into(), "-g".into()];
    cfg.remove_args = vec!["remove".into(), "-g".into()];
    cfg.list_args = vec![
        "list".into(),
        "-g".into(),
        "--depth=0".into(),
        "--json".into(),
    ];
    cfg.upgrade_reinstall_args = Some(cfg.install_args.clone());
    cfg.search_source = SearchSource::NpmRegistry;
    // `npm outdated` exits non-zero when it FINDS something, so this is read
    // through `run_output` rather than a status-checked reader.
    cfg.outdated = Some(OutdatedProbe {
        binary: None,
        args: vec!["outdated".into(), "-g".into(), "--json".into()],
        parse: std::sync::Arc::new(|o: &str| {
            crate::parsers::language::parse_npm_outdated(o, "pnpm")
        }),
    });
    // `pnpm root -g` already IS the global node_modules directory, so the package folder is
    // `<root>/<name>`; appending another `node_modules` yields a path that does not exist.
    cfg.property_probes = vec![
        PropertyProbe {
            property: "install_path".into(),
            args: vec!["root".into(), "-g".into()],
            template: "{base}/{name}".into(),
        },
        PropertyProbe {
            property: "bin_path".into(),
            args: vec!["bin".into(), "-g".into()],
            template: "{base}".into(),
        },
    ];
    let core = Arc::new(GenericBackendCore {
        name: "pnpm".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            // `pnpm list -g --json` returns an ARRAY of project objects where npm returns one
            // bare object. The shared parser handles both; parsing pnpm as npm yields nothing.
            installed_fn: |o| crate::parsers::language::parse_installed("pnpm", o),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, true, true);
}

fn register_yarn(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("yarn");
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    // yarn 1 spells global as a leading verb rather than a flag.
    cfg.install_args = vec!["global".into(), "add".into()];
    cfg.remove_args = vec!["global".into(), "remove".into()];
    // Not `--json`: yarn 1's JSON stream reports the *binaries* a global package installed
    // (`{"type":"bins-catj","items":["catj"]}`) and never the package, so the one line that
    // names it is the plain output's. Measured on a host with `catj` installed.
    cfg.list_args = vec!["global".into(), "list".into()];
    cfg.upgrade_reinstall_args = Some(cfg.install_args.clone());
    cfg.search_source = SearchSource::NpmRegistry;
    // `yarn global dir` returns the folder CONTAINING node_modules, unlike `pnpm root -g`.
    cfg.property_probes = vec![
        PropertyProbe {
            property: "install_path".into(),
            args: vec!["global".into(), "dir".into()],
            template: "{base}/node_modules/{name}".into(),
        },
        PropertyProbe {
            property: "bin_path".into(),
            args: vec!["global".into(), "bin".into()],
            template: "{base}".into(),
        },
    ];
    let core = Arc::new(GenericBackendCore {
        name: "yarn".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("yarn", o),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Rust / crates.io. Was 298 non-test lines.
///
/// The one thing worth carrying over: **`cargo install foo` on an already-installed foo
/// declines and exits 0.** Upgrading has to say `--force`, which is why
/// `upgrade_reinstall_args` carries args of its own instead of being a boolean — a boolean
/// would have upgraded cargo by asking it to do nothing, and reported success.
fn register_cargo(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("cargo");
    // `cargo install --list` reports exactly what was asked for; crates.io dependencies are
    // compiled in, never installed as separate entries.
    cfg.manual = ManualListing::AllInstalled;
    // `--version` is an option of `install`, not of the crate name, so it goes ahead of the
    // terminator and the name stays protected behind it.
    cfg.version_pin = Some(VersionPin::Before(vec![
        "--version".into(),
        "{version}".into(),
    ]));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["uninstall".into()];
    cfg.list_args = vec!["install".into(), "--list".into()];
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_reinstall_args = Some(vec!["install".into(), "--force".into()]);
    let core = Arc::new(GenericBackendCore {
        name: "cargo".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            // `cargo install --list` indents each crate's binaries beneath it; a column parser
            // would read those indented lines as package names.
            installed_fn: |o| crate::parsers::language::parse_installed("cargo", o),
            search_fn: |o| crate::parsers::language::parse_search("cargo", o),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Python applications in their own venvs. Was 193 non-test lines, of which the only part
/// outside this table was asking pipx where a venv lives.
fn register_pipx(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("pipx");
    // pipx installs one requested application per entry; its dependencies live inside that
    // application's venv and are never listed here.
    cfg.manual = ManualListing::AllInstalled;
    // pipx takes a pip requirement spec.
    cfg.version_pin = Some(VersionPin::Inline("{name}=={version}".into()));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["uninstall".into()];
    cfg.list_args = vec!["list".into(), "--json".into()];
    cfg.upgrade_args = vec!["upgrade-all".into()];
    cfg.property_probes = vec![PropertyProbe {
        property: "install_path".into(),
        args: vec!["environment".into(), "--value".into(), "PIPX_HOME".into()],
        template: "{base}/venvs/{name}".into(),
    }];
    let core = Arc::new(GenericBackendCore {
        name: "pipx".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("pipx", o),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, true);
}

/// uv's tool installs. Was 214 non-test lines.
fn register_uv(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("uv");
    // `uv tool list` reports installed tools only; there are no implicit or dependency tools
    // to tell apart.
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}=={version}".into()));
    cfg.install_args = vec!["tool".into(), "install".into()];
    cfg.remove_args = vec!["tool".into(), "uninstall".into()];
    cfg.list_args = vec!["tool".into(), "list".into()];
    cfg.upgrade_args = vec!["tool".into(), "upgrade".into(), "--all".into()];
    cfg.property_probes = vec![PropertyProbe {
        property: "install_path".into(),
        args: vec!["tool".into(), "dir".into()],
        template: "{base}/{name}".into(),
    }];
    let core = Arc::new(GenericBackendCore {
        name: "uv".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "uv"),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, true);
}

/// krew, the kubectl plugin manager. Its verbs are subcommands of `kubectl`.
///
/// Was 193 lines of hand-written Rust. The one thing that file knew and this table did not is
/// its availability check: krew is a *plugin*, so `kubectl` alone is not enough — a host with
/// kubectl and no krew reported READY and then failed every command with `unknown command
/// "krew"`, including `linix update`, which refreshes every backend at once. That is
/// `extra_probes` now, so the next plugin-shaped manager inherits the fix rather than
/// rediscovering it.
fn register_krew(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("krew");
    cfg.binary = Some("kubectl".into());
    cfg.extra_probes = Some(vec!["kubectl-krew".into()]);
    // Every krew plugin is one somebody asked for; krew installs no dependencies.
    cfg.manual = ManualListing::AllInstalled;
    // krew installs the index's current version and has no per-install version pin.
    cfg.install_args = vec!["krew".into(), "install".into()];
    cfg.remove_args = vec!["krew".into(), "uninstall".into()];
    cfg.list_args = vec!["krew".into(), "list".into()];
    cfg.search_args = vec!["krew".into(), "search".into()];
    cfg.upgrade_args = vec!["krew".into(), "upgrade".into()];
    cfg.update_args = Some(vec!["krew".into(), "update".into()]);
    let core = Arc::new(GenericBackendCore {
        name: "krew".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            // `kubectl krew list` prints `PLUGIN  VERSION` (older versions: bare names);
            // `search` prints `NAME  DESCRIPTION  INSTALLED`, so only the first column is a name.
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "krew"),
            search_fn: |o| crate::parsers::ecosystem::names_only(o, "krew"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Dart / pub, reached through `dart pub global`.
///
/// Was 197 lines. Nothing in it was outside the table: two-word verbs under a different binary.
fn register_pubdart(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("pub");
    cfg.binary = Some("dart".into());
    // `dart pub global list` reports exactly what was activated — all user-chosen.
    cfg.manual = ManualListing::AllInstalled;
    // pub pins with a trailing positional version (`activate <pkg> <version>`) — an operand,
    // so the `--` terminator stays in front of both it and the name.
    cfg.version_pin = Some(VersionPin::after(vec!["{version}".into()]));
    cfg.install_args = vec!["pub".into(), "global".into(), "activate".into()];
    cfg.remove_args = vec!["pub".into(), "global".into(), "deactivate".into()];
    cfg.list_args = vec!["pub".into(), "global".into(), "list".into()];
    // pub.dev has no upgrade-all verb: re-activating each package unpinned is the upgrade.
    cfg.upgrade_reinstall_args = Some(cfg.install_args.clone());
    let core = Arc::new(GenericBackendCore {
        name: "pub".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "pub"),
            search_fn: |_| vec![],
        }),
    });
    // Upgradable, not searchable: pub.dev has no CLI search, and upgrade is the re-activate
    // loop above rather than a verb.
    register_generic(reg, core, true, false, true);
}

fn register_asdf(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("asdf");
    // asdf lists the tool versions someone explicitly installed; it has no dep concept.
    cfg.manual = ManualListing::AllInstalled;
    // asdf refuses to install without a version, so an unpinned line asks for `latest`.
    // Removal needs none: measured, `asdf uninstall nodejs` returns 0 and the version leaves
    // `asdf list`.
    cfg.version_pin = Some(VersionPin::after_required(
        vec!["{version}".into()],
        "latest",
    ));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["uninstall".into()];
    cfg.list_args = vec!["list".into()];
    let core = Arc::new(GenericBackendCore {
        name: "asdf".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::asdf_list(o, "asdf"),
            search_fn: |_| vec![],
        }),
    });
    register_generic(reg, core, true, false, false);
}

/// GNU Guix (`guix`). Linux-only; gated by the `guix` binary. Per-user, no root needed.
fn register_guix(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("guix");
    // `guix package -I` lists the profile's manifest — what was explicitly installed —
    // not the store closure behind it.
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Inline("{name}@{version}".into()));
    cfg.install_args = vec!["install".into()];
    cfg.remove_args = vec!["remove".into()];
    cfg.list_args = vec!["package".into(), "-I".into()];
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_args = vec!["upgrade".into()];
    let core = Arc::new(GenericBackendCore {
        name: "guix".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::ws_name_version(o, "guix"),
            search_fn: |o| crate::parsers::ecosystem::guix_search(o, "guix"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Gentoo Portage (`emerge`). Linux-only; gated by the `emerge` binary. Installed packages
/// are listed via `qlist -I` (portage-utils). Needs root and serializes (Portage locks).
fn register_emerge(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("emerge");
    // Portage's @world file (/var/lib/portage/world) is the explicit set; `emerge -I`
    // lists the whole tree (306 packages vs an empty world on the gentoo test image).
    // Wiring the world file is the right fix; until it is verified, adopt nothing.
    cfg.manual = ManualListing::Unsupported;
    cfg.install_args = vec!["--ask=n".into(), "--quiet".into()];
    cfg.remove_args = vec!["--unmerge".into(), "--ask=n".into()];
    cfg.list_binary = Some("qlist".into());
    cfg.list_args = vec!["-I".into()];
    cfg.search_args = vec!["--search".into()];
    cfg.upgrade_args = vec![
        "--update".into(),
        "--deep".into(),
        "--newuse".into(),
        "--ask=n".into(),
        "@world".into(),
    ];
    cfg.needs_root = true;
    cfg.is_exclusive = true;
    let core = Arc::new(GenericBackendCore {
        name: "emerge".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::names_only(o, "emerge"),
            search_fn: |o| crate::parsers::ecosystem::emerge_search(o, "emerge"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Solus eopkg (`eopkg`). Linux-only; gated by the `eopkg` binary. Needs root, serializes.
fn register_eopkg(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("eopkg");
    // eopkg installs dependencies and `list-installed` reports them all.
    cfg.manual = ManualListing::Unsupported;
    cfg.install_args = vec!["install".into(), "-y".into()];
    cfg.remove_args = vec!["remove".into(), "-y".into()];
    cfg.list_args = vec!["list-installed".into()];
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_args = vec!["upgrade".into(), "-y".into()];
    cfg.needs_root = true;
    cfg.is_exclusive = true;
    let core = Arc::new(GenericBackendCore {
        name: "eopkg".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::eopkg_list(o, "eopkg"),
            search_fn: |o| crate::parsers::ecosystem::eopkg_list(o, "eopkg"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

/// Slackware slackpkg (`slackpkg`). Linux-only; gated by the `slackpkg` binary. Installed
/// packages are read from `/var/log/packages`. Needs root, serializes.
fn register_slackpkg(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("slackpkg");
    // Slackware does no dependency resolution: every installed package was chosen.
    cfg.manual = ManualListing::AllInstalled;
    cfg.install_args = vec![
        "-batch=on".into(),
        "-default_answer=y".into(),
        "install".into(),
    ];
    cfg.remove_args = vec![
        "-batch=on".into(),
        "-default_answer=y".into(),
        "remove".into(),
    ];
    cfg.list_binary = Some("ls".into());
    cfg.list_args = vec!["-1".into(), "/var/log/packages".into()];
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_args = vec![
        "-batch=on".into(),
        "-default_answer=y".into(),
        "upgrade-all".into(),
    ];
    cfg.needs_root = true;
    cfg.is_exclusive = true;
    let core = Arc::new(GenericBackendCore {
        name: "slackpkg".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::slackpkg_installed(o, "slackpkg"),
            search_fn: |o| crate::parsers::ecosystem::slackpkg_search(o, "slackpkg"),
        }),
    });
    register_generic(reg, core, true, true, true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LuaHooks;

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

    /// Which backends a bare `linix adopt` declines to take, and why it is a short list.
    ///
    /// Opting out is a real cost to the user — a backend that keeps itself out of `adopt` is a
    /// backend they have to write by hand — so it is spelled out here rather than left to
    /// whoever adds the next one. The bar is not "this list is noisy": it is *being on the
    /// machine is not evidence anybody chose it*, which is true of an init's running services
    /// and of nothing else LiNix drives.
    ///
    /// Measured before the ruling: `adopt` wrote 161 declarations on a Windows host and 150
    /// were services (owner ruling, 2026-08-05 — `Q39`).
    #[tokio::test]
    async fn only_the_backends_that_cannot_know_your_intent_opt_out_of_adopt() {
        let reg = build_registry().await;
        let available = reg.available();
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
        assert_caps(
            &reg,
            "gem",
            &[
                "installable",
                "queryable",
                "searchable",
                "repo_manager",
                "metadata_provider",
            ],
        );
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
            assert_caps(
                &reg,
                "winget",
                &[
                    "installable",
                    "queryable",
                    "searchable",
                    "repo_manager",
                    "metadata_provider",
                ],
            );
            assert_caps(
                &reg,
                "scoop",
                &[
                    "installable",
                    "queryable",
                    "searchable",
                    "repo_manager",
                    "metadata_provider",
                ],
            );
            assert_caps(
                &reg,
                "choco",
                &[
                    "installable",
                    "queryable",
                    "searchable",
                    "upgradable",
                    "repo_manager",
                    "metadata_provider",
                ],
            );
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
    /// A system package manager resolves its own dependency closure, so LiNix must not
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
            ArgvCase::pkg(
                "pacman",
                &|r, e| crate::backends::pacman::register(r, e, &Config::default()),
                Runs("pacman -S --noconfirm --needed jq"),
                Runs("pacman -Rs --noconfirm jq"),
            ),
            ArgvCase::pkg(
                "dnf",
                &|r, e| crate::backends::dnf::register(r, e, &Config::default()),
                Runs("dnf install -y jq"),
                Runs("dnf remove -y jq"),
            ),
            // Void's manager installs and removes with two different programs, which is the
            // `remove_binary` case a single-binary assumption gets wrong.
            ArgvCase::pkg(
                "xbps",
                &|r, e| crate::backends::xbps::register(r, e, &Config::default()),
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
                Runs("flatpak --system install -y --noninteractive -- jq"),
                Runs("flatpak --system uninstall -y --noninteractive -- jq"),
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
                &|r, e| crate::backends::conda::register(r, e, &Config::default()),
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
                "linix-probe",
                &[("url", "https://example.invalid/p.tgz")],
                Runs("helm plugin install -- https://example.invalid/p.tgz"),
                Runs("helm plugin uninstall -- linix-probe"),
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
                "/mnt/linix-probe",
                &[],
                Runs("btrfs subvolume create /mnt/linix-probe"),
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
                Runs(if cfg!(windows) {
                    "sc start nginx"
                } else {
                    "systemctl"
                }),
                Runs(if cfg!(windows) {
                    "sc stop nginx"
                } else {
                    "systemctl"
                }),
            ),
            ArgvCase::shaped(
                "setting",
                &|r, e| crate::backends::setting::register(r, e, &Config::default()),
                if cfg!(windows) {
                    "Software\\LinixProbe/Value"
                } else {
                    "org.linix.probe/key"
                },
                &[("value", "1")],
                Runs(if cfg!(windows) {
                    "reg add HKCU\\Software\\LinixProbe /v Value /d 1 /f"
                } else {
                    "gsettings set org.linix.probe key 1"
                }),
                Runs(if cfg!(windows) {
                    "reg delete HKCU\\Software\\LinixProbe /v Value /f"
                } else {
                    "gsettings reset org.linix.probe key"
                }),
            ),
            // ---- Backends that run no command. Each fetches over HTTP or writes to the
            // filesystem directly, so "no argv" is the right answer and not a hole — but it is
            // asserted, because a download backend that started shelling out to `curl` would
            // otherwise change from "no calls" to "some calls" with nothing watching.
            ArgvCase::shaped(
                "link",
                &|r, e| crate::backends::link::register(r, e, &Config::default()),
                "/tmp/linix-probe-src",
                &[("target", "/tmp/linix-probe-dst")],
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

        for case in argv_cases() {
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
            let removed = inst.remove(&[case.subject.to_string()], false).await;
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
        }
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
        spec.options.insert("version".into(), "2.7.0".into());
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
            spec.options.insert("version".into(), version.into());
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
                    installed_fn: |_| vec![],
                    search_fn: |_| vec![],
                }),
            });
            assert!(
                with_manager_policy(core).executor.classifies_absent_names(),
                "`{name}` has an entry in exit_policy::for_manager and did not carry it. A                  manager that cannot say \"no such package\" leaves the line in the manifest                  and every later command fails on it."
            );
        }
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
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/backends/registry.rs"),
        )
        .expect("read registry.rs");

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
                .add_repo("linixtest", "https://example.invalid/repo", false)
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
    /// The option terminator is LiNix's, and it was measured rather than assumed: both of the
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
                options: std::collections::HashMap::from([(
                    "version".to_string(),
                    "1.6.16".to_string(),
                )]),
                ..Default::default()
            }],
            false,
        )
        .await
        .unwrap();
        inst.remove(&["phx_new".to_string()], false).await.unwrap();

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
    /// machine corrects it that day instead of waiting for a LiNix release.
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
    /// It was a `HashMap`, so the order was Rust's per-process hash seed: `linix list` printed
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
        let avail = names(reg.available());
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
            .available()
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
}
