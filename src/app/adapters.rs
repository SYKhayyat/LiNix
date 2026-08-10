//! The eight ways to extend LiNix, in one table, with a way to ask what this machine has.
//!
//! Every one of them already worked: a `[[backend]]` row teaches a package manager, a
//! `[[snapshot]]` row teaches a rollback provider, and both go through II.12's ledger like any
//! other file LiNix executes on your behalf. What was missing is a **front door**. The eight
//! surfaces were eight paths on `Layout`, eight readers, and eight `warn!("ignoring
//! adapters/x.toml: …")` lines, and nothing in the program could answer *what have I extended?*
//!
//! **The failure that costs the most is not a bad row, it is a row nobody knows was dropped.**
//! `[[backends]]` instead of `[[backend]]` parses as perfectly good TOML describing a table
//! LiNix does not read — no parse error, no warning, and a `mymgr:` line that fails later with a
//! message about an unknown backend. So the survey below counts *rows the reader will actually
//! see*, per surface, and a file that parses into none of them says so.

use crate::model::layout::Layout;
use std::path::PathBuf;

/// One extension surface: a file a user writes, and what a row in it teaches LiNix.
pub struct Surface {
    /// The file's stem, which is also how a user names the surface: `linix adapters backends`.
    pub name: &'static str,
    /// The TOML array a reader looks for. `[[backend]]`, singular, in `backends.toml`, plural —
    /// the mismatch is deliberate in the format and is exactly the typo worth catching.
    pub key: &'static str,
    /// What one row teaches, in the words the readme uses.
    pub teaches: &'static str,
}

impl Surface {
    /// `adapters/<name>.toml` under this repo.
    pub fn path_in(&self, layout: &Layout) -> PathBuf {
        layout.adapter_file(self.name)
    }

    /// How a row opens, for a message that has to tell someone what to write.
    pub fn row(&self) -> String {
        format!("[[{}]]", self.key)
    }
}

/// **Every surface, and there are no others.** A ninth reader that opens a file under
/// `adapters/` without a row here is invisible to `linix adapters`, which is the whole defect
/// this table exists to close — `every_adapter_surface_is_in_the_table` fails on it.
pub const SURFACES: [Surface; 8] = [
    Surface {
        name: "backends",
        key: "backend",
        teaches: "how to drive a package manager LiNix does not ship",
    },
    Surface {
        name: "settings",
        key: "setting_store",
        teaches: "how to read and write a settings store",
    },
    Surface {
        name: "init",
        key: "init",
        teaches: "how to drive an init system",
    },
    Surface {
        name: "firewall",
        key: "firewall",
        teaches: "how to drive a firewall",
    },
    Surface {
        name: "snapshot",
        key: "snapshot",
        teaches: "how to take and restore a filesystem snapshot",
    },
    Surface {
        name: "secret",
        key: "secret",
        teaches: "how to decrypt a secret",
    },
    Surface {
        name: "prereq",
        key: "prereq",
        teaches: "the setup a manager needs before it can install anything",
    },
    Surface {
        name: "bootstrap",
        key: "bootstrap",
        teaches: "how to obtain a manager this machine does not have",
    },
];

/// The surface with this name, if it is one.
pub fn surface(name: &str) -> Option<&'static Surface> {
    SURFACES.iter().find(|s| s.name == name)
}

/// What this machine's copy of one surface is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// No such file. Not a problem — seven of the eight are absent on most machines.
    Absent,
    /// Approved by the ledger and read, with the number of rows the reader will see.
    InUse { rows: usize },
    /// The file is there, is approved, is valid TOML — and holds no rows of this surface's
    /// kind. A `[[backends]]` for a `[[backend]]` reader lands here, which is the point.
    NoRows,
    /// II.12 has not approved these bytes. The refusal names the id to approve.
    Unapproved(String),
    /// It is not TOML, or not TOML of the shape this surface reads.
    Malformed(String),
    /// It exists and could not be read at all.
    Unreadable(String),
}

impl Standing {
    /// Whether anything in this file is taking effect.
    pub fn effective(&self) -> bool {
        matches!(self, Standing::InUse { rows } if *rows > 0)
    }

