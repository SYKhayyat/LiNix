// Each parser takes the raw stdout plus the backend id, so a single implementation can be
// reused by several managers whose output shares a shape. They are wired to backends via
// non-capturing closures in `backends/registry.rs`, e.g.
// `|o| ecosystem::ws_name_version(o, "guix")`.
//
// Lenient about *lines*, strict about the *answer*. Package-manager output drifts across
// versions, so a parser skips blank lines, table headers and decorative rows rather than
// erroring on each. What it may not do is turn a whole output it recognised nothing in into an
// empty list: that is a different fact, the planner acts on it in the opposite direction, and
// `or_unrecognised` is where the two are told apart. See `parsers::Unrecognised`.

use crate::core::Package;
use crate::parsers::{or_unrecognised, ParseResult};
use crate::utils::text::sanitize;

/// Header tokens that commonly lead a table's first column and must not be mistaken for a
/// package name.
fn is_header_token(tok: &str) -> bool {
    matches!(
        tok,
        "NAME" | "Name" | "PLUGIN" | "Plugin" | "Package" | "PACKAGE" | "Repository"
            | "Bucket" | "Source" | "Version" | "VERSION" | "Global"
    ) ||
    // All-caps alphabetic word of length >= 2 (e.g. "STATUS") — table headers are usually caps.
    (tok.len() >= 2 && tok.chars().all(|c| c.is_ascii_uppercase()))
}

/// True for a decorative / non-data line: empty, a tree connector, a dashed separator, a
/// bracketed status banner, or a table header.
///
/// **The header belongs here and not in each reader's `filter_map`.** Ten readers dropped a
/// header row from the *packages* while leaving it in the *candidates*, and `or_unrecognised`
/// reads a non-empty candidate list that yielded nothing as *"I could not understand this"*. So
/// `helm plugin list` on a cluster with no plugins — which prints exactly
/// `NAME\tVERSION\tDESCRIPTION` and nothing else — was refused as unreadable rather than
/// answered as empty, and every verb that needs the installed set refused with it.
fn is_noise_line(line: &str) -> bool {
    let t = line.trim();
    t.is_empty()
        || t.split_whitespace().next().is_some_and(is_header_token)
        || t.starts_with('#')
        || t.chars()
            .all(|c| matches!(c, '-' | '=' | '─' | '│' | '├' | '└' | ' '))
        || t.starts_with(['─', '│', '├', '└'])
        || is_placeholder(t)
        || is_empty_result_sentence(t)
        // A heading that introduces the list — `nimble list --installed` prints
        // `Package list format:` above its legend when nothing is installed, and that is a
        // correct empty answer rather than a listing nobody could read.
        || crate::parsers::is_prose_line(t)
}

/// `{PackageName}`, `<name>` — a manager describing the shape of its own output. `nimble list
/// --installed` prints its format legend when nothing is installed, and its first token became
/// a package called `{PackageName}` (the S22 failure, in a second manager). No real package
/// name opens with a placeholder bracket.
fn is_placeholder(line: &str) -> bool {
    let first = line.split_whitespace().next().unwrap_or("");
    first.starts_with('{') || first.starts_with('<')
}

/// "No global environments found." — a manager saying it has nothing, not a package called
/// `No`. Every parser here takes the first token of a line, so an unfiltered empty-result
/// banner becomes a phantom package that `adopt` would write into a manifest and
/// `purge-undeclared` would try to delete.
///
/// Prose, not identifiers: a package line is tokens, never a sentence ending in a period.
fn is_empty_result_sentence(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("no ") && lower.ends_with('.') && line.split_whitespace().count() > 2
}

