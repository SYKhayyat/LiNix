// Package-manager INTERCEPTION — so `apt install`, `pacman -S`, `dnf install`, run by hand
// or by a script, are automatically recorded into Shall. "You don't have to use Shall to use
// Shall": keep your muscle memory, and your declarative state still tracks reality.
//
// This is distinct from `app::hooks` (Lua/Rhai lifecycle scripts). Two interception routes,
// used together (the user picked "both"):
//
//   1. NATIVE hooks — a file dropped into the manager's own hook directory that runs
//      `shall hook-record ...` after every transaction. Fires no matter how the manager
//      was invoked. One generator per manager; we support as many as have a stable
//      hook mechanism (pacman, apt/dpkg, dnf/dnf5, zypper, apk, xbps, portage, eopkg).
//
//   2. SHELL wrappers — shell functions that shadow the manager commands, forward to the
//      real binary, then record. A fallback for managers without a native hook, and the
//      vehicle for AUTO-LEARN: for an *unknown* manager the wrapper diffs the installed set
//      before/after and records whatever changed, detecting the operation from keywords.
//
// Everything user-facing here is thin file I/O; the decision logic (which file, what content,
// is-this-a-local-file, what-operation, what-changed) is pure and unit-tested below.

use std::path::PathBuf;

/// The operation a hook/observation represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookOp {
    Install,
    Remove,
}

impl HookOp {
    pub fn as_str(self) -> &'static str {
        match self {
            HookOp::Install => "install",
            HookOp::Remove => "remove",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "install" => Some(HookOp::Install),
            "remove" => Some(HookOp::Remove),
            _ => None,
        }
    }
}

/// How an install target reached the system, which decides whether Shall tracks it as
/// declarative (repo installs go into the manifest) or protects it as imperative (a local
/// `.deb`/`.rpm`/etc. file is not reproducible from a manifest, so it's pinned, never pruned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// From a configured repository — reproducible, tracked declaratively.
    Repo,
    /// From a local package file (`./foo.deb`, `/tmp/bar.rpm`, an AppImage, …) — imperative.
    LocalFile,
}

/// A native hook file to be written into a manager's hook directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSpec {
    /// Manager name (e.g. "pacman").
    pub manager: &'static str,
    /// Absolute path the hook file should be written to.
    pub path: PathBuf,
    /// The file contents.
    pub content: String,
    /// Whether writing this path typically needs root.
    pub needs_root: bool,
}

/// The full set of native hook files Shall knows how to install, parameterized by the path to
/// the `shall` binary (so the hook can call back into it). Only managers with a stable,
/// documented hook mechanism are represented.
pub fn hook_specs(shall_bin: &str) -> Vec<HookSpec> {
    let mut specs = Vec::new();

    // pacman targets arrive on STDIN (hence NeedsTargets + xargs), not as arguments.
    for (op, pac_ops) in [
        (HookOp::Install, &["Install", "Upgrade"][..]),
        (HookOp::Remove, &["Remove"][..]),
    ] {
        let triggers = pac_ops
            .iter()
            .map(|o| format!("Operation = {o}"))
            .collect::<Vec<_>>()
            .join("\n");
        specs.push(HookSpec {
            manager: "pacman",
            path: PathBuf::from(format!("/etc/pacman.d/hooks/shall-{}.hook", op.as_str())),
            content: format!(
                "# Managed by Shall — records manual pacman operations.\n\
                 [Trigger]\n{triggers}\nType = Package\nTarget = *\n\n\
                 [Action]\n\
                 Description = Shall: recording {} into declarative state\n\
                 When = PostTransaction\n\
                 Exec = /bin/sh -c 'xargs -r {shall_bin} hook-record --manager pacman --op {}'\n\
                 NeedsTargets\n",
                op.as_str(),
                op.as_str(),
            ),
            needs_root: true,
        });
    }

    // dpkg does not hand per-package targets to Post-Invoke, so apt can only reconcile by
    // diffing the installed set — it cannot record specific packages like pacman does.
    specs.push(HookSpec {
        manager: "apt",
        path: PathBuf::from("/etc/apt/apt.conf.d/99shall"),
        content: format!(
            "// Managed by Shall — reconciles declarative state after apt/dpkg transactions.\n\
             DPkg::Post-Invoke {{ \"{shall_bin} hook-reconcile --manager apt || true\"; }};\n"
        ),
        needs_root: true,
    });

    specs.push(HookSpec {
        manager: "dnf",
        path: PathBuf::from("/etc/dnf/plugins/post-transaction-actions.d/shall.action"),
        content: format!(
            "# Managed by Shall — reconcile declarative state after any dnf transaction.\n\
             *:any:{shall_bin} hook-reconcile --manager dnf\n"
        ),
        needs_root: true,
    });

    specs.push(HookSpec {
        manager: "zypper",
        path: PathBuf::from("/usr/lib/zypp/plugins/commit/shall"),
        content: format!(
            "#!/bin/sh\n# Managed by Shall — zypper commit plugin.\n\
             {shall_bin} hook-reconcile --manager zypper >/dev/null 2>&1 || true\n"
        ),
        needs_root: true,
    });

    // apk passes no targets to commit hooks, so this can only reconcile.
    specs.push(HookSpec {
        manager: "apk",
        path: PathBuf::from("/etc/apk/commit_hooks.d/shall.sh"),
        content: format!(
            "#!/bin/sh\n# Managed by Shall — apk commit hook.\n\
             {shall_bin} hook-reconcile --manager apk >/dev/null 2>&1 || true\n"
        ),
        needs_root: true,
    });

    specs.push(HookSpec {
        manager: "xbps",
        path: PathBuf::from("/etc/xbps.d/shall-hook.sh"),
        content: format!(
            "#!/bin/sh\n# Managed by Shall — xbps reconcile helper.\n\
             {shall_bin} hook-reconcile --manager xbps >/dev/null 2>&1 || true\n"
        ),
        needs_root: true,
    });

    specs.push(HookSpec {
        manager: "portage",
        path: PathBuf::from("/etc/portage/env/shall-record.sh"),
        content: format!(
            "#!/bin/sh\n# Managed by Shall — portage post_pkg_postinst reconcile.\n\
             post_pkg_postinst() {{ {shall_bin} hook-reconcile --manager portage >/dev/null 2>&1 || true; }}\n"
        ),
        needs_root: true,
    });

    specs.push(HookSpec {
        manager: "eopkg",
        path: PathBuf::from("/usr/libexec/shall-eopkg-hook.sh"),
        content: format!(
            "#!/bin/sh\n# Managed by Shall — eopkg reconcile helper.\n\
             {shall_bin} hook-reconcile --manager eopkg >/dev/null 2>&1 || true\n"
        ),
        needs_root: true,
    });

    specs
}