    /// Whether a user asked for something and is not getting it. `Absent` is not a problem;
    /// everything else that is not in use is.
    pub fn is_wrong(&self) -> bool {
        !matches!(self, Standing::Absent | Standing::InUse { .. })
    }

    /// One word for a table.
    pub fn word(&self) -> &'static str {
        match self {
            Standing::Absent => "absent",
            Standing::InUse { .. } => "in use",
            Standing::NoRows => "no rows",
            Standing::Unapproved(_) => "unapproved",
            Standing::Malformed(_) => "malformed",
            Standing::Unreadable(_) => "unreadable",
        }
    }

    /// The sentence under the word, when there is one.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Standing::Unapproved(s) | Standing::Malformed(s) | Standing::Unreadable(s) => Some(s),
            _ => None,
        }
    }
}

/// One surface as this machine has it.
pub struct Extension {
    pub surface: &'static Surface,
    pub path: PathBuf,
    pub standing: Standing,
}

/// Read all eight, in the order they are declared.
///
/// This asks the same two questions the readers ask, in the same order — does II.12 approve
/// these bytes, and does the file hold rows of the right kind — so a surface reported `in use`
/// here is one whose rows a `sync` will act on.
pub fn survey(layout: &Layout) -> Vec<Extension> {
    SURFACES
        .iter()
        .map(|surface| {
            let path = surface.path_in(layout);
            Extension {
                standing: standing_of(surface, &path, &layout.locks_dir()),
                surface,
                path,
            }
        })
        .collect()
}

fn standing_of(surface: &Surface, path: &std::path::Path, locks_dir: &std::path::Path) -> Standing {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Standing::Absent,
        Err(e) => return Standing::Unreadable(e.to_string()),
    };
    if let Some(refusal) = crate::core::hook_lock::adapter_refusal(path, &content, locks_dir) {
        return Standing::Unapproved(refusal);
    }
    let doc: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => return Standing::Malformed(e.to_string()),
    };
    match doc.get(surface.key) {
        Some(toml::Value::Array(rows)) if !rows.is_empty() => Standing::InUse { rows: rows.len() },
        Some(other) => Standing::Malformed(format!(
            "`{}` is {}, not the array of tables `{}` writes",
            surface.key,
            other.type_str(),
            surface.row()
        )),
        None => Standing::NoRows,
    }
}

