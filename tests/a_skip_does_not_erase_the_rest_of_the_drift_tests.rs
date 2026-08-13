//! One skipped declaration must not erase every other kind of drift from `check`.
//!
//! `check.rs:231` matches `!c.skipped.is_empty()` **before** the arm at `check.rs:288` that
//! prints the counts, so the two are alternatives rather than a sum. A machine with one skipped
//! declaration and a hundred pending changes reports the skip and nothing else:
//!
//! ```text
//! ->  drift  0 to install, 0 to remove, 1 to place, 0 to undo          (no skip)
//! ->  drift  1 package(s) installed and declared nowhere that `sync`   (one skip added;
//!            will not remove: apt:tree ...                              the resource is gone)
//! ```
//!
//! Nothing about the machine changed between those two lines except that one more declaration
//! was added, for a manager this host does not have. The declared file is still absent from
//! disk and `sync` would still place it.
//!
//! **The resource is lost from the JSON as well, and that is the half that cannot be argued.**
//! Package work survives in `counts` — `install` and `remove` are attached to both arms — but
//! resources have no key there. `r.summary()` inside the summary *string* is the only place a
//! pending `link:` is ever reported, and the skip arm writes a different string. The full drift
//! node this fixture produces is:
//!
//! ```json
//! {"counts":{"install":0,"remove":0,"skipped":1},"ok":false,"section":"drift",
//!  "summary":"1 package(s) installed and declared nowhere that `sync` will not remove: ..."}
//! ```
//!
//! There is no reading of that document under which a file is missing from disk.
//!
//! **`plan` knows.** The same fixture, same moment: *"0 install(s), 0 removal(s), 1 resource(s)
//! to place"*. The fact is computed and available; `check` discards it.
//!
//! **How a real machine gets here.** `sudo` ships `secure_path` set to the system directories
//! only, so `cargo`, `bun` and `uv` — installed under `~/.cargo/bin`, `~/.bun/bin` and
//! `~/.local/bin` — are not on the PATH of anything run through it. Measured on stock Ubuntu:
//! the identical repository reports `4 to install` normally and, under `sudo`, three skips and
//! no counts at all.
//!
//! Distinct from the claim in `a_skipped_install_is_not_an_undeclared_leftover_tests.rs`: that
//! one is about the sentence being false, this one is about the true sentences it replaces. The
//! detail view `shall check drift` is not a counter-example — it lists no resources in either
//! state, which is its own smaller gap and not caused by the skip.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Managers to try as "in `priority`, not on this host" — the same spread and the same
/// verify-the-choice rule as the sibling fixture.
const CANDIDATES: &[&str] = &[
    "apt", "pacman", "apk", "xbps", "zypper", "dnf", "emerge", "choco", "winget", "brew", "nix",
];

