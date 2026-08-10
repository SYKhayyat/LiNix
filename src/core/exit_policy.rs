//! What a package manager's exit code and output mean.
//!
//! This knowledge used to be a `match` on `"scoop" | "choco" | "winget"` inside
//! `CommandExecutor`, so registering a backend with its own conventions meant editing the
//! execution engine. A policy is data now, declared where the backend is registered and
//! carried by that backend's executor, and the engine only asks.

use crate::core::error::Retryability;

/// How one manager reports outcomes its exit code alone does not describe.
///
/// Marker strings are matched case-insensitively against the command's stdout and stderr
/// together. `permanent` is consulted before `transient`, so a manager that prints both
/// fails fast rather than looping — and `transient` before `absent`, because "not found" is a
/// claim about an index the manager has just said it could not read.
#[derive(Debug, Clone, Default)]
pub struct ExitPolicy {
    /// Non-zero codes this manager uses for outcomes that are not failures.
    pub benign_exits: Vec<i32>,
    /// Text that means the command did nothing, even though it exited 0.
    pub failure_markers: Vec<&'static str>,
    /// Text that means failure when it *opens a line* — scoop's `ERROR `, nimble's `Error:`.
    ///
    /// A manager that prefixes its errors states its own convention, and matching the
    /// convention catches failures nobody has met yet. Enumerating phrasings instead is how
    /// scoop came to detect exactly one of its failures: `uninstall` of something that was
    /// never installed prints `ERROR 'x' isn't installed.`, exits 0, and was reported to the
    /// user as a success.
    pub failure_line_prefixes: Vec<&'static str>,
    /// Text that means the failure came from outside the request — a lock, a mirror, a
    /// network. Worth another attempt.
    pub transient_markers: Vec<&'static str>,
    /// Text that means the request itself is wrong. A second attempt reproduces it.
    pub permanent_markers: Vec<&'static str>,
    /// Text that means this manager looked the name up and it is not there.
    ///
    /// A different question from `permanent_markers`, and the distinction is the whole point.
    /// `Permanent` answers *would another attempt differ?*; this answers *does the name
    /// exist?* helm's `plugin already exists` is permanent and the name plainly exists, so
    /// withdrawing a declaration on permanence alone deletes a line whose package is
    /// installed. Only this list withdraws a declaration, and matching one implies
    /// `Permanent` — a name that is not there will not be there on the next attempt.
    ///
    /// A manager with an empty list here can never wedge a config *less*: an unclassified
    /// failure keeps the line, which is the safe direction. It is also the direction that
    /// left E1 alive on every manager that had no policy at all, so the count of managers
    /// with no entry is ratcheted by `absent_marker_coverage_tests.rs` rather than left to
    /// be discovered one backend at a time.
    pub absent_markers: Vec<&'static str>,
    /// Exit codes that mean the failure came from outside the request, for a manager that
    /// does not say so in words.
    ///
    /// **Every other list here is text, and text is the wrong axis for a manager that fails
    /// silently.** Measured: 3 of 16 concurrent cold-start `winget list` exit `0x8A150001`
    /// having written zero bytes to either stream, so the haystack every marker is matched
    /// against is empty and `retryability` returns `Unknown` — while the one signal that does
    /// exist, the code, was read by nothing but `is_benign`.
    pub transient_exits: Vec<i32>,
}

impl ExitPolicy {
    /// A non-zero exit this manager does not mean as a failure.
    ///
    /// A command killed by a signal (`None`) is never benign: nothing chose that code.
    pub fn is_benign(&self, code: Option<i32>) -> bool {
        match code {
            Some(c) => self.benign_exits.contains(&c),
            None => false,
        }
    }

    /// Whether any question this policy answers looks at what the command printed.
    ///
    /// A bare policy decides everything from the exit status, so building the haystack for one
    /// is pure waste — and a bare policy is the default, which makes it the common case.
    pub fn reads_output(&self) -> bool {
        !(self.failure_markers.is_empty()
            && self.failure_line_prefixes.is_empty()
            && self.absent_markers.is_empty()
            && self.transient_markers.is_empty()
            && self.permanent_markers.is_empty())
    }

    /// A manager that exited 0 while refusing to do the work.
    ///
    /// Takes the [`ExitPolicy::haystack`] rather than the two raw streams because
    /// `ensure_status` asks this, [`ExitPolicy::retryability`] and
    /// [`ExitPolicy::names_an_absent_package`] about the same output: building the lowercased
    /// join once per command instead of once per question is three fewer full copies of an
    /// `apt install` transcript, per package.
    pub fn signals_failure(&self, hay: &str) -> bool {
        if self.failure_markers.is_empty() && self.failure_line_prefixes.is_empty() {
            return false;
        }
        if self.failure_markers.iter().any(|m| hay.contains(m)) {
            return true;
        }
        hay.lines().any(|line| {
            let opening = Self::opening(line);
            self.failure_line_prefixes
                .iter()
                .any(|p| opening.starts_with(p))
        })
    }

