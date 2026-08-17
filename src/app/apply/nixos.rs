//! `service:` and `firewall:` on a NixOS machine — `J5`'s fourth ruling, which was *everything*.
//!
//! **On NixOS these are not commands, they are configuration.** `systemctl enable nginx` writes
//! into a tree the next `nixos-rebuild switch` regenerates, so an enablement Shall issued
//! imperatively survives exactly until the next generation — including the generation Shall
//! itself builds one line later when a `nixos:` package changes. And `ufw` is not there at all:
//! a NixOS box declaring `firewall:22/tcp` reached [`Firewall::apply`](super::Firewall::apply),
//! found no adapter, and failed the whole sync by name. So the perimeter and the services go
//! into the same generated module the packages do, and one `nixos-rebuild` applies all three.
//!
//! **What is not moved here: a restart.** `services.<name>.enable` is a state; `@status=
//! restarted` is a transition, and no attribute in a NixOS module can express it. Those lines
//! keep going through the init exactly as on every other host — see
//! [`nixos::service_routing`](crate::backends::nixos::service_routing) for the split and
//! [`imperative_remainder`] for what `Dependents` is left holding.
//!
//! **The safety story is not weakened by the move.** The lockout check, the removal guard and
//! the addition ceiling all run here too, against the same functions the adapter path calls —
//! because a port dropped from `allowedTCPPorts` closes just as hard as one `ufw delete` takes
//! away, and it closes on a machine whose rebuild takes minutes to undo.

use crate::config::grammar::{Options, Statement};
use crate::core::{Error, Result};
use crate::model::firewall::{Direction, Proto, Rule};
use std::collections::{BTreeMap, BTreeSet};
use tracing::info;

/// Holds exactly what it uses, like every other applier in this module.
pub struct SystemConfig<'a> {
    pub(crate) config: &'a std::sync::Arc<crate::config::Config>,
    pub(crate) executor: &'a crate::core::CommandExecutor,
    pub(crate) registry: &'a std::sync::Arc<crate::backends::BackendRegistry>,
    pub(crate) reaping: &'a crate::app::sync::guard::Reaping,
}

/// What a desired state says the system configuration should hold, before anything on disk is
/// consulted. Pure, so every rule below is testable without a NixOS.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    pub services: BTreeMap<String, bool>,
    pub tcp_ports: BTreeSet<u16>,
    pub udp_ports: BTreeSet<u16>,
    pub firewall: Option<bool>,
    /// Whether any `service:` or `firewall:` line was declared at all. Not the same as the
    /// fields being empty: a config that used to declare a service and no longer does has an
    /// empty map and real work to do.
    pub any: bool,
}

impl Declared {
    fn ports(&self) -> impl Iterator<Item = (u16, Proto)> + '_ {
        self.tcp_ports
            .iter()
            .map(|p| (*p, Proto::Tcp))
            .chain(self.udp_ports.iter().map(|p| (*p, Proto::Udp)))
    }

    fn rules(&self) -> Vec<Rule> {
        self.ports()
            .map(|(port, proto)| Rule::Port { port, proto })
            .collect()
    }
}