/// The managers with a native hook available (names only), for help text and `hooks status`.
pub fn hookable_manager_names() -> Vec<&'static str> {
    // Derive from the spec list so the two never drift.
    let mut names: Vec<&'static str> = hook_specs("shall").iter().map(|s| s.manager).collect();
    names.dedup();
    names
}

/// Classify an install target as a repository package or a local package file. Local files
/// are recognized by a path-like shape (contains a path separator) or a package-file
/// extension. This is what implements "treat `.deb`/`.rpm` installs as imperative".
pub fn classify_install_target(target: &str) -> InstallKind {
    let t = target.trim();
    // A path separator strongly implies a local file (`./x.deb`, `/tmp/x.rpm`, `C:\x.msi`).
    if t.contains('/') || t.contains('\\') {
        return InstallKind::LocalFile;
    }
    // Otherwise, a known package-file extension.
    let lower = t.to_lowercase();
    const LOCAL_EXTS: [&str; 11] = [
        ".deb",
        ".rpm",
        ".apk",
        ".snap",
        ".flatpak",
        ".flatpakref",
        ".appimage",
        ".pkg.tar.zst",
        ".pkg.tar.xz",
        ".msi",
        ".xbps",
    ];
    if LOCAL_EXTS.iter().any(|ext| lower.ends_with(ext)) {
        InstallKind::LocalFile
    } else {
        InstallKind::Repo
    }
}

/// The package name to record for a local file, stripped of directory and archive extension
/// (`/tmp/htop_3.0_amd64.deb` -> `htop_3.0_amd64`). Best-effort — restore never depends on it.
pub fn local_file_stem(target: &str) -> String {
    let base = target
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(target)
        .to_string();
    // Strip a trailing known extension (longest match first for `.pkg.tar.zst`).
    let lower = base.to_lowercase();
    for ext in [".pkg.tar.zst", ".pkg.tar.xz"] {
        if lower.ends_with(ext) {
            return base[..base.len() - ext.len()].to_string();
        }
    }
    if let Some(dot) = base.rfind('.') {
        return base[..dot].to_string();
    }
    base
}

