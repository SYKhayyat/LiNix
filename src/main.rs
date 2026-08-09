use anyhow::{Context, Result};
use clap::Parser;
use linix::app::App;
use linix::cli::{Cli, Commands};
use linix::core::Output;
use std::collections::HashMap;
use std::env;
use tracing::warn;
use tracing_subscriber::EnvFilter;

// The dispatcher does reference every handler, so it globs the nine modules — and that is a
// different relationship from the one `LX-11` was about. What was deleted is `verbs::prelude`
// re-exporting all nine into *each other*, which left no module boundary inside `verbs/` at
// all: 8,587 lines in one namespace stored in nine files, where moving a function between them
// was a no-op. The siblings now import each other by name (`grep "^use crate::verbs::"
// src/verbs/*.rs` is the map, and it is short), which is what makes a rule about where a
// handler belongs something a person can state and a compiler can check.
//
// This glob stays honest only while it is the dispatcher's. If a *tenth* consumer appears, it
// is a sibling and it should import by name.
use linix::verbs::{check::*, cleanup::*, declare::*, history::*, packages::*, plan::*, setup::*, sync::*, upgrade::*};

#[tokio::main]
async fn main() -> Result<()> {
    // A closed output pipe (e.g. `linix search x | head`) makes `println!` fail with
    // EPIPE, which under `panic = "abort"` becomes a core dump ("Aborted"). Intercept
    // that one panic and exit quietly — the wanted output was already delivered. This
    // leaves SIGPIPE ignored for sockets, so network writes are unaffected.
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let is_broken_pipe = info.to_string().contains("Broken pipe")
            || info
                .payload()
                .downcast_ref::<String>()
                .is_some_and(|s| s.contains("Broken pipe"))
            || info
                .payload()
                .downcast_ref::<&str>()
                .is_some_and(|s| s.contains("Broken pipe"));
        if is_broken_pipe {
            std::process::exit(0);
        }
        default_panic_hook(info);
    }));

    // 1. Logging Initialization
    // Logs go to STDERR so that stdout carries only machine-readable payloads. Otherwise
    // `INFO` lines are interleaved with `--json` output on stdout, corrupting it for any
    // consumer (`linix search --json | jq`).
    //
    // The level is read straight off argv rather than off the parsed `Cli`, because this has
    // to be running before the shim hijack a few lines down — and reading it after clap is
    // exactly why `--verbose` used to do nothing at all.
    // A default run prints neither a timestamp nor a module path. `WARN
    // linix::verbs::packages` and an RFC3339 stamp are addressed to whoever is debugging
    // LiNix, and the person reading them typed a package name — the sentence is for them, the
    // provenance is not. Both come back at `-v`, where somebody has asked for the internals.
    let argv: Vec<String> = std::env::args().collect();
    let level = log_level_from_argv(&argv);
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    let verbose = level.contains("debug") || level.contains("trace");
    if verbose {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter())
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(filter())
            .without_time()
            .with_target(false)
            .init();
    }

    // 1.5 Where LiNix's own data lives — before the shim hijack, which builds an `App` and so
    // reads it.
    settle_data_dir(&argv)?;

    // 2. Shim hijack
    if let Some(res) = attempt_shim_hijack().await? {
        return res;
    }

    // 3. CLI & Config Bootstrap
    // Expand user-defined command aliases (config `[command_aliases]`) BEFORE clap parses, so
    // an alias `up` can stand in for `upgrade --all`. Built-in subcommands always win.
    let raw_argv: Vec<String> = std::env::args().collect();
    let prefs = preferences_path_from_argv(&raw_argv)
        .and_then(|p| linix::config::Config::from_file(&p).ok());
    let aliases = prefs
        .as_ref()
        .map(|c| c.command_aliases.clone())
        .unwrap_or_default();
    let verbs = prefs.as_ref().map(|c| c.verbs.clone()).unwrap_or_default();

    // U35: a user-defined verb runs a *sequence* of built-in verbs. It is intercepted here,
    // before clap, because the verb name is not a Cli subcommand — clap would reject it. A verb
    // never shadows a built-in (built-ins always win), and every step must itself be a built-in
    // (composition only; arbitrary argv is U33's key, off by default).
    if !verbs.is_empty() {
        let known = known_subcommands();
        match plan_user_verb(&raw_argv, &verbs, &known) {
            Some(Ok(steps)) => return run_user_verb(steps).await,
            Some(Err(msg)) => {
                eprintln!("{}", msg);
                std::process::exit(linix::core::Exit::Failed.code());
            }
            None => {}
        }
    }

    let cli = if aliases.is_empty() {
        parse_or_exit(Cli::try_parse())
    } else {
        let known = known_subcommands();
        parse_or_exit(Cli::try_parse_from(expand_command_aliases(
            raw_argv, &aliases, &known,
        )))
    };
    // Before the config is read, because loading it can already run an external vars provider
    // — and a breakdown that starts after the first child has nothing to say about it.
    if cli.timings {
        linix::core::timing::enable();
    }

    let mut config = load_and_merge_config(&cli).await?;
    // T4: `watch` runs unattended, so nobody is present to touch a hardware key. Set on the
    // config BEFORE the registry is built, because the link backend takes an `Arc<Config>` at
    // construction and a touch-required `@decrypt` is skipped under this flag rather than
    // hanging the reconcile.
    if matches!(cli.command, Commands::Watch { .. }) {
        config.unattended = true;
    }
    apply_process_wide_config(&config);

    // 4. A reconcile fired by a manager that LiNix itself is driving has nothing to add —
    //    the run that spawned it is already recording what it installed, and it holds the
    //    lock this process would wait two minutes for.
    if matches!(cli.command, Commands::HookReconcile { .. })
        && env::var_os(linix::core::executor::INSIDE_LINIX).is_some()
    {
        return Ok(());
    }

    // 5. One writer at a time. Held for the whole run, released when `main` returns — a
    //    lock dropped before the last write is a lock over part of a set that must agree.
    let _data_lock = acquire_data_lock(&cli.command)?;

    // 6. Kernel Initialization
    let app = App::new(config).await?;

    // 7. Command Dispatcher (Modular A+ Routing)
    //
    // U21: the result is mapped to the exit-code table rather than returned straight, so a
    // guard refusal (3) and a read-only command that found work (2) are distinguishable from
    // a failure (1). `anyhow`'s default would collapse all three into 1.
    // Timed here, around the one dispatch, rather than inside each verb: a budget every verb
    // has to remember to check is a budget the next verb forgets. Nothing measured latency at
    // all before this, which is how a 98-second `info` shipped while `search` answered the same
    // question in seconds (E14).
    let started = std::time::Instant::now();
    let outcome = dispatch(&app, &cli).await;
    linix::core::latency::report_if_over(
        &linix::core::latency::subcommand_name(&cli.command),
        started.elapsed(),
    );
    // Before `finish`, which maps a refusal or a failure onto an exit code and leaves: a run
    // that ended badly is the one whose timing a user most wants to see.
    linix::core::timing::report(linix::core::timing::elapsed());
    finish(&app, outcome).await
}

