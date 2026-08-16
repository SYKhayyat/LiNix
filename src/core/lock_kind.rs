//! What `lock` and `unlock` can freeze, one name per thing, and how a person selects a set.
//!
//! **There were three axes and there are nine kinds, because the granularity was already there.**
//! `scripts` was one word covering seven separately-approved things — a hook, an event hook, an
//! adapter file, an `exec:`, a `generate:`, a health-check command and a `vars` provider — each
//! of which already has its own identity in `locks/hooks.toml` and its own prefix in that
//! identity. Approving all seven to approve one was not a limitation of the ledger. It was a
//! limitation of the word.
//!
//! **Below the kind there is a sub-category, spelled `kind:qualifier`.** `versions:apt` is apt's
//! pins, `hooks:after_install` is one hook across every package, `events:before_sync` is one
//! event wherever it is declared. Four of the nine kinds have such a division and five do not —
//! an `exec:` script has no category above itself, only its own name — and asking for one where
//! none exists is refused rather than silently matching nothing.
//!
//! **Why the scope is a word and not a flag.** `--backend apt` reads well until you want
//! `--except`, and then it cannot be written at all: an exclusion is a list, and a flag applies
//! to the whole command. "Everything except cargo's pins" is `--except versions:cargo` and has
//! no spelling as a flag. One mechanism that works in an include list, an exclusion and
//! `preferences.toml` identically beats two that each work in one place.

use crate::core::{Error, Result};
use std::fmt;

/// One freezable thing.
///
/// The variants are the vocabulary: these strings appear in `shall lock`, in `shall unlock`, in
/// `--except`, and in `[lock] freeze` / `[lock] except`. One list, so a name that works in one
/// place works in all four — the alternative is a config vocabulary that drifts from the CLI's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LockKind {
    /// Package version pins — `locks/versions.json`.
    Versions,
    /// Which manager each unpinned bare name resolved to — `locks/bare.HOST.toml`.
    Backends,
    /// Lifecycle hooks, keyed `<hook>:<package>`.
    Hooks,
    /// Hooks on Shall's own events, keyed `event:<event>@<origin>`.
    Events,
    /// Files under `adapters/`, keyed `adapters:<filename>`.
    Adapters,
    /// `exec:` scripts, keyed `exec:<script>`.
    Exec,
    /// `generate:` commands, keyed `generate:<command>`.
    Generate,
    /// Declared health-check commands, keyed `health:<command>`.
    Health,
    /// The `vars` provider, keyed `vars:<filename>`.
    Vars,
}

/// Every kind, in the order help text and listings print them: the two that describe packages,
/// then the seven that approve code.
pub const ALL: [LockKind; 9] = [
    LockKind::Versions,
    LockKind::Backends,
    LockKind::Hooks,
    LockKind::Events,
    LockKind::Adapters,
    LockKind::Exec,
    LockKind::Generate,
    LockKind::Health,
    LockKind::Vars,
];

/// The seven kinds that live in the approval ledger and gate something the config can execute.
pub const SCRIPTS: [LockKind; 7] = [
    LockKind::Hooks,
    LockKind::Events,
    LockKind::Adapters,
    LockKind::Exec,
    LockKind::Generate,
    LockKind::Health,
    LockKind::Vars,
];

/// The two kinds that describe packages rather than code.
pub const PACKAGES: [LockKind; 2] = [LockKind::Versions, LockKind::Backends];

