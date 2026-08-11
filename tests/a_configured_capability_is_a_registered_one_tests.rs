//! **A manager whose config says how to upgrade everything must be `Upgradable`.**
//!
//! `winget` and `scoop` each carry `upgrade_args` — `winget upgrade --all --silent`,
//! `scoop update *` — and each carried an `OutdatedProbe` to find out what needed it. Neither
//! was registered `.with_upgradable(…)`. `choco`, the third Windows manager, is registered with
//! it, in the same file, forty lines away.
//!
//! So `shall upgrade` on Windows upgraded chocolatey packages and silently skipped the other
//! two. There is no error: `as_upgradable()` answers `None`, and every caller reads that as
//! *this manager has no such concept* — the same answer `link:` gives.
//!
//! **And the capability matrix test asserted the loss.** `assert_caps(&reg, "winget", &[…])`
//! listed five capabilities without `upgradable`, so the omission was pinned as correct. A
//! matrix written from the code cannot notice that the code is wrong; this asks the other
//! question, which is whether the *config* and the *registration* agree.
//!
//! **What this cannot do, and why the exemption list has reasons.** A non-empty `upgrade_args`
//! does not always mean an upgrade-all verb exists — `pip install --upgrade` needs names and
//! fails without them, and `bun upgrade` upgrades bun itself. Both are correct omissions, and
//! both were indistinguishable from the two real losses until somebody wrote down which is
//! which. That is what `EXEMPT` is: the list of managers whose config *looks* like it upgrades
//! everything and does not, each with the sentence explaining it.

use std::collections::BTreeSet;

use crate::ledger::Ledger;

/// Managers whose `upgrade_args` is not an upgrade-everything verb. Each entry is a decision,
/// and the reason is the whole point of the entry.
const EXEMPT: &[(&str, &str)] = &[
    (
        "register_pip",
        "`pip install --upgrade` takes package names and fails without them, so there is no \
         upgrade-all to register. The args are the per-package form, used when a spec asks for \
         a newer version.",
    ),
    (
        "register_bun",
        "`bun upgrade` upgrades the bun runtime, not the packages bun installed. Registering it \
         would make `shall upgrade` replace the user's toolchain while reporting that it had \
         updated their packages.",
    ),
];

/// One `fn register_*` and everything up to the next one.
fn register_fns(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in src.split("\nfn register_").skip(1) {
        let name = chunk
            .split(['(', '<'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        out.push((format!("register_{name}"), chunk.to_string()));
    }
    out
}

/// Whether a config field is set to a non-empty list, in either spelling this file uses: a
/// struct literal (`upgrade_args: vec![…]`) or an assignment onto `base_config` (`cfg.upgrade_args
/// = vec![…]`).
fn declares_list(body: &str, field: &str) -> bool {
    for opener in [format!("{field}: vec!["), format!("{field} = vec![")] {
        let mut from = 0;
        while let Some(at) = body[from..].find(&opener) {
            let start = from + at + opener.len();
            if !body[start..].trim_start().starts_with(']') {
                return true;
            }
            from = start;
        }
    }
    false
}

/// Whether the builder chain in this function registers a capability.
fn registers(body: &str, capability: &str) -> bool {
    body.contains(&format!(".with_{capability}("))
}

/// The registrations whose config promises an upgrade-all verb and whose builder does not
/// deliver one.
///
/// **The exemptions are not filtered out here.** A finding set with the excused sites already
/// removed cannot tell a live exemption from a dead one — that subtraction belongs to
/// [`Ledger`], which does it in both directions.
fn losses(src: &str) -> BTreeSet<String> {
    register_fns(src)
        .into_iter()
        .filter(|(_, body)| body.contains("BackendCapabilities::builder"))
        .filter(|(_, body)| declares_list(body, "upgrade_args") && !registers(body, "upgradable"))
        .map(|(name, _)| name)
        .collect()
}

fn registry_source() -> String {
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/backends/registry.rs"
    ))
    .expect("registry.rs is readable")
}

