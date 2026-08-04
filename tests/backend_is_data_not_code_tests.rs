//! Is a backend data, or is it another 300 lines of Rust?
//!
//! There are two ways a backend gets written here. The good way is a `ManagerConfig` — a record
//! saying "install with these words, remove with those, list with these" — about 23 lines, no
//! logic, parsed by machinery every other data backend shares. The other way is a module with
//! its own `impl BackendCore`, its own install loop, its own remove loop, its own argv builder:
//! 200 to 400 lines, of which most is the same shape as its neighbour's.
//!
//! 34 backends are data. 27 are modules, totalling ~4,890 non-test lines — `krew` and
//! `pubdart` came out on 2026-08-04, 390 lines of Rust replaced by two data rows. `npm.rs` and
//! `pnpm.rs` are ~85% identical once you rename; the real differences are three subcommand words
//! and one JSON quirk. A helper called `global_argv` is defined three separate times — npm,
//! pnpm, yarn — and npm's and pnpm's copies are character-for-character the same.
//!
//! **This file is the ratchet, and it deliberately comes before the conversions.** The direction
//! doc's own instruction: *"Write the ratchet before converting anything — it is worth more than
//! the conversions, and it is what stops backend thirty-nine from being written by copying
//! thirty-eight."* A conversion sweep nobody finishes leaves two ways of doing things, which is
//! how this repo got two of everything. A list that can only shrink turns "convert backend #12"
//! from a refactor somebody has to justify into a chore with a visible finish line.
//!
//! So: every backend module is data-driven, or it is named below with the reason it cannot be.
//! Adding a name is allowed and requires writing the reason. Removing one is the work.

use std::path::PathBuf;

/// A backend module that is hand-written Rust rather than a `ManagerConfig`, and why.
///
/// **A reason here is a claim about the manager, not about the calendar.** "Not converted yet"
/// is not a reason; it is the absence of one, and an exemption list of those is a list of things
/// nobody looked at (E29). Each entry says what the *generic* machinery cannot express.
struct HandWritten {
    module: &'static str,
    why: &'static str,
}

