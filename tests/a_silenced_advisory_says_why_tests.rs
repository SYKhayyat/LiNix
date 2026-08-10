//! **`deny.toml`'s ignore list is an exemption table, and it gets audited like one.**
//!
//! The supply-chain gate's own header says it: *"An advisory against a dependency fails the build.
//! That is the whole feature."* The list that switches that off for one advisory is therefore the
//! most dangerous list in the repository — and the failure mode is not that somebody adds a bad
//! entry, it is that entries accumulate, each one obvious to whoever added it and opaque to
//! everybody after. A gate with four silent exemptions is not a gate; it is a file that makes
//! people believe there is one.
//!
//! So an entry has to say what it is, why there was no fix, and what would end it. That is a
//! comment, which no schema can require — hence this.

use std::path::PathBuf;

fn deny_toml() -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("deny.toml"))
        .expect("deny.toml is the supply-chain gate and it must be there")
}

/// The ids inside `ignore = [ ... ]`, and the text of the block they sit in.
fn ignore_block(source: &str) -> (Vec<String>, String) {
    let from = source
        .find("ignore = [")
        .expect("the ignore list is gone; either it was renamed or the gate was");
    let rest = &source[from..];
    let to = rest.find(']').expect("an unterminated ignore list");
    let block = rest[..to].to_string();
    let ids = block
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('"'))
        .map(|l| l.trim_matches(|c: char| c == '"' || c == ',').to_string())
        .collect();
    (ids, block)
}

/// Every silenced advisory is named in prose above the line that silences it, at length.
///
/// Length is a crude proxy for a reason and it is the right one here: the entry that will do the
/// damage is the one added in a hurry with `# transitive, can't fix` above it.
#[test]
fn every_ignored_advisory_carries_its_reason() {
    let source = deny_toml();
    let (ids, block) = ignore_block(&source);
    for id in &ids {
        let cited: String = block
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('#'))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            cited.contains(id),
            "{id} is silenced without being named in a comment beside it"
        );
        assert!(
            cited.len() > 400,
            "the reasons in this block total {} characters for {} entry(ies). An advisory is \
             switched off here; say what it is, why no fix was available, and what would end \
             the entry.",
            cited.len(),
            ids.len()
        );
    }
}

/// **A silenced advisory must state its own end condition**, or it is permanent by accident.
/// Every one of these exists because of somebody else's repository, and those change.
#[test]
fn every_ignored_advisory_says_what_would_end_it() {
    let (ids, block) = ignore_block(&deny_toml());
    if ids.is_empty() {
        return; // the goal state, and it needs nothing
    }
    let lower = block.to_ascii_lowercase();
    assert!(
        lower.contains("what would end this entry") || lower.contains("what would end it"),
        "an entry here switches off a supply-chain gate; it has to say what would remove it \
         again, in those words, so a reader can check without re-deriving the whole story"
    );
}

/// **And the list stays short.** No number is principled; the point is that growth is a decision
/// somebody makes rather than a thing that happens. Raising this is a deliberate act with a
/// diff, which is the whole mechanism.
#[test]
fn the_ignore_list_does_not_grow_quietly() {
    let (ids, _) = ignore_block(&deny_toml());
    assert!(
        ids.len() <= 1,
        "{} advisories are silenced. Each one is a build made green by a decision rather than \
         a fix: {ids:?}",
        ids.len()
    );
}

/// The gate is still armed for everything else — a `deny.toml` whose advisories section had been
/// turned off wholesale would pass every test above by having nothing to audit.
#[test]
fn the_advisory_gate_is_still_the_default() {
    let source = deny_toml();
    for switched_off in [
        "unmaintained = \"allow\"",
        "vulnerability = \"allow\"",
        "unsound = \"allow\"",
        "yanked = \"allow\"",
    ] {
        assert!(
            !source.contains(switched_off),
            "`{switched_off}` turns off a whole class of advisory, which is not an exemption \
             anybody reviewed - it is the gate being disabled"
        );
    }
}