#[test]
fn a_manager_that_says_how_to_upgrade_everything_is_upgradable() {
    let src = registry_source();

    let scanned = register_fns(&src)
        .into_iter()
        .filter(|(_, body)| body.contains("BackendCapabilities::builder"))
        .count();

    Ledger::of(
        "declaring an upgrade-all verb without registering `Upgradable`",
        "EXEMPT",
    )
    .pairs(EXEMPT)
    .scanning_at_least(15)
    .reason_of_at_least(60)
    .remedy(
        "`shall upgrade` skips them without saying so. Add \
         `.with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))`.",
    )
    .audit(scanned, &losses(&src));
}

/// The one thing [`Ledger`] cannot know: an exemption naming a registration that is not there.
///
/// Its stale check would catch a rename too, but it would report it as *"no longer declares an
/// upgrade-all verb"* — which sends the reader looking at `upgrade_args` for a function that
/// does not exist.
#[test]
fn no_exemption_outlives_the_thing_it_exempts() {
    let fns = register_fns(&registry_source());
    for (name, reason) in EXEMPT {
        assert!(
            fns.iter().any(|(n, _)| n == name),
            "EXEMPT names `{name}`, which is not a registration in registry.rs — it was renamed \
             or deleted, and its exemption now silences nothing: {reason}"
        );
    }
}

/// **The oracle.** The predicates above are driven over planted bodies, so a scan that has
/// stopped matching anything cannot pass by finding nothing.
#[test]
fn the_scan_can_actually_fail() {
    let planted = "
fn register_liar(reg: &mut BackendRegistry) {
    let core = Arc::new(GenericBackendCore {
        config: ManagerConfig {
            upgrade_args: vec![\"upgrade\".into(), \"--all\".into()],
        },
    });
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .build(),
    ));
}

fn register_honest(reg: &mut BackendRegistry) {
    let core = Arc::new(GenericBackendCore {
        config: ManagerConfig {
            upgrade_args: vec![\"update\".into()],
        },
    });
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_upgradable(Arc::new(GenericUpgradable { core: core.clone() }))
            .build(),
    ));
}

fn register_silent(reg: &mut BackendRegistry) {
    let core = Arc::new(GenericBackendCore {
        config: ManagerConfig {
            upgrade_args: vec![],
        },
    });
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .build(),
    ));
}

fn register_from_base(reg: &mut BackendRegistry) {
    let mut cfg = base_config(\"x\");
    cfg.upgrade_args = vec![\"upgrade\".into()];
    reg.register(Arc::new(
        BackendCapabilities::builder(core.clone())
            .with_installable(Arc::new(GenericInstallable { core: core.clone() }))
            .build(),
    ));
}

fn register_not_a_backend(reg: &mut BackendRegistry) {
    let cfg = ManagerConfig { upgrade_args: vec![\"upgrade\".into()] };
}
";
    assert_eq!(
        losses(planted),
        ["register_from_base", "register_liar"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<String>>(),
        "the scan missed a planted offender, or invented one"
    );

    // The four predicates, driven one at a time so a failure names which one broke.
    let fns = register_fns(planted);
    assert_eq!(fns.len(), 5, "the function splitter missed one");
    let body = |n: &str| fns.iter().find(|(name, _)| name == n).unwrap().1.clone();
    assert!(declares_list(&body("register_liar"), "upgrade_args"));
    assert!(
        declares_list(&body("register_from_base"), "upgrade_args"),
        "the assignment spelling used after `base_config` was not read"
    );
    assert!(
        !declares_list(&body("register_silent"), "upgrade_args"),
        "an empty `vec![]` is not a declaration"
    );
    assert!(!declares_list(&body("register_liar"), "enumerate_args"));
    assert!(registers(&body("register_honest"), "upgradable"));
    assert!(!registers(&body("register_liar"), "upgradable"));

    // And an exemption really does silence one — through the same subtraction the live gate uses.
    let remaining = Ledger::of("planted", "EXEMPT")
        .pairs(&[("register_liar", "")])
        .unexplained_in(&losses(planted));
    assert_eq!(remaining, vec!["register_from_base".to_string()]);
}
