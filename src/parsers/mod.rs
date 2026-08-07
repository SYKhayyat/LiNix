pub mod apt;
pub mod bsd;
pub mod common;
pub mod conda;
pub mod dnf;
pub mod dotnet;
pub mod ecosystem;
pub mod language;
pub mod macos;
pub mod pacman;
pub mod pkgsrc;
pub mod utils;
pub mod windows;

use crate::core::Package;

/// A parser read a manager's output and recognised nothing in it.
///
/// **This is not the same fact as an empty list, and the planner acts on the two in opposite
/// directions.** `Ok(vec![])` is the manager reporting an empty machine: every declaration
/// becomes an install and every drift removal is silently dropped. `Err` is the manager
/// answering in a shape this parser does not know — which is what a format change looks like
/// from in here — and the safe reading of that is *stop*, not *the machine is bare*.
///
/// `4d4a890` fixed this chain at four layers and named the fifth in its own diagnosis:
/// *"`Ok("")` → a parser finding nothing → `list_installed` answering `Ok(vec![])`. Nothing in
/// the chain believed anything had failed."* The parser was the link that could not be fixed
/// without changing a type. This is the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecognised {
    /// The manager whose output this was.
    pub backend: String,
    /// How many lines carried something the parser had treated as a candidate — not blank, not
    /// a separator, not a header it knows. This count is what makes the answer an error rather
    /// than an empty machine, and every parser already computed the set in order to skip it.
    pub data_lines: usize,
    /// The first such line, so a failure names the bytes nobody recognised instead of asking
    /// the reader to reproduce them.
    pub sample: String,
}

