//! Making a command's output safe to read, once, where it is read.
//!
//! **Why this is not in `parsers/`.** It lived there, and half the family never called it: all
//! sixteen table-driven backends sanitized, while the parsers hand-rolled inside
//! `src/backends/` mostly did not — `brew`, `nix`, `cargo`, `go`, `yarn` and `storage` parsed
//! raw output, with `flatpak` and `snap` the two exceptions that remembered. That is not a bug
//! anyone had hit, because these managers do not colour a pipe; it is thirty backends and one
//! rule with no single place to state it (`CLAUDE.md`).
//!
//! So it moved down a layer and moved to the boundary. `CommandExecutor::run_output` and
//! `search_output` are where every backend's stdout becomes a `String`, and they sanitize now,
//! which means a backend written tomorrow inherits it by reading output the way everything else
//! does. This is the shape `dry_run` already argued for in its own module: *"the check moves to
//! where the write happens. A verb added tomorrow inherits it by calling the writer everything
//! else calls, rather than by remembering a convention."*
//!
//! **What is deliberately not sanitized: file contents.** `git show HEAD:path` returns a file,
//! not a report. Trimming it changes the file, and stripping escapes from it corrupts any file
//! that legitimately contains them. The rule is about output a human or a parser reads as
//! *text*, and a file is neither.

use once_cell::sync::Lazy;
use regex::Regex;

static ANSI_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"[\u001b\u009b]\[[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]")
        .unwrap()
});

/// Strip ANSI escapes, collapse CRLF, and trim.
///
/// Collapses CRLF only; a lone `\r` (winget's progress spinner) survives and must be
/// handled by the caller.
///
/// Runs on every command's output. The common case on Linux is text with no escapes and no
/// CRLF, where this still allocated three `String`s — one for `replace_all`, one for
/// `replace`, one for `trim().to_string()`. That case allocates one now, and only because the
/// signature promises an owned value.
pub fn sanitize(input: &str) -> String {
    let cleaned = ANSI_REGEX.replace_all(input, "");
    match cleaned {
        std::borrow::Cow::Borrowed(s) if !s.contains("\r\n") => s.trim().to_string(),
        other => other.replace("\r\n", "\n").trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_strips_escapes_and_collapses_crlf() {
        let input = "\u{1b}[32mSuccessfully installed\u{1b}[0m package-1.2.3\r\n";
        assert_eq!(sanitize(input), "Successfully installed package-1.2.3");
    }

    /// The case the hand-rolled parsers were exposed to. A coloured listing parsed raw yields
    /// package names with escape bytes welded to them — names that match nothing the installed
    /// listing reports, which is permanent phantom drift rather than a visible failure.
    #[test]
    fn a_coloured_listing_yields_clean_names() {
        let listing = "\u{1b}[1mripgrep\u{1b}[0m 14.1.0\r\n\u{1b}[1mfd\u{1b}[0m 10.2.0\r\n";
        let clean = sanitize(listing);
        let names: Vec<&str> = clean
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert_eq!(names, ["ripgrep", "fd"]);

        // And the same listing parsed raw, which is what six backends did: the escape bytes
        // ride along on the name.
        let raw: Vec<&str> = listing
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        assert_ne!(raw, ["ripgrep", "fd"]);
    }

    /// Sanitizing twice is sanitizing once. The parsers that already called it now receive
    /// sanitized output from the executor as well, and a second pass must be a no-op rather
    /// than something that trims one more layer off a legitimately-indented line.
    #[test]
    fn it_is_idempotent() {
        for input in [
            "\u{1b}[32mok\u{1b}[0m\r\n",
            "plain",
            "",
            "  leading and trailing  ",
            "a\r\nb\r\nc",
        ] {
            let once = sanitize(input);
            assert_eq!(sanitize(&once), once, "not idempotent for {input:?}");
        }
    }

    /// A lone `\r` survives, because winget's progress spinner uses it to overwrite a line and
    /// the caller has to decide which of the overwritten states it wanted.
    #[test]
    fn a_lone_carriage_return_survives() {
        assert_eq!(sanitize("first\rsecond"), "first\rsecond");
    }
}
