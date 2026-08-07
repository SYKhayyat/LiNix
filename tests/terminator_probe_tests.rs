//! Does `--` still mean "options end here" to the tool LiNix says it means it to?
//!
//! `src/core/argv.rs` holds one boolean per binary, and four of those booleans have been wrong:
//! `asdf` read the terminator as a plugin name, `spack` read it into the spec, `gem` read it as
//! the start of C-extension build arguments and installed nothing, `nimble` handed it to the Nim
//! compiler and broke every build that produced a binary. Each was added because someone
//! recognised the family. **A family resemblance is not a measurement**, and until this file
//! there was nothing that could tell the difference.
//!
//! **The probe is differential, so it does not have to understand any tool's error prose.** The
//! same argv is run twice — once without the terminator, then once with it in front of the
//! operand — and the tool is believed to honour `--` when the two runs agree. Agreement is: the
//! same exit code, the same answer to "did it echo the operand back **as a word of its own**",
//! and no bare `--` anywhere in the output.
//!
//! Each of those three signals was earned. The operand check counts whole tokens because
//! `spack install -- <name>` answers `Spec ~~<name> has no name`: the terminator was absorbed
//! into the operand rather than dropped, so a substring test finds the name and calls it a pass.
//! **Every argv a binary produces is checked, not the first**, because `nimble install -- <x>`
//! looks clean while `nimble uninstall -- <x>` answers `Unknown option: --` — one binary whose
//! verbs disagree, and the conservative merge is the only safe one: a binary terminates when
//! every one of its verbs does. The without-run goes first so a tool's one-time first-run banner
//! (`Welcome to .NET 8.0!`) lands there and not in the output being read for a stray `--`.
//!
//! **What it cannot see**: a tool that parses `--` happily and mishandles it later. `nimble`
//! passes everything after `--` to the Nim compiler, which only breaks when a package actually
//! builds a binary — and a probe that never resolves a real package never reaches that. This is
//! why the table's rows carry the tool's own sentence rather than this probe's verdict: the
//! probe is a ratchet against rows nobody asked, not a replacement for having asked.
//!
//! **The argvs come from the registry, not from a list here.** A hand-written table of "the verb
//! to probe each manager with" is a second copy of the truth, and the second copy is what goes
//! stale — which is this whole file's own subject, one level up. Every backend is driven through
//! a mock, the argv it would really have run is read back, and the operand is a sentinel so its
//! position is never in doubt.
//!
//! Opt-in, because it runs the managers for real: `TERMINATOR_PROBE=1`. The names are bogus, so
//! every run fails at resolution and nothing is installed — but it is still a machine being
//! spoken to, and a test that mutates a machine should be asked for. CI runs it inside the
//! `tools` and distro images, which is where the managers are.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

/// The operand, chosen so its position in an argv is never ambiguous and no manager can
/// resolve it.
const SENTINEL: &str = "linix-no-such-thing";

struct Run {
    code: Option<i32>,
    text: String,
}

/// Run a manager the way LiNix runs it — through the same shim-aware launcher, or a `.cmd`
/// manager is silently "unreachable" and every claim about it goes unchecked.
///
/// **In a scratch directory, never the repo.** A build tool handed a package name treats the
/// current directory as a project: the first run of this probe left a `dist-newstyle/` in the
/// working tree, because `cabal install <name>` writes its build cache wherever it is standing.
/// A test that alters the tree it is testing is a test nobody should have to think about.
fn run(program: &str, args: &[String], cwd: &std::path::Path) -> Option<Run> {
    let (prog, argv) = linix::core::executor::effective_command(program, args);
    // stdin closed: `mix` prompts `Shall I install Hex? [Yn]` and will read whatever is on the
    // handle. During development it read the rest of the probe script and the remaining
    // measurements silently never ran (II.12c, one layer out).
    let out = Command::new(prog)
        .args(&argv)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(Run {
        code: out.status.code(),
        text: format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    })
}

/// Does this output contain `--` standing on its own — the tool talking about the terminator
/// as if it were a name?
///
/// `asdf` says `No such plugin: --`; `nimble` says `Unknown option: --`; `spack` echoes
/// `-- linix-no-such-thing` back as the query it could not match. A tool that consumed the
/// terminator has nothing to say about it.
///
/// **Only meaningful against the run that had no terminator to talk about** — see
/// [`honours_terminator`]. composer's failure dumps its own usage line, which reads
/// `... [--] [<packages>...]`, and a `--` the tool printed from its own help is not a `--` it
/// mistook for a package.
fn mentions_bare_terminator(text: &str) -> bool {
    text.split_whitespace().any(|t| trim_wrapping(t) == "--")
}

