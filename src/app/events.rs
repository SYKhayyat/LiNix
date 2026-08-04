//! Running the scripts attached to LiNix's own events (XIII.13, U15).
//!
//! **Two locations, and both run (U15, ruled 2026-07-24).** A hook may live in the config
//! repo's `hooks/` directory — the policy every machine should run, committed and shared — or
//! in `preferences.toml`'s `[hooks]` table, which is where a machine keeps the notification
//! that talks to *its* Slack. The two kinds of hook are genuinely different, and forcing them
//! into one file makes one of them wrong.
//!
//! **Additive, never overriding.** Both fire, repo first. A precedence rule would mean that
//! adding a local notification silently disables the shared policy, which is the quiet failure
//! this whole model exists to avoid.
//!
//! **A failing hook warns; it does not fail the sync.** The sync's job is the machine's state,
//! and it succeeded or it did not — a Slack webhook that is down does not change that. The
//! inverse (a hook that can fail a converged sync) would make every integration a new way for
//! `sync` to break.
//!
//! **Approved before it runs, like everything else the repo can execute (II.12).** Same ledger,
//! same rule, no exception for "it's only a notification".

use crate::core::LockFile;
use crate::config::Config;
use crate::core::hook_lock::{event_id, hash_script, refusal, HookLedger};
use crate::core::{Error, Result};
use crate::model::event::{Event, Payload};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// One script attached to one event, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventHook {
    pub event: Event,
    /// The ledger identity — `event:on_drift@hooks/on_drift`. Distinct per location, so
    /// approving the repo's hook never silently approves this machine's.
    pub id: String,
    /// Where it came from, for a message a reader can act on.
    pub origin: String,
    pub script: String,
}

/// Every event hook this machine would run, from both locations.
#[derive(Debug, Clone, Default)]
pub struct EventHooks {
    hooks: Vec<EventHook>,
    locks_dir: PathBuf,
}

