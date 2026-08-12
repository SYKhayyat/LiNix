//! **Moving a machine somewhere else: `bundle` and `restore`.**
//!
//! One writes everything needed to rebuild this machine into a directory; the other reads it
//! back. They are two halves of one round trip, and a change to either that is not matched in
//! the other breaks it — which is the argument for them sharing a file and not a coincidence of
//! size.

use crate::verbs::plan::compute_full_changes;
use crate::verbs::prelude::*;

pub async fn handle_bundle(app: &App, out: &str, artifacts: bool, archive: bool) -> Result<()> {
    let out_path = std::path::PathBuf::from(out);

    // Freeze a plan so the target can review/apply it offline. Computed up front so it can be
    // written into the bundle (and captured inside the archive) by create_bundle.
    let plan_json = match compute_full_changes(app, None).await {
        Ok(full) => {
            let mut plan = crate::app::sync::SavedPlan::from_changes(
                &full.changes,
                &full.resources,
                Some(chrono::Utc::now().timestamp()),
            );
            plan.vars = full.state.vars;
            Some(serde_json::to_string_pretty(&plan)?)
        }
        Err(_) => None,
    };

    let report = crate::app::bundle::create_bundle(
        &app.config,
        &app.state,
        &app.vcs(),
        &out_path,
        artifacts,
        archive,
        plan_json.as_deref(),
    )
    .await?;

    // The tense comes from the writer, not from asking the flag a second time (Q15/V.105).
    // `--dry-run bundle` wrote all nine files and said "Bundle written to X" — a preview that
    // manufactured the artifact it was asked to describe, and reported it in the past tense.
    let lead = if report.previewed {
        format!("{} would write a bundle to", crate::core::dry_run::MARKER)
    } else {
        "Bundle written to".to_string()
    };
    println!(
        "{} {} — {} config file(s), {} package(s).",
        lead,
        report.out.display(),
        report.files_copied,
        report.package_count
    );
    // Honest per-part reporting: say plainly what did and did NOT make it into the bundle.
    println!(
        "  manifest history (git bundle): {}",
        if report.git_history_included {
            "included (config.bundle) — `git clone` it to roll back to any past commit"
        } else {
            "NOT included — the config is not a git repo (or has no commits); run `shall git init`"
        }
    );
    println!(
        "  ownership registry (registry.json): {}",
        if report.registry_included {
            "included"
        } else {
            "NOT included — none found"
        }
    );
    if artifacts {
        println!(
            "Artifacts: {} {}, {} skipped.",
            report.artifacts_fetched.len(),
            if report.previewed {
                "would be fetched"
            } else {
                "fetched"
            },
            report.artifacts_skipped.len()
        );
        // Honest reporting: never let a skipped backend read as "bundled everything".
        for s in &report.artifacts_skipped {
            println!("  skipped {}", s);
        }
    }
    if let Some((path, size)) = &report.archive {
        if report.previewed {
            println!("Archive: {} would be written.", path.display());
        } else {
            println!(
                "Archive: {} ({:.1} KiB) — copy this one file to an air-gapped host.",
                path.display(),
                *size as f64 / 1024.0
            );
        }
    }
    if report.previewed {
        println!("Nothing was written. Run without `--dry-run` to produce the bundle.");
    } else {
        println!(
            "See {}/RESTORE.md for offline restore steps.",
            report.out.display()
        );
    }
    Ok(())
}

pub async fn handle_restore(
    config: &Config,
    state: &tokio::sync::Mutex<crate::core::StateRegistry>,
    dir: &str,
    force: bool,
) -> Result<()> {
    let bundle_dir = std::path::PathBuf::from(dir);
    let config_root = config.config_root();
    let registry_path = { state.lock().await.path.clone() };

    let report =
        crate::app::bundle::restore_bundle(&bundle_dir, &config_root, &registry_path, force)
            .await?;

    println!(
        "Restored {} config file(s) into {}.",
        report.config_files,
        config_root.display()
    );
    println!(
        "  ownership registry: {}",
        if report.registry_restored {
            "restored"
        } else {
            "not in the bundle — a first `sync` will rebuild it"
        }
    );
    if report.git_history_present {
        println!(
            "  manifest history: `config.bundle` is in {} — `git clone` it there to keep the \
             history, or `shall sync --locked` to reproduce the current state.",
            bundle_dir.display()
        );
    }
    println!("Run `shall sync --locked` to reproduce the exact package set.");
    Ok(())
}
