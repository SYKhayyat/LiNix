//! GRADER round 4, 2026-07-30 — RED. `shall protected <bare name>` calls every unqualified name
//! protected, and gives the wrong reason for the names that really are.
//!
//! The command's own help documents the form: *"Check specific packages instead of listing the
//! rules (`apt:python3` or `jq`)"*. Measured on Windows, default config, 7 rules in force:
//!
//!     $ shall protected jq
//!     PACKAGE      PROTECTED  REASON
//!     jq           yes        its manager reports a name no package line can hold, …
//!
//!     $ shall protected sudo            # `sudo` IS in protected_packages
//!     sudo         yes        its manager reports a name no package line can hold, …
//!
//!     $ shall protected cargo:ripgrep
//!     cargo:ripgrep  no       no rule matches
//!
//! Every bare name — `jq`, `ripgrep`, `python3`, `sudo`, the help's own example — comes back
//! `yes`, with the one reason that is never true of a name a user typed. `--json` says
//! `"protected": true` for all of them.
//!
//! Why: `src/verbs/cleanup.rs` defaults the backend to `""` for an unqualified name, and
//! `protection_of` opens with `is_declarable(backend, name)`, which builds the line `":jq"` and
//! rejects it. So the declarability test fires before any rule is consulted, and the answer to
//! *which rule decides this?* — the only question the command exists to answer — is a sentence
//! about package lines. The comment three lines above says the intent plainly: *"a bare name is
//! checked against the config rules only."* It is checked against none of them.
//!
//! And the guard says what this costs, in `src/app/sync/guard.rs`: *"Everything that asks 'is
//! this protected?' must route through here — the `protected` command included. When the inspector
//! and the enforcer answer separately they drift apart, and an inspector that contradicts the
//! guard is worse than none, because it is believed."* The enforcer, asked about the same package
//! with the backend it really has, answers `no rule matches`.
//!
//! **Family.** Every caller of `protection_of` was read. `app/leases.rs`, `verbs/sync.rs`
//! (rebuild) and `core/transaction.rs` take the backend from the state registry or from a
//! resolved plan, so none can be empty; the extras path uses `RemovalKind::Extra`, which skips the
//! declarability test on purpose. The empty-backend call is unique to this inspector.
//!
//! **Why no gate caught it.** `tests/unknown_backend_family_tests.rs` derives the verbs that take a
//! package spec from `--help` — good — and then exempts this one by hand:
//! `("protected", "nothing — it lists the protected set")`. It takes `[PACKAGES]...`, and answers
//! about them. E29 was a stale exemption naming a verb that no longer existed; this is the same
//! failure with a live verb and a false reason.

use crate::harness::Fixture;

impl Fixture {
    /// The row for one queried name: `(protected, reason)`.
    fn row(&self, name: &str) -> (bool, String) {
        let (out, code) = self.run(&["protected", name]);
        assert_eq!(code, 0, "`protected {name}` exited {code}:\n{out}");
        let line = out
            .lines()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("no row for `{name}` in:\n{out}"));
        let rest = line[name.len()..].trim_start();
        let yes = rest.starts_with("yes");
        let reason = rest
            .trim_start_matches("yes")
            .trim_start_matches("no")
            .trim()
            .to_string();
        (yes, reason)
    }
}

/// The control first, so a green run below cannot mean the command stopped answering: a qualified
/// name that matches no rule is reported unprotected.
#[test]
fn a_qualified_name_with_no_rule_is_not_protected() {
    let f = Fixture::new("grade3-protected-control");
    let (yes, reason) = f.row("cargo:shall-probe-zzz");
    assert!(
        !yes,
        "the control failed: a qualified name matching no rule was called protected — {reason}"
    );
}

#[test]
fn a_bare_name_matching_no_rule_is_not_protected() {
    let f = Fixture::new("grade3-protected-bare");
    for name in ["jq", "ripgrep", "shall-probe-zzz"] {
        let (yes, reason) = f.row(name);
        assert!(
            !yes,
            "`shall protected {name}` says the guard protects it, because \
             `protection_of` was handed an empty backend and `is_declarable(\"\", \"{name}\")` is \
             false. Reason given: {reason}\n\nThe enforcer, asked about the same package with the \
             backend it really has, says no rule matches. An inspector that contradicts the guard \
             is worse than none, because it is believed."
        );
    }
}

