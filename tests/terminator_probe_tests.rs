//! Does `--` still mean "options end here" to the tool Shall says it means it to?
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
//! **Agreement only counts once the tool has reached the operand**, and for two versions of this
//! file it did not have to. A manager that fails before resolution — no writable `COMPOSER_HOME`,
//! no network, a search that answers `[]` — agrees with itself perfectly and says nothing at all
//! about its parser, and every such pair was read as a clean pass. So there are three verdicts
//! now ([`Verdict`]) and the vacuous ones are inconclusive rather than confirming. It is not a
//! hypothetical: composer's row was flipped to non-terminating on nightlies of exactly this
//! shape, and a container run with a flag-shaped operand then showed it honouring `--` on every
//! verb. Note which way the old bug could err — a vacuous pass can only move a row *into* the
//! terminating set, which is the unsafe half of `src/core/argv.rs`'s default.
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
//! **A pair that disagrees is measured again before it is believed** ([`ATTEMPTS`]), and when it
//! still disagrees the report names the signal that moved and quotes both runs whole. Every
//! signal here is read off prose a manager prints only once it has resolved the operand, so a
//! network that dropped one of the two consecutive runs looks exactly like a parser that ate the
//! terminator — and a first-line-only report cannot tell the two apart afterwards.
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
const SENTINEL: &str = "shall-no-such-thing";

struct Run {
    code: Option<i32>,
    text: String,
}

/// Run a manager the way Shall runs it — through the same shim-aware launcher, or a `.cmd`
/// manager is silently "unreachable" and every claim about it goes unchecked.
///
/// **In a scratch directory, never the repo.** A build tool handed a package name treats the
/// current directory as a project: the first run of this probe left a `dist-newstyle/` in the
/// working tree, because `cabal install <name>` writes its build cache wherever it is standing.
/// A test that alters the tree it is testing is a test nobody should have to think about.
fn run(program: &str, args: &[String], cwd: &std::path::Path) -> Option<Run> {
    let (prog, argv) = shall::core::launch::effective_command(program, args);
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
/// `-- shall-no-such-thing` back as the query it could not match. A tool that consumed the
/// terminator has nothing to say about it.
///
/// **Only meaningful against the run that had no terminator to talk about** — see
/// [`disagreement`]. composer's failure dumps its own usage line, which reads
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

/// What one argv's pair of runs establishes.
///
/// **Three answers, not two.** A tool that fails before it ever resolves the operand agrees
/// with itself perfectly — same exit code, same silence about `--`, same absence of the
/// operand — and for two versions of this file that read as [`Verdict::Honours`]. It is not:
/// it is a measurement that did not happen. See [`assess`].
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The tool resolved the operand and adding `--` changed nothing.
    Honours,
    /// A signal moved when the terminator was added, and this names which.
    Swallows(&'static str),
    /// The runs agree, but the tool never got far enough for the agreement to mean anything.
    Inconclusive,
}

/// The verdict for one argv.
///
/// **A difference is self-evident; an agreement is not.** A signal that moves can only have
/// been moved by the terminator, so [`disagreement`] is read first and its answer is always
/// conclusive. Agreement is the case that needs a premise: the tool has to have *reached* the
/// operand before "it behaved the same either way" says anything about its parser.
///
/// The premise is read off the run that had no terminator at all — if that run never names the
/// operand as a word of its own, the tool never resolved it, and both runs are agreeing about
/// something else entirely. `composer global search --format=json <bogus>` answers `[]`;
/// `composer global require <bogus>` on a host with no writable `COMPOSER_HOME` answers
/// `No composer.json present in the current directory`. Both agree with themselves under `--`
/// and neither is evidence about `--`.
///
/// **This cost the composer row two wrong values in three days.** It was flipped to
/// non-terminating on nightlies whose runs never resolved the operand, then a container run
/// with a flag-shaped operand showed it honouring the terminator on all three verbs. Note the
/// direction of the old bug: a vacuous pass can only ever push a row *into* the terminating
/// set, which is the unsafe half of `src/core/argv.rs`'s "default is does not terminate".
///
/// The cost of the premise is real and worth naming: a tool that honours `--` and answers
/// silently — `gem list -- <x>` lists everything rather than erroring — is now inconclusive
/// rather than confirmed. That is the conservative direction. This probe is a ratchet against
/// rows nobody asked, and declining to confirm is not the same as reporting a violation.
fn assess(with: &Run, without: &Run) -> Verdict {
    match disagreement(with, without) {
        Some(why) => Verdict::Swallows(why),
        None if !echoes_operand(&without.text) => Verdict::Inconclusive,
        None => Verdict::Honours,
    }
}

/// Which signal moved when the terminator was added, or `None` if the two runs agree.
///
/// **Every signal is differential**, including the bare-`--` one. A tool that dumps its own
/// usage on failure prints `--` either way, and reading that as "the tool is complaining about
/// the terminator" made composer — which names the operand and produces byte-identical output
/// both ways — come back as a violation for the wrong reason. The question is never "does this
/// output contain X", always "did adding the terminator change X".
///
/// The answer names the signal because the failure report is the only thing a nightly leaves
/// behind. A report that says "swallows" and prints one line of each run cannot be acted on:
/// composer's two runs open with the same `Changed current directory to …` and differ, if at
/// all, twenty lines further down.
fn disagreement(with: &Run, without: &Run) -> Option<&'static str> {
    if with.code != without.code {
        Some("the exit code changed")
    } else if mentions_bare_terminator(&with.text) != mentions_bare_terminator(&without.text) {
        Some("one run talks about a bare `--` and the other does not")
    } else if echoes_operand(&with.text) != echoes_operand(&without.text) {
        Some("only one run echoed the operand back as a word of its own")
    } else {
        None
    }
}