fn shall() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_shall"))
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(shall())
        .args(args)
        .env("SHALL_CONFIG_DIR", dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("the binary should run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A repository whose only declaration is one `link:` whose target is not on disk.
///
/// A resource, not a package, so the fixture asserts the same thing on every host: no manager
/// has to be present for a file to be missing.
fn one_file_to_place(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("shall-skip-erases-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let init = Command::new(shall())
        .arg("init")
        .env("SHALL_CONFIG_DIR", &dir)
        .env("SHALL_DATA_DIR", dir.join("data"))
        .output()
        .expect("init should run");
    assert!(init.status.success(), "init failed in {}", dir.display());

    let source = dir.join("src").join("dotrc");
    std::fs::write(&source, "managed\n").unwrap();
    let target = dir.join("out").join("dotrc");
    std::fs::write(
        dir.join("modules").join("starter.txt"),
        format!(
            "link:{} @target={}\n",
            source.display().to_string().replace('\\', "/"),
            target.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    assert!(!target.exists(), "the target must be absent to be drift");
    dir
}

/// Add a declaration this host cannot serve, and return the manager it settled on.
///
/// `init` writes `priority` from what it found, so an absent manager is also unlisted, and an
/// unlisted manager is refused at the grammar before the planner ever sees it. Appending it is
/// what a machine that *lost* a manager looks like — which is what `secure_path` does to
/// `cargo` on every `sudo` invocation.
fn add_a_declaration_this_host_cannot_serve(dir: &Path) -> Option<String> {
    let priority_path = dir.join("priority");
    let modules = dir.join("modules").join("starter.txt");
    let original_priority = std::fs::read_to_string(&priority_path).unwrap_or_default();
    let original_modules = std::fs::read_to_string(&modules).unwrap_or_default();

    for candidate in CANDIDATES {
        if original_priority.lines().any(|l| l.trim() == *candidate) {
            continue; // present here; its skip would never be planned
        }
        std::fs::write(
            &priority_path,
            format!("{original_priority}\n{candidate}\n"),
        )
        .unwrap();
        std::fs::write(&modules, format!("{original_modules}{candidate}:tree\n")).unwrap();

        // The warning is the proof the manager is genuinely absent, rather than merely unlisted
        // — in which case a real install would be planned and there would be no skip to measure.
        if run(dir, &["plan"]).contains("is not on this machine") {
            return Some((*candidate).to_string());
        }
        std::fs::write(&priority_path, &original_priority).unwrap();
        std::fs::write(&modules, &original_modules).unwrap();
    }
    None
}

/// The drift row from `shall check`, without the marker column.
fn drift_row(dir: &Path) -> String {
    run(dir, &["check"])
        .lines()
        .find(|l| l.contains("drift"))
        .unwrap_or_default()
        .to_string()
}

/// The control, and the whole finding rests on it: with nothing skipped, `check` reports the
/// pending resource, so the omission below is a loss and not a thing `check` never did.
#[test]
fn check_reports_a_file_it_would_place() {
    let dir = one_file_to_place("control");
    let row = drift_row(&dir);
    assert!(
        row.contains("1 to place"),
        "the control never held: `check` does not report a declared, absent file as drift, so \
         the tests below would be measuring nothing.\nrow: {row}"
    );
}

/// A second control: `plan` reports the same resource in the presence of the skip.
///
/// Without this, "the resource vanished" could mean the planner stopped finding it once a
/// second declaration was added, which would be a different bug in a different place.
#[test]
fn plan_still_reports_the_resource_when_something_is_skipped() {
    let dir = one_file_to_place("plan");
    let Some(_manager) = add_a_declaration_this_host_cannot_serve(&dir) else {
        return; // every candidate manager is installed here; nothing to measure
    };
    let out = run(&dir, &["plan"]);
    assert!(
        out.contains("1 resource(s) to place"),
        "`plan` stopped reporting the resource too, so the loss is in the planner rather than \
         in `check`'s reporting of it:\n{out}"
    );
}

/// `check` still reports the file it would place when another declaration was skipped.
#[test]
fn a_skip_does_not_hide_a_file_check_would_place() {
    let dir = one_file_to_place("row");
    let Some(manager) = add_a_declaration_this_host_cannot_serve(&dir) else {
        return;
    };
    let row = drift_row(&dir);
    assert!(
        row.contains("1 to place"),
        "declaring one package for the absent `{manager}` erased the pending resource from \
         `check`'s drift row. The file is still missing from disk and `plan` still says \
         `1 resource(s) to place`.\nrow: {row}"
    );
}

/// And the JSON accounts for it, which is where a monitor would look.
///
/// Package work survives the skip arm in `counts`; a resource is only ever carried in the
/// summary prose, and the skip arm replaces the prose.
#[test]
fn a_skip_does_not_hide_the_resource_from_the_json() {
    let dir = one_file_to_place("json");
    let Some(manager) = add_a_declaration_this_host_cannot_serve(&dir) else {
        return;
    };
    let out = run(&dir, &["check", "--json"]);
    let node = out
        .split("\"section\"")
        .find(|chunk| chunk.starts_with(": \"drift\"") || chunk.starts_with(":\"drift\""))
        .map(|chunk| chunk.split('}').next().unwrap_or_default().to_string())
        .unwrap_or_else(|| panic!("no drift section in `check --json`:\n{out}"));

    assert!(
        node.contains("place") || node.contains("resource"),
        "the drift node of `check --json` accounts for no pending resource at all once a \
         declaration for the absent `{manager}` is present: neither `counts` (which has keys \
         for `install`, `remove` and `skipped`, and none for resources) nor the summary \
         mentions the file that is missing from disk.\nnode: {{\"section\"{node}}}"
    );
}
