use anyhow::{Context, Result};
use clap::Parser;
use linix::app::App;
use linix::cli::{Cli, Commands};
use std::collections::HashMap;
use std::env;
use tracing::warn;
use tracing_subscriber::EnvFilter;

mod verbs;

use verbs::prelude::*;

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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(log_level_from_argv(&std::env::args().collect::<Vec<_>>()))),
        )
        .init();

    // 2. Shim hijack
    if let Some(res) = attempt_shim_hijack().await? {
        return res;
    }

    // 3. CLI & Config Bootstrap
    // Expand user-defined command aliases (config `[command_aliases]`) BEFORE clap parses, so
    // `linix up` can stand in for `linix upgrade --all`. Built-in subcommands always win.
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
        Cli::parse()
    } else {
        let known = known_subcommands();
        Cli::parse_from(expand_command_aliases(raw_argv, &aliases, &known))
    };
    let mut config = load_and_merge_config(&cli).await?;
    // T4: `watch` runs unattended, so nobody is present to touch a hardware key. Set on the
    // config BEFORE the registry is built, because the link backend takes an `Arc<Config>` at
    // construction and a touch-required `@decrypt` is skipped under this flag rather than
    // hanging the reconcile.
    if matches!(cli.command, Commands::Watch { .. }) {
        config.unattended = true;
    }
    linix::backends::node_registry::set_http_timeout(config.network_timeout_secs);

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
    let _data_lock = acquire_data_lock()?;

    // 6. Kernel Initialization
    let app = App::new(config).await?;

    // 7. Command Dispatcher (Modular A+ Routing)
    //
    // U21: the result is mapped to the exit-code table rather than returned straight, so a
    // guard refusal (3) and a read-only command that found work (2) are distinguishable from
    // a failure (1). `anyhow`'s default would collapse all three into 1.
    let outcome = dispatch(&app, &cli).await;
    finish(&app, outcome).await
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
                    // `on_guard_refusal` (XIII.13) fires here and nowhere else: this is the
                    // one point every refusal in the program passes through, so no command
                    // can be added that refuses without the hook hearing about it. Fired at
                    // this layer rather than inside the guard because announcing a refusal is
                    // a side effect, and a side effect inside a decision function runs
                    // wherever the decision is evaluated — tests included.
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
                _ => return Err(e),
            };
            std::process::exit(code.code());
        }
    }
}

pub(crate) async fn dispatch(app: &App, cli: &Cli) -> Result<()> {
    match &cli.command {
        Commands::Sync {
            locked,
            upgrade,
            json,
        } => handle_sync(app, *locked, *upgrade, *json).await,
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
                    json: *json,
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
        } => handle_install(app, packages, *json, temp.as_deref(), into.as_deref()).await,
        Commands::Uninstall {
            packages,
            json,
            temp,
            purge: _,
        } => handle_uninstall(app, packages, *json, temp.as_ref()).await,
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
        } => handle_search(app, query, *json, *installed).await,
        Commands::Teleport { package, backend } => handle_teleport(app, package, backend).await,
        Commands::List {
            backend,
            json,
            outdated,
        } => handle_list(app, backend.as_deref(), *json, *outdated).await,
        Commands::Info { package } => handle_info(app, package).await,
        Commands::RemoveOrphans => handle_remove_orphans(app).await,
        Commands::CleanCache { all } => handle_clean_cache(app, *all).await,
        Commands::Heal => handle_heal(app).await,
        Commands::Adopt => handle_adopt(app).await,
        Commands::History => handle_history(app).await,
        Commands::Activate { profiles, add } => handle_activate(app, profiles, *add).await,
        Commands::Deactivate { profiles } => handle_deactivate(app, profiles).await,
        Commands::Profile(args) => handle_profile(app, &args.command).await,
        Commands::Run { packages, command } => handle_run(app, packages, command).await,
        Commands::Lock => handle_lock(app).await,
        Commands::Unlock { names, list } => handle_unlock(app, names, *list).await,
        Commands::Plan { out } => handle_plan(app, out).await,
        Commands::Apply { plan, yes } => handle_apply(app, plan, *yes).await,
        Commands::Update => handle_update(app).await,
        Commands::Reset { force } => handle_reset(app, *force).await,
        Commands::Check { section, json } => handle_check(app, section.as_deref(), *json).await,
        Commands::Vars => handle_vars(app).await,
        Commands::PurgeUnmanaged { allow_mass_purge } => {
            handle_purge_unmanaged(app, *allow_mass_purge).await
        }
        Commands::Protected { packages, json } => handle_protected(app, packages, *json).await,
        Commands::Unmanage { packages, json } => handle_unmanage(app, packages, *json).await,
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
        Commands::Why { package, json } => handle_why(app, package, *json).await,
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
            anyhow::bail!("cargo install failed; linix was not upgraded.");
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