/// How many times a disagreeing pair is measured again before the disagreement is believed.
///
/// **A parser's answer is the same twice; a network is not.** Every signal here is read off
/// prose the tool prints *after* it has resolved the operand — `composer global require
/// <bogus>` has to reach packagist before it can say the name matches nothing. A run that
/// cannot get there fails with the same exit code, never names the operand, and is
/// indistinguishable to all three signals from a parser that ate it. That is one run of one
/// verb turning a nightly red on evidence about someone's DNS.
///
/// Re-measuring costs nothing on a real finding — a swallowed operand is swallowed every time —
/// and it is the difference between a gate and a coin toss.
///
/// **[`Verdict::Honours`] on any attempt is the answer, and it is the only verdict that can end
/// the loop early.** Both of the others are producible by a bad minute on the network: a run
/// that dies before resolution can differ from its partner (a spurious `Swallows`) or match it
/// while proving nothing (a spurious `Inconclusive`). Nothing transient can manufacture the
/// third — a tool that never resolved the operand cannot echo it back, and a tool that honours
/// `--` cannot be made to disagree with itself by repetition.
const ATTEMPTS: usize = 3;

/// One verb, measured until it stops changing its mind: the without-run, then the with-run,
/// repeated while they disagree.
///
/// The without-run goes first every time — a one-time first-run banner (`Welcome to .NET 8.0!`)
/// then lands there, and the run being read for a stray `--` is the tool's ordinary output.
///
/// `None` means the binary could not be launched at all, which is a missing tool and not a
/// finding about anyone's argument parser.
fn measure(
    program: &str,
    with: &[String],
    without: &[String],
    cwd: &std::path::Path,
) -> Option<Measured> {
    believe(|| {
        let without_run = run(program, without, cwd)?;
        let with_run = run(program, with, cwd)?;
        Some((with_run, without_run))
    })
}

/// The rule itself, over anything that yields a pair: keep measuring while the two disagree,
/// and stop the moment they don't. Separated from spawning the manager so the retry can be
/// tested against a source that misbehaves on cue — an untested retry is a claim, and this one
/// exists to decide whether a nightly is red.
fn believe(mut pair: impl FnMut() -> Option<(Run, Run)>) -> Option<Measured> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let (with, without) = pair()?;
        let verdict = assess(&with, &without);
        if verdict == Verdict::Honours || attempts == ATTEMPTS {
            return Some(Measured {
                verdict,
                attempts,
                with,
                without,
            });
        }
    }
}

/// The last pair of runs for one verb, and what it took to believe them.
struct Measured {
    verdict: Verdict,
    attempts: usize,
    with: Run,
    without: Run,
}

