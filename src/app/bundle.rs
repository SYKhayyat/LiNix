// src/app/bundle.rs
//
// Offline / air-gapped bundling. `shall bundle` packs a portable copy of the declarative
// configuration (manifests, modules, lockfile, keep-list, config) plus a resolved package
// list, so an environment can be reproduced on a disconnected machine. With `--artifacts`
// it additionally tries to pre-download package files for the backends that support an
// offline fetch — honestly reporting which backends it cannot bundle.

use crate::app::App;
use crate::core::{Error, Result};
use crate::model::Writes;
use crate::utils::file::copy_over;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct BundleReport {
    pub out: PathBuf,
    pub files_copied: usize,
    pub package_count: usize,
    pub artifacts_fetched: Vec<String>,
    pub artifacts_skipped: Vec<String>,
    /// True if a `git bundle` of the manifest history was included (repo with commits).
    pub git_history_included: bool,
    /// True if the ownership registry (`registry.json`) was copied.
    pub registry_included: bool,
    /// Set when `--archive` was requested: the single `.tar.gz` produced, and its size.
    pub archive: Option<(PathBuf, u64)>,
    /// True when nothing was written (Q15). Carried on the report rather than read from the
    /// flag a second time, so the sentence a user sees is decided by what the writer did.
    pub previewed: bool,
}

/// Pure: the command to *download* (not install) a package into `dest`, for backends that
/// support an offline fetch. `None` means the backend has no offline-download mode, so it is
/// bundled by declaration only. Unit tested.
pub fn offline_fetch_command(
    backend: &str,
    name: &str,
    dest: &str,
) -> Option<(String, Vec<String>)> {
    // The flags of the fetch command belong to the manager; the package name is a bare word
    // that must not be read as one of them. So each invocation ends its options with `--`
    // before the name — which also means a flag can never trail the name (pip's `-d <dest>`
    // moves ahead of it here).
    let v = |binary: &str, flags: &[&str], name: &str| {
        let mut args: Vec<String> = flags.iter().map(|s| s.to_string()).collect();
        crate::core::argv::push_names(&mut args, binary, [name]);
        Some((binary.to_string(), args))
    };
    match backend {
        "apt" => v("apt-get", &["download"], name), // downloads into the working dir
        "dnf" => v("dnf", &["download", "--destdir", dest], name),
        "pip" | "pipx" => v("pip", &["download", "-d", dest], name),
        "npm" | "pnpm" | "yarn" | "bun" => v("npm", &["pack"], name), // into working dir
        "brew" => v("brew", &["fetch"], name),
        "pacman" => v("pacman", &["-Sw", "--noconfirm"], name),
        "apk" => v("apk", &["fetch"], name),
        _ => None,
    }
}