impl EventHooks {
    /// Collect the hooks from the config repo and from `preferences.toml`.
    ///
    /// Repo first, so the shared policy runs before the local notification — a hook that tells
    /// you what happened should be told after the hook that does something about it.
    pub fn load(config: &Config) -> EventHooks {
        let root = config.config_root();
        let mut hooks = Vec::new();
        for event in Event::ALL {
            if let Some(hook) = repo_hook(&root, event) {
                hooks.push(hook);
            }
            if let Some(hook) = preference_hook(config, event) {
                hooks.push(hook);
            }
        }
        EventHooks {
            hooks,
            locks_dir: root.join("locks"),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn all(&self) -> &[EventHook] {
        &self.hooks
    }

    /// Fire an event: run every hook attached to it, in load order.
    ///
    /// **An undeclared event costs nothing** — no ledger read, no process, no allocation beyond
    /// the scan that found no hooks. That is what lets these be sprinkled at the interesting
    /// moments without asking whether anyone is listening.
    pub async fn fire(&self, event: Event, data: serde_json::Value) {
        let attached: Vec<&EventHook> = self.hooks.iter().filter(|h| h.event == event).collect();
        if attached.is_empty() {
            return;
        }
        let stdin = Payload::new(event, data).to_stdin();

        // Read the ledger once for the whole event, not once per hook.
        let ledger = match HookLedger::load(&HookLedger::path_in(&self.locks_dir)) {
            Ok(l) => l,
            // A ledger that cannot be read is not permission to run unapproved code.
            Err(e) => {
                warn!(
                    "not running the `{}` hooks: the approval ledger could not be read ({}).",
                    event, e
                );
                return;
            }
        };

        for hook in attached {
            let verdict = ledger.verdict(&hook.id, &hash_script(&hook.script));
            if !verdict.is_approved() {
                // A refusal here warns rather than failing, for the same reason a hook failure
                // does: the machine's state is already what it should be. It is loud, because
                // an unapproved hook is a hook that did not run.
                warn!("{}", refusal(&hook.id, &hook.origin, &verdict));
                continue;
            }
            if let Err(e) = run(hook, &stdin).await {
                warn!(
                    "the `{}` hook at {} failed: {}. The sync itself was unaffected.",
                    event, hook.origin, e
                );
            }
        }
    }

    /// The hooks that would NOT run because their current script is unapproved (II.12).
    ///
    /// Event hooks warn-and-skip rather than blocking a sync — which is right (a down webhook
    /// is not a reason to fail a converged machine) but means they are the one supply-chain
    /// item nothing surfaces until the moment it silently does nothing. `check` asks this so a
    /// hook you wrote and forgot to `linix lock` is a line in "what needs you", not a surprise
    /// the next time the machine drifts.
    ///
    /// A ledger that cannot be read counts everything as unapproved: unreadable is not
    /// permission to run.
    pub fn unapproved(&self) -> Vec<&EventHook> {
        let ledger = HookLedger::load(&HookLedger::path_in(&self.locks_dir)).unwrap_or_default();
        self.hooks
            .iter()
            .filter(|h| !ledger.verdict(&h.id, &hash_script(&h.script)).is_approved())
            .collect()
    }

    /// Approve every event hook at its current hash — what `linix lock` does. The only path
    /// that writes an approval, so approval stays a deliberate act.
    pub fn approve_all(&self) -> Result<usize> {
        if self.hooks.is_empty() {
            return Ok(0);
        }
        let path = HookLedger::path_in(&self.locks_dir);
        let mut ledger = HookLedger::load(&path)?;
        for hook in &self.hooks {
            ledger.approve(&hook.id, &hash_script(&hook.script));
        }
        ledger.save(&path)?;
        Ok(self.hooks.len())
    }
}

/// The config repo's `hooks/<event>` — the policy every machine runs.
///
/// A plain script file rather than a TOML string: it is a script, it wants a shebang and an
/// editor's syntax highlighting, and one file is one hash, which is what the approval ledger
/// wants too.
fn repo_hook(root: &Path, event: Event) -> Option<EventHook> {
    let path = root.join("hooks").join(event.as_str());
    let script = std::fs::read_to_string(&path).ok()?;
    if script.trim().is_empty() {
        return None;
    }
    let origin = format!("hooks/{}", event);
    Some(EventHook {
        event,
        id: event_id(event.as_str(), &origin),
        origin,
        script,
    })
}

/// `preferences.toml`'s `[events]` table — this machine's own.
///
/// A table apart from `[hooks]`, which is the package-lifecycle hooks (`before_install`, …) the
/// embedded Lua/Rhai interpreter runs. Both once read `[hooks]`, so a `preferences.toml`
/// `after_sync` fired twice — once as Lua, once as a script here. LiNix's own events are their
/// own table now, and the two can never name the same key.
fn preference_hook(config: &Config, event: Event) -> Option<EventHook> {
    let script = config.events.get(event.as_str())?;
    if script.trim().is_empty() {
        return None;
    }
    let origin = "preferences.toml".to_string();
    Some(EventHook {
        event,
        id: event_id(event.as_str(), &origin),
        origin,
        script: script.clone(),
    })
}

/// Run one hook with the payload on stdin.
///
/// The script is written to a temporary file and handed to the platform's interpreter — the
/// same one `exec:` uses, from the same place, so a hook is not a second thing that works on
/// Linux and quietly does nothing on Windows.
///
/// 0600 before it is run: the file holds whatever the hook's author wrote, in a world-readable
/// temp directory, and it exists for as long as the hook takes.
async fn run(hook: &EventHook, stdin: &str) -> Result<()> {
    use std::io::Write;
    use tokio::io::AsyncWriteExt;

    debug!("firing {} from {}", hook.event, hook.origin);

    let body = hook.script.clone();
    let script = tokio::task::spawn_blocking(move || -> Result<tempfile::TempPath> {
        let mut tmp = tempfile::Builder::new()
            .suffix(crate::model::script::SCRIPT_SUFFIX)
            .tempfile()
            .map_err(Error::from)?;
        tmp.write_all(body.as_bytes()).map_err(Error::from)?;
        tmp.flush().map_err(Error::from)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(tmp.path())
                .map_err(Error::from)?
                .permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(tmp.path(), perms).map_err(Error::from)?;
        }
        // The handle is closed here, keeping only the path (and the delete-on-drop). Windows
        // refuses to let a second process open a file this one still holds, so an interpreter
        // launched against a live `NamedTempFile` reads nothing and reports nothing useful.
        Ok(tmp.into_temp_path())
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))??;

    let (program, args) = crate::model::script::interpreter_for(&script);
    let mut child = tokio::process::Command::new(program)
        .args(&args)
        .env("LINIX_EVENT", hook.event.as_str())
        .env("LINIX_OS", std::env::consts::OS)
        .env("LINIX_ARCH", std::env::consts::ARCH)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| Error::Other(format!("could not start the hook: {}", e)))?;

