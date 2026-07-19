// src/app/fleet.rs
//
// `fleet` — compare many machines over SSH against their manifests and report drift,
// optionally reconciling each with `linix sync`. It assumes `linix` is installed on the
// remote hosts and SSH is configured non-interactively (keys/agent). Remote invocations
// are read-only unless you pass the flags that opt into changes.
//
// There is no `clone` command. It was removed, implementation and all, because copying the
// installed set without the intent produces a machine nobody can explain; `git clone` of the
// manifests plus `linix sync` is the supported path. Do not reintroduce it here.

use crate::app::App;
use crate::core::{Error, Result};
use serde_json::Value;
use tracing::{info, warn};

/// Reject a host `ssh` would read as an option. A value like `-oProxyCommand=…` runs a command
/// on THIS machine, not the remote one.
fn check_host(host: &str) -> Result<()> {
    if host.starts_with('-') {
        return Err(Error::Config(format!(
            "`{}` is not a host name — a host cannot begin with `-`, because ssh would read it \
             as an option and run a command on this machine instead of the remote one.",
            host
        )));
    }
    Ok(())
}

/// Run a command on a remote host over SSH and return its stdout.
async fn ssh_capture(host: &str, remote_cmd: &str) -> Result<String> {
    check_host(host)?;
    // `-o BatchMode=yes` fails fast instead of hanging on a password prompt. `--` must follow
    // it, not precede it, or ssh stops reading the `-o` pair as options.
    let out = tokio::process::Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("--")
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
    for host in &hosts {
        check_host(host)?;
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
            info!("syncing {} ...", h.host);
            match ssh_capture(&h.host, "linix sync -y").await {
                Ok(_) => {
                    println!("  {} synced.", h.host);
                    ok += 1;
                }
                Err(e) => {
                    warn!("sync failed on {}: {}", h.host, e);
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
    fn a_host_that_looks_like_an_ssh_option_is_refused() {
        let err = check_host("-oProxyCommand=touch /tmp/pwned").unwrap_err();
        assert!(
            err.to_string().contains("cannot begin with `-`"),
            "the error must say why: {}",
            err
        );
        assert!(check_host("-").is_err());
        check_host("build-01.example.com").unwrap();
        check_host("user@10.0.0.4").unwrap();
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
