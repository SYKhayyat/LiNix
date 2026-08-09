//! A config file saved by a Windows editor starts with a byte-order mark, and every file LiNix
//! reads has to work anyway (`Q22`, ruled 2026-07-31).
//!
//! Notepad writes UTF-8 **with** a BOM by default, and so does PowerShell 5.1's
//! `Set-Content -Encoding utf8` — which is the shell this project is developed in. The three
//! bytes `EF BB BF` are an encoding artefact, not content, and no editor shows them. Before the
//! ruling they became part of the first name on the first line:
//!
//! ```text
//! $ linix eval
//! Error: …/modules/starter.txt:1: `<U+FEFF>cargo` is not a backend LiNix uses
//!   add `<U+FEFF>cargo` to your `priority` file, or check the spelling.
//! ```
//!
//! Two names that render identically, and advice that cannot be followed — the user did add
//! `cargo` to `priority`. (The `<U+FEFF>` in that message is itself a fix from the same day; it
//! used to print the invisible character raw, so the two names looked *actually* identical.)
//!
//! **Every file, not the one that was reported.** A rule that covered `modules/` only would send
//! the next user to the same message from `priority`, which is exactly where the advice points.


const BOM: &str = "\u{feff}";

use crate::harness::Fixture;

/// The reported case: the module carries the mark.
#[test]
fn a_module_saved_with_a_byte_order_mark_resolves() {
    let f = Fixture::new("bom-module");
    f.write("priority", "cargo\n");
    f.write("modules/starter.txt", &format!("{BOM}cargo:ripgrep\n"));

    let (out, code) = f.run(&["eval"]);
    assert_eq!(
        code, 0,
        "a module saved by Notepad did not resolve:\n{out}\n\nThe first three bytes are an \
         encoding artefact no editor shows; they must not become part of the backend name."
    );
    assert!(
        out.contains(r#""backend": "cargo""#),
        "the backend name still carries the mark:\n{out}"
    );
}

/// The file the failing message sends the user to — so the fix must reach it, or following the
/// advice produces the same error from the next file along.
#[test]
fn the_priority_file_may_carry_one_too() {
    let f = Fixture::new("bom-priority");
    f.write("priority", &format!("{BOM}cargo\n"));
    f.write("modules/starter.txt", "cargo:ripgrep\n");

    let (out, code) = f.run(&["eval"]);
    assert_eq!(
        code, 0,
        "a `priority` saved by Notepad was not read:\n{out}"
    );
    assert!(
        out.contains(r#""backend": "cargo""#),
        "the declaration did not resolve against a BOM-prefixed priority:\n{out}"
    );
}

/// And the rest of the family, in one run: every line file the model reads, each carrying one.
/// `active` and `profiles/` decide what is declared at all, so a mark there is not a cosmetic
/// failure — it is a machine that converges to the wrong set.
#[test]
fn every_line_file_may_carry_one() {
    let f = Fixture::new("bom-family");
    f.write("priority", &format!("{BOM}cargo\n"));
    f.write("vars", &format!("{BOM}role = builder\n"));
    f.write("modules/starter.txt", &format!("{BOM}cargo:ripgrep\n"));
    f.write("profiles/Work", &format!("{BOM}use starter\n"));
    f.write("active", &format!("{BOM}Work\n"));

    let (out, code) = f.run(&["eval"]);
    assert_eq!(
        code, 0,
        "a config written entirely by Notepad failed:\n{out}"
    );
    assert!(
        out.contains(r#""name": "ripgrep""#),
        "the profile chain did not resolve through the marks:\n{out}"
    );

    // `vars` is read by its own command, and a variable named `<U+FEFF>role` is a variable
    // nothing can reference.
    let (vars, code) = f.run(&["vars"]);
    assert_eq!(code, 0, "`vars` failed:\n{vars}");
    assert!(
        vars.contains("$role"),
        "the variable name kept the mark, so nothing can name it:\n{vars}"
    );
}

/// `preferences.toml` is TOML rather than a line file, and its reader is a different one — the
/// same mark, the other parser. Its failure would be the loudest of the family: an unreadable
/// preferences file stops every command LiNix has.
///
/// **This one was already green before the fix**, and saying so matters. The `toml` crate
/// tolerates a leading BOM, so nothing here was broken; the strip in `Config::load` makes the
/// rule hold for LiNix's own reasons rather than by a dependency's courtesy, and this assertion
/// is a pin on behaviour rather than a repair of it. The three line-file tests above were red.
#[test]
fn the_preferences_file_may_carry_one() {
    let f = Fixture::new("bom-preferences");
    f.write("priority", "cargo\n");
    f.write("modules/starter.txt", "cargo:ripgrep\n");
    f.write(
        "preferences.toml",
        &format!("{BOM}[guard]\nmax_removals = 7\n"),
    );

    // `protected` is the command that reads this setting back — the first draft asked `policy`,
    // which reports the install/change rules and would never have printed it. A check that
    // cannot see the thing it names passes or fails for its own reasons; this one failed, which
    // is the only reason it was caught.
    let (out, code) = f.run(&["protected"]);
    assert_eq!(
        code, 0,
        "a `preferences.toml` saved by Notepad stopped the program:\n{out}"
    );
    assert!(
        out.contains("Maximum removals in one command: 7"),
        "the setting behind the mark was not read:\n{out}"
    );
}

/// The control, and the reason none of the above is "LiNix ignores junk it does not understand":
/// a mark in the MIDDLE of a line is not an encoding artefact — nothing puts it there but a
/// paste — and it is still refused, by a message that names it.
#[test]
fn a_mark_that_is_not_at_the_start_is_still_refused() {
    let f = Fixture::new("bom-midline");
    f.write("priority", "cargo\n");
    f.write("modules/starter.txt", &format!("cargo:rip{BOM}grep\n"));

    let (out, code) = f.run(&["eval"]);
    assert_ne!(
        code, 0,
        "a zero-width character inside a package name was accepted:\n{out}"
    );
    assert!(
        out.contains("<U+FEFF>"),
        "the refusal did not name the character it rejected, so the user cannot see it:\n{out}"
    );
}