impl std::fmt::Display for Unrecognised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` answered with {} line(s) this parser does not recognise, the first being \
             `{}`. Its output format has probably changed. Refusing to read that as an empty \
             machine, which would plan every declared package as a fresh install and drop every \
             removal.",
            self.backend, self.data_lines, self.sample
        )
    }
}

/// What a parser answers: the packages, or the admission that it did not understand the bytes.
pub type ParseResult = std::result::Result<Vec<Package>, Unrecognised>;

/// A line of the manager's *prose* rather than of its data.
///
/// Two shapes, and both matter to the judgement above rather than to the packages. A heading
/// that introduces the list — *"The following ports are currently installed:"* — and a manager
/// saying it has none — *"No ports are installed."*, *"Nothing to list."* A package listing is
/// tokens; it does not end in a colon and it is not a sentence.
///
/// Getting this wrong is expensive in the direction nobody would notice: without it, every Mac
/// with MacPorts and no ports installed reads its own *"No ports are installed."* as one data
/// line yielding no package, and the parser calls a correct answer a format change.
pub fn is_prose_line(line: &str) -> bool {
    let t = line.trim();
    if t.ends_with(':') {
        return true;
    }
    if !t.ends_with('.') || t.split_whitespace().count() <= 2 {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    if lower.starts_with("no ") || lower.starts_with("nothing ") {
        return true;
    }
    // `choco list` ends with a count, and on a machine with nothing installed that count is
    // `0 packages installed.` — a sentence, and the correct answer. A package line never opens
    // with a bare number, because a number is not a name.
    t.split_whitespace()
        .next()
        .is_some_and(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
}

/// The lines of a manager's output that could carry a package — the default candidate set.
///
/// Blank lines and the manager's own prose are excluded, because neither is evidence that
/// anything went unread.
pub fn data_lines(clean: &str) -> Vec<&str> {
    clean
        .lines()
        .filter(|l| !l.trim().is_empty() && !is_prose_line(l))
        .collect()
}

/// The judgement `LX-1` is about, made in one place so sixty parsers make it the same way.
///
/// `candidates` is the set of lines the parser was willing to read a package out of, *before*
/// it tried. Finding none of them parseable is the failure; having none to try is an empty
/// machine, and a machine with nothing installed is a real and common answer.
///
/// Passing an empty `candidates` therefore always succeeds, which is deliberate: a manager that
/// prints only a header when it has nothing has told the truth, and a parser that called that
/// drift would refuse to run on a clean box.
pub fn or_unrecognised(backend: &str, found: Vec<Package>, candidates: &[&str]) -> ParseResult {
    if !found.is_empty() || candidates.is_empty() {
        return Ok(found);
    }
    Err(Unrecognised {
        backend: backend.to_string(),
        data_lines: candidates.len(),
        sample: candidates[0].trim().chars().take(120).collect(),
    })
}

pub trait OutputParser: Send + Sync {
    /// What this manager reports as installed.
    ///
    /// Fallible on purpose — see [`Unrecognised`]. `parse_search` below is deliberately **not**,
    /// and the asymmetry is the point rather than an oversight: a search that returns nothing is
    /// a fact the user asked for and can see, while an installed listing that returns nothing is
    /// a fact the *planner* acts on, invisibly, in the direction of installing everything and
    /// removing nothing.
    fn parse_installed(&self, output: &str) -> ParseResult;

    fn parse_search(&self, output: &str) -> Vec<Package>;

    /// Parses a manager's listing of packages the OS itself treats as essential — the
    /// ones removal must never touch, whatever a manifest says. Default: the manager
    /// exposes no such concept, so it reports none.
    fn parse_essential(&self, _output: &str) -> Vec<String> {
        Vec::new()
    }
}

/// Parses a listing of bare package names, one per line — the shape every manager that
/// can report its *explicit* set emits (`apt-mark showmanual`, `dnf repoquery
/// --userinstalled`, `xbps-query --list-manual-pkgs`, apk's `/etc/apk/world`). Versions
/// are absent by design; callers needing one reconcile against `list_installed`.
///
/// A trailing version constraint (`busybox>=1.36`) and a repository tag (`nodejs@edge`)
/// are stripped — apk's world file carries both. `!name` entries are conflict markers,
/// not installs, and are dropped.
///
/// An architecture qualifier is also stripped: `apt-mark showmanual` prints `libc6:i386`
/// on a multi-arch host, while `dpkg-query -W -f='${Package}'` prints the bare `libc6`.
/// Keeping the suffix would record a managed package whose name matches nothing the
/// installed-listing ever reports — permanent phantom drift, and a removal candidate that
/// can never be satisfied.
pub fn parse_bare_names(output: &str, backend: &str) -> ParseResult {
    let clean = crate::utils::text::sanitize(output);
    let candidates: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
        .collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let name = l.split(['>', '<', '=', '~', '@', ':', ' ']).next()?.trim();
            (!name.is_empty()).then(|| Package::new(name, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// A Functional Strategy Parser that allows injecting functions as data.
/// Used in backends/registry.rs to configure GenericManagers without
/// creating dozens of boilerplate structs.
pub struct LambdaParser {
    pub installed_fn: fn(&str) -> ParseResult,
    pub search_fn: fn(&str) -> Vec<Package>,
}

/// The parser for a manager that has no listing verb at all.
///
/// `stack` is the one: it installs and cannot enumerate. Before this existed the row read
/// `installed_fn: |_| vec![]` — character for character the most dangerous return in the
/// registry, because *"this machine has nothing"* is a fact the planner acts on and it was
/// standing in for *"this question cannot be asked"*. Naming the case is what stops the two
/// sharing a spelling; no compiler can help while both are a literal empty vector.
///
/// Inert today, since a manager with no listing gets no `Queryable`. Written down anyway,
/// because the next such manager will be added by someone reading that row.
pub struct CannotList(pub &'static str);

impl OutputParser for CannotList {
    fn parse_installed(&self, _output: &str) -> ParseResult {
        Err(Unrecognised {
            backend: self.0.to_string(),
            data_lines: 0,
            sample: "this manager has no listing verb".into(),
        })
    }

    fn parse_search(&self, _output: &str) -> Vec<Package> {
        Vec::new()
    }
}

impl OutputParser for LambdaParser {
    fn parse_installed(&self, output: &str) -> ParseResult {
        (self.installed_fn)(output)
    }
    fn parse_search(&self, output: &str) -> Vec<Package> {
        (self.search_fn)(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_names_parses_apt_mark_showmanual() {
        // `apt-mark showmanual` prints bare names, no versions — which is why the normal
        // apt list parser (which splits "name version") silently returned nothing.
        let pkgs = parse_bare_names("apt\nbase-files\njq\n", "apt").expect("this fixture parses");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["apt", "base-files", "jq"]);
        assert_eq!(pkgs[0].backend, "apt");
    }

    #[test]
    fn bare_names_strips_the_architecture_qualifier() {
        // showmanual prints `libc6:i386` on a multi-arch host while dpkg-query prints the
        // bare `libc6`. Keeping the suffix records a package nothing can ever match.
        let pkgs = parse_bare_names("libc6:i386\n", "apt").expect("this fixture parses");
        assert_eq!(pkgs[0].name, "libc6");
    }

    #[test]
    fn bare_names_handles_apk_world_entries() {
        // apk's world file carries version constraints, repo tags, comments, and `!`
        // conflict markers, which are not installs.
        let pkgs = parse_bare_names(
            "# comment\nbusybox>=1.36\nnodejs@edge\nbash=5.2\n!conflicting\n\ncurl\n",
            "apk",
        ).expect("this fixture parses");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["busybox", "nodejs", "bash", "curl"]);
    }
}