/// Project a desired state onto the attributes NixOS reads.
///
/// Every refusal in here is P7's rule: a line this OS cannot express is named, never
/// approximated. A half-applied perimeter or a service that reports enabled and is not are both
/// worse than a message telling the user which line to change.
pub fn declared(state: &crate::model::DesiredState) -> Result<Declared> {
    let mut out = Declared::default();

    for (stmt, origin) in state.dependents() {
        let Statement::Service(name, opts) = stmt else {
            continue;
        };
        out.any = true;
        let (enable, _restart) =
            crate::backends::nixos::service_routing(opts.one("enabled"), opts.one("status"))
                .map_err(|e| Error::Validation(format!("{}: `service:{}` {}", origin, name, e)))?;
        if let Some(on) = enable {
            // A name declared twice with two answers is the same contradiction one line apart,
            // and it must not be settled by whichever module happened to load last.
            if let Some(previous) = out.services.insert(name.clone(), on) {
                if previous != on {
                    return Err(Error::Validation(format!(
                        "{}: `service:{}` is declared both on and off. On NixOS that is one \
                         attribute, `services.{}.enable`, so the two lines cannot both hold.",
                        origin, name, name
                    )));
                }
            }
        }
    }

    for (name, opts, origin) in state.firewall_rules() {
        out.any = true;
        let rule =
            Rule::parse(name).map_err(|e| Error::Validation(format!("{}: {}", origin, e)))?;
        match rule {
            Rule::Port { port, proto } => {
                match proto {
                    Proto::Tcp => out.tcp_ports.insert(port),
                    Proto::Udp => out.udp_ports.insert(port),
                };
            }
            Rule::Default {
                direction: Direction::Incoming,
            } => {
                let policy = opts.one("value").map(str::trim).unwrap_or("deny");
                out.firewall = Some(match policy {
                    "deny" | "reject" | "drop" => true,
                    "allow" | "accept" => false,
                    other => {
                        return Err(Error::Validation(format!(
                            "{}: `firewall:default/incoming @value={}` names no policy NixOS \
                             has. It is `deny` (which is `networking.firewall.enable = true`) \
                             or `allow` (which turns the firewall off).",
                            origin, other
                        )))
                    }
                });
            }
            // NixOS's `networking.firewall` filters what comes in. There is no attribute for a
            // default outgoing policy, and inventing one out of raw nftables rules would be
            // Shall writing a firewall rather than declaring one.
            Rule::Default {
                direction: Direction::Outgoing,
            } => {
                return Err(Error::Validation(format!(
                    "{}: `firewall:default/outgoing` asks for a default outgoing policy, and \
                     NixOS's `networking.firewall` has no such option — it filters incoming \
                     traffic. Remove the line, or write the rule yourself with \
                     `networking.firewall.extraCommands` in your own configuration.",
                    origin
                )))
            }
        }
    }

    Ok(out)
}

/// What `Dependents` is still left to do with a `service:` line here: `None` when the generated
/// module says all of it.
///
/// Returns the options trimmed to the transition alone, so the init is asked for the restart and
/// not for an enablement the configuration already owns. Passing the original options through
/// would re-issue `systemctl enable` beside `services.<name>.enable`, which is the two-owners
/// problem this whole module exists to remove.
pub fn imperative_remainder(opts: &Options) -> Option<Options> {
    let (_, restart) =
        crate::backends::nixos::service_routing(opts.one("enabled"), opts.one("status")).ok()?;
    if !restart {
        return None;
    }
    let mut trimmed = Options::default();
    trimmed.set("status", "restarted");
    Some(trimmed)
}

/// Whether this machine's `service:` and `firewall:` lines are the system configuration's
/// business.
///
/// Asked of the registered backend rather than re-testing `/etc/NIXOS`, so there is one answer to
/// *is this NixOS*. A free function because two callers need it and only one of them is a
/// [`SystemConfig`]: [`Dependents`](super::Dependents) has to know to pass the `service:` lines
/// over, and a second copy of this test in that file is how the two would come to disagree.
pub fn owns_extras(registry: &crate::backends::BackendRegistry) -> bool {
    registry
        .get("nixos")
        .map(|b| b.is_available())
        .unwrap_or(false)
}

