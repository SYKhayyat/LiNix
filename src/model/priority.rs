use crate::config::grammar::error::{GrammarError, Origin, Result};
use crate::config::parser::{eval_when, HostFacts};
use std::path::Path;

/// The `priority` file: which backends this setup uses, and in what order (SPEC II.6).
///
/// One list, one question. It replaces four settings that expressed one fact between them
/// — `backend_priority`, `enabled_backends`, `hostname_backends`, `default_backend` — of
/// which only two ever merged (V.15).
///
/// **Listed = available to LiNix, in this order. Not listed = LiNix does not use it at
/// all.** An explicit `snap:foo` failing when snap is not listed is the feature: it catches
/// typos, and it makes your backend set declared rather than inherited from whatever
/// happens to be installed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Priority {
    backends: Vec<String>,
}

impl Priority {
    pub fn from_backends(backends: Vec<String>) -> Self {
        Self { backends }
    }

    /// Parse the file body, applying `when` blocks for this host.
    ///
    /// The body is lines, and `when` gates them — the same rule as everywhere else (II.2).
    pub fn parse(file: &Path, body: &str, facts: &HostFacts) -> Result<Self> {
        let mut backends: Vec<String> = Vec::new();
        let mut gate: Option<bool> = None;

        for (idx, raw) in body.lines().enumerate() {
            let origin = Origin::new(file, idx + 1);
            let line = match raw.find('#') {
                Some(i) => &raw[..i],
                None => raw,
            }
            .trim();
            if line.is_empty() {
                continue;
            }

            if line == "}" {
                if gate.is_none() {
                    return Err(GrammarError::new(
                        origin,
                        "`}` closes a `when` that was never opened",
                    ));
                }
                gate = None;
                continue;
            }

            if let Some(pred) = line.strip_suffix('{') {
                let pred = pred.trim();
                let Some(pred) = pred.strip_prefix("when ") else {
                    return Err(GrammarError::new(
                        origin,
                        format!("`{}` is not a `when` block", pred),
                    )
                    .with_hint("`priority` holds backend names and `when` blocks, nothing else."));
                };
                if gate.is_some() {
                    return Err(GrammarError::new(origin, "a `when` block inside a `when` block")
                        .with_hint("`priority` nests one level: name the condition once."));
                }
                gate = Some(eval_when(pred.trim(), facts).map_err(|e| {
                    GrammarError::new(Origin::new(file, idx + 1), e.to_string())
                })?);
                continue;
            }

            if gate == Some(false) {
                continue;
            }

            if line.split_whitespace().count() > 1 {
                return Err(GrammarError::new(
                    origin,
                    format!("`{}` is not a backend name", line),
                )
                .with_hint("one backend per line."));
            }
            // First mention wins: a `when` block naming apt, then a global apt below, must
            // not move apt down the order.
            if !backends.iter().any(|b| b == line) {
                backends.push(line.to_string());
            }
        }

        if gate.is_some() {
            return Err(
                GrammarError::new(Origin::new(file, 0), "a `when` block is never closed")
                    .with_hint("add the matching `}`."),
            );
        }

        Ok(Self { backends })
    }

    /// Whether LiNix uses this backend at all.
    pub fn allows(&self, backend: &str) -> bool {
        self.backends.iter().any(|b| b == backend)
    }

    /// The order to probe a bare name in.
    pub fn order(&self) -> &[String] {
        &self.backends
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// The refusal for an explicit `snap:foo` when snap is not listed.
    pub fn reject(&self, backend: &str, origin: &Origin) -> GrammarError {
        GrammarError::new(
            origin.clone(),
            format!("`{}` isn't in your priority list.", backend),
        )
        .with_hint(format!(
            "add `{}` to `priority` if you want LiNix to use it. Not listed means LiNix \
             does not use it at all.",
            backend
        ))
    }
}

/// Order backends the way V.14 says, keeping only the ones this machine has.
///
/// Most of an alphabetical or hand-kept order is meaningless — apt, pacman and dnf never
/// coexist, so their relative order never decides anything. Exactly one distinction does:
/// **a system manager beats a language manager**, because your distro maintains that build
/// and updates it with everything else. A language manager is for what your distro does not
/// carry. `pip` is last on its own: it installs into the system Python and can break it.
///
/// Anything unrecognised sorts with the language managers rather than ahead of them: a
/// backend the onboarder added is not known to be safe to prefer over your distro.
pub fn starter_order(available: &[String]) -> Vec<String> {
    const SYSTEM: &[&str] = &[
        "apt", "dnf", "pacman", "zypper", "apk", "xbps", "yay", "paru", "winget", "scoop",
        "choco", "brew", "nix", "flatpak", "snap",
    ];

    let rank = |b: &str| -> usize {
        if b == "pip" {
            return 2;
        }
        if SYSTEM.contains(&b) {
            return 0;
        }
        1
    };

    let mut out: Vec<String> = available.to_vec();
    out.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            // Stable and predictable within a tier. The order inside a tier decides nothing
            // real, so it should at least not vary between runs.
            .then_with(|| a.cmp(b))
    });
    out
}

