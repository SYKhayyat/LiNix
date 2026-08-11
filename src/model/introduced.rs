//! When a declaration first appeared, and in which commit (XIII.19).
//!
//! **Git already knows.** The config repo is a git repo (II.1), every sync commits, and the
//! commit that first added a line is a fact already recorded — so the answer is a question put
//! to git, not a second history Shall writes at sync time and has to keep true. A store
//! recording "when did this line appear" would be a copy of git's answer that can disagree with
//! it, and the copy is always the one that is wrong.
//!
//! Pure: the argv, and parsing what git says. Running it is the caller's.

/// The commit that introduced a declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Introduced {
    pub commit: String,
    /// Author date, as git formatted it (`%ad` with `--date=short`).
    pub date: String,
    pub subject: String,
}

impl Introduced {
    /// One line for a human: `a1b2c3d 2026-03-14  add ripgrep for the new grep workflow`.
    pub fn summary(&self) -> String {
        format!("{}  {}  {}", self.short(), self.date, self.subject)
    }

    /// The abbreviated hash people actually type.
    pub fn short(&self) -> &str {
        let n = self.commit.len().min(7);
        &self.commit[..n]
    }
}

/// The separator between fields. A tab cannot appear in a commit hash or a `--date=short` date,
/// and a subject containing one still parses because the split is bounded to two.
const SEP: char = '\t';

/// Ask git which commits touched a line naming `needle`, oldest last.
///
/// `-S` is git's pickaxe: it finds commits where the *number of occurrences* of the string
/// changed, which is exactly "the commit that added this line" and not "every commit that
/// touched this file". `--` limits it to the config repo's declaration files, so a mention in
/// a README is not mistaken for a declaration.
pub fn argv(needle: &str, paths: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = [
        "log",
        "--format=%H\t%ad\t%s",
        "--date=short",
        "-S",
        needle,
        "--",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    out.extend(paths.iter().map(|p| p.to_string()));
    out
}

/// The commit that INTRODUCED the declaration, from `git log`'s output.
///
/// git lists newest first, so the introducing commit is the **last** line. Taking the first
/// would name the most recent commit that touched the line — which is the commit that last
/// edited it, a different and much less interesting fact, and one that would silently be right
/// whenever a line has only ever been touched once.
pub fn introduced_in(git_log: &str) -> Option<Introduced> {
    git_log
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .and_then(parse_line)
}

fn parse_line(line: &str) -> Option<Introduced> {
    let mut parts = line.splitn(3, SEP);
    let commit = parts.next()?.trim();
    let date = parts.next()?.trim();
    // A commit with an empty subject is legal, so this is the one field allowed to be missing.
    let subject = parts.next().unwrap_or("").trim();
    if commit.is_empty() || date.is_empty() {
        return None;
    }
    Some(Introduced {
        commit: commit.to_string(),
        date: date.to_string(),
        subject: subject.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "\
c3333333333333333333333333333333333333333\t2026-05-02\tpin ripgrep to 14.1
b2222222222222222222222222222222222222222\t2026-04-10\tmove ripgrep into tools
a1111111111111111111111111111111111111111\t2026-03-14\tadd ripgrep
";

    /// git lists newest first, so the INTRODUCING commit is the last line. Taking the first
    /// would name whoever last edited the line — a different fact, and one that looks correct
    /// whenever a line has only ever been touched once.
    #[test]
    fn the_oldest_commit_is_the_one_that_introduced_it() {
        let found = introduced_in(LOG).expect("a commit");
        assert_eq!(found.date, "2026-03-14");
        assert_eq!(found.subject, "add ripgrep");
        assert_eq!(found.short(), "a111111");
    }

    #[test]
    fn a_line_that_was_only_ever_added_once_still_works() {
        let one = "a1111111111111111111111111111111111111111\t2026-03-14\tadd ripgrep\n";
        assert_eq!(introduced_in(one).unwrap().subject, "add ripgrep");
    }

    /// A repo with no history for this line has no answer, and must not invent one.
    #[test]
    fn no_history_is_no_answer() {
        assert_eq!(introduced_in(""), None);
        assert_eq!(introduced_in("\n  \n"), None);
    }

    /// A subject containing a tab must not shift the fields — the split is bounded, so
    /// everything after the second tab is the subject, tabs and all.
    #[test]
    fn a_tab_in_the_subject_does_not_shift_the_fields() {
        let line = "a1111111111111111111111111111111111111111\t2026-03-14\tadd\tripgrep\n";
        let found = introduced_in(line).unwrap();
        assert_eq!(found.date, "2026-03-14");
        assert_eq!(found.subject, "add\tripgrep");
    }

    #[test]
    fn a_commit_with_an_empty_subject_is_not_a_parse_failure() {
        let line = "a1111111111111111111111111111111111111111\t2026-03-14\t\n";
        assert_eq!(introduced_in(line).unwrap().subject, "");
    }

    /// Garbage is not half-parsed into a plausible-looking commit.
    #[test]
    fn a_malformed_line_yields_nothing() {
        assert_eq!(introduced_in("not a log line\n"), None);
        assert_eq!(introduced_in("\t2026-03-14\tsubject\n"), None);
    }

    /// `-S` is the pickaxe: commits where the number of occurrences CHANGED. `git log <path>`
    /// would list every commit that touched the file, which for a busy module is all of them.
    #[test]
    fn the_query_uses_the_pickaxe_and_is_limited_to_the_given_paths() {
        let cmd = argv("ripgrep", &["modules", "profiles"]);
        assert!(cmd.iter().any(|a| a == "-S"), "{:?}", cmd);
        assert!(cmd.iter().any(|a| a == "ripgrep"), "{:?}", cmd);
        let sep = cmd
            .iter()
            .position(|a| a == "--")
            .expect("a path separator");
        assert_eq!(
            &cmd[sep + 1..],
            &["modules".to_string(), "profiles".to_string()]
        );
    }

    /// The date format is fixed, not the reader's locale: this string is parsed by the test
    /// above and read by a person, and `--date=short` is the one that is both.
    #[test]
    fn the_date_format_is_pinned() {
        assert!(argv("x", &[]).iter().any(|a| a == "--date=short"));
    }

    #[test]
    fn the_summary_reads_as_one_line() {
        let s = introduced_in(LOG).unwrap().summary();
        assert!(s.starts_with("a111111"), "{}", s);
        assert!(s.contains("2026-03-14"), "{}", s);
        assert!(s.contains("add ripgrep"), "{}", s);
    }
}