/// One package name per line, taking the first whitespace token. For managers whose list/
/// search prints bare identifiers (opam `--short`, spack `list`, emerge `qlist -I` atoms).
/// Skips blank lines and table headers.
///
/// **Not pixi.** This said "pixi `search`" and was wrong: pixi prints a detail record, and the
/// first token of each of its lines is a field label, a separator or a bare version number —
/// 20 of them in one search, measured. `pixi_search` reads the record. The comment naming a
/// manager this function does not suit is what kept the routing wrong, so the list here is the
/// list that has been checked against that manager's real output and nothing else.
pub fn names_only(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let tok = l.split_whitespace().next()?;
            if is_header_token(tok) {
                return None;
            }
            Some(Package::new(tok, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// `name version [extra…]` per line, whitespace- or tab-separated. Covers cabal
/// (`--simple-output`), spack (`find --format "{name} {version}"`), pub (`global list`),
/// krew (`list`), helm (`plugin list`), guix (`package -I`), luarocks (`--porcelain`).
/// The second column is treated as the version; any trailing columns are ignored.
pub fn ws_name_version(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            match parts.next() {
                Some(ver) => Some(Package::with_version(name, ver, backend)),
                None => Some(Package::new(name, backend)),
            }
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// `cabal list --installed --simple-output`, which is `ws_name_version` plus the chatter cabal
/// writes to **stdout** when it has no config file yet.
///
/// ```text
/// Config file path source is default config file.
/// Config file not found: /root/.config/cabal/config
/// Writing default configuration to /root/.config/cabal/config
/// Cabal 3.10.3.0
/// ```
///
/// Read as `name version` those three lines are the packages `Config@file`, `Config@file` and
/// `Writing@default`, and the first two collide on a name — so a container's first `cabal list`
/// reported three packages nobody installed, ahead of the thirty-eight that were real.
///
/// Haskell versions are PVP: dot-separated integers, always. A second column that does not start
/// with a digit is therefore not a version, and the line it is on is not a package.
pub fn cabal_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            let ver = parts.next()?;
            ver.starts_with(|c: char| c.is_ascii_digit())
                .then(|| Package::with_version(name, ver, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// `uv tool list`, which is one `name vVERSION` line per tool followed by one `- executable`
/// line per command that tool exposes.
///
/// ```text
/// ruff v0.16.2
/// - ruff
/// ```
///
/// Read as `ws_name_version` the second line is a package named `-` at version `ruff`, and every
/// uv machine reports twice as many packages as it has. The `v` goes too: uv prints it, nobody
/// writes it, and `uv:ruff@0.16.2` would never match a recorded `v0.16.2`.
pub fn uv_tool_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean
        .lines()
        .filter(|l| !is_noise_line(l) && !l.trim_start().starts_with("- "))
        .collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            match parts.next() {
                Some(ver) => Some(Package::with_version(
                    name,
                    ver.strip_prefix('v').unwrap_or(ver),
                    backend,
                )),
                None => Some(Package::new(name, backend)),
            }
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// eopkg `list-installed` / `search`: `name - Short description`. Take the field before
/// the first ` - ` (falling back to the first token).
pub fn eopkg_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let name = l.split(" - ").next().unwrap_or(l).trim();
            let name = name.split_whitespace().next().unwrap_or(name);
            if name.is_empty() || is_header_token(name) {
                return None;
            }
            Some(Package::new(name, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// guix `search`: recutils output with `name: <pkg>` and `version: <ver>` fields, one
/// blank-line-separated record per package. Pair each `name:` with the following
/// `version:` in the same record.
pub fn guix_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let mut out = Vec::new();
    let mut pending_name: Option<String> = None;
    for line in clean.lines() {
        if let Some(rest) = line.strip_prefix("name:") {
            pending_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("version:") {
            if let Some(name) = pending_name.take() {
                out.push(Package::with_version(&name, rest.trim(), backend));
            }
        } else if line.trim().is_empty() {
            // record boundary: a name with no version still counts.
            if let Some(name) = pending_name.take() {
                out.push(Package::new(name, backend));
            }
        }
    }
    if let Some(name) = pending_name.take() {
        out.push(Package::new(name, backend));
    }
    out
}

/// Strip a Slackware package filename (`name-version-arch-build`) down to its name. The
/// last three `-`-separated fields are version, arch and build; everything before them is
/// the (possibly hyphenated) name.
fn slack_pkgname(field: &str) -> &str {
    let parts: Vec<&str> = field.split('-').collect();
    if parts.len() >= 4 {
        // Rejoin all but the last three fields; find the byte index of the 3rd-from-last '-'.
        let keep = parts.len() - 3;
        let mut idx = 0;
        let mut seen = 0;
        for (i, c) in field.char_indices() {
            if c == '-' {
                seen += 1;
                if seen == keep {
                    idx = i;
                    break;
                }
            }
        }
        if idx > 0 {
            return &field[..idx];
        }
    }
    field
}

/// slackpkg installed list: output of `ls /var/log/packages`, one `name-ver-arch-build`
/// filename per line.
pub fn slackpkg_installed(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    let found = candidates
        .iter()
        .map(|l| Package::new(slack_pkgname(l), backend))
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// slackpkg `search`: rows like `[ installed ] - name-ver-arch-build`. Pull the package
/// field after `] - ` (or `- `) and strip it to a name.
pub fn slackpkg_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let field = l.rsplit("- ").next()?.trim();
            if field.is_empty() || !field.contains('-') {
                return None;
            }
            Some(Package::new(slack_pkgname(field), backend))
        })
        .collect()
}

/// nimble `list --installed`. Name is the first token; the version is the first entry inside
/// the brackets, in either of the two layouts nimble has shipped.
///
/// Nimble 0.13 printed a bare version list, `pkgname  [1.0.0, 0.9.0]`. Nimble 2 prints a record
/// per version instead:
///
/// ```text
/// nimpy  [(version: 0.2.1, checksum: 22173fb24ce9ca9d1c1db63fe15bdfb14e69c76a)]
/// ```
///
/// Taking the first comma-separated field of that yields `(version: 0.2.1`, which is what this
/// build recorded as nimpy's version until a container was asked. A pin can never match it and
/// `linix list` prints it verbatim.
pub fn nimble_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let t = l.trim();
            let name = t.split_whitespace().next()?;
            if is_header_token(name) {
                return None;
            }
            if let (Some(open), Some(close)) = (t.find('['), t.find(']')) {
                if close > open + 1 {
                    let first = t[open + 1..close].split(',').next().unwrap_or("");
                    // `(version: 0.2.1` in the record layout, `1.0.0` in the bare one. The
                    // label is what differs; the value is after the last colon either way.
                    let ver = first
                        .rsplit(':')
                        .next()
                        .unwrap_or(first)
                        .trim()
                        .trim_start_matches('(')
                        .trim();
                    if !ver.is_empty() {
                        return Some(Package::with_version(name, ver, backend));
                    }
                }
            }
            Some(Package::new(name, backend))
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// mix `archive`: lines like `* hex-2.0.6`. Strip the leading bullet, then split the
/// trailing `-version` off the archive name.
pub fn mix_archive(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let candidates: Vec<&str> = clean.lines().filter(|l| !is_noise_line(l)).collect();
    let found = candidates
        .iter()
        .filter_map(|l| {
            let t = l.trim();
            let entry = t.strip_prefix("* ").or_else(|| t.strip_prefix('*'))?.trim();
            if entry.is_empty() {
                return None;
            }
            match entry.rsplit_once('-') {
                Some((name, ver))
                    if !name.is_empty()
                        && ver.chars().next().is_some_and(|c| c.is_ascii_digit()) =>
                {
                    Some(Package::with_version(name, ver, backend))
                }
                _ => Some(Package::new(entry, backend)),
            }
        })
        .collect();
    or_unrecognised(backend, found, &candidates)
}

/// asdf `list`: non-indented lines are plugin (tool) names; indented lines are installed
/// versions of the preceding plugin and are skipped.
/// **This parser's own judgement about not understanding**, which is not the shared one. A
/// plugin added with nothing installed under it — `jq` followed by `  No versions installed` —
/// is a real and common state, and it produces zero packages out of two data lines. The shared
/// rule would call that drift on every machine that added a plugin and did not install a
/// version yet. So the count here is of lines that fit *neither* shape the format has, and an
/// output made entirely of understood lines is an empty answer however many of them there are.
pub fn asdf_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    let mut out: Vec<Package> = Vec::new();
    let mut plugin: Option<String> = None;
    let mut unaccounted: Vec<&str> = Vec::new();

    // A plugin line is unindented; the versions installed under it are indented beneath it. A
    // plugin with none still prints — `jq` followed by `  No versions installed` — so the name
    // alone means "added", not "installed", and reading it as installed is drift `sync` can
    // never converge: it removes the version and finds it again on the next run.
    for line in clean.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            let version = line.trim();
            if version.eq_ignore_ascii_case("No versions installed") {
                continue;
            }
            match plugin.take() {
                Some(name) => out.push(Package::new(&name, backend)),
                // An indented version with no plugin above it: the indentation that carries this
                // format's whole meaning did not hold.
                None => unaccounted.push(line),
            }
            continue;
        }
        let name = line.trim();
        if name.starts_with('*') || is_header_token(name) {
            plugin = None;
            continue;
        }
        plugin = Some(name.to_string());
    }
    or_unrecognised(backend, out, &unaccounted)
}

/// emerge `--search`: package hits are `*  category/pkg` lines.
pub fn emerge_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    clean
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let atom = t.strip_prefix("* ")?.trim();
            if atom.is_empty() || !atom.contains('/') {
                return None;
            }
            Some(Package::new(atom, backend))
        })
        .collect()
}

