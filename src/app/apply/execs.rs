use crate::core::LockFile;
use crate::core::{Error, Result};
use tracing::{debug, info, warn};

/// Execs holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::execs()` and can be built without one.
pub struct Execs<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) journal: &'a std::sync::Arc<tokio::sync::Mutex<crate::core::Journal>>,
}

impl Execs<'_> {
    /// Resolve one `exec:` line to the script Shall would run, its content hash, and what the
    /// two ledgers say about it. Shared by the preview and the run so a plan cannot describe a
    /// decision the sync then makes differently.
    ///
    /// The path is taken relative to the config repo when it is not absolute — the script
    /// travels with the configuration that declares it, the way a `link:` source does.
    fn exec_plan(
        &self,
        script: &str,
        opts: &crate::config::grammar::Options,
        hooks: &crate::core::hook_lock::HookLedger,
        runs: &crate::core::ExecLedger,
    ) -> Result<(std::path::PathBuf, String, crate::model::exec::Decision)> {
        use crate::core::hook_lock::{exec_id, hash_script};

        let declared = std::path::Path::new(script);
        let path = if declared.is_absolute() {
            declared.to_path_buf()
        } else {
            self.config.config_root().join(declared)
        };
        let body = std::fs::read_to_string(&path).map_err(|e| {
            Error::Validation(format!(
                "`exec:{}` — cannot read the script at {} ({}). An `exec:` names a file the \
                 config carries; its contents are what Shall hashes and runs.",
                script,
                path.display(),
                e
            ))
        })?;
        let hash = hash_script(&body);
        let decision = crate::model::exec::Decision::of(
            &hooks.verdict(&exec_id(script), &hash),
            runs.count(&hash),
            crate::core::Ceiling::read(opts.one("runs")),
        );
        Ok((path, hash, decision))
    }
    /// Print what each declared `exec:` will do, before anything happens (XIII.3's exit
    /// condition): the content hash, how many times that content has run here, and the
    /// decision that follows. Uses the same `exec_plan` the run uses, so the preview cannot
    /// describe one thing and the sync do another.
    ///
    /// A script that cannot be read is reported here rather than propagated: this is the
    /// preview, and the run raises the same problem as a real error a moment later.
    pub fn print_plan(&self, state: &crate::model::DesiredState, verb: crate::model::exec::Verb) {
        use crate::core::hook_lock::HookLedger;

        if !state.has_execs() {
            return;
        }
        let locks = self.config.layout().locks_dir();
        let (Ok(hooks), Ok(runs)) = (
            HookLedger::load(&HookLedger::path_in(&locks)),
            crate::core::ExecLedger::load(&crate::core::ExecLedger::path_in(&locks)),
        ) else {
            return;
        };
        println!("Scripts:");
        // **Every declared line, including the ones this verb will not run.** `execs_for` is
        // the running list and it is the wrong list to preview from: an `@on=upgrade` step
        // filtered out here is declared code that no preview anywhere shows, which is the
        // reporting hole `F12` was — a category dropped from the summary because the summary
        // was built from the actor's list rather than the reader's. The line says which verb
        // claims it instead, so nothing is hidden and nothing is misattributed.
        for (script, opts, origin) in state.execs() {
            let mine = verb.claims(opts.one("on"));
            match self.exec_plan(script, opts, &hooks, &runs) {
                Ok((_, hash, decision)) => {
                    println!("  exec:{}  ({})", script, origin);
                    match mine {
                        true => println!("    {}", decision.describe(&hash)),
                        false => println!(
                            "    {} — not this command; `@on={}` runs it",
                            decision.describe(&hash),
                            opts.one("on").unwrap_or("sync")
                        ),
                    }
                }
                Err(e) => println!("  exec:{}  ({}) — {}", script, origin, e),
            }
        }
    }
    /// Run the declared `exec:` scripts (XIII.3) — II.7's verb phase, after the packages and
    /// dependents a script is likely to depend on.
    ///
    /// Three things this does that a naive "run the command" would not: it refuses a script
    /// II.12 has not approved (a repo that can run code is the hook question with a different
    /// file name, and `-y` cannot approve); it runs a given *content* only as many times as its
    /// `@runs=` ceiling allows, so a settled sync executes nothing; and it records the run only
    /// when the script actually succeeded — a failed script has not happened, so the next sync
    /// must try it again.
    pub async fn apply(
        &self,
        state: &crate::model::DesiredState,
        verb: crate::model::exec::Verb,
    ) -> Result<()> {
        use crate::core::hook_lock::HookLedger;

        // No early return when nothing is declared: deleting the LAST `exec:` line is a real
        // change, and a teardown that only runs when something is still declared can never
        // undo the last one (S20 taught this for extras; it is the same shape here).
        let locks = self.config.layout().locks_dir();
        let hooks = HookLedger::load(&HookLedger::path_in(&locks))?;
        let runs_path = crate::core::ExecLedger::path_in(&locks);
        let mut runs = crate::core::ExecLedger::load(&runs_path)?;

        for (script, opts, origin) in state.execs_for(verb) {
            let (path, hash, decision) = self.exec_plan(script, opts, &hooks, &runs)?;
            if let crate::model::exec::Decision::NeedsApproval(verdict) = &decision {
                // A refusal, not a warning: this is code from the configuration, and II.12's
                // whole point is that nothing runs it until a human has looked.
                return Err(Error::Validation(format!(
                    "{}: {}",
                    origin,
                    crate::core::hook_lock::refusal(
                        &crate::core::hook_lock::exec_id(script),
                        "exec script",
                        verdict
                    )
                )));
            }
            if !decision.will_run() {
                debug!("exec:{} — {}", script, decision.describe(&hash));
                continue;
            }
            if self.config.dry_run {
                crate::would!("would run exec:{} ({})", script, origin);
                continue;
            }
            // `@runs=always` is named in the line it produces: a script that runs every sync
            // makes the sync non-idempotent, and the next person debugging a slow sync needs a
            // thread to pull (U13). A counted or once script does not need the note.
            if opts.one("runs") == Some("always") {
                info!(
                    "running exec:{} (runs=always — every sync) ({})",
                    script, origin
                );
            } else {
                info!("running exec:{} ({})", script, origin);
            }
            // Written and flushed BEFORE the interpreter starts, which is the whole point of a
            // write-ahead record: an entry made afterwards describes a mutation that already
            // happened, and the case it exists for is the one where "afterwards" never comes.
            // `exec:` is the only thing a sync runs that recovery cannot finish — a package can
            // be installed again, a `service:` re-converged from its line, but a script that
            // got half way has no recorded progress and no declared end state. So this record
            // buys the one thing left: the next run says what was interrupted instead of
            // silently running it again from the top.
            let started = self
                .record_start(crate::core::journal::JournalAction::Exec {
                    script: script.to_string(),
                    hash: hash.clone(),
                })
                .await;
            let outcome = self.run_exec_script(&path).await;
            self.resolve(started, &outcome).await;
            outcome?;
            // Recorded only on success. A script that failed did not happen, and the next sync
            // must be free to try it again.
            runs.record_run(
                &hash,
                chrono::Utc::now().to_rfc3339(),
                script,
                opts.one("undo"),
            );
            runs.save(&runs_path)?;
        }

        self.undo_departed_execs(state, &mut runs, &runs_path)
            .await?;
        Ok(())
    }
    /// Run the `@undo=` of every `exec:` whose line has gone away, then forget it (U3).
    ///
    /// **The undo is read from the ledger, not from the files**, because by the time it is
    /// needed the declaration has been deleted — that is what removal means. Reading the
    /// current config would find nothing and do nothing, which is the `link:` source-deletion
    /// mistake wearing a different hat.
    ///
    /// A script that declared no `@undo=` is simply forgotten: Shall cannot invent an inverse,
    /// and pretending to would be worse than saying nothing. `plan` says so in those words.
    async fn undo_departed_execs(
        &self,
        state: &crate::model::DesiredState,
        runs: &mut crate::core::ExecLedger,
        runs_path: &std::path::Path,
    ) -> Result<()> {
        let declared = self.declared_exec_paths()?;
        // An unreadable configuration yields an empty set, which must never be read as "every
        // script departed" — that would run every undo on the machine because of a stray brace.
        if declared.is_empty() && state.has_execs() {
            return Ok(());
        }
        let departed = runs.departed(&declared);
        if departed.is_empty() {
            return Ok(());
        }
        for (hash, record) in departed {
            let name = record.script.as_deref().unwrap_or(&hash);
            let Some(undo) = record.undo.as_deref().filter(|u| !u.trim().is_empty()) else {
                debug!("`exec:{}` is no longer declared; it had no `undo`.", name);
                if !self.config.dry_run {
                    runs.forget(&hash);
                }
                continue;
            };
            if self.config.dry_run {
                crate::would!("would undo `exec:{}` with: {}", name, undo);
                continue;
            }
            info!("`exec:{}` is no longer declared — running its undo.", name);
            // An `@undo=` is an arbitrary shell command a human wrote, and it is the second of
            // the two mutations a sync makes that nothing can recompute. Same rule as the
            // script above: recorded before it starts, resolved after.
            let started = self
                .record_start(crate::core::journal::JournalAction::ExecUndo {
                    script: name.to_string(),
                    command: undo.to_string(),
                })
                .await;
            let outcome = self.run_shell_command(undo).await;
            self.resolve(started, &outcome).await;
            match outcome {
                Ok(()) => runs.forget(&hash),
                // Kept in the ledger on failure, so the next sync tries again rather than
                // forgetting an undo that never happened.
                Err(e) => warn!(
                    "could not undo `exec:{}` ({}); it stays recorded and the next sync will \
                     try again.",
                    name, e
                ),
            }
        }
        runs.save(runs_path)
    }
    /// Every `exec:` script path the configuration contains — read from the FILES, ignoring
    /// `when` and ignoring which profiles are active.
    ///
    /// **This is deliberately not `state.execs()`.** That answers *does this machine want it
    /// right now*, which is a different question with a dangerous difference: a `when` that
    /// went false would read as a deleted line and run its `@undo=` — the enrol script
    /// un-enrolling itself on the sync after it succeeded, which is the flapping failure
    /// XIII.3 spends a section warning about. Deactivating a profile is likewise not a
    /// deletion. Only removing the line from the file is, and only the file can say so.
    ///
    /// A file that cannot be parsed contributes nothing rather than being treated as empty:
    /// concluding "every exec: has departed" from a syntax error would run every undo on the
    /// machine because of a stray brace.
    fn declared_exec_paths(&self) -> Result<std::collections::BTreeSet<String>> {
        use crate::config::grammar::Statement;

        let mut out = std::collections::BTreeSet::new();
        let modules = self.config.layout().modules_dir();
        let Ok(entries) = std::fs::read_dir(&modules) else {
            return Ok(out);
        };
        let known = |name: &str| self.registry.get(name).is_some();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(doc) = crate::config::grammar::parse_document(&path, &body, &known) else {
                // Unparseable: the resolver reports it properly elsewhere. Here it must not be
                // read as "this file declares nothing".
                warn!(
                    "{}: could not be parsed while looking for `exec:` lines; leaving its \
                     scripts recorded.",
                    path.display()
                );
                return Ok(std::collections::BTreeSet::new());
            };
            for (stmt, _, _) in doc.every_statement() {
                if let Statement::Exec(script, _) = stmt {
                    out.insert(script.clone());
                }
            }
        }
        Ok(out)
    }
    /// Open a write-ahead entry for a mutation that is about to start.
    ///
    /// A journal that cannot be written must not stop the script running — the log exists to
    /// describe work, not to gate it, and refusing to converge a machine because a lock file
    /// is read-only would be the tail wagging the dog. It is said out loud, because a run
    /// nothing recorded is a run `heal` cannot account for and the user is owed that fact.
    async fn record_start(&self, action: crate::core::journal::JournalAction) -> Option<String> {
        let described = action.key();
        match self.journal.lock().await.record_start(action) {
            Ok(id) => Some(id),
            Err(e) => {
                warn!(
                    "could not record `{}` in the write-ahead log ({}); it will still run, but \
                     an interruption will not be reported by the next sync.",
                    described, e
                );
                None
            }
        }
    }

    /// Close the entry `record_start` opened. Paired with it here rather than at each call
    /// site, so an outcome cannot be recorded against the wrong id or forgotten entirely —
    /// forgotten is the one that matters, because an entry left open keeps `needs_recovery`
    /// true and re-reports the same script in front of every sync for ever.
    async fn resolve(&self, id: Option<String>, outcome: &Result<()>) {
        let Some(id) = id else {
            return;
        };
        let mut journal = self.journal.lock().await;
        let _ = match outcome {
            Ok(()) => journal.record_success(&id),
            Err(e) => journal.record_failure(&id, &e.to_string()),
        };
    }

    /// Run a command line through the platform's shell. Used only for `@undo=`, which is
    /// written as a command rather than a script path.
    async fn run_shell_command(&self, command: &str) -> Result<()> {
        #[cfg(windows)]
        let (program, args) = (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                command.to_string(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = ("sh", vec!["-c".to_string(), command.to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.executor.run(program, &refs, false).await.map(|_| ())
    }
    /// Execute one script through the interpreter its first line names, or this platform's
    /// shell if it names none — `sh` on Unix and PowerShell on Windows. A repo that must ship
    /// two spellings of every script is a repo that cannot be shared, which is the reason the
    /// file travels with the config at all.
    async fn run_exec_script(&self, path: &std::path::Path) -> Result<()> {
        // A script that is not UTF-8 has no first line this can read, and falls through to the
        // platform default — which is what every `exec:` script got before shebangs were read.
        let contents = tokio::fs::read_to_string(path).await.unwrap_or_default();
        let launch = crate::model::script::launch_for(path, &contents)?;
        let refs: Vec<&str> = launch.args.iter().map(String::as_str).collect();
        self.executor
            .run(&launch.program, &refs, false)
            .await
            .map(|_| ())
    }
}
