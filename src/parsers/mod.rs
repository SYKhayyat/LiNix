pub mod apt;
pub mod bsd;
pub mod common;
pub mod conda;
pub mod dnf;
pub mod dotnet;
pub mod ecosystem;
pub mod language;
pub mod macos;
pub mod named;
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
             `{}`. Its output format has probably changed. This is not read as an empty \
             machine: that reading would plan every declared package as a fresh install and \
             drop every removal.",
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

/// The JSON document inside a manager's output, which is not always the whole of it.
///
/// **A `--json` flag buys the shape of the answer, not sole possession of the stream.**
/// composer prints `Changed current directory to /root/.composer` ahead of every global
/// command whenever a global config dir exists, which is every machine that has ever run
/// `composer global`. Parsed from byte zero that is a syntax error, and a reader that answers
/// `unwrap_or_default()` to a syntax error reports an empty machine — install everything, own
/// nothing. The same shape reaches pip behind a proxy warning, yarn behind node's deprecation
/// notice, and conda behind its own.
///
/// Reading stops at the end of the first value, so a manager that prints a summary line *after*
/// its document is read the same way: trailing bytes are not the reader's business.
pub fn json_document(output: &str) -> Option<serde_json::Value> {
    fn parse_from(text: &str) -> Option<serde_json::Value> {
        serde_json::Deserializer::from_str(text)
            .into_iter::<serde_json::Value>()
            .next()?
            .ok()
    }
    // The first bracket byte, which is where the document starts on every real banner seen so
    // far. Only if that fails does the first bracket-opening *line* get a turn: a later bracket
    // byte inside a failed document would parse as some sub-object and answer confidently with
    // the wrong half of the tree, so the second attempt is anchored to a line start, not to the
    // next brace along.
    if let Some(start) = output.find(['{', '[']) {
        if let Some(v) = parse_from(&output[start..]) {
            return Some(v);
        }
    }
    let mut offset = 0;
    for line in output.split_inclusive('\n') {
        if line.starts_with(['{', '[']) {
            if let Some(v) = parse_from(&output[offset..]) {
                return Some(v);
            }
        }
        offset += line.len();
    }
    None
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
///
/// **This is for text listings. A `--json` reader must use [`or_unrecognised_json`]** — counting
/// lines of a structured answer asks the wrong question, and the arm that used to paper over
/// that here made the answer *unconditionally* `Ok` for anything that parsed as JSON, which
/// disabled the check for the five backends that reached it.
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

/// The same judgement for a structured answer, where the question is the container and not the
/// lines.
///
/// `container` is how many entries the document offered this reader — `Some(n)` when the shape
/// was found, `None` when it was not there at all. The three answers it separates:
///
/// - **`Some(0)`** — the manager says the machine has none. `pipx list --json` on an empty box
///   prints `{"pipx_spec_version": "0.1", "venvs": {}}`, and that is a true and common answer.
/// - **`Some(n)` with nothing read** — `n` entries the reader could not get a name out of. That
///   is a schema change: npm renaming `dependencies`, pip capitalising `name`. Refuse.
/// - **`None`** — the document did not have the shape at all, which includes not being a
///   document. Refuse. This is the case a length alone cannot express, and the one that made
///   `Some(0)` too tempting to write as `0`.
///
/// **Reading an unrecognised shape as an empty machine installs everything and owns nothing**:
/// every declaration becomes an install, every removal is dropped, and nothing in the run
/// believes anything failed.
pub fn or_unrecognised_json(
    backend: &str,
    found: Vec<Package>,
    container: Option<usize>,
    what: &str,
    output: &str,
) -> ParseResult {
    if !found.is_empty() || container == Some(0) {
        return Ok(found);
    }
    Err(Unrecognised {
        backend: backend.to_string(),
        // A shape that was never found has no entry count, so fall back to the size of the
        // answer — a refusal that says "0 lines" about a screenful of output reads as a bug in
        // the refusal.
        data_lines: container
            .unwrap_or_else(|| output.lines().filter(|l| !l.trim().is_empty()).count()),
        sample: format!(
            "{what}: {}",
            output.trim().chars().take(100).collect::<String>()
        ),
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

    /// Verbatim shape of `composer global show --format=json` on a machine with a global config
    /// dir, which is every machine that has ever run `composer global`.
    const COMPOSER_WITH_BANNER: &str = concat!(
        "Changed current directory to /root/.composer\n",
        r#"{"installed":[{"name":"psr/log","version":"1.1.4"}]}"#,
        "\n"
    );

    /// The three answers a structured reader has to keep apart, asserted as three.
    ///
    /// The arm this replaces lived inside `or_unrecognised` and returned `Ok(found)` — empty —
    /// for anything whose output held a parseable JSON document, whether or not the reader had
    /// got a single package out of it. That is `Some(n)` and `None` both collapsed into
    /// `Some(0)`, which is the collapse the whole `LX-1` type exists to prevent.
    #[test]
    fn the_container_says_which_of_the_three_answers_this_is() {
        let pkg = || vec![Package::new("jq", "test")];

        // Read something: always fine, whatever the container said.
        assert!(or_unrecognised_json("test", pkg(), Some(9), "what", "out").is_ok());
        assert!(or_unrecognised_json("test", pkg(), None, "what", "out").is_ok());

        // An empty container is an empty machine, which is a true and common answer.
        assert_eq!(
            or_unrecognised_json("test", vec![], Some(0), "what", "out").expect("empty is empty"),
            vec![]
        );

        // Entries offered, none read: a schema change.
        let err = or_unrecognised_json("test", vec![], Some(4), "no `name` in any entry", "out")
            .expect_err("four unread entries is not an empty machine");
        assert_eq!(err.data_lines, 4, "the count is of entries, not of lines");
        assert!(err.sample.starts_with("no `name` in any entry"));

        // No container at all: also a schema change, and the one a length cannot express.
        let err = or_unrecognised_json("test", vec![], None, "no `data` key", "a\nb\n\nc\n")
            .expect_err("a missing container is not an empty machine");
        assert_eq!(
            err.data_lines, 3,
            "with no entry count to report, the refusal falls back to the size of the answer"
        );
    }

    /// composer prints `Changed current directory to /root/.composer` ahead of every global
    /// command, and a `--json` reader that starts at byte zero reads that as a syntax error and
    /// answers "nothing installed" about a full machine.
    #[test]
    fn a_banner_above_the_document_does_not_hide_it() {
        let composer = COMPOSER_WITH_BANNER;
        let doc = json_document(composer).expect("the document below the banner");
        assert_eq!(doc["installed"][0]["name"], "psr/log");

        // The same output read from byte zero, which is what every one of these readers did.
        assert!(
            serde_json::from_str::<serde_json::Value>(composer).is_err(),
            "if this ever parses, the fixture stopped reproducing the bug"
        );
    }

    /// The other half: a manager that prints its summary *after* the document. `from_str`
    /// rejects trailing bytes, so this failed for the same reason at the other end.
    #[test]
    fn a_note_below_the_document_does_not_hide_it_either() {
        let doc = json_document(concat!(
            r#"[{"name":"jq","version":"1.7"}]"#,
            "\ndone in 0.3s\n"
        ))
        .expect("the document above the note");
        assert_eq!(doc[0]["name"], "jq");
    }

    /// A banner that itself contains a brace. The first bracket byte fails, and the retry is
    /// anchored to a line start rather than to the next brace along — because the next brace
    /// along is inside the document, and parsing from there would answer confidently with one
    /// sub-object of the tree.
    #[test]
    fn a_brace_in_the_banner_does_not_get_read_as_the_document() {
        let out = concat!(
            "Changed current directory to /root/{conf}\n",
            r#"{"installed":[{"name":"psr/log","version":"1.1.4"}]}"#,
            "\n"
        );
        let doc = json_document(out).expect("the real document, not the brace in the path");
        assert_eq!(doc["installed"][0]["name"], "psr/log");
    }

    /// No document is `None`, which is what lets a caller tell "unreadable" from "empty".
    /// Returning `Value::Null` here is the shape that made a format change look like a bare
    /// machine, so it must not be spelled that way.
    #[test]
    fn output_with_no_document_is_none_and_not_null() {
        assert_eq!(json_document("error: could not connect\n"), None);
        assert_eq!(json_document(""), None);
        // An unterminated document is not a document.
        assert_eq!(json_document(r#"{"installed": ["#), None);
        // And an empty one is still one.
        assert_eq!(json_document("{}"), Some(serde_json::json!({})));
    }
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