    if let Some(mut pipe) = child.stdin.take() {
        // A hook that ignores stdin closes the pipe, and writing to a closed pipe is that
        // hook's choice rather than an error: it is told, and it may not care.
        let _ = pipe.write_all(stdin.as_bytes()).await;
        let _ = pipe.shutdown().await;
    }

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Other(format!("waiting for the hook: {}", e)))?;
    if !status.success() {
        return Err(Error::Other(format!("it exited {:?}", status.code())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config_at(root: &Path) -> Config {
        Config {
            config_root: root.to_path_buf(),
            preferences_file: root.join("preferences.toml"),
            ..Default::default()
        }
    }

    fn write_repo_hook(root: &Path, event: Event, body: &str) {
        let dir = root.join("hooks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(event.as_str()), body).unwrap();
    }

    /// A hook that creates `marker`, in this platform's language.
    ///
    /// The tests below run everywhere rather than under `#[cfg(unix)]`, because "the hook
    /// actually ran" is exactly the claim that is worth *nothing* if it is only ever checked
    /// on the platform where running a script is easy.
    fn touch_script(marker: &Path) -> String {
        let p = marker.display();
        if cfg!(windows) {
            format!("New-Item -ItemType File -Path '{}' | Out-Null\n", p)
        } else {
            format!("#!/bin/sh\ntouch '{}'\n", p)
        }
    }

    /// A hook that saves whatever it is told on stdin into `out`.
    fn capture_stdin_script(out: &Path) -> String {
        let p = out.display();
        if cfg!(windows) {
            format!("[Console]::In.ReadToEnd() | Set-Content -Path '{}'\n", p)
        } else {
            format!("#!/bin/sh\ncat > '{}'\n", p)
        }
    }

    fn exit_with(code: i32) -> String {
        if cfg!(windows) {
            format!("exit {}\n", code)
        } else {
            format!("#!/bin/sh\nexit {}\n", code)
        }
    }

    /// U15's ruling, and the reason it was a ruling: both locations, both run. A precedence
    /// rule would mean adding a local notification silently disables the shared policy.
    #[test]
    fn a_repo_hook_and_a_machine_hook_for_one_event_both_load() {
        let tmp = tempfile::tempdir().unwrap();
        write_repo_hook(tmp.path(), Event::OnDrift, "#!/bin/sh\necho repo\n");
        let mut config = config_at(tmp.path());
        config
            .events
            .insert("on_drift".into(), "#!/bin/sh\necho machine\n".to_string());

        let hooks = EventHooks::load(&config);
        let drift: Vec<&EventHook> = hooks
            .all()
            .iter()
            .filter(|h| h.event == Event::OnDrift)
            .collect();
        assert_eq!(drift.len(), 2, "{:?}", hooks.all());
        // Repo first: the shared policy acts, then this machine is told.
        assert_eq!(drift[0].origin, "hooks/on_drift");
        assert_eq!(drift[1].origin, "preferences.toml");
    }

    /// The two locations are separately approved. Approving the shared policy must not
    /// rubber-stamp whatever this machine has in `preferences.toml`, or the ledger's whole
    /// point — *is this the script I agreed to run* — is gone.
    #[test]
    fn the_two_locations_have_different_ledger_identities() {
        let tmp = tempfile::tempdir().unwrap();
        // The same bytes in both places, which is the case that would expose a shared identity.
        let body = "#!/bin/sh\ntrue\n";
        write_repo_hook(tmp.path(), Event::AfterSync, body);
        let mut config = config_at(tmp.path());
        config.events.insert("after_sync".into(), body.to_string());

        let hooks = EventHooks::load(&config);
        assert_eq!(hooks.all().len(), 2);
        assert_ne!(hooks.all()[0].id, hooks.all()[1].id);
    }

    /// The `[hooks]` table belongs to the package-lifecycle hooks; event hooks read `[events]`.
    /// When both read `[hooks]`, a `preferences.toml` `after_sync` fired twice — once as Lua,
    /// once as a script — so an `after_sync` under `[hooks]` must not be seen here at all.
    #[test]
    fn an_after_sync_under_hooks_is_not_an_event_hook() {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = config_at(tmp.path());
        config.hooks.insert(
            "after_sync".into(),
            [("*".to_string(), "print('lua')".to_string())]
                .into_iter()
                .collect(),
        );
        assert!(
            EventHooks::load(&config).is_empty(),
            "an [events] hook must not be read out of the [hooks] table"
        );
    }

    #[test]
    fn an_undeclared_event_has_no_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks = EventHooks::load(&config_at(tmp.path()));
        assert!(hooks.is_empty());
    }

    /// An empty file is not a hook. Touching `hooks/on_drift` to remind yourself to write one
    /// must not make LiNix execute nothing and call it a success — nor refuse the sync because
    /// nothing is unapproved.
    #[test]
    fn an_empty_hook_file_is_not_a_hook() {
        let tmp = tempfile::tempdir().unwrap();
        write_repo_hook(tmp.path(), Event::OnDrift, "   \n\n");
        assert!(EventHooks::load(&config_at(tmp.path())).is_empty());
    }

    /// Only the three real events are read. A file named after a typo must not be picked up
    /// as some other event's hook.
    #[test]
    fn a_file_named_after_a_typo_is_not_loaded() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("hooks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("after-sync"), "#!/bin/sh\ntrue\n").unwrap();
        std::fs::write(dir.join("on_drifts"), "#!/bin/sh\ntrue\n").unwrap();
        assert!(EventHooks::load(&config_at(tmp.path())).is_empty());
    }

    /// Firing an event nobody listens to must not read the ledger or start a process — that is
    /// what makes it safe to fire events at every interesting moment.
    #[tokio::test]
    async fn firing_an_undeclared_event_costs_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks = EventHooks::load(&config_at(tmp.path()));
        hooks.fire(Event::OnDrift, json!({"removed": ["jq"]})).await;
        // No `locks/` was created, which is the observable proof the ledger was never opened.
        assert!(!tmp.path().join("locks").exists());
    }

    /// `check` surfaces the hooks that will silently not run. Before `lock`, every declared
    /// hook is unapproved; after, none is.
    #[test]
    fn unapproved_lists_what_lock_would_fix() {
        let tmp = tempfile::tempdir().unwrap();
        write_repo_hook(tmp.path(), Event::OnDrift, "#!/bin/sh\ntrue\n");
        let hooks = EventHooks::load(&config_at(tmp.path()));

        assert_eq!(
            hooks.unapproved().len(),
            1,
            "an unlocked hook is unapproved"
        );
        hooks.approve_all().unwrap();
        assert!(
            EventHooks::load(&config_at(tmp.path()))
                .unapproved()
                .is_empty(),
            "lock should approve it"
        );
    }

    /// An edited hook is unapproved again — the same signal `check` should show after an edit.
    #[test]
    fn editing_a_hook_makes_it_unapproved_again() {
        let tmp = tempfile::tempdir().unwrap();
        write_repo_hook(tmp.path(), Event::OnDrift, "#!/bin/sh\ntrue\n");
        EventHooks::load(&config_at(tmp.path()))
            .approve_all()
            .unwrap();
        write_repo_hook(tmp.path(), Event::OnDrift, "#!/bin/sh\nfalse\n");
        assert_eq!(
            EventHooks::load(&config_at(tmp.path())).unapproved().len(),
            1
        );
    }

    /// II.12 admits no exception for "it's only a notification": an unapproved hook does not
    /// run. It warns rather than failing, because the machine's state is already correct.
    #[tokio::test]
    async fn an_unapproved_hook_does_not_run() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("ran");
        write_repo_hook(tmp.path(), Event::OnDrift, &touch_script(&marker));
        let hooks = EventHooks::load(&config_at(tmp.path()));
        hooks.fire(Event::OnDrift, json!({})).await;
        assert!(!marker.exists(), "an unapproved hook ran");
    }

    /// The payload reaches the hook on stdin, and approval is what let it run.
    #[tokio::test]
    async fn an_approved_hook_runs_and_is_told_what_happened() {
        let tmp = tempfile::tempdir().unwrap();
        let out = tmp.path().join("payload.json");
        write_repo_hook(tmp.path(), Event::OnDrift, &capture_stdin_script(&out));
        let hooks = EventHooks::load(&config_at(tmp.path()));
        assert_eq!(hooks.approve_all().unwrap(), 1);

        hooks.fire(Event::OnDrift, json!({"removed": ["jq"]})).await;

        let body = std::fs::read_to_string(&out).expect("the hook ran and read stdin");
        let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
        assert_eq!(v["event"], "on_drift");
        assert_eq!(v["data"]["removed"][0], "jq");
    }

    /// A hook that exits non-zero warns and does not fail anything — 7j's exit condition. The
    /// sync's job is the machine's state, and a webhook that is down does not change it.
    #[tokio::test]
    async fn a_failing_hook_does_not_fail_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        write_repo_hook(tmp.path(), Event::AfterSync, &exit_with(9));
        let hooks = EventHooks::load(&config_at(tmp.path()));
        hooks.approve_all().unwrap();
        // `fire` returns nothing to propagate: a hook has no way to fail its caller by
        // construction, not by a caller remembering to ignore it.
        hooks.fire(Event::AfterSync, json!({})).await;
    }

    /// An edited hook is unapproved again — the ledger's actual job.
    #[tokio::test]
    async fn editing_an_approved_hook_stops_it_running() {
        let tmp = tempfile::tempdir().unwrap();
        let marker = tmp.path().join("ran");
        write_repo_hook(tmp.path(), Event::OnDrift, &exit_with(0));
        EventHooks::load(&config_at(tmp.path()))
            .approve_all()
            .unwrap();

        write_repo_hook(tmp.path(), Event::OnDrift, &touch_script(&marker));
        EventHooks::load(&config_at(tmp.path()))
            .fire(Event::OnDrift, json!({}))
            .await;
        assert!(!marker.exists(), "an edited hook ran on the old approval");
    }
}
