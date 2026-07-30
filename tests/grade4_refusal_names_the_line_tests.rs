//! GRADER round 5, 2026-07-30 — RED. A name rejected by the *character* validator is refused
//! without naming the file or the line, and the refusal reprints the offending bytes raw.
//!
//! Two error classes, measured on one 60-line module with one bad line at line 40:
//!
//!     cargo:<U+202E>reversed  ->  Error: Validation error: Invalid characters in package name: <RLO>reversed
//!     cargo:<ESC>[31mred…     ->  Error: Validation error: Invalid characters in package name: [31mred[0m
//!     cargo:aaa…(300 chars)   ->  Error: Configuration error: …/big.txt:40: …          <- located
//!     cargo:rip<TAB>grep      ->  Error: Configuration error: …/big.txt:40: …          <- located
//!
//! The grammar's own refusals are excellent and name `file:line`. The character validator's name
//! neither — and the character at fault is a bidi override, a NUL or an escape, so it cannot be
//! found by looking at the file either. That is the worst pairing available: an unlocatable error
//! about an invisible character.
//!
//! And it echoes. Byte-level, in a Linux container so neither a pipe nor a terminal could be
//! inventing it:
//!
//!     === the manifest, in bytes:
//!     0000000   c   a   r   g   o   : 033   [   3   1   m   r   e   d 033   [ …
//!     === what linix prints, in bytes:
//!     0000060   …   n   a   m   e   :     033   [   3   1   m   r   e   d 033 …
//!     0000060   …   n   a   m   e   :     342 200 256   r   e   v   e   r   s …
//!
//! `342 200 256` is U+202E RIGHT-TO-LEFT OVERRIDE — the trojan-source character — reprinted raw by
//! the message whose entire subject is that those characters are invalid. Manifests arrive from
//! shared configs as well as from the user's own hand, which is what lifts this above
//! self-inflicted.
//!
//! The rule being asserted is `READINESS` §8.1's A+ line and `GRADER` §4's: **every user-visible
//! failure names the file or command the user can act on.**
//!
//! A note from writing this file: the first draft pasted the measured output into the comment
//! above verbatim, and `rustc` refused to compile it — *"unicode codepoint changing visible
//! direction of text present in doc comment"*, deny-by-default. The compiler will not let this
//! text through even in a comment. `<RLO>` above is where that character was.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The offending line is buried at 40 of 60, because an error with no location is only a problem
/// when there is somewhere for it to hide.
const BAD_LINE: usize = 40;

fn fixture(name: &str, bad: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let (out, code) = run(&root, &["init"]);
    assert_eq!(code, 0, "the fixture's own `init` failed:\n{out}");

    let mut lines: Vec<String> = (1..BAD_LINE).map(|i| format!("cargo:pkg{i}")).collect();
    lines.push(bad.to_string());
    lines.extend((BAD_LINE + 1..=60).map(|i| format!("cargo:pkg{i}")));
    std::fs::write(
        root.join("config").join("modules").join("big.txt"),
        lines.join("\n") + "\n",
    )
    .unwrap();

    let profile = root.join("config").join("profiles").join("Main");
    let mut p = std::fs::read_to_string(&profile).unwrap();
    p.push_str("\nuse big\n");
    std::fs::write(&profile, p).unwrap();
    root
}

fn run(dir: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .current_dir(dir)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// U+202E RIGHT-TO-LEFT OVERRIDE, and an ANSI colour sequence: the two that a user cannot see in
/// their editor, and therefore the two that most need the line number.
#[test]
fn a_refusal_about_an_invisible_character_names_the_line() {
    let cases = [
        ("rtl", "cargo:\u{202E}reversed"),
        ("ansi", "cargo:\u{1b}[31mred\u{1b}[0m"),
    ];
    let mut unlocated = Vec::new();

    for (tag, bad) in cases {
        let dir = fixture(&format!("grade4-loc-{tag}"), bad);
        let (out, code) = run(&dir, &["eval"]);
        assert_ne!(
            code, 0,
            "the fixture was accepted, so nothing was refused:\n{out}"
        );

        if !out.contains(&format!("big.txt:{BAD_LINE}")) {
            unlocated.push(format!(
                "{tag}: {}",
                out.lines().next().unwrap_or("").trim()
            ));
        }
    }

    assert!(
        unlocated.is_empty(),
        "a refusal named neither the file nor the line, for a character the user cannot see in \
         their editor:\n  {}\n\nThe control is in the same file: a 300-character name and a tab \
         both produce `…/big.txt:{BAD_LINE}: …`. Only the character validator's refusals lose the \
         location.",
        unlocated.join("\n  ")
    );
}

/// The control that makes the test above mean something: the grammar's refusals over the same
/// fixture DO name the line, so a failure above is about the validator and not about the harness.
#[test]
fn the_grammars_own_refusal_names_the_line() {
    let dir = fixture("grade4-loc-control", "notabackend:thing");
    let (out, code) = run(&dir, &["eval"]);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains(&format!("big.txt:{BAD_LINE}")),
        "even the grammar's refusal lost the location, so this fixture proves nothing about the \
         validator:\n{out}"
    );
}

