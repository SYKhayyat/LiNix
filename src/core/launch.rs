//! **How a program is found, and how it is launched.**
//!
//! Split out of `executor.rs` because it is the part with a security boundary in it, and a
//! boundary buried in the middle of a 3,425-line file is a boundary nobody re-reads. Every
//! function here answers one question — *given a name, what actually gets spawned?* — and the
//! answer is platform-shaped: on Windows a "program" is as likely to be a `.ps1` or a `.cmd`
//! shim as an executable, and the two need different launchers.
//!
//! **B-1 lived here.** The `.cmd` arm built `cmd /C <script> <args…>` by hand under a comment
//! saying `cmd` forwards arguments cleanly. It does not — it parses `&`, `>`, `<`, `|` and `^`
//! out of its own command line first — so a package name of `q&calc.exe&rem` reaching a `.cmd`
//! shim ran `calc.exe`. Meanwhile the validator's doc comment carried the safety argument for
//! the whole program: *"no package-manager command is ever a shell string — every one is
//! argv."* True everywhere except this file. The two facts never met, because nothing put them
//! in the same place.

use dashmap::DashMap;
use std::path::{Path, PathBuf};

/// Where a program lives, answered once per name per process.
///
/// `is_available()` on nearly every backend is a PATH lookup, `registry.available()` calls it
/// for all ~45 of them, and `available()` itself is called at 20+ sites — six times in
/// `AppContext` alone. On Windows a *miss* walks every PATH entry × every `PATHEXT` extension,
/// and a miss is the common case, because most registered backends are not installed on any
/// given host. The same lookup runs again on every spawn to decide how to launch the program.
///
/// One backend had already cached its own probe and the other forty-four had not; a memo here
/// closes all of them at once and dedupes across backends that probe the same program (`krew`
/// probes `kubectl`; `yay`/`paru`/`pacman` overlap).
static PATH_LOOKUP: once_cell::sync::Lazy<DashMap<String, Option<PathBuf>>> =
    once_cell::sync::Lazy::new(DashMap::new);

/// Resolve a program on PATH, from the memo.
///
/// Uses the `which` *crate* — an in-process PATH/PATHEXT search — rather than spawning the
/// external `which`/`where` program: minimal fedora/arch/alpine images do not ship `which`,
/// which made every backend read as OFFLINE there.
pub fn resolve_program(cmd: &str) -> Option<PathBuf> {
    if let Some(hit) = PATH_LOOKUP.get(cmd) {
        return hit.clone();
    }
    let found = first_runnable(cmd);
    PATH_LOOKUP.insert(cmd.to_string(), found.clone());
    found
}

/// The first candidate on PATH with bytes in it, falling back to the first candidate at all.
///
/// A Windows *app execution alias* — `%LOCALAPPDATA%\Microsoft\WindowsApps\python3.exe` — is a
/// zero-length reparse point. A configured one launches the real program; an unconfigured one
/// opens the Microsoft Store and runs nothing at all. **The two cannot be told apart by
/// inspection:** measured on this host, the *working* `python3.exe` alias is also zero bytes, and
/// it is what `which` returns first. So the rule is not "detect the dead alias" but "prefer a
/// candidate that is unambiguously a program" — which finds the real `python3` two PATH entries
/// later, and still leaves the alias in place for `winget`, which has no other form.
fn first_runnable(cmd: &str) -> Option<PathBuf> {
    let first = which::which(cmd).ok()?;
    if has_bytes(&first) {
        return Some(first);
    }
    which::which_all(cmd)
        .ok()
        .and_then(|mut all| all.find(|candidate| has_bytes(candidate)))
        .or(Some(first))
}

pub(crate) fn has_bytes(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.len() > 0)
        .unwrap_or(false)
}

pub fn program_exists(cmd: &str) -> bool {
    resolve_program(cmd).is_some()
}

/// Drop the memo, for the one case where PATH really does change mid-run: Shall has just
/// installed the program it is about to ask about.
///
/// Without this, `shall init` would install a manager and then keep answering from the
/// lookup it took before the installer ran.
pub fn forget_path_lookups() {
    PATH_LOOKUP.clear();
    #[cfg(windows)]
    LAUNCH_PATH.clear();
}