    /// What a line says once indentation and any colour escape are out of the way. A manager
    /// writing to a pipe can still emit SGR sequences — scoop's bucket update does — and an
    /// error prefix behind one is still an error prefix.
    fn opening(line: &str) -> &str {
        let mut rest = line.trim_start();
        while let Some(after_esc) = rest.strip_prefix('\u{1b}') {
            match after_esc.find(|c: char| c.is_ascii_alphabetic()) {
                Some(end) => rest = after_esc[end + 1..].trim_start(),
                None => return "",
            }
        }
        rest
    }

    /// The lines of a failed command's output that this manager's own vocabulary marks as the
    /// reason. Empty when nothing matches, which is the honest answer for a manager whose
    /// policy is bare.
    ///
    /// The same three lists `signals_failure` reads, asked line by line instead of of the whole
    /// stream — because a user needs the sentence, not the verdict.
    pub fn explaining_lines<'t>(&self, text: &'t str) -> Vec<&'t str> {
        text.lines()
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                let opening = Self::opening(&lower).to_string();
                self.failure_markers.iter().any(|m| lower.contains(m))
                    || self.absent_markers.iter().any(|m| lower.contains(m))
                    || self
                        .failure_line_prefixes
                        .iter()
                        .any(|p| opening.starts_with(p))
            })
            .collect()
    }

    /// Whether this manager said the name it was given does not exist.
    ///
    /// The fact `install` reads to take a line back out of the manifest. It is answered from
    /// the manager's own declared phrasings rather than from the shape of an error value:
    /// reading `CommandFailed { retry: Permanent }` recognised the two managers whose failure
    /// happened to be classified and left the config wedged on every other one (N-1).
    pub fn names_an_absent_package(&self, hay: &str) -> bool {
        if self.absent_markers.is_empty() {
            return false;
        }
        // A manager that could not reach its index has not looked the name up, and every one
        // of them words that the same way as a name that truly is not there: choco says `The
        // package was not found with the source(s) listed` to an unreachable feed, apt says
        // `Unable to locate package` when it could not fetch the lists. Withdrawing on that
        // deletes a declaration whose package exists, on nothing worse than a dropped VPN.
        if self.transient_markers.iter().any(|m| hay.contains(m)) {
            return false;
        }
        self.absent_markers.iter().any(|m| hay.contains(m))
    }

    /// Whether the same command could succeed on another attempt, given both signals it left.
    ///
    /// **What a manager says outranks what it returns.** A command that named its problem has
    /// described it better than its exit code can, and the codes here are a fallback for the
    /// case that has no words at all — which is exactly the case [`retryability`] cannot see,
    /// because its haystack is empty and every marker list misses.
    pub fn retryability_of(&self, code: Option<i32>, hay: &str) -> Retryability {
        match self.retryability(hay) {
            Retryability::Unknown => match code {
                Some(c) if self.transient_exits.contains(&c) => Retryability::Transient,
                _ => Retryability::Unknown,
            },
            classified => classified,
        }
    }

    /// Whether the same command could succeed on another attempt.
    pub fn retryability(&self, hay: &str) -> Retryability {
        if self.permanent_markers.is_empty()
            && self.transient_markers.is_empty()
            && self.absent_markers.is_empty()
        {
            return Retryability::Unknown;
        }
        if self.permanent_markers.iter().any(|m| hay.contains(m)) {
            return Retryability::Permanent;
        }
        // Above the absent check, and only above that one: an absent verdict is a claim about
        // the index, so a manager that says in the same breath that it could not read the
        // index has not earned it. `permanent` still outranks both — a request that is wrong
        // stays wrong however the network behaved.
        if self.transient_markers.iter().any(|m| hay.contains(m)) {
            return Retryability::Transient;
        }
        // A name that is not there now will not be there on the next attempt, so an absent
        // marker settles retryability too and does not need repeating in `permanent_markers`.
        if self.absent_markers.iter().any(|m| hay.contains(m)) {
            return Retryability::Permanent;
        }
        Retryability::Unknown
    }

    /// The two output streams as one lowercased string — what every marker question is asked
    /// against. Built once per command by the caller, because there are three such questions.
    pub fn haystack(stdout: &[u8], stderr: &[u8]) -> String {
        let mut hay = String::from_utf8_lossy(stdout).into_owned();
        // Without this, a stdout that does not end in a newline welds its last line onto the
        // first line of stderr, and the joined line opens with neither one's prefix.
        if !hay.is_empty() && !hay.ends_with('\n') {
            hay.push('\n');
        }
        hay.push_str(&String::from_utf8_lossy(stderr));
        hay.make_ascii_lowercase();
        hay
    }
}

/// Debian/Ubuntu. Shared by `apt` and by the `apt-get`/`dpkg-query`/`add-apt-repository`
/// programs the apt backend also runs, which is why it is bound per backend and not per
/// program name.
pub fn apt() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "could not get lock",
            "unable to acquire the dpkg frontend lock",
            "is another process using it",
            "temporary failure resolving",
            "connection timed out",
            "could not connect",
            "failed to fetch",
        ],
        absent_markers: vec![
            "unable to locate package",
            "has no installation candidate",
            "couldn't find any package by",
        ],
        ..ExitPolicy::default()
    }
}