/// A refusal must not hand the terminal the control bytes it is complaining about. A bidi
/// override reverses the rest of the line as it renders; an ANSI sequence recolours it.
#[test]
fn a_refusal_does_not_reprint_the_control_bytes_it_rejects() {
    let cases: [(&str, &str, &[u8]); 2] = [
        ("rtl", "cargo:\u{202E}reversed", &[0xE2, 0x80, 0xAE]),
        ("ansi", "cargo:\u{1b}[31mred\u{1b}[0m", &[0x1b]),
    ];
    let mut echoed = Vec::new();

    for (tag, bad, bytes) in cases {
        let dir = fixture(&format!("grade4-echo-{tag}"), bad);
        let out = Command::new(env!("CARGO_BIN_EXE_linix"))
            .arg("eval")
            .current_dir(&dir)
            .env("LINIX_CONFIG_DIR", dir.join("config"))
            .env("LINIX_DATA_DIR", dir.join("data"))
            .env("NO_COLOR", "1")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("the binary should run");

        let mut all = out.stdout.clone();
        all.extend_from_slice(&out.stderr);
        if all.windows(bytes.len()).any(|w| w == bytes) {
            echoed.push(tag);
        }
    }

    assert!(
        echoed.is_empty(),
        "the refusal reprinted the control bytes it was rejecting, verbatim, for: {}\n\nNO_COLOR=1 \
         was set, so this is the manifest's bytes and not LiNix's own styling. U+202E is the \
         trojan-source character; the message that says the characters are invalid renders under \
         their control.",
        echoed.join(", ")
    );
}

// ---------------------------------------------------------------------------------------
// BUILDER round 6 — the family. The two tests above drive one command, `eval`, with two
// characters. The class is "a refusal that skipped the location decoration" and "a message
// that hands the terminal bytes it took from a file", and neither is a property of `eval`.
//
// Driven through the binary across every command that reads the model, so the assertion is
// about what a user sees rather than about which `format!` a fix happened to touch. A fix
// applied at one call site passes the tests above and fails these.
// ---------------------------------------------------------------------------------------

/// The characters that do something to a terminal, and the bytes that prove it.
const HOSTILE: [(&str, char, &[u8]); 5] = [
    ("rtl", '\u{202E}', &[0xE2, 0x80, 0xAE]), // RIGHT-TO-LEFT OVERRIDE
    ("lri", '\u{2066}', &[0xE2, 0x81, 0xA6]), // LEFT-TO-RIGHT ISOLATE
    ("esc", '\u{1b}', &[0x1b]),               // ANSI introducer
    ("zwsp", '\u{200B}', &[0xE2, 0x80, 0x8B]), // ZERO WIDTH SPACE
    ("ls", '\u{2028}', &[0xE2, 0x80, 0xA8]),  // LINE SEPARATOR
];

/// Every command a hostile manifest is read by. `eval` was the only one measured.
const READERS: [&[&str]; 5] = [
    &["eval"],
    &["check", "config"],
    &["check"],
    &["--dry-run", "sync", "-y"],
    &["why", "cargo:anything"],
];

fn raw(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(env!("CARGO_BIN_EXE_linix"))
        .args(args)
        .current_dir(dir)
        .env("LINIX_CONFIG_DIR", dir.join("config"))
        .env("LINIX_DATA_DIR", dir.join("data"))
        .env("NO_COLOR", "1")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    let mut all = out.stdout;
    all.extend_from_slice(&out.stderr);
    all
}

#[test]
fn no_command_reprints_a_control_character_it_read_from_a_manifest() {
    let mut leaks = Vec::new();

    for (tag, ch, bytes) in HOSTILE {
        let dir = fixture(&format!("grade4-fam-echo-{tag}"), &format!("cargo:{ch}bad"));
        for args in READERS {
            if raw(&dir, args).windows(bytes.len()).any(|w| w == bytes) {
                leaks.push(format!("`linix {}` leaked {tag}", args.join(" ")));
            }
        }
    }

    assert!(
        leaks.is_empty(),
        "these commands handed the terminal the control bytes they read out of a module:\n  {}\n\n\
         NO_COLOR=1 was set, so nothing here is LiNix's own styling. Escaping one message is not \
         the fix — the name reaches a user through whichever command reads the model first.",
        leaks.join("\n  ")
    );
}

/// The location half of the family, over the same sweep. A refusal a user cannot locate is
/// the same defect whichever command made it.
#[test]
fn every_command_that_refuses_a_hostile_name_says_where_it_is() {
    let mut unlocated = Vec::new();

    for (tag, ch, _) in HOSTILE {
        let dir = fixture(&format!("grade4-fam-loc-{tag}"), &format!("cargo:{ch}bad"));
        for args in [READERS[0], READERS[1], READERS[3]] {
            let (out, code) = run(&dir, args);
            if code == 0 {
                // This command accepted the fixture, so it refused nothing to locate.
                continue;
            }
            if !out.contains(&format!("big.txt:{BAD_LINE}")) {
                unlocated.push(format!(
                    "`linix {}` for {tag}: {}",
                    args.join(" "),
                    out.lines()
                        .find(|l| l.contains("rror"))
                        .unwrap_or("")
                        .trim()
                ));
            }
        }
    }

    assert!(
        unlocated.is_empty(),
        "a refusal named neither the file nor the line:\n  {}",
        unlocated.join("\n  ")
    );
}