/// Which of the four published codes a clap outcome is (Q3, II.8, V.92).
///
/// clap's own convention for a usage error is 2, and II.8 spends 2 on *a read-only command
/// looked and found work to do* — so a CI job branching on the documented table read a
/// mistyped subcommand as a drifted machine. A typo has not looked at the machine at all.
/// Asking for help or a version is an answer and stays 0.
fn clap_exit_code(kind: clap::error::ErrorKind) -> i32 {
    use clap::error::ErrorKind;
    match kind {
        // Asked for and answered.
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
        // `linix` with no subcommand. clap prints help as a courtesy and files it next to the
        // real thing, but nobody asked for help and no command ran — a script that reaches
        // here has a bug, and 0 would tell it everything is fine.
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => linix::core::Exit::Failed.code(),
        _ => linix::core::Exit::Failed.code(),
    }
}

/// Hand clap's own message to the user, then leave with LiNix's code rather than clap's.
fn parse_or_exit(parsed: Result<Cli, clap::Error>) -> Cli {
    match parsed {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            std::process::exit(clap_exit_code(e.kind()));
        }
    }
}

/// Turn a command's result into this process's exit code (U21, `core::Exit`).
///
/// A refusal and a difference are printed as themselves — plainly, with no `Error:` prefix —
/// because neither is a malfunction. Only a real failure is reported as one.
pub(crate) async fn finish(app: &App, outcome: Result<()>) -> Result<()> {
    use linix::core::Exit;
    match outcome {
        Ok(()) => Ok(()),
        Err(e) => {
            let code = match e.downcast_ref::<linix::core::Error>() {
                Some(linix::core::Error::Refused(msg)) => {
                    eprintln!("{}", msg);
                    // `on_guard_refusal` (XIII.13) fires here and nowhere else. Fired at this
                    // layer rather than inside the guard because announcing a refusal is a
                    // side effect, and a side effect inside a decision function runs wherever
                    // the decision is evaluated — tests included.
                    //
                    // This arm used to claim it was "the one point every refusal in the
                    // program passes through, so no command can be added that refuses without
                    // the hook hearing about it". That was false for nine sites — every
                    // security refusal, the whole SEC/T series — which returned 1 and were
                    // never announced. The claim is now checked rather than asserted:
                    // `tests/grader_refusal_exit_code_tests.rs` enumerates every site whose
                    // message says it is refusing and fails on one not built as
                    // `Error::Refused`, and fires a real hook through a real refusal. A
                    // sentence that quantifies over paths belongs in a test, not in a comment.
                    linix::app::events::EventHooks::load(&app.config)
                        .fire(
                            linix::model::event::Event::OnGuardRefusal,
                            serde_json::json!({ "message": msg }),
                        )
                        .await;
                    Exit::Refused
                }
                Some(linix::core::Error::Differences(msg)) => {
                    if !msg.is_empty() {
                        eprintln!("{}", msg);
                    }
                    Exit::Differences
                }
                _ => {
                    // R-3's other half. LiNix classifies every failure it can — a rate limit
                    // is `Transient` and says why — and nothing downstream could see the
                    // answer. The sweep harness therefore tested transience by RETRYING THE
                    // INSTALL IMMEDIATELY, which cannot succeed inside a 1236-second rate-limit
                    // window: it scored `defect`, the macOS leg went red, and the real-lifecycle
                    // ratchet fell 8 -> 7 and went red behind it. Two red CI jobs over a
                    // classification the program had already made.
                    //
                    // One stable line, on failure only, on stderr, in a shape a script can read
                    // without grepping an English sentence — the token is pinned by
                    // `tests/failure_class_line_tests.rs` precisely so the wording above it
                    // stays free to change.
                    print_failure_class(&e);
                    return Err(e);
                }
            };
            std::process::exit(code.code());
        }
    }
}

/// The one machine-readable line: what LiNix thinks the failure it is about to report *is*.
///
/// `retryability()` already answers this and only two places consulted it. A caller that has to
/// re-derive the answer by running the command again is not reading the classification, it is
/// guessing at it — and an immediate retry is a guess that is wrong for exactly the failures the
/// classification gets right.
///
/// The vocabulary is `Retryability`'s own, so a variant added there and not handled here is a
/// compile error rather than a silently missing token.
fn print_failure_class(e: &anyhow::Error) {
    use linix::core::Retryability;
    use std::io::IsTerminal;

    // Addressed to a program, so it is written only where a program is listening. On a terminal
    // it was internal vocabulary on the first line of the first command a new user runs:
    //
    //     $ linix sync
    //     linix-failure-class: permanent
    //     Error: Configuration error: no `priority` file at …
    //
    // A pipe is exactly the condition under which both harnesses read it (G-6).
    if std::io::stderr().is_terminal() {
        return;
    }
    let class = match e
        .downcast_ref::<linix::core::Error>()
        .map(|x| x.retryability())
    {
        Some(Retryability::Transient) => "transient",
        Some(Retryability::Permanent) => "permanent",
        Some(Retryability::Exhausted) => "exhausted",
        // No LiNix error at all, or one nothing classified: the same answer either way, and it
        // is the honest one — nobody looked.
        Some(Retryability::Unknown) | None => "unknown",
    };
    eprintln!("linix-failure-class: {class}");
}