/// What to say when a surface's file cannot be used — in one voice, from one place.
///
/// The eight readers each wrote their own `warn!("ignoring adapters/x.toml: {e}")`, and a serde
/// message on its own (*"missing field `name` at line 4 column 1"*) tells a user which line and
/// nothing else: not which of eight files, not what a row of that kind looks like, not that the
/// rest of the file is inert, and not that there is a command which would have said all three.
pub fn cannot_use(surface: &Surface, why: impl std::fmt::Display) -> String {
    format!(
        "adapters/{}.toml is not in use: {why}. Nothing in it takes effect — a row there \
         teaches LiNix {}, and one opens with `{}`. `linix adapters` lists every extension \
         surface and what this machine has on each.",
        surface.name,
        surface.teaches,
        surface.row()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A repo whose adapter files are written AND approved, because the ledger is the first
    /// question `standing_of` asks and an unapproved file never reaches the parse.
    fn layout_with(files: &[(&str, &str)]) -> (TempDir, Layout) {
        use crate::core::hook_lock::{adapter_id, hash_script, HookLedger};
        use crate::core::LockFile;

        let dir = TempDir::new().unwrap();
        let adapters = dir.path().join("adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        let locks = dir.path().join("locks");
        std::fs::create_dir_all(&locks).unwrap();

        let mut ledger = HookLedger::default();
        for (name, body) in files {
            let file = format!("{name}.toml");
            std::fs::write(adapters.join(&file), body).unwrap();
            ledger.approve(&adapter_id(&file), &hash_script(body));
        }
        ledger.save(&HookLedger::path_in(&locks)).unwrap();

        let layout = Layout::new(dir.path().to_path_buf(), dir.path().join("data"));
        (dir, layout)
    }

    #[test]
    fn a_file_the_ledger_has_never_seen_is_unapproved_and_is_not_parsed() {
        // The order is the point: II.12 asks first. A malformed file that is also unapproved
        // is reported as unapproved, because that is the thing standing between it and use.
        let dir = TempDir::new().unwrap();
        let adapters = dir.path().join("adapters");
        std::fs::create_dir_all(&adapters).unwrap();
        std::fs::create_dir_all(dir.path().join("locks")).unwrap();
        std::fs::write(adapters.join("backends.toml"), "not toml at all
").unwrap();
        let layout = Layout::new(dir.path().to_path_buf(), dir.path().join("data"));
        let s = survey(&layout)
            .into_iter()
            .find(|e| e.surface.name == "backends")
            .unwrap()
            .standing;
        assert!(matches!(s, Standing::Unapproved(_)), "{s:?}");
        assert!(s.detail().is_some_and(|d| d.contains("linix lock")), "{s:?}");
    }

    fn standing(files: &[(&str, &str)], name: &str) -> Standing {
        let (_d, layout) = layout_with(files);
        survey(&layout)
            .into_iter()
            .find(|e| e.surface.name == name)
            .expect("every surface is surveyed")
            .standing
    }

    #[test]
    fn a_machine_that_has_extended_nothing_says_so_for_all_eight() {
        let (_d, layout) = layout_with(&[]);
        let all = survey(&layout);
        assert_eq!(all.len(), SURFACES.len());
        assert!(all.iter().all(|e| e.standing == Standing::Absent));
        assert!(all.iter().all(|e| !e.standing.is_wrong()));
    }

    #[test]
    fn a_plural_row_header_is_reported_rather_than_silently_dropped() {
        // The whole reason this module exists. `[[backends]]` is valid TOML describing a table
        // no reader opens: no parse error, no warning, and the `mymgr:` line fails much later
        // with a message about an unknown backend.
        assert_eq!(
            standing(&[("backends", "[[backends]]\nname = \"mymgr\"\n")], "backends"),
            Standing::NoRows
        );
    }

    #[test]
    fn a_row_of_the_right_kind_is_counted() {
        let body = "[[backend]]\nname = \"a\"\n\n[[backend]]\nname = \"b\"\n";
        assert_eq!(
            standing(&[("backends", body)], "backends"),
            Standing::InUse { rows: 2 }
        );
        assert!(standing(&[("backends", body)], "backends").effective());
    }

    #[test]
    fn a_file_that_is_not_toml_names_the_place_it_broke() {
        let s = standing(&[("secret", "this is not toml\n")], "secret");
        assert!(matches!(s, Standing::Malformed(_)), "{s:?}");
        assert!(s.is_wrong());
        assert!(s.detail().is_some_and(|d| d.contains("line")), "{s:?}");
    }

    #[test]
    fn the_row_key_being_a_table_rather_than_an_array_is_not_the_same_as_no_rows() {
        // `[backend]` instead of `[[backend]]` — one bracket, and every reader sees nothing.
        // Reported as malformed rather than empty, because the user plainly meant a row.
        let s = standing(&[("backends", "[backend]\nname = \"a\"\n")], "backends");
        assert!(matches!(s, Standing::Malformed(_)), "{s:?}");
        assert!(
            s.detail().is_some_and(|d| d.contains("[[backend]]")),
            "the message must show what a row looks like: {s:?}"
        );
    }

    #[test]
    fn the_refusal_names_the_surface_the_file_and_how_a_row_opens() {
        let msg = cannot_use(surface("firewall").unwrap(), "missing field `name`");
        assert!(msg.contains("adapters/firewall.toml"));
        assert!(msg.contains("[[firewall]]"));
        assert!(msg.contains("missing field `name`"));
        assert!(msg.contains("linix adapters"));
    }

    #[test]
    fn every_surface_is_named_once_and_resolves_by_name() {
        let mut names: Vec<&str> = SURFACES.iter().map(|s| s.name).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two surfaces share a name");
        for s in &SURFACES {
            assert!(surface(s.name).is_some());
        }
        assert!(surface("nonesuch").is_none());
    }
}
