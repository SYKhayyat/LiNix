use crate::config::grammar::error::{GrammarError, Origin, Result};
use crate::config::grammar::options::Options;
use crate::config::grammar::{gated, Vocabulary};
use crate::config::parser::HostFacts;
use std::collections::BTreeMap;
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
    /// A backend's machine-wide defaults — `formats`, `channel` (VIII.2) — from the options
    /// body on its line. Absent for a backend written as a bare name, which is most of them.
    options: BTreeMap<String, Options>,
}

impl Priority {
    pub fn from_backends(backends: Vec<String>) -> Self {
        Self {
            backends,
            options: BTreeMap::new(),
        }
    }

    /// Every backend the file names, `when` blocks included whether or not they match.
    ///
    /// The bootstrap half of `priority`'s two passes. `priority` says which backends exist and
    /// resolving variables needs that vocabulary, so one of the two has to go first without the
    /// other — and evaluating `when $role == travel` before variables exist is the unknown-key
    /// refusal that made a variable unusable here at all. Taking the union instead evaluates
    /// nothing: it can only be a superset, and the `vars` file names no backend, so nothing this
    /// pass over-includes can change what a variable resolves to. **The result is a vocabulary
    /// and never an order** — [`Priority::parse`] against the resolved facts decides that.
    pub fn every_backend(file: &Path, body: &str) -> Result<Self> {
        let mut backends: Vec<String> = Vec::new();
        for entry in gated::read_every(file, body, &Self::vocabulary())? {
            if !backends.iter().any(|b| b == &entry.text) {
                backends.push(entry.text);
            }
        }
        Ok(Self::from_backends(backends))
    }

