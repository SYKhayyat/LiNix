//! `firewall:22/tcp` — a perimeter you declare (Part XI, N1–N7).
//!
//! **The lockout check is this feature's precondition, not one of its features.** Everything
//! else here manages rules; this decides whether LiNix is allowed to close the port carrying
//! the connection it is being typed over. Building the backend before the check is building
//! the lockout — so the check lives at the bottom of the module and everything above it is
//! written knowing it exists.
//!
//! Rulings of 2026-07-24:
//!
//! - **N4** — the default policy may be a `firewall:` statement *or* a `preferences.toml` key,
//!   and where both speak, **the declaration wins**: it is the one you can read, review and
//!   share, and a machine-local key silently overriding it would be the invisible answer
//!   beating the visible one.
//! - **N5** — removing a rule restores the firewall's own default, exactly as a removed
//!   `setting:` resets to the schema default. Restoring a per-rule prior state would mean
//!   keeping a per-rule store of it.
//! - **N6** — declaring rules *and* linking a ruleset file **warns and applies both**, with the
//!   `firewall:` line winning where they disagree. A base file plus overrides is legible; two
//!   silent owners are not.
//! - **N7** — unattended `watch` **reverts** drift, because drift is corrected everywhere else
//!   in this model — *except* where the revert would close the session's own port, which is
//!   reported and left alone. An un-reverted rule is fixed tomorrow; a reverted one can be a
//!   machine you cannot reach.

use std::fmt;

/// What a `firewall:` line names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    /// `firewall:22/tcp` — a port, and the protocol it is spoken over.
    Port { port: u16, proto: Proto },
    /// `firewall:default/incoming` — the policy for traffic no rule mentions. The most
    /// consequential line in a firewall, which is why it is declarable at all (N4).
    Default { direction: Direction },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Incoming,
    Outgoing,
}

impl fmt::Display for Proto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        })
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Direction::Incoming => "incoming",
            Direction::Outgoing => "outgoing",
        })
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rule::Port { port, proto } => write!(f, "{}/{}", port, proto),
            Rule::Default { direction } => write!(f, "default/{}", direction),
        }
    }
}

impl Rule {
    /// Parse the name half of a `firewall:` line. One parser, so the refusal and the adapter
    /// cannot disagree about what a rule is.
    pub fn parse(name: &str) -> Result<Rule, String> {
        let (head, tail) = name.trim().split_once('/').ok_or_else(|| {
            format!(
                "`{}` is not a firewall rule. Write a port and protocol (`22/tcp`) or a \
                 default policy (`default/incoming`).",
                name
            )
        })?;
        let (head, tail) = (head.trim(), tail.trim());

        if head.eq_ignore_ascii_case("default") {
            return match tail.to_lowercase().as_str() {
                "incoming" => Ok(Rule::Default {
                    direction: Direction::Incoming,
                }),
                "outgoing" => Ok(Rule::Default {
                    direction: Direction::Outgoing,
                }),
                _ => Err(format!(
                    "`default/{}` names no direction. It is `default/incoming` or \
                     `default/outgoing`.",
                    tail
                )),
            };
        }

        let port: u16 = head
            .parse()
            .map_err(|_| format!("`{}` is not a port number (1–65535).", head))?;
        if port == 0 {
            return Err("port 0 is not a port you can open.".to_string());
        }
        let proto = match tail.to_lowercase().as_str() {
            "tcp" => Proto::Tcp,
            "udp" => Proto::Udp,
            _ => {
                return Err(format!(
                    "`{}` is not a protocol. LiNix speaks `tcp` and `udp`; anything else is a \
                     rule your firewall's own tool should write.",
                    tail
                ))
            }
        };
        Ok(Rule::Port { port, proto })
    }

    /// The port this rule concerns, if it concerns one. A default policy concerns them all,
    /// which is why the lockout check treats it separately.
    pub fn port(&self) -> Option<u16> {
        match self {
            Rule::Port { port, .. } => Some(*port),
            Rule::Default { .. } => None,
        }
    }
}

/// Whether applying `plan` would cut the connection LiNix is being run over (Part XI's
/// precondition).
///
/// `session_port` is the local port carrying the controlling connection — `None` when LiNix is
/// on a console and there is nothing to lose.
///
/// **A tightened default policy counts.** Closing `default/incoming` is not a rule about port
/// 22, but it closes port 22 all the same unless something else opens it — and the whole point
/// of this check is that the user is on the far end of that port right now.
pub fn would_close_session(
    removing: &[Rule],
    default_incoming_becomes_deny: bool,
    still_open: &[Rule],
    session_port: Option<u16>,
) -> Option<u16> {
    let port = session_port?;

    // Explicitly taking away the rule that keeps this connection alive.
    if removing.iter().any(|r| r.port() == Some(port)) {
        return Some(port);
    }

    // Or shutting the door everything unlisted came through, with nothing left to hold it open.
    if default_incoming_becomes_deny && !still_open.iter().any(|r| r.port() == Some(port)) {
        return Some(port);
    }
    None
}

