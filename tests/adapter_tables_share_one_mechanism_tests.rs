//! K17 is one mechanism, and this is what keeps it one.
//!
//! "Adapters are a table, and the built-ins are rows in it" was ruled once and implemented
//! **seven** times — firewalls, init systems, settings stores, snapshot providers, bootstrap
//! commands, prereq steps and secret providers. Seven row *types* is right: a firewall's argv is
//! `allow`/`deny` and an init's is `start`/`stop`, and folding those into one schema would be a
//! struct of twenty optional fields. Seven copies of the machinery *around* them was not, and by
//! the time it was counted four of the five shared questions had already been answered
//! differently by different tables — including one table with no `os` field at all.
//!
//! Two properties, and the second is the one that matters:
//!
//! 1. **Every adapter table goes through `core::adapter`.** Enumerated from the source, so a
//!    table added tomorrow fails this until somebody says which mechanism it uses.
//! 2. **Nobody writes the machinery again.** A file that re-implements the OS filter or the
//!    usability floor is a build failure, not a review comment. This is the half that a ledger
//!    alone cannot give: the seven copies were each written by someone who had not read the
//!    other six.

use linix::core::adapter::{self, AdapterRow, Detected};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// A `[[table]]` of rows read out of a TOML file, and what it does about it.
struct Table {
    /// The `*File` wrapper struct, which is how such a table is spelled in this codebase.
    wrapper: &'static str,
    /// Where it lives, for the message.
    file: &'static str,
    /// What it is, if it is not an adapter table. `None` means it is one.
    not_an_adapter: Option<&'static str>,
}

/// Every `[[table]]` in the tree, and whether it is an adapter table.
///
/// The reason is not decoration: a wrapper struct excluded without one is a table nobody
/// checked, which is how the count reached seven in the first place.
const TABLES: &[Table] = &[
    Table {
        wrapper: "FirewallAdapterFile",
        file: "src/backends/firewall.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "InitProviderFile",
        file: "src/backends/service.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "SettingStoreFile",
        file: "src/backends/setting.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "SnapshotProviderFile",
        file: "src/core/snapshot.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "BootstrapFile",
        file: "src/model/bootstrap.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "PrereqFile",
        file: "src/model/prereq.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "SecretProviderFile",
        file: "src/model/secret.rs",
        not_an_adapter: None,
    },
    Table {
        wrapper: "CustomBackendsFile",
        file: "src/backends/onboarder.rs",
        // Checked and deliberately excluded. `[[backend]]` is the *backend* table (F-5's data
        // path), not an adapter: it has no `detect` and no `os`, its floor is a valid command
        // name rather than a set of argv, and Q6 lets a row take a name a built-in already
        // holds — the exact opposite of the rule `adapter::merge` enforces. Putting it through
        // the same merge would silently delete the `override` feature.
        not_an_adapter: Some("the [[backend]] table — Q6 lets a row override a built-in name"),
    },
];

/// Every table in the tree is in the ledger above, and every ledger entry still describes a
/// table that exists.
#[test]
fn every_table_of_rows_is_accounted_for() {
    let found = wrapper_structs_in_src();
    let mut problems = Vec::new();

    for wrapper in &found {
        if !TABLES.iter().any(|t| t.wrapper == wrapper.name) {
            problems.push(format!(
                "UNACCOUNTED: `{}` in {} is a table of rows and is in no ledger entry.\n    \
                 Add it to TABLES in this file — as an adapter table, or with the reason it is \
                 not one. K17 was implemented seven separate times because nothing counted the \
                 implementations.",
                wrapper.name, wrapper.file
            ));
        }
    }

    for entry in TABLES {
        let Some(actual) = found.iter().find(|w| w.name == entry.wrapper) else {
            problems.push(format!(
                "STALE: TABLES names `{}` but no such table exists any more. Delete the entry.",
                entry.wrapper
            ));
            continue;
        };
        if actual.file != entry.file {
            problems.push(format!(
                "MOVED: TABLES puts `{}` in {} and it is now in {}. Update the entry — the \
                 path is what the next check reads.",
                entry.wrapper, entry.file, actual.file
            ));
            continue;
        }
        // The claim each entry makes, checked rather than trusted: an adapter table's file
        // implements the trait, and a file excluded from the mechanism does not quietly
        // implement it anyway.
        let source = std::fs::read_to_string(root().join(entry.file))
            .unwrap_or_else(|e| panic!("{} must be readable: {}", entry.file, e));
        let implements = source.contains("impl AdapterRow for");
        match (entry.not_an_adapter, implements) {
            (None, false) => problems.push(format!(
                "NOT WIRED UP: `{}` is listed as an adapter table but {} has no \
                 `impl AdapterRow`. It is answering the shared questions its own way, which is \
                 how K17 came to have seven implementations.",
                entry.wrapper, entry.file
            )),
            (Some(why), true) => problems.push(format!(
                "EXCLUDED BUT WIRED UP: `{}` is excluded because {} — and yet {} implements \
                 `AdapterRow`. One of the two is out of date.",
                entry.wrapper, why, entry.file
            )),
            _ => {}
        }
    }

    assert!(
        problems.is_empty(),
        "the set of adapter tables has moved since it was last counted:\n\n{}",
        problems.join("\n\n")
    );
}