const HAND_WRITTEN: &[HandWritten] = &[
    // ---- Not package managers at all. These do not run a manager's CLI; the generic machinery
    // has nothing to configure because there is no argv template to fill in.
    HandWritten {
        module: "link.rs",
        why: "writes symlinks (or copies) through the filesystem layer and runs no command at \
              all. `ManagerConfig` is a table of argv; this backend has none.",
    },
    HandWritten {
        module: "web.rs",
        why: "fetches a URL over HTTP and writes the file itself — scheme and checksum policy, \
              not argv. Nothing to template.",
    },
    HandWritten {
        module: "github.rs",
        why: "resolves a release through the GitHub API, selects among assets by rule, and \
              records the choice in a lock. The selection logic IS the backend; there is no \
              command line anywhere in it.",
    },
    HandWritten {
        module: "appimage.rs",
        why: "downloads an image over HTTP and marks it executable through the filesystem \
              layer — `web.rs` one file format along, and argv-free for the same reason.",
    },
    HandWritten {
        module: "setting.rs",
        why: "dispatches on which settings STORE the host has (registry / gsettings / …), each \
              with read-before-write semantics. It is already data — `setting_stores.toml` — \
              but of a different table than `ManagerConfig`, which has no read-then-decide step.",
    },
    HandWritten {
        module: "service.rs",
        why: "dispatches on which init system the host has, from `init_providers.toml`, and a \
              declaration maps to a SEQUENCE of actions (enable, start) rather than one install \
              verb. Already data, again of a different table.",
    },
    HandWritten {
        module: "storage.rs",
        why: "lvm and zfs address `group/volume`, take a size, and must check existence before \
              creating. Two backends from one module, neither of which installs a package.",
    },
    HandWritten {
        module: "btrfs.rs",
        why: "subvolumes plus fstab editing plus an unmount ordering constraint — the mount must \
              be dropped before the subvolume, or the machine stops in the initramfs at next \
              boot. That ordering cannot live in an argv table.",
    },
    // ---- Package managers whose shape the generic machinery does not yet cover.
    HandWritten {
        module: "nix.rs",
        why: "removes by profile INDEX, so removal must list-then-match rather than name the \
              package, and it parses two different JSON layouts across nix versions. Generic \
              removal templates a name it does not have.",
    },
    HandWritten {
        module: "go.rs",
        why: "installs a module path and removes by deleting the binary out of `go env GOPATH`. \
              Removal is a filesystem operation informed by a query, not a manager verb.",
    },
    HandWritten {
        module: "emacs.rs",
        why: "is handed an Emacs Lisp form after `--eval`, not a subcommand. The argv is one \
              long program, which is a different thing from a template with slots.",
    },
    HandWritten {
        module: "psresource.rs",
        why: "runs PowerShell with a `-Command` script and named parameters (`-Name 'x' \
              -Scope CurrentUser`), not positional argv. `ManagerConfig` templates positions.",
    },
    HandWritten {
        module: "snap.rs",
        why: "probes with `snap info` before installing, because a classic snap needs \
              `--classic` and a non-classic one refuses it. Install is conditional on a read.",
    },
    HandWritten {
        module: "vscode.rs",
        why: "the `code` CLI is located differently per platform and per install flavour \
              (system, user, Insiders, snap), which is discovery rather than argv.",
    },
    HandWritten {
        module: "conda.rs",
        why: "environment-scoped: every verb carries `-n <env>` resolved at call time from the \
              declaration and config, so the argv is not fixed at registration.",
    },
    HandWritten {
        module: "mise.rs",
        why: "manages tool VERSIONS rather than packages — `use -g name@version` where the \
              version is part of the identity — and reads its own config to list them.",
    },
    HandWritten {
        module: "flatpak.rs",
        why: "carries an installation scope (`--system`/`--user`) and addresses applications by \
              reverse-DNS ID with an optional remote, which the name slot cannot hold alone.",
    },
    HandWritten {
        module: "brew.rs",
        why: "formulae and casks are two namespaces behind one command, and `brew list \
              --versions` and the search headers need parsing the generic parsers do not have.",
    },
    HandWritten {
        module: "pacman.rs",
        why: "the removal guard needs pacman's own essential/required-by data, and AUR helpers \
              re-use this module's syntax through a separate registrar.",
    },
    HandWritten {
        module: "dnf.rs",
        why: "reads its own history to distinguish user-installed from dependency, which is a \
              second command whose output changes what the first one means.",
    },
    HandWritten {
        module: "xbps.rs",
        why: "installs with `xbps-install` and removes with `xbps-remove` — two binaries — and \
              its listing needs the manual/automatic split from a third.",
    },
    // ---- The formulaic ones. THESE ARE THE WORK. Each is here because it has ONE piece the
    // generic machinery cannot yet express, and the rest of the file is boilerplate around it.
    HandWritten {
        module: "npm.rs",
        why: "TO CONVERT. Install/remove/upgrade are formulaic. What blocks it: `info` reports \
              an install path derived from `npm prefix -g` with a per-OS layout, and `search` \
              goes to the npm registry over HTTP rather than to a subcommand.",
    },
    HandWritten {
        module: "pnpm.rs",
        why: "TO CONVERT, and ~85% identical to npm.rs once renamed. Same two blockers, plus \
              `pnpm list -g --json` returns an ARRAY of project objects where npm returns one.",
    },
    HandWritten {
        module: "yarn.rs",
        why: "TO CONVERT. Third copy of the Node shape: `yarn global add` instead of `-g`, and \
              the same HTTP search. `global_argv` is defined here for the third time.",
    },
    HandWritten {
        module: "cargo.rs",
        why: "TO CONVERT. `cargo install --list` has an indented sub-listing of binaries that \
              the generic column parser would read as package names.",
    },
    HandWritten {
        module: "pipx.rs",
        why: "TO CONVERT. Only `info` blocks it: the venv path comes from `pipx environment \
              --value PIPX_HOME`, a second command.",
    },
    HandWritten {
        module: "uv.rs",
        why: "TO CONVERT. Same shape as pipx; `uv tool list` output needs a parser that is not \
              yet in `src/parsers/`.",
    },
];

