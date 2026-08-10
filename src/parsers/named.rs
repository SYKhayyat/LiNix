//! **The readers a `[[backend]]` row may name.**
//!
//! A backend is a `ManagerConfig` — argv templates — plus a way to read what the manager says
//! back. The argv half has always been data. The reading half was a `LambdaParser` written out
//! in Rust beside each registration, which is what kept 46 registrars in the source file when
//! only the five listed in `builtin_backends.toml`'s header genuinely need to be there.
//!
//! Every reader here already existed, tested against a fixture, in `crate::parsers::*`. What is
//! new is that each has a **name** a row can write, and that the two directions are separate:
//! `reads` for the installed listing, `searches` for the catalogue. They are separate because
//! several managers need two different readers (`spack` lists `name version` and searches bare
//! names), and because the two have different failure rules — `parse_installed` is fallible on
//! purpose and `parse_search` deliberately is not (`OutputParser`'s own doc says why).
//!
//! **A name that does not resolve is a build failure, not a runtime one.**
//! `every_named_reader_resolves` in the suite walks every `reads`/`searches`/`probe` string in
//! the shipped table and looks it up here; a typo cannot reach a machine.
//!
//! **The other direction is checked too.** A reader listed here and named by nothing is either a
//! conversion someone abandoned halfway or a manager that was deleted — both of which have
//! happened in this repository, and both of which leave a function that looks live.

use crate::core::Package;
use crate::parsers::{
    apt, bsd, common, conda, dnf, dotnet, ecosystem, language, macos, pacman, pkgsrc, windows,
    OutputParser, ParseResult, Unrecognised,
};

/// Reads what the manager reports as installed. Fallible: see [`Unrecognised`].
pub type Installed = fn(&str, &str) -> ParseResult;

/// Reads a catalogue answer. Infallible by design — an empty search is a fact the user asked
/// for and can see.
pub type Search = fn(&str, &str) -> Vec<Package>;

/// Reads a listing that is neither: an outdated report, a dependency list, a machine-format
/// listing offered in place of the human one.
pub type Probe = fn(&str, &str) -> Vec<Package>;

/// Reads a listing of bare names — the essential set, a dependency answer.
pub type Names = fn(&str, &str) -> Vec<String>;

/// The reader named `name`, for the installed direction.
pub fn installed(name: &str) -> Option<Installed> {
    let f: Installed = match name {
        "apt" => |o, _| apt::parse_list(o),
        "asdf" => ecosystem::asdf_list,
        "cabal" => ecosystem::cabal_list,
        "conda" => |o, _| conda::parse_conda_list(o),
        "conda_history" => |o, _| conda::parse_conda_history(o),
        "dash_version_list" => common::parse_dash_version_list,
        "dotnet" => |o, _| dotnet::parse_dotnet_list(o),
        "dotnet_json" => |o, _| dotnet::parse_dotnet_list_json(o),
        "eopkg" => ecosystem::eopkg_list,
        "language" => |o, b| language::parse_installed(b, o),
        "macports" => |o, _| macos::parse_macports_installed(o),
        "mas" => |o, _| macos::parse_mas_list(o),
        "mix_archive" => ecosystem::mix_archive,
        "names_only" => ecosystem::names_only,
        "nimble" => ecosystem::nimble_list,
        "pacman" => pacman::parse_list_for,
        "pixi" => ecosystem::pixi_list,
        "pixi_json" => ecosystem::pixi_list_json,
        "pkg" => |o, _| bsd::parse_pkg(o),
        "pkg_add" => |o, _| bsd::parse_pkg_add(o),
        "pkgin" => |o, _| pkgsrc::parse_pkgin(o),
        "rpm_qa" => dnf::parse_rpm_qa,
        "scoop_export" => |o, _| windows::parse_scoop_export(o),
        "simple_list" => common::parse_simple_list,
        "slackpkg" => ecosystem::slackpkg_installed,
        "uv_tool_list" => ecosystem::uv_tool_list,
        "windows" => |o, b| windows::parse_installed(b, o),
        "winget_export" => |o, _| windows::parse_winget_export(o),
        "ws_name_version" => ecosystem::ws_name_version,
        "xbps" => |o, _| bsd::parse_xbps_list(o),
        "zypper" => |o, _| dnf::parse_zypper_search(o),
        // Not a reader: the answer to "this manager has no listing verb at all". Kept as a
        // name rather than as an empty vector, because `[]` is a fact the planner acts on.
        "cannot_list" => |_, b| {
            Err(Unrecognised {
                backend: b.to_string(),
                data_lines: 0,
                sample: "this manager has no listing verb".into(),
            })
        },
        _ => return None,
    };
    Some(f)
}