/// The half that matters most: for a name that *is* protected, the command has to name the rule.
/// Saying `yes` for the wrong reason is not a smaller defect — the reason is the answer.
#[test]
fn a_bare_name_that_is_protected_names_the_rule_that_protects_it() {
    let f = Fixture::new("grade3-protected-rule");
    let listed = f.run(&["protected"]).0;
    assert!(
        listed.contains("sudo"),
        "the fixture's default rules do not include `sudo`, so this test would prove nothing:\n\
         {listed}"
    );

    let (yes, reason) = f.row("sudo");
    assert!(
        yes,
        "`sudo` is in protected_packages and was reported unprotected"
    );
    assert!(
        reason.contains("config rule"),
        "`shall protected sudo` explains a `protected_packages` match as: {reason}\n\nThe rule is \
         in force — `shall protected` lists it — and the command whose job is *which rule decides \
         this* names none."
    );
}

/// The JSON surface carries the same wrong boolean, and it is the one a script reads.
#[test]
fn the_json_answer_matches_the_rules() {
    let f = Fixture::new("grade3-protected-json");
    let (out, code) = f.run(&["protected", "jq", "--json"]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("\"protected\": false"),
        "`protected jq --json` reports `\"protected\": true` for a name no rule mentions:\n{out}"
    );
}

/// Every removal rule the guard holds is in the listing, both surfaces.
///
/// **Enumerated from `GuardSettings` rather than typed out here**, so a rule added tomorrow is
/// covered by this test on the day it is added rather than on the day someone remembers. That
/// distinction is not theoretical: `purge_ratio` stopped being a private constant and became a
/// setting that refuses removals, and neither the human listing nor `--json` mentioned it — while
/// the JSON branch carried a comment about that precise omission having happened once already
/// ("a consumer asking *what will this machine refuse* got a third of the answer and no way to
/// tell it was a third"). A gate naming each rule by hand would have needed the same person to
/// remember the same thing twice.
///
/// The exempted seven are `[guard]`'s install/change rules, which are `shall policy`'s subject —
/// this command's own first line is *"Removal guard — what Shall refuses to remove"*. They are
/// listed with that reason rather than filtered by a pattern, because a pattern would silently
/// swallow a removal rule that happened to be named like a policy one.
#[test]
fn the_listing_shows_every_removal_rule_the_guard_holds() {
    /// Shown by `shall policy`, not here: these refuse an install or a change, not a removal.
    const SHOWN_BY_POLICY: [&str; 7] = [
        "deny_packages",
        "pinned_only",
        "require_snapshot",
        "deny_vulnerable",
        "confine_bin",
        "require_signed_history",
        "never_unattended",
    ];

    let guard = shall::config::Config::default().guard;
    let all = serde_json::to_value(&guard).expect("GuardSettings serialises");
    let fields: Vec<String> = all
        .as_object()
        .expect("GuardSettings is a struct")
        .keys()
        .cloned()
        .collect();

    // The self-test. A scan yielding nothing would make every assertion below vacuous, which is
    // the failure mode this whole file was opened to record.
    assert!(
        fields.len() > 10,
        "read {} fields off GuardSettings; the scan is broken, not the code",
        fields.len()
    );
    for exempt in SHOWN_BY_POLICY {
        assert!(
            fields.contains(&exempt.to_string()),
            "the exemption list names `{exempt}`, which GuardSettings no longer has — a stale \
             exemption is how E29 shipped"
        );
    }

    let f = Fixture::new("grade3-protected-every-rule");
    let (json, code) = f.run(&["protected", "--json"]);
    assert_eq!(code, 0, "`protected --json` exited {code}:\n{json}");
    let (human, code) = f.run(&["protected"]);
    assert_eq!(code, 0, "`protected` exited {code}:\n{human}");

    let mut missing = Vec::new();
    for field in &fields {
        if SHOWN_BY_POLICY.contains(&field.as_str()) {
            continue;
        }
        if !json.contains(&format!("\"{field}\"")) {
            missing.push(format!("`{field}` is not in `shall protected --json`"));
        }
        // The human listing names its two package lists in prose rather than as keys, and says
        // so in the closing paragraph; every other rule appears under its own name.
        let named_in_prose = field.ends_with("_packages");
        if !named_in_prose && !human.contains(field.as_str()) {
            missing.push(format!("`{field}` is not in `shall protected`'s listing"));
        }
    }

    assert!(
        missing.is_empty(),
        "the guard holds rules the command that lists the guard does not show:\n  {}\n\nAn \
         inspector that shows some of the rules is believed about all of them.",
        missing.join("\n  ")
    );
}
