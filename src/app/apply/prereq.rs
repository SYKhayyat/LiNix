use crate::core::Result;
use crate::model::prereq::PrereqDef;
use tracing::{info, warn};

/// The setup steps LiNix ships with (Q10, Q11, Q13). Compiled in rather than read from the
/// repo, because a fresh machine has no `adapters/` file and `mix:` has to work on it.
pub const BUILTIN: &str = include_str!("prereq_builtins.toml");

/// Prereqs holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::prereqs()` and can be built without one.
pub struct Prereqs<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
}

impl Prereqs<'_> {
    /// The rows in force: the repo's own first, then the built-ins.
    ///
    /// The user's file goes through II.12's ledger like every other `adapters/` file — a setup
    /// command that arrives with a pulled repo must be approved before it can be offered. The
    /// built-in file is not gated, for the reason `snapshot_builtins.toml` is not: it is a
    /// first-party compiled-in asset, and gating it would leave a fresh machine unable to
    /// install through mix until `linix lock` had run.
    pub fn rows(&self) -> Vec<PrereqDef> {
        let mut rows = Vec::new();
        let layout = self.config.layout();
        if let Some(body) = crate::backends::onboarder::read_approved_definitions(
            &layout.adapter_prereq_file(),
            &layout.locks_dir(),
        ) {
            match toml::from_str::<crate::model::prereq::PrereqFile>(&body) {
                Ok(f) => rows.extend(f.prereq),
                Err(e) => warn!("ignoring adapters/prereq.toml: {}", e),
            }
        }
        match toml::from_str::<crate::model::prereq::PrereqFile>(BUILTIN) {
            Ok(f) => rows.extend(f.prereq),
            // Unreachable in a shipped binary — the file is compiled in and a test parses it —
            // and reported rather than unwrapped so a bad edit costs a message, not a panic.
            Err(e) => warn!("the built-in prereq rows did not parse: {}", e),
        }
        rows
    }

    /// Offer the setup a declared manager needs before it can install anything.
    ///
    /// **Ask, then do** (owner ruling, 2026-07-29). The three this ships with — Hex for `mix`,
    /// a plugin for `asdf`, a switch for `opam` — each made *every* install through that
    /// manager fail with a message only the user could act on. LiNix knows the command, so it
    /// offers to run it; it does not run it unasked, because `asdf plugin add` fetches a
    /// third-party repository whose scripts asdf then executes, and `opam switch create`
    /// builds a compiler and pins it for the account.
    ///
    /// A manager that is not installed at all is [`Bootstrap`](super::Bootstrap)'s question,
    /// and it runs first: there is no point probing `mix` for Hex on a machine with no Elixir.
    pub async fn offer(&self, state: &crate::model::DesiredState) -> Result<()> {
        let rows = self.rows();
        if rows.is_empty() {
            return Ok(());
        }
        let os = std::env::consts::OS;

        let mut managers: Vec<&String> = state.packages.keys().collect();
        managers.sort();
        for manager in managers {
            // A manager that is not here is not one whose setup can be probed. Its own absence
            // is the finding, and `Bootstrap` has already offered what it could.
            if !self
                .registry
                .get(manager)
                .map(|b| b.is_available())
                .unwrap_or(false)
            {
                continue;
            }
            for row in crate::model::prereq::for_manager(&rows, manager, os) {
                let names: Vec<String> = if row.is_per_package() {
                    state.packages[manager]
                        .iter()
                        .map(|s| s.name.clone())
                        .collect()
                } else {
                    vec![String::new()]
                };
                self.offer_row(row, manager, &names).await;
            }
        }
        Ok(())
    }

    /// One row, against every declared package it is about.
    ///
    /// The probe runs once and is read per package: `asdf plugin list` answers for all of
    /// them, and asking it once per declared tool would be a command per line.
    async fn offer_row(&self, row: &PrereqDef, manager: &str, names: &[String]) {
        let probe_once = row.probe_command(names.first().map(String::as_str).unwrap_or(""));
        let output = if row.probe_output.is_some() {
            let (p, a) = probe_once.split_first().expect("a usable row has a probe");
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            self.executor.run_output(p, &refs, false).await.ok()
        } else {
            None
        };

        for name in names {
            let satisfied = match row.expected_output(name) {
                Some(want) => output
                    .as_deref()
                    .is_some_and(|o| PrereqDef::output_satisfies(o, &want)),
                None => {
                    let cmd = row.probe_command(name);
                    let (p, a) = cmd.split_first().expect("a usable row has a probe");
                    let refs: Vec<&str> = a.iter().map(String::as_str).collect();
                    self.executor.run(p, &refs, false).await.is_ok()
                }
            };
            if satisfied {
                continue;
            }
            self.ask_and_run(row, manager, name).await;
        }
    }

    /// Print what is missing, ask, and run it. `--yes` answers for the caller; a
    /// non-interactive run without it says what it would have asked and changes nothing.
    async fn ask_and_run(&self, row: &PrereqDef, manager: &str, name: &str) {
        use std::io::IsTerminal;

        println!(
            "\n`{}` cannot install anything here yet: it needs {}.\nLiNix can set that up with:\n\n    {}\n",
            manager,
            row.missing_line(name),
            row.command_line(name)
        );

        if self.config.dry_run {
            crate::would_print!("a real run would ask before running that.");
            return;
        }
        let proceed = crate::core::prompt::confirm(
            self.config.yes,
            "Run that now?",
            crate::core::prompt::Unattended::Decline(
                "Not asking in a non-interactive shell — run it yourself, or re-run with \
                 `--yes` to have LiNix run it.",
            ),
        )
        .unwrap_or(false);
        if !proceed {
            if std::io::stdin().is_terminal() {
                println!(
                    "Left it alone. `{}` installs will fail until it is set up.",
                    manager
                );
            }
            return;
        }

        let cmd = row.run_command(name);
        let (program, args) = cmd.split_first().expect("a usable row has a command");
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        info!("setting up `{}`...", manager);
        match self.executor.run(program, &refs, false).await {
            Ok(_) => info!("done. The sync will use it."),
            // Reported, not fatal: the packages that needed it fail by name a moment later,
            // with the manager's own message, which is more useful than this one.
            Err(e) => warn!("could not set `{}` up: {}", manager, e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::executor::{DryRunOutput, MockExecutor};
    use dashmap::DashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Harness {
        exec: crate::core::CommandExecutor,
        mock: Arc<MockExecutor>,
        config: Arc<crate::config::Config>,
        registry: Arc<crate::backends::BackendRegistry>,
    }

    /// `answers` maps a whole command line to whether it succeeds and what it prints.
    /// Anything unmentioned succeeds silently, which is the mock's own default and is why
    /// every probe below is named explicitly.
    fn harness(answers: &[(&str, bool, &str)]) -> Harness {
        let vfs: Arc<DashMap<PathBuf, String>> = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        for (cmd, ok, out) in answers {
            let response = if *ok {
                DryRunOutput {
                    stdout: out.as_bytes().to_vec(),
                    stderr: vec![],
                }
                .into()
            } else {
                DryRunOutput::faulted(out)
            };
            mock.set_response(cmd, Ok(response));
        }
        let exec = crate::core::CommandExecutor::with_layer(
            false,
            false,
            mock.clone(),
            vfs,
            Arc::new(DashMap::new()),
        );
        Harness {
            exec,
            mock,
            config: Arc::new(crate::config::Config::default()),
            registry: Arc::new(crate::backends::BackendRegistry::new()),
        }
    }

    impl Harness {
        fn prereqs(&self) -> Prereqs<'_> {
            Prereqs {
                config: &self.config,
                executor: &self.exec,
                registry: &self.registry,
            }
        }
        async fn ran(&self, cmd: &str) -> bool {
            self.mock.get_calls().await.iter().any(|c| c == cmd)
        }
    }

    fn builtin(manager: &str) -> PrereqDef {
        let file: crate::model::prereq::PrereqFile = toml::from_str(BUILTIN).unwrap();
        file.prereq
            .into_iter()
            .find(|r| r.manager == manager)
            .expect("a shipped row")
    }

    /// The half that keeps the offer from becoming noise: Hex is already there, so nothing is
    /// printed, nothing is asked, and nothing runs. Without this, every `mix` sync on every
    /// machine would offer to install a thing that is installed.
    #[tokio::test]
    async fn a_prerequisite_that_is_already_met_runs_nothing() {
        // `yes` is set on purpose. Without it this passes whatever the probe decides, because
        // an unconfirmed offer runs nothing either — the check would be testing the
        // confirmation and claiming to test the probe. Measured: with `yes` off, deleting the
        // satisfied-check leaves this green.
        let mut h = harness(&[("mix hex.info", true, "Hex: 2.5.1")]);
        h.config = Arc::new(crate::config::Config {
            yes: true,
            ..Default::default()
        });
        h.prereqs()
            .offer_row(&builtin("mix"), "mix", &[String::new()])
            .await;
        assert!(
            !h.ran("mix local.hex --force").await,
            "it set up something that was already set up"
        );
    }

    /// The reported case: mix cannot install anything, and `--yes` is the flag that says do it
    /// (owner ruling, 2026-07-29). Measured in the tools image: without Hex, `mix hex.info` is
    /// `** (Mix) The task "hex.info" could not be found`, exit 1.
    #[tokio::test]
    async fn an_unmet_prerequisite_is_set_up_under_yes() {
        let mut h = harness(&[(
            "mix hex.info",
            false,
            "** (Mix) The task \"hex.info\" could not be found",
        )]);
        h.config = Arc::new(crate::config::Config {
            yes: true,
            ..Default::default()
        });
        h.prereqs()
            .offer_row(&builtin("mix"), "mix", &[String::new()])
            .await;
        assert!(
            h.ran("mix local.hex --force").await,
            "the setup command never ran: {:?}",
            h.mock.get_calls().await
        );
    }

    /// And without it. A test's stdin is not a terminal, so this is also the unattended path:
    /// LiNix says what it would have asked and changes nothing.
    #[tokio::test]
    async fn nothing_is_set_up_without_being_asked() {
        let h = harness(&[("mix hex.info", false, "could not be found")]);
        h.prereqs()
            .offer_row(&builtin("mix"), "mix", &[String::new()])
            .await;
        assert!(
            !h.ran("mix local.hex --force").await,
            "it ran a setup command nobody agreed to"
        );
    }

    /// A preview performs nothing — the property this repo's flagship bug broke. `--yes` is
    /// set too, so the only thing stopping it is the dry-run check.
    #[tokio::test]
    async fn a_dry_run_sets_nothing_up() {
        let mut h = harness(&[("mix hex.info", false, "could not be found")]);
        h.config = Arc::new(crate::config::Config {
            yes: true,
            dry_run: true,
            ..Default::default()
        });
        h.prereqs()
            .offer_row(&builtin("mix"), "mix", &[String::new()])
            .await;
        assert!(
            !h.ran("mix local.hex --force").await,
            "a preview installed something"
        );
    }

    /// asdf's prerequisite is per declared tool, and the probe answers for all of them at
    /// once. `asdf plugin list` exits 0 whatever it says, so a plugin that IS there must be
    /// read out of the output — and one that is not must still be offered.
    #[tokio::test]
    async fn a_per_package_row_asks_once_per_missing_package_only() {
        let mut h = harness(&[("asdf plugin list", true, "jq\n")]);
        h.config = Arc::new(crate::config::Config {
            yes: true,
            ..Default::default()
        });
        h.prereqs()
            .offer_row(
                &builtin("asdf"),
                "asdf",
                &["jq".to_string(), "nodejs".to_string()],
            )
            .await;
        assert!(
            h.ran("asdf plugin add nodejs").await,
            "the missing plugin was not offered: {:?}",
            h.mock.get_calls().await
        );
        assert!(
            !h.ran("asdf plugin add jq").await,
            "it added a plugin that `asdf plugin list` already showed"
        );
        assert_eq!(
            h.mock
                .get_calls()
                .await
                .iter()
                .filter(|c| c.as_str() == "asdf plugin list")
                .count(),
            1,
            "the probe ran once for the whole manager, not once per declared tool"
        );
    }

    /// The rows ship compiled in, so a machine with no `adapters/` directory still gets them —
    /// which is the whole point: `mix:` has to work on a fresh clone.
    #[test]
    fn the_shipped_rows_are_available_without_a_repo_file() {
        let h = harness(&[]);
        let rows = h.prereqs().rows();
        for manager in ["mix", "asdf", "opam"] {
            assert!(
                rows.iter().any(|r| r.manager == manager),
                "{manager} has no shipped row"
            );
        }
    }
}