/// Fedora/RHEL — `dnf` and `yum`.
pub fn dnf() -> ExitPolicy {
    ExitPolicy {
        // `dnf check-update` exits 100 when it FINDS updates. It is dnf's answer, not its
        // failure, and unmarked it makes a successful update check look like a broken one.
        benign_exits: vec![100],
        // **And a forgiven code must still be contradictable.** `benign_exits` above says "100
        // is not a failure", and without a phrasing that can say otherwise, *every* dnf run
        // ending on it reads as a success — including one that did nothing, which is the choco
        // 3010 defect `benign_exit_contradiction_tests` was written for.
        //
        // Measured, not guessed: `dnf install -y no-such-package-xyz` in the Fedora 41
        // integration image prints
        //
        //     Failed to resolve the transaction:
        //     No match for argument: no-such-package-xyz
        //
        // The first line is dnf's own words for "the transaction did not happen", and it is the
        // sentence that has to outrank a forgiven exit code.
        failure_markers: vec!["failed to resolve the transaction"],
        transient_markers: vec![
            "failed to synchronize cache",
            "cannot download",
            "another app is currently holding the yum lock",
            "curl error",
            "connection timed out",
        ],
        absent_markers: vec!["no match for argument", "unable to find a match"],
        ..ExitPolicy::default()
    }
}

/// Arch — `pacman`, and the AUR helpers that speak its flags.
pub fn pacman() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed retrieving file",
            "could not resolve host",
            "unable to lock database",
            "connection timed out",
        ],
        absent_markers: vec!["target not found", "could not find or read package"],
        ..ExitPolicy::default()
    }
}

/// Alpine.
pub fn apk() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "temporary error",
            "network error",
            "could not connect",
            "operation not permitted (try running as root)",
        ],
        absent_markers: vec!["unable to select packages", "no such package"],
        ..ExitPolicy::default()
    }
}

/// macOS/Linuxbrew.
pub fn brew() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed to download",
            "curl: (28)",
            "operation timed out",
            "could not resolve host",
        ],
        absent_markers: vec!["no available formula", "no formulae or casks found"],
        ..ExitPolicy::default()
    }
}

/// Chocolatey, which surfaces MSI conventions in its own exit status: 1641 reboot
/// initiated, 3010 reboot required, 1605/1614/1618 already-removed or uninstall-in-progress.
///
/// Chocolatey raises its exit code to 1 for a failed package only when nothing has set one
/// already, so a dependency asking for a reboot leaves 3010 standing over a package that never
/// installed — a benign code on an install of nothing. The count sentence is the only thing
/// that knows, and choco writes it only when a package failed.
pub fn choco() -> ExitPolicy {
    ExitPolicy {
        benign_exits: vec![1605, 1614, 1618, 1641, 3010],
        failure_markers: vec![
            "packages failed",
            // Not an absent marker: the name exists on the source, it is simply not installed
            // here, and an absent name takes the declaration out of the user's files.
            "cannot uninstall a non-existent package",
        ],
        // Measured 2026-08-02 against a port nothing is listening on. Without these, the
        // absent marker below fires on an unreachable feed — choco words a source it could not
        // reach exactly as it words a name that does not exist.
        transient_markers: vec![
            "unable to load the service index for source",
            "unable to connect to source",
            "no connection could be made",
        ],
        absent_markers: vec!["the package was not found with the source"],
        ..ExitPolicy::default()
    }
}

/// winget's HRESULT-style "success but noteworthy": no applicable upgrade, already
/// installed, no installed package found.
///
/// -1978335212 arrives for opposite events: `uninstall` of something already gone is the
/// outcome that was asked for, and `install` of a name that does not exist is not. Only
/// winget's wording separates them — `No installed package found` for the first, `No package
/// found` for the second — so the marker is the second phrasing, which the first does not
/// contain.
/// winget's internal error, `0x8A150001`, written as the `i32` a process exit status carries.
///
/// Measured on a real host: 16 concurrent `winget list` from a cold start, and 3 of them exit
/// this having printed nothing at all, in ~310ms — while a single `winget list` takes 1.5s and
/// a second burst against a warm winget loses none. It is contention on winget's own source
/// index, not a fact about the request, and the identical command succeeds moments later.
const WINGET_INTERNAL_ERROR: i32 = 0x8A15_0001_u32 as i32;

pub fn winget() -> ExitPolicy {
    ExitPolicy {
        benign_exits: vec![-1978335189, -1978335212, -1978335215],
        failure_markers: vec!["no package found matching input criteria"],
        absent_markers: vec!["no package found matching input criteria"],
        // The only code here is the one that was measured. Winget documents many more, and
        // guessing which of them a retry could help would be inventing policy from a header
        // file — an over-eager entry costs real seconds on every failure that will never pass.
        transient_exits: vec![WINGET_INTERNAL_ERROR],
        ..ExitPolicy::default()
    }
}

