use crate::core::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::warn;

/// The allowlist must stay wide enough for names that are legitimately not bare words:
/// npm `@scope`, github `owner/repo`, versioned `pkg:1.2+build`.
static PACKAGE_NAME_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9._@:+/-]+$").unwrap());

/// The same allowlist plus the characters a Windows package identifier is built from.
///
/// `winget list` reports 185 of 278 names on a stock box as `ARP\Machine\X64\...` or
/// `MSIX\...`, and the ARP rows for MSI installers are GUIDs in braces. Those are the
/// identifiers `winget install` and `winget uninstall` take, so they are the names Shall has to
/// be able to carry (V.113).
///
/// **The space is here for the same reason the backslash is.** `ARP\Machine\X64\Mozilla Firefox`
/// is what winget answers and what winget accepts back; a rule that admits the backslash and
/// stops at the space carries most of an identifier and not the one the user has. It is safe
/// for the reason stated below: a name is argv, never a shell string. The grammar makes such a
/// name writable by quoting it, and this is the second half of V.113 — a name is admitted by a
/// grammar **and** a validator, and the two must agree.
///
/// **Inside the name, never at its edge.** A trailing space is invisible in a manifest, in a
/// listing and in every error message, so `Firefox ` and `Firefox` are one package that reads
/// as itself twice and reconciles as two. The grammar refuses the same shape at the same
/// boundary; a rule enforced on one side of a round trip is a rule the other side breaks.
static WINDOWS_IDENTIFIER_NAME_REGEX: Lazy<Regex> = Lazy::new(|| {
    // The space is the only character legal in the middle and not at either end, so it is the
    // only one lifted out of the shared class.
    const INNER: &str = r"a-zA-Z0-9._@:+/\\{}-";
    Regex::new(&format!("^[{INNER}](?:[ {INNER}]*[{INNER}])?$")).unwrap()
});

/// Shell metacharacters, minus the three a Windows package identifier is made of.
///
/// Safe because **no package-manager command is ever a shell string** — every one is argv, and
/// that is the property the executor's own tests exist to keep. This list is defence in depth
/// against a name reaching a shell that does not exist; it is not what stands between a crafted
/// name and a command line.
static SHELL_INJECTION_REGEX_WINDOWS_ID: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\*\?\!]").unwrap());

/// Shell metacharacters blocked to prevent command injection.
static SHELL_INJECTION_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[;&|><`$\(\)\[\]\{\}\*\?\!\\]").unwrap());

/// The characters that let a string stop being one argument and start being a second command.
///
/// Deliberately narrower than [`SHELL_INJECTION_REGEX`], which is the metacharacter half of a
/// package-*name* rule and also bans `\`, `{}`, `*` and `?`. A winget identifier carries the
/// first two and an ordinary search query carries the last two, so that rule cannot be applied
/// to a string whose kind is not yet known. This one refuses the separators and nothing else:
///
/// - `& | < > ^` — `cmd` parses these out of its own command line before a `.cmd` shim sees
///   them. A package name of `q&calc.exe&rem` reaching a `.cmd` shim ran `calc.exe`.
/// - ``; $ ` ( )`` — the POSIX separators and substitutions. Shall never builds a shell string,
///   and the point of a second layer is to survive the day one of its callers does.
/// - `"` and the line terminators, which end an argument inside whatever is quoting it.
static COMMAND_METACHARACTER_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new("[;&|<>^`$()\"\r\n]").unwrap());

/// Sensitive system paths that Shall is prohibited from accessing.
static FORBIDDEN_PATHS: &[&str] = &[
    "/etc/shadow",
    "/etc/sudoers",
    "/etc/passwd",
    "/etc/gshadow",
    "C:\\Windows\\System32\\config\\SAM",
    "C:\\Windows\\System32\\config\\SECURITY",
];

