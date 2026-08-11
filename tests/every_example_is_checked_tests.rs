//! `examples/` shipped two things. `preferences.toml` had an `include_str!` test behind it and
//! did not rot. `examples/groups/` had nothing behind it and rotted completely: five files
//! describing a directory layout that no longer exists — `group:` is on `target-state.md`'s
//! deleted-syntax list — of which `bloatware.txt` told the reader to run
//! `shall sync --remove-bloatware`, a flag on the deleted-config list, and was itself named on
//! the deleted-*files* list. A straight NO-LEGACY violation, read by nothing.
//!
//! **That contrast is the whole argument, so it is a gate rather than a cleanup.** Deleting the
//! five rotten files fixes today; requiring every example to be checked by something is what
//! stops the sixth being written next week.

use std::path::{Path, PathBuf};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

fn every_example() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&examples_dir(), &mut out);
    out.sort();
    out
}

/// The directory names Shall's own layout uses, asked of `Layout` rather than listed here.
fn layout_directories() -> Vec<String> {
    let root = PathBuf::from("/root");
    let layout = shall::model::Layout::new(root.clone(), root.join("data"));
    [
        layout.modules_dir(),
        layout.profiles_dir(),
        layout.adapters_dir(),
        layout.locks_dir(),
        layout.snapshots_dir(),
    ]
    .iter()
    .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
    .collect()
}

/// **The assertion that would actually have caught the rot**, which the parse check below
/// would not: `bloatware.txt` was a hundred percent comments and parses as an empty manifest,
/// and `base.txt` was bare package names that are still perfectly legal lines. What was wrong
/// with `examples/groups/` was not any line in it — it was the **directory**, describing a
/// layout Shall stopped having. `groups` is a *file* in the layout (`Layout::groups_file`, the
/// U18 backend groups), never a folder of manifests, and that mismatch is the whole defect
/// stated in one comparison.
#[test]
fn every_example_directory_is_one_the_layout_has() {
    let allowed = layout_directories();
    assert!(
        allowed.len() >= 5,
        "the layout answered with {:?}, which is too few to be the real one",
        allowed
    );
    assert!(
        !allowed.contains(&"groups".to_string()),
        "`groups` is a file in the layout, not a directory — if that has changed, this test is \
         the thing to re-read, not to delete"
    );

    for entry in std::fs::read_dir(examples_dir())
        .expect("examples/ exists")
        .flatten()
    {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            allowed.contains(&name),
            "examples/{}/ is a directory Shall's layout does not have. That is what
             `examples/groups/` was: five files teaching a folder shape the program stopped \
             using, one of them advertising a `--remove-bloatware` flag on the deleted list.",
            name
        );
    }
}

#[test]
fn every_shipped_example_is_something_shall_can_actually_read() {
    let examples = every_example();

    // The instrument before the assertion: a walk that finds nothing passes silently, and the
    // whole point of this file is that an unchecked example is invisible.
    assert!(
        !examples.is_empty(),
        "the walk found no examples at all in {} — it is looking in the wrong place, and a \
         scan that matches nothing looks exactly like a tree with nothing wrong in it",
        examples_dir().display()
    );

    for path in &examples {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{} could not be read: {}", name, e));

        match path.extension().and_then(|e| e.to_str()) {
            // A settings file has to deserialize into the real `Config`, which is what caught
            // `[retention.generations]` surviving a whole phase after the format was deleted.
            Some("toml") => {
                toml::from_str::<shall::config::Config>(&text).unwrap_or_else(|e| {
                    panic!("examples/{} does not parse as a Config: {}", name, e)
                });
            }
            // A manifest has to parse with the real grammar, against a vocabulary that knows
            // every backend Shall ships — which is what would have refused `group:` lines and
            // the `# shall sync --remove-bloatware` advice's file for being a manifest that is
            // not one.
            Some("txt") => {
                let known = |_: &str| true;
                for (n, line) in text.lines().enumerate() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let origin = shall::config::grammar::Origin::new(PathBuf::from(&name), n + 1);
                    shall::config::grammar::statement::parse(&origin, line, &known).unwrap_or_else(
                        |e| {
                            panic!(
                                "examples/{}:{} is not a line Shall can read: {}\n  {}",
                                name,
                                n + 1,
                                e.what,
                                line
                            )
                        },
                    );
                }
            }
            _ => panic!(
                "examples/{} is shipped and nothing checks it. Either give it a check here — \
                 the one example that had one is the one that did not rot — or delete it.",
                name
            ),
        }
    }
}