fn on_path(program: &str) -> bool {
    which::which(program).is_ok()
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("").trim()
}

/// The tool's own output, indented under the report and bounded so one chatty manager cannot
/// bury the finding next to it. **Truncation says so.** A report that quietly drops the rest is
/// how this file spent its first run printing one line per run and proving nothing.
fn quoted(text: &str) -> String {
    const KEEP: usize = 60;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return "        | (no output)".to_string();
    }
    let mut out: Vec<String> = lines
        .iter()
        .take(KEEP)
        .map(|l| format!("        | {}", l.trim_end()))
        .collect();
    if let Some(dropped) = lines.len().checked_sub(KEEP).filter(|d| *d > 0) {
        out.push(format!("        | … {dropped} more line(s), not shown"));
    }
    out.join("\n")
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
    use shall::core::executor::MockExecutor;
    use shall::core::{CommandExecutor, PackageSpec};
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
    let config = shall::config::Config::default();
    let registry = shall::backends::create_default_registry(
        exec,
        &config,
        Arc::new(shall::app::hooks::LuaHooks::new(&config).expect("hooks")),
    )
    .await;

    // Drive the three verbs that carry an operand. `available()`, not `all()`: a backend whose
    // program is absent cannot answer, and asking anyway is how a missing manager gets reported
    // as a defect.
    for backend in registry.present_on_this_machine() {
        let spec = PackageSpec {
            name: SENTINEL.into(),
            backend: backend.name().into(),
            ..Default::default()
        };
        if let Some(i) = backend.as_installable() {
            let _ = i.install(std::slice::from_ref(&spec), false).await;
            let _ = i
                .remove(
                    &[SENTINEL.to_string()],
                    false,
                    shall::app::sync::guard::Reaped::for_reason(
                        shall::app::sync::guard::GuardScope::Remove,
                        "a unit test of the effector itself",
                    ),
                )
                .await;
        }
        if let Some(s) = backend.as_searchable() {
            let _ = s.search(SENTINEL).await;
        }
    }

    let claims: BTreeMap<&str, bool> = shall::core::argv::known_terminator_claims()
        .into_iter()
        .collect();

    let mut wrong: Vec<String> = Vec::new();
    let mut confirmed: BTreeMap<String, bool> = BTreeMap::new();
    let mut probed: BTreeSet<(String, String)> = BTreeSet::new();
    let mut verbs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut absent: BTreeSet<String> = BTreeSet::new();
    let mut inconclusive: BTreeSet<String> = BTreeSet::new();
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
        let Some(m) = measure(&program, &with, &without, scratch.path()) else {
            absent.insert(base);
            continue;
        };
        // An inconclusive verb is not a vote. It joins neither side of the merge below, and a
        // binary whose every verb is inconclusive never reaches `confirmed` at all — so it can
        // neither confirm its row nor contradict it. Counting these as "honours" is what put
        // two different wrong values in composer's row inside three days.
        match m.verdict {
            Verdict::Inconclusive => {
                inconclusive.insert(base.clone());
            }
            // The conservative merge: a binary terminates only if EVERY verb of it does. One
            // verb that swallows the operand is enough to make the terminator unsafe for that
            // binary, because the table is keyed on the binary (Q30 — per-verb keying was
            // measured and rejected: `gem list -- <x>` does not error, it silently lists
            // everything).
            _ => {
                let honours = m.verdict == Verdict::Honours;
                confirmed
                    .entry(base.clone())
                    .and_modify(|v| *v &= honours)
                    .or_insert(honours);
            }
        }
        // Kept per verb, judged per binary below. Reporting a single verb as a violation is
        // how `nimble install` (which parses `--` happily) and `spack list` (which has no
        // operand to mangle) each came back as a disagreement while the binary's merged
        // verdict agreed with its row perfectly.
        //
        // A verb that agrees is one line; a verb that swallows brings both runs in full. The
        // asymmetry is the point — the disagreeing verb is the only one anybody reads, and it
        // is the one the old format summarised away.
        verbs.entry(base).or_default().push(match m.verdict {
            Verdict::Honours => format!(
                "    agrees: {} {}\n      -> with    exit {:?}: {}\n      -> without exit {:?}: {}",
                program,
                with.join(" "),
                m.with.code,
                first_line(&m.with.text),
                m.without.code,
                first_line(&m.without.text),
            ),
            // The without-run in full: the claim being made is "this tool never resolved the
            // operand", and the only thing that can support or refute it is what the tool
            // actually said when it had no terminator to blame.
            Verdict::Inconclusive => format!(
                "    inconclusive: {} {}\n      the run without the terminator never named \
                 `{SENTINEL}`, so it never resolved the operand and the two runs agree about \
                 nothing — after {} attempt(s)\n      -> without exit {:?}:\n{}",
                program,
                with.join(" "),
                m.attempts,
                m.without.code,
                quoted(&m.without.text),
            ),
            Verdict::Swallows(why) => format!(
                "    swallows: {} {}\n      {why} — still, after {} attempt(s)\n      \
                 -> with    exit {:?}:\n{}\n      -> without exit {:?}:\n{}",
                program,
                with.join(" "),
                m.attempts,
                m.with.code,
                quoted(&m.with.text),
                m.without.code,
                quoted(&m.without.text),
            ),
        });
    }

    let mut could_be_upgraded: Vec<String> = Vec::new();
    for (base, &honours) in &confirmed {
        let Some(&claim) = claims.get(base.as_str()) else {
            continue;
        };
        if honours == claim {
            continue;
        }
        // **A row whose hosts disagree cannot be right on both of them**, and this probe runs on
        // three. `stack` honours the terminator on the tools image and on ubuntu-latest and eats
        // the operand on windows-latest — measured, both ways, by this same code. Whichever value
        // the row carried, one platform's nightly went red, and a gate that is a coin toss on
        // which runner you read is a gate people learn to ignore.
        //
        // So the row declares the divergence (`Evidence::Divergent`), takes the refusing answer,
        // and this accepts the safe direction from a host that disagrees. The unsafe direction —
        // the row says it terminates and this host swallows the operand — is never exempt, and
        // `a_divergent_row_takes_the_refusing_answer` makes it unreachable by construction: a
        // divergent row is always `false`, so `claim` here is always `false` and the only
        // disagreement it can produce is the harmless one.
        if honours && !claim && shall::core::argv::terminator_answer_differs_by_host(base) {
            could_be_upgraded.push(format!(
                "{base} (recorded as differing by host; this one honours the terminator)"
            ));
            continue;
        }
        let (measured, why) =
            shall::core::argv::terminator_evidence(base).unwrap_or((false, "no evidence recorded"));
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
    // Named, and named separately from "not installed". A binary that is here, was driven, and
    // still could not be measured is a hole in this gate's coverage on this host — and the one
    // failure mode that reads as a clean pass if it is not printed. `confirmed` above lists what
    // the probe knows; this lists what it went and failed to find out.
    for b in &inconclusive {
        let also_measured = confirmed.contains_key(b);
        eprintln!(
            "  inconclusive: {b} ({})",
            match also_measured {
                true => "some verbs answered, these did not",
                false => "no verb of it ever resolved the operand here, so its row is unchecked",
            }
        );
    }
    for b in &undriven {
        eprintln!("  skipped: {b} (no backend built an argv carrying an operand for it here)");
    }
    // Printed rather than swallowed. A divergence that is only a `continue` is an exemption
    // nobody ever reads again, and the next person to ask whether `stack` can be upgraded needs
    // to see that this host said yes.
    for b in &could_be_upgraded {
        eprintln!("  divergent: {b}");
    }

    // **Learning nothing is not the same as finding nothing wrong.** `wrong` is empty both when
    // every row was confirmed and when no row could be read at all, and only one of those is a
    // pass — the vacuous pass this file's header warns about, with nothing asserting against it.
    //
    // **This has not been observed, and the honest note is that it has not.** What prompted it
    // was narrower: the arch image began running its harness unprivileged so `yay` and `paru`
    // could be driven, and on that user `pacman` — whose row reads `Measured` on the strength of
    // this leg — comes back inconclusive while six other binaries still answer. So the leg went
    // on passing while it had quietly stopped checking one row, and the run that catches THAT is
    // the one nobody has written. This assertion catches only the whole-leg version of it, and
    // is here because it costs nothing and the partial version proved the shape is reachable.
    //
    // Nothing installed at all is a different case and stays quiet: `inconclusive` is only
    // populated by a binary that was present, was driven, and still would not answer.
    assert!(
        !confirmed.is_empty() || inconclusive.is_empty(),
        "this probe drove {} installed binaries and could measure none of them: {:?}\n\n\
         Every one was present and answered nothing this gate could read, so it proved no row \
         on this host while passing. The usual cause is privileges — a manager that refuses \
         before it parses an operand echoes no operand either way — and the fix is to run the \
         probe as root (CI passes `-u 0` for exactly this), not to relax this assertion.",
        inconclusive.len(),
        inconclusive
    );

    assert!(
        wrong.is_empty(),
        "the terminator table disagrees with the tools themselves:\n\n{}\n\n\
         The table's default is \"does not terminate\", and a binary joins the terminating set \
         when someone has checked its argument parser. Fix the row in `src/core/argv.rs` and put \
         the tool's own words in its evidence.",
        wrong.join("\n\n")
    );
}