impl LockKind {
    /// The one spelling. Parsing and printing read the same table, so a kind cannot be
    /// accepted under a name nothing prints.
    pub fn as_str(self) -> &'static str {
        match self {
            LockKind::Versions => "versions",
            LockKind::Backends => "backends",
            LockKind::Hooks => "hooks",
            LockKind::Events => "events",
            LockKind::Adapters => "adapters",
            LockKind::Exec => "exec",
            LockKind::Generate => "generate",
            LockKind::Health => "health",
            LockKind::Vars => "vars",
        }
    }

    /// What `kind:qualifier` means for this kind, in words a refusal can print, or `None` where
    /// the kind has no division below itself.
    ///
    /// Five kinds return `None` and that is not an oversight: an `exec:` script, an adapter
    /// file, a health command, a generator and the `vars` provider are each a flat set of items.
    /// Their granularity is the item's own name, which is what the positional `NAME…` argument
    /// has always selected.
    pub fn subdivision(self) -> Option<&'static str> {
        match self {
            LockKind::Versions | LockKind::Backends => Some("a package manager"),
            LockKind::Hooks => Some("a hook name, such as `after_install`"),
            LockKind::Events => Some("an event name, or the origin it is declared in"),
            LockKind::Adapters
            | LockKind::Exec
            | LockKind::Generate
            | LockKind::Health
            | LockKind::Vars => None,
        }
    }

    /// The prefix this kind's entries carry in `locks/hooks.toml`, for the six whose identity
    /// begins with a fixed word.
    ///
    /// `Hooks` is `None` and that is the interesting case: a lifecycle hook is keyed
    /// `<hook_name>:<package>` — `after_install:nginx` — so its first segment is the hook's own
    /// name and there is no constant to match. It is recognised by elimination, which is only
    /// safe because this list and [`ALL`] are checked against each other.
    pub fn ledger_prefix(self) -> Option<&'static str> {
        match self {
            LockKind::Events => Some("event"),
            LockKind::Adapters => Some("adapters"),
            LockKind::Exec => Some("exec"),
            LockKind::Generate => Some("generate"),
            LockKind::Health => Some("health"),
            LockKind::Vars => Some("vars"),
            LockKind::Hooks | LockKind::Versions | LockKind::Backends => None,
        }
    }

    /// Which kind an approval-ledger id belongs to.
    ///
    /// Anything whose prefix is not one of the six named ones is a lifecycle hook, because that
    /// is the one identity built from the hook's own name. `every_ledger_prefix_is_claimed`
    /// fails if a seventh prefix is ever added without a kind, which is what keeps the
    /// by-elimination step honest.
    pub fn of_ledger_id(id: &str) -> LockKind {
        let head = id.split_once(':').map_or(id, |(h, _)| h);
        SCRIPTS
            .iter()
            .copied()
            .find(|k| k.ledger_prefix() == Some(head))
            .unwrap_or(LockKind::Hooks)
    }

    /// The sub-category an entry of this kind falls into, given its ledger id (or, for
    /// `Backends`, the manager the name resolved to — the one case where the answer is in the
    /// value rather than the key).
    ///
    /// `None` means "this entry has no sub-category", which a qualified target never matches.
    pub fn sub_of(self, key: &str, resolved_backend: Option<&str>) -> Option<String> {
        match self {
            // `apt:curl` — the manager is the head.
            LockKind::Versions => key.split_once(':').map(|(m, _)| m.to_string()),
            // A bare-name entry is keyed by the name; the manager is what it resolved to.
            LockKind::Backends => resolved_backend.map(str::to_string),
            // `after_install:nginx` — the hook's own name is the head.
            LockKind::Hooks => key.split_once(':').map(|(h, _)| h.to_string()),
            // `event:before_sync@repo` — matched on either half below.
            LockKind::Events => key
                .strip_prefix("event:")
                .map(|rest| rest.split_once('@').map_or(rest, |(e, _)| e).to_string()),
            _ => None,
        }
    }

    /// The second thing an `events:` qualifier may name: where the hook is declared.
    fn event_origin(key: &str) -> Option<String> {
        key.strip_prefix("event:")
            .and_then(|rest| rest.split_once('@'))
            .map(|(_, origin)| origin.to_string())
    }
}

impl fmt::Display for LockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One selected thing: a kind, optionally narrowed to a sub-category, optionally subtracted.
///
/// `negated` is a field rather than a marker inside `qualifier`, because a qualifier holds text
/// the user typed and any sentinel put in there is a string a user can also type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockTarget {
    pub kind: LockKind,
    pub qualifier: Option<String>,
    pub negated: bool,
}