/// The refusal, in the words a locked-out user would have needed.
pub fn lockout_refusal(port: u16, scope: crate::app::sync::guard::GuardScope) -> String {
    format!(
        "refusing to apply the firewall change: it would close port {}, which is carrying this \
         session.\n  \
         LiNix is being run over that port, so applying this would end the connection and \
         leave no way back in.\n  \
         Declare `firewall:{}/tcp` to keep it open, or make this change from the machine's own \
         console.\n  \
         (refused during {})",
        port,
        port,
        scope.during()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(p: u16) -> Rule {
        Rule::Port {
            port: p,
            proto: Proto::Tcp,
        }
    }

    #[test]
    fn a_port_and_protocol_parse() {
        assert_eq!(Rule::parse("22/tcp"), Ok(port(22)));
        assert_eq!(
            Rule::parse("53/udp"),
            Ok(Rule::Port {
                port: 53,
                proto: Proto::Udp
            })
        );
        assert_eq!(Rule::parse(" 8080 / TCP "), Ok(port(8080)));
    }

    #[test]
    fn a_default_policy_parses_in_both_directions() {
        assert_eq!(
            Rule::parse("default/incoming"),
            Ok(Rule::Default {
                direction: Direction::Incoming
            })
        );
        assert_eq!(
            Rule::parse("default/outgoing"),
            Ok(Rule::Default {
                direction: Direction::Outgoing
            })
        );
    }

    /// A rule LiNix cannot express must be refused, not approximated: a firewall line that
    /// half-applies is a perimeter nobody can reason about.
    #[test]
    fn anything_else_is_refused_with_a_reason() {
        for bad in [
            "22",
            "22/sctp",
            "http/tcp",
            "0/tcp",
            "99999/tcp",
            "default/sideways",
        ] {
            let err = Rule::parse(bad).unwrap_err();
            assert!(!err.is_empty(), "{} produced an empty error", bad);
        }
    }

    #[test]
    fn a_rule_round_trips_through_display() {
        for text in ["22/tcp", "53/udp", "default/incoming", "default/outgoing"] {
            assert_eq!(Rule::parse(text).unwrap().to_string(), text);
        }
    }

    // ---- the precondition -------------------------------------------------

    /// The plain case: removing the rule that holds this connection open.
    #[test]
    fn removing_the_session_port_is_refused() {
        assert_eq!(
            would_close_session(&[port(22)], false, &[], Some(22)),
            Some(22)
        );
    }

    /// The subtle case, and the reason this function takes the default policy at all: tightening
    /// `default/incoming` closes the session's port without ever naming it.
    #[test]
    fn tightening_the_default_closes_the_session_port_without_naming_it() {
        assert_eq!(would_close_session(&[], true, &[], Some(22)), Some(22));
        // ...unless something still holds it open.
        assert_eq!(would_close_session(&[], true, &[port(22)], Some(22)), None);
    }

    #[test]
    fn a_change_that_leaves_the_session_alone_is_permitted() {
        assert_eq!(
            would_close_session(&[port(8080)], false, &[port(22)], Some(22)),
            None
        );
    }

    /// On a console there is no connection to lose, so nothing is refused on these grounds.
    #[test]
    fn with_no_session_port_nothing_is_refused() {
        assert_eq!(would_close_session(&[port(22)], true, &[], None), None);
    }

    /// The refusal has to be actionable: it names the port, what would happen, and the two ways
    /// out. A user reading it is, by construction, about to lose their connection.
    ///
    /// **It used to take the prose as a `&str`, and this test handed it the prose directly** —
    /// which is why nobody noticed that the only producer emitted `"an unattended watch tick"`
    /// while the only consumer matched on `"watch"`. The test asserted the string it had just
    /// written down. It now passes the scope, so the wording is the enum's answer and this
    /// asserts the round trip rather than a literal.
    #[test]
    fn the_refusal_names_the_port_and_the_way_out() {
        use crate::app::sync::guard::GuardScope;

        let msg = lockout_refusal(22, GuardScope::Watch);
        assert!(msg.contains("port 22"), "{}", msg);
        assert!(msg.contains("firewall:22/tcp"), "{}", msg);
        assert!(msg.contains("console"), "{}", msg);
        assert!(
            msg.contains("an unattended watch tick"),
            "an unattended tick is the dangerous one (N7) and the refusal must say so: {}",
            msg
        );

        // And it must not say the same thing about every scope, which is what the label it
        // replaced did for nine of the twelve.
        let sync = lockout_refusal(22, GuardScope::Sync);
        assert!(
            !sync.contains("unattended"),
            "an attended `sync` was described as unattended: {}",
            sync
        );
        let purge = lockout_refusal(22, GuardScope::PurgeUndeclared);
        assert!(
            purge.contains("purge-undeclared"),
            "a purge was reported under another command's name: {}",
            purge
        );
    }
}
