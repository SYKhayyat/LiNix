//! GRADER round 6, 2026-07-31 — RED. A package name containing `\` is read as **set math**, and
//! the refusal a user sees talks about profile algebra.
//!
//! Measured on Windows, release-equivalent debug binary, one line in a module:
//!
//! ```text
//! $ printf 'winget:a\\b\n' > $LINIX_CONFIG_DIR/modules/starter.txt
//! $ linix eval
//! Error: Configuration error: …\modules\starter.txt:1: a module cannot use a set expression
//!   a module is a list of what it holds; set math is how a profile chooses between them.
//!   To say something must NOT exist, write `absent:apt:foo`.
//! ```
//!
//! Nothing on that line asks for set math. `looks_like_expression` fires on `\ | & (`, and
//! `parse` consults it for any line that does not open with a **statement prefix** — so
//! `link:C:\…` and `setting:HKCU\…` are safe and `winget:ARP\Machine\X64\Firefox` is not,
//! because a `backend:name` package line carries no prefix from that list.
//!
//! **This is the same bug the repo already fixed once, one line-type over.**
//! `statement.rs::a_prefix_whose_payload_looks_like_set_math_is_still_that_statement` was written
//! after a Windows registry key was handed to the set-expression parser; it pins `generate:` and
//! `link:` and nothing else. The sibling — the plain package line, which is the most common line
//! in the language — was never covered.
//!
//! It is not hypothetical on the platform this repo is developed on. `winget list` on this
//! machine reports 278 names, and **185 of them contain a backslash** (119 `ARP\Machine\…`,
//! 66 `MSIX\…`). Every one of those is a name a user could reasonably paste into a module after
//! reading `linix list`.
//!
//! The control below is the reason this measures what it says: a name with a *space* refuses
//! correctly and legibly (``` `Mozilla Firefox` is not a package name ```), so the grammar is
//! not simply rejecting everything unusual — it is specifically mis-classifying `\`.

use linix::config::grammar::statement::{parse, Statement};
use linix::config::grammar::Origin;

fn known(name: &str) -> bool {
    matches!(name, "apt" | "cargo" | "npm" | "winget" | "scoop" | "choco")
}

fn at(line: usize) -> Origin {
    Origin::new("modules/win.txt", line)
}

/// Package names as `winget list` prints them on a real Windows machine.
const REAL_WINGET_NAMES: &[&str] = &[
    r"winget:ARP\Machine\X64\Firefox",
    r"winget:ARP\Machine\X86\ILST_30_2_1",
    r"winget:ARP\Machine\X64\{8BD2A40D-67A6-45F5-877D-6D9D04C9D5A2}",
    r"winget:MSIX\Microsoft.AV1VideoExtension_2.0.24.0_x64__8wekyb3d8bbwe",
    // The minimal case, so the diagnosis is not confused by anything else on the line.
    r"winget:a\b",
];

/// A `backend:name` line is a package line whatever punctuation the name carries — the same rule
/// `link:` and `setting:` already get.
#[test]
fn a_package_name_with_a_backslash_is_a_package_and_not_an_expression() {
    let mut eaten = Vec::new();

    for line in REAL_WINGET_NAMES {
        match parse(&at(1), line, &known) {
            Ok(Statement::Package(_)) => {}
            Ok(Statement::Expr(e)) => eaten.push(format!(
                "`{line}` was read as a set expression: {e}\n     \
                 the user is told \"a module cannot use a set expression\""
            )),
            Ok(other) => eaten.push(format!("`{line}` parsed as {other:?}, not a package")),
            Err(e) => eaten.push(format!("`{line}` was refused: {e}")),
        }
    }

    assert!(
        eaten.is_empty(),
        "{} of {} real `winget list` names are not read as package lines:\n  {}\n\n\
         `looks_like_expression` fires on `\\`, and `parse` only shields lines that open with a \
         statement prefix. `link:`/`setting:`/`generate:` are shielded; `backend:name` is not. \
         185 of the 278 names winget reports on a real Windows box carry a backslash.",
        eaten.len(),
        REAL_WINGET_NAMES.len(),
        eaten.join("\n  ")
    );
}

/// The control. A name with a space is refused, and the refusal names the actual problem — so a
/// green result above would mean backslashes are handled, not that the grammar rejects anything
/// that looks odd.
#[test]
fn the_control_a_name_with_a_space_refuses_and_says_why() {
    let err = match parse(&at(1), "winget:Mozilla Firefox", &known) {
        Err(e) => e.to_string(),
        Ok(other) => panic!("`winget:Mozilla Firefox` should be refused, got {other:?}"),
    };
    assert!(
        err.contains("is not a package name"),
        "the space case must refuse by naming the name, or this file's diagnosis is wrong. \
         Got: {err}"
    );
    assert!(
        !err.contains("set expression"),
        "the space case must NOT be blamed on set math either. Got: {err}"
    );
}

/// The family, from the other end: the shield that protects `link:`/`setting:` is exactly what a
/// package line lacks. If a backslash ever becomes legal in a package name, these must all agree.
#[test]
fn the_prefixed_statements_this_was_fixed_for_are_still_fixed() {
    for line in [
        r"generate:C:\tools\list-packages.ps1",
        r"link:C:\Users\me\.vimrc @target=~/.vimrc",
    ] {
        match parse(&at(1), line, &known) {
            Ok(Statement::Expr(e)) => {
                panic!("regression: `{line}` was read as a set expression: {e}")
            }
            Ok(_) => {}
            Err(e) => panic!("`{line}` did not parse: {e}"),
        }
    }
}