/// pixi `global list`: tree rows like `├── python: 3.11.0` (older `- python 3.11.0`).
///
/// **Only the top level is a package.** pixi prints each environment's properties as indented
/// children of its row:
///
/// ```text
/// └── ripgrep: 15.2.0
///     └─ exposes: rg
/// ```
///
/// and `exposes` is the word for *this environment puts these binaries on PATH*, not a second
/// tool. Reported as one, it became a package `list` printed and `check` counted, with `rg` as
/// its version. The depth is the tool's own structure, so this reads that rather than
/// blocklisting the property names pixi happens to print today.
///
/// **This parser has no unread case, and saying so is the honest answer** rather than inventing
/// one. Every unindented line resolves to exactly one of three things it understands: a package,
/// pixi's own banner (a multi-word left side before the colon, which is what
/// `Global environments as specified in 'C:\…'` is), or noise it already names. The failure mode
/// this parser has ever had is *junk*, not emptiness — `exposes: rg` reported as a tool — and
/// there are captured fixtures for that. Forcing a made-up emptiness rule on it would redden a
/// machine with pixi installed and nothing in it, which is the one case its banner covers.
pub fn pixi_list(output: &str, backend: &str) -> ParseResult {
    let clean = sanitize(output);
    // Only the unindented rows are packages; the indented ones are an environment's properties.
    //
    // The noise check belongs *after* the tree connectors are trimmed: a row reads
    // `└── ripgrep: 15.2.0`, so testing the raw line would drop every package pixi printed and
    // leave only the banner.
    let found = clean
        .lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .filter_map(|l| {
            let t = l
                .trim()
                .trim_start_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '├' | '└' | '│' | '─' | '-' | '*')
                })
                .trim();
            if t.is_empty() || is_noise_line(t) {
                return None;
            }
            if let Some((name, ver)) = t.split_once(':') {
                let name = name.trim();
                let ver = ver.trim();
                // A real package name is a single token; a multi-word left side is a banner
                // like "Global environments at /path:" and must be skipped.
                if name.is_empty() || name.contains(char::is_whitespace) || is_header_token(name) {
                    return None;
                }
                return if ver.is_empty() {
                    Some(Package::new(name, backend))
                } else {
                    Some(Package::with_version(name, ver, backend))
                };
            }
            let mut parts = t.split_whitespace();
            let name = parts.next()?;
            if is_header_token(name) {
                return None;
            }
            match parts.next() {
                Some(ver) => Some(Package::with_version(name, ver, backend)),
                None => Some(Package::new(name, backend)),
            }
        })
        .collect();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_name_version_parses_and_skips_header() {
        let out = "NAME       VERSION\nfoo        1.2.3\nbar        0.1.0   some-desc\n";
        let pkgs = ws_name_version(out, "helm").expect("this fixture parses");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "foo");
        assert_eq!(pkgs[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(pkgs[1].name, "bar");
        assert_eq!(pkgs[0].backend, "helm");
    }

    /// **The family, not the finding.** `helm plugin list` with no plugins prints its header
    /// and stops, and every reader in this module dropped the header from the packages while
    /// leaving it in the candidates — so `or_unrecognised` saw one data line yielding nothing
    /// and refused the whole listing. A manager with nothing installed is not a manager nobody
    /// can read, and the two answers send the planner in opposite directions.
    ///
    /// One case per reader that takes a header, because the fix is in `is_noise_line` and a
    /// reader that stops calling it would go back to refusing.
    #[test]
    fn a_listing_that_is_only_a_header_is_empty_rather_than_unreadable() {
        /// A backend, the reader its row names, and a listing of nothing but a header.
        type HeaderCase = (&'static str, fn(&str, &str) -> ParseResult, &'static str);
        let cases: [HeaderCase; 6] = [
            ("helm", ws_name_version, "NAME\tVERSION\tDESCRIPTION\n"),
            ("krew", names_only, "PLUGIN\n"),
            ("cabal", cabal_list, "Package Version\n"),
            ("uv", uv_tool_list, "NAME VERSION\n"),
            ("nimble", nimble_list, "Name Version\n"),
            ("eopkg", eopkg_list, "Package - Description\n"),
        ];
        for (backend, reader, header) in cases {
            let pkgs = reader(header, backend)
                .unwrap_or_else(|e| panic!("{backend}: a bare header was refused — {e:?}"));
            assert!(
                pkgs.is_empty(),
                "{backend}: read {pkgs:?} out of a header row"
            );
        }
    }

    /// Real bytes, and the whole reason `cabal_list` exists: a container's first `cabal list`
    /// writes three lines of configuration chatter to **stdout** ahead of the packages.
    #[test]
    fn cabal_reads_past_the_chatter_of_a_first_run() {
        let out = "Config file path source is default config file.\n\
                   Config file not found: /root/.config/cabal/config\n\
                   Writing default configuration to /root/.config/cabal/config\n\
                   Cabal 3.10.3.0\n\
                   array 0.5.8.0\n";
        let pkgs = cabal_list(out, "cabal").expect("this is what haskell:9.6 printed");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "Cabal");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.5.8.0"));
    }

    /// The executables a uv tool exposes are indented under it and are not tools.
    #[test]
    fn a_uv_tool_s_executables_are_not_packages() {
        let pkgs = uv_tool_list("ruff v0.16.2\n- ruff\n- ruff-lsp\n", "uv")
            .expect("this is what `uv tool list` printed");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "ruff");
        assert_eq!(pkgs[0].version.as_deref(), Some("0.16.2"));
    }

    /// Both nimble layouts, because the record one arrived in nimble 2 and the bare one is
    /// still what older installs print.
    #[test]
    fn nimble_reads_a_version_out_of_either_layout() {
        let record = nimble_list(
            "nimpy  [(version: 0.2.1, checksum: 22173fb24ce9ca9d1c1db63fe15bdfb14e69c76a)]\n",
            "nimble",
        )
        .expect("nimble 2 output");
        assert_eq!(record[0].version.as_deref(), Some("0.2.1"));

        let bare = nimble_list("chronos  [1.0.0, 0.9.0]\n", "nimble").expect("nimble 0.13 output");
        assert_eq!(bare[0].version.as_deref(), Some("1.0.0"));
    }

    /// **Synthetic, and labelled as such — this is the test the rule 250 lines below condemns.**
    ///
    /// Its input was typed by hand and labelled `"spack"`, and `names_only` is the *installed*
    /// lister for `opam` and `emerge`. The rule at the bottom of this file was written about
    /// exactly this shape: *"a parser is tested against output captured from the tool it parses,
    /// and from no other tool. `names_only` serves five managers and its only test used a spack
    /// fixture — it passed, and said nothing whatever about pixi, which is exactly where it was
    /// wrong."*
    ///
    /// It is kept because what it asserts is real and cheap — a header row and a dashed rule are
    /// not packages, which is a property of the *function* rather than of any manager. It is
    /// renamed so that nobody reads it as coverage of a manager. The coverage lives in
    /// `tests/installed_listing_fixture_tests.rs`, against output captured from the tools
    /// themselves.
    #[test]
    fn names_only_drops_furniture_synthetic_not_a_managers_output() {
        let out = "Package\n----------\nripgrep\nfd\n\n";
        let pkgs = names_only(out, "spack").expect("this fixture parses");
        assert_eq!(
            pkgs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["ripgrep", "fd"]
        );
    }

    #[test]
    fn eopkg_list_takes_name_before_dash() {
        let out = "nano - Small, friendly text editor\ngit - Distributed VCS\n";
        let pkgs = eopkg_list(out, "eopkg").expect("this fixture parses");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "nano");
        assert_eq!(pkgs[1].name, "git");
    }

    #[test]
    fn guix_search_pairs_name_and_version() {
        let out = "name: hello\nversion: 2.12\nsynopsis: Hello, GNU world\n\nname: emacs\nversion: 29.1\n";
        let pkgs = guix_search(out, "guix");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "hello");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.12"));
        assert_eq!(pkgs[1].name, "emacs");
        assert_eq!(pkgs[1].version.as_deref(), Some("29.1"));
    }

    #[test]
    fn slackpkg_installed_strips_version_arch_build() {
        let out = "bash-5.1.016-x86_64-4\naaa_base-15.0-x86_64-3\nvim-9.0.2000-x86_64-1\n";
        let pkgs = slackpkg_installed(out, "slackpkg").expect("this fixture parses");
        assert_eq!(pkgs[0].name, "bash");
        assert_eq!(pkgs[1].name, "aaa_base");
        assert_eq!(pkgs[2].name, "vim");
    }

    #[test]
    fn slackpkg_search_extracts_pkg_field() {
        let out = "[ installed ] - mc-4.8.29-x86_64-1\n[uninstalled] - htop-3.2.1-x86_64-1\n";
        let pkgs = slackpkg_search(out, "slackpkg");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "mc");
        assert_eq!(pkgs[1].name, "htop");
    }

    #[test]
    fn nimble_list_reads_name_and_bracket_version() {
        let out = "  jester  [0.5.0]\n  nimx  [0.1.0, 0.2.0]\n";
        let pkgs = nimble_list(out, "nimble").expect("this fixture parses");
        assert_eq!(pkgs[0].name, "jester");
        assert_eq!(pkgs[0].version.as_deref(), Some("0.5.0"));
        assert_eq!(pkgs[1].name, "nimx");
        assert_eq!(pkgs[1].version.as_deref(), Some("0.1.0"));
    }

    #[test]
    fn mix_archive_splits_name_and_version() {
        let out = "* hex-2.0.6\n* phx_new-1.7.0\n";
        let pkgs = mix_archive(out, "mix").expect("this fixture parses");
        assert_eq!(pkgs[0].name, "hex");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.0.6"));
        assert_eq!(pkgs[1].name, "phx_new");
        assert_eq!(pkgs[1].version.as_deref(), Some("1.7.0"));
    }

    #[test]
    fn asdf_list_keeps_plugins_skips_versions() {
        let out = "nodejs\n  18.0.0\n  20.0.0\npython\n  3.11.0\n";
        let pkgs = asdf_list(out, "asdf").expect("this fixture parses");
        assert_eq!(
            pkgs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["nodejs", "python"]
        );
    }

    /// A plugin that is ADDED and has nothing installed is not an installed package, and
    /// `asdf list` prints its name either way. Captured from asdf v0.14.1 in the `tools`
    /// image, 2026-07-29, on both sides of an `asdf uninstall jq`:
    ///
    /// ```text
    /// $ asdf list          $ asdf uninstall jq && asdf list
    /// jq                   jq
    ///   1.8.2                No versions installed
    /// ```
    ///
    /// The sweep found it as `asdf: jq is gone from list (expected non-zero, got 0)`. LiNix
    /// removed the version correctly and went on reporting it as installed, which is permanent
    /// phantom drift: `sync` would take it away and put it back forever.
    #[test]
    fn asdf_list_does_not_report_a_plugin_with_nothing_installed() {
        const INSTALLED: &str = include_str!("../../tests/fixtures/asdf/list-installed.txt");
        const EMPTY_PLUGIN: &str =
            include_str!("../../tests/fixtures/asdf/list-plugin-without-versions.txt");
        const NO_PLUGINS: &str = include_str!("../../tests/fixtures/asdf/list-no-plugins.txt");

        assert_eq!(
            asdf_list(INSTALLED, "asdf")
                .expect("this fixture parses")
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["jq"],
            "the control: a plugin WITH a version is installed"
        );
        assert!(
            asdf_list(EMPTY_PLUGIN, "asdf")
                .expect("this fixture parses")
                .is_empty(),
            "a plugin with no versions was reported as an installed package"
        );
        assert!(
            asdf_list(NO_PLUGINS, "asdf")
                .expect("this fixture parses")
                .is_empty(),
            "asdf's own empty-state sentence was read as a package"
        );
    }

    #[test]
    fn emerge_search_extracts_atoms() {
        let out = "Searching...\n[ Results for search key : vim ]\n\n*  app-editors/vim\n      Latest version available: 9.0\n*  app-editors/gvim\n";
        let pkgs = emerge_search(out, "emerge");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "app-editors/vim");
        assert_eq!(pkgs[1].name, "app-editors/gvim");
    }

    /// The tool's own output, not a hand-written imitation of it: the invented fixture had two
    /// rows, no nested child and a banner in a wording pixi no longer uses, so it passed while
    /// the parser reported `exposes` as an installed package (GRADER §3.3).
    #[test]
    fn pixi_list_reads_the_tools_own_output() {
        const LIST: &str = include_str!("../../tests/fixtures/pixi/list-one-tool.txt");
        let pkgs = pixi_list(LIST, "pixi").expect("this fixture parses");
        let names: Vec<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["ripgrep"],
            "an `exposes:` child is not a package"
        );
        assert_eq!(pkgs[0].version.as_deref(), Some("15.2.0"));
    }

    #[test]
    fn pixi_list_handles_tree_and_flat() {
        let tree =
            "Global environments at /root/.pixi/envs:\n├── python: 3.11.0\n└── ripgrep: 14.0.0\n";
        let pkgs = pixi_list(tree, "pixi").expect("this fixture parses");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "python");
        assert_eq!(pkgs[0].version.as_deref(), Some("3.11.0"));
        assert_eq!(pkgs[1].name, "ripgrep");
    }

    #[test]
    fn an_empty_result_banner_is_not_a_package() {
        // `pixi global list` on a machine with nothing installed prints a sentence, and the
        // parser used to take its first token -- reporting a phantom `pixi:No` that `adopt`
        // would write to a manifest and `purge-undeclared` would try to delete.
        assert!(pixi_list(
            "No global environments found.
",
            "pixi"
        )
        .expect("this fixture parses")
        .is_empty());
        assert!(names_only(
            "No packages found.
",
            "spack"
        )
        .expect("this fixture parses")
        .is_empty());

        // A real listing that merely starts with a package beginning "no" still parses.
        let pkgs = names_only(
            "nodejs
nom
",
            "spack",
        )
        .expect("this fixture parses");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "nodejs");
    }

    #[test]
    fn a_format_legend_is_not_a_package() {
        // `nimble list --installed` with nothing installed prints the shape of its own output.
        // Its first two lines parsed as packages named `{PackageName}` and `└──` — S22 again,
        // in a manager that was not covered because the banner is not a sentence.
        let legend = "Package list format: 
{PackageName}
                      └── @{Version} ({CheckSum})[Special Versions (if any)] ({InstallPath})
";
        assert!(
            nimble_list(legend, "nimble")
                .expect("this fixture parses")
                .is_empty(),
            "{:?}",
            nimble_list(legend, "nimble").expect("this fixture parses")
        );

        // A real listing still parses, brackets and all.
        let real = "  chronos  [3.2.0, 3.1.0]
";
        let pkgs = nimble_list(real, "nimble").expect("this fixture parses");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "chronos");
    }

    #[test]
    fn guix_list_via_ws_name_version() {
        // `guix package -I` is tab-separated name<TAB>version<TAB>outputs<TAB>path.
        let out = "hello\t2.12\tout\t/gnu/store/xxx\nemacs\t29.1\tout\t/gnu/store/yyy\n";
        let pkgs = ws_name_version(out, "guix").expect("this fixture parses");
        assert_eq!(pkgs[0].name, "hello");
        assert_eq!(pkgs[0].version.as_deref(), Some("2.12"));
        assert_eq!(pkgs[1].name, "emacs");
    }
}

