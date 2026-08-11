//! Nothing writes an escape sequence into a pipe.
//!
//! `main` built the tracing subscriber with no `.with_ansi(…)`, so `tracing-subscriber`'s own
//! default decided it — and that default is colour, always, regardless of where the writer
//! points. Measured:
//!
//! ```text
//! $ linix install nosuchpkg-zzz 2>&1 | grep -c $'\033'   → 1   (piped, still coloured)
//! $ NO_COLOR=1 …                                          → 0   (respected)
//! $ TERM=dumb  …                                          → 1   (ignored)
//! ```
//!
//! Two faults, one line apart. The subscriber never asked whether stderr was a terminal, and
//! `utils::style::color_enabled` — the function that does ask — asks about **stdout**, which is
//! the wrong stream for a diagnostic writer and is routinely a pipe while stderr is not.
//! `TERM=dumb` was honoured by nothing at all.
//!
//! A test process's child has no controlling terminal on either stream, so every command below
//! is in the "piped" case by construction. That is the case that matters: escape codes in a log
//! file are escape codes somebody greps through a year later.

use std::process::{Command, Stdio};

const ESC: char = '\u{1b}';

/// Commands that are *meant* to say something on stderr. A command that prints nothing cannot
/// fail this test, so the point is to pick ones that warn or refuse.
const NOISY: &[&[&str]] = &[
    &["install", "nosuchpkg-zzz-does-not-exist", "-y"],
    &["sync", "-y"],
    &["info", "nosuchbackend-zzz:foo"],
    &["list"],
    &["check"],
];

fn run(args: &[&str], env: &[(&str, &str)]) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ansi-is-for-terminals");
    std::fs::create_dir_all(dir.join("config/modules")).unwrap();
    std::fs::create_dir_all(dir.join("config/profiles")).unwrap();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let _ = std::fs::write(dir.join("config/priority"), "\n");
    let _ = std::fs::write(dir.join("config/active"), "");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_linix"));
    cmd.args(args)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
        .env_remove("NO_COLOR")
        .env_remove("TERM")
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the binary should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn no_command_writes_an_escape_sequence_into_a_pipe() {
    let mut coloured = Vec::new();
    for args in NOISY {
        let out = run(args, &[]);
        if out.contains(ESC) {
            coloured.push(format!(
                "`linix {}` — first offending line: {:?}",
                args.join(" "),
                out.lines()
                    .find(|l| l.contains(ESC))
                    .unwrap_or("<no line>")
                    .chars()
                    .take(120)
                    .collect::<String>()
            ));
        }
    }
    assert!(
        coloured.is_empty(),
        "{} command(s) wrote ANSI escape sequences to a stream that is not a terminal:\n  {}\n\n\
         Neither stdout nor stderr is a tty here, so this is what a user's `2>&1 | tee \
         install.log` gets. The subscriber must ask before it colours \
         (`style::color_enabled_on_stderr`).",
        coloured.len(),
        coloured.join("\n  ")
    );
}

/// The two environment conventions, both of them, on the noisiest command.
///
/// `NO_COLOR` was already honoured for stdout and is asserted here so a refactor cannot lose it.
/// `TERM=dumb` was honoured by nothing — it is the answer a terminal gives when it *is* a
/// terminal and still cannot render escapes, which is the one case `is_terminal` gets wrong.
#[test]
fn the_environment_can_switch_colour_off_by_either_convention() {
    for env in [
        vec![("NO_COLOR", "1")],
        vec![("TERM", "dumb")],
        vec![("NO_COLOR", "1"), ("TERM", "xterm-256color")],
    ] {
        let out = run(&["install", "nosuchpkg-zzz-does-not-exist", "-y"], &env);
        assert!(
            !out.contains(ESC),
            "with {env:?} set, LiNix still wrote escape sequences:\n{out}"
        );
    }
}
