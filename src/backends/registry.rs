// src/backends/registry.rs

use crate::app::LuaHooks;
use crate::backends::generic::ManualFormat;
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable, ManagerConfig, ManualListing, VersionPin,
};
use crate::backends::generic::{GenericEnumerable, OrphanDryRun};
use crate::backends::pip_search::PipSearchable;
use crate::config::Config;
use crate::core::{BackendCapabilities, CommandExecutor};
use crate::parsers::windows;
use crate::parsers::LambdaParser;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::trace;

pub struct BackendRegistry {
    backends: HashMap<String, Arc<BackendCapabilities>>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
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
        self.available()
            .into_iter()
            .filter(|b| enabled.contains(&b.name().to_string()))
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
        register_aur_helper(
            &mut reg,
            &executor,
            "yay",
            |o| crate::parsers::pacman::parse_list_for(o, "yay"),
            |o| crate::parsers::pacman::parse_search_for(o, "yay"),
        );
        register_aur_helper(
            &mut reg,
            &executor,
            "paru",
            |o| crate::parsers::pacman::parse_list_for(o, "paru"),
            |o| crate::parsers::pacman::parse_search_for(o, "paru"),
        );
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
    crate::backends::cargo::register(&mut reg, &executor, config);
    crate::backends::pipx::register(&mut reg, &executor, config);
    crate::backends::uv::register(&mut reg, &executor, config);
    crate::backends::npm::register(&mut reg, &executor, config);
    crate::backends::pnpm::register(&mut reg, &executor, config);
    crate::backends::yarn::register(&mut reg, &executor, config);
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
    #[cfg(target_os = "windows")]
    crate::backends::psresource::register(&mut reg, &executor, config);

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
    crate::backends::pubdart::register(&mut reg, &executor, config);
    crate::backends::krew::register(&mut reg, &executor, config);

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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(crate::parsers::apt::AptParser),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn,
            search_fn,
        }),
    });
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
            depends_args: Some(vec!["info".into(), "--requires".into(), "{name}".into()]),
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dnf::parse_zypper_search,
            search_fn: crate::parsers::dnf::parse_zypper_search,
        }),
    });
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
            version_pin: Some(VersionPin::Flag(vec![
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
            // winget installs no dependencies of its own: everything listed was asked for.
            manual: ManualListing::AllInstalled,
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("winget", o),
            search_fn: |o| windows::parse_search("winget", o),
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("scoop", o),
            search_fn: |o| windows::parse_search("scoop", o),
        }),
    });
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
            version_pin: Some(VersionPin::Flag(vec![
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
            search_args: vec!["search".into()],
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| windows::parse_installed("choco", o),
            search_fn: |o| windows::parse_search("choco", o),
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::macos::parse_mas_list,
            search_fn: crate::parsers::macos::parse_mas_search,
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("pip", o),
            search_fn: |_| vec![],
        }),
    });
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
            version_pin: Some(VersionPin::Flag(vec!["-v".into(), "{version}".into()])),
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("gem", o),
            search_fn: |o| crate::parsers::language::parse_search("gem", o),
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::language::parse_installed("bun", o),
            search_fn: |_| vec![],
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::macos::parse_macports_installed,
            search_fn: crate::parsers::macos::parse_macports_search,
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o),
            search_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o),
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg(o),
        }),
    });
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg_add(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg_add(o),
        }),
    });
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
            version_pin: Some(VersionPin::Flag(vec![
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
            flag_map: HashMap::new(),
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dotnet::parse_dotnet_list,
            search_fn: crate::parsers::dotnet::parse_dotnet_search,
        }),
    });
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
        flag_map: HashMap::new(),
    }
}