/// Parse `pixi search`, which prints a **detail record** rather than a list of names.
///
/// It was routed to [`names_only`], documented there as "search prints bare identifiers". Real
/// pixi output is `Name`/`Version`/`Build`/`Size`/`License`/… field rows plus a build table, so
/// taking the first token of each line produced 19 junk rows in one search: the field labels
/// themselves, the `-----` separator, `...`, and bare version numbers from the "Other Versions"
/// table. The record carries the answer in its `Name` and `Version` fields; this reads those.
pub fn pixi_search(output: &str, backend: &str) -> Vec<Package> {
    let clean = sanitize(output);
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    for line in clean.lines() {
        let mut fields = line.split_whitespace();
        match (fields.next(), fields.next()) {
            (Some("Name"), Some(v)) if name.is_none() => name = Some(v.to_string()),
            (Some("Version"), Some(v)) if version.is_none() => version = Some(v.to_string()),
            _ => {}
        }
        if name.is_some() && version.is_some() {
            break;
        }
    }
    match name {
        Some(n) => {
            let mut p = Package::new(n, backend);
            p.version = version;
            vec![p]
        }
        // "No packages found matching 'x'" is an answer, and an empty one.
        None => vec![],
    }
}

#[cfg(test)]
mod pixi_real_output_tests {
    use super::*;