/// Recursively copy a directory tree, returning the number of files copied. Missing sources
/// are a no-op (return 0), not an error.
async fn copy_dir_recursive(
    src: &Path,
    dst: &Path,
    skip: Option<&Path>,
    writes: Writes,
) -> Result<usize> {
    if !src.exists() {
        return Ok(0);
    }
    // Canonicalize the skip target once so we can recognize it no matter how it's spelled. This
    // is what stops `bundle --out <dir-inside-config>` from copying the bundle into itself (a
    // runaway recursion): the output dir lives under `src`, so we must not descend into it.
    let skip_canon = skip.and_then(|p| std::fs::canonicalize(p).ok());
    let mut count = 0;
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((s, d)) = stack.pop() {
        writes.mkdir(&d).await?;
        let mut rd = tokio::fs::read_dir(&s).await.map_err(Error::from)?;
        while let Some(entry) = rd.next_entry().await.map_err(Error::from)? {
            let ft = entry.file_type().await.map_err(Error::from)?;
            let from = entry.path();
            // Skip the bundle output dir if it happens to sit inside the source tree.
            if ft.is_dir()
                && skip_canon.is_some()
                && std::fs::canonicalize(&from).ok() == skip_canon
            {
                continue;
            }
            let to = d.join(entry.file_name());
            if ft.is_dir() {
                stack.push((from, to));
            } else {
                writes.copy(&from, &to).await?;
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Run a download command inside `dir` (for tools that write to the working directory).
async fn run_in_dir(prog: &str, args: &[&str], dir: &Path) -> bool {
    use tokio::process::Command;
    let mut cmd = Command::new(prog);
    // curl and wget both ask for credentials on a 401. `supervised_output` closes stdin, so the
    // question is captured rather than asked, and the bound is what ends it.
    cmd.args(args).current_dir(dir);
    crate::core::supervise::supervised_output(cmd, prog, false)
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Build the offline bundle at `out`. `plan_json`, if given, is written as `plan.json` inside
/// the bundle (a frozen plan the target can review/apply offline). With `archive`, the finished
/// directory is also packed into a single portable `<out>.tar.gz` (kept alongside the dir).
pub async fn create_bundle(
    app: &App,
    out: &Path,
    include_artifacts: bool,
    archive: bool,
    plan_json: Option<&str>,
) -> Result<BundleReport> {
    let writes = Writes::for_run(app.config.dry_run);
    writes.mkdir(out).await?;
    let mut report = BundleReport {
        out: out.to_path_buf(),
        previewed: writes.previewing(),
        ..Default::default()
    };

    // Your repo, whole. Under II.1 the repo IS the config root — modules, profiles,
    // `active`, `priority`, `locks/`, `preferences.toml` — so copying two named folders
    // out of it silently left the rest behind, and a bundle that restores half your
    // declarations is worse than one that fails.
    let root = app.config.config_root();
    report.files_copied += copy_dir_recursive(&root, out, Some(out), writes).await?;

    // The manifest HISTORY, not just the current files: a `git bundle` carries every commit,
    // so the far side can `rollback` to any past state, not only restore what's current. It is
    // honest about the miss — if the config is not a git repo (or has no commits) the bundle
    // simply reports history was not included, rather than pretending.
    // A `git bundle` is a write like any other, so a preview asks whether there is history to
    // carry without producing the file. `has_commits` is the same question `bundle` answers
    // with its `Ok(true)`, asked without the side effect.
    let history = if writes.previewing() {
        app.git_manager().has_commits()
    } else {
        matches!(
            app.git_manager().bundle(&out.join("config.bundle")),
            Ok(true)
        )
    };
    if history {
        report.git_history_included = true;
        report.files_copied += 1;
    }

    // The ownership registry (`registry.json`), which lives in the data root (II.1), NOT the
    // config repo — so the config-root copy above never included it. Without it the far side
    // knows what to install but not what Shall considers *its own* to manage.
    {
        let registry_path = {
            let state = app.state.lock().await;
            state.path.clone()
        };
        if tokio::fs::try_exists(&registry_path).await.unwrap_or(false)
            && writes
                .copy(&registry_path, &out.join("registry.json"))
                .await
                .is_ok()
        {
            report.registry_included = true;
            report.files_copied += 1;
        }
    }

    let managed: Vec<(String, String, Option<String>)> = {
        let state = app.state.lock().await;
        state
            .packages
            .iter()
            .map(|p| (p.backend.clone(), p.name.clone(), p.version.clone()))
            .collect()
    };
    report.package_count = managed.len();
    let pkgs: Vec<_> = managed
        .iter()
        .map(|(b, n, v)| json!({ "backend": b, "name": n, "version": v }))
        .collect();
    writes
        .write(
            &out.join("packages.json"),
            &serde_json::to_string_pretty(&json!({ "packages": pkgs }))?,
        )
        .await?;

    // Artifact pre-fetch downloads real files from the network. A preview reports what it
    // would fetch and fetches nothing — this is the one part of a bundle whose side effects
    // reach outside the output directory.
    if include_artifacts && writes.previewing() {
        for (backend, name, _) in &managed {
            match offline_fetch_command(backend, name, "") {
                Some(_) => report
                    .artifacts_fetched
                    .push(format!("{}:{}", backend, name)),
                None => report.artifacts_skipped.push(format!(
                    "{}:{} (no offline fetch for backend '{}')",
                    backend, name, backend
                )),
            }
        }
    } else if include_artifacts {
        let dest_root = out.join("artifacts");
        crate::utils::file::ensure_dir_async(&dest_root).await?;
        for (backend, name, _) in &managed {
            let dest = dest_root.join(backend);
            match offline_fetch_command(backend, name, &dest.to_string_lossy()) {
                Some((prog, args)) => {
                    tokio::fs::create_dir_all(&dest).await.ok();
                    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                    if run_in_dir(&prog, &arg_refs, &dest).await {
                        report
                            .artifacts_fetched
                            .push(format!("{}:{}", backend, name));
                    } else {
                        report
                            .artifacts_skipped
                            .push(format!("{}:{} (fetch failed)", backend, name));
                    }
                }
                None => report.artifacts_skipped.push(format!(
                    "{}:{} (no offline fetch for backend '{}')",
                    backend, name, backend
                )),
            }
        }
    }

    let restore = format!(
        "# Shall offline bundle\n\n\
         Packages: {}\nConfig files: {}\nArtifacts pre-fetched: {}\n\n\
         ## Restore on the target machine\n\n\
         1. Copy this directory to the machine.\n\
         2. `shall restore <dir>` — puts the declarations, `locks/` and registry back.\n\
            It refuses a config directory that already has something in it; `--force`\n\
            overwrites.\n\
         3. Reproduce the exact versions:  `shall sync --locked`\n\
            (`locks/versions.json` pins every version).\n\n\
         If you bundled with `--artifacts`, the `artifacts/<backend>/` folders hold the\n\
         downloaded package files for a fully air-gapped install; point your package manager\n\
         at them as a local source.\n",
        report.package_count,
        report.files_copied,
        if include_artifacts {
            report.artifacts_fetched.len()
        } else {
            0
        },
    );
    writes.write(&out.join("RESTORE.md"), &restore).await?;

    // 4b. Frozen plan, so the target can review/apply it offline (before archiving, so it is
    // captured inside the tarball).
    if let Some(pj) = plan_json {
        writes.write(&out.join("plan.json"), pj).await?;
    }

    // The tar stores everything under one top folder named after the bundle dir; without
    // that prefix it would unpack loose files into the extractor's cwd.
    if archive && writes.previewing() {
        report.archive = Some((PathBuf::from(format!("{}.tar.gz", out.display())), 0));
    } else if archive {
        let root_name = out
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "shall-bundle".to_string());
        let tar_path = PathBuf::from(format!("{}.tar.gz", out.display()));
        let src = out.to_path_buf();
        let dest = tar_path.clone();
        // create_tar_gz is blocking (std::fs); keep it off the async reactor.
        let size = tokio::task::spawn_blocking(move || {
            crate::utils::archive::create_tar_gz(&src, &dest, &root_name)
        })
        .await
        .map_err(|e| Error::Other(format!("archive task join error: {e}")))??;
        report.archive = Some((tar_path, size));
    }

    Ok(report)
}

/// What a restore did, for honest reporting.
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub config_files: usize,
    pub registry_restored: bool,
    pub git_history_present: bool,
}

/// Files a bundle carries that describe the bundle rather than being part of the config, so
/// `restore` copies everything else into the config root but not these.
const BUNDLE_META: &[&str] = &[
    "config.bundle",
    "registry.json",
    "packages.json",
    "plan.json",
    "RESTORE.md",
];

/// Put a bundle back (V.59): the other half of `bundle`, a command rather than a README.
///
/// `bundle` packs the config root, `locks/`, the registry and the git history; this reverses
/// it. The config directory it writes into must be empty unless `force` is set, because the
/// machine you reach for a backup on usually still has something on it, and a restore that
/// silently merged over a live config would be the worst kind of surprise.
///
/// It restores files; it does not sync. `sync --locked` afterward reproduces the exact
/// versions from the restored `locks/`.
pub async fn restore_bundle(
    bundle_dir: &Path,
    config_root: &Path,
    registry_path: &Path,
    force: bool,
) -> Result<RestoreReport> {
    if !bundle_dir.join("packages.json").exists() && !bundle_dir.join("modules").exists() {
        return Err(Error::Other(format!(
            "{} does not look like a Shall bundle — no `packages.json` and no `modules/`.",
            bundle_dir.display()
        )));
    }

    if !force && dir_has_entries(config_root).await {
        // `Error::Refused`, not `Other`: Shall worked correctly and declined on purpose, which
        // readme.md's table calls exit 3. It said "refuses" where the rest of the family says
        // "refusing to", which is how it survived the round-2 sweep of exactly this class.
        return Err(Error::Refused(format!(
            "{} is not empty. A restore writes your declarations over what is there, so it \
             refuses unless you pass --force.",
            config_root.display()
        )));
    }

    let mut report = RestoreReport {
        git_history_present: bundle_dir.join("config.bundle").exists(),
        ..Default::default()
    };

    // Copy the config, entry by entry, skipping the bundle's own metadata files so the
    // restored root is a config root and not a bundle.
    crate::utils::file::ensure_dir_async(config_root).await?;
    let mut rd = tokio::fs::read_dir(bundle_dir).await.map_err(Error::from)?;
    while let Some(entry) = rd.next_entry().await.map_err(Error::from)? {
        let name = entry.file_name();
        if BUNDLE_META.contains(&name.to_string_lossy().as_ref())
            || name.to_string_lossy() == "artifacts"
        {
            continue;
        }
        let from = entry.path();
        let to = config_root.join(&name);
        if entry.file_type().await.map_err(Error::from)?.is_dir() {
            report.config_files += copy_dir_recursive(&from, &to, None, Writes::ToDisk).await?;
        } else {
            if let Some(p) = to.parent() {
                crate::utils::file::ensure_dir_async(p).await?;
            }
            copy_over(&from, &to).await?;
            report.config_files += 1;
        }
    }

    // The registry lives in the data root, not the config repo, so it is restored separately.
    let bundled_registry = bundle_dir.join("registry.json");
    if bundled_registry.exists() {
        if let Some(p) = registry_path.parent() {
            crate::utils::file::ensure_dir_async(p).await?;
        }
        copy_over(&bundled_registry, registry_path).await?;
        report.registry_restored = true;
    }

    Ok(report)
}

/// Whether a directory exists and holds at least one entry. A missing directory is empty.
async fn dir_has_entries(dir: &Path) -> bool {
    match tokio::fs::read_dir(dir).await {
        Ok(mut rd) => rd.next_entry().await.ok().flatten().is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restore_refuses_a_nonempty_config_unless_forced() {
        let tmp = std::env::temp_dir().join(format!("shall-restore-{}", std::process::id()));
        let bundle = tmp.join("bundle");
        let cfg = tmp.join("cfg");
        let data = tmp.join("data");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(bundle.join("modules")).unwrap();
        std::fs::write(bundle.join("modules/tools.txt"), "apt:jq\n").unwrap();
        std::fs::write(bundle.join("active"), "Work\n").unwrap();
        std::fs::write(bundle.join("packages.json"), "{\"packages\":[]}").unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join("something"), "keep me").unwrap();

        let reg = data.join("registry.json");
        let refused = restore_bundle(&bundle, &cfg, &reg, false).await;
        assert!(refused.is_err(), "a non-empty config must be refused");
        // And refused as a REFUSAL: `Error::Other` here exited 1, which readme.md's table
        // defines as "Shall could not carry it out", and never fired `on_guard_refusal`.
        assert!(
            matches!(refused, Err(Error::Refused(_))),
            "a deliberate refusal is `Error::Refused` (exit 3), got {refused:?}"
        );

        // Into a clean directory it restores the declarations.
        let clean = tmp.join("clean");
        let report = restore_bundle(&bundle, &clean, &reg, false).await.unwrap();
        assert!(report.config_files >= 2);
        assert_eq!(
            std::fs::read_to_string(clean.join("modules/tools.txt")).unwrap(),
            "apt:jq\n"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `--force` restores over a config that is already there — which is the only reason it
    /// exists — and a restored tree contains read-only files by construction: `bundle` copies
    /// the whole config root, that root is a git repo, and git writes its objects at 0444.
    /// `fs::copy` carries those bits across, so the second restore meets a destination it
    /// cannot open for writing. Root does not notice (it bypasses the check), which is why
    /// every container run was green and only the macOS sweep failed.
    #[tokio::test]
    async fn force_restores_over_files_the_first_restore_left_read_only() {
        let tmp = std::env::temp_dir().join(format!("shall-restore-ro-{}", std::process::id()));
        let bundle = tmp.join("bundle");
        let cfg = tmp.join("cfg");
        let data = tmp.join("data");
        let _ = std::fs::remove_dir_all(&tmp);
        // A git-shaped bundle: a nested read-only object, and an ordinary file beside it.
        std::fs::create_dir_all(bundle.join(".git/objects/ab")).unwrap();
        std::fs::create_dir_all(bundle.join("modules")).unwrap();
        std::fs::write(bundle.join("modules/tools.txt"), "apt:jq\n").unwrap();
        std::fs::write(bundle.join("packages.json"), "{\"packages\":[]}").unwrap();
        let obj = bundle.join(".git/objects/ab/cdef");
        std::fs::write(&obj, "object").unwrap();
        let mut perms = std::fs::metadata(&obj).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&obj, perms).unwrap();

        let reg = data.join("registry.json");
        restore_bundle(&bundle, &cfg, &reg, false)
            .await
            .expect("first restore into a clean directory");
        assert!(
            std::fs::metadata(cfg.join(".git/objects/ab/cdef"))
                .unwrap()
                .permissions()
                .readonly(),
            "the copy must carry the read-only bit across, or this test proves nothing"
        );

        restore_bundle(&bundle, &cfg, &reg, true)
            .await
            .expect("--force must overwrite a read-only file, not fail with EACCES");
        assert_eq!(
            std::fs::read_to_string(cfg.join("modules/tools.txt")).unwrap(),
            "apt:jq\n"
        );

        // Windows refuses to delete a read-only file, so the tree has to be made writable
        // before cleanup. Unix needs nothing: removal is governed by the directory's mode,
        // not the file's — and `set_readonly(false)` there would hand write to everyone.
        #[cfg(windows)]
        for f in [cfg.join(".git/objects/ab/cdef"), obj.clone()] {
            if let Ok(m) = std::fs::metadata(&f) {
                let mut p = m.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                p.set_readonly(false);
                let _ = std::fs::set_permissions(&f, p);
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// An I/O failure that names no file is a failure nobody can act on. This is the error
    /// that cost a CI round: `Permission denied (os error 13)`, on one of several hundred
    /// copied paths, with nothing to say which.
    #[tokio::test]
    async fn a_copy_that_fails_names_the_file() {
        let tmp = std::env::temp_dir().join(format!("shall-copy-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let missing = tmp.join("no-such-source");
        let err = copy_over(&missing, &tmp.join("dest"))
            .await
            .expect_err("copying a file that is not there must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("no-such-source"),
            "the error must name the path it could not copy, got: {msg}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn offline_fetch_covers_common_backends() {
        assert_eq!(
            offline_fetch_command("apt", "curl", "/d"),
            Some((
                "apt-get".into(),
                vec!["download".into(), "--".into(), "curl".into()]
            ))
        );
        // The name lands behind the terminator, with `-d <dest>` moved ahead of it so no flag
        // trails the name.
        assert_eq!(
            offline_fetch_command("pip", "requests", "/d"),
            Some((
                "pip".into(),
                vec![
                    "download".into(),
                    "-d".into(),
                    "/d".into(),
                    "--".into(),
                    "requests".into()
                ]
            ))
        );
        // No terminator here, unlike apt and pip above, and it is not an omission: `dnf` is
        // dnf5 on Fedora 41+, whose parser refuses `--`. Same table, same reason, and this is
        // the third place the same fact had to be written down — which is how the argv table
        // came to be the one source the others are checked against.
        assert_eq!(
            offline_fetch_command("dnf", "curl", "/d"),
            Some((
                "dnf".into(),
                vec![
                    "download".into(),
                    "--destdir".into(),
                    "/d".into(),
                    "curl".into()
                ]
            ))
        );
        // pnpm/yarn/bun all route through `npm pack`
        assert_eq!(
            offline_fetch_command("yarn", "left-pad", "/d").unwrap().0,
            "npm"
        );
    }

    #[test]
    fn backends_without_offline_fetch_return_none() {
        assert_eq!(offline_fetch_command("cargo", "ripgrep", "/d"), None);
        assert_eq!(offline_fetch_command("winget", "Foo", "/d"), None);
        assert_eq!(offline_fetch_command("service", "nginx", "/d"), None);
    }

    #[tokio::test]
    async fn copy_dir_recursive_handles_missing_source() {
        let n = copy_dir_recursive(
            Path::new("/nonexistent/xyz"),
            Path::new("/tmp/whatever"),
            None,
            Writes::ToDisk,
        )
        .await
        .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn copy_dir_recursive_skips_nested_output_dir() {
        // src contains a file AND the destination dir (out) nested inside it. Without the skip,
        // copying src -> out/groups would recurse into out forever. With it, only the real file
        // is copied and the run terminates.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("cfg");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("local.txt"), "apt:curl\n").unwrap();
        let out = src.join("bundle"); // output dir lives INSIDE src
        std::fs::create_dir_all(&out).unwrap();

        let n = copy_dir_recursive(&src, &out.join("groups"), Some(&out), Writes::ToDisk)
            .await
            .unwrap();
        assert_eq!(
            n, 1,
            "only local.txt should be copied, never the nested out dir"
        );
    }

    #[test]
    fn tar_gz_round_trips_through_extract() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("bundle");
        std::fs::create_dir_all(src.join("modules")).unwrap();
        std::fs::write(src.join("modules/dev.txt"), "apt:curl\n").unwrap();
        std::fs::write(src.join("packages.json"), "{}").unwrap();

        let tar = tmp.path().join("bundle.tar.gz");
        let size = crate::utils::archive::create_tar_gz(&src, &tar, "bundle").unwrap();
        assert!(size > 0);
        assert!(tar.exists());

        // Unpack it and confirm the tree survived under the single root folder.
        let dest = tmp.path().join("unpacked");
        crate::utils::archive::extract_archive(&tar, &dest).unwrap();
        let restored = dest.join("bundle/modules/dev.txt");
        assert_eq!(std::fs::read_to_string(restored).unwrap(), "apt:curl\n");
    }
}