/// The half a ledger cannot give: nobody may write the machinery a second time.
///
/// Each of these was, at the point this was written, present in four or more files. They are
/// now in `core/adapter.rs` and nowhere else — and a file that grows its own copy fails the
/// build rather than passing review, which is the only thing that would have stopped the
/// seventh copy.
#[test]
fn the_shared_machinery_is_written_exactly_once() {
    const WRITTEN_ONCE: &[(&str, &str)] = &[
        (
            "eq_ignore_ascii_case(std::env::consts::OS)",
            "the OS filter — `AdapterRow::applies_to` takes the OS as a parameter so the \
             Windows arm of a table is testable on Linux, which four hand-written copies \
             reading `consts::OS` directly were not",
        ),
        (
            "fn applies_to_this_os",
            "the OS filter under its other name — the question had two spellings, one taking \
             the OS and one reading it",
        ),
        (
            "fn is_usable(",
            "the usability floor — `AdapterRow::why_unusable` says what this table adds to it, \
             and the empty-key check belongs to the trait",
        ),
    ];

    let mut problems = Vec::new();
    for (pattern, why) in WRITTEN_ONCE {
        let sites = sites_of(pattern);
        // `core/adapter.rs` is where each of these is allowed to live.
        let elsewhere: Vec<&(String, usize)> = sites
            .iter()
            .filter(|(file, _)| file != "src/core/adapter.rs")
            .collect();
        if !elsewhere.is_empty() {
            problems.push(format!(
                "`{}` appears outside src/core/adapter.rs, at {}.\n    That is {}.",
                pattern,
                elsewhere
                    .iter()
                    .map(|(f, l)| format!("{}:{}", f, l))
                    .collect::<Vec<_>>()
                    .join(", "),
                why
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "an adapter table has grown its own copy of shared machinery:\n\n{}\n\n\
         `core/adapter.rs` answers everything asked *about* rows. A row says what it is.",
        problems.join("\n\n")
    );
}

/// Every adapter row type answers the trait, proved by calling it.
///
/// A compile-time assertion at heart: this file cannot build unless each type implements
/// `AdapterRow`, and the ledger above is what makes the *set* of types complete.
#[test]
fn every_adapter_row_answers_the_same_three_questions() {
    fn asks<R: AdapterRow>(row: &R, expect_name: &str) {
        assert_eq!(row.name(), expect_name);
        // Answering at all is the point; what it answers is each table's own test.
        let _ = row.only_on();
        let _ = row.unusable();
        let _ = row.applies_to("linux");
        assert!(!R::WHAT.trim().is_empty(), "a table must name itself");
    }

    let firewall: linix::backends::firewall::FirewallAdapter =
        toml::from_str(FIREWALL_ROW).expect("the sample firewall row parses");
    asks(&firewall, "ufw");
    assert_eq!(firewall.detect_command(), "ufw");

    let init: linix::backends::service::InitProvider =
        toml::from_str(INIT_ROW).expect("the sample init row parses");
    asks(&init, "systemd");
    assert_eq!(init.detect_command(), "systemctl");

    let setting: linix::backends::setting::SettingAdapter =
        toml::from_str(SETTING_ROW).expect("the sample settings row parses");
    asks(&setting, "gsettings");
    assert_eq!(setting.detect_command(), "gsettings");

    let snapshot: linix::core::snapshot::SnapshotProviderDef =
        toml::from_str(SNAPSHOT_ROW).expect("the sample snapshot row parses");
    asks(&snapshot, "lvm");
    assert_eq!(snapshot.detect_command(), "lvcreate");

    let bootstrap: linix::model::bootstrap::BootstrapDef =
        toml::from_str(BOOTSTRAP_ROW).expect("the sample bootstrap row parses");
    asks(&bootstrap, "brew");

    let prereq: linix::model::prereq::PrereqDef =
        toml::from_str(PREREQ_ROW).expect("the sample prereq row parses");
    asks(&prereq, "mix");

    let secret: linix::model::secret::SecretProvider =
        toml::from_str(SECRET_ROW).expect("the sample secret row parses");
    asks(&secret, "vault");
}

/// The `os` field, on every table — asked of each type rather than of the trait, because the
/// gap this closes was one type not having the field at all.
#[test]
fn every_adapter_row_can_be_restricted_to_one_os() {
    fn restricted<R: AdapterRow + serde::de::DeserializeOwned>(base: &str, table: &str) {
        let with_os = format!("{}\nos = \"definitely-not-this-os\"\n", base.trim_end());
        let row: R = toml::from_str(&with_os).unwrap_or_else(|e| {
            panic!(
                "a `{}` row must accept an `os` field — every adapter table can be confined to \
                 the platform its commands were written for, and `[[secret]]` was the one that \
                 could not: {}",
                table, e
            )
        });
        assert_eq!(row.only_on(), Some("definitely-not-this-os"));
        assert!(!row.applies_to("linux"));
        assert!(!row.applies_to("windows"));

        // And the control: without the field, the row applies everywhere.
        let bare: R = toml::from_str(base).expect("the base row parses");
        assert_eq!(bare.only_on(), None);
        assert!(bare.applies_to("linux") && bare.applies_to("windows"));
    }

    restricted::<linix::backends::firewall::FirewallAdapter>(FIREWALL_ROW, "firewall");
    restricted::<linix::backends::service::InitProvider>(INIT_ROW, "init");
    restricted::<linix::backends::setting::SettingAdapter>(SETTING_ROW, "setting_store");
    restricted::<linix::core::snapshot::SnapshotProviderDef>(SNAPSHOT_ROW, "snapshot");
    restricted::<linix::model::bootstrap::BootstrapDef>(BOOTSTRAP_ROW, "bootstrap");
    restricted::<linix::model::prereq::PrereqDef>(PREREQ_ROW, "prereq");
    restricted::<linix::model::secret::SecretProvider>(SECRET_ROW, "secret");
}

/// The shipped rows still clear the floor, through the same call a user's row does — which is
/// the whole of K17/U1. Asked of the real built-in tables, not of samples.
#[test]
fn every_shipped_row_clears_the_floor() {
    fn all_usable<R: AdapterRow>(rows: Vec<R>, table: &str) {
        assert!(!rows.is_empty(), "{} ships no rows at all", table);
        for row in &rows {
            assert_eq!(
                row.unusable(),
                None,
                "the shipped `{}` {} does not clear the floor",
                row.name(),
                R::WHAT
            );
        }
    }

    all_usable(linix::backends::firewall::adapters(Vec::new()), "firewall");
    all_usable(linix::backends::service::providers(Vec::new()), "init");
    all_usable(linix::backends::setting::adapters(Vec::new()), "setting");

    let prereqs: linix::model::prereq::PrereqFile =
        toml::from_str(linix::app::apply::prereq::BUILTIN).expect("the built-in prereq rows parse");
    all_usable(prereqs.prereq, "prereq");

    // `usable` drops rather than refuses, so a shipped row that stopped clearing the floor
    // would vanish silently. That it drops nothing is the assertion.
    let shipped = linix::backends::firewall::adapters(Vec::new()).len();
    assert!(
        shipped >= 3,
        "ufw, firewalld and windows-defender ship — found {}",
        shipped
    );
}

/// The oracle. Before trusting the two scans, feed each something it must catch.
#[test]
fn the_scans_can_actually_see_what_they_look_for() {
    // The wrapper-struct reader, on the shapes the real files use.
    assert_eq!(
        wrapper_structs_in("pub struct FooFile {\n    foo: Vec<Bar>,\n}")
            .into_iter()
            .map(|w| w.name)
            .collect::<Vec<_>>(),
        vec!["FooFile"],
        "a public wrapper is a table"
    );
    assert_eq!(
        wrapper_structs_in("struct FooFile {\n    foo: Vec<Bar>,\n}")
            .into_iter()
            .map(|w| w.name)
            .collect::<Vec<_>>(),
        vec!["FooFile"],
        "a private wrapper is a table too — `CustomBackendsFile` is one"
    );
    assert!(
        wrapper_structs_in("pub struct LockFile {\n    path: PathBuf,\n}").is_empty(),
        "a struct with no `Vec<…>` field is not a table of rows"
    );

    // And on the real tree it must find the tables rather than an empty list.
    let real = wrapper_structs_in_src();
    assert!(
        real.len() >= TABLES.len(),
        "the wrapper reader found {} tables in the real tree — it has stopped matching",
        real.len()
    );

    // The duplication scan must find a pattern that is really there, and nothing that is not.
    assert!(
        !sites_of("fn applies_to").is_empty(),
        "the duplication scan cannot find `fn applies_to`, which is in core/adapter.rs — it \
         has stopped matching, and would report every future copy as absent"
    );
    assert!(
        sites_of("fn this_identifier_does_not_exist_anywhere").is_empty(),
        "the duplication scan reports hits for a pattern that is nowhere"
    );
}

// --- the sample rows, one per table --------------------------------------------------------
//
// Written out rather than lifted from the shipped tables, so a change to a built-in row cannot
// quietly change what these assert.

const FIREWALL_ROW: &str = r#"
name = "ufw"
detect = "ufw"
allow = ["ufw", "allow", "{port}/{proto}"]
deny = ["ufw", "delete", "allow", "{port}/{proto}"]
list = ["ufw", "status"]
list_pattern = '(\d+)/(tcp|udp)'
"#;

const INIT_ROW: &str = r#"
name = "systemd"
detect = "systemctl"
start = [["systemctl", "start", "--", "{name}"]]
stop = [["systemctl", "stop", "--", "{name}"]]
"#;

const SETTING_ROW: &str = r#"
name = "gsettings"
detect = "gsettings"
read = ["gsettings", "get", "{schema}", "{key}"]
write = ["gsettings", "set", "{schema}", "{key}", "{value}"]
reset = ["gsettings", "reset", "{schema}", "{key}"]
"#;

const SNAPSHOT_ROW: &str = r#"
name = "lvm"
detect = "lvcreate"
source = "vg0/root"
create = ["lvcreate", "-s", "-n", "{id}", "{source}"]
list = ["lvs", "--noheadings", "-o", "lv_name"]
delete = ["lvremove", "-f", "{id}"]
list_pattern = '(linix_\S+)'
"#;

const BOOTSTRAP_ROW: &str = r#"
manager = "brew"
run = ["/bin/sh", "-c", "install brew"]
"#;

const PREREQ_ROW: &str = r#"
manager = "mix"
missing = "Hex"
probe = ["mix", "hex.info"]
run = ["mix", "local.hex", "--force"]
"#;

const SECRET_ROW: &str = r#"
name = "vault"
decrypt = ["vault", "kv", "get", "{ref}"]
stdout_only = true
"#;

// --- the scans -----------------------------------------------------------------------------

struct Wrapper {
    name: String,
    file: String,
}

/// A `struct XFile { … Vec<Row> … }` — how a table of TOML rows is spelled in this codebase.
fn wrapper_structs_in(source: &str) -> Vec<Wrapper> {
    let mut out = Vec::new();
    let mut lines = source.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("pub struct ")
            .or_else(|| trimmed.strip_prefix("struct "))
        else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.ends_with("File") {
            continue;
        }
        // A table holds rows. Read forward to the closing brace looking for one.
        let mut holds_rows = false;
        for body in lines.by_ref() {
            if body.trim() == "}" {
                break;
            }
            if body.contains("Vec<") {
                holds_rows = true;
            }
        }
        if holds_rows {
            out.push(Wrapper {
                name,
                file: String::new(),
            });
        }
    }
    out
}

fn wrapper_structs_in_src() -> Vec<Wrapper> {
    let mut out = Vec::new();
    for path in rust_files(&root().join("src")) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = relative(&path);
        for mut w in wrapper_structs_in(&text) {
            w.file = rel.clone();
            out.push(w);
        }
    }
    out
}

/// Where `pattern` appears in `src/`, as `(file, line)`. Comments are skipped: this file's own
/// ledger quotes the patterns, and a module doc explaining why a rule exists must not read as a
/// breach of it.
fn sites_of(pattern: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    for path in rust_files(&root().join("src")) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = relative(&path);
        for (i, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if line.contains(pattern) {
                out.push((rel.clone(), i + 1));
            }
        }
    }
    out
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// `adapter::fill` is the one substitution, and every table's own helper now calls it. Checked
/// through a real table's public command builder rather than through `fill` directly, so this
/// fails if a table goes back to hand-rolling the chain.
#[test]
fn a_tables_argv_is_filled_by_the_shared_substitution() {
    let firewall: linix::backends::firewall::FirewallAdapter =
        toml::from_str(FIREWALL_ROW).expect("the sample firewall row parses");
    assert_eq!(
        firewall.allow_command(22, linix::model::firewall::Proto::Tcp),
        vec!["ufw", "allow", "22/tcp"]
    );

    // Left to right, which is what all five hand-written copies did — stated so a future
    // "simultaneous substitution" rewrite has to argue with a test rather than a comment.
    assert_eq!(
        adapter::fill(
            &["{a}".to_string()],
            &[("{a}", "{b}"), ("{b}", "substituted again")]
        ),
        vec!["substituted again"],
    );
}