// The probe itself only runs where the managers are, and that is a nightly image. These run
// everywhere, on every push: the three signals and the way they are reported are what decides
// whether a row is a finding, and none of that had a test until the report of a real
// disagreement turned out to be unreadable.

fn run_of(code: i32, text: &str) -> Run {
    Run {
        code: Some(code),
        text: text.to_string(),
    }
}

#[test]
fn a_tool_that_ignores_the_terminator_entirely_agrees_with_itself() {
    let text = format!("Could not find a matching version of package {SENTINEL}.");
    assert_eq!(disagreement(&run_of(1, &text), &run_of(1, &text)), None);
}

#[test]
fn each_signal_is_named_when_it_is_the_one_that_moved() {
    let named = format!("no such package: {SENTINEL}");
    // Exit code.
    assert_eq!(
        disagreement(&run_of(2, &named), &run_of(1, &named)),
        Some("the exit code changed")
    );
    // A bare `--` the tool only mentions once it has been handed one — `asdf` says
    // `No such plugin: --`, `nimble` says `Unknown option: --`.
    assert_eq!(
        disagreement(&run_of(1, "Unknown option: --"), &run_of(1, &named)),
        Some("one run talks about a bare `--` and the other does not")
    );
    // The operand stops being a word of its own — `spack` glues the terminator onto it.
    assert_eq!(
        disagreement(
            &run_of(1, &format!("Spec ~~{SENTINEL} has no name")),
            &run_of(1, &named)
        ),
        Some("only one run echoed the operand back as a word of its own")
    );
}

