//! **A manager whose config says how to upgrade everything must be `Upgradable`.**
//!
//! `winget` and `scoop` each carry `upgrade_args` — `winget upgrade --all --silent`,
//! `scoop update *` — and each carried an `OutdatedProbe` to find out what needed it. Neither
//! was registered `.with_upgradable(…)`. `choco`, the third Windows manager, is registered with
//! it, in the same file, forty lines away.
//!
//! So `linix upgrade` on Windows upgraded chocolatey packages and silently skipped the other
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
         would make `linix upgrade` replace the user's toolchain while reporting that it had \
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
/// deliver one — minus the ones a human has explained.
fn losses(src: &str) -> Vec<String> {
    let exempt: BTreeSet<&str> = EXEMPT.iter().map(|(name, _)| *name).collect();
    register_fns(src)
        .into_iter()
        .filter(|(_, body)| body.contains("BackendCapabilities::builder"))
        .filter(|(name, body)| {
            declares_list(body, "upgrade_args")
                && !registers(body, "upgradable")
                && !exempt.contains(name.as_str())
        })
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

    // A scan over an empty set is not a clean scan. `registry.rs` builds every built-in, and if
    // this ever finds fewer than a dozen it is reading the wrong file or the wrong shape.
    let scanned = register_fns(&src)
        .into_iter()
        .filter(|(_, body)| body.contains("BackendCapabilities::builder"))
        .count();
    assert!(
        scanned >= 15,
        "only {scanned} registrations were scanned; the extractor has stopped matching"
    );

    let losses = losses(&src);
    assert!(
        losses.is_empty(),
        "these managers declare an upgrade-all verb and are not registered `Upgradable`, so \
         `linix upgrade` skips them without saying so: {losses:?}\n\
         Either add `.with_upgradable(Arc::new(GenericUpgradable {{ core: core.clone() }}))`, \
         or add the manager to EXEMPT in this file with the sentence explaining why its \
         `upgrade_args` is not an upgrade-all."
    );
}

/// Every exemption names a registration that exists and still needs exempting.
///
/// An exemption for a manager that has since been registered `Upgradable`, or renamed, is a
/// line that silences nothing and reads as if it does.
#[test]
fn no_exemption_outlives_the_thing_it_exempts() {
    let src = registry_source();
    let fns = register_fns(&src);
    for (name, reason) in EXEMPT {
        let Some((_, body)) = fns.iter().find(|(n, _)| n == name) else {
            panic!("EXEMPT names `{name}`, which is not a registration in registry.rs");
        };
        assert!(
            declares_list(body, "upgrade_args"),
            "`{name}` no longer declares `upgrade_args`, so its exemption is dead: {reason}"
        );
        assert!(
            !registers(body, "upgradable"),
            "`{name}` is registered `Upgradable` now — drop its exemption: {reason}"
        );
        assert!(
            reason.len() > 60,
            "`{name}`'s exemption has no explanation, which is the thing being exempted"
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
        vec![
            "register_liar".to_string(),
            "register_from_base".to_string()
        ],
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

    // And an exemption really does silence one.
    let exempt: BTreeSet<&str> = ["register_liar"].into_iter().collect();
    let remaining: Vec<String> = register_fns(planted)
        .into_iter()
        .filter(|(_, b)| b.contains("BackendCapabilities::builder"))
        .filter(|(n, b)| {
            declares_list(b, "upgrade_args")
                && !registers(b, "upgradable")
                && !exempt.contains(n.as_str())
        })
        .map(|(n, _)| n)
        .collect();
    assert_eq!(remaining, vec!["register_from_base".to_string()]);
}