/// Detect which operation a raw manager command line represents, by scanning its arguments
/// for install/remove keywords across the common managers. Returns `None` for read-only or
/// unrecognized commands (so autolearn stays quiet on `apt list`, `pacman -Q`, etc.).
///
/// This is the keyword heuristic behind auto-learn: "check for words like install in the
/// input … same for words like uninstall."
pub fn detect_operation(argv: &[String]) -> Option<HookOp> {
    // Long-form keywords (subcommands) for install/remove across managers.
    const INSTALL_WORDS: [&str; 6] = ["install", "add", "in", "get", "reinstall", "emerge"];
    const REMOVE_WORDS: [&str; 8] = [
        "remove",
        "uninstall",
        "erase",
        "del",
        "delete",
        "purge",
        "rm",
        "autoremove",
    ];

    let mut op: Option<HookOp> = None;
    for arg in argv.iter().skip(1) {
        let a = arg.to_lowercase();
        if a.starts_with('-') {
            // Short flags: pacman `-S`/`-R`, apk `add`/`del`, xbps uses separate binaries.
            // pacman: -S install, -R remove (also -Rns etc.).
            if a.starts_with("-s") || a.starts_with("-u") {
                op = Some(HookOp::Install);
            } else if a.starts_with("-r") || a.starts_with("-e") {
                op = Some(HookOp::Remove);
            }
            continue;
        }
        if INSTALL_WORDS.contains(&a.as_str()) {
            return Some(HookOp::Install);
        }
        if REMOVE_WORDS.contains(&a.as_str()) {
            return Some(HookOp::Remove);
        }
    }
    op
}

/// Extract the package-name operands from a raw manager command line: drop the binary, any
/// flags (`-x`/`--long`), and the install/remove subcommand keyword itself. What remains are
/// the targets the user asked to install or remove. Used by `hook-observe` to record a
/// command after the shell wrapper ran it.
pub fn extract_targets(argv: &[String]) -> Vec<String> {
    const KEYWORDS: [&str; 14] = [
        "install",
        "add",
        "in",
        "get",
        "reinstall",
        "emerge",
        "remove",
        "uninstall",
        "erase",
        "del",
        "delete",
        "purge",
        "rm",
        "autoremove",
    ];
    argv.iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .filter(|a| !KEYWORDS.contains(&a.to_lowercase().as_str()))
        .cloned()
        .collect()
}

/// Diff two installed-package name lists (before vs after a transaction) into (added, removed).
/// This is the autolearn core: whatever appeared is an install; whatever vanished is a remove.
pub fn diff_installed(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    use std::collections::HashSet;
    let before_set: HashSet<&String> = before.iter().collect();
    let after_set: HashSet<&String> = after.iter().collect();
    let added: Vec<String> = after
        .iter()
        .filter(|p| !before_set.contains(*p))
        .cloned()
        .collect();
    let removed: Vec<String> = before
        .iter()
        .filter(|p| !after_set.contains(*p))
        .cloned()
        .collect();
    (added, removed)
}