#[test]
fn a_usage_line_printed_both_ways_is_not_a_complaint_about_the_terminator() {
    // composer dumps its own synopsis on failure, and it contains `[--]`. Reading that as the
    // tool objecting to the terminator is a finding manufactured out of a document rather than
    // out of a difference — the same mistake the composer row itself was built on, from the
    // other side.
    let usage = format!(
        "Could not find a matching version of package {SENTINEL}.\n\
         require [--dev] [--no-install] [--] [<packages>...]"
    );
    assert!(mentions_bare_terminator(&usage));
    assert!(echoes_operand(&usage));
    assert_eq!(disagreement(&run_of(1, &usage), &run_of(1, &usage)), None);
}

#[test]
fn quoted_output_keeps_the_whole_answer_and_owns_up_when_it_cannot() {
    let short = quoted("first\nsecond");
    assert!(short.contains("| first") && short.contains("| second"));
    assert!(!short.contains("more line(s)"));

    let long: String = (0..80).map(|i| format!("line {i}\n")).collect();
    let cut = quoted(&long);
    assert!(cut.contains("| line 59"), "keeps what it says it keeps");
    assert!(!cut.contains("| line 60"));
    assert!(cut.contains("… 20 more line(s), not shown"));

    assert!(quoted("").contains("(no output)"));
}

#[test]
fn a_pair_that_disagrees_once_and_then_agrees_is_not_a_finding() {
    let named = format!("no such package: {SENTINEL}");
    // The shape being defended against: the first run cannot reach the index, so it never names
    // the operand — same exit code, same silence about `--`, and nothing to do with parsing.
    let mut taken = 0;
    let m = believe(|| {
        taken += 1;
        Some(if taken == 1 {
            (
                run_of(1, &named),
                run_of(1, "curl error 6 while downloading"),
            )
        } else {
            (run_of(1, &named), run_of(1, &named))
        })
    })
    .expect("a pair");
    assert_eq!(m.verdict, Verdict::Honours);
    assert_eq!(m.attempts, 2, "stops the moment they agree");
}