pub(crate) async fn dispatch(app: &App, cli: &Cli) -> Result<()> {
    match &cli.command {
        Commands::Sync {
            locked,
            upgrade,
            json,
        } => {
            handle_sync(
                app,
                SyncMode {
                    locked: *locked,
                    upgrade: *upgrade,
                },
                Output::from_json_flag(*json),
            )
            .await
        }
        Commands::Watch {
            interval,
            on_change,
            pull,
            once,
        } => handle_watch(app, *interval, *on_change, *pull, *once).await,
        Commands::Upgrade {
            packages,
            backend,
            all,
            security,
            except,
            profile,
            module,
            json,
            canary,
            test,
        } => {
            handle_upgrade(
                app,
                UpgradeRequest {
                    packages,
                    backend: backend.as_deref(),
                    all: *all,
                    security: *security,
                    except,
                    profile,
                    module,
                    out: Output::from_json_flag(*json),
                    canary: *canary,
                    test,
                },
            )
            .await
        }
        Commands::Install {
            packages,
            json,
            temp,
            into,
        } => handle_install(
            app,
            packages,
            Output::from_json_flag(*json),
            temp.as_deref(),
            into.as_deref(),
        )
        .await,
        Commands::Uninstall {
            packages,
            json,
            temp,
            purge: _,
        } => handle_uninstall(app, packages, Output::from_json_flag(*json), temp.as_ref()).await,
        Commands::Shell { packages } => handle_shell(app, packages).await,
        Commands::Module(args) => handle_module(app, &args.command).await,
        Commands::Schedule(args) => handle_schedule(app, &args.command).await,
        Commands::Snapshot(args) => handle_snapshot(app, &args.command).await,
        Commands::Rollback { reference } => handle_rollback(app, reference).await,
        Commands::Diff { from, to } => handle_diff(app, from, to.as_deref()).await,
        Commands::Eval => handle_eval(app).await,
        Commands::Repl => linix::app::repl::run(app).await.map_err(Into::into),
        Commands::Try { image } => handle_try(app, image.as_deref()).await,
        Commands::Add {
            source,
            trust,
            force,
        } => handle_add(app, source, *trust, *force).await,
        Commands::Git(args) => handle_git(app, &args.command).await,
        Commands::Repo(args) => handle_repo(app, &args.command).await,
        Commands::Search {
            query,
            json,
            installed,
        } => handle_search(app, query, Output::from_json_flag(*json), *installed).await,
        Commands::Teleport { package, backend } => handle_teleport(app, package, backend).await,
        Commands::List {
            backend,
            json,
            outdated,
        } => handle_list(
            app,
            backend.as_deref(),
            Output::from_json_flag(*json),
            *outdated,
        )
        .await,
        Commands::Info { package } => handle_info(app, package).await,
        Commands::RemoveOrphans => handle_remove_orphans(app).await,
        Commands::CleanCache { all } => handle_clean_cache(app, *all).await,
        Commands::Heal => handle_heal(app).await,
        Commands::Adopt {
            backends,
            enabled_only,
        } => handle_adopt(app, backends.clone(), *enabled_only).await,
        Commands::History => handle_history(app).await,
        Commands::Activate { profiles, add } => handle_activate(app, profiles, *add).await,
        Commands::Deactivate { profiles } => handle_deactivate(app, profiles).await,
        Commands::Profile(args) => handle_profile(app, &args.command).await,
        Commands::Run {
            packages,
            command,
            args,
        } => handle_run(app, packages, command, args).await,
        Commands::Lock { axis, names, list } => handle_lock(app, *axis, names, *list).await,
        Commands::Unlock { axis, names, list } => handle_unlock(app, *axis, names, *list).await,
        Commands::Plan { out } => handle_plan(app, out).await,
        Commands::Apply { plan, yes } => handle_apply(app, plan, *yes).await,
        Commands::Update => handle_update(app).await,
        Commands::Reset { force } => handle_reset(app, *force).await,
        Commands::Check { section, json } => {
            handle_check(app, section.as_deref(), Output::from_json_flag(*json)).await
        }
        Commands::Vars => handle_vars(app).await,
        Commands::PurgeUndeclared { allow_mass_purge } => {
            handle_purge_undeclared(app, *allow_mass_purge).await
        }
        Commands::Protected { packages, json } => {
            handle_protected(app, packages, Output::from_json_flag(*json)).await
        }
        Commands::Unmanage { packages, json } => {
            handle_unmanage(app, packages, Output::from_json_flag(*json)).await
        }
        Commands::Rebuild {
            packages,
            backend,
            all,
        } => handle_rebuild(app, packages, backend.as_deref(), *all).await,
        Commands::Config(args) => handle_config(app, &args.command).await,
        Commands::Path { explain, set } => handle_path(cli, *explain, set.as_deref()).await,
        Commands::Edit { file } => handle_edit(cli, file.as_deref()).await,
        Commands::Init { force, interactive } => handle_init(app, *force, *interactive).await,
        Commands::Sbom => handle_sbom(app).await,
        Commands::Export {
            format,
            out,
            stdout,
            force,
        } => handle_export(app, format.as_deref(), out, *stdout, *force).await,
        Commands::Bundle {
            out,
            artifacts,
            archive,
        } => handle_bundle(app, out, *artifacts, *archive).await,
        Commands::Restore { dir, force } => handle_restore(app, dir, *force).await,
        Commands::Why { package, json } => {
            handle_why(app, package, Output::from_json_flag(*json)).await
        }
        Commands::Service(args) => handle_service(app, &args.command).await,
        Commands::Bisect { test, yes } => linix::app::bisect::bisect(app, test, *yes)
            .await
            .map_err(|e| e.into()),
        Commands::Fleet(args) => linix::app::fleet::fleet(app, &args.hosts, args.sync, args.apply)
            .await
            .map_err(|e| e.into()),
        Commands::Hooks(args) => handle_hooks(app, &args.command).await,
        Commands::HookRecord {
            manager,
            op,
            targets,
        } => handle_hook_record(app, manager, op, targets).await,
        Commands::HookReconcile { manager } => handle_hook_reconcile(app, manager).await,
        Commands::HookObserve {
            manager,
            learn,
            argv,
        } => handle_hook_observe(app, manager.as_deref(), *learn, argv).await,
        Commands::Hold { packages } => handle_hold(app, packages).await,
        Commands::Unhold { packages } => handle_unhold(app, packages).await,
        Commands::Policy => handle_policy(app).await,
        Commands::Completions { shell } => {
            let mut cmd = <Cli as clap::CommandFactory>::command();
            linix::cli::generate_completions(*shell, &mut cmd);
            Ok(())
        }
        Commands::SelfUpgrade { git, check } => handle_self_upgrade(git.as_deref(), *check).await,
    }
}

