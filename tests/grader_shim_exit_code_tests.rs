//! GRADER, 2026-07-28 — RED. A Windows shim that eats the exit code.
//!
//! `windows_effective_command` asks `which::which` for the manager and wraps whatever comes
//! back. `which` honours `PATHEXT`, and the Windows default `PATHEXT` does not list `.PS1`.
//! Where a manager ships both shims — scoop ships `scoop.cmd` and `scoop.ps1` side by side —
//! `which` therefore returns the `.cmd`, LiNix takes the `cmd /C` branch, and `cmd /C` does
//! not propagate the child's exit code.
//!
//! Measured on this host, same failing command down each branch:
//!
//!     cmd /C ...\scoop.cmd install definitely-not-real-xyz123      -> exit 0
//!     powershell -Command "$o = (scoop install '...' | Out-String); ...; exit $LASTEXITCODE"
//!                                                                  -> exit 1
//!
//! So the branch LiNix uses reports success for a failed install, and the branch that reports
//! it correctly is already written, twenty lines above, and unreachable on a default box.
//! Every scoop verdict then rests on `ExitPolicy` string-matching stdout, which is one
//! upstream wording change away from silence.

#![cfg(windows)]

/// Which interpreter LiNix would really launch this manager through.
fn branch_for(mgr: &str) -> String {
    let (prog, argv) = linix::core::executor::effective_command(mgr, &["--version".to_string()]);
    format!("{} {}", prog, argv.join(" "))
}

#[test]
fn a_manager_with_both_shims_is_launched_through_the_one_that_keeps_the_exit_code() {
    if which::which("scoop").is_err() {
        eprintln!("scoop is not on this host; nothing to measure");
        return;
    }
    let resolved = which::which("scoop").unwrap();
    let ext = resolved
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    // The premise: a `.ps1` sits beside whatever `which` picked. Without this the test would
    // pass on a host where only one shim exists, which proves nothing about the choice.
    let ps1 = resolved.with_extension("ps1");
    if !ps1.exists() {
        eprintln!(
            "no .ps1 shim beside {}; nothing to choose between",
            resolved.display()
        );
        return;
    }

    let launch = branch_for("scoop");
    assert!(
        !(ext == "cmd" || ext == "bat"),
        "LiNix launches scoop as `{launch}` because which::which resolved {}.\n\
         `cmd /C` does not propagate the child's exit code — measured on this host, a failing\n\
         `scoop install` exits 0 through this branch and 1 through the `.ps1` branch that\n\
         windows_shim_wrap already implements. A `.ps1` shim exists at {}.\n\
         Prefer it, or every scoop failure is decided by string-matching stdout.",
        resolved.display(),
        ps1.display(),
    );
}

/// The same question asked of every manager on the host, so the fix is not scoop-shaped.
///
/// Measured, not inferred from the extension. `cmd /C` propagates the errorlevel of the
/// script's last command perfectly well — `npm.cmd` ends in the node invocation and returns 1
/// exactly as its `.ps1` does, so calling every `.cmd` lossy would be a finding manufactured
/// out of a file suffix. The defect is only where the shim LiNix picks reports 0 for a failure
/// AND a shim that reports it correctly is sitting beside it.
#[test]
fn no_manager_loses_a_failure_that_its_sibling_shim_would_have_reported() {
    // A read-only query for a name that cannot exist: it fails on every one of these tools and
    // changes nothing on the machine.
    const PROBES: &[(&str, &[&str])] = &[
        ("scoop", &["info", "linix-no-such-pkg-zzz"]),
        ("npm", &["view", "linix-no-such-pkg-zzz"]),
        ("yarn", &["info", "linix-no-such-pkg-zzz"]),
        ("gem", &["specification", "linix-no-such-pkg-zzz"]),
        ("pipx", &["runpip", "linix-no-such-pkg-zzz", "--version"]),
    ];

    let run = |prog: &str, argv: &[String]| -> Option<i32> {
        std::process::Command::new(prog)
            .args(argv)
            .output()
            .ok()?
            .status
            .code()
    };

    let mut losing = Vec::new();
    for (mgr, args) in PROBES {
        let Ok(resolved) = which::which(mgr) else {
            continue;
        };
        let ps1 = resolved.with_extension("ps1");
        if !ps1.exists() || resolved == ps1 {
            continue; // nothing better to choose; a different fix, not this one
        }
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        // What LiNix really launches.
        let (prog, argv) = linix::core::executor::effective_command(mgr, &owned);
        let Some(theirs) = run(&prog, &argv) else {
            continue;
        };

        // What the `.ps1` branch of windows_shim_wrap would have produced.
        let esc = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let invocation = std::iter::once(mgr.to_string())
            .chain(owned.iter().map(|a| esc(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let ps_argv: Vec<String> = vec![
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-Command".into(),
            format!(
                "$o = ({invocation} | Out-String -Width 4096); Write-Output $o; exit $LASTEXITCODE"
            ),
        ];
        let Some(via_ps1) = run("powershell", &ps_argv) else {
            continue;
        };

        if theirs == 0 && via_ps1 != 0 {
            losing.push(format!(
                "{mgr}: LiNix launches `{prog} {}` and gets exit {theirs} for a command that \
                 fails; the .ps1 shim at {} reports {via_ps1}",
                argv.join(" "),
                ps1.display()
            ));
        }
    }
    assert!(
        losing.is_empty(),
        "these managers report a failure as success through the shim LiNix chose, while the \
         .ps1 sibling reports it correctly:\n  {}",
        losing.join("\n  ")
    );
}