/// Windows: the path a program is actually launched through, memoised.
///
/// Resolving it means a PATH scan *and* a `.ps1` stat beside the resolved shim, and it ran on
/// every single spawn — synchronously, inside an `async fn`, on the same task the planner's
/// `buffer_unordered` relies on to interleave.
#[cfg(windows)]
static LAUNCH_PATH: once_cell::sync::Lazy<DashMap<String, Option<PathBuf>>> =
    once_cell::sync::Lazy::new(DashMap::new);

#[cfg(windows)]
fn launch_path(cmd: &str) -> Option<PathBuf> {
    if let Some(hit) = LAUNCH_PATH.get(cmd) {
        return hit.clone();
    }
    let plan = resolve_program(cmd).map(|resolved| preferred_shim(&resolved));
    LAUNCH_PATH.insert(cmd.to_string(), plan.clone());
    plan
}

/// Windows only: some tools on PATH are not `.exe` files but shim scripts —
/// e.g. scoop ships as `scoop.ps1`. `where`/`which` find them (so availability checks
/// pass), but `CreateProcess` can't launch a `.ps1` directly, so a plain spawn fails with
/// "program not found". Given the resolved path, return the interpreter and argv to run it
/// through.
///
/// **A `.cmd`/`.bat` shim is returned as itself, and that is the security-relevant part.**
/// This arm used to build `cmd /C <script> <args…>` by hand, and `cmd` parses `&`, `>`, `<`,
/// `|` and `^` out of its own command line before the batch file sees anything — so a package
/// name of `q&calc.exe&rem` reaching a `.cmd` shim ran `calc.exe`. `std` spawns a batch file
/// through `cmd.exe /e:ON /v:OFF /d /c` with per-argument escaping of exactly those
/// characters, and returns an error rather than mis-escaping an argument it cannot express.
/// Handing it the resolved path is therefore both the fix and the deletion of a second
/// implementation: the escaping lives in one place and it is not this one.
#[cfg(windows)]
fn windows_shim_wrap(cmd: &str, resolved: &Path, args: &[String]) -> Option<(String, Vec<String>)> {
    let ext = resolved.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ps1" => {
            // PowerShell tools like scoop emit *objects*, which only render when PowerShell
            // formats them. `-File`, `& 'path'`, and a trailing `; exit` all cause the
            // buffered table to be dropped when stdout is captured. The form that reliably
            // yields text AND propagates the exit code: invoke through the call operator (so
            // the tool's own output formatting kicks in), pipe through Out-String into a
            // variable, emit it, then exit with the tool's last exit code.
            //
            // **The program name is escaped the same way the arguments are.** The claim here
            // used to be that a crafted package name could not break out of the string and so
            // there was no command-injection surface. That was true of the arguments and not
            // of `cmd`, which was concatenated in raw. Nothing user-authored reaches `cmd`
            // today — manager names come from the compiled registry and `managers.toml` — so
            // the invariant was held up by nobody having added a config key for a manager's
            // binary name. That is B-1's shape exactly: a safety argument written for the
            // whole program, true everywhere except this file.
            let esc = |s: &str| format!("'{}'", s.replace('\'', "''"));
            // `&` because a single-quoted string is a string until the call operator makes it
            // a command, and bare interpolation is what made escaping impossible here.
            let mut invocation = format!("& {}", esc(cmd));
            for a in args {
                invocation.push(' ');
                invocation.push_str(&esc(a));
            }
            // **`exit $LASTEXITCODE` alone reports a failure as a success.** That variable is
            // set by a *native* process. A failure at the PowerShell level — the command not
            // found, the `.ps1` missing, an exception thrown — leaves it `$null`, and `exit
            // $null` exits 0. `ensure_status` reads 0 as success, so a `scoop install` that
            // never ran was recorded as installed. That is the defect the `.cmd` arm was
            // rejected for, one arm over: it was measured returning 0 for a failed install and
            // this branch was chosen because it returns a real code — which it does, for
            // native failures only.
            //
            // So the terminating failures are caught and the catch is the exit code, while
            // `$LASTEXITCODE` still decides whenever the tool actually ran. **Deliberately no
            // `$ErrorActionPreference = 'Stop'`:** it would make the tool's own non-terminating
            // errors fatal, and scoop emits those on installs that succeed — a bound on the
            // wrapper must not rewrite the callee's error semantics. Measured against the
            // installed scoop: `scoop list` gives identical output and the same exit code as
            // the old form, `scoop install <missing>` still exits 1, and a missing command or
            // `.ps1` now exits 1 where it used to exit 0.
            let command = format!(
                "try {{ $o = ({} | Out-String -Width 4096); $native = $LASTEXITCODE; \
                 Write-Output $o }} catch {{ Write-Output $_; exit 1 }}; \
                 exit $(if ($null -ne $native) {{ $native }} else {{ 0 }})",
                invocation
            );
            Some((
                "powershell".to_string(),
                vec![
                    "-NoProfile".to_string(),
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-Command".to_string(),
                    command,
                ],
            ))
        }
        // Named rather than folded into the `_` arm: the resolved path is what must be
        // spawned, not the bare `cmd` name the caller passed. Resolution is how the preferred
        // shim gets picked when a manager ships more than one, and `std` keys its batch-file
        // handling off the extension of the program it is given.
        "cmd" | "bat" => Some((resolved.to_string_lossy().to_string(), args.to_vec())),
        _ => None,
    }
}