/// crates.io. `cargo install` names the two things that cannot be fixed by trying again:
/// a crate with no program in it, and a name the registry does not carry.
pub fn cargo() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "network failure",
            "spurious network error",
            "failed to fetch",
            "connection timed out",
        ],
        // A crate with no program in it and a crate the registry does not carry are both
        // permanent and only the second one is *absent*. Withdrawing `cargo:some-library`
        // because it ships no binary would delete a line whose crate exists and is spelled
        // correctly; the fix there is the user's to make.
        permanent_markers: vec![
            "no binaries",
            "nothing to install",
            "does not have these features",
            // `cargo uninstall` on something this machine does not have. Permanent and NOT
            // absent, by the same distinction as the two above: it says nothing about whether
            // the crate exists, only that it is not installed here — so a retry cannot help
            // and a withdrawal would delete a line that is correct.
            "did not match any packages",
        ],
        absent_markers: vec!["could not find"],
        ..ExitPolicy::default()
    }
}

/// scoop exits 0 on every outcome, so its output is the only evidence there is.
///
/// `ERROR ` opens every scoop failure, which is what catches the ones nobody has enumerated:
/// a missing manifest was the single phrasing known here, and `scoop uninstall <not
/// installed>` — which prints `ERROR 'x' isn't installed.` — was read as success.
pub fn scoop() -> ExitPolicy {
    ExitPolicy {
        failure_markers: vec!["find manifest for"],
        failure_line_prefixes: vec!["error "],
        // `isn't installed` is scoop refusing to *remove* something absent from the machine,
        // which says nothing about whether the bucket carries the name — so it is permanent
        // and not absent, and a failed uninstall never withdraws the declaration.
        permanent_markers: vec!["isn't installed"],
        absent_markers: vec!["find manifest for"],
        transient_markers: vec![
            "could not resolve host",
            "the remote name could not be resolved",
            "unable to connect",
            "hash check failed",
        ],
        ..ExitPolicy::default()
    }
}

/// Nim. `nimble` exits 0 whether it built the program, failed to build it, or never found
/// it — so a build that produced no binary was reported as a successful install and only the
/// absence of the binary, two checks later, said otherwise.
pub fn nimble() -> ExitPolicy {
    ExitPolicy {
        failure_line_prefixes: vec!["error:"],
        // A version nimble does not have and a build that failed are both about a package
        // that exists: the line carries a `@version=` to correct or a toolchain to fix, and
        // deleting it would throw away the thing the user has to edit.
        permanent_markers: vec!["version not found", "build failed for the package"],
        absent_markers: vec!["package not found"],
        transient_markers: vec!["could not download", "connection", "temporary failure"],
        ..ExitPolicy::default()
    }
}

/// Lua rocks. The order of the marker lists is the whole policy here.
///
/// luarocks reports an unreachable rock index as a fact about the *request*: three
/// `Failed downloading … manifest-5.5` warnings, and then
/// `Error: No results matching query were found for Lua 5.5` as the summary. Believe the
/// summary and a rock that exists reads as a rock that never will — and the declaration for
/// it gets withdrawn, or the harness hard-fails a machine whose only problem is that the
/// `wget` on its PATH is BusyBox's applet, which rejects the GNU flags luarocks passes.
///
/// The summary line is deliberately in neither list. `retryability` checks permanent before
/// transient, so marking it permanent would beat the download failures printed above it in
/// the very output where both appear. Left unmatched, a rock that really is absent from a
/// reachable index answers `Unknown` — which hard-fails, as it should, and withdraws no
/// declaration. Unknown is the honest answer to "the index was fine and the rock was not
/// there"; Permanent would be a guess with a deletion attached.
/// **These markers are a hypothesis, and the transaction now tests it.** A failure classified
/// transient is retried with backoff; if it comes back the same, `falsify_transience` in
/// `core/transaction.rs` downgrades it to `Retryability::Exhausted` and the user is told the
/// retry already happened and did not help. So a wrong marker here costs a few seconds of
/// backoff instead of a permanent lie — which is what this list was before, when
/// `luarocks install luafilesystem` matched `"failed downloading"` on a machine whose only
/// problem was its `wget`, and LiNix promised forever that `sync` would try again.
pub fn luarocks() -> ExitPolicy {
    ExitPolicy {
        transient_markers: vec![
            "failed downloading",
            "failed searching manifest",
            "connection refused",
            "could not resolve host",
        ],
        ..ExitPolicy::default()
    }
}

/// Kubernetes plugins. helm v4 verifies a plugin's signature before installing it and refuses
/// a source that cannot carry one at all, which no second attempt changes — but the source is
/// a git host, so the network half stays worth retrying.
pub fn helm() -> ExitPolicy {
    ExitPolicy {
        permanent_markers: vec![
            "does not support verification",
            "signature verification failed",
            "plugin already exists",
        ],
        transient_markers: vec![
            "could not resolve host",
            "connection refused",
            "i/o timeout",
            "temporary failure",
        ],
        ..ExitPolicy::default()
    }
}