impl LockTarget {
    fn whole(kind: LockKind) -> Self {
        Self {
            kind,
            qualifier: None,
            negated: false,
        }
    }

    /// Whether this target admits an entry of its kind. An unqualified target admits every
    /// entry; a qualified one admits those in its sub-category.
    fn admits(&self, key: &str, resolved_backend: Option<&str>) -> bool {
        let Some(want) = &self.qualifier else {
            return true;
        };
        if self.kind.sub_of(key, resolved_backend).as_deref() == Some(want.as_str()) {
            return true;
        }
        // An `events:` qualifier names the event or the origin. U15 keeps those separately
        // approved precisely because they are separate decisions, so both are addressable.
        self.kind == LockKind::Events && LockKind::event_origin(key).as_deref() == Some(want)
    }
}

impl fmt::Display for LockTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.qualifier {
            Some(q) => write!(f, "{}:{}", self.kind, q),
            None => write!(f, "{}", self.kind),
        }
    }
}

/// One word a person may type: a kind, or a group standing for several.
fn expand(word: &str) -> Option<Vec<LockKind>> {
    match word {
        "everything" => Some(ALL.to_vec()),
        "scripts" => Some(SCRIPTS.to_vec()),
        "packages" => Some(PACKAGES.to_vec()),
        other => ALL
            .iter()
            .copied()
            .find(|k| k.as_str() == other)
            .map(|k| vec![k]),
    }
}

/// The words a refusal offers back. Groups first, because a person who typed a wrong word
/// usually wanted a group.
fn vocabulary() -> String {
    let kinds: Vec<&str> = ALL.iter().map(|k| k.as_str()).collect();
    format!(
        "groups: everything, packages, scripts\n  kinds:  {}",
        kinds.join(", ")
    )
}

/// Parse one comma-separated word into targets, resolving groups and `kind:qualifier`.
fn parse_word(word: &str, flag: &str) -> Result<Vec<LockTarget>> {
    let (head, qualifier) = match word.split_once(':') {
        Some((h, q)) => (h.trim(), Some(q.trim())),
        None => (word, None),
    };

    let Some(kinds) = expand(head) else {
        return Err(Error::Validation(format!(
            "{flag}`{head}` is not something Shall can freeze.\n  {}",
            vocabulary()
        )));
    };

    let Some(want) = qualifier else {
        return Ok(kinds.into_iter().map(LockTarget::whole).collect());
    };
    if want.is_empty() {
        return Err(Error::Validation(format!(
            "{flag}`{head}:` names no sub-category. Write `{head}` for all of it, or \
             `{head}:something` for part."
        )));
    }

    // A qualifier on a group narrows every member that HAS that division and drops the rest:
    // `packages:apt` is apt's pins and apt's resolutions, and says nothing about `exec`.
    let divisible: Vec<LockKind> = kinds
        .iter()
        .copied()
        .filter(|k| k.subdivision().is_some())
        .collect();
    if divisible.is_empty() {
        return Err(Error::Validation(format!(
            "{flag}`{head}` has no sub-category, so `{head}:{want}` selects nothing. Every \
             {head} entry is\n  addressed by its own name instead: `shall lock {head} {want}`."
        )));
    }

    Ok(divisible
        .into_iter()
        .map(|kind| LockTarget {
            kind,
            qualifier: Some(want.to_string()),
            negated: false,
        })
        .collect())
}

/// A set of targets, however it was spelled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockSelection {
    targets: Vec<LockTarget>,
    /// Whether the user named anything explicitly, as opposed to taking the default. Config
    /// narrowing applies to the default and never to an explicit request: a `[lock] freeze` that
    /// could veto `shall lock exec` would make a command decline the one thing it was told.
    explicit: bool,
}