/// Strip the punctuation tools wrap a name in when they quote it back.
///
/// **Not `-` and not `~`.** `spack` renders a swallowed terminator as `~~<name>`, so trimming
/// `~` would turn the one case this exists to catch into a clean pass.
fn trim_wrapping(token: &str) -> &str {
    token.trim_matches([
        '\'', '"', '`', ',', '.', ':', ';', '(', ')', '[', ']', '!', '?',
    ])
}

/// Did the tool echo the operand back as a word of its own?
///
/// Whole tokens, not `contains`. `spack install -- <name>` answers `Spec ~~<name> has no name`:
/// the operand is in there, glued to the terminator the tool failed to drop, and a substring
/// test reads that as the tool having understood us.
fn echoes_operand(text: &str) -> bool {
    text.split_whitespace()
        .any(|t| trim_wrapping(t) == SENTINEL)
}

/// The verdict for one argv: did the terminator disappear into the parser, or into an operand?
///
/// **Every signal is differential**, including the bare-`--` one. A tool that dumps its own
/// usage on failure prints `--` either way, and reading that as "the tool is complaining about
/// the terminator" made composer — which honours `--`, names the operand, and produces
/// byte-identical output both ways — come back as a violation. The question is never "does this
/// output contain X", always "did adding the terminator change X".
fn honours_terminator(with: &Run, without: &Run) -> bool {
    with.code == without.code
        && mentions_bare_terminator(&with.text) == mentions_bare_terminator(&without.text)
        && echoes_operand(&with.text) == echoes_operand(&without.text)
}

fn on_path(program: &str) -> bool {
    which::which(program).is_ok()
}

/// The argv with `--` immediately before the operand, and the argv with no `--` at all.
fn both_forms(tokens: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let at = tokens.iter().position(|t| t == SENTINEL)?;
    let without: Vec<String> = tokens
        .iter()
        .filter(|t| t.as_str() != "--")
        .cloned()
        .collect();
    let mut with: Vec<String> = tokens
        .iter()
        .filter(|t| t.as_str() != "--")
        .cloned()
        .collect();
    // `at` indexes the original; recompute against the stripped copy.
    let at = with.iter().position(|t| t == SENTINEL).unwrap_or(at);
    with.insert(at, "--".to_string());
    Some((with, without))
}