/// The generated starter `priority`, with its reason in a comment.
///
/// F1/V.14: most of the old 10-backend order was meaningless — apt, pacman and dnf never
/// coexist. The order that decides anything is system manager before language manager, and
/// a default nobody can explain is a default nobody can safely change (P5). So the file
/// says why.
pub fn starter_file(detected: &[String]) -> String {
    let mut out = String::from(
        "# Which package managers this machine uses, and in what order.\n\
         #\n\
         # Listed = LiNix uses it. Not listed = LiNix does not use it at all, and an\n\
         # explicit `snap:foo` will say so rather than guess.\n\
         #\n\
         # The order only decides one thing: when two managers both have a package, which\n\
         # one wins. System managers come first because your distro maintains that build\n\
         # and updates it with everything else; language managers are for what your distro\n\
         # does not carry. pip is last because it installs into your system Python and can\n\
         # break it.\n\
         #\n\
         # `when host == laptop { ... }` gates a group to one machine.\n\n",
    );
    for b in detected {
        out.push_str(b);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn facts() -> HostFacts {
        HostFacts {
            os: "linux".into(),
            arch: "x86_64".into(),
            host: "laptop".into(),
            family: "unix".into(),
        }
    }

    fn parse(body: &str) -> Result<Priority> {
        Priority::parse(&PathBuf::from("priority"), body, &facts())
    }

    #[test]
    fn a_plain_list_keeps_its_order() {
        let p = parse("apt\ndnf\ncargo\nsnap\n").unwrap();
        assert_eq!(p.order(), ["apt", "dnf", "cargo", "snap"]);
    }

    #[test]
    fn not_listed_means_linix_does_not_use_it() {
        // V.15. This is the feature, not a limitation: it catches typos and makes your
        // backend set declared rather than inherited.
        let p = parse("apt\ncargo\n").unwrap();
        assert!(p.allows("apt"));
        assert!(!p.allows("snap"));
    }

    #[test]
    fn the_refusal_says_how_to_fix_it() {
        let p = parse("apt\n").unwrap();
        let err = p.reject("snap", &Origin::new("modules/dev.txt", 4));
        assert!(err.what.contains("isn't in your priority list"), "{}", err);
        assert!(err.to_string().contains("modules/dev.txt:4"), "{}", err);
    }

    #[test]
    fn a_when_block_gates_the_backends_inside_it() {
        let body = "when host == laptop {\n  cargo\n}\napt\n";
        assert_eq!(parse(body).unwrap().order(), ["cargo", "apt"]);

        let body = "when host == server {\n  cargo\n}\napt\n";
        assert_eq!(parse(body).unwrap().order(), ["apt"]);
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let p = parse("# which managers\n\napt   # the system one\ncargo\n").unwrap();
        assert_eq!(p.order(), ["apt", "cargo"]);
    }

    #[test]
    fn a_backend_named_twice_keeps_its_first_position() {
        let p = parse("when host == laptop {\n  apt\n}\ncargo\napt\n").unwrap();
        assert_eq!(p.order(), ["apt", "cargo"]);
    }

    #[test]
    fn a_line_that_is_not_a_backend_name_is_an_error() {
        assert!(parse("apt install curl\n").is_err());
    }

    #[test]
    fn an_unclosed_when_is_an_error() {
        assert!(parse("when host == laptop {\n  cargo\n").is_err());
    }

    #[test]
    fn a_stray_brace_is_an_error() {
        assert!(parse("apt\n}\n").is_err());
    }

    #[test]
    fn a_system_manager_outranks_a_language_manager() {
        // V.14: if both apt and cargo have ripgrep, apt wins — your distro maintains that
        // build and updates it with everything else.
        let out = starter_order(&["cargo".into(), "npm".into(), "apt".into()]);
        assert_eq!(out, ["apt", "cargo", "npm"]);
    }

    #[test]
    fn pip_is_last_because_it_can_break_your_system_python() {
        let out = starter_order(&["pip".into(), "cargo".into(), "apt".into()]);
        assert_eq!(out, ["apt", "cargo", "pip"]);
    }

    #[test]
    fn an_unknown_backend_does_not_outrank_your_distro() {
        // The onboarder can add backends at runtime, and nothing says a custom one is safe
        // to prefer over the distro's own build.
        let out = starter_order(&["weird".into(), "apt".into()]);
        assert_eq!(out, ["apt", "weird"]);
    }

    #[test]
    fn only_what_this_machine_has_is_listed() {
        // "Detected, never configured" (V.41). Listing apt on a machine with no apt is
        // homework, and `priority` is not a wish list.
        let out = starter_order(&["cargo".into()]);
        assert_eq!(out, ["cargo"]);
    }

    #[test]
    fn the_starter_file_carries_its_reason() {
        // F1/V.14, and P5: a default without a reason cannot be safely changed.
        let body = starter_file(&["apt".into(), "cargo".into()]);
        assert!(body.contains("System managers come first"));
        assert!(body.contains("pip is last"));
        // And it must parse back.
        let p = Priority::parse(&PathBuf::from("priority"), &body, &facts()).unwrap();
        assert_eq!(p.order(), ["apt", "cargo"]);
    }
}