    /// Captured from pixi 0.73 on this machine.
    const SEARCH: &str = include_str!("../../tests/fixtures/pixi/search-ripgrep.txt");
    const NOT_FOUND: &str = include_str!("../../tests/fixtures/pixi/search-not-found.txt");

    /// The rule this enforces repo-wide: **a parser is tested against output captured from the
    /// tool it parses, and from no other tool.** `names_only` serves five managers and its only
    /// test used a spack fixture — it passed, and said nothing whatever about pixi, which is
    /// exactly where it was wrong.
    #[test]
    fn pixi_search_reads_the_record_and_emits_no_junk() {
        let found = pixi_search(SEARCH, "pixi");
        assert_eq!(found.len(), 1, "one search, one package: {found:?}");
        assert_eq!(found[0].name, "ripgrep");
        assert_eq!(found[0].version.as_deref(), Some("15.2.0"));
    }

    /// What the old parser actually produced, asserted so it cannot come back: field labels,
    /// separators and bare version numbers offered to a user as packages to install.
    #[test]
    fn no_field_label_or_separator_is_ever_a_package() {
        let names: Vec<String> = pixi_search(SEARCH, "pixi")
            .into_iter()
            .map(|p| p.name)
            .collect();
        for junk in [
            "-",
            "...",
            "Name",
            "Version",
            "Build",
            "Size",
            "License",
            "Timestamp",
            "Subdir",
            "URL",
            "MD5",
            "SHA256",
            "Dependencies:",
            "Using",
            "15.1.0",
            "15.0.0",
        ] {
            assert!(
                !names.iter().any(|n| n == junk),
                "`{junk}` came back: {names:?}"
            );
        }
    }