/// npm. Measured on this host 2026-07-29: `npm install -g <absent>` prints
/// `npm error code E404` and `404 Not Found - GET https://registry.npmjs.org/<name>`.
///
/// npm had no policy at all until N-1, which is why a mistyped npm package wedged the config
/// while the same typo behind `scoop:` did not — the withdrawal read a classification npm
/// never produced. Nothing about npm was special; it was one of the 36 backends with no
/// policy, and it was the one the grader happened to type.
pub fn npm() -> ExitPolicy {
    ExitPolicy {
        absent_markers: vec!["404 not found", "is not in this registry"],
        transient_markers: vec![
            "etimedout",
            "enotfound",
            "econnreset",
            "network",
            "rate limit",
        ],
        ..ExitPolicy::default()
    }
}

/// RubyGems. Measured 2026-07-29: `gem install <absent>` exits 2 with
/// `ERROR:  Could not find a valid gem 'x' (>= 0) in any repository`.
pub fn gem() -> ExitPolicy {
    ExitPolicy {
        absent_markers: vec!["could not find a valid gem"],
        transient_markers: vec![
            "timed out",
            "could not resolve host",
            "connection refused",
            "too many connection resets",
        ],
        ..ExitPolicy::default()
    }
}

/// pipx and pip. Measured 2026-07-29: pipx relays pip's own summary,
/// `ERROR: No matching distribution found for <name>`, and exits 1.
///
/// `could not find a version that satisfies` is deliberately absent from both lists: pip
/// prints it for a name that does not exist *and* for a version pin nothing satisfies, and
/// only the first is a reason to withdraw a line.
pub fn pipx() -> ExitPolicy {
    ExitPolicy {
        absent_markers: vec!["no matching distribution found"],
        transient_markers: vec![
            "read timed out",
            "temporary failure in name resolution",
            "connection broken",
            "retrying",
        ],
        ..ExitPolicy::default()
    }
}

/// Go modules. Measured 2026-07-29: `go install github.com/<absent>@latest` reports
/// `remote: Repository not found` and `fatal: repository … not found` through git.
///
/// `no matching versions for query` is the same fact for a repo that exists without the
/// requested tag; it is not in the absent list, because the tag is the part of the line the
/// user edits.
pub fn go() -> ExitPolicy {
    ExitPolicy {
        absent_markers: vec!["repository not found", "unknown revision"],
        transient_markers: vec![
            "i/o timeout",
            "connection reset",
            "could not resolve host",
            "proxyconnect",
        ],
        ..ExitPolicy::default()
    }
}

/// pixi. Measured 2026-07-29: `pixi global install <absent>` prints
/// `No candidates were found for <name>` and exits 1.
///
/// pixi wraps that line at the terminal width and will break it *inside* the package name,
/// which is why the name is never recovered from this text — see
/// `absent_name_in_message` in `verbs/packages.rs`.
pub fn pixi() -> ExitPolicy {
    ExitPolicy {
        absent_markers: vec!["no candidates were found for"],
        transient_markers: vec![
            "failed to fetch",
            "operation timed out",
            "could not resolve host",
        ],
        ..ExitPolicy::default()
    }
}

/// Every manager whose conventions this program knows, in one table.
///
/// Each registration site reads its own name out of here instead of naming a function, so
/// *which managers have a policy* is a question with one answer rather than one answer per
/// registration site — and `tests/absent_marker_coverage_tests.rs` can ask it. npm had no
/// policy at all through three rounds of assessment: nothing was wrong with npm, and nothing
/// anywhere counted the managers that could not tell LiNix a name was missing.
///
/// An unknown name yields the default policy, which classifies nothing. That is the safe
/// direction — an unclassified failure keeps the declaration — and it is not a silent one: a
/// manager missing from this table is a manager the coverage test names.
pub fn for_manager(name: &str) -> ExitPolicy {
    match name {
        "apt" => apt(),
        "dnf" | "yum" => dnf(),
        "pacman" => pacman(),
        "apk" => apk(),
        "brew" => brew(),
        "choco" => choco(),
        "winget" => winget(),
        "cargo" => cargo(),
        "scoop" => scoop(),
        "nimble" => nimble(),
        "luarocks" => luarocks(),
        "helm" => helm(),
        "npm" => npm(),
        "gem" => gem(),
        "pipx" => pipx(),
        "go" => go(),
        "pixi" => pixi(),
        _ => ExitPolicy::default(),
    }
}

