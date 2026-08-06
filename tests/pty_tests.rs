//! LiNix with a real terminal on its handles.
//!
//! Nothing else in this suite, in the container harnesses, or in CI ever ran the binary
//! attached to a pty — and a defect that lived in exactly that gap made every read come back
//! empty while the package manager's own output bled to the screen and looked like an answer.
//! `script -qec` is the cheapest pty there is; the check is that what LiNix *parsed* is what
//! it *printed*, which is only true if the child's output was captured.
//!
//! The manager is a stub on `PATH`, so this proves the plumbing without needing a real apt.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

const CANARY: &str = "linix-pty-canary";

fn have(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {} >/dev/null 2>&1", cmd))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn stub(dir: &Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{}\n", body)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A throwaway `PATH` where `apt` exists and `dpkg-query` answers with two known packages.
fn fake_apt(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("linix-pty-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(root.join("cfg/groups")).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    stub(&bin, "apt", "exit 0");
    stub(&bin, "apt-mark", "exit 0");
    stub(&bin, "apt-cache", "exit 0");
    stub(
        &bin,
        "dpkg-query",
        &format!("printf '{} 9.9.9\\nlinix-pty-other 1.0\\n'", CANARY),
    );
    root
}

fn linix_under_pty(root: &Path, args: &str) -> String {
    let bin = env!("CARGO_BIN_EXE_linix");
    // `timeout` bounds the run: the failure this guards against is a child that captures the
    // terminal and waits for a keypress, which without a bound is an eternal test.
    let line = format!("timeout 60 '{}' {}", bin, args);
    let out = Command::new("script")
        .args(["-qec", &line, "/dev/null"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("LINIX_CONFIG_DIR", root.join("cfg"))
        .env("LINIX_DATA_DIR", root.join("data"))
        .output()
        .expect("script(1) should run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Rows LiNix *parsed*, counted in a form only LiNix can emit.
///
/// The naive count — occurrences of the package name — passes against the broken build,
/// because the stub's raw output bleeding through an inherited handle contains the name too.
/// That is the whole deception: on screen it looks like an answer.
fn parsed_rows(out: &str) -> usize {
    out.matches(&format!("\"name\": \"{}\"", CANARY)).count()
}

/// The whole finding in one assertion: under a pty, the rows LiNix prints are rows LiNix
/// parsed. Before the fix the child's stdout was inherited, `output.stdout` came back empty,
/// and what reached the screen was `dpkg-query`'s raw text passing for a package list.
#[test]
fn a_read_under_a_pty_is_parsed_rather_than_echoed() {
    if !have("script") || !have("timeout") {
        panic!("this check needs script(1) and timeout(1); both ship with util-linux/coreutils");
    }
    let root = fake_apt("read");
    let out = linix_under_pty(&root, "list -b apt --json");

    assert_eq!(
        parsed_rows(&out),
        1,
        "LiNix did not parse the manager's output under a pty:\n{}",
        out
    );
}

/// Piped and attached must agree. They differed by 609 rows to 1.
#[test]
fn a_pty_and_a_pipe_report_the_same_packages() {
    if !have("script") || !have("timeout") {
        panic!("this check needs script(1) and timeout(1)");
    }
    let root = fake_apt("agree");
    let bin = env!("CARGO_BIN_EXE_linix");
    let piped = Command::new(bin)
        .args(["list", "-b", "apt", "--json"])
        .env(
            "PATH",
            format!(
                "{}:{}",
                root.join("bin").display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("LINIX_CONFIG_DIR", root.join("cfg"))
        .env("LINIX_DATA_DIR", root.join("data"))
        .output()
        .expect("LiNix should run");
    let piped = String::from_utf8_lossy(&piped.stdout).into_owned();
    let under_pty = linix_under_pty(&root, "list -b apt --json");

    assert!(
        parsed_rows(&piped) > 0,
        "the piped run parsed nothing:\n{}",
        piped
    );
    assert_eq!(
        parsed_rows(&piped),
        parsed_rows(&under_pty),
        "piped and pty disagree.\npiped:\n{}\npty:\n{}",
        piped,
        under_pty
    );
}

/// G-6: the machine-readable failure class is for a program, and a terminal is not one.
///
/// `linix-failure-class: permanent` was the first line a new user saw on the first command they
/// run. The line itself is a good contract — both harnesses read it instead of guessing by
/// retrying (W35) — so it stays on a pipe and goes on a terminal. Asserted from both sides here,
/// because "it is gone" and "it is gone everywhere" are different findings.
#[test]
fn the_failure_class_line_is_for_a_pipe_and_not_for_a_terminal() {
    if !have("script") || !have("timeout") {
        panic!("this check needs script(1) and timeout(1)");
    }
    let root = fake_apt("failure-class");
    // No `priority` file was ever written for this fixture, so `sync` fails on the config —
    // the exact failure the finding was reported against.
    let under_pty = linix_under_pty(&root, "sync -y");
    assert!(
        !under_pty.contains("linix-failure-class"),
        "internal vocabulary on a user's terminal:\n{under_pty}"
    );

    let bin = env!("CARGO_BIN_EXE_linix");
    let piped = Command::new(bin)
        .args(["sync", "-y"])
        .env("LINIX_CONFIG_DIR", root.join("cfg"))
        .env("LINIX_DATA_DIR", root.join("data"))
        .output()
        .expect("LiNix should run");
    let piped = format!(
        "{}{}",
        String::from_utf8_lossy(&piped.stdout),
        String::from_utf8_lossy(&piped.stderr)
    );
    assert!(
        piped.contains("linix-failure-class:"),
        "the harnesses' one machine-readable line is gone from a pipe too, which is where they \
         read it:\n{piped}"
    );
}