/// Register a generic backend, attaching Installable + MetadataProvider always and the
/// other capabilities per the boolean flags. Installable is always present (install is the
/// point); `query`/`search`/`upgrade` are opt-in because not every manager supports them.
#[allow(clippy::fn_params_excessive_bools)]
fn register_generic(
    reg: &mut BackendRegistry,
    core: Arc<GenericBackendCore>,
    query: bool,
    search: bool,
    upgrade: bool,
) {
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
    cfg.version_pin = Some(VersionPin::Flag(vec!["{version}".into()]));
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
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_args = vec!["global".into(), "upgrade-all".into()];
    let core = Arc::new(GenericBackendCore {
        name: "pixi".into(),
        executor: executor.duplicate(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::ecosystem::pixi_list(o, "pixi"),
            search_fn: |o| crate::parsers::ecosystem::names_only(o, "pixi"),
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
    cfg.install_args = vec!["archive.install".into(), "hex".into(), "--force".into()];
    cfg.remove_args = vec!["archive.uninstall".into()];
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
        crate::backends::artifact::capability::install_source_key("helm").map(Into::into);
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
fn register_asdf(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("asdf");
    // asdf lists the tool versions someone explicitly installed; it has no dep concept.
    cfg.manual = ManualListing::AllInstalled;
    cfg.version_pin = Some(VersionPin::Flag(vec!["{version}".into()]));
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
    #[tokio::test]
    async fn every_os_native_backend_sends_the_argv_its_manager_expects() {
        use crate::core::executor::MockExecutor;
        use dashmap::DashMap;

        type Registrar = fn(&mut BackendRegistry, &CommandExecutor);
        // backend, registrar, the install argv, the remove argv.
        let cases: &[(&str, Registrar, &str, &str)] = &[
            // OS-native system managers — each invisible to every platform's CI but its own.
            (
                "apt",
                register_apt,
                "apt install -y -- jq",
                "apt remove -y -- jq",
            ),
            ("apk", register_apk, "apk add -- jq", "apk del -- jq"),
            (
                "zypper",
                register_zypper,
                "zypper install -y",
                "zypper remove -y",
            ),
            (
                "winget",
                register_winget,
                "winget install",
                "winget uninstall",
            ),
            ("scoop", register_scoop, "scoop install", "scoop uninstall"),
            ("choco", register_choco, "choco install", "choco uninstall"),
            ("mas", register_mas, "mas install", "mas uninstall"),
            (
                "macports",
                register_macports,
                "port install",
                "port uninstall",
            ),
            ("guix", register_guix, "guix install", "guix remove"),
            ("emerge", register_emerge, "emerge", "--unmerge"),
            (
                "eopkg",
                register_eopkg,
                "eopkg install -y",
                "eopkg remove -y",
            ),
            (
                "slackpkg",
                register_slackpkg,
                "slackpkg -batch=on",
                "remove",
            ),
            // The BSD tools, where removal is a different program.
            (
                "pkgin",
                register_pkgin,
                "pkgin -y install",
                "pkgin -y remove",
            ),
            (
                "pkg",
                register_pkg_freebsd,
                "pkg install -y",
                "pkg delete -y",
            ),
            ("pkg_add", register_pkg_add_openbsd, "pkg_add", "pkg_delete"),
            // Language and ecosystem managers: the verbs are where a mock sees nothing.
            ("pip", register_pip, "pip install", "pip uninstall"),
            ("gem", register_gem, "gem install", "gem uninstall"),
            ("bun", register_bun, "bun add", "bun remove"),
            (
                "dotnet",
                register_dotnet,
                "dotnet tool install",
                "dotnet tool uninstall",
            ),
            (
                "composer",
                register_composer,
                "composer global require",
                "global remove",
            ),
            ("opam", register_opam, "opam install", "opam remove"),
            (
                "luarocks",
                register_luarocks,
                "luarocks install",
                "luarocks remove",
            ),
            (
                "nimble",
                register_nimble,
                "nimble install",
                "nimble uninstall",
            ),
            (
                "pixi",
                register_pixi,
                "pixi global install",
                "pixi global uninstall",
            ),
            ("spack", register_spack, "spack install", "spack uninstall"),
            (
                "mix",
                register_mix,
                "mix archive.install",
                "mix archive.uninstall",
            ),
            // `helm` is deliberately absent: it installs from an option this table cannot
            // carry, so its install call never happens and the row would pass on the remove
            // alone — a check that tests nothing (IV.1). It has its own tests and a live run.
        ];

        for (name, register, want_install, want_remove) in cases {
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
            let inst = b
                .as_installable()
                .unwrap_or_else(|| panic!("{} cannot install", name));
            let spec = crate::core::PackageSpec {
                name: "jq".into(),
                backend: (*name).into(),
                ..Default::default()
            };
            let _ = inst.install(&[spec], false).await;
            let _ = inst.remove(&["jq".to_string()], false).await;

            let calls = mock.get_calls().await;
            for want in [want_install, want_remove] {
                assert!(
                    calls.iter().any(|c| c.contains(want)),
                    "{}: no call contained `{}`\n  calls: {:?}",
                    name,
                    want,
                    calls
                );
            }
        }
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
}