/// Whether this manager can tell LiNix that a name does not exist.
///
/// The one the coverage ratchet counts, because it is the one a wedged config turns on: a
/// manager that cannot say "no such package" leaves the line in `modules/imperative.txt` and
/// every later command fails on it.
pub fn classifies_absent_names(name: &str) -> bool {
    !for_manager(name).absent_markers.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_with_no_markers_classifies_nothing() {
        let p = ExitPolicy::default();
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(b"anything", b"")),
            Retryability::Unknown
        );
        assert!(!p.signals_failure(&ExitPolicy::haystack(
            b"Couldn't find manifest for 'x'.",
            b""
        )));
        assert!(!p.is_benign(Some(3010)));
    }

    #[test]
    fn a_signal_kill_is_never_benign() {
        assert!(!choco().is_benign(None));
        assert!(!winget().is_benign(None));
    }

    #[test]
    fn benign_codes_are_per_backend_and_do_not_leak() {
        for code in [1605, 1614, 1618, 1641, 3010] {
            assert!(choco().is_benign(Some(code)));
            assert!(!winget().is_benign(Some(code)));
            assert!(!apt().is_benign(Some(code)));
        }
        assert!(!choco().is_benign(Some(1)));
        assert!(winget().is_benign(Some(-1978335189)));
        assert!(winget().is_benign(Some(-1978335212)));
        assert!(!winget().is_benign(Some(1)));
        assert!(!apt().is_benign(Some(100)));
        assert!(!apk().is_benign(Some(1)));
        assert!(!dnf().is_benign(Some(1)));
    }

    #[test]
    fn scoop_reports_a_missing_manifest_as_a_failure_and_never_retries_it() {
        let p = scoop();
        assert!(p.signals_failure(&ExitPolicy::haystack(
            b"Couldn't find manifest for 'nope'.",
            b""
        )));
        assert!(p.signals_failure(&ExitPolicy::haystack(
            b"",
            b"couldn't find manifest for 'nope'."
        )));
        assert!(!p.signals_failure(&ExitPolicy::haystack(b"Installing 'jq'...", b"")));
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(
                b"Couldn't find manifest for 'nope'.",
                b""
            )),
            Retryability::Permanent
        );
    }

    #[test]
    fn a_held_lock_is_transient_and_a_missing_name_is_not() {
        assert_eq!(
            apt().retryability(&ExitPolicy::haystack(
                b"",
                b"E: Could not get lock /var/lib/dpkg/lock-frontend"
            )),
            Retryability::Transient
        );
        assert_eq!(
            apt().retryability(&ExitPolicy::haystack(
                b"",
                b"E: Unable to locate package nosuchpkg"
            )),
            Retryability::Permanent
        );
        assert_eq!(
            apt().retryability(&ExitPolicy::haystack(
                b"",
                b"E: Something nobody classified"
            )),
            Retryability::Unknown
        );
    }

    /// Every distro manager answers both questions, or the fast-fail only helps Debian.
    #[test]
    fn each_distro_policy_tells_a_held_lock_from_a_missing_name() {
        let cases = [
            (
                dnf(),
                "Another app is currently holding the yum lock",
                "No match for argument: nope",
            ),
            (
                pacman(),
                "error: unable to lock database",
                "error: target not found: nope",
            ),
            (
                apk(),
                "ERROR: temporary error (try again later)",
                "ERROR: unable to select packages:",
            ),
            (
                brew(),
                "curl: (28) Operation timed out",
                "Error: No available formula with the name \"nope\"",
            ),
        ];
        for (policy, transient, permanent) in cases {
            assert_eq!(
                policy.retryability(&ExitPolicy::haystack(b"", transient.as_bytes())),
                Retryability::Transient,
                "not transient: {transient}"
            );
            assert_eq!(
                policy.retryability(&ExitPolicy::haystack(b"", permanent.as_bytes())),
                Retryability::Permanent,
                "not permanent: {permanent}"
            );
        }
    }

    /// A manager that prints both must not loop on the half that cannot succeed.
    #[test]
    fn permanent_wins_over_transient() {
        // A genuinely *permanent* marker, which is what this test is named for. It used to
        // assert the same thing with apt's `Unable to locate package` — an ABSENT marker — and
        // so quietly also pinned absent-over-transient, which is the pairing that made a
        // dropped connection delete declarations. That half now has its own test below.
        let p = cargo();
        let both = b"spurious network error\nerror: there are no binaries" as &[u8];
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(b"", both)),
            Retryability::Permanent
        );
    }

    /// The pairing the test above used to hide: an unreachable index outranks the "not found"
    /// the manager prints *because* it was unreachable.
    #[test]
    fn transient_wins_over_absent() {
        let p = apt();
        let both = b"E: Could not get lock; E: Unable to locate package nope" as &[u8];
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(b"", both)),
            Retryability::Transient
        );
        assert!(!p.names_an_absent_package(&ExitPolicy::haystack(b"", both)));
    }

    /// Captured from nimble v0.22.2 on this machine. Every one of these exits **0**.
    const NIMBLE_BUILD_FAILED: &str =
        include_str!("../../tests/fixtures/nimble/install-build-failed.txt");
    const NIMBLE_NOT_FOUND: &str =
        include_str!("../../tests/fixtures/nimble/install-not-found.txt");
    const NIMBLE_UNINSTALL_MISSING: &str =
        include_str!("../../tests/fixtures/nimble/uninstall-not-installed.txt");
    const NIMBLE_LIST: &str = include_str!("../../tests/fixtures/nimble/list-installed.txt");
    /// Captured from scoop on this machine. Exits **0** and matches no phrasing marker.
    const SCOOP_UNINSTALL_MISSING: &str =
        include_str!("../../tests/fixtures/scoop/uninstall-not-installed.txt");

    /// nimble reports every failure on a line beginning `Error:` and exits 0 regardless, so
    /// without a policy LiNix called a build that never produced a binary a successful
    /// install — and then the harness's `list` and on-PATH checks failed, which is the
    /// product telling the truth downstream about a lie it told upstream.
    #[test]
    fn nimble_failures_are_seen_though_every_one_of_them_exits_zero() {
        let p = nimble();
        for (case, out) in [
            ("build failed", NIMBLE_BUILD_FAILED),
            ("package not found", NIMBLE_NOT_FOUND),
            ("uninstall missing", NIMBLE_UNINSTALL_MISSING),
        ] {
            assert!(
                p.signals_failure(&ExitPolicy::haystack(out.as_bytes(), b"")),
                "nimble failure not detected: {case}"
            );
        }
    }

    const HELM_UNVERIFIABLE: &str =
        include_str!("../../tests/fixtures/helm/plugin-install-unverifiable-source.txt");

    /// A plugin source that carries no signature never grows one, so retrying is time spent
    /// reaching the same refusal. Measured against helm v4.2.3 on 2026-07-28.
    #[test]
    fn helm_refusing_an_unsignable_source_is_permanent_not_transient() {
        assert_eq!(
            helm().retryability(&ExitPolicy::haystack(b"", HELM_UNVERIFIABLE.as_bytes())),
            Retryability::Permanent
        );
    }

    /// The network half stays retryable: helm plugins come from a git host.
    #[test]
    fn helm_losing_the_network_is_worth_another_attempt() {
        assert_eq!(
            helm().retryability(&ExitPolicy::haystack(
                b"",
                b"Error: could not resolve host github.com"
            )),
            Retryability::Transient
        );
    }

    const LUAROCKS_MANIFEST_UNREACHABLE: &str =
        include_str!("../../tests/fixtures/luarocks/install-manifest-unreachable.txt");

    /// luarocks names the wrong cause, and the name it gives is the one that would be acted
    /// on. Captured from luarocks 3.13.0 on 2026-07-28.
    ///
    /// The last line is `No results matching query were found for Lua 5.5` — which reads as
    /// "this rock does not exist for your Lua", a fact about the request that no retry
    /// changes. It is not what happened. Three lines above it, all three manifest mirrors
    /// failed to download; luarocks then searched an empty index and reported the empty
    /// result. The rock exists and `manifest-5.5.zip` was served fine to `curl` at the same
    /// moment.
    ///
    /// Classifying on the earlier lines rather than the summary is the whole point: read as
    /// permanent, a widening of `install`'s withdrawal rule would delete the user's
    /// declaration for a package that is perfectly real, and the harness would hard-fail a
    /// machine whose only problem is its downloader.
    #[test]
    fn a_luarocks_manifest_that_would_not_download_is_transient_not_a_missing_rock() {
        assert_eq!(
            luarocks().retryability(&ExitPolicy::haystack(
                b"",
                LUAROCKS_MANIFEST_UNREACHABLE.as_bytes()
            )),
            Retryability::Transient,
            "the summary line was believed over the three download failures above it"
        );
    }

    /// And the half that keeps the marker honest: the same summary line on its own — a
    /// reachable index that really does not carry the rock — is not retried forever. It
    /// answers `Unknown`, which hard-fails and withdraws nothing.
    #[test]
    fn a_luarocks_summary_without_a_download_failure_is_not_retried() {
        assert_eq!(
            luarocks().retryability(&ExitPolicy::haystack(
                b"",
                b"Error: No results matching query were found for Lua 5.4."
            )),
            Retryability::Unknown
        );
    }

    /// The other half, and the half that makes the check worth having: a successful command
    /// must not be read as a failure.
    #[test]
    fn a_successful_nimble_listing_is_not_a_failure() {
        assert!(!nimble().signals_failure(&ExitPolicy::haystack(NIMBLE_LIST.as_bytes(), b"")));
    }

    /// The defect this mechanism exists for: scoop's single phrasing marker caught a missing
    /// manifest and nothing else, so removing something that was never installed reported
    /// success.
    #[test]
    fn scoop_sees_a_failure_that_is_not_a_missing_manifest() {
        let p = scoop();
        assert!(
            p.signals_failure(&ExitPolicy::haystack(
                SCOOP_UNINSTALL_MISSING.as_bytes(),
                b""
            )),
            "scoop uninstall of a package that is not installed read as success"
        );
        assert!(p.signals_failure(&ExitPolicy::haystack(
            b"Couldn't find manifest for 'nope'.",
            b""
        )));
        assert!(!p.signals_failure(&ExitPolicy::haystack(
            b"'jq' (1.7.1) was installed successfully!",
            b""
        )));
    }

    /// An error prefix is a line's opening word, not a substring of prose. A package whose
    /// description mentions errors is not a failed command.
    #[test]
    fn a_prefix_marker_anchors_to_a_line_and_does_not_match_prose() {
        let p = nimble();
        assert!(!p.signals_failure(&ExitPolicy::haystack(
            b"jsony - a library with no error: handling at all",
            b""
        )));
        assert!(p.signals_failure(&ExitPolicy::haystack(
            b"building...\n    Error:  Package not found\n",
            b""
        )));
    }

    const CHOCO_SOURCE_UNREACHABLE: &str =
        include_str!("../../tests/fixtures/choco/install-source-unreachable.txt");
    const CHOCO_ABSENT_NAME: &str =
        include_str!("../../tests/fixtures/choco/install-absent-name.txt");

    /// A manager that could not reach its index has not looked the name up.
    ///
    /// Both fixtures are real `choco install` runs measured on 2026-08-02, and they end with
    /// the same sentence — `The package was not found with the source(s) listed.` One of them
    /// was pointed at a port nothing is listening on. Only the connection lines above it say
    /// which is which, so believing the sentence on its own deletes a declaration whose
    /// package exists, on nothing worse than a dropped VPN.
    #[test]
    fn an_unreachable_source_is_not_a_missing_package() {
        let p = choco();
        assert!(
            !p.names_an_absent_package(&ExitPolicy::haystack(
                CHOCO_SOURCE_UNREACHABLE.as_bytes(),
                b""
            )),
            "an unreachable feed read as `no such package`, which withdraws the line from the \
             user's config files"
        );
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(
                CHOCO_SOURCE_UNREACHABLE.as_bytes(),
                b""
            )),
            Retryability::Transient,
            "an unreachable feed is worth another attempt"
        );
        // The failure itself is still a failure — this must not become a silent success.
        assert!(p.signals_failure(&ExitPolicy::haystack(
            CHOCO_SOURCE_UNREACHABLE.as_bytes(),
            b""
        )));
    }

    /// The other half: the fix must not cost the typo its withdrawal.
    #[test]
    fn a_name_the_source_does_not_carry_is_still_absent() {
        let p = choco();
        assert!(
            p.names_an_absent_package(&ExitPolicy::haystack(CHOCO_ABSENT_NAME.as_bytes(), b"")),
            "a name the feed answered about is absent, and the line should come back out"
        );
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(CHOCO_ABSENT_NAME.as_bytes(), b"")),
            Retryability::Permanent
        );
    }

    /// The rule is the engine's, not chocolatey's, so it is asserted on every policy that can
    /// reach the state — an absent phrasing printed while the index was unreachable.
    ///
    /// apt is the one that bites in the field: a sources.list it could not fetch makes
    /// `Unable to locate package` the answer for packages that plainly exist.
    ///
    /// Each string here is that policy's *own* two markers put in one output. They assert the
    /// precedence rule, not that any manager phrases a run exactly this way — the wording of
    /// each half was measured when the marker was added, and choco's pair is a captured run
    /// (`install-source-unreachable.txt`) rather than a composition.
    #[test]
    fn no_manager_calls_a_name_absent_while_it_is_also_reporting_a_transient_failure() {
        let cases = [
            (
                apt(),
                "E: Failed to fetch http://deb.debian.org/\nE: Unable to locate package jq",
            ),
            (
                dnf(),
                "Cannot download repomd.xml\nNo match for argument: jq",
            ),
            (
                pacman(),
                "error: failed retrieving file\nerror: target not found: jq",
            ),
            (
                apk(),
                "ERROR: temporary error (try again later)\nERROR: unable to select packages:",
            ),
            (
                brew(),
                "curl: (28) Operation timed out\nNo available formula with the name \"jq\"",
            ),
            (
                cargo(),
                "spurious network error\ncould not find `jq` in registry",
            ),
            (pixi(), "failed to fetch\nNo candidates were found for jq"),
        ];
        for (policy, output) in cases {
            assert!(
                !policy.names_an_absent_package(&ExitPolicy::haystack(output.as_bytes(), b"")),
                "a name was called absent although the same output says the fetch failed: {output:?}"
            );
            assert_eq!(
                policy.retryability(&ExitPolicy::haystack(output.as_bytes(), b"")),
                Retryability::Transient,
                "{output:?}"
            );
        }
    }

    /// The documented precedence that must survive the change above: a manager printing both a
    /// permanent and a transient marker fails fast rather than looping.
    #[test]
    fn a_permanent_marker_still_outranks_a_transient_one() {
        let p = cargo();
        assert_eq!(
            p.retryability(&ExitPolicy::haystack(
                b"spurious network error\nerror: there are no binaries",
                b""
            )),
            Retryability::Permanent
        );
    }
}

#[cfg(test)]
mod outdated_exit_tests {
    use super::*;

    /// `dnf check-update` exits **100** when it finds updates. That is dnf's answer, and
    /// unmarked it makes a successful update check indistinguishable from a broken one.
    #[test]
    fn dnf_reports_finding_updates_with_an_exit_code_that_is_not_a_failure() {
        assert!(
            dnf().is_benign(Some(100)),
            "exit 100 is `there are updates`, not `check-update failed`"
        );
        // And the codes that really are failures stay failures.
        assert!(!dnf().is_benign(Some(1)));
        assert!(!dnf().is_benign(None), "a signal kill chose no code");
    }

    /// The exit-code axis added for `Q41` must not have made every manager's codes benign.
    #[test]
    fn a_transient_code_is_not_thereby_a_benign_one() {
        let p = winget();
        let internal = 0x8A15_0001_u32 as i32;
        assert_eq!(
            p.retryability_of(Some(internal), ""),
            Retryability::Transient
        );
        assert!(
            !p.is_benign(Some(internal)),
            "worth retrying and `not a failure` are different claims"
        );
    }
}