/// Commands that only read. Everything else takes the data-directory lock, because the
/// default has to be the safe one: locking a reader costs a wait, and not locking a writer
/// costs an entry out of `registry.json`, which is a removal.
///
/// `plan` and `status` are here and `plan --save` is not a counter-example: it writes a plan
/// file, not state. **`--dry-run` is not on this list and never exempts anything** (S25): a
/// preview of a writer reads the same state a concurrent writer is rewriting, and the run
/// that proved it mattered was a `--dry-run sync` that entered recovery.
pub(crate) const READ_ONLY_COMMANDS: &[&str] = &[
    "plan",
    "status",
    "check",
    "list",
    "search",
    "doctor",
    "diff",
    "unmanaged",
    "absent",
    "vars",
    "export",
    "sbom",
    "insight",
    "why",
    "info",
    "show",
    "audit",
    "outdated",
    "history",
    "log",
    "completions",
    "help",
    "locate",
    "metrics",
    "verify",
    "eval",
    "try",
    "repl",
];

/// Take the lock for a mutating command. The command is read from argv rather than matched
/// out of `Commands`, so a subcommand added later is locked by default instead of being
/// forgotten by a match arm nobody updated.
pub(crate) fn acquire_data_lock() -> Result<Option<linix::core::datalock::DataLock>> {
    let argv: Vec<String> = std::env::args().collect();
    let name = find_subcommand_index(&argv)
        .map(|i| argv[i].clone())
        .unwrap_or_default();
    if READ_ONLY_COMMANDS.contains(&name.as_str()) {
        return Ok(None);
    }
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
/// a `bool` and consumes nothing — so `linix --progress up` skipped past `up` and the alias
/// never expanded.
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
/// composition, and threading `linix refresh --dry-run` into some steps and not others is the
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

/// Run a user verb: build the config and app once from the shared leading flags, then dispatch
/// each step against them in order, stopping at the first failure. One data lock covers the
/// whole verb — the verb name is unknown to `acquire_data_lock`, so it locks as a writer, the
/// safe default for a sequence that may install or remove.
pub(crate) async fn run_user_verb(steps: Vec<Vec<String>>) -> Result<()> {
    let first = Cli::parse_from(&steps[0]);
    let config = load_and_merge_config(&first).await?;
    linix::backends::node_registry::set_http_timeout(config.network_timeout_secs);
    let _data_lock = acquire_data_lock()?;
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
    config.merge_cli_overrides(
        Some(cli.dry_run),
        Some(cli.yes),
        None,
        Some(cli.verbose > 0),
        Some(cli.allow_mass_removal),
        Some(cli.allow_mass_install),
    )?;
    // A per-run acknowledgement, never a config key (U23): a machine that always bypasses the
    // dotfiles collision check is a machine where the check does not exist.
    if cli.replace_existing {
        config.replace_existing = true;
    }
    // --quiet has no config-file merge counterpart; apply it directly (a set flag wins).
    if cli.quiet {
        config.quiet = true;
    }
    // `uninstall --purge` is a `[remove] purge` for this run only. Read here, where CLI flags
    // become config, because `config` is shared read-only by the time a command runs.
    if let Commands::Uninstall { purge: true, .. } = cli.command {
        config.purge_this_run = true;
    }
    // --no-progress is the real off-switch for the progress indicators (S5). A set flag wins
    // over the `show_progress` config default.
    if cli.no_progress {
        config.show_progress = false;
    }
    // Fold the user-editable keep-list into the protected set. It lives in the GLOBAL
    // groups folder, which `-g` no longer moves — previously `-g /tmp/foo` made this look
    // for /tmp/foo/keep.txt, found nothing, returned early, and every keep-list protection
    // silently evaporated for that command. Every `is_protected` consumer honors it.
    Ok(config)
}

pub(crate) async fn perform_maintenance(app: &App) -> Result<()> {
    app.journal.lock().await.cleanup()?;
    // Reclaim expired temporary installs so leases are enforced on every state-changing run.
    if let Err(e) = app.leases().sweep_expired().await {
        warn!("Maintenance: lease sweep failed: {}", e);
    }
    // Restore temporary uninstalls whose timer has elapsed (mirror of the lease sweep).
    if let Err(e) = app.leases().sweep_due_suspensions().await {
        warn!("Maintenance: suspension sweep failed: {}", e);
    }
    // Version-control the manifests/config if the user opted in via `linix git init`.
    app.git_autocommit("linix: sync manifest state").await;
    if app.config.snapshot_retention().prunes() {
        app.prune_snapshots(false).await?;
    }
    Ok(())
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
    use super::log_level_from_argv;

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
        assert_eq!(log_level_from_argv(&argv(&["linix", "-vv", "list"])), "debug");
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "-v", "-v", "list"])),
            "debug"
        );
        assert_eq!(
            log_level_from_argv(&argv(&["linix", "--verbose", "--verbose", "list"])),
            "debug"
        );
        // Past two there is nothing more to say, and it must not fall back to the default.
        assert_eq!(log_level_from_argv(&argv(&["linix", "-vvvv", "list"])), "debug");
    }

    /// A short flag can arrive bundled with its neighbours, and every letter in the bundle
    /// counts — `-nv` is a dry run that talks.
    #[test]
    fn bundled_short_flags_are_read_letter_by_letter() {
        assert_eq!(log_level_from_argv(&argv(&["linix", "-nv", "sync"])), "info");
        assert_eq!(log_level_from_argv(&argv(&["linix", "-nvv", "sync"])), "debug");
        assert_eq!(log_level_from_argv(&argv(&["linix", "-nq", "sync"])), "error");
    }

    #[test]
    fn quiet_wins_over_loud_whichever_order_they_come_in() {
        assert_eq!(log_level_from_argv(&argv(&["linix", "-q", "list"])), "error");
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
}