/// Repository a `self-upgrade` installs from: explicit `--git`, else `$LINIX_REPO`, else the
/// upstream default (kept in sync with `scripts/install.sh`).
pub(crate) fn self_upgrade_repo(git: Option<&str>) -> String {
    git.map(|s| s.to_string())
        .or_else(|| std::env::var("LINIX_REPO").ok())
        .unwrap_or_else(|| "https://github.com/SYKhayyat/LiNix".to_string())
}

pub(crate) async fn cargo_install_from(
    repo: &str,
    locked: bool,
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("install").arg("--git").arg(repo).arg("--force");
    if locked {
        cmd.arg("--locked");
    }
    cmd.status().await
}

pub(crate) async fn handle_self_upgrade(git: Option<&str>, check: bool) -> Result<()> {
    let repo = self_upgrade_repo(git);
    println!("Current version : linix {}", linix::VERSION);
    if check {
        println!("Upgrade source  : {}", repo);
        println!("Run `linix self-upgrade` to rebuild and install the latest from source.");
        return Ok(());
    }
    if which::which("cargo").is_err() {
        anyhow::bail!(
            "`cargo` (the Rust toolchain) is required to self-upgrade. Install it from \
             https://rustup.rs, or re-run the LiNix install script."
        );
    }
    println!("Rebuilding linix from {repo} via cargo — this can take a few minutes...");
    // Reproducible build first (--locked); fall back to a loose build, exactly like install.sh.
    let first = cargo_install_from(&repo, true).await;
    let ok = matches!(&first, Ok(s) if s.success());
    if !ok {
        warn!("locked build failed; retrying without --locked...");
        let second = cargo_install_from(&repo, false)
            .await
            .context("running `cargo install`")?;
        if !second.success() {
            anyhow::bail!("cargo install failed; LiNix was not upgraded.");
        }
    }
    println!("Done. Run `linix --version` to confirm the new build.");
    Ok(())
}

