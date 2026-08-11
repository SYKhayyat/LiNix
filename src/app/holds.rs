//! Which packages are frozen against an upgrade — from **both** places that can say so.
//!
//! A hold has two sources and only one of them was ever read. `shall hold` writes a ledger into
//! `data/registry.json`; `@hold=true` on a manifest line is a declaration. The option is in
//! `PACKAGE_OPTION_KEYS`, `validate_package` refuses it beside `@version` as a contradiction, and
//! II.2 documents it — and until 2026-08-11 no code in the tree read it, so a declared hold
//! parsed, validated, and did nothing at all.
//!
//! **The first fix was to teach two readers about the declaration, and that was the same mistake
//! one size smaller.** There were four, not two: `upgrade --security` built its own closure over
//! `StateRegistry::held` so it did not even contain the string `is_held`; the "holds are not
//! enforced by a native whole-system upgrade" warning counted the ledger and would have said
//! nothing to somebody whose holds were all declared; and `shall hold` with no arguments — the
//! command whose entire job is *tell me what is held* — answered `No packages are held.` over a
//! manifest that held three. That last one is a `list` disagreeing with the machine, which is the
//! defect this repository grades itself against.
//!
//! So the union is a type, asked once, and nothing else asks the ledger. `spec_is_missing` in the
//! planner is the one deliberate exception and it is not really one: it is handed the spec, so
//! the declaration is already in its hand and the union it computes is this one, per package.

use crate::core::{PackageSpec, StateRegistry};
use std::collections::{BTreeSet, HashMap};

/// Every package frozen against an upgrade, and which source froze it.
pub struct Holds {
    /// `shall hold` entries, verbatim: `backend:name`, or a bare `name` that matches under any
    /// backend. Matching stays here rather than being normalised, because the bare form is a
    /// deliberate feature of the ledger and normalising it would need a backend to guess at.
    ledger: Vec<String>,
    /// `(backend, name)` for every present declaration carrying `@hold=true`, with the file and
    /// line it came from so a listing can name it.
    declared: BTreeSet<(String, String, String)>,
}

impl Holds {
    /// Read both sources. The desired state is optional: `upgrade` and `hold` must not fail
    /// because a module has a syntax error, so a model that will not resolve yields the ledger
    /// alone — and the caller says so out loud rather than silently honouring half the holds.
    pub fn new(state: &StateRegistry, desired: Option<&HashMap<String, Vec<PackageSpec>>>) -> Self {
        let declared = desired
            .map(|d| {
                d.values()
                    .flatten()
                    .filter(|s| s.present && s.declares_hold())
                    .map(|s| {
                        (
                            s.backend.clone(),
                            s.name.clone(),
                            s.options
                                .one("__source")
                                .unwrap_or("a manifest")
                                .to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            ledger: state.list_held().to_vec(),
            declared,
        }
    }

    /// Is this package frozen, by either source?
    pub fn contains(&self, backend: &str, name: &str) -> bool {
        let qualified = format!("{}:{}", backend, name);
        self.ledger.iter().any(|k| k == name || *k == qualified)
            || self
                .declared
                .iter()
                .any(|(b, n, _)| b == backend && n == name)
    }

    /// Whether this one was frozen by a declaration rather than by the ledger — the two are
    /// released differently, and telling somebody to run `shall unhold` against a manifest line
    /// sends them to a command that will report nothing to do.
    pub fn is_declared(&self, backend: &str, name: &str) -> bool {
        self.declared
            .iter()
            .any(|(b, n, _)| b == backend && n == name)
    }

    /// What releases this hold, in the words of the thing that made it.
    pub fn release(&self, backend: &str, name: &str) -> &'static str {
        if self.is_declared(backend, name) {
            "remove `@hold` from the line"
        } else {
            "`shall unhold`"
        }
    }

    pub fn len(&self) -> usize {
        self.ledger.len() + self.declared.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// One line per hold, naming where it came from.
    ///
    /// The source matters more than it looks: the two are released by different commands, and a
    /// listing that prints them identically is a listing that sends half its readers to the
    /// wrong one.
    pub fn describe(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .ledger
            .iter()
            .map(|k| format!("{:<40} (shall hold)", k))
            .collect();
        out.extend(
            self.declared.iter().map(|(b, n, from)| {
                format!("{:<40} (@hold=true, {})", format!("{}:{}", b, n), from)
            }),
        );
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(backend: &str, name: &str, hold: bool) -> PackageSpec {
        let mut s = PackageSpec {
            name: name.into(),
            backend: backend.into(),
            ..Default::default()
        };
        if hold {
            s.options.set("hold", "true");
        }
        s.options.set("__source", "modules/tools.txt:3");
        s
    }

    fn desired(specs: Vec<PackageSpec>) -> HashMap<String, Vec<PackageSpec>> {
        let mut out: HashMap<String, Vec<PackageSpec>> = HashMap::new();
        for s in specs {
            out.entry(s.backend.clone()).or_default().push(s);
        }
        out
    }

    #[test]
    fn a_declared_hold_counts_and_so_does_a_ledger_one() {
        let mut state = StateRegistry::default();
        state.hold("cargo:ripgrep");
        let d = desired(vec![
            spec("npm", "typescript", true),
            spec("npm", "eslint", false),
        ]);
        let holds = Holds::new(&state, Some(&d));

        assert!(holds.contains("cargo", "ripgrep"), "the ledger entry");
        assert!(holds.contains("npm", "typescript"), "the declared one");
        assert!(!holds.contains("npm", "eslint"), "no `@hold` on this line");
        assert_eq!(holds.len(), 2);
    }

    /// The two are released by different commands, and the answer has to say which.
    #[test]
    fn each_hold_is_released_by_the_command_that_made_it() {
        let mut state = StateRegistry::default();
        state.hold("cargo:ripgrep");
        let d = desired(vec![spec("npm", "typescript", true)]);
        let holds = Holds::new(&state, Some(&d));

        assert_eq!(holds.release("cargo", "ripgrep"), "`shall unhold`");
        assert_eq!(
            holds.release("npm", "typescript"),
            "remove `@hold` from the line"
        );
    }

    /// A bare ledger entry matches under any backend — that is what the ledger's own matching
    /// does, and this union must not narrow it.
    #[test]
    fn a_bare_ledger_entry_still_matches_any_backend() {
        let mut state = StateRegistry::default();
        state.hold("curl");
        let holds = Holds::new(&state, None);
        assert!(holds.contains("apt", "curl"));
        assert!(holds.contains("brew", "curl"));
        assert!(!holds.contains("apt", "wget"));
    }

    /// A model that will not resolve yields the ledger alone rather than failing: `upgrade` is
    /// not a command that needs the manifest to parse.
    #[test]
    fn an_unresolvable_model_leaves_the_ledger_holds_working() {
        let mut state = StateRegistry::default();
        state.hold("cargo:ripgrep");
        let holds = Holds::new(&state, None);
        assert!(holds.contains("cargo", "ripgrep"));
        assert_eq!(holds.len(), 1);
    }

    /// An `absent:` line carrying `@hold` is not a hold: it declares the package must NOT be
    /// there, and freezing something against an upgrade it is not going to get is meaningless.
    #[test]
    fn an_absent_declaration_holds_nothing() {
        let mut s = spec("npm", "typescript", true);
        s.present = false;
        let holds = Holds::new(&StateRegistry::default(), Some(&desired(vec![s])));
        assert!(holds.is_empty());
    }
}