impl LockSelection {
    /// Everything, as a bare `shall lock` means it.
    pub fn everything() -> Self {
        Self {
            targets: ALL.iter().copied().map(LockTarget::whole).collect(),
            explicit: false,
        }
    }

    /// Parse `what` (comma-separated words, or `everything`) minus `except`.
    ///
    /// **An exclusion that selects nothing is an error, not an empty run.** `--except everything`
    /// and `lock exec --except exec` both describe a command with no work in it, and a `lock`
    /// that silently freezes nothing is indistinguishable from one that worked.
    pub fn parse(what: &str, except: &[String]) -> Result<Self> {
        let explicit = what != "everything";
        let mut targets: Vec<LockTarget> = Vec::new();
        for word in what.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            for target in parse_word(word, "")? {
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
        if targets.is_empty() {
            return Err(Error::Validation(format!(
                "nothing was named to freeze.\n  {}",
                vocabulary()
            )));
        }

        for word in except {
            for drop in parse_word(word.trim(), "--except ")? {
                if drop.qualifier.is_none() {
                    // `--except exec` removes the kind outright.
                    targets.retain(|t| t.kind != drop.kind);
                } else if targets.iter().any(|t| t.kind == drop.kind) {
                    // `--except versions:cargo` narrows a kind that is still wanted, so it is
                    // recorded as a subtraction the matcher consults rather than a removal.
                    targets.push(LockTarget {
                        negated: true,
                        ..drop
                    });
                }
            }
        }
        if targets.iter().all(|t| t.negated) {
            return Err(Error::Validation(
                "every kind was excluded, so this command would freeze nothing. Drop an \
                 `--except`, or name what you do want."
                    .to_string(),
            ));
        }

        targets.sort();
        Ok(Self { targets, explicit })
    }

    /// Narrow by a machine's standing preference, which applies only when the user named nothing.
    pub fn narrowed_by_config(mut self, allowed: &[LockKind]) -> Self {
        if !self.explicit {
            self.targets.retain(|t| allowed.contains(&t.kind));
        }
        self
    }

    /// Whether any work of this kind is selected at all — the question a caller asks before
    /// running that kind's approver.
    pub fn includes(&self, kind: LockKind) -> bool {
        self.targets.iter().any(|t| t.kind == kind && !t.negated)
    }

    /// Whether this specific entry is selected. `resolved_backend` is only consulted for
    /// `Backends`, whose sub-category lives in the value rather than the key.
    ///
    /// **An exclusion beats an inclusion**, so `everything --except versions:cargo` drops
    /// cargo's pins while keeping every other manager's. The other order would make the
    /// exclusion unwritable, since `everything` admits the entry first.
    pub fn admits(&self, kind: LockKind, key: &str, resolved_backend: Option<&str>) -> bool {
        let mine = || self.targets.iter().filter(|t| t.kind == kind);
        if mine()
            .filter(|t| t.negated)
            .any(|t| t.admits(key, resolved_backend))
        {
            return false;
        }
        mine()
            .filter(|t| !t.negated)
            .any(|t| t.admits(key, resolved_backend))
    }

    /// Whether **every** entry of this kind is selected: chosen with no qualifier narrowing it,
    /// and nothing subtracted from it.
    ///
    /// The question a caller asks before taking the rebuild-the-whole-file shortcut instead of
    /// merging entry by entry. `everything --except versions:cargo` still *includes* versions
    /// and must not take that shortcut, which is the case a plain "is this kind selected" check
    /// gets wrong.
    pub fn takes_all_of(&self, kind: LockKind) -> bool {
        let mut whole = false;
        for target in self.targets.iter().filter(|t| t.kind == kind) {
            if target.negated {
                return false;
            }
            whole |= target.qualifier.is_none();
        }
        whole
    }

    /// Whether every one of `set` is selected whole — the question the group-level reports ask
    /// before printing a summary that speaks for all of them.
    pub fn includes_all_whole(&self, set: &[LockKind]) -> bool {
        set.iter().all(|k| {
            self.targets
                .iter()
                .any(|t| t.kind == *k && t.qualifier.is_none() && !t.negated)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.targets.iter().all(|t| t.negated)
    }

    pub fn targets(&self) -> &[LockTarget] {
        &self.targets
    }

    /// Every manager named as a qualifier **on `backends:`**, for validating them against the
    /// registry before a typo scopes the run to nothing (`Q9`, which `upgrade --backend aptt`
    /// already cost once).
    ///
    /// **`versions:` is deliberately not asked**, and the reason is the same one that keeps the
    /// name check off that kind: `locks/versions.json` travels between machines, so
    /// `versions:apt` is an ordinary scope on a host with no apt — a repo shared with a Debian
    /// box records apt pins, and a Windows machine has every right to release them. The bare
    /// lock is the opposite: it records what *this* host resolved, so a manager it does not run
    /// is a typo and nothing else. A typo on `versions:` still gets an answer, from the kind's
    /// own "nothing matched" warning, which quotes the scope back.
    pub fn managers_named(&self) -> Vec<String> {
        self.targets
            .iter()
            .filter(|t| t.kind == LockKind::Backends)
            .filter_map(|t| t.qualifier.clone())
            .collect()
    }
}

impl fmt::Display for LockSelection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<String> = self
            .targets
            .iter()
            .filter(|t| !t.negated)
            .map(|t| t.to_string())
            .collect();
        f.write_str(&names.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(what: &str) -> LockSelection {
        LockSelection::parse(what, &[]).expect("should parse")
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn kinds_of(s: &LockSelection) -> Vec<LockKind> {
        let mut k: Vec<LockKind> = s
            .targets()
            .iter()
            .filter(|t| !t.negated)
            .map(|t| t.kind)
            .collect();
        k.sort();
        k.dedup();
        k
    }

    /// The groups are exactly their parts, and the two of them are exactly everything. Asserted
    /// rather than assumed, because a kind added to `ALL` and to neither group would be
    /// unreachable by any group name and nobody would notice.
    #[test]
    fn the_groups_partition_every_kind() {
        let mut from_groups: Vec<LockKind> =
            PACKAGES.iter().chain(SCRIPTS.iter()).copied().collect();
        from_groups.sort();
        let mut all = ALL.to_vec();
        all.sort();
        assert_eq!(from_groups, all, "a kind belongs to no group, or to both");
        assert_eq!(ALL.len(), PACKAGES.len() + SCRIPTS.len());
    }

    /// Every kind's name round-trips, and no two share one. A duplicate would make `expand`
    /// return whichever came first in the array and silently shadow the other.
    #[test]
    fn every_kind_has_one_name_and_no_two_share_it() {
        for kind in ALL {
            assert_eq!(kinds_of(&sel(kind.as_str())), vec![kind], "{kind}");
        }
        let mut names: Vec<&str> = ALL.iter().map(|k| k.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "two kinds share a name");
    }

    /// The three ways of writing a set, and the same set out of two of them.
    #[test]
    fn a_set_can_be_listed_or_named_as_a_group() {
        assert_eq!(kinds_of(&sel("everything")), ALL.to_vec());
        assert_eq!(kinds_of(&sel("packages")), PACKAGES.to_vec());
        assert!(sel("exec,hooks").includes(LockKind::Exec));
        assert!(sel("exec,hooks").includes(LockKind::Hooks));
        assert!(!sel("exec,hooks").includes(LockKind::Versions));
        assert_eq!(
            kinds_of(&sel("versions,backends")),
            kinds_of(&sel("packages"))
        );
    }

    /// Whitespace around a comma is a person typing, not an error. And a repeated word is not
    /// a duplicate entry — the set is a set.
    #[test]
    fn spacing_and_repetition_do_not_change_the_set() {
        assert_eq!(
            kinds_of(&sel(" exec , hooks ")),
            kinds_of(&sel("hooks,exec"))
        );
        assert_eq!(kinds_of(&sel("exec,exec")), vec![LockKind::Exec]);
        assert_eq!(kinds_of(&sel("packages,versions")), PACKAGES.to_vec());
    }

    /// **`kind:qualifier` selects part of a kind.** One manager's pins, one hook across every
    /// package, one event wherever it is declared — and in each case the neighbours are not
    /// selected, which is the half that proves the narrowing happened.
    #[test]
    fn a_qualifier_selects_one_sub_category_and_not_its_neighbours() {
        let apt = sel("versions:apt");
        assert!(apt.admits(LockKind::Versions, "apt:curl", None));
        assert!(!apt.admits(LockKind::Versions, "cargo:ripgrep", None));

        let after = sel("hooks:after_install");
        assert!(after.admits(LockKind::Hooks, "after_install:nginx", None));
        assert!(!after.admits(LockKind::Hooks, "before_install:nginx", None));

        // A backends entry is keyed by the bare name, so its manager comes from the value.
        let cargo = sel("backends:cargo");
        assert!(cargo.admits(LockKind::Backends, "ripgrep", Some("cargo")));
        assert!(!cargo.admits(LockKind::Backends, "curl", Some("apt")));
    }

    /// An `events:` qualifier names the event **or** the origin, because U15 approves those
    /// separately and both are therefore real things to address.
    #[test]
    fn an_event_qualifier_matches_the_event_or_where_it_is_declared() {
        let by_event = sel("events:before_sync");
        assert!(by_event.admits(LockKind::Events, "event:before_sync@repo", None));
        assert!(!by_event.admits(LockKind::Events, "event:after_sync@repo", None));

        let by_origin = sel("events:preferences");
        assert!(by_origin.admits(LockKind::Events, "event:before_sync@preferences", None));
        assert!(!by_origin.admits(LockKind::Events, "event:before_sync@repo", None));
    }

    /// An unqualified kind admits every entry in it — the control without which every
    /// assertion above passes against a matcher that admits nothing.
    #[test]
    fn an_unqualified_kind_admits_all_of_its_entries() {
        let all_versions = sel("versions");
        for key in ["apt:curl", "cargo:ripgrep", "brew:jq"] {
            assert!(all_versions.admits(LockKind::Versions, key, None), "{key}");
        }
        let all_hooks = sel("hooks");
        for key in ["after_install:nginx", "before_install:*"] {
            assert!(all_hooks.admits(LockKind::Hooks, key, None), "{key}");
        }
    }

    /// A qualifier on a group narrows the members that divide and drops the ones that do not:
    /// `packages:apt` is apt's pins and apt's resolutions, and claims nothing about `exec`.
    #[test]
    fn a_qualifier_on_a_group_narrows_only_the_members_that_divide() {
        let apt = sel("packages:apt");
        assert_eq!(kinds_of(&apt), PACKAGES.to_vec());
        assert!(apt.admits(LockKind::Versions, "apt:curl", None));
        assert!(!apt.admits(LockKind::Versions, "cargo:ripgrep", None));

        // `everything:apt` is the same rule over the wider group: every kind that divides is
        // narrowed to `apt`, and the five flat kinds drop out. It reaches `hooks` and `events`
        // too — a hook literally named `apt` — which is odd to read and is the price of one
        // rule applied uniformly. The alternative, a group that quietly means something
        // narrower than its own members, is worse: `packages:apt` and `everything:apt` would
        // then subset differently for no reason a user could state.
        let wide = sel("everything:apt");
        assert_eq!(
            kinds_of(&wide),
            vec![
                LockKind::Versions,
                LockKind::Backends,
                LockKind::Hooks,
                LockKind::Events
            ]
        );
        assert!(
            !wide.includes(LockKind::Exec),
            "a flat kind is not narrowed, it is dropped"
        );
    }

    /// **A kind with no sub-category refuses a qualifier rather than matching nothing**, and
    /// the refusal points at the thing that does work: naming the item.
    #[test]
    fn a_flat_kind_refuses_a_qualifier_and_says_what_to_type_instead() {
        for kind in ALL.iter().filter(|k| k.subdivision().is_none()) {
            let word = format!("{}:something", kind.as_str());
            let err = LockSelection::parse(&word, &[]).expect_err(&word);
            let msg = err.to_string();
            assert!(msg.contains(kind.as_str()), "{msg}");
            assert!(
                msg.contains("its own name"),
                "the refusal must point at what does work: {msg}"
            );
        }
        // And the four that do divide accept one, which is the other half of the same claim.
        for kind in ALL.iter().filter(|k| k.subdivision().is_some()) {
            let word = format!("{}:something", kind.as_str());
            assert!(
                LockSelection::parse(&word, &[]).is_ok(),
                "{kind} divides and must accept a qualifier"
            );
        }
    }

    /// A trailing colon names no sub-category, which is a typo rather than a set.
    #[test]
    fn a_qualifier_that_is_empty_is_refused() {
        let err = LockSelection::parse("versions:", &[]).expect_err("empty qualifier");
        assert!(err.to_string().contains("no sub-category"), "{err}");
    }

    /// `everything` minus a kind, and minus a whole group.
    #[test]
    fn an_exclusion_subtracts_from_whatever_was_named() {
        let no_exec = LockSelection::parse("everything", &words(&["exec"])).unwrap();
        assert!(!no_exec.includes(LockKind::Exec));
        assert_eq!(kinds_of(&no_exec).len(), ALL.len() - 1);

        let no_scripts = LockSelection::parse("everything", &words(&["scripts"])).unwrap();
        assert_eq!(kinds_of(&no_scripts), PACKAGES.to_vec());

        let listed = LockSelection::parse("packages", &words(&["backends"])).unwrap();
        assert_eq!(kinds_of(&listed), vec![LockKind::Versions]);
    }

    /// **The exclusion that has no spelling as a flag, and the reason the scope is a word.**
    /// Everything except one manager's pins: the kind is still wanted, one sub-category is not.
    #[test]
    fn an_exclusion_can_name_a_sub_category_and_keep_the_rest_of_the_kind() {
        let s = LockSelection::parse("everything", &words(&["versions:cargo"])).unwrap();
        assert!(s.includes(LockKind::Versions), "the kind is still wanted");
        assert!(s.admits(LockKind::Versions, "apt:curl", None));
        assert!(
            !s.admits(LockKind::Versions, "cargo:ripgrep", None),
            "cargo's pins were excluded and are still being taken"
        );
        // Untouched kinds are untouched.
        assert!(s.admits(LockKind::Exec, "exec:./setup.sh", None));
        assert!(s.admits(LockKind::Hooks, "after_install:nginx", None));
    }

    /// **A selection that comes out empty is refused.** Every route to it, because the empty
    /// set is the one outcome indistinguishable from success.
    #[test]
    fn a_selection_that_comes_out_empty_is_an_error_rather_than_a_quiet_no_op() {
        for (what, except) in [
            ("everything", words(&["everything"])),
            ("exec", words(&["exec"])),
            ("packages", words(&["versions", "backends"])),
            ("packages", words(&["packages"])),
            ("", vec![]),
        ] {
            assert!(
                LockSelection::parse(what, &except).is_err(),
                "an empty selection must be refused: {what:?} minus {except:?}"
            );
        }
    }

    /// An unknown word is named back, and the message carries the whole vocabulary — a refusal
    /// that says only "unknown" makes the reader go and find the list.
    #[test]
    fn an_unknown_word_is_refused_with_the_list_of_real_ones() {
        for bad in ["firewall", "version", "script", "packagess"] {
            let err = LockSelection::parse(bad, &[]).expect_err(bad);
            let msg = err.to_string();
            assert!(msg.contains(bad), "it must quote the word back: {msg}");
            assert!(msg.contains("everything"), "the groups are missing: {msg}");
            assert!(msg.contains("adapters"), "the kinds are missing: {msg}");
        }
        // And on the exclusion side, which is a separate parse and was the half that got
        // forgotten when this was two code paths.
        let err = LockSelection::parse("everything", &words(&["firewal"])).expect_err("typo");
        assert!(err.to_string().contains("firewal"), "{err}");
        assert!(err.to_string().contains("--except"), "{err}");
    }

    /// Every ledger id maps to a kind, and the by-elimination case maps only what it should.
    #[test]
    fn every_ledger_prefix_is_claimed_by_exactly_one_kind() {
        for (id, expect) in [
            ("exec:./setup.sh", LockKind::Exec),
            ("generate:./pick.sh", LockKind::Generate),
            ("event:before_sync@repo", LockKind::Events),
            ("adapters:backends.toml", LockKind::Adapters),
            ("health:systemctl is-active nginx", LockKind::Health),
            ("vars:vars.shall", LockKind::Vars),
            ("after_install:nginx", LockKind::Hooks),
            ("before_install:redis", LockKind::Hooks),
            ("after_install:*", LockKind::Hooks),
        ] {
            assert_eq!(LockKind::of_ledger_id(id), expect, "{id}");
        }

        let mut prefixes: Vec<&str> = SCRIPTS.iter().filter_map(|k| k.ledger_prefix()).collect();
        prefixes.sort_unstable();
        let before = prefixes.len();
        prefixes.dedup();
        assert_eq!(before, prefixes.len(), "two kinds share a ledger prefix");
        assert_eq!(before, SCRIPTS.len() - 1, "only `hooks` may lack a prefix");
    }

    /// A standing preference narrows the default and never an explicit request. Both halves:
    /// the point of the second is that `shall lock exec` must freeze `exec` on a machine whose
    /// preferences leave it out — a command that declines what it was told to do is worse than
    /// no preference at all.
    #[test]
    fn config_narrows_the_default_but_never_a_named_kind() {
        let allowed = [LockKind::Exec, LockKind::Hooks];

        let defaulted = LockSelection::everything().narrowed_by_config(&allowed);
        assert_eq!(kinds_of(&defaulted), vec![LockKind::Hooks, LockKind::Exec]);
        assert!(!defaulted.includes(LockKind::Versions));

        let asked = sel("versions").narrowed_by_config(&allowed);
        assert!(
            asked.includes(LockKind::Versions),
            "naming a kind is asking for it, whatever preferences.toml prefers"
        );

        let spelled = sel("everything").narrowed_by_config(&allowed);
        assert!(!spelled.includes(LockKind::Versions));
    }

    /// The managers a run named on the bare lock, so a typo can be refused before it scopes the
    /// run to nothing (`Q9`). Both the include and the exclude side, since either can carry one.
    #[test]
    fn every_manager_named_on_the_bare_lock_is_reported_for_checking() {
        let s = LockSelection::parse("packages:apt", &words(&["backends:cargo"])).unwrap();
        let mut named = s.managers_named();
        named.sort();
        named.dedup();
        assert_eq!(named, vec!["apt".to_string(), "cargo".to_string()]);

        // An exclusion naming a kind that was never selected is a no-op, so its manager is not
        // reported either — there is nothing for the registry check to be checking on behalf of.
        let unrelated = LockSelection::parse("backends:apt", &words(&["versions:cargo"])).unwrap();
        assert_eq!(unrelated.managers_named(), vec!["apt".to_string()]);

        // **A version pin's manager is not this host's business.** `locks/versions.json` travels
        // with the config repo, so `versions:apt` is an ordinary scope on a machine that has
        // never run apt, and handing it to the registry check would refuse it there.
        assert!(sel("versions:apt").managers_named().is_empty());

        // A hook name is not a manager and must not be handed to the registry check.
        let hooks = sel("hooks:after_install");
        assert!(hooks.managers_named().is_empty());
    }
}
