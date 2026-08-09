//! An option that is applied at install time and never again (Q19, Q20).
//!
//! **One bug class, two backends.** Q18 made `@quota`, `@size`, `@mount` and `@mount_options`
//! writable and applied them at creation; `@classic` had been in the same state on `snap:` since
//! it was written. Nothing decided what a *changed* one meant, and the answer the code gave was
//! "nothing": the package exists under its name, so `sync` found no drift and reported success
//! over a declaration it was no longer applying.
//!
//! These drive the **real planner** over a scripted `zfs list` / `lvs` / `snap info`, because the
//! defect was never in the argv — it was in nobody asking. The `@mount`-hides-`@quota` and
//! `@channel`-hides-`@classic` cases are the same fault twice: a drift check that `return`ed on
//! the first option it recognised, so writing two options together killed one of them.

use linix::app::sync::planner::{ChangePlanner, HostBackends, PlanScope};
use linix::core::executor::DryRunOutput;
use linix::core::{GraphAction, PackageSpec};
use std::collections::HashMap;

mod mock_providers;
use mock_providers::TestKernel;

const TEN_GIB: u64 = 10 * (1 << 30);

fn spec(backend: &str, name: &str, options: &[(&str, &str)]) -> PackageSpec {
    PackageSpec {
        name: name.into(),
        backend: backend.into(),
        options: options
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        requires: vec![],
        present: true,
    }
}

/// Whether `sync` would act on this declaration, given what the tools report.
async fn plans_a_change(reports: &[(&str, &str)], spec: PackageSpec) -> bool {
    let kernel = TestKernel::new().await;
    for (command, stdout) in reports {
        let out = DryRunOutput {
            stdout: stdout.as_bytes().to_vec(),
            ..Default::default()
        };
        kernel.mock_executor.set_response(command, Ok(out.into()));
    }
    let mut desired: HashMap<String, Vec<PackageSpec>> = HashMap::new();
    desired.insert(spec.backend.clone(), vec![spec]);

    let state_guard = kernel.state.lock().await;
    let planner = ChangePlanner::new(
        kernel.app.registry.clone(),
        &state_guard,
        &kernel.app.config,
    );
    let changes = planner
        .plan(&desired, PlanScope::Whole(HostBackends::default()))
        .await
        .expect("planning failed");
    let scheduled = changes
        .graph
        .node_weights()
        .any(|w| matches!(w, GraphAction::Install(_)));
    scheduled
}

const ZFS_LIST: &str = "zfs list -H -p -o name,quota,mountpoint";
const LVS_LIST: &str = "lvs --noheadings --units b --nosuffix -o vg_name,lv_name,lv_size";

/// The failure mode Q19's own entry named: a comparison that gets units wrong reports a change
/// on every sync for ever. `10G`, `10240M` and the raw byte count are one quota, and `-p` is why
/// the reported side is never a display string.
#[tokio::test]
async fn a_quota_that_matches_is_not_a_change_however_it_is_spelled() {
    for declared in ["10G", "10240M", "10737418240", "10GiB"] {
        assert!(
            !plans_a_change(
                &[(ZFS_LIST, &format!("tank/data\t{}\t/mnt/data\n", TEN_GIB))],
                spec("zfs", "tank/data", &[("quota", declared)]),
            )
            .await,
            "@quota={} against a 10 GiB dataset planned a change",
            declared
        );
    }
}

