//! **`utils/file.rs` opened with "There were two of these… so there is now one." There were
//! four.**
//!
//! `atomic_write` fsyncs. `CommandExecutor::write_atomic` did not — no `flush`, no `sync_all` —
//! and it is what writes a systemd unit and a `link:` target. `CommandExecutor::write_secret`
//! flushed and did not fsync. `InstalledListings::save_to_disk` writes and renames with neither,
//! deliberately, because it is a cache.
//!
//! **A rename is atomic against a concurrent reader and says nothing about power loss.** The
//! directory entry can reach the disk before the bytes it points at do, which leaves a file of
//! the right name and zero length. So a crash after a sync could leave an empty systemd unit
//! while `registry.json` and the WAL — which went through `persist` — survived intact: LiNix's
//! record of what it did, without the thing it did.
//!
//! This scan is the half a comment cannot do. The durability is one function now
//! ([`linix::utils::file::durable_write`], private to the crate); what stays plural is the
//! *preview policy*, deliberately, and this enumerates both so a third cannot appear unnoticed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The files allowed to hold a rename-into-place of their own, and why.
///
/// Two entries, and they are the two front doors:
///
/// - `utils/file.rs` is `durable_write` itself, plus `persist`'s preview policy for the config
///   repo: *print `would write …` and stop*.
/// - `core/installed.rs` is the installed-listing cache, which is **correctly** neither durable
///   nor preview-aware: a torn cache file is a cache miss, an fsync per listing would be a disk
///   barrier on the read path, and its temp name carries the pid because two `linix` runs
///   sharing one temp path is the torn listing the mechanism exists to prevent.
const MAY_RENAME: &[(&str, &str)] = &[
    (
        "src/utils/file.rs",
        "`durable_write` is the one durable write, and `persist` is the config repo's preview \
         policy in front of it.",
    ),
    (
        "src/core/installed.rs",
        "The installed-listing cache. A torn file is a cache miss, so it is deliberately not \
         durable; its temp name carries the pid because the rename is only atomic per writer.",
    ),
];

/// A line that moves a temporary file over a real one, or creates the temporary file to do it
/// with. Both spellings, because a writer that does the second and not the first is a writer
/// that has not finished being written.
fn renames_into_place(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or(line);
    // `.persist(` with a dot is `tempfile`'s rename; `file::persist(` — the sanctioned front
    // door — has no dot before the name, which is the whole of the distinction.
    code.contains("NamedTempFile::new") || code.contains("fs::rename(") || code.contains(".persist(")
}

/// Every `src/` file with a rename-into-place in it, excluding test modules — a test that writes
/// a fixture through a temp file is not a product writer.
fn files_that_rename(root: &Path) -> (BTreeSet<String>, usize) {
    let mut found = BTreeSet::new();
    let mut scanned = 0;
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            scanned += 1;
            // Everything from the first `#[cfg(test)]` on belongs to the tests.
            let product = src.split("#[cfg(test)]").next().unwrap_or(&src);
            if product.lines().any(renames_into_place) {
                found.insert(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    (found, scanned)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn only_the_two_sanctioned_writers_rename_a_file_into_place() {
    let (found, scanned) = files_that_rename(&repo_root());

    // A scan that reads nothing is not a clean scan.
    assert!(
        scanned > 100,
        "only {scanned} source files were read; the walk is looking in the wrong place"
    );
    assert!(
        !found.is_empty(),
        "no rename-into-place was found at all, which means the predicate has stopped matching"
    );

    let allowed: BTreeSet<String> = MAY_RENAME.iter().map(|(f, _)| f.to_string()).collect();
    let extra: Vec<&String> = found.difference(&allowed).collect();
    assert!(
        extra.is_empty(),
        "these files write to the disk through a rename of their own: {extra:?}\n\
         Use `utils::file::persist` (the config repo, preview-aware) or \
         `CommandExecutor::write_atomic`/`write_secret` (the machine, VFS-aware). Both go \
         through `durable_write`, which flushes and fsyncs before the rename — the three steps \
         two of the four hand-rolled copies were missing."
    );

    // And the allowance does not outlive its subject: an entry naming a file that no longer
    // renames anything is a permission granted to nothing, reading as if it guards something.
    let stale: Vec<&String> = allowed.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "MAY_RENAME allows {stale:?}, which no longer renames a file into place — drop the entry"
    );

    for (_, reason) in MAY_RENAME {
        assert!(reason.len() > 60, "an allowance with no reason is a hole");
    }
}

/// **The oracle.** The predicate is driven over planted lines, so a scan that has stopped
/// matching cannot pass by finding nothing.
#[test]
fn the_scan_can_actually_fail() {
    assert!(renames_into_place("    let t = NamedTempFile::new_in(dir)?;"));
    assert!(renames_into_place("        std::fs::rename(&tmp, &path)?;"));
    assert!(renames_into_place("    temp.persist(path).map_err(e)?;"));

    // Not a writer: a comment about one, and a rename of something that is not a file write.
    assert!(!renames_into_place(
        "    // NamedTempFile::new_in is how the durable write works"
    ));
    assert!(!renames_into_place("    let renamed = old.rename(new);"));
    assert!(!renames_into_place("    fs::write(&path, body)?;"));
    assert!(!renames_into_place(""));

    // And the walk finds a planted file. Driven over a temp tree so the assertion is about the
    // walk rather than about the repo it happens to be pointed at.
    let tmp = tempfile::tempdir().expect("a temp dir");
    let src = tmp.path().join("src").join("deep");
    std::fs::create_dir_all(&src).expect("the tree");
    std::fs::write(
        src.join("offender.rs"),
        "fn save() {\n    let t = NamedTempFile::new_in(d)?;\n}\n",
    )
    .expect("the offender");
    std::fs::write(
        src.join("innocent.rs"),
        "fn save() {\n    crate::utils::file::persist(&p, &body)?;\n}\n",
    )
    .expect("the innocent");
    std::fs::write(
        src.join("only_in_tests.rs"),
        "fn save() {}\n#[cfg(test)]\nmod t {\n    fn f() { NamedTempFile::new_in(d); }\n}\n",
    )
    .expect("the test-only file");

    let (found, scanned) = files_that_rename(tmp.path());
    assert_eq!(scanned, 3);
    assert_eq!(
        found.into_iter().collect::<Vec<_>>(),
        vec!["src/deep/offender.rs".to_string()],
        "the walk missed the planted offender, or caught one of the two controls"
    );
}