fn backend_modules() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/backends");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("cannot read src/backends") {
        let path = entry.expect("bad entry").path();
        let Some(name) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !name.ends_with(".rs") {
            continue;
        }
        // Not backends: the generic machinery itself, the registry, the runtime-defined
        // backends, the shared capability table, a shared search helper, and test files.
        if matches!(
            name,
            "generic.rs"
                | "registry.rs"
                | "onboarder.rs"
                | "capability.rs"
                | "mod.rs"
                | "node_registry.rs"
                | "pip_search.rs"
        ) || name.ends_with("_test.rs")
        {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("cannot read module");
        out.push((name.to_string(), src));
    }
    out
}

/// **The ratchet.** A backend module is data, or it is named with a reason.
#[test]
fn every_backend_is_data_or_says_why_not() {
    let modules = backend_modules();
    assert!(
        modules.len() > 20,
        "found only {} backend modules — the scan is broken, not the code",
        modules.len()
    );

    let mut unexplained: Vec<String> = Vec::new();
    for (name, src) in &modules {
        let hand_written = src.contains("impl BackendCore for");
        let listed = HAND_WRITTEN.iter().any(|h| h.module == name);
        if hand_written && !listed {
            unexplained.push(name.clone());
        }
    }

    assert!(
        unexplained.is_empty(),
        "these backend modules are hand-written Rust with no reason given:\n    {}\n\n\
         Build the backend from a `ManagerConfig` in registry.rs — adding a backend should be \
         adding data — or add it to HAND_WRITTEN with what the generic machinery cannot express. \
         \"Not converted yet\" is not a reason; it is the absence of one.",
        unexplained.join("\n    ")
    );
}

/// The list may only shrink. An entry for a module that is now data — or one that never
/// existed — is an exemption nobody re-read, and it is what makes the finish line move away.
#[test]
fn the_hand_written_list_has_no_stale_entries() {
    let modules = backend_modules();
    let mut stale: Vec<&str> = Vec::new();
    for h in HAND_WRITTEN {
        match modules.iter().find(|(n, _)| n == h.module) {
            None => stale.push(h.module),
            Some((_, src)) if !src.contains("impl BackendCore for") => stale.push(h.module),
            Some(_) => {}
        }
    }
    assert!(
        stale.is_empty(),
        "HAND_WRITTEN names modules that are gone or are already data-driven: {stale:?}\n\n\
         Delete the entry. This list is the finish line, and a finish line that keeps entries \
         it no longer needs never arrives.",
    );
}

/// A reason is the exemption. This is the assertion that stops the list becoming a formality.
#[test]
fn every_reason_says_what_the_generic_machinery_cannot_express() {
    for h in HAND_WRITTEN {
        assert!(
            h.why.len() > 60,
            "{}'s reason has no substance: {:?}",
            h.module,
            h.why
        );
        assert!(
            !h.why.to_lowercase().contains("not converted yet")
                || h.why.contains("What blocks it")
                || h.why.contains("blocks it"),
            "{}'s reason is a schedule, not a constraint: {:?}",
            h.module,
            h.why
        );
    }
}

/// How far there is to go, printed rather than asserted.
///
/// **Not a threshold.** A test that failed when the count rose would be gamed by adding a
/// reason, and one that failed when it fell would have to be edited on every conversion. The
/// number is here to be read — `cargo test --test backend_is_data_not_code_tests -- --nocapture`
/// — because the direction doc asks each phase to say what it bought, and a count nobody can see
/// is a count nobody writes down.
#[test]
fn report_the_distance_to_go() {
    let modules = backend_modules();
    let hand = modules
        .iter()
        .filter(|(_, s)| s.contains("impl BackendCore for"))
        .count();
    let to_convert = HAND_WRITTEN
        .iter()
        .filter(|h| h.why.starts_with("TO CONVERT"))
        .count();
    eprintln!(
        "backend modules: {} total, {hand} hand-written, {to_convert} of those marked TO CONVERT",
        modules.len()
    );
    assert!(hand >= to_convert);
}
