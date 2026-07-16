// src/app/bundle.rs
//
// Offline / air-gapped bundling. `linix bundle` packs a portable copy of the declarative
// configuration (manifests, modules, lockfile, keep-list, config) plus a resolved package
// list, so an environment can be reproduced on a disconnected machine. With `--artifacts`
// it additionally tries to pre-download package files for the backends that support an
// offline fetch — honestly reporting which backends it cannot bundle.

use crate::app::App;
use crate::core::{Error, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct BundleReport {
    pub out: PathBuf,
    pub files_copied: usize,
    pub package_count: usize,
    pub artifacts_fetched: Vec<String>,
    pub artifacts_skipped: Vec<String>,
    /// Set when `--archive` was requested: the single `.tar.gz` produced, and its size.
    pub archive: Option<(PathBuf, u64)>,
}

/// Pure: the command to *download* (not install) a package into `dest`, for backends that
/// support an offline fetch. `None` means the backend has no offline-download mode, so it is
/// bundled by declaration only. Unit tested.
pub fn offline_fetch_command(
    backend: &str,
    name: &str,
    dest: &str,
) -> Option<(String, Vec<String>)> {
    let v = |p: &str, a: &[&str]| {
        Some((
            p.to_string(),
            a.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        ))
    };
    match backend {
        "apt" => v("apt-get", &["download", name]), // downloads into the working dir
        "dnf" => v("dnf", &["download", "--destdir", dest, name]),
        "pip" | "pipx" => v("pip", &["download", name, "-d", dest]),
        "npm" | "pnpm" | "yarn" | "bun" => v("npm", &["pack", name]), // into working dir
        "brew" => v("brew", &["fetch", name]),
        "pacman" => v("pacman", &["-Sw", "--noconfirm", name]),
        "apk" => v("apk", &["fetch", name]),
        _ => None,
    }
}

/// Recursively copy a directory tree, returning the number of files copied. Missing sources
/// are a no-op (return 0), not an error.
async fn copy_dir_recursive(src: &Path, dst: &Path, skip: Option<&Path>) -> Result<usize> {
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
        tokio::fs::create_dir_all(&d).await.map_err(Error::from)?;
        let mut rd = tokio::fs::read_dir(&s).await.map_err(Error::from)?;
        while let Some(entry) = rd.next_entry().await.map_err(Error::from)? {
            let ft = entry.file_type().await.map_err(Error::from)?;
            let from = entry.path();
            // Never ship the machine-local lock-signing secret into a portable bundle — the
            // target verifies-or-proceeds without it (see core::locksig).
            if entry.file_name() == ".linix-lock.key" {
                continue;
            }
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
                tokio::fs::copy(&from, &to).await.map_err(Error::from)?;
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Run a download command inside `dir` (for tools that write to the working directory).
async fn run_in_dir(prog: &str, args: &[&str], dir: &Path) -> bool {
    use tokio::process::Command;
    Command::new(prog)
        .args(args)
        .current_dir(dir)
        .output()
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
    tokio::fs::create_dir_all(out).await.map_err(Error::from)?;
    let mut report = BundleReport {
        out: out.to_path_buf(),
        ..Default::default()
    };

    // 1. Declarative config: groups/ (local.txt, host-*.txt, locks.json, keep.txt) + modules/.
    report.files_copied +=
        copy_dir_recursive(&app.config.groups_dir, &out.join("groups"), Some(out)).await?;
    report.files_copied +=
        copy_dir_recursive(&app.config.modules_dir, &out.join("modules"), Some(out)).await?;
    if let Some(parent) = app.config.groups_dir.parent() {
        let cfg = parent.join("config.toml");
        if cfg.exists() {
            let _ = tokio::fs::copy(&cfg, out.join("config.toml")).await;
            report.files_copied += 1;
        }
    }

    // 2. Resolved managed package list.
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
    tokio::fs::write(
        out.join("packages.json"),
        serde_json::to_string_pretty(&json!({ "packages": pkgs }))?,
    )
    .await
    .map_err(Error::from)?;

    // 3. Optional artifact pre-download for air-gapped installs.
    if include_artifacts {
        let dest_root = out.join("artifacts");
        tokio::fs::create_dir_all(&dest_root)
            .await
            .map_err(Error::from)?;
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

    // 4. Human restore instructions.
    let restore = format!(
        "# LiNix offline bundle\n\n\
         Packages: {}\nConfig files: {}\nArtifacts pre-fetched: {}\n\n\
         ## Restore on the target machine\n\n\
         1. Copy this directory to the machine.\n\
         2. Point LiNix at it (e.g. set the config dir to this bundle) or copy `groups/` and\n\
            `modules/` into your LiNix config directory.\n\
         3. Reproduce the exact versions:  `linix sync --locked`\n\
            (locks.json inside `groups/` pins every version).\n\n\
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
    tokio::fs::write(out.join("RESTORE.md"), restore)
        .await
        .map_err(Error::from)?;

    // 4b. Frozen plan, so the target can review/apply it offline (before archiving, so it is
    // captured inside the tarball).
    if let Some(pj) = plan_json {
        tokio::fs::write(out.join("plan.json"), pj)
            .await
            .map_err(Error::from)?;
    }

    // 5. Optional single-file archive for easy transfer to an air-gapped host. The tar stores
    // everything under one top folder named after the bundle dir, so it unpacks cleanly.
    if archive {
        let root_name = out
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "linix-bundle".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_fetch_covers_common_backends() {
        assert_eq!(
            offline_fetch_command("apt", "curl", "/d"),
            Some(("apt-get".into(), vec!["download".into(), "curl".into()]))
        );
        assert_eq!(
            offline_fetch_command("pip", "requests", "/d"),
            Some((
                "pip".into(),
                vec![
                    "download".into(),
                    "requests".into(),
                    "-d".into(),
                    "/d".into()
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

        let n = copy_dir_recursive(&src, &out.join("groups"), Some(&out))
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
        std::fs::create_dir_all(src.join("groups")).unwrap();
        std::fs::write(src.join("groups/local.txt"), "apt:curl\n").unwrap();
        std::fs::write(src.join("packages.json"), "{}").unwrap();

        let tar = tmp.path().join("bundle.tar.gz");
        let size = crate::utils::archive::create_tar_gz(&src, &tar, "bundle").unwrap();
        assert!(size > 0);
        assert!(tar.exists());

        // Unpack it and confirm the tree survived under the single root folder.
        let dest = tmp.path().join("unpacked");
        crate::utils::archive::extract_archive(&tar, &dest).unwrap();
        let restored = dest.join("bundle/groups/local.txt");
        assert_eq!(std::fs::read_to_string(restored).unwrap(), "apt:curl\n");
    }
}
