//! What a declared `@version=` can be turned into, and what happens when it cannot (`Q53`).
//!
//! **One implementation, two severities.** A pin the named manager cannot honour is a fact about
//! the declaration, and two callers need it at once: the planner refuses that package by name and
//! carries on with the rest, and `sync --locked` — which promises to put an exact machine back —
//! treats the same fact as fatal. Deriving it twice is how a rule comes to hold on one command
//! and not on its neighbour, so both ask here.
//!
//! **Every pin that reaches this point was typed by a person.** `StateResolver::apply_locks`
//! stops injecting a recorded version into a spec whose backend cannot replay one, so a version
//! surviving to here on such a backend can only have come from a line in a module. That is what
//! makes refusing it correct rather than officious: a version you typed is a decision, and the
//! one outcome worse than honouring it or refusing it is quietly installing something else.

use crate::backends::{capability, BackendRegistry};
use crate::core::PackageSpec;

/// A pin somebody wrote that the manager it names cannot express.
pub struct UnmeetablePin {
    /// `backend:name`, the way every other report keys a package.
    pub key: String,
    pub backend: String,
    pub name: String,
    pub version: String,
}

impl UnmeetablePin {
    /// The sentence a refusal prints.
    ///
    /// Names the manager, the pin and the reason. A message that says only "cannot be met" leaves
    /// the reader to guess whether they mistyped the version, the package or the manager.
    pub fn message(&self) -> String {
        match capability::cannot_pin_reason(&self.backend) {
            Some(why) => format!(
                "`{}` cannot install an exact version, so `{}@version={}` cannot be met — {}",
                self.backend, self.name, self.version, why
            ),
            // A backend with no ledger row is a backend somebody added without answering the
            // question. The refusal still fires — refusing is the safe half — and says plainly
            // that the reason is missing rather than inventing one.
            None => format!(
                "`{}` cannot install an exact version, so `{}@version={}` cannot be met (no \
                 reason is recorded for `{}` — see `capability::CANNOT_PIN_VERSION`)",
                self.backend, self.name, self.version, self.backend
            ),
        }
    }
}

