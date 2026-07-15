// src/app/fleet.rs
//
// Cross-machine operations that only make sense because LiNix already models a machine's
// whole package universe declaratively:
//
//   * `clone` — reproduce another machine's installed packages onto this one over SSH,
//               translating each package to a backend that exists locally (so an
//               apt: package from a Linux box installs via brew:/cargo: on a Mac).
//   * `fleet` — compare many machines over SSH against their manifests and report drift,
//               optionally reconciling each with `linix sync`.
//
// Both assume `linix` is installed on the remote hosts and SSH is configured
// non-interactively (keys/agent). Remote invocations are read-only unless you pass the
// flags that opt into changes.

use crate::app::App;
use crate::config::parser::add_package_to_local;
use crate::core::{Error, Package, Result};
use serde_json::Value;
use tracing::{info, warn};

/// Run a command on a remote host over SSH and return its stdout.
async fn ssh_capture(host: &str, remote_cmd: &str) -> Result<String> {
    // `-o BatchMode=yes` fails fast instead of hanging on a password prompt.
    let out = tokio::process::Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg(host)
        .arg(remote_cmd)
        .output()
        .await
        .map_err(|e| Error::Other(format!("failed to launch ssh for {}: {}", host, e)))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "ssh {} `{}` failed: {}",
            host,
            remote_cmd,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Replicate `host`'s installed packages onto the local machine, translating backends.
pub async fn clone(app: &App, host: &str, dry_run: bool) -> Result<()> {
    info!("Clone: reading installed packages from {} ...", host);
    let json = ssh_capture(host, "linix list --json").await?;
    let remote: Vec<Package> = serde_json::from_str(&json)
        .map_err(|e| Error::Json(format!("parsing `linix list --json` from {}: {}", host, e)))?;

    if remote.is_empty() {
        println!("{} reports no packages to clone.", host);
        return Ok(());
    }

    // Translate each remote package to a spec this machine can install.
    let mut plan: Vec<(String, String)> = Vec::new(); // (human description, spec string)
    for pkg in &remote {
        let local_has = app
            .registry
            .get(&pkg.backend)
            .map(|b| b.is_available())
            .unwrap_or(false);
        if local_has {
            plan.push((
                format!("{0}:{1}", pkg.backend, pkg.name),
                format!("{}:{}", pkg.backend, pkg.name),
            ));
        } else {
            // Backend absent here: fall back to a bare name so the resolver auto-detects a
            // backend from this machine's priority list.
            plan.push((
                format!("{0}:{1} -> auto", pkg.backend, pkg.name),
                pkg.name.clone(),
            ));
        }
    }

    println!(
        "Cloning {} package(s) from {} onto this machine:",
        plan.len(),
        host
    );
    for (desc, _) in &plan {
        println!("    {}", desc);
    }

    if dry_run || app.config.dry_run {
        println!("\n[dry-run] Nothing installed. Re-run without --dry-run to apply.");
        return Ok(());
    }

    let (mut ok, mut failed) = (0u32, 0u32);
    for (_, spec_str) in &plan {
        match install_one(app, spec_str).await {
            Ok(true) => ok += 1,
            Ok(false) => {} // already present / no installable backend
            Err(e) => {
                failed += 1;
                warn!("Clone: failed to install '{}': {}", spec_str, e);
            }
        }
    }
    app.state.lock().await.save()?;
    println!("\nClone complete: {} installed, {} failed.", ok, failed);
    Ok(())
}

/// Resolve a spec string and install it locally, recording provenance as "clone".
/// Returns Ok(true) if something was installed, Ok(false) if there was nothing to do.
async fn install_one(app: &App, spec_str: &str) -> Result<bool> {
    let mut installed_any = false;
    for spec in app.resolve_spec(spec_str).await? {
        let Some(b) = app.registry.get(&spec.backend) else {
            continue;
        };
        let Some(inst) = b.as_installable() else {
            continue;
        };
        inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
            .await?;
        app.state.lock().await.add(
            &spec.backend,
            &spec.name,
            None,
            spec.options.clone(),
            Some("clone".into()),
            false,
        );
        let _ = add_package_to_local(
            &app.config.groups_dir,
            &format!("{}:{}", spec.backend, spec.name),
        )
        .await;
        installed_any = true;
    }
    Ok(installed_any)
}

/// Per-host drift summary from a remote `linix status --json`.
#[derive(Debug)]
pub struct HostDrift {
    pub host: String,
    pub to_install: usize,
    pub to_remove: usize,
    pub unmanaged: usize,
    pub error: Option<String>,
}

impl HostDrift {
    pub fn in_sync(&self) -> bool {
        self.error.is_none() && self.to_install == 0 && self.to_remove == 0
    }
}

/// Parse the counts out of a remote `linix status --json` document. Pure — unit tested.
fn parse_status(json: &str) -> Result<(usize, usize, usize)> {
    let v: Value = serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))?;
    let count = |key: &str| {
        v.get(key)
            .and_then(|x| x.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    };
    Ok((count("to_install"), count("to_remove"), count("unmanaged")))
}