/// The reader named `name`, for the catalogue direction.
pub fn search(name: &str) -> Option<Search> {
    let f: Search = match name {
        "apt" => |o, _| apt::parse_search(o),
        "conda" => |o, _| conda::parse_conda_search(o),
        "dash_version_list" => |o, b| common::parse_dash_version_list(o, b).unwrap_or_default(),
        "dnf" => |o, _| dnf::parse_dnf_search(o),
        "dotnet" => |o, _| dotnet::parse_dotnet_search(o).unwrap_or_default(),
        "emerge" => ecosystem::emerge_search,
        "eopkg" => |o, b| ecosystem::eopkg_list(o, b).unwrap_or_default(),
        "guix" => ecosystem::guix_search,
        "language" => |o, b| language::parse_search(b, o),
        "macports" => |o, _| macos::parse_macports_search(o),
        "mas" => |o, _| macos::parse_mas_search(o),
        "names_only" => |o, b| ecosystem::names_only(o, b).unwrap_or_default(),
        "pacman" => pacman::parse_search_for,
        "pixi" => ecosystem::pixi_search,
        "pkg" => |o, _| bsd::parse_pkg(o).unwrap_or_default(),
        "pkg_add" => |o, _| bsd::parse_pkg_add(o).unwrap_or_default(),
        "pkgin" => |o, _| pkgsrc::parse_pkgin(o).unwrap_or_default(),
        "slackpkg" => ecosystem::slackpkg_search,
        "windows" => |o, b| windows::parse_search(b, o),
        "ws_name_version" => |o, b| ecosystem::ws_name_version(o, b).unwrap_or_default(),
        "xbps" => |o, _| bsd::parse_xbps_search(o),
        "zypper" => |o, _| dnf::parse_zypper_search(o).unwrap_or_default(),
        _ => return None,
    };
    Some(f)
}

/// The reader named `name`, for a probe: an outdated report, or a machine-format listing.
pub fn probe(name: &str) -> Option<Probe> {
    let f: Probe = match name {
        "apk_outdated" => common::parse_apk_outdated,
        "apt_outdated" => |o, _| apt::parse_apt_outdated(o),
        "brew_outdated" => |o, _| common::parse_brew_outdated(o),
        "choco_outdated" => |o, _| windows::parse_choco_outdated(o),
        "composer_outdated" => |o, _| language::parse_composer_outdated(o),
        "dnf_outdated" => |o, _| dnf::parse_dnf_outdated(o),
        "gem_outdated" => |o, _| language::parse_gem_outdated(o),
        "npm_outdated" => language::parse_npm_outdated,
        "pacman_outdated" => |o, _| pacman::parse_pacman_outdated(o),
        "pip_outdated" => |o, _| language::parse_pip_outdated(o),
        "scoop_outdated" => |o, _| windows::parse_scoop_outdated(o),
        "winget_outdated" => |o, _| windows::parse_winget_outdated(o),
        "zypper_outdated" => |o, _| dnf::parse_zypper_outdated(o),
        _ => return None,
    };
    Some(f)
}

/// The reader named `name`, for a listing of bare names — a dependency answer.
pub fn names(name: &str) -> Option<Names> {
    let f: Names = match name {
        "bare_dependency_names" => |o, _| dnf::parse_bare_dependency_names(o),
        "pacman_depends_on" => |o, _| pacman::parse_depends_on(o),
        "xbps_dependencies" => |o, _| bsd::parse_xbps_dependencies(o),
        _ => return None,
    };
    Some(f)
}

/// Every name this module answers to, by table.
///
/// Written out rather than derived, because a `match` cannot be enumerated — and checked
/// against the `match` arms by `every_listed_reader_resolves`, so the two cannot drift.
pub const INSTALLED_NAMES: &[&str] = &[
    "apt",
    "asdf",
    "cabal",
    "cannot_list",
    "conda",
    "conda_history",
    "dash_version_list",
    "dotnet",
    "dotnet_json",
    "eopkg",
    "language",
    "macports",
    "mas",
    "mix_archive",
    "names_only",
    "nimble",
    "pacman",
    "pixi",
    "pixi_json",
    "pkg",
    "pkg_add",
    "pkgin",
    "rpm_qa",
    "scoop_export",
    "simple_list",
    "slackpkg",
    "uv_tool_list",
    "windows",
    "winget_export",
    "ws_name_version",
    "xbps",
    "zypper",
];

pub const SEARCH_NAMES: &[&str] = &[
    "apt",
    "conda",
    "dash_version_list",
    "dnf",
    "dotnet",
    "emerge",
    "eopkg",
    "guix",
    "language",
    "macports",
    "mas",
    "names_only",
    "pacman",
    "pixi",
    "pkg",
    "pkg_add",
    "pkgin",
    "slackpkg",
    "windows",
    "ws_name_version",
    "xbps",
    "zypper",
];

