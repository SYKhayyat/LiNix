//! Driving whichever firewall this machine runs (Part XI).
//!
//! **Rows, not Rust** — K17's ruling, applied here because it was the reason 7o was scheduled
//! after it: `ufw`, `firewalld` and Windows Defender are three commands for one idea, and a
//! machine running a fourth should not wait for a LiNix release. The shipped three are rows in
//! `firewall_adapters.toml`, parsed by the loader a user's own row goes through, because an
//! adapter mechanism the built-ins bypass is one nobody has tested.
//!
//! **One spelling across firewalls is the whole point (XI.2).** `firewall:22/tcp` means the
//! same thing on the Debian laptop and the Windows workstation — which a user-defined
//! `[[backend]]` naming `ufw` could never do, and which is why this is a built-in rather than
//! something the onboarder covers.

use crate::model::firewall::{Direction, Proto, Rule};
use serde::Deserialize;

/// One firewall: how to tell it is here, and the commands that open, close, list and set the
/// default policy.
#[derive(Debug, Clone, Deserialize)]
pub struct FirewallAdapter {
    pub name: String,
    pub detect: String,
    #[serde(default)]
    pub os: Option<String>,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    #[serde(default)]
    pub default_in: Vec<String>,
    #[serde(default)]
    pub default_out: Vec<String>,
    pub list: Vec<String>,
    /// A regex whose first two capture groups are the port and the protocol.
    pub list_pattern: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FirewallAdapterFile {
    #[serde(default)]
    pub firewall: Vec<FirewallAdapter>,
}

const BUILTIN: &str = include_str!("firewall_adapters.toml");

impl FirewallAdapter {
    fn fill(args: &[String], port: &str, proto: &str, policy: &str) -> Vec<String> {
        args.iter()
            .map(|a| {
                a.replace("{port}", port)
                    .replace("{proto}", proto)
                    .replace("{policy}", policy)
            })
            .collect()
    }

    /// The command that opens a port.
    pub fn allow_command(&self, port: u16, proto: Proto) -> Vec<String> {
        Self::fill(&self.allow, &port.to_string(), &proto.to_string(), "")
    }

    /// The command that closes one.
    pub fn deny_command(&self, port: u16, proto: Proto) -> Vec<String> {
        Self::fill(&self.deny, &port.to_string(), &proto.to_string(), "")
    }

    /// The command that sets a default policy, if this firewall can express one in that
    /// direction. `None` is a refusal the caller reports by name — never a silent skip.
    pub fn default_command(&self, direction: Direction, policy: &str) -> Option<Vec<String>> {
        let args = match direction {
            Direction::Incoming => &self.default_in,
            Direction::Outgoing => &self.default_out,
        };
        if args.is_empty() {
            return None;
        }
        Some(Self::fill(args, "", "", policy))
    }

    pub fn list_command(&self) -> Vec<String> {
        self.list.clone()
    }

    /// The rules this firewall reports as in force.
    ///
    /// A line that does not parse is skipped rather than guessed at: a firewall's status output
    /// carries headers, chains and comments, and inventing a rule out of one would put a
    /// phantom in the plan — the S22/S23 class, one domain over.
    pub fn parse_rules(&self, output: &str) -> Vec<Rule> {
        let Ok(re) = crate::utils::regex_cache::compiled(&self.list_pattern) else {
            return Vec::new();
        };
        // Every match on every line, not the first per line: `ufw status` puts one rule on a
        // line, while `firewall-cmd --list-ports` puts them all on one. A per-line parser reads
        // the second firewall as having a single rule and silently drops the rest, which would
        // make `sync` "correct" drift that was never there.
        let mut out = Vec::new();
        for line in output.lines() {
            for caps in re.captures_iter(line) {
                let (Some(port), Some(proto)) = (caps.get(1), caps.get(2)) else {
                    continue;
                };
                if let Ok(rule) = Rule::parse(&format!("{}/{}", port.as_str(), proto.as_str())) {
                    if !out.contains(&rule) {
                        out.push(rule);
                    }
                }
            }
        }
        out
    }

    fn applies_to_this_os(&self) -> bool {
        match &self.os {
            Some(os) => os.eq_ignore_ascii_case(std::env::consts::OS),
            None => true,
        }
    }

    /// A row LiNix will act on, or why it will not.
    fn is_usable(&self) -> Option<&'static str> {
        if self.name.trim().is_empty() {
            return Some("it has no `name`");
        }
        if self.detect.trim().is_empty() {
            return Some("it has no `detect` command");
        }
        if self.allow.is_empty() || self.deny.is_empty() {
            return Some("it cannot both open and close a port");
        }
        if self.list.is_empty() {
            return Some("it cannot list its rules, so drift could never be seen");
        }
        None
    }
}

/// Every firewall adapter this machine knows: the shipped rows, then the user's.
pub fn adapters(user_rows: Vec<FirewallAdapter>) -> Vec<FirewallAdapter> {
    let shipped: FirewallAdapterFile =
        toml::from_str(BUILTIN).expect("the shipped firewall_adapters.toml must parse");
    let mut out: Vec<FirewallAdapter> = Vec::new();
    for row in shipped.firewall.into_iter().chain(user_rows) {
        if let Some(why) = row.is_usable() {
            tracing::warn!("ignoring the `{}` firewall adapter: {}.", row.name, why);
            continue;
        }
        if out.iter().any(|a| a.name.eq_ignore_ascii_case(&row.name)) {
            tracing::warn!("ignoring a second firewall adapter named `{}`.", row.name);
            continue;
        }
        out.push(row);
    }
    out
}