/// Every declared pin the manager named cannot honour.
///
/// **Only where the manager is actually here.** A `pacman:jq@version=1.7` on Windows is not an
/// unmeetable pin — it is a declaration for a machine this is not, and the planner already has a
/// sentence for that. Answering "pacman cannot pin" there would replace a true message with a
/// misleading one, since pacman is not the reason nothing happens.
/// Takes the specs as pairs rather than as one map shape, because the two callers hold two
/// different ones — the planner a flat `backend:name` index, the resolver a map of lists — and a
/// signature that picked one would make the other rebuild its collection to ask a question about
/// it.
pub fn unmeetable<'s>(
    registry: &BackendRegistry,
    specs: impl Iterator<Item = (&'s str, &'s PackageSpec)>,
) -> Vec<UnmeetablePin> {
    let mut out = Vec::new();
    for (backend, spec) in specs {
        if !registry.runs_here(backend) || registry.pins_version(backend) {
            continue;
        }
        // `absent:` says a package must not exist; a version on it pins nothing to install.
        if !spec.present {
            continue;
        }
        let Some(version) = spec.options.one("version") else {
            continue;
        };
        // `latest` and `*` are not pins — they are the absence of one written out loud, and
        // every manager here can honour them by doing what it was going to do anyway.
        if !crate::backends::concrete_version(version) {
            continue;
        }
        out.push(UnmeetablePin {
            key: format!("{}:{}", backend, spec.name),
            backend: backend.to_string(),
            name: spec.name.clone(),
            version: version.to_string(),
        });
    }
    // Stable order, so two runs over the same config report the same list in the same order —
    // a `HashMap` iteration order would make a diff of two reports noise.
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::manager::{BackendCapabilities, BackendCore, Installable};
    use std::sync::Arc;

    struct Fake {
        name: String,
        pins: bool,
        here: bool,
    }

    #[async_trait::async_trait]
    impl BackendCore for Fake {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_available(&self) -> bool {
            self.here
        }
        fn probes(&self) -> Vec<String> {
            Vec::new()
        }
        fn needs_root(&self) -> bool {
            false
        }
    }

    #[async_trait::async_trait]
    impl Installable for Fake {
        async fn install(&self, _: &[PackageSpec], _: bool) -> crate::core::Result<()> {
            Ok(())
        }
        async fn remove(
            &self,
            _: &[String],
            _: bool,
            _: crate::app::sync::guard::Reaped,
        ) -> crate::core::Result<()> {
            Ok(())
        }
        fn pins_version(&self) -> bool {
            self.pins
        }
    }

    fn registry(backends: &[(&str, bool, bool)]) -> BackendRegistry {
        let mut reg = BackendRegistry::new();
        for (name, pins, here) in backends {
            let fake = Arc::new(Fake {
                name: (*name).to_string(),
                pins: *pins,
                here: *here,
            });
            reg.register(Arc::new(
                BackendCapabilities::builder(fake.clone())
                    .with_installable(fake)
                    .build(),
            ));
        }
        reg
    }

    fn spec(name: &str, backend: &str, version: Option<&str>) -> PackageSpec {
        let mut options = crate::config::grammar::Options::default();
        if let Some(v) = version {
            options.set("version", v.to_string());
        }
        PackageSpec {
            name: name.into(),
            backend: backend.into(),
            options,
            requires: vec![],
            present: true,
        }
    }

    fn found(reg: &BackendRegistry, specs: &[PackageSpec]) -> Vec<String> {
        unmeetable(reg, specs.iter().map(|s| (s.backend.as_str(), s)))
            .into_iter()
            .map(|p| p.key)
            .collect()
    }

    /// The whole point: a pin the manager cannot express is found, and the same pin on a manager
    /// that can is not. Both halves, because a check that finds everything is as useless as one
    /// that finds nothing.
    #[test]
    fn a_pin_is_unmeetable_only_where_the_manager_cannot_express_one() {
        let reg = registry(&[("cannot", false, true), ("can", true, true)]);
        assert_eq!(
            found(
                &reg,
                &[
                    spec("jq", "cannot", Some("1.7")),
                    spec("jq", "can", Some("1.7")),
                ]
            ),
            vec!["cannot:jq".to_string()]
        );
    }

    /// **A manager that is not on this machine is not the reason nothing happens.** The planner
    /// already says "`pacman` is not on this machine"; answering "`pacman` cannot pin" instead
    /// would replace a true sentence with a misleading one, and would fire on every host for
    /// every declaration meant for a different one.
    #[test]
    fn a_manager_that_is_not_here_raises_nothing() {
        let reg = registry(&[("cannot", false, false)]);
        assert!(found(&reg, &[spec("jq", "cannot", Some("1.7"))]).is_empty());
    }

    /// `latest` and `*` are the absence of a pin written out loud, and every manager honours
    /// them by doing what it was going to do anyway. Refusing those would refuse the ordinary
    /// case — and `@version=` parsing to an empty string is `AU10`, which is caught in the
    /// grammar and must not turn into a refusal here.
    #[test]
    fn only_a_concrete_version_is_a_pin() {
        let reg = registry(&[("cannot", false, true)]);
        for loose in ["latest", "*", ""] {
            assert!(
                found(&reg, &[spec("jq", "cannot", Some(loose))]).is_empty(),
                "`{loose}` was read as a pin"
            );
        }
        assert!(found(&reg, &[spec("jq", "cannot", None)]).is_empty());
    }

    /// `absent:jq@version=1.7` pins nothing to install — it says the package must not be there,
    /// and a version on it cannot fail to be honoured by a removal.
    #[test]
    fn an_absent_declaration_pins_nothing() {
        let reg = registry(&[("cannot", false, true)]);
        let mut s = spec("jq", "cannot", Some("1.7"));
        s.present = false;
        assert!(found(&reg, &[s]).is_empty());
    }

    /// The message names the manager, the pin **and** the reason. A refusal that says only
    /// "cannot be met" leaves the reader guessing which of the three they got wrong.
    #[test]
    fn the_message_carries_the_ledger_s_reason() {
        let reg = registry(&[("brew", false, true)]);
        let declared = spec("tokei", "brew", Some("14.0.0"));
        let pins = unmeetable(&reg, std::iter::once(("brew", &declared)));
        let message = pins[0].message();
        assert!(message.contains("brew"), "{message}");
        assert!(message.contains("tokei@version=14.0.0"), "{message}");
        assert!(
            message.contains("different formula"),
            "the ledger's reason is missing, so the refusal cannot say why: {message}"
        );
    }
}