/// Query each host's drift versus its manifests and report; optionally reconcile.
/// `do_sync` reconciles only the DRIFTED machines; `do_apply` runs `linix sync -y` on EVERY
/// reachable host regardless of drift (a deliberate fleet-wide push).
pub async fn fleet(app: &App, hosts: &[String], do_sync: bool, do_apply: bool) -> Result<()> {
    let hosts: Vec<String> = if hosts.is_empty() {
        app.config.fleet_hosts.clone()
    } else {
        hosts.to_vec()
    };
    if hosts.is_empty() {
        return Err(Error::Config(
            "no hosts given and `fleet_hosts` is empty in config.toml".into(),
        ));
    }

    let mut report = Vec::new();
    for host in &hosts {
        match ssh_capture(host, "linix status --json").await {
            Ok(json) => match parse_status(&json) {
                Ok((ti, tr, un)) => report.push(HostDrift {
                    host: host.clone(),
                    to_install: ti,
                    to_remove: tr,
                    unmanaged: un,
                    error: None,
                }),
                Err(e) => report.push(HostDrift {
                    host: host.clone(),
                    to_install: 0,
                    to_remove: 0,
                    unmanaged: 0,
                    error: Some(e.to_string()),
                }),
            },
            Err(e) => report.push(HostDrift {
                host: host.clone(),
                to_install: 0,
                to_remove: 0,
                unmanaged: 0,
                error: Some(e.to_string()),
            }),
        }
    }

    let in_sync = report.iter().filter(|h| h.in_sync()).count();
    println!(
        "{} of {} machine(s) match their manifests.\n",
        in_sync,
        report.len()
    );
    println!(
        "{:<28} {:>9} {:>8} {:>10}  STATUS",
        "HOST", "INSTALL", "REMOVE", "UNMANAGED"
    );
    for h in &report {
        if let Some(err) = &h.error {
            println!(
                "{:<28} {:>9} {:>8} {:>10}  ERROR: {}",
                h.host, "-", "-", "-", err
            );
        } else {
            let status = if h.in_sync() { "in sync" } else { "DRIFT" };
            println!(
                "{:<28} {:>9} {:>8} {:>10}  {}",
                h.host, h.to_install, h.to_remove, h.unmanaged, status
            );
        }
    }

    // Reconciliation. `--apply` pushes to every reachable host; `--sync` touches only drift.
    if do_apply || do_sync {
        let targets: Vec<&HostDrift> = if do_apply {
            println!("\nApplying `linix sync -y` to all reachable machines ...");
            report.iter().filter(|h| h.error.is_none()).collect()
        } else {
            println!("\nReconciling drifted machines with `linix sync -y` ...");
            report
                .iter()
                .filter(|h| !h.in_sync() && h.error.is_none())
                .collect()
        };
        if targets.is_empty() {
            println!("  (nothing to do)");
        }
        let mut ok = 0usize;
        let mut failed = 0usize;
        for h in targets {
            info!("Fleet: syncing {} ...", h.host);
            match ssh_capture(&h.host, "linix sync -y").await {
                Ok(_) => {
                    println!("  {} synced.", h.host);
                    ok += 1;
                }
                Err(e) => {
                    warn!("Fleet: sync failed on {}: {}", h.host, e);
                    failed += 1;
                }
            }
        }
        println!("\nApplied to {} host(s), {} failed.", ok, failed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_counts_arrays() {
        let json = r#"{
            "to_install": [{"backend":"apt","name":"a"},{"backend":"apt","name":"b"}],
            "to_remove": [{"backend":"apt","name":"c"}],
            "unmanaged": []
        }"#;
        assert_eq!(parse_status(json).unwrap(), (2, 1, 0));
    }

    #[test]
    fn parse_status_tolerates_missing_keys() {
        assert_eq!(parse_status("{}").unwrap(), (0, 0, 0));
    }

    #[test]
    fn in_sync_logic() {
        let clean = HostDrift {
            host: "h".into(),
            to_install: 0,
            to_remove: 0,
            unmanaged: 3,
            error: None,
        };
        assert!(clean.in_sync(), "unmanaged packages alone are not drift");
        let drift = HostDrift {
            host: "h".into(),
            to_install: 1,
            to_remove: 0,
            unmanaged: 0,
            error: None,
        };
        assert!(!drift.in_sync());
        let errored = HostDrift {
            host: "h".into(),
            to_install: 0,
            to_remove: 0,
            unmanaged: 0,
            error: Some("x".into()),
        };
        assert!(!errored.in_sync());
    }
}
