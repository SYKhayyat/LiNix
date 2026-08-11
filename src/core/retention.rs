// Declarative retention for the one history Shall stores itself: filesystem snapshots.
// Archived manifests and generations were the other two, and both are git's job now — this
// file opened by naming all three for a phase after they went, which is how a reader ends up
// looking for the other two policies.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How long to retain entries of one history. The two *rules* combine as a UNION: an entry
/// survives if it matches either. `keep` is not a rule but a veto — it can save an entry, never
/// condemn one. Both rules at zero is an inactive policy, which keeps everything (the safe
/// default — retention never deletes unless asked to).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// Keep the N most-recent entries (0 = rule disabled).
    #[serde(default)]
    pub keep_last: usize,
    /// Keep entries younger than this many days (0 = rule disabled).
    #[serde(default)]
    pub keep_days: u64,
    /// Always keep entries whose id OR label matches one of these (declarative pins).
    #[serde(default)]
    pub keep: Vec<String>,
}

impl RetentionPolicy {
    /// Whether this policy deletes anything at all. Both rules off is how a user says "keep
    /// everything" — there is no second on/off switch beside it (II.13: one engine).
    ///
    /// **`keep` is not one of the rules, and that is the whole point.** There were two of these
    /// predicates: this one, which the caller in `verbs::mod` used to decide whether to prune at
    /// all, and an `is_active` that counted a non-empty `keep` as a rule. Under the second, a
    /// policy reading only `keep = ["known-good"]` — a pin list, and nothing else — turned
    /// deletion *on*, matched nothing against a count rule of zero or an age rule of zero, and
    /// so deleted every snapshot but the pinned one and the newest. Pinning a snapshot is how a
    /// user says *keep this*; it can never be how they say *delete the others*.
    pub fn prunes(&self) -> bool {
        self.keep_last > 0 || self.keep_days > 0
    }
}

/// `deny_unknown_fields` for the same reason `Config` carries it: a table nobody reads is a
/// setting that documents itself as working. `[retention.generations]` outlived the
/// generation format by a whole phase, in the shipped example, silently ignored.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    // `generations` is gone: the generation format was deleted (git is the manifest history
    // now), so there is no generation history to retain — only filesystem snapshots.
    #[serde(default = "default_snapshot_retention")]
    pub snapshots: RetentionPolicy,
}

/// Filesystem snapshots can be large and are taken on every sync, so — unlike generations and
/// manifests, which default to keep-everything — they carry an active default: the 10 most
/// recent, and anything from the last 30 days. This preserves the behaviour the deleted legacy
/// `[snapshots]` keys used to provide, now expressed in the one retention dialect.
fn default_snapshot_retention() -> RetentionPolicy {
    RetentionPolicy {
        keep_last: 10,
        keep_days: 30,
        keep: Vec::new(),
    }
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            snapshots: default_snapshot_retention(),
        }
    }
}

/// A minimal view of one stored entry, enough for the policy to judge it.
#[derive(Debug, Clone)]
pub struct RetentionItem {
    pub id: String,
    /// Optional human label (e.g. "known-good-2026-07"); empty when unlabeled.
    pub label: String,
    pub timestamp: DateTime<Utc>,
    /// Imperatively pinned by whatever stores the entry; always survives.
    pub pinned: bool,
}

impl RetentionItem {
    pub fn new(id: impl Into<String>, timestamp: DateTime<Utc>) -> Self {
        Self {
            id: id.into(),
            label: String::new(),
            timestamp,
            pinned: false,
        }
    }