/// The defect itself. Editing the number did nothing, on any of the three backends that carry
/// one — so each is asserted, not just the one that was reported.
#[tokio::test]
async fn an_edited_quota_or_size_is_drift() {
    assert!(
        plans_a_change(
            &[(ZFS_LIST, &format!("tank/data\t{}\t/mnt/data\n", TEN_GIB))],
            spec("zfs", "tank/data", &[("quota", "20G")]),
        )
        .await,
        "zfs: 10G on disk, 20G declared"
    );
    assert!(
        plans_a_change(
            &[(LVS_LIST, &format!("  vg0 data {}\n", TEN_GIB))],
            spec("lvm", "vg0/data", &[("size", "20G")]),
        )
        .await,
        "lvm: 10G on disk, 20G declared"
    );
    // Shrinking is drift too — the planner's job is to notice, and the backend's to refuse
    // without `@allow_shrink`. A planner that only noticed growth would make the refusal
    // unreachable and the flag decorative.
    assert!(
        plans_a_change(
            &[(LVS_LIST, &format!("  vg0 data {}\n", TEN_GIB))],
            spec("lvm", "vg0/data", &[("size", "5G")]),
        )
        .await,
        "lvm: 10G on disk, 5G declared"
    );
}

/// `none` is a state, not an unknown: the dataset was read and carries no limit, so a line that
/// declares one is unsatisfied. This is the `@mount` rule from Q18 applied to the sibling
/// property — a quota that silently never happened would be invisible for ever.
#[tokio::test]
async fn a_dataset_with_no_limit_at_all_is_drift_against_a_declared_one() {
    assert!(
        plans_a_change(
            &[(ZFS_LIST, "tank/data\t0\t/mnt/data\n")],
            spec("zfs", "tank/data", &[("quota", "10G")]),
        )
        .await
    );
}

/// The sibling the fix would have missed. `@mount` used to `return` out of the drift check, so a
/// line carrying a mount *and* a quota had only the mount looked at: the quota was dead the
/// moment somebody wrote the two options together, which is the ordinary way to write them.
#[tokio::test]
async fn a_satisfied_mount_does_not_hide_a_drifted_quota() {
    let listing = format!("tank/data\t{}\t/mnt/data\n", TEN_GIB);
    assert!(
        plans_a_change(
            &[(ZFS_LIST, &listing)],
            spec(
                "zfs",
                "tank/data",
                &[("mount", "/mnt/data"), ("quota", "20G")],
            ),
        )
        .await,
        "the mount matches and the quota does not — that is a change"
    );
    // And the other way round: both satisfied is still nothing to do.
    assert!(
        !plans_a_change(
            &[(ZFS_LIST, &listing)],
            spec(
                "zfs",
                "tank/data",
                &[("mount", "/mnt/data"), ("quota", "10G")],
            ),
        )
        .await
    );
}

/// Q20 — the same defect on a different backend, and the reason this file is not called
/// `lvm_tests`. `@classic` was applied when the install argv was built and never again, so a snap
/// that gained the option after it was installed stayed strictly confined for ever with `sync`
/// reporting nothing to do.
#[tokio::test]
async fn a_snap_that_gained_classic_after_it_was_installed_is_drift() {
    const INFO: &str = "snap info -- code";
    let strict = "name:      code\ntracking:     latest/stable\ninstalled:  1.85.1 (139) 351MB -\n";
    let classic =
        "name:      code\ntracking:     latest/stable\ninstalled:  1.85.1 (139) 351MB classic\n";

    assert!(
        plans_a_change(
            &[(INFO, strict)],
            spec("snap", "code", &[("classic", "true")]),
        )
        .await,
        "declared classic, installed strict — that is a change"
    );
    assert!(
        !plans_a_change(
            &[(INFO, classic)],
            spec("snap", "code", &[("classic", "true")]),
        )
        .await,
        "declared classic, installed classic — nothing to do"
    );
    // Absent means unmanaged, exactly as it does for `@quota`: a line that says nothing about
    // confinement is not asking for strict, and must never schedule the remove-and-reinstall
    // that narrowing would take.
    assert!(!plans_a_change(&[(INFO, classic)], spec("snap", "code", &[])).await);
}

/// The sibling of Q19's `@mount` fault, on the backend that had it too: `@channel` used to
/// `return` out of the drift check, so a snap carrying a channel *and* `@classic` had only the
/// channel looked at.
#[tokio::test]
async fn a_satisfied_channel_does_not_hide_a_drifted_confinement() {
    const INFO: &str = "snap info -- code";
    let strict_on_stable =
        "name:      code\ntracking:     latest/stable\ninstalled:  1.85.1 (139) 351MB -\n";
    assert!(
        plans_a_change(
            &[(INFO, strict_on_stable)],
            spec(
                "snap",
                "code",
                &[("channel", "stable"), ("classic", "true")]
            ),
        )
        .await,
        "the channel matches and the confinement does not — that is a change"
    );
}