/// Resolve the actual (program, args) to spawn on Windows, wrapping shim scripts. Bare
/// `.exe`/native commands pass through unchanged.
#[cfg(windows)]
pub(crate) fn windows_effective_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    if let Some(resolved) = launch_path(cmd) {
        if let Some(wrapped) = windows_shim_wrap(cmd, &resolved, args) {
            return wrapped;
        }
    }
    (cmd.to_string(), args.to_vec())
}

/// The shim to actually launch, when a manager ships more than one.
///
/// `which::which` honours `PATHEXT`, and the Windows default `PATHEXT` does not list `.PS1`.
/// scoop ships `scoop.cmd` and `scoop.ps1` side by side, so on a default box `which` returns
/// the `.cmd`, the `cmd /C` arm below runs it — and **`cmd /C` does not propagate the child's
/// exit code**. Measured on this host, the same failing install:
///
/// ```text
/// cmd /C ...\scoop.cmd install <bad>   -> exit 0
/// the `.ps1` branch                    -> exit 1
/// ```
///
/// So the careful PowerShell arm that already knew how to return `$LASTEXITCODE` was dead code
/// on every default installation, and every scoop verdict fell back to string-matching stdout —
/// one upstream wording change away from reporting a failed install as a success.
///
/// The choice is made on what is on disk, never on `PATHEXT`: a machine's `PATHEXT` is a user
/// setting, and a correctness property that depends on one is a property that holds on the
/// developer's box and not the user's.
#[cfg(windows)]
fn preferred_shim(resolved: &Path) -> std::path::PathBuf {
    let ext = resolved
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "cmd" || ext == "bat" {
        let ps1 = resolved.with_extension("ps1");
        if ps1.is_file() {
            return ps1;
        }
    }
    resolved.to_path_buf()
}

/// How Shall actually launches `cmd` on this platform.
///
/// On Windows a manager is usually a `.cmd`/`.ps1` shim that `Command::new` cannot execute at
/// all, so the real launch goes through an interpreter. Anything that runs a manager the way
/// Shall runs it must come through here — including the argv-drift gate, which asks each
/// manager about its own subcommands and was skipping every shimmed one on this platform as
/// "its help could not be read". A gate that launches programs differently from the product is
/// testing a different program from the one that ships.
pub fn effective_command(cmd: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        windows_effective_command(cmd, args)
    }
    #[cfg(not(windows))]
    {
        (cmd.to_string(), args.to_vec())
    }
}

/// Name a spawned command well enough to find it again, in one line.
///
/// `powershell` alone does not identify a hang on a host running six of them; the argv is what
/// says which. Truncated from the right because the discriminating part is the front — the
/// cmdlet, the subcommand, the package name.
pub(crate) fn describe(cmd: &str, args: &[String]) -> String {
    const CAP: usize = 160;
    let mut s = String::from(cmd);
    for a in args {
        s.push(' ');
        s.push_str(a);
        if s.len() >= CAP {
            break;
        }
    }
    if s.chars().count() > CAP {
        s = s.chars().take(CAP).collect::<String>() + "…";
    }
    s
}

#[cfg(all(test, windows))]
mod windows_shim_tests {
    use super::windows_shim_wrap;
    use std::path::Path;

    #[test]
    fn wraps_ps1_via_command_with_out_string() {
        let (prog, args) = windows_shim_wrap(
            "scoop",
            Path::new(r"C:\tools\scoop\shims\scoop.ps1"),
            &["search".to_string(), "ripgrep".to_string()],
        )
        .expect("ps1 should be wrapped");
        assert_eq!(prog, "powershell");
        assert!(args.contains(&"-Command".to_string()));
        let command = args.last().unwrap();
        assert!(command.contains("& 'scoop' 'search' 'ripgrep' | Out-String"));
        assert!(command.contains("$native = $LASTEXITCODE"));
    }