    /// The entry's human name, which `keep` may match as well as the id.
    pub fn labelled(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
}

impl RetentionPolicy {
    /// Return the ids that should be DELETED, given all current `items` and the current
    /// time. Guarantees:
    /// - An inactive policy returns nothing.
    /// - The single most-recent entry is NEVER deleted (a hard floor, so an age rule can
    ///   never wipe out the last rollback point on a long-idle machine).
    /// - Pinned entries and entries matched by `keep` (id or label) are never deleted.
    /// - Otherwise an entry survives iff it is within `keep_last` OR younger than
    ///   `keep_days` (union). Entries matching no rule are deleted.
    pub fn select_deletions(&self, items: &[RetentionItem], now: DateTime<Utc>) -> Vec<String> {
        if !self.prunes() || items.is_empty() {
            return Vec::new();
        }

        // Newest first, so index 0 is the most recent (the protected floor) and the first
        // `keep_last` indices are the survivors of the count rule.
        let mut sorted: Vec<&RetentionItem> = items.iter().collect();
        sorted.sort_by_key(|item| std::cmp::Reverse(item.timestamp));

        let kept: HashSet<&str> = self.keep.iter().map(String::as_str).collect();
        let age_cutoff = (self.keep_days > 0).then(|| now - Duration::days(self.keep_days as i64));

        let mut deletions = Vec::new();
        for (idx, item) in sorted.iter().enumerate() {
            // Hard floor: always keep the most-recent entry.
            if idx == 0 {
                continue;
            }
            // Pins (imperative flag or declarative id/label match) always survive.
            if item.pinned
                || kept.contains(item.id.as_str())
                || (!item.label.is_empty() && kept.contains(item.label.as_str()))
            {
                continue;
            }
            // Union of the count and age rules.
            let within_count = self.keep_last > 0 && idx < self.keep_last;
            let within_age = age_cutoff.is_some_and(|cutoff| item.timestamp >= cutoff);
            if within_count || within_age {
                continue;
            }
            deletions.push(item.id.clone());
        }
        deletions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(days_ago: i64) -> DateTime<Utc> {
        // A fixed reference minus N days, so tests are deterministic.
        DateTime::parse_from_rfc3339("2026-07-03T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            - Duration::days(days_ago)
    }

    fn now() -> DateTime<Utc> {
        at(0)
    }

    fn item(id: &str, days_ago: i64) -> RetentionItem {
        RetentionItem::new(id, at(days_ago))
    }

    #[test]
    fn a_pin_list_on_its_own_is_not_permission_to_delete() {
        // Two predicates disagreed about this policy: `prunes()` said it deletes nothing, and
        // an `is_active()` beside it counted a non-empty `keep` as a rule — under which every
        // snapshot except the pin and the newest was reaped, on the strength of a line whose
        // whole meaning is *keep*.
        let p = RetentionPolicy {
            keep: vec!["known-good".into()],
            ..Default::default()
        };
        let items = vec![
            item("newest", 1),
            item("known-good", 300),
            item("ordinary", 400),
        ];
        assert!(!p.prunes());
        assert!(
            p.select_deletions(&items, now()).is_empty(),
            "a policy that only pins deleted something"
        );
    }

    #[test]
    fn a_pin_beside_a_rule_still_only_saves() {
        // The other half: once a real rule is on, `keep` is a veto within it and nothing more.
        let p = RetentionPolicy {
            keep_last: 1,
            keep: vec!["known-good".into()],
            ..Default::default()
        };
        let items = vec![
            item("newest", 1),
            item("known-good", 300),
            item("ordinary", 400),
        ];
        assert_eq!(
            p.select_deletions(&items, now()),
            vec!["ordinary".to_string()]
        );
    }

    #[test]
    fn a_label_is_matched_the_same_way_an_id_is() {
        let p = RetentionPolicy {
            keep_last: 1,
            keep: vec!["golden".into()],
            ..Default::default()
        };
        let items = vec![
            item("newest", 1),
            item("some-id", 300).labelled("golden"),
            item("ordinary", 400),
        ];
        assert_eq!(
            p.select_deletions(&items, now()),
            vec!["ordinary".to_string()]
        );
    }

    #[test]
    fn inactive_policy_deletes_nothing() {
        let p = RetentionPolicy::default();
        let items = vec![item("a", 100), item("b", 200), item("c", 300)];
        assert!(p.select_deletions(&items, now()).is_empty());
    }

    #[test]
    fn keep_last_keeps_newest_n_and_deletes_the_rest() {
        let p = RetentionPolicy {
            keep_last: 2,
            ..Default::default()
        };
        // g5 newest ... g1 oldest.
        let items = vec![
            item("g1", 5),
            item("g5", 1),
            item("g3", 3),
            item("g2", 4),
            item("g4", 2),
        ];
        let mut del = p.select_deletions(&items, now());
        del.sort();
        assert_eq!(del, vec!["g1", "g2", "g3"]);
    }

    #[test]
    fn keep_days_keeps_recent_and_the_floor() {
        let p = RetentionPolicy {
            keep_days: 30,
            ..Default::default()
        };
        let items = vec![item("recent", 10), item("old", 90), item("ancient", 400)];
        let del = p.select_deletions(&items, now());
        // 'recent' is within 30d; 'old'/'ancient' are older — but the single most-recent
        // ('recent') is the floor anyway. Both stale ones are deleted.
        assert_eq!(del, vec!["old".to_string(), "ancient".to_string()]);
    }

    #[test]
    fn age_rule_never_wipes_the_last_entry() {
        // Everything is older than the window, but the newest must still survive.
        let p = RetentionPolicy {
            keep_days: 7,
            ..Default::default()
        };
        let items = vec![item("a", 100), item("b", 200)];
        assert_eq!(p.select_deletions(&items, now()), vec!["b".to_string()]);
    }

    #[test]
    fn union_of_count_and_age() {
        // keep_last=1 alone would drop everything but the newest; keep_days=30 rescues the
        // one that is old-by-count but young-by-age.
        let p = RetentionPolicy {
            keep_last: 1,
            keep_days: 30,
            ..Default::default()
        };
        let items = vec![item("newest", 1), item("young", 10), item("stale", 90)];
        assert_eq!(p.select_deletions(&items, now()), vec!["stale".to_string()]);
    }

    #[test]
    fn pins_and_labels_always_survive() {
        let p = RetentionPolicy {
            keep_last: 1,
            keep: vec!["keep-me-id".into(), "golden".into()],
            ..Default::default()
        };
        let mut pinned = item("pinned-id", 500);
        pinned.pinned = true;
        let mut labeled = item("some-id", 600);
        labeled.label = "golden".into();
        let items = vec![
            item("newest", 1),
            item("keep-me-id", 700),
            pinned,
            labeled,
            item("doomed", 800),
        ];
        let del = p.select_deletions(&items, now());
        assert_eq!(del, vec!["doomed".to_string()]);
    }
}