/// Y23 — the same defect on the *other* backend that publishes channels. flatpak calls a channel
/// a branch, `@channel` really does reach the installed ref (`org.gimp.GIMP//beta`), and the
/// listing LiNix read asked for `application,version` — so the branch was never known, the drift
/// check had nothing to compare, and editing a flatpak's channel did nothing for ever.
#[tokio::test]
async fn an_edited_flatpak_branch_is_drift() {
    const LIST: &str = "flatpak list --app --columns=application,version,branch";
    let on_stable = "org.gimp.GIMP\t2.10\tstable\n";

    assert!(
        plans_a_change(
            &[(LIST, on_stable)],
            spec("flatpak", "org.gimp.GIMP", &[("channel", "beta")]),
        )
        .await,
        "declared beta, installed on stable — that is a change"
    );
    assert!(
        !plans_a_change(
            &[(LIST, on_stable)],
            spec("flatpak", "org.gimp.GIMP", &[("channel", "stable")]),
        )
        .await,
        "declared stable, installed on stable — nothing to do"
    );
    // A line that says nothing about a branch is not asking for one, exactly as `@classic` is
    // not asking for strict confinement.
    assert!(!plans_a_change(&[(LIST, on_stable)], spec("flatpak", "org.gimp.GIMP", &[])).await);
}

/// D13's rule where flatpak is the backend that forces it. flatpak installs branches side by
/// side and its listing has no column saying which one is current, so an app on two branches is
/// a channel LiNix cannot read — and an unreadable value is left alone. Reporting either row
/// would schedule a switch on every sync for ever, which is worse than the drift it would catch.
#[tokio::test]
async fn an_app_on_two_branches_is_left_alone_rather_than_switched_for_ever() {
    const LIST: &str = "flatpak list --app --columns=application,version,branch";
    let both = "org.gimp.GIMP\t2.10\tstable\norg.gimp.GIMP\t2.99\tbeta\n";

    for declared in ["stable", "beta", "23.08"] {
        assert!(
            !plans_a_change(
                &[(LIST, both)],
                spec("flatpak", "org.gimp.GIMP", &[("channel", declared)]),
            )
            .await,
            "@channel={declared} against a two-branch install planned a change"
        );
    }
}

/// The versionless case, which is most of flathub. flatpak emits an empty middle field
/// (`org.gimp.GIMP\t\tstable`), so a whitespace split reads `stable` as the version and the
/// branch as absent — and a declaration that matched the machine would plan a change every run.
#[tokio::test]
async fn a_flatpak_with_no_version_still_has_a_readable_branch() {
    const LIST: &str = "flatpak list --app --columns=application,version,branch";
    assert!(
        !plans_a_change(
            &[(LIST, "org.gimp.GIMP\t\tstable\n")],
            spec("flatpak", "org.gimp.GIMP", &[("channel", "stable")]),
        )
        .await,
        "the branch is readable even with no version beside it"
    );
    assert!(
        plans_a_change(
            &[(LIST, "org.gimp.GIMP\t\tstable\n")],
            spec("flatpak", "org.gimp.GIMP", &[("channel", "beta")]),
        )
        .await
    );
}

/// A volume whose declaration matches the disk is not touched. The whole feature is worthless if
/// it schedules work on a machine that is already correct — that is the "change for ever" failure
/// wearing the opposite sign.
#[tokio::test]
async fn a_matching_declaration_plans_nothing() {
    assert!(
        !plans_a_change(
            &[(LVS_LIST, &format!("  vg0 data {}\n", TEN_GIB))],
            spec("lvm", "vg0/data", &[("size", "10G")]),
        )
        .await
    );
}
