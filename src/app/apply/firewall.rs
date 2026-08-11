use crate::core::{Error, Result};
use tracing::{info, warn};

/// Firewall holds only what it uses. It is built from an [`App`](crate::app::App) by
/// `App::firewall()` and can be built without one.
pub struct Firewall<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    /// Held for one reason: [`crate::app::sync::guard::enforce_extras`] takes it, and closing an
    /// undeclared port is a removal. This struct carried only what it used, and what it used did
    /// not include the guard.
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    /// The command's teardown budget. Held rather than counted here: this phase passed `0` for
    /// what the rest of the command had already removed, which is the whole of `S55`.
    pub(crate) reaping: &'a crate::app::sync::guard::Reaping,
}

impl Firewall<'_> {
    /// Apply the declared perimeter (Part XI), or refuse for the one reason that cannot be
    /// undone from the far end of an SSH connection.
    ///
    /// The order is deliberate and is the feature's whole safety story:
    ///
    /// 1. Work out what would change.
    /// 2. **Ask whether it would close the port carrying this session** — before any command
    ///    runs, on every path that can close a port (N1's ruling makes that `sync`,
    ///    `purge-undeclared` and an unattended `watch` tick, the last being the dangerous one
    ///    because nobody is there to read a refusal).
    /// 3. Only then open, close and set policy.
    ///
    /// N6: a config that both declares rules and links a ruleset file **warns and applies
    /// both**, with the declaration winning. A base file plus overrides is legible; two silent
    /// owners are not.
    pub async fn apply(
        &self,
        state: &crate::model::DesiredState,
        scope: crate::app::sync::guard::GuardScope,
    ) -> Result<()> {
        use crate::model::firewall::{self, Direction, Rule};

        let declared: Vec<(Rule, &crate::config::grammar::Options)> = state
            .firewall_rules()
            .map(|(name, opts, _)| (Rule::parse(name), opts))
            .filter_map(|(r, o)| r.ok().map(|r| (r, o)))
            .collect();
        if declared.is_empty() {
            return Ok(());
        }

        let user_rows = self.firewall_adapter_rows();
        let all = crate::backends::firewall::adapters(user_rows);
        let present = |cmd: &str| self.executor.command_exists_sync(cmd);
        let Some(adapter) = crate::backends::firewall::detect(&all, &present) else {
            let known: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
            return Err(Error::Validation(format!(
                "`firewall:` lines are declared and Shall found no firewall here. It looked for \
                 {}. Add a `[[firewall]]` row to `adapters/firewall.toml` for the one this \
                 machine runs, or remove the lines — a perimeter Shall cannot apply must not \
                 read as one it did.",
                known.join(", ")
            )));
        };

        // N6: two owners of one perimeter is legible but must never be silent.
        self.warn_on_linked_ruleset(state);

        let wanted_ports: Vec<Rule> = declared
            .iter()
            .filter(|(r, _)| r.port().is_some())
            .map(|(r, _)| r.clone())
            .collect();
        let default_denies_incoming = declared.iter().any(|(r, o)| {
            matches!(
                r,
                Rule::Default {
                    direction: Direction::Incoming
                }
            ) && o.one("value").map(str::trim) == Some("deny")
        });

        // What is in force now, so the difference can be computed rather than reapplied.
        let list = adapter.list_command();
        let refs: Vec<&str> = list.iter().skip(1).map(String::as_str).collect();
        let in_force = match self.executor.run_output(&list[0], &refs, true).await {
            Ok(out) => adapter.parse_rules(&out),
            // A firewall that cannot be read is one whose drift cannot be seen, and closing
            // ports against an unknown baseline is how a machine goes dark. Refuse.
            Err(e) => {
                return Err(Error::Validation(format!(
                    "could not read the current rules from `{}` ({}). Shall will not change a \
                     perimeter it cannot see first.",
                    adapter.name, e
                )))
            }
        };

        let to_close: Vec<Rule> = in_force
            .iter()
            .filter(|r| !wanted_ports.contains(r))
            .cloned()
            .collect();

        // THE PRECONDITION. Before any command runs.
        if let Some(port) = firewall::would_close_session(
            &to_close,
            default_denies_incoming,
            &wanted_ports,
            self.session_port(),
        ) {
            return Err(Error::Refused(firewall::lockout_refusal(port, scope)));
        }

        let to_open: Vec<Rule> = wanted_ports
            .iter()
            .filter(|r| !in_force.contains(r))
            .cloned()
            .collect();

        if self.config.dry_run {
            for r in &to_open {
                crate::would!("would open {} via {}", r, adapter.name);
            }
            for r in &to_close {
                crate::would!("would close {} via {}", r, adapter.name);
            }
            return Ok(());
        }

        // Opening a port takes nothing away, so it answers to no ceiling of its own — but it is
        // a change, and `max_total_changes` counts changes (`N8`).
        crate::app::sync::guard::enforce_additions(self.config, to_open.len(), self.reaping, scope)
            .await?;
        for rule in &to_open {
            if let Rule::Port { port, proto } = rule {
                let argv = adapter.allow_command(*port, *proto);
                self.run_firewall(&argv).await?;
                info!("opened {} ({})", rule, adapter.name);
            }
        }
        // N7: drift is corrected, because it is corrected everywhere else in this model — and
        // the one exception was refused above rather than special-cased here.
        // **The teardown that was outside the guard.** `readme.md:358` promised every path
        // removing anything went through one guard and named six resource kinds; `firewall:` is
        // the seventh, and the word `guard` appeared nowhere in this file — not an import, not a
        // call, not a comment. `max_removals` did not count these, `protected` could not name
        // them, `--allow-mass-removal` was not consulted.
        //
        // Whoever wrote this understood the danger exactly: there are three bespoke refusals
        // above — an unreadable baseline, the SSH lockout, the linked-ruleset warning. **That is
        // what made it worse rather than better.** Three custom guards were written instead of
        // calling the one two hundred lines away that already counts, caps, protects and reports.
        if !to_close.is_empty() {
            let removals: Vec<(String, String)> = to_close
                .iter()
                .map(|r| ("firewall".to_string(), r.to_string()))
                .collect();
            let reaped = crate::app::sync::guard::enforce_ports(
                self.config,
                self.registry,
                &removals,
                self.reaping,
                scope,
            )
            .await?;
            for rule in &to_close {
                if let Rule::Port { port, proto } = rule {
                    let argv = adapter.deny_command(*port, *proto);
                    self.close_port(&argv, reaped).await?;
                    info!("closed {} — it was not declared ({})", rule, adapter.name);
                }
            }
        }
        for (rule, opts) in &declared {
            if let Rule::Default { direction } = rule {
                let policy = opts.one("value").unwrap_or("deny");
                match adapter.default_command(*direction, policy) {
                    Some(argv) => {
                        self.run_firewall(&argv).await?;
                        info!(
                            "default {} policy set to {} ({})",
                            direction, policy, adapter.name
                        );
                    }
                    // P7's rule: a refusal beats a pretence. Reporting success for a policy the
                    // firewall cannot express would be the worst outcome here.
                    None => {
                        return Err(Error::Validation(format!(
                            "`{}` cannot set a default {} policy, and `firewall:default/{}` \
                             asks for one. Remove the line, or add `default_{}` to that \
                             `[[firewall]]` row.",
                            adapter.name,
                            direction,
                            direction,
                            match direction {
                                Direction::Incoming => "in",
                                Direction::Outgoing => "out",
                            }
                        )))
                    }
                }
            }
        }
        Ok(())
    }
    /// The `[[firewall]]` rows the repo carries, through the same approval every adapter file
    /// goes through (7a/II.12).
    fn firewall_adapter_rows(&self) -> Vec<crate::backends::firewall::FirewallAdapter> {
        let layout = self.config.layout();
        let path = layout.adapter_firewall_file();
        let Some(body) =
            crate::backends::onboarder::read_approved_definitions(&path, &layout.locks_dir())
        else {
            return Vec::new();
        };
        match toml::from_str::<crate::backends::firewall::FirewallAdapterFile>(&body) {
            Ok(f) => f.firewall,
            Err(e) => {
                warn!(
                    "{}",
                    crate::app::adapters::cannot_use(
                        crate::app::adapters::surface("firewall").expect("a declared surface"),
                        e,
                    )
                );
                Vec::new()
            }
        }
    }
    /// N6: warn when a `link:` also owns a firewall ruleset file. Both are applied and the
    /// declaration wins, but two owners of one perimeter is never silent.
    fn warn_on_linked_ruleset(&self, state: &crate::model::DesiredState) {
        use crate::config::grammar::Statement;
        const RULESET_HINTS: [&str; 4] = ["ufw", "firewalld", "iptables", "nftables"];
        for (stmt, origin) in &state.extras {
            let Statement::Link(name, opts) = stmt else {
                continue;
            };
            let target = opts.one("target").unwrap_or(name).to_lowercase();
            if RULESET_HINTS.iter().any(|h| target.contains(h)) {
                warn!(
                    "{}: this `link:` writes what looks like a firewall ruleset ({}), and \
                     `firewall:` lines are declared too. Both are applied and the declared \
                     rules win where they disagree — but two things own this perimeter, so a \
                     change in one may be undone by the other.",
                    origin, target
                );
            }
        }
    }
    /// The local port carrying the connection Shall is being run over, if any.
    ///
    /// Read from `SSH_CONNECTION` — the shell sets it to
    /// `<client ip> <client port> <server ip> <server port>`, and the last field is the port
    /// this session arrived on. A console session has no such variable and nothing to lose.
    fn session_port(&self) -> Option<u16> {
        let raw = std::env::var("SSH_CONNECTION").ok()?;
        raw.split_whitespace().nth(3)?.parse().ok()
    }
    /// Run a firewall command that **opens** a port or sets a policy — anything but a removal.
    ///
    /// The split from [`Firewall::close_port`] is the point: one function for every firewall
    /// command meant the removing call and the non-removing calls were indistinguishable, which
    /// is why `removal_guard_enumeration_tests.rs`'s scanner could not see one of them.
    async fn run_firewall(&self, argv: &[String]) -> Result<()> {
        let (program, args) = argv
            .split_first()
            .expect("an adapter command is never empty");
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // A firewall is root's business on every platform Shall drives.
        self.executor.run(program, &refs, true).await.map(|_| ())
    }

    /// Close a port that is open and undeclared. **A removal**, and it takes the proof.
    ///
    /// `deny_command` returns argv rather than performing the removal, so the token goes on the
    /// call that runs it. The shape is not perfectly uniform with the other five effectors and
    /// that is better said than papered over — what matters is that the only path in this file
    /// that takes something away cannot be reached without asking.
    async fn close_port(
        &self,
        argv: &[String],
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        self.run_firewall(argv).await
    }
}