/// Generate shell functions that shadow the given managers so manual use is recorded, with an
/// auto-learn fallback wrapper for any command. Supports bash/zsh (POSIX-ish) syntax.
pub fn shell_wrappers(shall_bin: &str, shell: &str) -> String {
    // Known managers we wrap directly (name -> real binary is the same name via `command`).
    let managers = [
        "apt",
        "apt-get",
        "dnf",
        "yum",
        "pacman",
        "zypper",
        "apk",
        "xbps-install",
    ];
    let mut out = String::new();
    out.push_str(&format!(
        "# Shall shell integration ({shell}). Source this from your rc file:\n\
         #   eval \"$({shall_bin} hooks shell-init {shell})\"\n\n"
    ));
    for m in managers {
        // Each wrapper: run the real command; on success, let Shall reconcile that manager.
        out.push_str(&format!(
            "{m}() {{\n  command {m} \"$@\"; local rc=$?\n  \
             if [ $rc -eq 0 ]; then {shall_bin} hook-observe --manager {m} -- {m} \"$@\" >/dev/null 2>&1 || true; fi\n  \
             return $rc\n}}\n"
        ));
    }
    // A generic auto-learn helper the user can prefix onto ANY unknown manager:
    //   shalllearn some-new-pm install foo
    out.push_str(&format!(
        "\n# Auto-learn any other manager: prefix its command with `shalllearn`.\n\
         shalllearn() {{ {shall_bin} hook-observe --learn -- \"$@\"; }}\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn hook_specs_reference_the_binary_and_cover_many_managers() {
        let specs = hook_specs("/usr/bin/shall");
        let managers: Vec<&str> = specs.iter().map(|s| s.manager).collect();
        for expected in [
            "pacman", "apt", "dnf", "zypper", "apk", "xbps", "portage", "eopkg",
        ] {
            assert!(managers.contains(&expected), "missing hook for {expected}");
        }
        // Every hook actually invokes the shall binary.
        assert!(specs.iter().all(|s| s.content.contains("/usr/bin/shall")));
        // pacman gets both an install and a remove hook.
        assert_eq!(specs.iter().filter(|s| s.manager == "pacman").count(), 2);
    }

    #[test]
    fn pacman_hook_passes_targets_and_sets_operation() {
        let specs = hook_specs("shall");
        let install = specs
            .iter()
            .find(|s| s.manager == "pacman" && s.content.contains("--op install"))
            .unwrap();
        assert!(install.content.contains("NeedsTargets"));
        assert!(install.content.contains("Operation = Install"));
        assert!(install.content.contains("Operation = Upgrade"));
        assert!(install.content.contains("xargs"));
    }

    #[test]
    fn local_file_detection_by_path_and_extension() {
        assert_eq!(
            classify_install_target("./htop.deb"),
            InstallKind::LocalFile
        );
        assert_eq!(
            classify_install_target("/tmp/x.rpm"),
            InstallKind::LocalFile
        );
        assert_eq!(
            classify_install_target("C:\\pkgs\\a.msi"),
            InstallKind::LocalFile
        );
        assert_eq!(
            classify_install_target("foo-1.0.pkg.tar.zst"),
            InstallKind::LocalFile
        );
        assert_eq!(classify_install_target("ripgrep"), InstallKind::Repo);
        assert_eq!(classify_install_target("apt:curl"), InstallKind::Repo);
    }

    #[test]
    fn local_file_stem_strips_dir_and_extension() {
        assert_eq!(local_file_stem("/tmp/htop_3.0_amd64.deb"), "htop_3.0_amd64");
        assert_eq!(local_file_stem("foo-1.0.pkg.tar.zst"), "foo-1.0");
        assert_eq!(local_file_stem("bar.rpm"), "bar");
        assert_eq!(local_file_stem("plainname"), "plainname");
    }

    #[test]
    fn detect_operation_long_form() {
        assert_eq!(
            detect_operation(&s(&["apt", "install", "curl"])),
            Some(HookOp::Install)
        );
        assert_eq!(
            detect_operation(&s(&["dnf", "remove", "nano"])),
            Some(HookOp::Remove)
        );
        assert_eq!(
            detect_operation(&s(&["apt-get", "purge", "x"])),
            Some(HookOp::Remove)
        );
        assert_eq!(
            detect_operation(&s(&["apk", "add", "jq"])),
            Some(HookOp::Install)
        );
        assert_eq!(
            detect_operation(&s(&["brew", "uninstall", "wget"])),
            Some(HookOp::Remove)
        );
    }

    #[test]
    fn detect_operation_pacman_short_flags() {
        assert_eq!(
            detect_operation(&s(&["pacman", "-S", "htop"])),
            Some(HookOp::Install)
        );
        assert_eq!(
            detect_operation(&s(&["pacman", "-Rns", "htop"])),
            Some(HookOp::Remove)
        );
        assert_eq!(
            detect_operation(&s(&["pacman", "-Syu"])),
            Some(HookOp::Install)
        );
    }

    #[test]
    fn detect_operation_ignores_read_only() {
        assert_eq!(detect_operation(&s(&["apt", "list", "--installed"])), None);
        assert_eq!(detect_operation(&s(&["pacman", "-Q"])), None);
        assert_eq!(detect_operation(&s(&["dnf", "search", "editor"])), None);
    }

    #[test]
    fn diff_installed_finds_added_and_removed() {
        let before = s(&["curl", "nano", "htop"]);
        let after = s(&["curl", "htop", "ripgrep"]);
        let (added, removed) = diff_installed(&before, &after);
        assert_eq!(added, vec!["ripgrep"]);
        assert_eq!(removed, vec!["nano"]);
    }

    #[test]
    fn diff_installed_no_change() {
        let list = s(&["a", "b"]);
        let (added, removed) = diff_installed(&list, &list);
        assert!(added.is_empty() && removed.is_empty());
    }

    #[test]
    fn shell_wrappers_wrap_known_managers_and_add_learn_helper() {
        let w = shell_wrappers("shall", "bash");
        assert!(w.contains("apt()"));
        assert!(w.contains("pacman()"));
        assert!(w.contains("command apt \"$@\""));
        assert!(w.contains("shalllearn()"));
        assert!(w.contains("hook-observe"));
    }

    #[test]
    fn extract_targets_drops_binary_flags_and_keyword() {
        assert_eq!(
            extract_targets(&s(&["apt", "install", "-y", "curl", "htop"])),
            vec!["curl", "htop"]
        );
        assert_eq!(
            extract_targets(&s(&["pacman", "-S", "ripgrep"])),
            vec!["ripgrep"]
        );
        assert_eq!(
            extract_targets(&s(&["dnf", "remove", "--assumeyes", "nano"])),
            vec!["nano"]
        );
    }

    #[test]
    fn hook_op_round_trips() {
        assert_eq!(HookOp::parse("install"), Some(HookOp::Install));
        assert_eq!(HookOp::parse("remove"), Some(HookOp::Remove));
        assert_eq!(HookOp::parse("nonsense"), None);
        assert_eq!(HookOp::Install.as_str(), "install");
    }
}