#[tokio::test]
async fn every_terminator_claim_still_holds_where_the_tool_is_installed() {
    use dashmap::DashMap;
    use linix::core::executor::MockExecutor;
    use linix::core::{CommandExecutor, PackageSpec};
    use std::sync::Arc;

    if std::env::var("TERMINATOR_PROBE").is_err() {
        // Loudly, not silently. A gate that skips without saying so reads as a pass.
        eprintln!(
            "terminator probe: SKIPPED — set TERMINATOR_PROBE=1 to run it. It invokes the real \
             managers (with an unresolvable package name, so nothing installs)."
        );
        return;
    }

    // Where the managers are run from — see `run`. Dropped at the end of the test with whatever
    // any of them decided to write there.
    let scratch = tempfile::tempdir().expect("scratch dir");

    let vfs = Arc::new(DashMap::new());
    let mock = Arc::new(MockExecutor::new(vfs.clone()));
    let exec =
        CommandExecutor::with_layer(true, false, mock.clone(), vfs, Arc::new(DashMap::new()));
    let config = linix::config::Config::default();
    let registry = linix::backends::create_default_registry(
        exec,
        &config,
        Arc::new(linix::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    // Drive the three verbs that carry an operand. `available()`, not `all()`: a backend whose
    // program is absent cannot answer, and asking anyway is how a missing manager gets reported
    // as a defect.
    for backend in registry.available() {
        let spec = PackageSpec {
            name: SENTINEL.into(),
            backend: backend.name().into(),
            ..Default::default()
        };
        if let Some(i) = backend.as_installable() {
            let _ = i.install(std::slice::from_ref(&spec), false).await;
            let _ = i.remove(&[SENTINEL.to_string()], false, linix::app::sync::guard::Reaped::for_reason(linix::app::sync::guard::GuardScope::Remove, "a unit test of the effector itself")).await;
        }
        if let Some(s) = backend.as_searchable() {
            let _ = s.search(SENTINEL).await;
        }
    }

    let claims: BTreeMap<&str, bool> = linix::core::argv::known_terminator_claims()
        .into_iter()
        .collect();

    let mut wrong: Vec<String> = Vec::new();
    let mut confirmed: BTreeMap<String, bool> = BTreeMap::new();
    let mut probed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut verbs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut absent: BTreeSet<String> = BTreeSet::new();
    let mut undriven: BTreeSet<String> = claims.keys().map(|k| k.to_string()).collect();

    for call in mock.get_calls().await {
        let mut tokens: Vec<String> = call.split_whitespace().map(str::to_string).collect();
        // Root-elevated calls are recorded with their elevation; the parser under test is the
        // manager's, not sudo's.
        if tokens.first().is_some_and(|t| t == "sudo") {
            tokens.remove(0);
        }
        let Some(program) = tokens.first().cloned() else {
            continue;
        };
        let base = program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&program)
            .strip_suffix(".exe")
            .unwrap_or_else(|| program.rsplit(['/', '\\']).next().unwrap_or(&program))
            .to_string();
        // In the table at all? A program the table has no row for — `sh`, `sudo`, a helper —
        // has no claim to check. The verdict itself is decided per binary after this loop.
        if !claims.contains_key(base.as_str()) {
            continue;
        }
        undriven.remove(base.as_str());
        if !on_path(&program) {
            absent.insert(base);
            continue;
        }
        let Some((with, without)) = both_forms(&tokens[1..]) else {
            continue;
        };
        // Every verb, not the first one seen. `nimble install -- <x>` gets as far as package
        // resolution and looks clean; `nimble uninstall -- <x>` answers `Unknown option: --`.
        // One binary, two parsers' worth of behaviour, and stopping at the first verdict picked
        // whichever the registry happened to build first.
        if !probed.insert((base.clone(), with.join(" "))) {
            continue;
        }
        // The without-run first: a one-time first-run banner (`Welcome to .NET 8.0!`) then lands
        // there, and the run being read for a stray `--` is the tool's ordinary output.
        let (Some(without_run), Some(with_run)) = (
            run(&program, &without, scratch.path()),
            run(&program, &with, scratch.path()),
        ) else {
            absent.insert(base);
            continue;
        };
        let honours = honours_terminator(&with_run, &without_run);
        // The conservative merge: a binary terminates only if EVERY verb of it does. One verb
        // that swallows the operand is enough to make the terminator unsafe for that binary,
        // because the table is keyed on the binary (Q30 — per-verb keying was measured and
        // rejected: `gem list -- <x>` does not error, it silently lists everything).
        confirmed
            .entry(base.clone())
            .and_modify(|v| *v &= honours)
            .or_insert(honours);
        // Kept per verb, judged per binary below. Reporting a single verb as a violation is
        // how `nimble install` (which parses `--` happily) and `spack list` (which has no
        // operand to mangle) each came back as a disagreement while the binary's merged
        // verdict agreed with its row perfectly.
        verbs.entry(base).or_default().push(format!(
            "    {}: {} {}\n      -> with    exit {:?}: {}\n      -> without exit {:?}: {}",
            if honours { "agrees" } else { "swallows" },
            program,
            with.join(" "),
            with_run.code,
            with_run.text.lines().next().unwrap_or("").trim(),
            without_run.code,
            without_run.text.lines().next().unwrap_or("").trim(),
        ));
    }

    for (base, &honours) in &confirmed {
        let Some(&claim) = claims.get(base.as_str()) else {
            continue;
        };
        if honours == claim {
            continue;
        }
        let (measured, why) =
            linix::core::argv::terminator_evidence(base).unwrap_or((false, "no evidence recorded"));
        wrong.push(format!(
            "`{base}` is listed as {} and behaves as {}.\n  the row's evidence ({}): {why}\n\
             \n  every verb this run drove:\n{}",
            if claim {
                "terminating"
            } else {
                "NOT terminating"
            },
            if honours {
                "terminating"
            } else {
                "NOT terminating"
            },
            if measured {
                "someone ran it"
            } else {
                "nobody had run it"
            },
            verbs.get(base).map(|v| v.join("\n")).unwrap_or_default(),
        ));
    }

    // Printed on every run, pass or fail. These names are what lets `UNASKED_CEILING` come
    // down: a row this probe has confirmed is a row that no longer has to say nobody asked.
    eprintln!(
        "terminator probe: {} binaries measured here",
        confirmed.len()
    );
    for (b, honours) in &confirmed {
        eprintln!(
            "  measured: {b} {}",
            if *honours {
                "terminates"
            } else {
                "does NOT terminate"
            }
        );
    }
    for b in &absent {
        eprintln!("  skipped: {b} (not installed here)");
    }
    for b in &undriven {
        eprintln!("  skipped: {b} (no backend built an argv carrying an operand for it here)");
    }

    assert!(
        wrong.is_empty(),
        "the terminator table disagrees with the tools themselves:\n\n{}\n\n\
         The table's default is \"does not terminate\", and a binary joins the terminating set \
         when someone has checked its argument parser. Fix the row in `src/core/argv.rs` and put \
         the tool's own words in its evidence.",
        wrong.join("\n\n")
    );
}