impl SystemConfig<'_> {
    pub fn owns_extras(&self) -> bool {
        owns_extras(self.registry)
    }

    fn core(&self) -> crate::backends::nixos::NixosBackendCore {
        crate::backends::nixos::NixosBackendCore::new(
            self.executor.clone(),
            self.config.nixos.config_dir.clone(),
            self.config.nixos.manage_imports,
        )
    }

    /// Write the declared services and perimeter into the system configuration and switch to it.
    pub async fn apply(
        &self,
        state: &crate::model::DesiredState,
        scope: crate::app::sync::guard::GuardScope,
    ) -> Result<()> {
        let want = declared(state)?;
        let core = self.core();
        let current = core.declared();

        // **Nothing declared and nothing already ours is not a reason to write a file.** A NixOS
        // user who only ever says `nix:` should not find `/etc/nixos` edited and an `imports`
        // line added because Shall ran.
        if !want.any && !core.generated_path().exists() {
            return Ok(());
        }

        let wanted_ports: BTreeSet<u16> = want.ports().map(|(p, _)| p).collect();
        let closing: Vec<Rule> = current
            .ports()
            .filter(|(p, _)| !wanted_ports.contains(p))
            .map(|(port, proto)| Rule::Port { port, proto })
            .collect();
        let opening: Vec<Rule> = want
            .rules()
            .into_iter()
            .filter(|r| match r {
                Rule::Port { port, proto } => match proto {
                    Proto::Tcp => !current.tcp_ports.contains(port),
                    Proto::Udp => !current.udp_ports.contains(port),
                },
                Rule::Default { .. } => false,
            })
            .collect();
        let disabling: Vec<(String, String)> = current
            .services
            .keys()
            .filter(|name| !want.services.contains_key(*name))
            .map(|name| ("service".to_string(), name.clone()))
            .collect();

        // THE PRECONDITION, before anything is written. A port that leaves `allowedTCPPorts`
        // is shut by the rebuild just as surely as by `ufw delete`, and the machine that shuts
        // it takes minutes to rebuild back.
        if let Some(port) = crate::model::firewall::would_close_session(
            &closing,
            want.firewall == Some(true),
            &want.rules(),
            crate::model::firewall::session_port(),
        ) {
            return Err(Error::Refused(crate::model::firewall::lockout_refusal(
                port, scope,
            )));
        }

        if self.config.dry_run {
            for r in &opening {
                crate::would!("would open {} in the NixOS system configuration", r);
            }
            for r in &closing {
                crate::would!("would close {} in the NixOS system configuration", r);
            }
            for (name, on) in &want.services {
                if current.services.get(name) != Some(on) {
                    crate::would!("would declare services.{}.enable = {}", name, on);
                }
            }
            for (_, name) in &disabling {
                crate::would!("would stop declaring services.{}", name);
            }
            return Ok(());
        }

        // Placing takes nothing away; it is still a change, and `max_total_changes` counts
        // changes (`N8`).
        let additions = opening.len()
            + want
                .services
                .iter()
                .filter(|(name, on)| current.services.get(*name) != Some(on))
                .count();
        crate::app::sync::guard::enforce_additions(self.config, additions, self.reaping, scope)
            .await?;

        // Two budgets, because they are two kinds of loss: a closed port answers to
        // `max_port_closures`, an undeclared resource to `max_extra_removals` (`N8`).
        if !closing.is_empty() {
            let removals: Vec<(String, String)> = closing
                .iter()
                .map(|r| ("firewall".to_string(), r.to_string()))
                .collect();
            let _reaped = crate::app::sync::guard::enforce_ports(
                self.config,
                self.registry,
                &removals,
                self.reaping,
                scope,
            )
            .await?;
        }
        if !disabling.is_empty() {
            let _reaped = crate::app::sync::guard::enforce_extras(
                self.config,
                self.registry,
                &disabling,
                self.reaping,
                scope,
            )
            .await?;
        }

        let module = crate::backends::nixos::Module {
            // The packages are the other writer's half and are carried through untouched.
            packages: current.packages,
            services: want.services.clone(),
            tcp_ports: want.tcp_ports.clone(),
            udp_ports: want.udp_ports.clone(),
            firewall: want.firewall,
        };
        core.write_and_switch(&module).await?;
        for r in &opening {
            info!("opened {} (NixOS system configuration)", r);
        }
        for r in &closing {
            info!("closed {} — it was not declared (NixOS)", r);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::grammar::Origin;

    fn origin() -> Origin {
        Origin::new("modules/test.txt", 1)
    }

    fn opts(pairs: &[(&str, &str)]) -> Options {
        let mut o = Options::default();
        for (k, v) in pairs {
            o.set(*k, *v);
        }
        o
    }

    fn state(extras: Vec<Statement>) -> crate::model::DesiredState {
        crate::model::DesiredState {
            extras: extras.into_iter().map(|s| (s, origin())).collect(),
            ..Default::default()
        }
    }

    /// The whole of `J5`'s fourth ruling in one projection: services and the perimeter reach the
    /// attributes NixOS reads.
    #[test]
    fn services_and_the_perimeter_become_nixos_attributes() {
        let d = declared(&state(vec![
            Statement::Service("nginx".into(), Options::default()),
            Statement::Service("telnet".into(), opts(&[("status", "stopped")])),
            Statement::Firewall("22/tcp".into(), Options::default()),
            Statement::Firewall("53/udp".into(), Options::default()),
            Statement::Firewall("default/incoming".into(), opts(&[("value", "deny")])),
        ]))
        .expect("a state every attribute can express");

        assert_eq!(d.services.get("nginx"), Some(&true));
        assert_eq!(d.services.get("telnet"), Some(&false));
        assert_eq!(d.tcp_ports, [22u16].into_iter().collect());
        assert_eq!(d.udp_ports, [53u16].into_iter().collect());
        assert_eq!(d.firewall, Some(true));
        assert!(d.any);
    }

    /// `allow` is the other half of the default policy, and it is the one that would be silently
    /// wrong: an unwritten attribute is not "allow", it is NixOS's own default.
    #[test]
    fn an_allow_default_turns_the_firewall_off_rather_than_leaving_it_unsaid() {
        let d = declared(&state(vec![Statement::Firewall(
            "default/incoming".into(),
            opts(&[("value", "allow")]),
        )]))
        .expect("allow is expressible");
        assert_eq!(d.firewall, Some(false));
    }

    /// Nothing declared is `any == false`, which is what keeps Shall out of `/etc/nixos` on a
    /// machine that never asked it to write there.
    #[test]
    fn a_state_with_neither_declares_nothing_and_says_so() {
        let d = declared(&state(vec![])).expect("an empty state");
        assert_eq!(d, Declared::default());
        assert!(!d.any);
    }

    /// Every shape NixOS cannot express, refused by name rather than approximated (P7). Each of
    /// these produced a *different* wrong answer before: a policy silently ignored, a service
    /// enabled when it was declared stopped, a perimeter half applied.
    #[test]
    fn a_line_this_os_cannot_express_is_refused_by_name() {
        let cases: Vec<(Statement, &str)> = vec![
            (
                Statement::Firewall("default/outgoing".into(), Options::default()),
                "outgoing",
            ),
            (
                Statement::Firewall("default/incoming".into(), opts(&[("value", "maybe")])),
                "policy",
            ),
            (
                Statement::Service(
                    "nginx".into(),
                    opts(&[("enabled", "false"), ("status", "running")]),
                ),
                "enable",
            ),
        ];
        for (stmt, needle) in cases {
            let err = declared(&state(vec![stmt.clone()]))
                .expect_err(&format!("{:?} must be refused", stmt))
                .to_string();
            assert!(err.contains(needle), "{err}");
            // And the refusal has to say which line, or a user with forty modules cannot act
            // on it.
            assert!(err.contains("modules/test.txt"), "{err}");
        }
    }

    /// Two modules declaring one service two ways is the same contradiction one line apart. It
    /// must not be settled by load order — on systemd the last write wins and the machine ends
    /// up in whichever state was applied last, which is exactly the drift this tool removes.
    #[test]
    fn one_service_declared_two_ways_is_refused_rather_than_settled_by_order() {
        let err = declared(&state(vec![
            Statement::Service("nginx".into(), opts(&[("enabled", "true")])),
            Statement::Service("nginx".into(), opts(&[("enabled", "false")])),
        ]))
        .expect_err("two answers for one attribute")
        .to_string();
        assert!(err.contains("nginx"), "{err}");
        // The same name declared the same way twice is not a contradiction and must pass.
        declared(&state(vec![
            Statement::Service("nginx".into(), opts(&[("enabled", "true")])),
            Statement::Service("nginx".into(), Options::default()),
        ]))
        .expect("two lines agreeing is not a conflict");
    }

    /// A restart is the one thing that stays imperative, and the options handed on carry the
    /// transition alone — the enablement is the configuration's now, and asking for it twice is
    /// the two-owners problem.
    #[test]
    fn only_a_restart_is_left_for_the_init_to_do() {
        assert!(imperative_remainder(&Options::default()).is_none());
        assert!(imperative_remainder(&opts(&[("enabled", "true")])).is_none());
        assert!(imperative_remainder(&opts(&[("status", "running")])).is_none());
        assert!(imperative_remainder(&opts(&[("status", "stopped")])).is_none());

        for restart in ["restarted", "restart"] {
            let left = imperative_remainder(&opts(&[("status", restart)]))
                .expect("a restart is still the init's");
            assert_eq!(left.one("status"), Some("restarted"));
            assert_eq!(left.one("enabled"), None);
        }
        // And with an enablement beside it, the enablement does NOT come along.
        let left = imperative_remainder(&opts(&[("enabled", "true"), ("status", "restarted")]))
            .expect("the restart half survives");
        assert_eq!(left.one("enabled"), None);
        assert_eq!(left.len(), 1);
    }

    /// A `service:` line whose declaration is a contradiction returns `None` here rather than
    /// panicking — the refusal is `declared`'s to raise, with the origin in hand, and this must
    /// not race it to a worse message.
    #[test]
    fn a_contradictory_line_leaves_the_refusal_to_the_projection() {
        assert!(
            imperative_remainder(&opts(&[("enabled", "false"), ("status", "running")])).is_none()
        );
    }
}