/// The adapter for the firewall this machine is running.
pub fn detect<'a>(
    adapters: &'a [FirewallAdapter],
    present: &dyn Fn(&str) -> bool,
) -> Option<&'a FirewallAdapter> {
    adapters
        .iter()
        .find(|a| a.applies_to_this_os() && present(&a.detect))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped(name: &str) -> FirewallAdapter {
        adapters(vec![])
            .into_iter()
            .find(|a| a.name == name)
            .unwrap_or_else(|| panic!("{} must ship", name))
    }

    /// The shipped rows go through the loader, not around it — K17's rule, which exists so the
    /// built-ins exercise the same path a user's row does.
    #[test]
    fn the_shipped_table_parses_and_carries_three_firewalls() {
        let all = adapters(vec![]);
        let names: Vec<&str> = all.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"ufw"), "{:?}", names);
        assert!(names.contains(&"firewalld"), "{:?}", names);
        assert!(names.contains(&"windows-defender"), "{:?}", names);
    }

    /// XI.2: one spelling, several firewalls. The same declaration produces each tool's own
    /// command — which is the argument for a built-in backend over a per-machine definition.
    #[test]
    fn one_declaration_becomes_each_firewalls_own_command() {
        let ufw = shipped("ufw").allow_command(22, Proto::Tcp);
        assert_eq!(ufw, vec!["ufw", "allow", "22/tcp"]);

        let fd = shipped("firewalld").allow_command(22, Proto::Tcp);
        assert_eq!(fd, vec!["firewall-cmd", "--permanent", "--add-port=22/tcp"]);

        let win = shipped("windows-defender").allow_command(22, Proto::Tcp);
        assert!(win.iter().any(|a| a == "localport=22"), "{:?}", win);
        assert!(win.iter().any(|a| a == "protocol=tcp"), "{:?}", win);
    }

    #[test]
    fn closing_a_port_is_its_own_command() {
        assert_eq!(
            shipped("ufw").deny_command(8080, Proto::Udp),
            vec!["ufw", "delete", "allow", "8080/udp"]
        );
    }

    /// N4: the default policy is expressible where the firewall has one...
    #[test]
    fn a_default_policy_becomes_the_firewalls_own_verb() {
        let cmd = shipped("ufw")
            .default_command(Direction::Incoming, "deny")
            .expect("ufw sets a default");
        assert_eq!(cmd, vec!["ufw", "default", "deny", "incoming"]);
    }

    /// ...and where it is not, that is a named `None` rather than a silent skip. firewalld has
    /// no outgoing target, and pretending otherwise would report a policy nobody set.
    #[test]
    fn a_direction_a_firewall_cannot_express_is_not_pretended() {
        assert!(shipped("firewalld")
            .default_command(Direction::Outgoing, "deny")
            .is_none());
        assert!(shipped("firewalld")
            .default_command(Direction::Incoming, "DROP")
            .is_some());
    }

    /// Real `ufw status numbered` output. Headers and chatter must not become phantom rules —
    /// the S22/S23 class, one domain over.
    #[test]
    fn ufw_status_parses_only_the_rules() {
        let out = "Status: active\n\
                   \n\
                        To                         Action      From\n\
                        --                         ------      ----\n\
                   [ 1] 22/tcp                     ALLOW IN    Anywhere\n\
                   [ 2] 8080/udp                   ALLOW IN    Anywhere\n";
        let rules = shipped("ufw").parse_rules(out);
        assert_eq!(rules.len(), 2, "{:?}", rules);
        assert_eq!(rules[0].to_string(), "22/tcp");
        assert_eq!(rules[1].to_string(), "8080/udp");
    }

    #[test]
    fn an_empty_or_noisy_listing_yields_no_rules() {
        assert!(shipped("ufw").parse_rules("Status: inactive\n").is_empty());
        assert!(shipped("ufw").parse_rules("").is_empty());
    }

    #[test]
    fn firewalld_ports_parse() {
        let rules = shipped("firewalld").parse_rules("22/tcp 443/tcp 53/udp\n");
        assert_eq!(rules.len(), 3, "{:?}", rules);
    }

    /// A row that cannot list its rules is refused: drift it cannot see is drift it would
    /// silently never correct, and N7 turns on seeing it.
    #[test]
    fn a_row_that_cannot_list_is_refused() {
        let mut blind = shipped("ufw");
        blind.name = "blind".into();
        blind.list = vec![];
        assert!(!adapters(vec![blind]).iter().any(|a| a.name == "blind"));
    }

    #[test]
    fn detection_picks_the_firewall_that_is_present() {
        let all = adapters(vec![]);
        let only_ufw = |cmd: &str| cmd == "ufw";
        let found = detect(&all, &only_ufw);
        // On a non-Linux host the ufw row does not apply, and that is the right answer.
        if cfg!(target_os = "linux") {
            assert_eq!(found.map(|a| a.name.as_str()), Some("ufw"));
        } else {
            assert!(found.is_none());
        }
        assert!(detect(&all, &|_| false).is_none());
    }
}