    /// The red this fix was watched against, kept as evidence rather than as a memory.
    /// `names_only` is still the right parser for opam, spack and emerge; routing pixi to it
    /// was the defect, and this says what that cost on real output.
    #[test]
    fn names_only_on_pixi_output_is_what_the_bug_looked_like() {
        let junk: Vec<String> = names_only(SEARCH, "pixi")
            .expect("this fixture parses")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(
            junk.len() > 10,
            "expected the old parser to emit a pile of junk, got {junk:?}"
        );
        // What it really emitted, verbatim: field labels, the separator, the ellipsis, and
        // six bare version numbers out of the "Other Versions" table.
        //
        // `Dependencies:` was on this list and is not any more: `is_noise_line` now drops a
        // line ending in a colon, because a heading that introduces a list is the manager's
        // prose and not a candidate package. That narrows the junk without narrowing the
        // finding — the pile below is still a pile, and none of it is `ripgrep`.
        for j in ["SHA256", "Timestamp", "License", "-", "...", "15.1.0"] {
            assert!(
                junk.iter().any(|n| n == j),
                "expected junk `{j}` in {junk:?}"
            );
        }
        assert!(
            !junk.iter().any(|n| n == "ripgrep"),
            "the old parser never even produced the right answer: {junk:?}"
        );
    }