#[test]
fn a_pair_that_keeps_disagreeing_is_believed_and_says_how_often_it_was_asked() {
    let named = format!("no such package: {SENTINEL}");
    let mut taken = 0;
    let m = believe(|| {
        taken += 1;
        Some((run_of(1, "Unknown option: --"), run_of(1, &named)))
    })
    .expect("a pair");
    assert_eq!(
        m.verdict,
        Verdict::Swallows("one run talks about a bare `--` and the other does not")
    );
    assert_eq!(m.attempts, ATTEMPTS);
    assert_eq!(taken, ATTEMPTS, "re-measured, not merely re-read");
}

// ---- The premise: agreement is only evidence once the tool reached the operand. ------------
//
// These are the composer bug, in both of the shapes that produced a wrong row. Neither is
// hypothetical prose: the strings below are what composer actually printed on the hosts whose
// nightlies decided the row.

#[test]
fn a_tool_that_never_resolved_the_operand_proves_nothing_by_agreeing() {
    // ubuntu-latest, nightly 2026-08-14: composer never reached packagist because it had no
    // project to work in. Byte-identical, exit-identical, and evidence about nothing.
    let dead = "::error ::No composer.json present in the current directory (./composer.json)";
    assert_eq!(
        assess(&run_of(1, dead), &run_of(1, dead)),
        Verdict::Inconclusive,
        "the operand is never named, so neither run got far enough to have a parser opinion"
    );

    // The same shape with nothing to say at all: `composer global search --format=json <bogus>`.
    assert_eq!(
        assess(&run_of(0, "[]"), &run_of(0, "[]")),
        Verdict::Inconclusive
    );
}

#[test]
fn agreement_counts_once_the_operand_comes_back_named() {
    // The tools image, same command, on a host where composer could resolve. This is the only
    // one of the three that is a measurement.
    let named = format!("Could not find a matching version of package {SENTINEL}.");
    assert_eq!(
        assess(&run_of(1, &named), &run_of(1, &named)),
        Verdict::Honours
    );
}

#[test]
fn a_moved_signal_is_conclusive_even_when_the_operand_is_never_named() {
    // `asdf install -- <x>` answers `No such plugin: --` and the operand appears in neither
    // run. The premise is only required for AGREEMENT — a difference can only have been made
    // by the terminator, so it is believed without it. Reversing this order would have made
    // asdf, spack and nimble inconclusive and un-caught.
    assert_eq!(
        assess(
            &run_of(1, "No such plugin: --"),
            &run_of(1, "No such plugin")
        ),
        Verdict::Swallows("one run talks about a bare `--` and the other does not")
    );
}

#[test]
fn an_inconclusive_pair_is_re_measured_before_it_is_believed() {
    // A tool that could not reach its index on the first attempt and could on the second is a
    // network story, not a parser story — the same defence `ATTEMPTS` gives a spurious
    // difference, owed equally to a spurious silence.
    let named = format!("no such package: {SENTINEL}");
    let mut taken = 0;
    let m = believe(|| {
        taken += 1;
        Some(match taken {
            1 => (run_of(1, "curl error 6"), run_of(1, "curl error 6")),
            _ => (run_of(1, &named), run_of(1, &named)),
        })
    })
    .expect("a pair");
    assert_eq!(m.verdict, Verdict::Honours);
    assert_eq!(m.attempts, 2, "stops the moment one run resolves the name");

    // And when it never resolves, the answer stands after the full budget rather than being
    // quietly upgraded to a pass.
    let mut taken = 0;
    let m = believe(|| {
        taken += 1;
        Some((run_of(0, "[]"), run_of(0, "[]")))
    })
    .expect("a pair");
    assert_eq!(m.verdict, Verdict::Inconclusive);
    assert_eq!(taken, ATTEMPTS, "re-measured, not merely re-read");
}

#[test]
fn a_binary_that_cannot_be_launched_is_no_finding_at_all() {
    assert!(believe(|| None).is_none());
}
