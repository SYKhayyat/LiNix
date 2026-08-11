//! GRADER, 2026-07-28 — RED. A Windows shim that eats the exit code.
//!
//! `windows_effective_command` asks `which::which` for the manager and wraps whatever comes
//! back. `which` honours `PATHEXT`, and the Windows default `PATHEXT` does not list `.PS1`.
//! Where a manager ships both shims — scoop ships `scoop.cmd` and `scoop.ps1` side by side —
//! `which` therefore returns the `.cmd`, Shall takes the `cmd /C` branch, and `cmd /C` does
//! not propagate the child's exit code.
//!
//! Measured on this host, same failing command down each branch:
//!
//!     cmd /C ...\scoop.cmd install definitely-not-real-xyz123      -> exit 0
//!     powershell -Command "$o = (scoop install '...' | Out-String); ...; exit $LASTEXITCODE"
//!                                                                  -> exit 1
//!
//! So the branch Shall uses reports success for a failed install, and the branch that reports
//! it correctly is already written, twenty lines above, and unreachable on a default box.
//! Every scoop verdict then rests on `ExitPolicy` string-matching stdout, which is one
//! upstream wording change away from silence.

/// Which interpreter Shall would really launch this manager through.
fn branch_for(mgr: &str) -> String {
    let (prog, argv) = shall::core::executor::effective_command(mgr, &["--version".to_string()]);
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

    // **Measured, not inferred.** This assertion used to read `which::which("scoop")`'s file
    // extension — which is the resolver's answer, not Shall's launch, and the defect was never
    // in the resolver. `which` still returns `scoop.cmd` after the fix, because `PATHEXT` still
    // has no `.PS1`; what changed is that Shall no longer launches what `which` handed it. A
    // check on the extension would therefore have stayed red over a working program, which is
    // the mirror image of the checks this file exists to replace.
    //
    // So: run a command that genuinely fails, the way Shall runs it, and require the failure
    // to survive. `scoop info <name>` is read-only and changes nothing on the machine.
    let probe: Vec<String> = ["info".to_string(), "shall-no-such-pkg-zzz".to_string()].into();
    let (prog, argv) = shall::core::executor::effective_command("scoop", &probe);
    let code = std::process::Command::new(&prog)
        .args(&argv)
        .output()
        .expect("the shim should launch")
        .status
        .code()
        .unwrap_or(-1);

    assert_ne!(
        code,
        0,
        "Shall launches scoop as `{launch}` and a command that fails came back 0.\n\
         which::which resolved {} (PATHEXT has no .PS1), and `cmd /C` does not propagate the\n\
         child's exit code — measured on this host, a failing `scoop install` exits 0 through\n\
         that branch and 1 through the `.ps1` branch windows_shim_wrap already implements.\n\
         A `.ps1` shim exists at {}. Prefer it, or every scoop verdict is decided by\n\
         string-matching stdout.",
        resolved.display(),
        ps1.display(),
    );

    // And the control for the assertion above: a command that succeeds must still say so, or
    // "non-zero" would be satisfied by a launch that is simply broken.
    let ok_probe: Vec<String> = ["--version".to_string()].into();
    let (prog, argv) = shall::core::executor::effective_command("scoop", &ok_probe);
    let ok_code = std::process::Command::new(&prog)
        .args(&argv)
        .output()
        .expect("the shim should launch")
        .status
        .code()
        .unwrap_or(-1);
    assert_eq!(
        ok_code, 0,
        "`scoop --version` came back {ok_code} through `{launch}` — the launch is broken, so \
         the failure above proves nothing."
    );
    let _ = ext;
}

/// The same question asked of every manager on the host, so the fix is not scoop-shaped.
///
/// Measured, not inferred from the extension. `cmd /C` propagates the errorlevel of the
/// script's last command perfectly well — `npm.cmd` ends in the node invocation and returns 1
/// exactly as its `.ps1` does, so calling every `.cmd` lossy would be a finding manufactured
/// out of a file suffix. The defect is only where the shim Shall picks reports 0 for a failure
/// AND a shim that reports it correctly is sitting beside it.
#[test]
fn no_manager_loses_a_failure_that_its_sibling_shim_would_have_reported() {
    // A read-only query for a name that cannot exist: it fails on every one of these tools and
    // changes nothing on the machine.
    const PROBES: &[(&str, &[&str])] = &[
        ("scoop", &["info", "shall-no-such-pkg-zzz"]),
        ("npm", &["view", "shall-no-such-pkg-zzz"]),
        ("yarn", &["info", "shall-no-such-pkg-zzz"]),
        ("gem", &["specification", "shall-no-such-pkg-zzz"]),
        ("pipx", &["runpip", "shall-no-such-pkg-zzz", "--version"]),
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

        // What Shall really launches.
        let (prog, argv) = shall::core::executor::effective_command(mgr, &owned);
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
                "{mgr}: Shall launches `{prog} {}` and gets exit {theirs} for a command that \
                 fails; the .ps1 shim at {} reports {via_ps1}",
                argv.join(" "),
                ps1.display()
            ));
        }
    }
    assert!(
        losing.is_empty(),
        "these managers report a failure as success through the shim Shall chose, while the \
         .ps1 sibling reports it correctly:\n  {}",
        losing.join("\n  ")
    );
}