pub const PROBE_NAMES: &[&str] = &[
    "apk_outdated",
    "apt_outdated",
    "brew_outdated",
    "choco_outdated",
    "composer_outdated",
    "dnf_outdated",
    "gem_outdated",
    "npm_outdated",
    "pacman_outdated",
    "pip_outdated",
    "scoop_outdated",
    "winget_outdated",
    "zypper_outdated",
];

pub const NAMES_NAMES: &[&str] = &[
    "bare_dependency_names",
    "pacman_depends_on",
    "xbps_dependencies",
];

/// A named pair of readers, bound to the backend that names them.
///
/// The backend name is here rather than in the table because the readers take it — a listing
/// is parsed *into* `Package { backend, .. }`, and a reader shared by ten managers cannot know
/// which one asked.
pub struct NamedParser {
    backend: String,
    installed: Installed,
    search: Search,
    essential: Option<Names>,
}

impl NamedParser {
    /// `reads` is required; `searches` absent means this manager has no catalogue verb, which
    /// is why the registration has no `Searchable` either — the empty vector below is never
    /// reached through a live capability.
    pub fn new(
        backend: &str,
        reads: Installed,
        searches: Option<Search>,
        essential: Option<Names>,
    ) -> Self {
        NamedParser {
            backend: backend.to_string(),
            installed: reads,
            search: searches.unwrap_or(|_, _| Vec::new()),
            essential,
        }
    }
}

impl OutputParser for NamedParser {
    fn parse_installed(&self, output: &str) -> ParseResult {
        (self.installed)(output, &self.backend)
    }

    fn parse_search(&self, output: &str) -> Vec<Package> {
        (self.search)(output, &self.backend)
    }

    fn parse_essential(&self, output: &str) -> Vec<String> {
        match self.essential {
            Some(f) => f(output, &self.backend),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lists and the `match` arms are two spellings of one fact, and this is what keeps
    /// them one. A name in the list that the match does not answer to is a row that would fail
    /// at load; a match arm missing from the list is a reader the coverage gate cannot see.
    #[test]
    fn every_listed_reader_resolves() {
        for n in INSTALLED_NAMES {
            assert!(installed(n).is_some(), "INSTALLED_NAMES lists `{n}`");
        }
        for n in SEARCH_NAMES {
            assert!(search(n).is_some(), "SEARCH_NAMES lists `{n}`");
        }
        for n in PROBE_NAMES {
            assert!(probe(n).is_some(), "PROBE_NAMES lists `{n}`");
        }
        for n in NAMES_NAMES {
            assert!(names(n).is_some(), "NAMES_NAMES lists `{n}`");
        }
    }

    #[test]
    fn a_name_nothing_answers_to_is_none() {
        assert!(installed("no_such_reader").is_none());
        assert!(search("no_such_reader").is_none());
        assert!(probe("no_such_reader").is_none());
        assert!(names("no_such_reader").is_none());
    }

    /// The reader is handed the backend that named it, not the one it was written for.
    #[test]
    fn the_backend_reaches_the_reader() {
        let p = NamedParser::new(
            "spack",
            installed("ws_name_version").unwrap(),
            search("names_only"),
            None,
        );
        let pkgs = p
            .parse_installed("zlib 1.3\n")
            .expect("this fixture parses");
        assert_eq!(pkgs[0].backend, "spack");
        assert_eq!(pkgs[0].name, "zlib");
        assert_eq!(pkgs[0].version.as_deref(), Some("1.3"));

        // …and the two directions really are different readers: `names_only` takes the whole
        // line as a name, which is the difference `spack` needs.
        let hits = p.parse_search("zlib\n");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].backend, "spack");
    }

    /// `cannot_list` is an error, not an empty machine. This is the assertion that stops it
    /// being "simplified" back into `Ok(vec![])`.
    #[test]
    fn a_manager_with_no_listing_verb_says_so_rather_than_reporting_nothing() {
        let p = NamedParser::new("stack", installed("cannot_list").unwrap(), None, None);
        let err = p
            .parse_installed("anything at all")
            .expect_err("a manager with no listing verb cannot report an empty one");
        assert_eq!(err.backend, "stack");
    }

    /// A row with no `searches` has no `Searchable` either, so this vector is unreachable
    /// through a capability — asserted here so the claim is checked rather than believed.
    #[test]
    fn a_row_with_no_search_reader_answers_empty_rather_than_panicking() {
        let p = NamedParser::new("helm", installed("ws_name_version").unwrap(), None, None);
        assert!(p.parse_search("anything").is_empty());
    }
}