    /// **A PowerShell-level failure must not exit 0.**
    ///
    /// `exit $LASTEXITCODE` propagates a *native* process's code and nothing else; the command
    /// not being found, the `.ps1` being gone, an exception thrown all leave it `$null`, and
    /// `exit $null` exits 0 — which `ensure_status` reads as a successful install. The catch is
    /// what covers those, and `$LASTEXITCODE` still decides whenever the tool actually ran.
    #[test]
    fn a_powershell_level_failure_is_not_reported_as_success() {
        let (_prog, args) =
            windows_shim_wrap("scoop", Path::new(r"C:\s.ps1"), &["install".to_string()]).unwrap();
        let command = args.last().unwrap();
        assert!(command.contains("catch"), "no catch: {command}");
        assert!(
            command.contains("exit 1"),
            "the catch must exit non-zero: {command}"
        );
        assert!(
            !command.contains("exit $LASTEXITCODE"),
            "the bare form is the defect: {command}"
        );
        // Not `Stop`: the tool's own non-terminating errors are the tool's business, and scoop
        // emits them on installs that succeed.
        assert!(!command.contains("ErrorActionPreference"), "{command}");
    }

    /// **The program name is escaped like every argument.** Nothing user-authored reaches
    /// `cmd` today; what kept that true was that no config key names a manager's binary, which
    /// is not an enforcement.
    #[test]
    fn the_program_name_is_quoted_and_invoked_through_the_call_operator() {
        let hostile = "evil'; rm x; '";
        let (_prog, args) =
            windows_shim_wrap(hostile, Path::new(r"C:\s.ps1"), &["list".to_string()]).unwrap();
        let command = args.last().unwrap();
        assert!(
            command.contains(&format!("& '{}'", hostile.replace('\'', "''"))),
            "the program name must stay one literal: {command}"
        );
    }

    #[test]
    fn ps1_args_are_single_quote_escaped_no_injection() {
        let (_prog, args) = windows_shim_wrap(
            "scoop",
            Path::new(r"C:\s.ps1"),
            &["install".to_string(), "evil'; rm x; '".to_string()],
        )
        .unwrap();
        // The embedded quote is doubled so the whole thing stays one literal string.
        assert!(args.last().unwrap().contains("'evil''; rm x; '''"));
    }

    /// A `.cmd`/`.bat` shim is spawned as itself. `std` recognises the extension and runs it
    /// through `cmd.exe` with per-argument escaping; building `cmd /C …` here instead is what
    /// let a package name of `q&calc.exe&rem` launch `calc.exe` (B-1).
    #[test]
    fn a_cmd_shim_is_spawned_as_itself_never_through_a_hand_built_cmd_c() {
        for ext in ["cmd", "bat"] {
            let path = format!(r"C:\x\foo.{ext}");
            let (prog, args) =
                windows_shim_wrap("foo", Path::new(&path), &["list".to_string()]).unwrap();
            assert_eq!(prog, path, "the resolved shim is the program to spawn");
            assert_eq!(
                args,
                vec!["list".to_string()],
                "argv passes through unchanged"
            );
        }
    }

    /// The regression proper: a crafted name stays **one argument**. Whatever escaping the
    /// spawn needs is `std`'s to apply, and it cannot apply any if we have already flattened
    /// the name into a command line.
    #[test]
    fn a_crafted_package_name_stays_one_argument_through_a_cmd_shim() {
        let hostile = "q&calc.exe&rem";
        let (prog, args) = windows_shim_wrap(
            "gem",
            Path::new(r"C:\Ruby\bin\gem.cmd"),
            &["search".to_string(), hostile.to_string()],
        )
        .unwrap();
        assert!(prog.ends_with("gem.cmd"));
        assert_eq!(args, vec!["search".to_string(), hostile.to_string()]);
        // No argument is `/C`, and none is a command line with the name spliced into it —
        // the two shapes that let `cmd` reparse it.
        assert!(!args.iter().any(|a| a == "/C"));
        assert!(!args.iter().any(|a| a.contains("gem.cmd")));
    }

    #[test]
    fn leaves_exe_alone() {
        assert!(windows_shim_wrap("winget", Path::new(r"C:\x\winget.exe"), &[]).is_none());
    }
}