    /// The not-found case, which is where junk rows come from in three of the four managers
    /// `names_only` still serves.
    #[test]
    fn pixi_finding_nothing_yields_nothing() {
        assert!(pixi_search(NOT_FOUND, "pixi").is_empty());
        assert!(pixi_search("", "pixi").is_empty());
    }
}

/// pixi `global list --json`: an array of environments, each with its dependencies (`Q43`).
///
/// **The environment is the package, and its version comes from the dependency of the same
/// name.** That is what the text form prints as `ripgrep: 15.2.0`, and this must not widen it:
/// an environment may pull in dependencies nobody declared, and reporting those as installed
/// packages is the fault [`pixi_list`] already had to fix once, where `exposes: rg` became a
/// package named `exposes` at version `rg`.
///
/// **Both early returns used to be `Vec::new()`**, which is this whole finding in two lines:
/// output that is not JSON, and JSON that is not an array, are two ways of not understanding
/// the answer, and both were spelled *"the machine has nothing installed"*. This is also the
/// negotiated `--json` path, so the version of pixi that does not have the flag is precisely
/// the one whose usage message would have been read as an empty machine.
pub fn pixi_list_json(output: &str, backend: &str) -> ParseResult {
    let Some(doc) = crate::parsers::json_document(output) else {
        return crate::parsers::or_unrecognised_json(backend, vec![], None, "not JSON", output);
    };
    let envs = doc.as_array();
    let found = envs
        .into_iter()
        .flatten()
        .filter_map(|env| {
            let name = env.get("name")?.as_str()?.trim();
            if name.is_empty() {
                return None;
            }
            let version = env
                .get("dependencies")
                .and_then(|d| d.as_array())
                .and_then(|deps| {
                    deps.iter()
                        .find(|d| d.get("name").and_then(|n| n.as_str()) == Some(name))
                })
                .and_then(|d| d.get("version"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty());
            Some(match version {
                Some(v) => Package::with_version(name, v, backend),
                None => Package::new(name, backend),
            })
        })
        .collect::<Vec<_>>();
    // An empty array is pixi saying it has no global environments, which is a real answer. A
    // non-empty array none of whose entries carried a usable `name` is a schema change, and so
    // is a document that is not the array at all.
    crate::parsers::or_unrecognised_json(
        backend,
        found,
        envs.map(Vec::len),
        "JSON that is not the array of environments this reads, or an array of environments \
         none of them carrying a `name`",
        output,
    )
}

#[cfg(test)]
mod pixi_json_tests {
    use super::*;

    /// Verbatim from `pixi global list --json` on the host this was written on.
    const REAL: &str = r#"[
      { "name": "ripgrep",
        "dependencies": [ { "name": "ripgrep", "version": "15.2.0" } ],
        "exposed": [ { "exposed_name": "rg", "executable": "rg" } ] }
    ]"#;

    #[test]
    fn an_environment_is_one_package_at_its_own_version() {
        let pkgs = pixi_list_json(REAL, "pixi").expect("this fixture parses");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "ripgrep");
        assert_eq!(pkgs[0].version.as_deref(), Some("15.2.0"));
        assert_eq!(pkgs[0].backend, "pixi");
    }

    /// The JSON form must report exactly what the text form does — a difference between them
    /// is a machine that changes shape depending on which pixi is installed.
    #[test]
    fn the_json_and_the_tree_agree_about_the_same_machine() {
        let tree = "Global environments as specified in '/tmp/pixi-global.toml'\n\
                    └── ripgrep: 15.2.0 \n    └─ exposes: rg\n";
        let from_tree = pixi_list(tree, "pixi").expect("this fixture parses");
        let from_json = pixi_list_json(REAL, "pixi").expect("this fixture parses");
        assert_eq!(
            from_tree
                .iter()
                .map(|p| (&p.name, &p.version))
                .collect::<Vec<_>>(),
            from_json
                .iter()
                .map(|p| (&p.name, &p.version))
                .collect::<Vec<_>>(),
        );
    }

    /// `exposes` is what the environment puts on PATH, not a second tool. The tree parser
    /// reported it as a package once; the JSON parser must not find a new way to.
    #[test]
    fn what_an_environment_exposes_is_not_a_package() {
        let pkgs = pixi_list_json(REAL, "pixi").expect("this fixture parses");
        assert!(!pkgs.iter().any(|p| p.name == "rg" || p.name == "exposed"));
    }

    /// A dependency that is not the environment's own name is a dependency, not something the
    /// user asked for. Adopting those would write a dependency graph into a manifest.
    #[test]
    fn a_pulled_in_dependency_is_not_reported_as_installed() {
        let json = r#"[{"name":"ripgrep","dependencies":[
            {"name":"ripgrep","version":"15.2.0"},
            {"name":"libgcc","version":"14.1"}]}]"#;
        let pkgs = pixi_list_json(json, "pixi").expect("this fixture parses");
        assert_eq!(pkgs.len(), 1, "got {:?}", pkgs);
        assert_eq!(pkgs[0].name, "ripgrep");
    }

    /// **This test used to assert the bug.** Its name said *"reports nothing rather than
    /// guessing"* and every case answered `Ok(vec![])` — which is not "nothing", it is
    /// *"this machine has no packages installed"*, the most consequential claim a listing can
    /// make. Four inputs, three of which are a manager failing, all four spelled as an empty
    /// machine.
    ///
    /// `""` is the one that matters most: it is what `4d4a890` measured a cold `winget list`
    /// producing three times in sixteen tries, having written zero bytes.
    #[test]
    fn an_empty_array_is_an_empty_machine_and_everything_else_is_unread() {
        // The one real empty answer: pixi has no global environments.
        assert!(pixi_list_json("[]", "pixi")
            .expect("an empty array is pixi saying it has none")
            .is_empty());

        // And the three that are not. `""` is what `4d4a890` measured a cold `winget list`
        // producing three times in sixteen tries, having written zero bytes — the input this
        // whole type exists to stop being read as a bare machine.
        for (input, why) in [
            ("", "a manager that printed nothing at all"),
            ("not json", "a usage message, a warning, or anything else"),
            (
                r#"{"name":"x"}"#,
                "JSON of a shape this does not read — a schema change",
            ),
        ] {
            let err =
                pixi_list_json(input, "pixi").expect_err(&format!("{why} is not an empty machine"));
            assert_eq!(err.backend, "pixi");
        }
    }
}