    fn vocabulary() -> Vocabulary<'static> {
        Vocabulary {
            noun: "backend name",
            holds: "`priority` holds backend names and `when` blocks, nothing else. One backend per line.",
            nesting: "`priority` nests one level: name the condition once.",
            body: Some(
                "a backend's body holds its defaults, one `key = value` per line: \
                 `github { formats = deb }`.",
            ),
        }
    }

    /// Parse the file body, applying `when` blocks for this host.
    ///
    /// The block structure is the shared one (`grammar::gated`) — `active` reads the same
    /// shape, and two copies of it had already drifted.
    ///
    /// `facts` must carry this run's variables: `when $role == travel { cargo }` is legal here,
    /// and reading it without them is an unknown key, not a block that does not match.
    pub fn parse(file: &Path, body: &str, facts: &HostFacts) -> Result<Self> {
        let mut backends: Vec<String> = Vec::new();
        let mut options: BTreeMap<String, Options> = BTreeMap::new();
        for entry in gated::read(file, body, facts, &Self::vocabulary())? {
            if !entry.on {
                continue;
            }
            // First mention wins: a `when` block naming apt, then a global apt below, must
            // not move apt down the order. The same rule decides the options body, so the
            // `when` arm that matched beats the unconditional line below it.
            if !backends.iter().any(|b| b == &entry.text) {
                backends.push(entry.text.clone());
            }
            if !entry.options.is_empty() {
                // The same rules as a declaration's options: an option on a backend that
                // cannot read it is an error here too, or `priority` becomes the one file
                // where a line that does nothing is legal.
                crate::config::grammar::statement::validate_backend_options(
                    &Origin::new(file, entry.line),
                    Some(entry.text.as_str()),
                    &entry.options,
                )?;
                options.entry(entry.text).or_insert(entry.options);
            }
        }
        Ok(Self { backends, options })
    }

    /// A backend's machine-wide defaults, empty if its line carried no body.
    pub fn options(&self, backend: &str) -> Option<&Options> {
        self.options.get(backend)
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
        "apt", "dnf", "pacman", "zypper", "apk", "xbps", "yay", "paru", "winget", "scoop", "choco",
        "brew", "nix", "flatpak", "snap",
    ];

    // `service:` and `link:` are dependent STATEMENTS, not package managers: they never
    // gate on `priority` and never resolve a bare name, so listing them in the file that
    // orders package managers is noise (S14). Everything else that installs by explicit
    // spec — `web`, `github`, `appimage` — stays, because the model refuses an explicit
    // `web:…` unless `web` is listed.
    const NOT_A_MANAGER: &[&str] = &["service", "link"];

    let rank = |b: &str| -> usize {
        if b == "pip" {
            return 2;
        }
        if SYSTEM.contains(&b) {
            return 0;
        }
        1
    };

    let mut out: Vec<String> = available
        .iter()
        .filter(|b| !NOT_A_MANAGER.contains(&b.as_str()))
        .cloned()
        .collect();
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
            family: "debian".into(),
            vars: Default::default(),
        }
    }

    fn parse(body: &str) -> Result<Priority> {
        Priority::parse(&PathBuf::from("priority"), body, &facts())
    }

    #[test]
    fn a_variable_block_chooses_a_backend() {
        let mut f = facts();
        f.vars.insert(
            "role".into(),
            crate::model::vars::Value::parse_literal("travel"),
        );
        let p = Priority::parse(
            &PathBuf::from("priority"),
            "apt\nwhen $role == travel {\n  cargo\n}\n",
            &f,
        )
        .unwrap();
        assert_eq!(p.order(), ["apt", "cargo"]);
    }

    #[test]
    fn the_bootstrap_pass_names_every_backend_and_evaluates_nothing() {
        // It runs before variables exist, so a predicate it cannot answer must not be an
        // error — and the superset it returns is a vocabulary for parsing `vars`, never an
        // order. `dnf` is inside a block that will not match; it is still named here.
        let p = Priority::every_backend(
            &PathBuf::from("priority"),
            "apt\nwhen $role == travel {\n  cargo\n}\nwhen os == plan9 {\n  dnf\n}\n",
        )
        .unwrap();
        assert_eq!(p.order(), ["apt", "cargo", "dnf"]);
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
    fn a_backend_line_may_carry_its_defaults() {
        // VIII.2: `formats` in `priority` is the machine-wide default, and it is an options
        // body on the backend line rather than a new file or a new block kind.
        let p = parse("apt\ngithub {\n  formats = deb\n  formats = tarball\n}\n").unwrap();
        assert_eq!(p.order(), ["apt", "github"]);
        assert_eq!(
            p.options("github").unwrap().all("formats"),
            ["deb", "tarball"]
        );
    }

    #[test]
    fn a_body_listing_a_backend_also_enables_it() {
        // D7: listed = available (V.15). A body is still a listing, so a user who writes
        // only a formats block has enabled the backend — one list, one question.
        let p = parse("github {\n  formats = deb\n}\n").unwrap();
        assert!(p.allows("github"));
    }

    #[test]
    fn a_body_inside_a_when_block_applies_only_there() {
        let body = "when family == debian {\n  github {\n    formats = deb\n  }\n}\n";
        assert_eq!(
            parse(body)
                .unwrap()
                .options("github")
                .unwrap()
                .all("formats"),
            ["deb"]
        );

        let body = "when family == arch {\n  github {\n    formats = deb\n  }\n}\n";
        assert!(parse(body).unwrap().options("github").is_none());
    }

    #[test]
    fn the_when_arm_that_matched_beats_the_plain_line_below_it() {
        // Same first-mention-wins rule the order uses, so the two cannot disagree.
        let body = "when family == debian {\n  github {\n    formats = deb\n  }\n}\n\
                    github {\n  formats = binary\n}\n";
        assert_eq!(
            parse(body)
                .unwrap()
                .options("github")
                .unwrap()
                .all("formats"),
            ["deb"]
        );
    }

    #[test]
    fn an_option_the_backend_cannot_read_is_an_error_here_too() {
        // VIII.4: silently ignoring an option is how a config grows lines that do nothing.
        let err = parse("apt {\n  formats = deb\n}\n").unwrap_err();
        assert!(err.to_string().contains("apt"), "{}", err);

        let err = parse("github {\n  channel = stable\n}\n").unwrap_err();
        assert!(err.to_string().contains("channel"), "{}", err);
    }

    #[test]
    fn an_unknown_format_in_a_body_is_an_error_naming_the_real_ones() {
        let err = parse("github {\n  formats = nonsense\n}\n").unwrap_err();
        assert!(err.to_string().contains("nonsense"), "{}", err);
        assert!(err.to_string().contains("tarball"), "{}", err);
    }

    #[test]
    fn a_body_line_without_an_equals_is_an_error() {
        assert!(parse("github {\n  formats deb\n}\n").is_err());
    }

    #[test]
    fn an_unclosed_body_is_an_error_naming_the_backend() {
        let err = parse("github {\n  formats = deb\n").unwrap_err();
        assert!(err.to_string().contains("github"), "{}", err);
    }

    #[test]
    fn a_bare_name_still_carries_no_options() {
        assert!(parse("apt\ncargo\n").unwrap().options("apt").is_none());
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
    fn service_and_link_are_not_listed_but_artifact_backends_are() {
        // S14: `service`/`link` are dependent statements, not package managers, so they
        // are noise in `priority`. `web`/`github` install by explicit spec, which the
        // model refuses unless the backend is listed — so they stay.
        let out = starter_order(&[
            "apt".into(),
            "service".into(),
            "link".into(),
            "web".into(),
            "github".into(),
        ]);
        assert!(!out.contains(&"service".to_string()), "{:?}", out);
        assert!(!out.contains(&"link".to_string()), "{:?}", out);
        assert!(out.contains(&"web".to_string()), "{:?}", out);
        assert!(out.contains(&"github".to_string()), "{:?}", out);
        assert_eq!(out.first().map(String::as_str), Some("apt"));
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
