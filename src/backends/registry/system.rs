//! **The managers that own the machine's own software: apt, pacman, dnf, apk, zypper, xbps,
//! the AUR helpers, macports, pkgin and the two BSD `pkg`s.**
//!
//! What they share is the property that decides how carefully everything else treats them:
//! `needs_root` and `is_exclusive`. These are the managers whose removals can take the
//! operating system with them, which is why the guard, the essential-package query and the
//! manager-lock wait all exist.

// src/backends/registry.rs

use crate::backends::generic::{
    CacheClean, DependsProbe, GenericEnumerable, OrphanDryRun, RepoListing,
};
use crate::backends::generic::{
    GenericBackendCore, GenericInstallable, GenericQueryable, GenericRepoManager,
    GenericSearchable, GenericUpgradable, ManagerConfig, ManualListing, SearchSource, VersionPin,
};
use crate::backends::generic::{ManualFormat, OutdatedProbe};
use crate::core::{BackendCapabilities, CommandExecutor};
use crate::parsers::LambdaParser;
use std::sync::Arc;

use super::{base_config, with_manager_policy, BackendRegistry};

// ============================================================================
// Generic (CLI-config-driven) backend registrations
// ============================================================================

pub(super) fn register_apt(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apt".into(),
        executor: executor.clone(),
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
            // **`${db:Status-Status}` is load-bearing and must not be dropped to tidy the
            // format.** `dpkg-query -W` has no status filter: it lists every package dpkg
            // knows about, and dpkg keeps knowing about one after `apt remove` — the state
            // `deinstall ok config-files`, which `remove_args` below mints on every removal of
            // a package with a file under `/etc`. Without the status field this listing calls
            // those packages installed, so `list` names software that is not there, `check`
            // reports no drift and `sync` will not reinstall it (B0).
            list_args: vec![
                "-W".into(),
                "-f=${db:Status-Status} ${Package} ${Version}\\n".into(),
            ],
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
            //
            // Deliberately NOT status-filtered, unlike `list_args` above. This query builds a
            // protected list, so over-inclusion protects a name that is not there and
            // under-inclusion lets Shall remove something the machine needs. It cannot be
            // wrong in the direction B0 was: dpkg refuses to remove an `Essential: yes`
            // package, so the `config-files` state is unreachable for the rows this keeps.
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
            foreign_args: None,
            repo_add_args: Some(vec!["-y".into(), "{url}".into()]),
            repo_remove_args: Some(vec!["--remove".into(), "-y".into(), "{name}".into()]),
            repo_list_args: None,
            // `add-apt-repository` is its own program. Left as the first *argument* it ran as
            // `apt add-apt-repository -y <url>`, which apt refuses — so repo add and remove
            // could never have worked on apt at all.
            repo_binary: Some("add-apt-repository".into()),
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            // No transitive dependency expansion for apt. apt resolves and installs a
            // package's full dependency closure itself at `apt-get install` time, so Shall
            // re-deriving it is redundant. Worse, the planner's expansion is a recursive
            // BFS (walks jq -> libc6 -> libgcc-s1 -> …), and because apt's local cache lets
            // `apt depends` answer offline, that recursion fans out into hundreds of
            // subprocess calls and effectively hangs `status`/`sync`. It also wrongly tags
            // every transitive dependency as a Shall-managed install. Leave dependency
            // resolution to apt. See the sync harness.
            depends: None,
            // `apt clean` exists on modern apt; `apt-get clean` exists on every apt there has ever
            // been, and it is already this row's binary for `autoremove --dry-run`.
            clean_cache: Some(CacheClean {
                binary: Some("apt-get".into()),
                args: vec!["clean".into()],
            }),
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
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
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

/// Arch. **The row `register_aur_helper` had already been proving for a year.**
///
/// `yay` and `paru` speak pacman's flags and have been data since they were written —
/// `["-S", "--noconfirm", "--needed"]` at their row, character-identical to what `pacman.rs`
/// built by hand two hundred lines away. The module's exemption said the removal guard needed
/// pacman's own essential data; `grep -n essential src/backends/pacman.rs` returned nothing and
/// there was no `essential()` impl to need it. What the hand-written path did have was argv
/// built by hand, and so no `--` before the package name on either verb — the one thing every
/// backend on this path gets without remembering.
pub(super) fn register_pacman(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("pacman");
    cfg.install_args = vec!["-S".into(), "--noconfirm".into(), "--needed".into()];
    cfg.remove_args = vec!["-Rs".into(), "--noconfirm".into()];
    // Arch is rolling: an exact-version pin is not part of its model, which is why the AUR
    // helpers that share this syntax carry no `version_pin` either.
    cfg.list_args = vec!["-Q".into()];
    // `-Qe` = explicitly installed only.
    cfg.manual = ManualListing::Command {
        binary: None,
        args: vec!["-Qe".into()],
        format: ManualFormat::SameAsInstalled,
    };
    // pacman has no per-package essential flag: `base` is a convention and `HoldPkg` is user
    // config, so there is nothing authoritative to query. The same `None` the AUR rows carry.
    cfg.essential_args = None;
    // `-Qmq` prints the installed packages no sync database carries — the AUR set, plus
    // anything built by hand. pacman can remove every one of them and put none of them back,
    // which is why a shared-database row for a foreign package names the helper instead
    // (`J3`). Bare names, one per line.
    cfg.foreign_args = Some(vec!["-Qmq".into()]);
    cfg.search_args = vec!["-Ss".into()];
    // `-Ssq` is the search form that prints bare names from the sync databases with no query —
    // the catalogue, which is what II.15's `re:` expands against. `-Ss` matches descriptions
    // too and cannot answer a name pattern.
    cfg.enumerate_args = Some(vec!["-Ssq".into()]);
    cfg.upgrade_args = vec!["-Syu".into(), "--noconfirm".into()];
    cfg.update_args = Some(vec!["-Sy".into()]);
    // `-Qdtq` prints the orphans themselves, one bare name per line, so there is no prefix to
    // strip. It is a listing rather than a dry run, and `list_orphans` only ever wanted names.
    cfg.orphan_dry_run = Some(OrphanDryRun {
        binary: None,
        args: vec!["-Qdtq".into()],
        removes_line_prefix: String::new(),
    });
    cfg.clean_cache = Some(CacheClean {
        binary: None,
        args: vec!["-Sc".into(), "--noconfirm".into()],
    });
    // Drop-in policy: write `/etc/pacman.d/shall-<name>.conf` and add one `Include =` line to
    // `/etc/pacman.conf`, never rewriting its body. The name lands in a path, so the row asks
    // for `{name_component}` and the generic path refuses anything that could leave the
    // directory.
    cfg.repo_binary = Some("sh".into());
    cfg.repo_add_args = Some(vec![
        "-c".into(),
        "set -e; printf '[%s]\\nServer = %s\\n' '{name_component}' '{url}' \
         > '/etc/pacman.d/shall-{name_component}.conf'; \
         grep -qxF 'Include = /etc/pacman.d/shall-{name_component}.conf' /etc/pacman.conf || \
         printf '\\n%s\\n' 'Include = /etc/pacman.d/shall-{name_component}.conf' \
         >> /etc/pacman.conf"
            .into(),
    ]);
    cfg.repo_remove_args = Some(vec![
        "-c".into(),
        // A `#` delimiter for sed avoids escaping the slashes in the path.
        "rm -f '/etc/pacman.d/shall-{name_component}.conf'; \
         sed -i '\\#Include = /etc/pacman.d/shall-{name_component}.conf#d' /etc/pacman.conf"
            .into(),
    ]);
    // `pacman-conf --repo-list` prints names and nothing else; the mirror for one repository
    // is a second question about that name.
    cfg.repo_list_binary = Some("pacman-conf".into());
    cfg.repo_list_args = Some(vec!["--repo-list".into()]);
    cfg.repo_list_shape =
        RepoListing::NamesThenDetail(vec!["-r".into(), "{name}".into(), "Server".into()]);
    // Reported, never planned from (`Y9`). `-Si`'s `Depends On` row carries SEVERAL names on
    // one line, which the labelled parser — written for apt's one-per-line shape — would read
    // as one.
    cfg.depends = Some(DependsProbe {
        binary: None,
        args: vec!["-Si".into(), "{name}".into()],
        parse: Arc::new(crate::parsers::pacman::parse_depends_on),
    });
    cfg.outdated = Some(OutdatedProbe {
        binary: None,
        args: vec!["-Qu".into()],
        parse: Arc::new(crate::parsers::pacman::parse_pacman_outdated),
        silence_is_none: true,
    });
    cfg.needs_root = true;
    cfg.is_exclusive = true;

    let core = Arc::new(GenericBackendCore {
        name: "pacman".into(),
        executor: executor.clone(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::pacman::parse_list,
            search_fn: crate::parsers::pacman::parse_search,
        }),
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

/// Fedora / RHEL.
///
/// The module's exemption said dnf *"reads its own history to distinguish user-installed from
/// dependency, which is a second command whose output changes what the first one means."* It
/// does not: `rpm -qa` and `dnf repoquery --userinstalled` are two independent commands read by
/// **the same function**, which is `ManualListing::Command { format: SameAsInstalled }` spelled
/// out in Rust. apt's row does the strictly harder version — a third binary printing a
/// different shape — as data.
pub(super) fn register_dnf(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("dnf");
    // Reproducible installs: dnf pins with `name-version`.
    cfg.version_pin = Some(VersionPin::Inline("{name}-{version}".into()));
    cfg.install_args = vec!["install".into(), "-y".into()];
    cfg.remove_args = vec!["remove".into(), "-y".into()];
    // The installed set comes from rpm, which is the database dnf writes into and the one
    // command that answers without touching the network.
    cfg.list_binary = Some("rpm".into());
    cfg.list_args = vec![
        "-qa".into(),
        "--queryformat".into(),
        "%{NAME}|%{VERSION}\n".into(),
    ];
    // The user's own set is dnf's to answer, not rpm's — so the row names the binary rather
    // than falling through to `list_binary`.
    cfg.manual = ManualListing::Command {
        binary: Some("dnf".into()),
        args: vec![
            "repoquery".into(),
            "--userinstalled".into(),
            "--qf".into(),
            "%{name}|%{version}".into(),
        ],
        format: ManualFormat::SameAsInstalled,
    };
    cfg.search_args = vec!["search".into()];
    cfg.upgrade_args = vec!["upgrade".into(), "-y".into()];
    cfg.update_args = Some(vec!["makecache".into()]);
    cfg.orphan_dry_run = Some(OrphanDryRun {
        binary: None,
        args: vec![
            "repoquery".into(),
            "--unneeded".into(),
            "--queryformat".into(),
            "%{name}".into(),
        ],
        removes_line_prefix: String::new(),
    });
    cfg.clean_cache = Some(CacheClean {
        binary: None,
        args: vec!["clean".into(), "all".into()],
    });
    // `config-manager` is a dnf plugin (dnf-plugins-core); removal is deleting the drop-in
    // file, which is `rm`'s job and not dnf's — the same shape apk's row uses for a manager
    // whose sources are a file.
    cfg.repo_add_args = Some(vec![
        "config-manager".into(),
        "--add-repo".into(),
        "{url}".into(),
    ]);
    cfg.repo_remove_args = Some(vec![
        "-f".into(),
        "/etc/yum.repos.d/{name_component}.repo".into(),
    ]);
    // dnf has no verb that removes a repository — `config-manager` only adds and toggles — so
    // the drop-in file is deleted. Adding and removing are two programs, which is why the
    // remove binary is its own field.
    cfg.repo_remove_binary = Some("rm".into());
    cfg.repo_list_args = Some(vec!["repolist".into(), "--all".into()]);
    cfg.depends = Some(DependsProbe {
        binary: None,
        args: vec![
            "repoquery".into(),
            "--requires".into(),
            "--resolve".into(),
            "--queryformat".into(),
            // rpm's own format language. `{name}` here is six characters of `%{name}`, which
            // is why the operand is the argument that IS `{name}` and never one containing it.
            "%{name}".into(),
            "{name}".into(),
        ],
        parse: Arc::new(crate::parsers::dnf::parse_bare_dependency_names),
    });
    // `dnf check-update -q` exits **100** when it finds something. That is dnf saying "there
    // are updates", not a failure, so it goes through the read that judges by whether an
    // answer arrived rather than by the exit code.
    cfg.outdated = Some(OutdatedProbe {
        binary: None,
        args: vec!["check-update".into(), "-q".into()],
        parse: Arc::new(crate::parsers::dnf::parse_dnf_outdated),
        silence_is_none: false,
    });
    cfg.needs_root = true;
    cfg.is_exclusive = true;

    let core = Arc::new(GenericBackendCore {
        name: "dnf".into(),
        executor: executor.clone(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::dnf::parse_rpm_qa(o, "dnf"),
            search_fn: crate::parsers::dnf::parse_dnf_search,
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

/// Void Linux. **Three binaries, which is three fields.**
///
/// The module's exemption named the three — install with `xbps-install`, remove with
/// `xbps-remove`, list with `xbps-query` — as though they were the obstacle. They are
/// `binary`, `remove_binary` and `list_binary`, and `generic.rs`'s own doc comment names
/// OpenBSD's `pkg_add`/`pkg_delete` as exactly this case, two hundred lines above the row that
/// ships it.
///
/// What the module had that the machinery did not was the lock: it made both binaries
/// exclusive over `"xbps"` while the shared path keyed on the program, so converting it before
/// `lock_key` existed would have given install and remove two locks over one package database.
pub(super) fn register_xbps(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let mut cfg = base_config("xbps");
    cfg.binary = Some("xbps-install".into());
    cfg.remove_binary = Some("xbps-remove".into());
    cfg.list_binary = Some("xbps-query".into());
    cfg.search_binary = Some("xbps-query".into());
    // Void is rolling, like Arch: no exact-version pin. The syntax exists — `xbps-install
    // name-1.2.3_1` — but the operand needs the package's *revision* suffix, which `@version=`
    // does not carry and cannot derive, so building `name-1.2.3` would be naming a package that
    // does not exist and hoping. Same shape as Portage's atom needing a category (`Q53`).
    cfg.version_pin = None;
    cfg.install_args = vec!["-Sy".into()];
    cfg.remove_args = vec!["-y".into()];
    cfg.list_args = vec!["-l".into()];
    // `-m` lists only what was registered as manually installed.
    cfg.manual = ManualListing::Command {
        binary: None,
        args: vec!["-m".into()],
        format: ManualFormat::SameAsInstalled,
    };
    cfg.search_args = vec!["-Rs".into()];
    cfg.upgrade_args = vec!["-Suy".into()];
    cfg.update_args = Some(vec!["-S".into()]);
    // `xbps-query -O` prints the orphans, one per line.
    cfg.orphan_dry_run = Some(OrphanDryRun {
        binary: Some("xbps-query".into()),
        args: vec!["-O".into()],
        removes_line_prefix: String::new(),
    });
    // Void empties its cache with the REMOVER, not the installer — which is the whole reason
    // `CacheClean` carries a binary of its own.
    cfg.clean_cache = Some(CacheClean {
        binary: Some("xbps-remove".into()),
        args: vec!["-Oy".into()],
    });
    cfg.depends = Some(DependsProbe {
        binary: Some("xbps-query".into()),
        args: vec!["-x".into(), "{name}".into()],
        parse: Arc::new(crate::parsers::bsd::parse_xbps_dependencies),
    });
    cfg.needs_root = true;
    cfg.is_exclusive = true;

    let core = Arc::new(GenericBackendCore {
        name: "xbps".into(),
        executor: executor.clone(),
        config: cfg,
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::bsd::parse_xbps_list,
            search_fn: crate::parsers::bsd::parse_xbps_search,
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

/// The two AUR helpers, each a two-argument registrar so the argv table can name it.
///
/// They are registered on Linux only, which makes them exactly the class
/// `every_os_native_backend_sends_the_argv_its_manager_expects` exists for — and until these
/// wrappers existed the five-argument `register_aur_helper` could not appear in that table, so
/// neither could they.
pub(super) fn register_yay(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    register_aur_helper(
        reg,
        executor,
        "yay",
        |o| crate::parsers::pacman::parse_list_for(o, "yay"),
        |o| crate::parsers::pacman::parse_search_for(o, "yay"),
    );
}

pub(super) fn register_paru(reg: &mut BackendRegistry, executor: &CommandExecutor) {
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
pub(super) fn register_aur_helper(
    reg: &mut BackendRegistry,
    executor: &CommandExecutor,
    name: &'static str,
    installed_fn: fn(&str) -> crate::parsers::ParseResult,
    search_fn: fn(&str) -> Vec<crate::core::Package>,
) {
    let core = Arc::new(GenericBackendCore {
        name: name.into(),
        // AUR helpers speak pacman's flags, and they speak pacman's complaints too.
        executor: executor.clone(),
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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            needs_root: false,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
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

pub(super) fn register_apk(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "apk".into(),
        executor: executor.clone(),
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
            foreign_args: None,
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
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            // No transitive dependency expansion for apk. `apk info -R <pkg>` emits a
            // header line ("<pkg>-<ver>-<rev> depends on:") plus virtual provider tokens
            // (`so:libc.musl…`, `pc:…`, `cmd:…`) — none of which are installable package
            // names. The generic label-parser would turn the header into a bogus target
            // (`jq-1.8.1-r0`) and the `so:` provides into non-existent packages, so `apk add`
            // would fail with "no such package". apk resolves its own dependency closure at
            // install time, so Shall does not need to expand it. See the sync harness.
            depends: None,
            // apk-cache(8). A host with no cache directory configured has nothing to delete and
            // says so; it is not an error.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["cache".into(), "clean".into()],
            }),
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
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
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
            search_fn: |o| {
                crate::parsers::common::parse_dash_version_list(o, "apk").unwrap_or_default()
            },
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

pub(super) fn register_zypper(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "zypper".into(),
        executor: executor.clone(),
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
            foreign_args: None,
            repo_add_args: Some(vec!["addrepo".into(), "{url}".into(), "{name}".into()]),
            repo_remove_args: Some(vec!["removerepo".into(), "{name}".into()]),
            repo_list_args: Some(vec!["repos".into()]),
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            // None, like apt, dnf and pacman: zypper resolves its own dependency closure at
            // install time, so Shall re-deriving one adds nodes the planner then tries to
            // install by name. What `info --requires` reports are RPM capabilities
            // (`libjq.so.1()(64bit)`), not packages anyone declares — and until 2026-07-30 this
            // was the only system manager that set it, which is why it was the only one whose
            // first real run could not install anything.
            depends: None,
            // `--all` is both the metadata and the package caches. zypper(8).
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["clean".into(), "--all".into()],
            }),
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
                silence_is_none: false,
            }),
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: crate::parsers::dnf::parse_zypper_search,
            search_fn: |o| crate::parsers::dnf::parse_zypper_search(o).unwrap_or_default(),
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

pub(super) fn register_macports(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "macports".into(),
        executor: executor.clone(),
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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
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
pub(super) fn register_pkgin(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkgin".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "pkgin".into(),
            binary: None,
            remove_binary: None,
            // `pkgin install name-1.2.3` — pkgsrc spells a version as the operand's suffix.
            version_pin: Some(VersionPin::Inline("{name}-{version}".into())),
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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            // pkgin(1) — the downloaded binary packages under `/var/db/pkgin/cache`.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["clean".into()],
            }),
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o),
            search_fn: |o| crate::parsers::pkgsrc::parse_pkgin(o).unwrap_or_default(),
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
pub(super) fn register_pkg_freebsd(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkg".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "pkg".into(),
            binary: None,
            remove_binary: None,
            // `pkg install name-1.2.3` — FreeBSD spells a version as the operand's suffix.
            version_pin: Some(VersionPin::Inline("{name}-{version}".into())),
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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            // FreeBSD keeps fetched packages in `/var/cache/pkg`. `-y` because a converger has
            // nobody at the terminal.
            clean_cache: Some(CacheClean {
                binary: None,
                args: vec!["clean".into(), "-y".into()],
            }),
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg(o).unwrap_or_default(),
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
pub(super) fn register_pkg_add_openbsd(reg: &mut BackendRegistry, executor: &CommandExecutor) {
    let core = Arc::new(GenericBackendCore {
        name: "pkg_add".into(),
        executor: executor.clone(),
        config: ManagerConfig {
            name: "pkg_add".into(),
            binary: None,
            // The uninstaller is its own program; `pkg_delete <name>` takes no subcommand, so
            // remove_args stays empty and the separate-binary path in `remove` handles it.
            remove_binary: Some("pkg_delete".into()),
            // `pkg_add name-1.2.3` — OpenBSD spells a version as the operand's suffix.
            version_pin: Some(VersionPin::Inline("{name}-{version}".into())),
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
            foreign_args: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            needs_root: true,
            is_exclusive: true,
            install_source_option: None,
            extra_probes: None,
            upgrade_reinstall_args: None,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
        },
        parser: Arc::new(LambdaParser {
            installed_fn: |o| crate::parsers::bsd::parse_pkg_add(o),
            search_fn: |o| crate::parsers::bsd::parse_pkg_add(o).unwrap_or_default(),
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
