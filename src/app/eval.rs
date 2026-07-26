//! `linix eval` — the resolved desired state, as data (XIII.15, U17).
//!
//! Everything else in LiNix answers a question about the machine. This answers the question
//! *what did my configuration actually resolve to* — every `when` decided, every bare name
//! given a backend, every variable substituted — without looking at the machine at all.
//!
//! **Versioned from the first release (U17, ruled 2026-07-24).** It is the one output that will
//! acquire consumers LiNix cannot see, and P2 leaves no legacy to carry: the version is free
//! now and impossible to add later without breaking whatever grew up around it.
//!
//! **It takes no locks and changes nothing.** A command that reads has no business serialising
//! against one that writes, and a command people will put in a pipeline must be safe to run
//! while a sync is in flight.

use serde::Serialize;

/// The schema version of `linix eval`'s output.
///
/// Bump when a consumer would break: a field removed, renamed, or given a new meaning. Adding a
/// field is not a bump — a consumer that ignores unknown keys keeps working, and one that does
/// not was never going to survive any change.
pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Evaluation {
    /// The schema this document is written to. First field, so a consumer can branch on it
    /// before reading anything whose meaning depends on it.
    pub schema: u32,
    /// The variables every `when` was decided against (Part IX), with their types intact — a
    /// `Num` stays a number, so a consumer is not handed `"3"` and left to guess.
    pub vars: std::collections::BTreeMap<String, crate::model::vars::Value>,
    /// What must be installed, resolved: a backend for every name, in a stable order.
    pub present: Vec<ResolvedPackage>,
    /// What must not exist (`absent:`).
    pub absent: Vec<ResolvedPackage>,
    /// The non-package declarations — `service:nginx`, `firewall:22/tcp`.
    pub extras: Vec<ResolvedExtra>,
    /// Lines whose `@until` has passed (II.16). They are not in `present`, and a consumer that
    /// only read `present` would never learn that a declaration silently stopped counting.
    pub lapsed: Vec<Lapsed>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedPackage {
    pub backend: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The file and line that declared it. The whole value of this output is being able to ask
    /// "why is this here", and an answer with no origin cannot be checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedExtra {
    pub kind: String,
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Lapsed {
    pub line: String,
    pub source: String,
}

/// A `file:line` origin as this document reports it: relative to the config repo, with forward
/// slashes.
///
/// Two evaluations are meant to be diffed — the same repo on two machines, or before and after
/// an edit. An absolute path makes every line differ because the repo is checked out somewhere
/// else, and a backslash makes every line differ because one machine is Windows. Neither
/// difference is one the configuration made, so neither belongs in the output.
fn repo_relative(source: &str, root: &std::path::Path) -> String {
    let (path, line) = match source.rsplit_once(':') {
        // A drive letter is not a line number: `C:/x` must not split into `C` and `/x`.
        Some((p, n)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => (p, Some(n)),
        _ => (source, None),
    };
    let rel = std::path::Path::new(path)
        .strip_prefix(root)
        .unwrap_or_else(|_| std::path::Path::new(path))
        .to_string_lossy()
        .replace('\\', "/");
    match line {
        Some(n) => format!("{}:{}", rel, n),
        None => rel,
    }
}

impl Evaluation {
    /// Build the document from a resolved model.
    ///
    /// Sorted throughout: a consumer diffing two evaluations wants the difference to be the
    /// configuration's, not the order a `HashMap` happened to iterate in.
    pub fn of(state: &crate::model::DesiredState, config_root: &std::path::Path) -> Evaluation {
        let package = |p: &crate::core::PackageSpec| ResolvedPackage {
            backend: p.backend.clone(),
            name: p.name.clone(),
            version: p.options.get("version").cloned(),
            source: p
                .options
                .get("__source")
                .map(|s| repo_relative(s, config_root)),
        };

        let mut present: Vec<ResolvedPackage> = state.present().map(package).collect();
        let mut absent: Vec<ResolvedPackage> = state.absent().map(package).collect();
        present.sort();
        absent.sort();

        // Only statements with a keyword: a package line's prefix is a backend, and it is
        // already reported above under `present`/`absent`.
        let mut extras: Vec<ResolvedExtra> = state
            .extras
            .iter()
            .filter_map(|(stmt, origin)| {
                Some(ResolvedExtra {
                    kind: stmt.kind()?.to_string(),
                    name: stmt.subject()?,
                    source: repo_relative(&origin.to_string(), config_root),
                })
            })
            .collect();
        extras.sort();

        let mut lapsed: Vec<Lapsed> = state
            .lapsed
            .iter()
            .map(|(line, origin)| Lapsed {
                line: line.clone(),
                source: repo_relative(&origin.to_string(), config_root),
            })
            .collect();
        lapsed.sort();

        Evaluation {
            schema: SCHEMA,
            vars: state.vars.clone(),
            present,
            absent,
            extras,
            lapsed,
        }
    }

    /// The document as the bytes `linix eval` writes.
    ///
    /// Pretty-printed with a trailing newline: this output is read by people at least as often
    /// as by programs, and a JSON document without a final newline is the kind of thing that
    /// makes a shell prompt land in the middle of it.
    pub fn render(&self) -> crate::core::Result<String> {
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| crate::core::Error::Other(format!("serializing the evaluation: {}", e)))?;
        Ok(format!("{}\n", body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::grammar::Origin;
    use std::path::Path;

    /// The version is the contract. A consumer reads it before anything else, so it must be
    /// present and first — and it must not move without someone meaning to move it.
    #[test]
    fn the_document_is_versioned() {
        let doc = Evaluation::of(&crate::model::DesiredState::default(), Path::new("/repo"));
        assert_eq!(doc.schema, SCHEMA);
        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.starts_with("{\"schema\":1"), "{}", json);
    }

    #[test]
    fn an_empty_configuration_evaluates_to_an_empty_document() {
        let doc = Evaluation::of(&crate::model::DesiredState::default(), Path::new("/repo"));
        assert!(doc.present.is_empty() && doc.absent.is_empty() && doc.extras.is_empty());
        // ...and still serialises, so a consumer never has to special-case "nothing".
        assert!(serde_json::to_string(&doc).is_ok());
    }

    fn spec(backend: &str, name: &str, present: bool) -> crate::core::PackageSpec {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "__source".to_string(),
            format!("/repo/modules/x.txt:{}", name.len()),
        );
        crate::core::PackageSpec {
            name: name.into(),
            backend: backend.into(),
            options,
            present,
            ..Default::default()
        }
    }

    #[test]
    fn present_and_absent_are_separate_and_sorted() {
        let mut state = crate::model::DesiredState::default();
        state.packages.insert(
            "apt".into(),
            vec![
                spec("apt", "zsh", true),
                spec("apt", "acl", true),
                spec("apt", "nano", false),
            ],
        );
        let doc = Evaluation::of(&state, Path::new("/repo"));

        let names: Vec<&str> = doc.present.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["acl", "zsh"], "present must be sorted");
        assert_eq!(doc.absent.len(), 1);
        assert_eq!(doc.absent[0].name, "nano");
    }

    /// The output is for diffing, so the same configuration must serialise identically twice —
    /// otherwise every comparison is noise.
    #[test]
    fn the_same_state_serialises_identically_twice() {
        let mut state = crate::model::DesiredState::default();
        state.packages.insert(
            "apt".into(),
            vec![spec("apt", "a", true), spec("apt", "b", true)],
        );
        state
            .packages
            .insert("cargo".into(), vec![spec("cargo", "c", true)]);
        let a = serde_json::to_string(&Evaluation::of(&state, Path::new("/repo"))).unwrap();
        let b = serde_json::to_string(&Evaluation::of(&state, Path::new("/repo"))).unwrap();
        assert_eq!(a, b);
    }

    /// Every package carries where it came from: the point of this output is answering "why is
    /// this here", and an answer with no origin cannot be checked.
    #[test]
    fn every_package_names_the_line_that_declared_it() {
        let mut state = crate::model::DesiredState::default();
        state
            .packages
            .insert("apt".into(), vec![spec("apt", "jq", true)]);
        let doc = Evaluation::of(&state, Path::new("/repo"));
        assert!(
            doc.present[0]
                .source
                .as_deref()
                .is_some_and(|s| s.contains("modules/x.txt")),
            "{:?}",
            doc.present[0]
        );
    }

    /// An extra is reported by kind and subject, not by re-splitting its key on `:` — which
    /// `firewall:22/tcp` and `setting:org.gnome.desktop/x` would both survive, and
    /// `repo:apt:ppa:x/y` would not.
    #[test]
    fn extras_split_into_kind_and_subject() {
        use crate::config::grammar::{Options, Statement};
        let state = crate::model::DesiredState {
            extras: vec![
                (
                    Statement::Firewall("22/tcp".into(), Options::default()),
                    Origin::new("/repo/modules/net.txt", 3),
                ),
                (
                    Statement::Repo {
                        backend: "apt".into(),
                        spec: "ppa:x/y".into(),
                    },
                    Origin::new("/repo/modules/net.txt", 1),
                ),
            ],
            ..Default::default()
        };
        let doc = Evaluation::of(&state, Path::new("/repo"));
        let by_kind = |k: &str| {
            doc.extras
                .iter()
                .find(|e| e.kind == k)
                .unwrap_or_else(|| panic!("{} missing from {:?}", k, doc.extras))
        };
        assert_eq!(by_kind("firewall").name, "22/tcp");
        assert_eq!(by_kind("repo").name, "apt:ppa:x/y");
        assert_eq!(by_kind("repo").source, "modules/net.txt:1");
    }

    /// A lapsed line is not in `present`, so a consumer reading only `present` would never
    /// learn a declaration had quietly stopped counting (II.16).
    #[test]
    fn lapsed_lines_are_reported() {
        let state = crate::model::DesiredState {
            lapsed: vec![(
                "apt:temp-tool @until=2020-01-01".into(),
                Origin::new("/repo/modules/x.txt", 9),
            )],
            ..Default::default()
        };
        let doc = Evaluation::of(&state, Path::new("/repo"));
        assert_eq!(doc.lapsed.len(), 1);
        assert_eq!(doc.lapsed[0].source, "modules/x.txt:9");
    }

    /// A variable keeps its type. Flattening to strings would hand a consumer `"3"` and
    /// `"true"` and leave it guessing, which is the thing `Value` exists to prevent.
    #[test]
    fn variables_keep_their_types() {
        use crate::model::vars::Value;
        let mut state = crate::model::DesiredState::default();
        state.vars.insert("gpu".into(), Value::Bool(true));
        state.vars.insert("cores".into(), Value::Num(8.0));
        state.vars.insert("host".into(), Value::Str("aria".into()));
        let json = Evaluation::of(&state, Path::new("/repo")).render().unwrap();
        assert!(json.contains("\"gpu\": true"), "{}", json);
        assert!(json.contains("\"cores\": 8.0"), "{}", json);
        assert!(json.contains("\"host\": \"aria\""), "{}", json);
    }

    /// Sources are reported relative to the repo, with forward slashes — the two differences
    /// that would otherwise make every line of a cross-machine diff differ for no reason the
    /// configuration is responsible for.
    #[test]
    fn sources_are_repo_relative_and_forward_slashed() {
        assert_eq!(
            repo_relative(r"C:\repo\modules\x.txt:7", Path::new(r"C:\repo")),
            "modules/x.txt:7"
        );
        assert_eq!(
            repo_relative(
                "/home/a/.config/linix/modules/x.txt:7",
                Path::new("/home/a/.config/linix")
            ),
            "modules/x.txt:7"
        );
        // A file-level origin has no line, and must not grow one.
        assert_eq!(
            repo_relative(r"C:\repo\active", Path::new(r"C:\repo")),
            "active"
        );
    }

    /// A drive letter is not a line number. Splitting `C:/repo/x` on the last `:` would be
    /// right on Linux and wrong on Windows, so the split tests what it found.
    #[test]
    fn a_drive_letter_is_not_mistaken_for_a_line_number() {
        assert_eq!(repo_relative("C:/x.txt", Path::new("/nowhere")), "C:/x.txt");
    }

    /// A path outside the repo is left absolute rather than mangled into a relative path that
    /// would name a different file. `preferences.toml` on the machine is not in the repo.
    #[test]
    fn a_source_outside_the_repo_stays_absolute() {
        let out = repo_relative("/etc/linix/machine.toml:2", Path::new("/home/a/repo"));
        assert_eq!(out, "/etc/linix/machine.toml:2");
    }

    /// Rendered output ends in a newline — a JSON document without one puts the next shell
    /// prompt in the middle of it.
    #[test]
    fn the_rendered_document_ends_in_a_newline() {
        let out = Evaluation::of(&crate::model::DesiredState::default(), Path::new("/repo"))
            .render()
            .unwrap();
        assert!(out.ends_with("}\n"), "{:?}", out);
    }
}