/// The value of a `--flag VALUE` / `--flag=VALUE` in raw argv.
///
/// Command aliases are expanded before clap runs, so this pre-parse cannot ask clap where the
/// repo is. It peeks at the flags and hands them to the same resolver the app uses — peeking
/// is unavoidable here, resolving a second time is not.
pub(crate) fn flag_from_argv(argv: &[String], names: &[&str]) -> Option<String> {
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if names.contains(&a.as_str()) {
            return it.next().cloned();
        }
        for n in names {
            if let Some(rest) = a.strip_prefix(&format!("{}=", n)) {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// `--data-dir`, and the two environment variables, settled once before anything reads them.
///
/// **The flag sets the variable rather than becoming a second answer.** Six places ask "where is
/// LiNix's data" — `safe_data_dir`, `Layout::from_env`, `StateRegistry::load_default`, the
/// config default, the rehearsal sandbox, the test fixtures — and every one of them reads
/// `$LINIX_DATA_DIR`. A flag threaded through as a separate value would have to reach all six,
/// and the one it missed would be the one that wrote to the developer's real registry. Config
/// got a first-class flag and state got an undocumented variable, and that asymmetry is what
/// turned `--config-dir` from a testing affordance into a trap (AU4): a fresh sandbox planned
/// seven removals against the real machine's managed state.
///
/// Read from argv rather than from the parsed `Cli` because the shim hijack builds an `App`
/// before clap runs — the same reason the log level is read here.
///
/// Both variables are checked for absoluteness at this one point, because the readers above
/// return a `PathBuf` and cannot refuse anything (AU2).
fn settle_data_dir(argv: &[String]) -> Result<()> {
    use linix::config::settings::absolute_or_refuse;

    if let Some(flag) = flag_from_argv(argv, &["--data-dir"]) {
        let dir = absolute_or_refuse(std::path::PathBuf::from(flag), "`--data-dir`")?;
        std::env::set_var("LINIX_DATA_DIR", dir);
    } else if let Some(dir) = std::env::var_os("LINIX_DATA_DIR").filter(|v| !v.is_empty()) {
        absolute_or_refuse(std::path::PathBuf::from(dir), "`$LINIX_DATA_DIR`")?;
    }
    if let Some(dir) = std::env::var_os("LINIX_CONFIG_DIR").filter(|v| !v.is_empty()) {
        absolute_or_refuse(std::path::PathBuf::from(dir), "`$LINIX_CONFIG_DIR`")?;
    }
    Ok(())
}

/// Where `preferences.toml` is, for the pre-clap alias load.
///
/// `--config` names the file; otherwise `locate` answers with `--config-dir`,
/// `$LINIX_CONFIG_DIR`, the settings file, then the default — the one resolution, so the
/// aliases come out of the file the rest of the run will read (X.6).
/// How much LiNix says about itself, from argv alone.
///
/// The default is `warn`, not `info`: an ordinary run's answer goes to stdout, and what was
/// left on the `info` channel was LiNix narrating its own startup over the top of it. The
/// narration is still there for anyone who asks — that is what `-v` is for, and asking is the
/// difference. `RUST_LOG` outranks all of this; it is checked before this is called.
///
/// `--quiet` reaches further than `-v` in the other direction and wins when both are given: a
/// run that says "be quiet" and "be loud" meant the quiet half, or it would not have typed it.
fn log_level_from_argv(argv: &[String]) -> &'static str {
    let mut verbosity = 0u8;
    for arg in argv.iter().skip(1) {
        match arg.as_str() {
            "--quiet" => return "error",
            "--verbose" => verbosity += 1,
            // A bundled short run (`-nv`, `-qv`): every flag in it is a letter here. `--`
            // ends the flags, and anything after it belongs to the command.
            "--" => break,
            _ if arg.starts_with('-') && !arg.starts_with("--") => {
                if arg.contains('q') {
                    return "error";
                }
                verbosity += arg.matches('v').count() as u8;
            }
            _ => {}
        }
    }
    match verbosity {
        0 => "warn",
        1 => "info",
        _ => "debug",
    }
}

pub(crate) fn preferences_path_from_argv(argv: &[String]) -> Option<std::path::PathBuf> {
    if let Some(p) = flag_from_argv(argv, &["-c", "--config"]) {
        return Some(std::path::PathBuf::from(p));
    }
    let dir = flag_from_argv(argv, &["--config-dir"]).map(std::path::PathBuf::from);
    linix::app::locate::locate(dir.as_deref())
        .ok()
        .map(|r| r.path.join(linix::config::PREFERENCES_FILE_NAME))
}

/// Take the lock for a mutating command, asking the command itself.
///
/// It used to be read from argv and matched against a hand-written list of twenty-one names,
/// on the reasoning that a subcommand added later would then be locked by default rather than
/// forgotten by a match arm. The list was the thing that rotted — twelve of its entries once
/// named commands the program did not have, `history` was on it while reaching the whole
/// install path, and `fleet` was off it while touching nothing local. `Commands::writes` is
/// exhaustive, so a subcommand added later does not compile until it answers, which is the
/// property the argv read was reaching for and could not have.
pub(crate) fn acquire_data_lock(
    command: &Commands,
) -> Result<Option<linix::core::datalock::DataLock>> {
    if !command.writes() {
        return Ok(None);
    }
    let name = linix::core::latency::subcommand_name(command);
    // 120s: long enough to outlast the longest wait a holder can legitimately make before it
    // starts doing work — the rate-limit ceiling, 30s by default — with room for the install
    // it then performs. It is not meant to outlast a whole sync: past this point the honest
    // answer is that someone else is writing, not a longer silence (S27).
    let lock = linix::core::datalock::DataLock::acquire(
        &linix::utils::safe_data_dir(),
        &name,
        std::time::Duration::from_secs(120),
    )?;
    Ok(Some(lock))
}

pub(crate) fn known_subcommands() -> std::collections::HashSet<String> {
    <Cli as clap::CommandFactory>::command()
        .get_subcommands()
        .flat_map(|s| {
            std::iter::once(s.get_name().to_string())
                .chain(s.get_all_aliases().map(|a| a.to_string()))
        })
        .collect()
}

/// Global flags that take a separate-argument value (`-c path`), asked of clap rather than
/// hand-listed. A hand-written list is a second copy of a fact clap already owns, and it
/// silently rotted: it named `-b`/`-g` after both were deleted, and `--progress`, which is
/// a `bool` and consumes nothing — so `--progress` in front of an alias swallowed the alias
/// name, and it never expanded.
pub(crate) fn global_value_flags() -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for a in <Cli as clap::CommandFactory>::command().get_arguments() {
        if !matches!(
            a.get_action(),
            clap::ArgAction::Set | clap::ArgAction::Append
        ) {
            continue;
        }
        if let Some(l) = a.get_long() {
            out.insert(format!("--{}", l));
        }
        if let Some(c) = a.get_short() {
            out.insert(format!("-{}", c));
        }
    }
    out
}

/// Index of the subcommand token in argv, skipping the program name, leading global flags, and
/// any values those flags consume. `None` if there is no subcommand (e.g. only `--version`).
pub(crate) fn find_subcommand_index(argv: &[String]) -> Option<usize> {
    let value_flags = global_value_flags();
    let mut i = 1;
    while i < argv.len() {
        let a = &argv[i];
        if a == "--" {
            return if i + 1 < argv.len() {
                Some(i + 1)
            } else {
                None
            };
        }
        if a.starts_with('-') {
            // `--flag=value` is one token; `-c value` consumes the next token too.
            if value_flags.contains(a.as_str()) {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            return Some(i);
        }
    }
    None
}

/// Rewrite argv, expanding a user command-alias in the subcommand slot into its full token
/// list. Pure and unit-tested. The slot is located past any leading global flags; a name that
/// matches a built-in subcommand is left untouched (built-ins always win).
pub(crate) fn expand_command_aliases(
    argv: Vec<String>,
    aliases: &HashMap<String, String>,
    known: &std::collections::HashSet<String>,
) -> Vec<String> {
    let Some(idx) = find_subcommand_index(&argv) else {
        return argv;
    };
    let cmd = &argv[idx];
    if known.contains(cmd) {
        return argv;
    }
    if let Some(expansion) = aliases.get(cmd) {
        let mut out = Vec::with_capacity(argv.len() + 2);
        out.extend(argv[..idx].iter().cloned());
        out.extend(expansion.split_whitespace().map(|s| s.to_string()));
        out.extend(argv[idx + 1..].iter().cloned());
        return out;
    }
    argv
}

/// Plan a user-defined verb (U35) into the per-step argv it runs, or `None` when the invocation
/// is not a verb (a built-in, an alias, or `--version` with no subcommand).
///
/// Pure and unit-tested. Each step inherits the leading global flags (`-c path`) so config
/// selection is the same for every step, and gains no trailing arguments — a verb is a fixed
/// composition, and threading `linix update --dry-run` into some steps and not others is the
/// kind of surprise the closed vocabulary exists to avoid. **Composition only:** a step whose
/// first token is not a built-in subcommand is an error, because a verb that runs arbitrary argv
/// is `exec:` wearing a command's clothes (U33, off by default).
pub(crate) fn plan_user_verb(
    argv: &[String],
    verbs: &HashMap<String, Vec<String>>,
    known: &std::collections::HashSet<String>,
) -> Option<std::result::Result<Vec<Vec<String>>, String>> {
    let idx = find_subcommand_index(argv)?;
    let cmd = &argv[idx];
    // Built-ins always win, so a verb can never mask a real command.
    if known.contains(cmd) {
        return None;
    }
    let steps = verbs.get(cmd)?;

    // A verb takes no arguments of its own: it is a fixed sequence. Anything after the name is
    // refused loudly rather than silently dropped or smeared across every step.
    if argv.len() > idx + 1 {
        return Some(Err(format!(
            "the verb `{}` takes no arguments, but `{}` was given.\n  \
             A verb is a fixed sequence of built-in commands (U35). To vary a step, edit the \
             `[verbs]` entry.",
            cmd,
            argv[idx + 1..].join(" ")
        )));
    }

    let leading = &argv[1..idx];
    let mut planned = Vec::with_capacity(steps.len());
    for step in steps {
        let tokens: Vec<String> = step.split_whitespace().map(|s| s.to_string()).collect();
        let Some(first) = tokens.first() else {
            return Some(Err(format!(
                "the verb `{}` has an empty step. Every step must be a built-in command.",
                cmd
            )));
        };
        if !known.contains(first) {
            return Some(Err(format!(
                "the verb `{}` step `{}` is not a built-in command.\n  \
                 A user verb may only compose built-in commands (U35). Running an arbitrary \
                 command from a verb is `exec:`'s job and is off by default (U33).",
                cmd, first
            )));
        }
        let mut one = Vec::with_capacity(1 + leading.len() + tokens.len());
        one.push(argv[0].clone());
        one.extend(leading.iter().cloned());
        one.extend(tokens);
        planned.push(one);
    }
    Some(Ok(planned))
}

/// Seed the settings that live in process-wide cells rather than in `App`.
///
/// One function because there are two entry points that load a config, and a setting wired
/// into one of them is a setting that does nothing under `run_user_verb`.
fn apply_process_wide_config(config: &linix::config::Config) {
    linix::backends::node_registry::set_http_timeout(config.network_timeout_secs);
    linix::core::executor::set_command_idle_timeout(config.command_idle_timeout_secs);
    linix::core::executor::set_query_bounds(
        config.query_idle_timeout_secs,
        config.read_retry_attempts,
    );
}

/// Run a user verb: build the config and app once from the shared leading flags, then dispatch
/// each step against them in order, stopping at the first failure.
///
/// **One data lock covers the whole verb, and it is taken when ANY step writes.** The verb name
/// is not a subcommand, so this used to lock unconditionally as the safe default for a sequence
/// that may install or remove; now that each step parses to a `Commands`, the sequence can be
/// asked instead. A verb of five readers stops holding the writer lock, and a verb whose third
/// step syncs takes it before the first step runs rather than partway through.
pub(crate) async fn run_user_verb(steps: Vec<Vec<String>>) -> Result<()> {
    let parsed: Vec<Cli> = steps.iter().map(Cli::parse_from).collect();
    let config = load_and_merge_config(&parsed[0]).await?;
    apply_process_wide_config(&config);
    // The lock spans the whole verb, so the question is whether any step writes — not whether
    // the first one does. Taking it per step would release it between two commands that have
    // to agree about the same registry.
    let _data_lock = match parsed.iter().find(|c| c.command.writes()) {
        Some(writer) => acquire_data_lock(&writer.command)?,
        None => None,
    };
    let app = App::new(config).await?;
    for step in &steps {
        let cli = Cli::parse_from(step);
        let outcome = dispatch(&app, &cli).await;
        if outcome.is_err() {
            return finish(&app, outcome).await;
        }
    }
    finish(&app, Ok(())).await
}

// ============================================================================
// KERNEL HELPERS
// ============================================================================

pub(crate) async fn attempt_shim_hijack() -> Result<Option<Result<()>>> {
    let current_name = env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "linix".to_string());
    if current_name != "linix" && !current_name.starts_with("linix") {
        let root = linix::app::locate::locate(None)?.path;
        let config =
            linix::config::Config::from_file(&root.join(linix::config::PREFERENCES_FILE_NAME))
                .unwrap_or_default();
        let app = App::new(config).await?;
        return Ok(Some(
            app.runner()
                .exec_shim(&current_name, &env::args().collect::<Vec<_>>()[1..])
                .await
                .map_err(|e| e.into()),
        ));
    }
    Ok(None)
}

pub(crate) async fn load_and_merge_config(cli: &Cli) -> Result<linix::config::Config> {
    // Where the repo is: --config-dir, then $LINIX_CONFIG_DIR, then LiNix's settings file,
    // then the default. This has to resolve BEFORE `preferences.toml` is opened, because
    // that file lives inside the root it would otherwise have to announce.
    let located = linix::app::locate::locate(cli.config_dir.as_deref())?;
    let path = cli
        .config
        .clone()
        .unwrap_or_else(|| located.path.join(linix::config::PREFERENCES_FILE_NAME));
    let mut config =
        tokio::task::spawn_blocking(move || linix::config::Config::from_file(&path)).await??;
    config.config_root = located.path;
    config.merge_cli_overrides(linix::config::CliOverrides {
        dry_run: cli.dry_run,
        yes: cli.yes,
        verbose: cli.verbose > 0,
        allow_mass_removal: cli.allow_mass_removal,
        allow_mass_install: cli.allow_mass_install,
        config_path: None,
    });
    // The one place `--dry-run` becomes a property of the process. Set after the config merge
    // so a `dry_run = true` in `preferences.toml` counts too, and before dispatch so no write
    // can run ahead of it. Every config write consults this instead of each verb remembering
    // to — which five verbs did not (`activate`, `deactivate`, `lock`, `git init`,
    // `config init`), and `--dry-run activate Work` left you on Work without printing a line.
    linix::core::dry_run::set(config.dry_run);

    // A per-run acknowledgement, never a config key (U23): a machine that always bypasses the
    // dotfiles collision check is a machine where the check does not exist.
    if cli.replace_existing {
        config.replace_existing = true;
    }
    // --quiet has no config-file merge counterpart; apply it directly (a set flag wins).
    if cli.quiet {
        config.quiet = true;
    }
    // `--no-cache` is the whole off-switch: the TTL is what the disk layer is built from, so
    // zeroing it here means nothing downstream has a second way to be on. The same zero is
    // what keeps a cached listing out of every command that writes its answer down
    // (`cache_may_answer`) — the setting says how long a reading may be reused, never that a
    // plan or an adoption may be built on one.
    if cli.no_cache
        || !linix::core::installed::InstalledListings::cache_may_answer(
            &linix::core::latency::subcommand_name(&cli.command),
        )
    {
        config.installed_cache_secs = 0;
    }
    // `uninstall --purge` is a `[remove] purge` for this run only. Read here, where CLI flags
    // become config, because `config` is shared read-only by the time a command runs.
    if let Commands::Uninstall { purge: true, .. } = cli.command {
        config.purge_this_run = true;
    }
    // `--keep-going` is per-run by construction: there is no file key to read it from.
    if cli.keep_going {
        config.keep_going_this_run = true;
    }
    // --no-progress is the real off-switch for the progress indicators (S5). A set flag wins
    // over the `show_progress` config default.
    if cli.no_progress {
        config.show_progress = false;
    }
    Ok(config)
}

#[cfg(test)]
mod alias_tests {
    use super::*;
    use std::collections::HashSet;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn expands_a_defined_alias_into_tokens() {
        let mut aliases = HashMap::new();
        aliases.insert("up".to_string(), "upgrade --all".to_string());
        let known: HashSet<String> = ["upgrade".to_string()].into_iter().collect();

        let out = expand_command_aliases(argv(&["linix", "up", "--dry-run"]), &aliases, &known);
        assert_eq!(out, argv(&["linix", "upgrade", "--all", "--dry-run"]));
    }

    #[test]
    fn expands_alias_after_a_value_taking_global_flag() {
        let mut aliases = HashMap::new();
        aliases.insert("up".to_string(), "upgrade --all".to_string());
        let known: HashSet<String> = ["upgrade".to_string()].into_iter().collect();

        let out = expand_command_aliases(argv(&["linix", "-c", "/c.toml", "up"]), &aliases, &known);
        assert_eq!(out, argv(&["linix", "-c", "/c.toml", "upgrade", "--all"]));
    }

    #[test]
    fn expands_alias_after_a_valueless_global_flag() {
        // `--progress` is a bool: clap gives it SetTrue, so it consumes no value. The old
        // hand-written flag list claimed it took one, so `i += 2` walked past `up` and the
        // alias silently never expanded.
        let mut aliases = HashMap::new();
        aliases.insert("up".to_string(), "upgrade --all".to_string());
        let known: HashSet<String> = ["upgrade".to_string()].into_iter().collect();

        let out = expand_command_aliases(argv(&["linix", "--progress", "up"]), &aliases, &known);
        assert_eq!(out, argv(&["linix", "--progress", "upgrade", "--all"]));
    }

    #[test]
    fn value_flags_are_exactly_what_clap_says_take_a_value() {
        let flags = global_value_flags();
        assert!(flags.contains("--config") && flags.contains("-c"));
        // Every bool global: named here, they would each eat the following token.
        for valueless in ["--progress", "--dry-run", "-y", "--yes", "-v", "-q"] {
            assert!(
                !flags.contains(valueless),
                "{} takes no value; listing it skips a real token",
                valueless
            );
        }
        // Deleted flags cannot linger: the list is derived, not maintained.
        for gone in ["-g", "--groups-dir", "-b", "--backend", "--no-global"] {
            assert!(!flags.contains(gone), "{} was deleted", gone);
        }
    }

    #[test]
    fn subcommand_index_skips_flags_and_their_values() {
        assert_eq!(find_subcommand_index(&argv(&["linix", "up"])), Some(1));
        assert_eq!(
            find_subcommand_index(&argv(&["linix", "-c", "x", "up"])),
            Some(3)
        );
        assert_eq!(
            find_subcommand_index(&argv(&["linix", "--dry-run", "up"])),
            Some(2)
        );
        assert_eq!(find_subcommand_index(&argv(&["linix", "--version"])), None);
    }

    #[test]
    fn builtin_subcommand_is_never_shadowed() {
        let mut aliases = HashMap::new();
        aliases.insert("upgrade".to_string(), "install evil".to_string());
        let known: HashSet<String> = ["upgrade".to_string()].into_iter().collect();
        // `upgrade` is a real command → alias ignored.
        let out = expand_command_aliases(argv(&["linix", "upgrade"]), &aliases, &known);
        assert_eq!(out, argv(&["linix", "upgrade"]));
    }

    #[test]
    fn leaves_unknown_and_flag_first_tokens_alone() {
        let aliases = HashMap::new();
        let known = HashSet::new();
        assert_eq!(
            expand_command_aliases(argv(&["linix", "--version"]), &aliases, &known),
            argv(&["linix", "--version"])
        );
        assert_eq!(
            expand_command_aliases(argv(&["linix", "notanalias"]), &aliases, &known),
            argv(&["linix", "notanalias"])
        );
    }

    fn verbs(pairs: &[(&str, &[&str])]) -> HashMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(k, steps)| (k.to_string(), steps.iter().map(|s| s.to_string()).collect()))
            .collect()
    }

    fn builtins() -> HashSet<String> {
        ["sync", "upgrade", "check"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    #[test]
    fn a_verb_expands_to_one_argv_per_step() {
        let v = verbs(&[("refresh", &["sync", "upgrade --all"])]);
        let steps = plan_user_verb(&argv(&["linix", "refresh"]), &v, &builtins())
            .unwrap()
            .unwrap();
        assert_eq!(
            steps,
            vec![
                argv(&["linix", "sync"]),
                argv(&["linix", "upgrade", "--all"]),
            ]
        );
    }

    #[test]
    fn a_verb_inherits_leading_global_flags_on_every_step() {
        let v = verbs(&[("refresh", &["sync", "check"])]);
        let steps = plan_user_verb(
            &argv(&["linix", "-c", "/c.toml", "refresh"]),
            &v,
            &builtins(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            steps,
            vec![
                argv(&["linix", "-c", "/c.toml", "sync"]),
                argv(&["linix", "-c", "/c.toml", "check"]),
            ]
        );
    }

    #[test]
    fn a_verb_never_shadows_a_builtin() {
        let v = verbs(&[("sync", &["upgrade"])]);
        // `sync` is a real command, so the verb is invisible and normal parsing proceeds.
        assert!(plan_user_verb(&argv(&["linix", "sync"]), &v, &builtins()).is_none());
    }

    #[test]
    fn a_verb_step_must_be_a_builtin() {
        let v = verbs(&[("evil", &["rm -rf /"])]);
        let err = plan_user_verb(&argv(&["linix", "evil"]), &v, &builtins())
            .unwrap()
            .unwrap_err();
        assert!(err.contains("not a built-in"), "{}", err);
        assert!(err.contains("exec:"), "{}", err);
    }

    #[test]
    fn a_verb_takes_no_arguments() {
        let v = verbs(&[("refresh", &["sync"])]);
        let err = plan_user_verb(&argv(&["linix", "refresh", "--dry-run"]), &v, &builtins())
            .unwrap()
            .unwrap_err();
        assert!(err.contains("takes no arguments"), "{}", err);
    }

    #[test]
    fn a_name_that_is_neither_builtin_nor_verb_is_left_alone() {
        let v = verbs(&[("refresh", &["sync"])]);
        assert!(plan_user_verb(&argv(&["linix", "whatever"]), &v, &builtins()).is_none());
    }
}

#[cfg(test)]
mod log_level_tests {
    use super::{known_subcommands, log_level_from_argv};

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// The ruling: an ordinary run prints its answer and nothing else.
    #[test]
    fn an_ordinary_run_says_nothing_about_itself() {
        assert_eq!(log_level_from_argv(&argv(&["linix", "list"])), "warn");
        assert_eq!(log_level_from_argv(&argv(&["linix", "sync"])), "warn");
    }

    /// The defect this replaced: `--verbose` promised debug logging and delivered none,
    /// because the level was read after clap had parsed and the subscriber was already built.
    #[test]
    fn asking_for_more_gets_more_in_both_spellings() {
        for one in [&["linix", "-v", "list"], &["linix", "--verbose", "list"]] {
            assert_eq!(log_level_from_argv(&argv(one)), "info");
        }
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-vv", "list"])),
            "debug"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-v", "-v", "list"])),
            "debug"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "--verbose", "--verbose", "list"])),
            "debug"
        );
        // Past two there is nothing more to say, and it must not fall back to the default.
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-vvvv", "list"])),
            "debug"
        );
    }

    /// A short flag can arrive bundled with its neighbours, and every letter in the bundle
    /// counts — `-nv` is a dry run that talks.
    #[test]
    fn bundled_short_flags_are_read_letter_by_letter() {
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-nv", "sync"])),
            "info"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-nvv", "sync"])),
            "debug"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-nq", "sync"])),
            "error"
        );
    }

    #[test]
    fn quiet_wins_over_loud_whichever_order_they_come_in() {
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-q", "list"])),
            "error"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-q", "-vv", "list"])),
            "error"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-vv", "-q", "list"])),
            "error"
        );
    }

    /// Everything after `--` is the command's, not LiNix's. A script named `-v` does not
    /// turn logging on, and `linix run -- mytool -q` does not silence LiNix.
    #[test]
    fn flags_stop_at_the_double_dash() {
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "run", "--", "mytool", "-v"])),
            "warn"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "run", "--", "mytool", "-q"])),
            "warn"
        );
    }

    /// A long flag that merely contains the letters must not be read as one: `--yes` has no
    /// `v`, but `--dry-run` and `--verbose-something` are the shapes that catch a naive scan.
    #[test]
    fn a_long_flag_is_never_read_letter_by_letter() {
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "--dry-run", "--yes", "sync"])),
            "warn"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "--allow-mass-removal", "sync"])),
            "warn"
        );
    }

    /// argv[0] is a path, and on this developer's machine it contains a `v` (`Videos`) and on
    /// plenty of others a `q`. It is never a flag.
    #[test]
    fn the_program_path_is_not_a_flag() {
        assert_eq!(
            log_level_from_argv(&argv(&["/home/q/Videos/linix", "list"])),
            "warn"
        );
    }

    /// The lock classification, asked of the enum and of clap — the two things that cannot
    /// drift from each other.
    ///
    /// **The `undo` disease, found in the lock list this replaces.** Twelve of its thirty-three
    /// entries named commands the program does not have — `status` (now `check drift`),
    /// `doctor`, `unmanaged`, `absent`, `insight`, `show`, `audit`, `outdated`, `log`, `locate`,
    /// `metrics`, `verify`. Two tests guarded it and **both guarded invention**: that every name
    /// on the list was real. Nothing guarded omission or misclassification, which is the half
    /// that costs an entry out of `registry.json` — and both were live. `history` was exempt
    /// while reaching `handle_rollback` → `handle_sync`, the entire install/remove path, and
    /// `fleet` was absent from the list while touching no local state at all.
    #[test]
    fn the_readers_are_exactly_the_commands_that_read() {
        // The reader set, read out of `Commands::writes` itself rather than restated. A variant
        // moving between the arms shows up here as a diff, which is what the old list could
        // never do: it lived seventy lines from the enum and nothing compared them.
        let src = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/args.rs"),
        )
        .expect("args.rs");
        let body = src
            .split_once("pub fn writes(&self) -> bool {")
            .expect("`Commands::writes` is gone — this test guards nothing")
            .1;
        // Up to the LAST `=> false,`: everything after it is the writer arm, whose variant
        // names are spelled the same way and would otherwise be collected as readers.
        let cut = body
            .rfind("=> false,")
            .expect("no reader arm — every command would take the exclusive lock");
        let body = &body[..cut];

        let readers: std::collections::BTreeSet<String> = body
            .split("Self::")
            .skip(1)
            .filter_map(|c| c.split([' ', '{']).next())
            .filter(|c| !c.is_empty())
            .map(|c| c.to_string())
            .collect();

        let expected: std::collections::BTreeSet<String> = [
            "Check", "Completions", "Config", "Diff", "Edit", "Eval", "Export", "Fleet",
            "History", "Info", "List", "Path", "Plan", "Policy", "Protected", "Repl", "Sbom",
            "Search", "Try", "Vars", "Why",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        assert_eq!(
            readers, expected,
            "the set of commands exempted from the data lock changed. Adding a WRITER is free;              adding a reader means claiming it never writes under `data/`, so it has to be              claimed here too. Not locking a writer costs an entry out of `registry.json`,              which is a removal."
        );

        // Invention, the half the old tests did cover: every exempted name is a real command.
        let known = known_subcommands();
        let ghosts: Vec<&String> = readers
            .iter()
            .filter(|name| !known.contains(&to_kebab(name)))
            .collect();
        assert!(
            ghosts.is_empty(),
            "these are exempt from the data lock and are not commands: {ghosts:?}"
        );
    }

    fn to_kebab(variant: &str) -> String {
        let mut out = String::new();
        for (i, c) in variant.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('-');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The direction that matters for correctness, driven through clap rather than asserted
    /// about a list of strings: a command LiNix cannot run without writing takes the lock.
    #[test]
    fn the_commands_that_write_take_the_lock() {
        use clap::Parser;
        use linix::cli::args::Cli;
        for argv in [
            vec!["linix", "sync"],
            vec!["linix", "install", "apt:jq"],
            vec!["linix", "uninstall", "apt:jq"],
            vec!["linix", "adopt"],
            vec!["linix", "heal"],
            vec!["linix", "rollback", "HEAD"],
            vec!["linix", "init"],
            vec!["linix", "purge-undeclared"],
            vec!["linix", "remove-orphans"],
            vec!["linix", "rebuild"],
            vec!["linix", "apply", "linix-plan.json"],
            vec!["linix", "self-upgrade"],
        ] {
            let cli = Cli::parse_from(&argv);
            assert!(
                cli.command.writes(),
                "`{}` writes state and must take the data lock",
                argv[1]
            );
        }

        for argv in [
            vec!["linix", "plan"],
            vec!["linix", "list"],
            vec!["linix", "why", "apt:jq"],
            // The two the old list got wrong, in opposite directions.
            vec!["linix", "history"],
            vec!["linix", "fleet"],
        ] {
            let cli = Cli::parse_from(&argv);
            assert!(
                !cli.command.writes(),
                "`{}` only reads and must not hold the 120-second exclusive lock",
                argv[1]
            );
        }
    }
}