/// Render untrusted text for a message a terminal will draw.
///
/// A refusal about invisible characters that reprints them is worse than no refusal: U+202E
/// reverses everything after it as it renders, so the message can be made to read as its own
/// opposite, and an ANSI escape recolours or erases the lines around it. Manifests arrive from
/// shared configs, not only from the user's own hand.
///
/// Everything outside printable ASCII-and-ordinary-Unicode is named by codepoint instead of
/// emitted. Ordinary non-ASCII stays as itself — a package name in Cyrillic should read as one
/// — so the rule is drawn at *what the character does to the display*, not at what alphabet it
/// belongs to: C0/C1 controls, the bidi overrides and embeddings, the invisible formatting
/// characters, and the line/paragraph separators.
pub fn printable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let dangerous = c.is_control()
            // Bidi overrides, embeddings and isolates — the trojan-source family.
            || ('\u{202A}'..='\u{202E}').contains(&c)
            || ('\u{2066}'..='\u{2069}').contains(&c)
            // Zero-width and other invisible formatting.
            || matches!(c, '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{00AD}')
            // Line and paragraph separators, which break a message across lines.
            || matches!(c, '\u{2028}' | '\u{2029}');
        if dangerous {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

pub struct Validator;

impl Validator {
    /// Backends whose "name" is legitimately a filesystem path (`link`) or a URL / owner-repo
    /// (`web`, `github`, `appimage`). For these the "looks like an absolute path" guard — a
    /// leading `/` or `\` — would wrongly reject valid input (e.g. `link:/home/me/.vimrc`).
    /// They still get every other check: `..` traversal, the character allowlist, and
    /// shell-injection blocking. Only the leading-separator rule is lifted for them.
    fn is_path_oriented_backend(backend: &str) -> bool {
        // `btrfs` was missing until 2026-07-30, and it is the member whose name is *most*
        // literally a path: `btrfs:/mnt/data/vol` installs by running
        // `btrfs subvolume create /mnt/data/vol`. No declaration of it could be written, and
        // nothing noticed because no harness had a btrfs filesystem to install into.
        //
        // Not `lvm` or `zfs`: `lvm:vg0/data` and `zfs:tank/data` carry a separator and never a
        // leading one, so the strict rule is correct for them and widening it would buy nothing.
        matches!(backend, "link" | "web" | "github" | "appimage" | "btrfs")
    }

    /// Backends whose manager's own identifiers carry a path separator or braces.
    ///
    /// `winget` is the whole list, and a list rather than a rule because those characters are
    /// worth refusing everywhere else: no second manager on any platform names things this way.
    /// `..` stays forbidden for it, exactly as for everything else.
    fn names_carry_windows_identifiers(backend: &str) -> bool {
        backend == "winget"
    }

    /// Refuse a string that reaches a manager's argv *before* it is known to be a package name.
    ///
    /// Two strings do that, and both were live. A **bare** (unprefixed) name is handed to each
    /// candidate manager to find out which one owns it — that probe runs before the model is
    /// collected and therefore before [`Validator::validate_package_name_for`] ever sees the
    /// name. A **search query** is free text by definition and passes no name rule at all. On
    /// Windows both reach managers that ship as `.cmd` shims: `shall search 'q&calc.exe&rem'`
    /// launched `calc.exe`, and one crafted bare name in a shared module wrote files on every
    /// machine that evaluated it.
    ///
    /// **This is the second layer, not the fix.** The fix is that a `.cmd` shim is spawned as
    /// itself so the standard library escapes its arguments (`core/executor.rs`). A comment
    /// calling this validator the thing that stands between a crafted name and a command line
    /// is what let the first layer stay broken.
    pub fn refuse_command_metacharacters(text: &str, what: &str) -> Result<()> {
        if let Some(m) = COMMAND_METACHARACTER_REGEX.find(text) {
            return Err(Error::Validation(format!(
                "`{}` is not usable as {}: it contains `{}`, which a command interpreter reads \
                 as the end of one command and the start of another.",
                printable(text),
                what,
                printable(m.as_str())
            )));
        }
        Ok(())
    }

    /// Validates package names against injection and traversal, with no knowledge of the
    /// backend — the strict rule (a leading path separator is rejected). Prefer
    /// [`Validator::validate_package_name_for`] when the backend is known.
    pub fn validate_package_name(name: &str) -> Result<()> {
        Self::validate_package_name_for(name, "")
    }

    /// Validates a package name for a specific backend. Identical to
    /// [`Validator::validate_package_name`] except that the "absolute path" guard (a leading
    /// `/` or `\`) is lifted for the path/URL-oriented backends (see
    /// [`Validator::is_path_oriented_backend`]), whose names are legitimately paths/URLs.
    /// Directory traversal (`..`), the character allowlist, and shell-injection blocking
    /// always apply, for every backend.
    pub fn validate_package_name_for(name: &str, backend: &str) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Validation("Empty package name".into()));
        }
        if name.len() > 256 {
            return Err(Error::Validation("Name too long".into()));
        }

        // Directory traversal is ALWAYS forbidden, regardless of backend.
        if name.contains("..") {
            return Err(Error::Validation(format!(
                "Path traversal detected in name: {}",
                printable(name)
            )));
        }
        // **A name that opens with `-` is a flag, and no manager has a package called one.**
        //
        // This is what makes a wrong row in the terminator table exploitable rather than merely
        // wrong: `composer:--version` passed every check here and reached composer's argv, on a
        // manager Shall believed `--` would protect and which ignores it (B5). The terminator
        // table stays — it is the mechanism — but it is one boolean per binary measured on one
        // image, and a guard that depends on fifty of those being right is a guard with fifty
        // ways to be wrong.
        //
        // Refused for every backend, including the path-oriented ones: a path may begin with
        // `/` or `~`, and a leading `-` is a flag there too (`rm -rf` reads `-rf` no differently
        // for having come out of a `link:` line).
        if name.starts_with('-') {
            return Err(Error::Validation(format!(
                "`{}` starts with `-`, so a package manager reads it as an option rather than \
                 as a name. No manager has a package called that.",
                printable(name)
            )));
        }

        // A leading path separator normally signals an absolute-path injection attempt — but
        // for a path/URL-oriented backend (e.g. `link`, whose name IS a path) it is valid.
        if !Self::is_path_oriented_backend(backend)
            && (name.starts_with('/') || name.starts_with('\\'))
        {
            return Err(Error::Validation(format!(
                "Path traversal detected in name: {}",
                printable(name)
            )));
        }

        // A manager that prints a name must be able to be handed it back (V.113). `winget`'s
        // identifiers carry backslashes and braces; the grammar was taught to accept them and
        // this check was not, so `adopt` wrote rows that then failed to parse and wedged the
        // model — measured on the native sweep at `adopted.txt:78`.
        let (allowed, injection) = if Self::names_carry_windows_identifiers(backend) {
            (
                &*WINDOWS_IDENTIFIER_NAME_REGEX,
                &*SHELL_INJECTION_REGEX_WINDOWS_ID,
            )
        } else {
            (&*PACKAGE_NAME_REGEX, &*SHELL_INJECTION_REGEX)
        };

        if !allowed.is_match(name) {
            return Err(Error::Validation(format!(
                "Invalid characters in package name: {}",
                printable(name)
            )));
        }

        if injection.is_match(name) {
            return Err(Error::Validation(
                "Shell injection characters detected".into(),
            ));
        }

        Ok(())
    }

    /// Forbidden zones are matched as path prefixes on the *resolved* path, never as
    /// substrings: a substring test both misses `/etc/../etc/shadow` and rejects innocent
    /// names that merely contain a forbidden one. A path that does not exist is returned
    /// unresolved and unchecked — callers must not treat this as proof it is allowed.
    pub async fn validate_path(path: &Path) -> Result<PathBuf> {
        if !tokio::fs::try_exists(path).await.unwrap_or(false) {
            return Ok(path.to_path_buf());
        }

        let path_owned = path.to_path_buf();
        let canonical = tokio::task::spawn_blocking(move || path_owned.canonicalize())
            .await
            .map_err(|e| Error::Other(e.to_string()))?
            .map_err(|e| Error::Validation(format!("Path resolution failed: {}", e)))?;

        Self::refuse_forbidden(&canonical)?;
        Ok(canonical)
    }

    /// [`Validator::validate_path`] for a caller that cannot await — the `vars` standard
    /// library's `read_file`, which runs inside Rhai.
    pub fn validate_path_sync(path: &Path) -> Result<PathBuf> {
        if !path.exists() {
            return Ok(path.to_path_buf());
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| Error::Validation(format!("Path resolution failed: {}", e)))?;
        Self::refuse_forbidden(&canonical)?;
        Ok(canonical)
    }

    fn refuse_forbidden(canonical: &Path) -> Result<()> {
        for forbidden in FORBIDDEN_PATHS {
            if canonical.starts_with(Path::new(forbidden)) {
                warn!(
                    "Security Block: Attempted access to forbidden path: {:?}",
                    canonical
                );
                return Err(Error::Permission(format!("Access Denied: {}", forbidden)));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_validation_blocks_absolute_paths_for_normal_backends() {
        // A leading slash on an ordinary package name is an injection attempt.
        assert!(Validator::validate_package_name("/etc/passwd").is_err());
        assert!(Validator::validate_package_name_for("/etc/passwd", "apt").is_err());
        assert!(Validator::validate_package_name_for("ripgrep", "apt").is_ok());
        // github owner/repo and web URLs (no leading slash) pass either way.
        assert!(Validator::validate_package_name_for("BurntSushi/ripgrep", "github").is_ok());
    }

    #[test]
    fn path_oriented_backends_allow_absolute_paths_but_never_traversal() {
        // `link` legitimately names a filesystem path — an absolute path is allowed.
        assert!(Validator::validate_package_name_for("/home/me/.vimrc", "link").is_ok());
        assert!(Validator::validate_package_name_for("/tmp/shall-link-src", "link").is_ok());
        // …but `..` traversal is STILL blocked, for every backend including path-oriented ones.
        assert!(Validator::validate_package_name_for("/home/../etc/shadow", "link").is_err());
        assert!(Validator::validate_package_name_for("../secrets", "link").is_err());
        // …and shell-injection characters are still blocked (backslash, $(), etc.).
        assert!(Validator::validate_package_name_for("/tmp/$(rm -rf)", "link").is_err());
    }

    /// `btrfs:` names a subvolume by its filesystem path — `btrfs subvolume create <path>` is
    /// the whole install — and it was absent from the path-oriented list until 2026-07-30. So
    /// the one backend whose name is *literally* a filesystem path was the one the list forgot,
    /// and no declaration of it could be written:
    ///
    /// ```text
    /// $ shall -y install btrfs:/mnt/data/vol
    /// Error: Validation error: Path traversal detected in name: /mnt/data/vol
    /// ```
    ///
    /// Found by the first privileged container run in the project's history, because until
    /// there was a real btrfs filesystem to install into, nothing ever tried.
    ///
    /// The family is every backend whose name may begin with a separator. `lvm:vg/lv`,
    /// `zfs:pool/dataset` and `setting:SCHEMA/KEY` all carry a separator and never a leading
    /// one, so the strict rule is right for them and they are asserted here to keep it that way.
    #[test]
    fn a_backend_whose_name_is_a_path_may_say_so_and_the_others_may_not() {
        for good in [
            "/mnt/data/vol",
            "/mnt/shall-btrfs/canary",
            "/.snapshots/root",
        ] {
            assert!(
                Validator::validate_package_name_for(good, "btrfs").is_ok(),
                "`btrfs:{good}` is a subvolume path and the install runs `subvolume create` on it"
            );
        }
        // The bans that make widening the list narrow: traversal, injection, and the allowlist.
        assert!(Validator::validate_package_name_for("/mnt/../etc/shadow", "btrfs").is_err());
        assert!(Validator::validate_package_name_for("/mnt/$(id)", "btrfs").is_err());

        // The siblings that must NOT be widened — each names a path-shaped thing that is not a
        // filesystem path, so a leading separator is still an injection attempt.
        for (name, backend) in [
            ("/vg0/data", "lvm"),
            ("/tank/data", "zfs"),
            ("/org.gnome.desktop/idle-delay", "setting"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_err(),
                "`{backend}:` names {name} with no leading separator; allowing one would widen \
                 the guard for nothing"
            );
        }
    }

    /// A name that opens with `-` is an option, whatever the terminator table believes.
    ///
    /// `--` is the mechanism that keeps a name out of a manager's option parser, and the table
    /// recording which managers honour it is fifty booleans measured on one image. `composer`
    /// was in the honouring set and is not — so `composer:--version` reached composer's argv as
    /// a flag (B5). One wrong row should not be the whole of the defence.
    #[test]
    fn a_name_that_opens_with_a_hyphen_is_an_option_and_is_refused_everywhere() {
        for (name, backend) in [
            ("--version", "composer"),
            ("-rf", "apt"),
            ("--force", "npm"),
            ("-", "cargo"),
            // Including the backends whose names are legitimately paths: a leading `-` is a
            // flag to the program being run, whatever the string is a name *of*.
            ("--no-preserve-root", "link"),
            ("-x", "winget"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_err(),
                "`{backend}:{name}` reaches the manager's argv as an option"
            );
        }

        // A hyphen anywhere else is ordinary, and most package names have one.
        for (name, backend) in [
            ("python3-dev", "apt"),
            ("gcc-14-base", "apt"),
            ("left-pad", "npm"),
            ("ripgrep-all", "cargo"),
            ("/etc/x-y", "link"),
            (r"ARP\Machine\X64\7-Zip", "winget"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_ok(),
                "`{backend}:{name}` is an ordinary name and was refused"
            );
        }
    }

    /// The strings that reach a manager before anything knows what kind of string they are.
    ///
    /// The proof of concept was `shall search 'q&calc.exe&rem'` launching `calc.exe` through a
    /// `.cmd` shim, and a bare `q>PROOF` in a module writing a file. Both are asserted here
    /// alongside the shapes that must keep working, because a gate that refuses an ordinary
    /// search query is a gate somebody removes.
    #[test]
    fn a_string_that_becomes_argv_may_not_carry_a_command_separator() {
        for hostile in [
            "q&calc.exe&rem",
            "q>MANIFEST-REDIR.txt",
            "q|whoami",
            "q<in.txt",
            "q^&calc",
            "jq; rm -rf /",
            "jq$(id)",
            "jq`id`",
            "jq\nsecond-line",
            "say \"hi\"",
        ] {
            assert!(
                Validator::refuse_command_metacharacters(hostile, "a search query").is_err(),
                "`{hostile}` reaches a `.cmd` shim as part of a command line"
            );
        }

        // What must still pass. A query is free text — spaces, globs and regex anchors are how
        // people search — and a winget identifier carries backslashes and braces. Refusing
        // these would move the cost of the fix onto every ordinary user.
        for ordinary in [
            "ripgrep",
            "json parser",
            "rip*",
            "ri?grep",
            "@angular/cli",
            "sharkdp/fd",
            r"ARP\Machine\X64\{8BD2A40D-67A6-45F5-877D-6D9D04C9D5A2}",
            r"ARP\Machine\X64\Mozilla Firefox",
            "python3.11-dev",
            "libssl-dev:amd64",
            "[tool]",
            "!important",
        ] {
            assert!(
                Validator::refuse_command_metacharacters(ordinary, "a search query").is_ok(),
                "`{ordinary}` is something a person types and it was refused"
            );
        }
    }

    /// `winget`'s own identifiers, and the four things widening the allowlist must NOT do.
    ///
    /// Found by running the native sweep: the grammar was taught to accept a backslash in a
    /// name (G-2) and this validator was not, so `adopt` wrote 340 winget rows it believed it
    /// could write and the next command could not parse the file — `adopted.txt:78`, a wedged
    /// model, which is E1's class arriving through the other door.
    #[test]
    fn winget_identifiers_are_names_and_the_widening_stops_there() {
        for name in [
            r"ARP\Machine\X64\{8BD2A40D-67A6-45F5-877D-6D9D04C9D5A2}",
            r"ARP\Machine\X86\ILST_30_2_1",
            r"MSIX\Microsoft.AV1VideoExtension_2.0.24.0_x64__8wekyb3d8bbwe",
            "7zip.7zip",
            // The space, which is 6 of the 161 names this box could not declare.
            r"ARP\Machine\X64\Mozilla Firefox",
        ] {
            assert!(
                Validator::validate_package_name_for(name, "winget").is_ok(),
                "winget prints `{name}` and cannot be handed it back"
            );
        }

        // 0. The space is legal *inside* a name and never at its edge. A trailing space is
        //    invisible in a manifest, in a listing and in every message about either, so
        //    `Firefox ` and `Firefox` would be one package that reconciles as two — and a name
        //    made only of spaces names nothing while looking like a name. The grammar refuses
        //    the same shapes; a rule kept on one side of a round trip is a rule the other side
        //    breaks.
        for edged in [
            r"ARP\Machine\X64\Mozilla Firefox ",
            r" ARP\Machine\X64\Mozilla Firefox",
            " ",
            "   ",
        ] {
            assert!(
                Validator::validate_package_name_for(edged, "winget").is_err(),
                "`{edged}` was accepted, so an invisible edge is a second package"
            );
        }

        // 1. Only winget. Every other backend keeps the strict allowlist.
        assert!(
            Validator::validate_package_name_for(r"ARP\Machine\X64\thing", "cargo").is_err(),
            "the widening leaked to a backend whose manager never prints such a name"
        );
        // 2. Traversal is still forbidden, for winget as for everything else.
        assert!(
            Validator::validate_package_name_for(r"ARP\..\..\Windows\System32", "winget").is_err(),
            "`..` must stay refused whatever else the name may carry"
        );
        // 3. The shell metacharacters that are NOT part of a Windows identifier stay blocked.
        for hostile in [
            r"ARP\Machine; rm -rf /",
            r"ARP\Machine`whoami`",
            r"ARP\Machine$(id)",
            r"ARP\Machine|cat",
        ] {
            assert!(
                Validator::validate_package_name_for(hostile, "winget").is_err(),
                "`{hostile}` is not an identifier, it is a command line"
            );
        }
        // 4. And the ordinary names every other backend depends on still pass.
        for (name, backend) in [
            ("@angular/cli", "npm"),
            ("serde_json", "cargo"),
            ("sharkdp/fd", "github"),
        ] {
            assert!(
                Validator::validate_package_name_for(name, backend).is_ok(),
                "`{name}` stopped being a legal {backend} name"
            );
        }
    }
}
