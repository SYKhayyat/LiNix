use anyhow::{Context, Result};
use clap::Parser;
use linix::app::sync::planner::Scope as PlannerScope;
use linix::app::{ui::TuiPreview, App};
use linix::cli::{
    Cli, Commands, ConfigCommand, GitCommand, HooksCommand,
    ModuleCommand, ProfileCommand, RepoCommand, ScheduleCommand, ServiceCommand,
    SnapshotCommand,
};
use std::collections::HashMap;
use std::env;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

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
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // 2. Shim hijack
    if let Some(res) = attempt_shim_hijack().await? {
        return res;
    }

    // 3. CLI & Config Bootstrap
    // Expand user-defined command aliases (config `[command_aliases]`) BEFORE clap parses, so
    // `linix up` can stand in for `linix upgrade --all`. Built-in subcommands always win.
    let cli = {
        let raw_argv: Vec<String> = std::env::args().collect();
        let aliases = preferences_path_from_argv(&raw_argv)
            .and_then(|p| linix::config::Config::from_file(&p).ok())
            .map(|c| c.command_aliases)
            .unwrap_or_default();
        if aliases.is_empty() {
            Cli::parse()
        } else {
            let known = known_subcommands();
            Cli::parse_from(expand_command_aliases(raw_argv, &aliases, &known))
        }
    };
    let config = load_and_merge_config(&cli).await?;
    linix::backends::node_registry::set_http_timeout(config.network_timeout_secs);

    // 4. Kernel Initialization
    let app = App::new(config).await?;

    // 5. Command Dispatcher (Modular A+ Routing)
    match &cli.command {
        Commands::Sync { locked, json } => handle_sync(&app, *locked, *json).await,
        Commands::Watch {
            interval,
            on_change,
            pull,
            once,
        } => handle_watch(&app, *interval, *on_change, *pull, *once).await,
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
                &app,
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
        } => handle_install(&app, packages, *json, temp.as_deref(), into.as_deref()).await,
        Commands::Uninstall {
            packages,
            json,
            temp,
        } => handle_uninstall(&app, packages, *json, temp.as_ref()).await,
        Commands::Shell { packages } => handle_shell(&app, packages).await,
        Commands::Module(args) => handle_module(&app, &args.command).await,
        Commands::Schedule(args) => handle_schedule(&app, &args.command).await,
        Commands::Snapshot(args) => handle_snapshot(&app, &args.command).await,
        Commands::Rollback { reference } => handle_rollback(&app, reference).await,
        Commands::Diff { from, to } => handle_diff(&app, from, to.as_deref()).await,
        Commands::Git(args) => handle_git(&app, &args.command).await,
        Commands::Repo(args) => handle_repo(&app, &args.command).await,
        Commands::Search {
            query,
            json,
            installed,
        } => handle_search(&app, query, *json, *installed).await,
        Commands::List {
            backend,
            json,
            outdated,
        } => handle_list(&app, backend.as_deref(), *json, *outdated).await,
        Commands::Info { package } => handle_info(&app, package).await,
        Commands::RemoveOrphans => handle_remove_orphans(&app).await,
        Commands::CleanCache => handle_clean_cache(&app).await,
        Commands::Heal => handle_heal(&app).await,
        Commands::Doctor { fix, json } => handle_doctor(&app, *fix, *json).await,
        Commands::Adopt => handle_adopt(&app).await,
        Commands::Undo => handle_undo(&app).await,
        Commands::History => handle_history(&app).await,
        Commands::Activate { profiles, add } => handle_activate(&app, profiles, *add).await,
        Commands::Deactivate { profiles } => handle_deactivate(&app, profiles).await,
        Commands::Profile(args) => handle_profile(&app, &args.command).await,
        Commands::Run { packages, command } => handle_run(&app, packages, command).await,
        Commands::Status { json } => handle_status(&app, *json).await,
        Commands::Lock => handle_lock(&app).await,
        Commands::Plan { out } => handle_plan(&app, out).await,
        Commands::Apply { plan, yes } => handle_apply(&app, plan, *yes).await,
        Commands::Update => handle_update(&app).await,
        Commands::Unmanaged => handle_unmanaged(&app).await,
        Commands::Check => handle_check(&app).await,
        Commands::Absent => handle_absent(&app).await,
        Commands::PurgeUnmanaged { allow_mass_purge } => {
            handle_purge_unmanaged(&app, *allow_mass_purge).await
        }
        Commands::Protected { packages, json } => handle_protected(&app, packages, *json).await,
        Commands::Unmanage { packages, json } => handle_unmanage(&app, packages, *json).await,
        Commands::Rebuild {
            packages,
            backend,
            all,
        } => handle_rebuild(&app, packages, backend.as_deref(), *all).await,
        Commands::Config(args) => handle_config(&app, &args.command).await,
        Commands::Path { explain, set } => handle_path(&cli, *explain, set.as_deref()).await,
        Commands::Edit { file } => handle_edit(&cli, file.as_deref()).await,
        Commands::Init { force, interactive } => handle_init(&app, *force, *interactive).await,
        Commands::Audit { json } => handle_audit(&app, *json).await,
        Commands::Sbom => handle_sbom(&app).await,
        Commands::Export {
            format,
            out,
            stdout,
            force,
        } => handle_export(&app, format.as_deref(), out, *stdout, *force).await,
        Commands::Bundle {
            out,
            artifacts,
            archive,
        } => handle_bundle(&app, out, *artifacts, *archive).await,
        Commands::Why { package, json } => handle_why(&app, package, *json).await,
        Commands::Service(args) => handle_service(&app, &args.command).await,
        Commands::Bisect { test, yes } => linix::app::bisect::bisect(&app, test, *yes)
            .await
            .map_err(|e| e.into()),
        Commands::Fleet(args) => linix::app::fleet::fleet(&app, &args.hosts, args.sync, args.apply)
            .await
            .map_err(|e| e.into()),
        Commands::Hooks(args) => handle_hooks(&app, &args.command).await,
        Commands::HookRecord {
            manager,
            op,
            targets,
        } => handle_hook_record(&app, manager, op, targets).await,
        Commands::HookReconcile { manager } => handle_hook_reconcile(&app, manager).await,
        Commands::HookObserve {
            manager,
            learn,
            argv,
        } => handle_hook_observe(&app, manager.as_deref(), *learn, argv).await,
        Commands::Hold { packages } => handle_hold(&app, packages).await,
        Commands::Unhold { packages } => handle_unhold(&app, packages).await,
        Commands::Conflicts { json } => handle_conflicts(&app, *json).await,
        Commands::Policy => handle_policy(&app).await,
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
fn self_upgrade_repo(git: Option<&str>) -> String {
    git.map(|s| s.to_string())
        .or_else(|| std::env::var("LINIX_REPO").ok())
        .unwrap_or_else(|| "https://github.com/OWNER/linix".to_string())
}

async fn cargo_install_from(repo: &str, locked: bool) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("install").arg("--git").arg(repo).arg("--force");
    if locked {
        cmd.arg("--locked");
    }
    cmd.status().await
}

async fn handle_self_upgrade(git: Option<&str>, check: bool) -> Result<()> {
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
fn flag_from_argv(argv: &[String], names: &[&str]) -> Option<String> {
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
fn preferences_path_from_argv(argv: &[String]) -> Option<std::path::PathBuf> {
    if let Some(p) = flag_from_argv(argv, &["-c", "--config"]) {
        return Some(std::path::PathBuf::from(p));
    }
    let dir = flag_from_argv(argv, &["--config-dir"]).map(std::path::PathBuf::from);
    linix::app::locate::locate(dir.as_deref())
        .ok()
        .map(|r| r.path.join(linix::config::PREFERENCES_FILE_NAME))
}

/// The set of built-in subcommand names (and their clap aliases), so a user command-alias can
/// never shadow a real command.
fn known_subcommands() -> std::collections::HashSet<String> {
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
fn global_value_flags() -> std::collections::HashSet<String> {
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
fn find_subcommand_index(argv: &[String]) -> Option<usize> {
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
fn expand_command_aliases(
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

// ============================================================================
// COMMAND HANDLERS
// ============================================================================

/// How one reconcile pass should behave. The pass itself is identical for `sync` and
/// `watch` — II.7's ordering phases, the guard, the same planner — and these are the only
/// things that legitimately differ between an attended run and an unattended one.
struct Reconcile {
    /// Strict version matching against the lockfile.
    locked: bool,
    /// Emit the change report as JSON instead of a planned-changes list.
    json: bool,
    /// Which scope the guard reports refusals under.
    scope: linix::app::sync::guard::GuardScope,
    /// Whether to ask before applying. `watch` is unattended by definition and never asks;
    /// `sync` asks unless `--yes`.
    confirm: bool,
}

/// One reconcile pass: resolve the model, apply repos, plan, apply, then dependents,
/// schedules and extras — II.7's ordering, in order.
///
/// Returns the number of package changes applied. `sync` and `watch` both call this; the
/// copy `watch` used to carry drifted from this body every time sync's ordering changed,
/// which is why it is one function now.
async fn reconcile(app: &App, opts: Reconcile) -> Result<usize> {
    let engine = app.sync_engine().await;
    if app.journal.lock().await.needs_recovery() {
        warn!("the transaction journal records an interrupted run; healing first.");
        engine.heal().await?;
    }

    let resolver = linix::app::sync::resolver::StateResolver::new(
        &app.config,
        app.registry.clone(),
        opts.locked,
    )
    .await;
    // The whole desired state, extras included — repos must be applied before packages
    // (II.7), so this needs more than the package map.
    let state = resolver.resolve_model().await?;
    let desired = state.packages.clone();
    enforce_policy(app, &desired).await?;

    // Ordering phase 1: repos → refresh indexes. A package from a PPA cannot install until
    // the PPA is added, so this runs before the package plan (not inside it).
    app.apply_repositories(&state).await?;

    // Drift is scoped to the backends this host lists in `priority`: a full sync must not
    // reap a backend you have simply stopped listing.
    let enabled = app.priority_backends().await;
    let mut changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_enabled(enabled);
        planner.plan(&desired, None).await?
    };

    // A config can be all dependents/schedules and no package changes (just a `service:` or a
    // `schedule:` line). That is still work, so the "nothing to do" exit has to account for
    // the dependent phase and the schedule phase too.
    if changes.is_empty() && !state.has_dependents() && state.schedules().next().is_none() {
        // Even with no packages/dependents/schedules to apply, an extra may have been
        // *removed* — deleting the last `service:` line is a real change (S20). Reconcile the
        // applied-extras ledger so that undo still happens; it is a cheap no-op otherwise.
        app.reconcile_extras(&state).await?;
        return Ok(0);
    }

    let applied = changes.total_install() + changes.total_remove();

    if !opts.json && !changes.is_empty() {
        print_flight_plan(app, &changes);
    }

    // Dry-run is preview-only: never prompt, never mutate. (JSON dry-run emits the report.)
    if app.config.dry_run {
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        }
        // Ordering phase 3, previewed: the dependent extras that a real run would apply
        // after the packages, then the schedule phase, then the undo of removed extras.
        app.apply_dependents(&state).await?;
        app.apply_schedules(&state).await?;
        app.reconcile_extras(&state).await?;
        return Ok(applied);
    }

    // The package plan runs only when it has something in it — a dependents-only sync skips
    // straight to phase 3, with no planned-changes list and no confirmation to answer.
    if !changes.is_empty() {
        // Interactive confirmation — but only with a real terminal. A non-interactive caller
        // (pipe/CI/script) must pass --yes (or --json); otherwise we neither hang on a TUI
        // that can't receive input nor silently apply unconfirmed changes.
        if opts.confirm && !app.config.yes && !opts.json {
            use std::io::IsTerminal;
            if !std::io::stdin().is_terminal() {
                anyhow::bail!(
                    "Refusing to apply changes without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
                );
            }
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !preview.run()? {
                return Ok(0);
            }
            changes = preview.get_filtered_changes();
        }

        engine.sync(changes, opts.scope).await?;
    }

    // Ordering phase 3: the dependent extras, now that every package they lean on is in.
    app.apply_dependents(&state).await?;
    // Ordering phase 4 (S21): provision the declared schedules onto the OS scheduler.
    app.apply_schedules(&state).await?;
    // Phase 5 (S20): undo extras that were applied before but are no longer declared.
    app.reconcile_extras(&state).await?;
    perform_maintenance(app).await?;
    Ok(applied)
}

/// `linix rebuild` — remove and reinstall what is declared, one backend at a time (X.1, K1).
async fn handle_rebuild(
    app: &App,
    packages: &[String],
    backend: Option<&str>,
    all: bool,
) -> Result<()> {
    use linix::app::rebuild::{self, Scope};
    use linix::app::sync::guard::{self, GuardScope};
    use linix::core::transaction::GraphAction;

    // K2: no default scope. The failure mode is declared software missing from a machine, and
    // `--all` is not something to arrive at by pressing enter.
    let scope = match (packages.is_empty(), backend, all) {
        (_, Some(b), _) => Scope::Backend(b.to_string()),
        (_, None, true) => Scope::All,
        (false, None, false) => {
            let registry = app.registry.clone();
            Scope::Packages(
                packages
                    .iter()
                    .map(|p| rebuild::Target::parse(p, |b| registry.get(b).is_some()))
                    .collect(),
            )
        }
        (true, None, false) => anyhow::bail!(
            "rebuild needs a scope — it removes software in order to put it back:\n\n\
             \x20   linix rebuild fd ripgrep       one or more packages (`cargo:fd` picks a backend)\n\
             \x20   linix rebuild --backend cargo  everything that backend declares\n\
             \x20   linix rebuild --all            every declared package on this machine"
        ),
    };

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    // A rebuild reinstalls, so it is a change path and the `[guard]` gate applies. Checked
    // against the declared set before anything is removed — a `deny_packages` hit must stop
    // the removal, not be discovered between the removal and the reinstall.
    enforce_policy(app, &desired).await?;
    let declared: Vec<linix::core::PackageSpec> = desired.into_values().flatten().collect();

    let priority = app.priority_backends().await;
    let registry = app.registry.clone();
    let is_foundation = |b: &str| registry.get(b).map(|m| m.needs_root()).unwrap_or(false);

    let mut plan = {
        let state = app.state.lock().await;
        rebuild::plan(
            &scope,
            &declared,
            &|backend, name| state.is_managed(backend, name),
            &priority,
            &is_foundation,
        )
    };

    // The guard refuses to remove a protected package, and it is right to: a rebuild's removal
    // is only safe because a reinstall follows, and if that reinstall fails the machine is
    // genuinely without it. Narrow the scope here rather than ask the guard for an exception —
    // `rebuild --all` stays usable on a machine whose `bash` is protected, and the refusal
    // keeps meaning what it says.
    {
        let all_pairs: Vec<(String, String)> = plan
            .batches
            .iter()
            .flat_map(|b| b.specs.iter().map(|s| (b.backend.clone(), s.name.clone())))
            .collect();
        let backends: std::collections::HashSet<String> =
            all_pairs.iter().map(|(b, _)| b.clone()).collect();
        let essential = guard::essential_names(&app.registry, &backends).await;
        rebuild::without_protected(&mut plan, &|backend, name| {
            guard::protection_of(&app.config, backend, name, &essential).map(|p| p.reason())
        });
    }

    for skip in &plan.skipped {
        info!("skipping {} — {}", skip.key, skip.reason);
    }
    if plan.is_empty() {
        info!("nothing to rebuild.");
        return Ok(());
    }

    println!(
        "\nRebuilding {} package(s) across {} backend(s), one backend at a time:",
        plan.total(),
        plan.batches.len()
    );
    for batch in &plan.batches {
        println!("  {:<10} {}", batch.backend, batch.names().join(" "));
    }
    println!(
        "\nEach backend's packages are removed together, then reinstalled together. If a \
         reinstall fails,\nthat backend's software is missing until it succeeds — later \
         backends are not started."
    );

    if app.config.dry_run {
        return Ok(());
    }

    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to rebuild without confirmation in a non-interactive shell. Re-run with --yes, or --dry-run to preview."
            );
        }
        let proceed = dialoguer::Confirm::new()
            .with_prompt("Remove and reinstall these packages?")
            .default(false)
            .interact()?;
        if !proceed {
            return Ok(());
        }
    }

    let engine = app.sync_engine().await;
    for batch in &plan.batches {
        info!(
            "rebuilding {} ({} package(s))",
            batch.backend,
            batch.specs.len()
        );

        // Removal and reinstall are two transactions, not one graph. The transaction engine
        // runs independent nodes concurrently, and a Remove and an Install of the same package
        // have no edge between them — in one graph they would race.
        let mut down = linix::app::sync::planner::SyncChanges::default();
        for spec in &batch.specs {
            down.removal_tracker
                .insert(format!("{}:{}", batch.backend, spec.name));
            down.graph.add_node(GraphAction::Remove {
                name: spec.name.clone(),
                backend: batch.backend.clone(),
            });
        }
        engine.sync(down, GuardScope::Rebuild).await?;

        let mut up = linix::app::sync::planner::SyncChanges::default();
        for spec in &batch.specs {
            let idx = up.graph.add_node(GraphAction::Install(spec.clone()));
            up.install_map.insert(spec.name.clone(), idx);
        }
        // K3: the removal has already happened. If this fails the batch's software is gone,
        // so say which packages and stop — starting the next backend would widen a hole.
        if let Err(e) = engine.sync(up, GuardScope::Rebuild).await {
            anyhow::bail!(
                "rebuild of `{}` failed while reinstalling: {}\n\n\
                 These packages were removed and are NOT back:\n    {}\n\n\
                 The pre-sync snapshot (if one was taken) can restore them, or re-run \
                 `linix rebuild --backend {}` once the cause is fixed.\n\
                 Remaining backends were not started.",
                batch.backend,
                e,
                batch.names().join(" "),
                batch.backend
            );
        }
    }

    info!("rebuild complete.");
    Ok(())
}

async fn handle_sync(app: &App, locked: bool, json: bool) -> Result<()> {
    let applied = reconcile(
        app,
        Reconcile {
            locked,
            json,
            scope: linix::app::sync::guard::GuardScope::Sync,
            confirm: true,
        },
    )
    .await?;
    if applied == 0 {
        info!("already up to date");
    }
    Ok(())
}

/// A cheap fingerprint of the manifest directory: (path, size, mtime) for every `*.txt`. If it
/// changes between ticks, a manifest was edited. Best-effort — errors just yield an empty sig.
/// A fingerprint of every wish-list manifest, so `watch` notices an edit.
///
async fn manifest_signature(dir: &std::path::Path) -> Vec<(String, u64, i64)> {
    let mut sig = Vec::new();
    {
        let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
            return sig;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            if path.extension().map(|e| e == "txt").unwrap_or(false) {
                if let Ok(meta) = entry.metadata().await {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    sig.push((path.to_string_lossy().into_owned(), meta.len(), mtime));
                }
            }
        }
    }
    sig.sort();
    sig
}

/// One unattended reconcile pass. `watch` is unattended by definition, so it never asks —
/// that flag is the only thing separating it from `sync`, which is why both go through the
/// same [`reconcile`].
async fn watch_reconcile(app: &App) -> Result<usize> {
    reconcile(
        app,
        Reconcile {
            locked: false,
            json: false,
            scope: linix::app::sync::guard::GuardScope::Watch,
            confirm: false,
        },
    )
    .await
}

async fn handle_watch(
    app: &App,
    interval: u64,
    on_change: bool,
    pull: bool,
    once: bool,
) -> Result<()> {
    let interval = interval.max(1);
    println!(
        "linix watch: reconciling {} every {}s{}{}. Ctrl-C to stop.",
        app.config.config_root().display(),
        interval,
        if pull { " (git pull each tick)" } else { "" },
        if on_change { " (on change only)" } else { "" },
    );
    let mut last_sig = manifest_signature(&app.config.config_root().join("modules")).await;
    let mut first = true;
    loop {
        if pull {
            let git = app.git_manager();
            if git.is_repo() {
                match git.pull() {
                    Ok(msg) => info!("watch: git pull — {}", msg.lines().last().unwrap_or("")),
                    Err(e) => warn!("watch: git pull failed: {}", e),
                }
            }
        }
        let sig = manifest_signature(&app.config.config_root().join("modules")).await;
        let changed = sig != last_sig;
        // Reconcile on the first pass and whenever something changed; with --on-change we skip
        // ticks where nothing moved (the manifests and, after a pull, the repo are unchanged).
        if first || changed || !on_change {
            if changed && !first {
                println!("watch: manifests changed — reconciling.");
            }
            match watch_reconcile(app).await {
                Ok(0) => {
                    if changed || first {
                        println!("watch: already in sync.");
                    }
                }
                Ok(n) => println!("watch: applied {} change(s).", n),
                Err(e) => warn!("watch: reconcile failed: {}", e),
            }
            last_sig = sig;
        }
        first = false;
        if once {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
    }
    Ok(())
}

/// Everything `handle_upgrade` needs, bundled so the dispatch site stays readable and the
/// handler doesn't grow an unwieldy positional signature.
struct UpgradeRequest<'a> {
    packages: &'a [String],
    backend: Option<&'a str>,
    all: bool,
    security: bool,
    except: &'a [String],
    profile: &'a Option<String>,
    module: &'a Option<String>,
    json: bool,
    canary: bool,
    test: &'a Option<String>,
}

impl UpgradeRequest<'_> {
    fn scope(&self) -> Option<PlannerScope> {
        if let Some(p) = self.profile {
            Some(PlannerScope::Profile(p.clone()))
        } else {
            self.module.as_ref().map(|m| PlannerScope::Module(m.clone()))
        }
    }
}

/// True if `except` names this package, matching either the bare name or `backend:name`.
fn upgrade_excluded(except: &[String], backend: &str, name: &str) -> bool {
    let qualified = format!("{}:{}", backend, name);
    except
        .iter()
        .any(|e| e == name || e == &qualified || e.eq_ignore_ascii_case(name))
}

/// Upgrade a single managed package by routing through the normal install path. When
/// `version` is `Some`, pin to exactly that version (`options["version"]`, which pin-capable
/// backends honor) — used by `--security` to land on the fixed version rather than blindly
/// jumping to latest. `None` means "newest the backend offers".
async fn upgrade_one(app: &App, backend: &str, name: &str, version: Option<&str>) -> Result<bool> {
    let spec_str = format!("{}:{}", backend, name);
    let resolved = app.resolve_spec(&spec_str).await?;
    let mut acted = false;
    for mut spec in resolved {
        if let Some(v) = version {
            spec.options.insert("version".to_string(), v.to_string());
        }
        if let Some(b) = app.registry.get(&spec.backend) {
            if let Some(inst) = b.as_installable() {
                info!(
                    "Upgrading {}:{} to {}...",
                    spec.backend,
                    spec.name,
                    version.unwrap_or("latest")
                );
                inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                    .await?;
                acted = true;
            }
        }
    }
    Ok(acted)
}

/// Upgrade an explicit set of managed packages (or one backend's worth) to latest.
async fn upgrade_targeted(
    app: &App,
    packages: &[String],
    backend: Option<&str>,
    except: &[String],
) -> Result<()> {
    // Snapshot the managed set once so we can resolve names → backends without holding the lock.
    let managed: Vec<(String, String)> = {
        let state = app.state.lock().await;
        state
            .packages
            .iter()
            .map(|p| (p.backend.clone(), p.name.clone()))
            .collect()
    };

    // Build the target list.
    let mut targets: Vec<(String, String)> = Vec::new();
    if !packages.is_empty() {
        for req in packages {
            let (want_backend, want_name) = linix::config::parser::split_removal_target(
                req,
                |b| app.registry.get(b).is_some(),
            );
            let hit = managed.iter().find(|(b, n)| {
                n == &want_name && want_backend.as_ref().is_none_or(|wb| wb == b)
            });
            match hit {
                Some((b, n)) => targets.push((b.clone(), n.clone())),
                None => {
                    // Not currently managed — still honor an explicit, backend-qualified
                    // upgrade by resolving it fresh; otherwise warn and skip.
                    match want_backend {
                        Some(b) => targets.push((b, want_name)),
                        None => {
                            eprintln!("upgrade: '{}' is not a managed package — skipping.", req)
                        }
                    }
                }
            }
        }
    } else if let Some(scope) = backend {
        for (b, n) in &managed {
            if b == scope {
                targets.push((b.clone(), n.clone()));
            }
        }
        if targets.is_empty() {
            println!("No managed packages under backend '{}'.", scope);
            return Ok(());
        }
    }

    // Apply --backend as a filter even when explicit packages were given, and drop excludes.
    // Held packages are skipped for a broad (--backend) upgrade, but an EXPLICITLY named
    // package overrides its hold (with a warning) — naming it is a clear intent to upgrade.
    let explicit = !packages.is_empty();

    // Dry-run: describe the upgrades (after filters/holds) without touching anything.
    if app.config.dry_run {
        println!("[DRY-RUN] would upgrade:");
        let mut n = 0;
        for (b, name) in &targets {
            if let Some(scope) = backend {
                if b != scope {
                    continue;
                }
            }
            if upgrade_excluded(except, b, name) {
                continue;
            }
            if !explicit && app.state.lock().await.is_held(b, name) {
                continue;
            }
            println!("  ↑ {}:{}", b, name);
            n += 1;
        }
        if n == 0 {
            println!("  (nothing)");
        }
        return Ok(());
    }

    let mut upgraded = 0usize;
    let mut skipped = 0usize;
    for (b, n) in targets {
        if let Some(scope) = backend {
            if b != scope {
                continue;
            }
        }
        if upgrade_excluded(except, &b, &n) {
            skipped += 1;
            continue;
        }
        if app.state.lock().await.is_held(&b, &n) {
            if explicit {
                eprintln!(
                    "upgrade: '{}:{}' is held — upgrading anyway because you named it (still held; `linix unhold` to change).",
                    b, n
                );
            } else {
                println!(
                    "upgrade: skipping held {}:{} (`linix unhold` to allow).",
                    b, n
                );
                skipped += 1;
                continue;
            }
        }
        if upgrade_one(app, &b, &n, None).await? {
            upgraded += 1;
        }
    }

    app.state.lock().await.save()?;
    println!(
        "Upgraded {} package(s){}.",
        upgraded,
        if skipped > 0 {
            format!(" ({} held back by --except)", skipped)
        } else {
            String::new()
        }
    );
    perform_maintenance(app).await
}

/// Upgrade exactly the packages `audit` reports as vulnerable, to a non-vulnerable version.
/// Honors `--except`. This is the `audit → upgrade` bridge.
async fn upgrade_security(app: &App, except: &[String], json: bool) -> Result<()> {
    let report = linix::app::insight::audit(app).await?;
    if report.findings.is_empty() {
        if json {
            println!("{}", serde_json::json!({ "upgraded": [], "vulnerable": 0 }));
        } else {
            println!(
                "No known vulnerabilities across {} scanned package(s) — nothing to upgrade.",
                report.scanned
            );
        }
        return Ok(());
    }

    // Aggregate advisories per package. A package can have several; to be safe from ALL of
    // them we must reach at least the HIGHEST fixed version across its advisories, so we take
    // the max `fixed` (not the first). Packages with no reported fix pin to None (→ latest).
    use version_compare::{compare, Cmp};
    let held: Vec<String> = app.state.lock().await.held.clone();
    let is_held = |backend: &str, name: &str| {
        let q = format!("{}:{}", backend, name);
        held.iter().any(|k| k == name || k == &q)
    };
    let mut order: Vec<String> = Vec::new();
    let mut agg: std::collections::HashMap<String, (String, String, Option<String>)> =
        std::collections::HashMap::new();
    let mut excluded_keys = std::collections::HashSet::new();
    let mut held_keys = std::collections::HashSet::new();
    for f in &report.findings {
        let key = format!("{}:{}", f.backend, f.name);
        if upgrade_excluded(except, &f.backend, &f.name) {
            excluded_keys.insert(key);
            continue;
        }
        // A held package is NOT silently remediated — hold is an explicit "don't touch". We
        // surface it loudly so the user can `unhold` and re-run if they want the fix.
        if is_held(&f.backend, &f.name) {
            held_keys.insert(key);
            continue;
        }
        let entry = agg.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            (f.backend.clone(), f.name.clone(), None)
        });
        if let Some(new_fixed) = &f.fixed {
            // Keep the larger of the current best and this advisory's fixed version.
            let keep_current =
                matches!(&entry.2, Some(cur) if compare(cur, new_fixed) == Ok(Cmp::Ge));
            if !keep_current {
                entry.2 = Some(new_fixed.clone());
            }
        }
    }
    let plan: Vec<(String, String, Option<String>)> =
        order.into_iter().filter_map(|k| agg.remove(&k)).collect();
    let seen_total = plan.len() + excluded_keys.len() + held_keys.len();
    let excepted = excluded_keys.len();
    if !json {
        println!(
            "Security upgrade: {} vulnerable package(s){}.",
            plan.len(),
            if excepted > 0 {
                format!(", {} held back by --except", excepted)
            } else {
                String::new()
            }
        );
        // Vulnerable AND held: neither auto-fixed nor silently ignored — call it out.
        if !held_keys.is_empty() {
            eprintln!(
                "warning: {} vulnerable package(s) are HELD and were NOT upgraded: {}. \
                 `linix unhold <pkg>` then re-run to remediate.",
                held_keys.len(),
                {
                    let mut v: Vec<_> = held_keys.iter().cloned().collect();
                    v.sort();
                    v.join(", ")
                }
            );
        }
    }

    // Dry-run: show the remediation plan without installing.
    if app.config.dry_run {
        if !json {
            println!("[DRY-RUN] would upgrade to remediate:");
            for (backend, name, fixed) in &plan {
                match fixed {
                    Some(v) => println!("  ↑ {}:{} → {}", backend, name, v),
                    None => println!("  ↑ {}:{} → latest", backend, name),
                }
            }
            if plan.is_empty() {
                println!("  (nothing)");
            }
        }
        return Ok(());
    }

    let mut upgraded = Vec::new();
    for (backend, name, fixed) in plan {
        // Pin to the fixed version when OSV reports one; pin-capable backends land exactly
        // there, and those that ignore the pin fall back to latest (still ≥ fixed).
        match upgrade_one(app, &backend, &name, fixed.as_deref()).await {
            Ok(true) => upgraded.push(serde_json::json!({
                "backend": backend, "name": name, "pinned_to": fixed,
            })),
            Ok(false) => {}
            // Per the agreed policy: a package we can't remediate is a warning, not a stop.
            Err(e) => eprintln!("  warning: could not upgrade {}:{}: {}", backend, name, e),
        }
    }
    app.state.lock().await.save()?;

    if json {
        let mut held_list: Vec<_> = held_keys.iter().cloned().collect();
        held_list.sort();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "vulnerable": seen_total,
                "upgraded": upgraded,
                "held_unremediated": held_list,
            }))?
        );
    } else {
        println!(
            "Upgraded {} package(s) to remediate advisories.",
            upgraded.len()
        );
    }
    perform_maintenance(app).await
}

async fn handle_upgrade(app: &App, req: UpgradeRequest<'_>) -> Result<()> {
    // Canary keeps its own health-gated, scoped path.
    if req.canary {
        return handle_canary(app, req.scope(), req.test).await;
    }

    // Mode 1: audit-driven security upgrade.
    if req.security {
        return upgrade_security(app, req.except, req.json).await;
    }

    // Mode 2: explicit packages, or a --backend scope → targeted managed upgrade.
    if !req.packages.is_empty() || req.backend.is_some() {
        return upgrade_targeted(app, req.packages, req.backend, req.except).await;
    }

    // Mode 3: --all, or a bare `upgrade` with no declarative scope → native whole-system
    // batch upgrade across every backend (this is the path that actually bumps
    // `latest`-pinned packages, which the constraint-driven planner never touches).
    if req.all || req.scope().is_none() {
        if !req.except.is_empty() {
            eprintln!(
                "note: --except is ignored for the native whole-system upgrade; \
                 pass package names or use --backend/--security to scope exclusions."
            );
        }
        // Native batch upgrades (`apt upgrade`, `brew upgrade`, …) run inside each manager and
        // can't be told to skip individual packages, so LiNix holds aren't enforced here. Be
        // honest about it rather than pretend the hold was respected.
        let held_count = app.state.lock().await.held.len();
        if held_count > 0 {
            eprintln!(
                "note: {} package hold(s) are NOT enforced by the native whole-system upgrade. \
                 Use `linix upgrade --backend <b>` or per-package upgrades to honor holds.",
                held_count
            );
        }
        // `apt upgrade` is a change path, so it passes the `[guard]` gate like every other
        // one. `deny_packages` is close to meaningless against "upgrade everything";
        // `require_snapshot` is not, and a gate honoured by some change paths is a gate on
        // nothing.
        let resolver = linix::app::sync::resolver::StateResolver::new(
            &app.config,
            app.registry.clone(),
            false,
        )
        .await;
        let desired = resolver.resolve_desired_state().await?;
        enforce_policy(app, &desired).await?;

        if app.config.dry_run {
            println!(
                "[DRY-RUN] would run each backend's native whole-system upgrade (e.g. `apt upgrade`)."
            );
            return Ok(());
        }
        return app.upgrade().await.map_err(Into::into);
    }

    // Mode 4: scoped declarative upgrade (profile/module/group) via the change planner.
    let scope = req.scope();
    let json = req.json;

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, scope).await?
    };

    if app.config.dry_run {
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&changes.generate_report())?
            );
        } else {
            print_flight_plan(app, &changes);
            println!("(dry-run: scoped upgrade previewed; nothing applied.)");
        }
        return Ok(());
    }

    if !json && !changes.is_empty() {
        print_flight_plan(app, &changes);
    }


    if !changes.is_empty() {
        app.sync_engine()
            .await
            .sync(changes, linix::app::sync::guard::GuardScope::Upgrade)
            .await?;
        perform_maintenance(app).await?;
    }
    Ok(())
}

async fn handle_install(
    app: &App,
    packages: &[String],
    json: bool,
    temp: Option<&str>,
    into: Option<&str>,
) -> Result<()> {
    // P1: this command IS a shortcut for editing a file and syncing. So the edit comes
    // first and convergence follows — S15. Backwards, every refusal on the write (nothing
    // active, several profiles active, an unwritable file) landed after the package was
    // already installed: on the machine, in no file, and drift by the next sync.
    let mut lines: Vec<String> = Vec::with_capacity(packages.len());
    for pkg_str in packages {
        lines.push(match temp {
            // II.16: a lease is a dated line. `--temp 2h` is a fine thing to type and an
            // impossible thing to store, so the duration is resolved against `now` here and
            // the file gets the moment it runs out (V.38). Nothing sweeps it up later —
            // the line simply stops counting, and sync removes what nothing declares.
            Some(dur) => {
                let at = linix::model::dated::absolute_after(chrono::Utc::now(), dur)
                    .with_context(|| {
                        format!("Invalid --temp duration '{}'. Use forms like 2h, 30m, 7d.", dur)
                    })?;
                format!("{}@expires={}", pkg_str.trim(), at)
            }
            None => pkg_str.trim().to_string(),
        });
    }

    // Dry-run answers "what would this do" without touching your files or the machine.
    if app.config.dry_run {
        let mut planned = Vec::new();
        for line in &lines {
            for spec in app.resolve_spec(line).await? {
                planned.push(serde_json::json!({
                    "action": "install", "backend": spec.backend, "name": spec.name,
                    "temporary": temp.is_some(),
                }));
            }
        }
        if json {
            println!("{}", serde_json::to_string_pretty(&planned)?);
        } else {
            println!("[DRY-RUN] would install {} package spec(s):", planned.len());
            for p in &planned {
                println!(
                    "  + {}:{}",
                    p["backend"].as_str().unwrap_or(""),
                    p["name"].as_str().unwrap_or("")
                );
            }
        }
        return Ok(());
    }

    for line in &lines {
        app.declare(line, into, linix::model::Landing::Imperative)
            .await?;
    }

    // And now the ordinary declarative pipeline makes it true — which is also what puts an
    // imperative install behind the guard for the first time (II.10).
    handle_sync(app, false, json).await
}

/// `uninstall PKG… [--temp]` — remove the line from every active module, sync (II.8).
///
/// P1, like `install`: the file edit IS the command, and convergence carries it out. So the
/// removal goes through the guard, the plan and the counts, exactly as any other removal
/// does — rather than reaching for the backend directly and asking the guard afterwards.
async fn handle_uninstall(
    app: &App,
    packages: &[String],
    json: bool,
    temp: Option<&Option<String>>,
) -> Result<()> {
    // Bare `--temp` restores when a `linix shell` session ends. That is the ephemeral shell's
    // business and it is outside the model by design (II.8), so it never touches a file.
    if let Some(None) = temp {
        let has_session = app.state.lock().await.active_session_id.is_some();
        if !has_session {
            anyhow::bail!(
                "Bare `--temp` restores on shell exit, but no `linix shell` session is \
                 active. Give a duration (e.g. --temp=2h) to schedule a timed restore."
            );
        }
        return suspend_for_session(app, packages).await;
    }

    let vocab = app.vocabulary().await?;
    let layout = app.config.layout();
    let facts = linix::config::parser::HostFacts::current();

    for pkg in packages {
        // II.8: a `--temp` uninstall of something undeclared has nothing to come back to.
        if let Some(Some(dur)) = temp {
            let declared = !linix::model::active_module_files(&layout, &vocab, &facts)
                .is_empty()
                && app.declares(pkg).await?;
            if !declared {
                anyhow::bail!(
                    "{} isn't declared, so there's nothing for it to come back to. \
                     Did you mean a plain uninstall?",
                    pkg
                );
            }

            // II.16/V.37: "take the game away until the weekend". An `absent:` line with a
            // date beats the module that wants it (II.7 rule 6) until the date passes —
            // then the module wins again and it comes back. No timer, no sweep: the same
            // dated-line machinery `install --temp` uses, pointed the other way.
            let at = linix::model::dated::absolute_after(chrono::Utc::now(), dur)
                .with_context(|| {
                    format!("Invalid --temp duration '{}'. Use forms like 2h, 30m, 7d.", dur)
                })?;
            let spec = app
                .resolve_spec(pkg)
                .await?
                .into_iter()
                .next()
                .with_context(|| format!("no package `{}` in any backend you use", pkg))?;
            app.declare(
                &format!("absent:{}:{}@until={}", spec.backend, spec.name, at),
                None,
                linix::model::Landing::Imperative,
            )
            .await?;
            continue;
        }

        // A line you can see deleted, while an identical line waits in a module you forgot
        // about, is a package that returns the next time you switch profiles (II.8).
        for module in linix::model::inactive_declarations(&layout, &vocab, &facts, pkg) {
            warn!(
                "{} is still declared in module `{}`, which isn't active. It will come back \
                 if a profile you activate uses it.",
                pkg, module
            );
        }

        let edits = app.undeclare(pkg).await?;
        if edits.is_empty() {
            warn!("{} is not declared in any active file.", pkg);
        }
    }

    // And the ordinary pipeline removes it: the package is now drift, and removing drift is
    // what sync is (V.34).
    handle_sync(app, false, json).await
}

/// Bare `--temp` inside an ephemeral shell: suspend now, restore when the session ends.
///
/// Outside the model on purpose (II.8) — a shell session is not a declaration, and writing
/// a file for something that ends when the shell does would leave the file behind.
async fn suspend_for_session(app: &App, packages: &[String]) -> Result<()> {
    for pkg_str in packages {
        let (scoped_backend, bare_name) =
            linix::config::parser::split_removal_target(pkg_str, |b| app.registry.get(b).is_some());

        let mut done = false;
        for b in app.registry.available() {
            if scoped_backend.as_deref().is_some_and(|sb| sb != b.name()) {
                continue;
            }
            let Some(inst) = b.as_installable() else {
                continue;
            };
            let (present, version) = match b.as_queryable() {
                Some(q) => match q.info(&bare_name).await? {
                    Some(p) => (true, p.version),
                    None => (false, None),
                },
                None => (scoped_backend.as_deref() == Some(b.name()), None),
            };
            if !present {
                continue;
            }

            // Every removal path calls the guard (II.10), this one included.
            linix::app::sync::guard::enforce(
                &app.config,
                &app.registry,
                &[(b.name().to_string(), bare_name.clone())],
                linix::app::sync::guard::GuardScope::Remove,
            )
            .await?;

            if app.config.dry_run {
                println!("[DRY-RUN] would suspend {}:{}", b.name(), bare_name);
                done = true;
                break;
            }

            inst.remove(std::slice::from_ref(&bare_name), b.sudo_for_write())
                .await?;
            app.state.lock().await.remove(b.name(), &bare_name);
            app.state
                .lock()
                .await
                .suspend(b.name(), &bare_name, version, None)?;
            info!(
                "{} suspended; it comes back when this shell exits.",
                bare_name
            );
            done = true;
            break;
        }
        if !done {
            warn!("'{}' is not installed under any backend you use.", pkg_str);
        }
    }
    app.state.lock().await.save()?;
    Ok(())
}

async fn handle_repo(app: &App, cmd: &RepoCommand) -> Result<()> {
    let explicit = match cmd {
        RepoCommand::Add { backend, .. } => backend.clone(),
        RepoCommand::Remove { backend, .. } => backend.clone(),
        RepoCommand::List { backend } => backend.clone(),
    };
    // No explicit `--backend`: fall back to the first backend in the `priority` file (this
    // host's default manager), or `apt` if the file names nothing.
    let b_name = match explicit {
        Some(b) => b,
        None => app
            .priority_backends()
            .await
            .into_iter()
            .next()
            .unwrap_or_else(|| "apt".into()),
    };

    let b = app.registry.get(&b_name).context("Backend not found")?;
    let mgr = b
        .as_repo_manager()
        .context("Backend does not support repository management.")?;

    match cmd {
        RepoCommand::Add { name, url, .. } => {
            info!("Repo: Adding {} to {}...", name, b_name);
            mgr.add_repo(name, url, b.sudo_for_write()).await?;
        }
        RepoCommand::Remove { name, .. } => {
            info!("Repo: Removing {} from {}...", name, b_name);
            mgr.remove_repo(name, b.sudo_for_write()).await?;
        }
        RepoCommand::List { .. } => {
            let repos = mgr.list_repos().await?;
            println!("{:<20} SOURCE", "NAME");
            for (n, u) in repos {
                println!("{:<20} {}", n, u);
            }
        }
    }
    Ok(())
}

/// Destroying a file you wrote is a plain refusal plus `--force`, like every other tool
/// (II.8). It has nothing to do with packages, so no removal setting reaches it — one prompt
/// standing for two unrelated questions is how it came to mean neither (E12).
fn refuse_overwrite(path: &std::path::Path, name: &str, force: bool) -> Result<()> {
    if force || !path.exists() {
        return Ok(());
    }
    anyhow::bail!(
        "module `{}` already exists at {}.\n  \
         Pass --force to overwrite it, or pick another name.",
        name,
        path.display()
    )
}

fn module_name(name: &str) -> Result<linix::model::ModuleName> {
    linix::model::ModuleName::new(name).map_err(|e| anyhow::anyhow!(e))
}

async fn handle_module(app: &App, cmd: &ModuleCommand) -> Result<()> {
    let layout = app.config.layout();
    match cmd {
        ModuleCommand::List => {
            // **The folder decides** (II.3): `modules/*.txt`, so a README.md in there costs
            // nothing. It used to list `*.module.txt`, a suffix II.1 does not have — so this
            // listed nothing on a real repo.
            let vocab = app.vocabulary().await?;
            let loader = linix::model::modules::ModuleLoader::new(&layout, &vocab);
            let names = loader.available();
            if names.is_empty() {
                println!(
                    "No modules yet. `linix module create <name>`, or `linix install` writes \
                     one for you."
                );
            }
            for n in names {
                println!("{}", n);
            }
        }
        ModuleCommand::Show { name } => {
            let path = layout.module_file(&module_name(name)?);
            let body = tokio::fs::read_to_string(&path).await.with_context(|| {
                format!("no module `{}` — looked in {}", name, path.display())
            })?;
            println!("{}", body);
        }
        ModuleCommand::Create { name, force } => {
            let path = layout.module_file(&module_name(name)?);
            refuse_overwrite(&path, name, *force)?;
            tokio::fs::create_dir_all(layout.modules_dir()).await.ok();
            tokio::fs::write(
                &path,
                format!(
                    "# Module: {}\n\
                     #\n\
                     # A list of what this module holds, one per line:\n\
                     #\n\
                     #   apt:curl\n\
                     #   ripgrep            (no backend named — LiNix asks each one in\n\
                     #                       `priority` order, then locks the answer)\n\
                     #   use base           (bring in another module)\n\
                     #   absent:apt:nano    (this must NOT exist)\n\
                     #\n\
                     # Nothing here happens until a profile reaches it: `use {}`.\n",
                    name, name
                ),
            )
            .await?;
            println!("Created {}", path.display());
            println!("  Add it to a profile with `use {}` — nothing reads a module no profile names.", name);
        }
        ModuleCommand::Add { source, name, force } => {
            use linix::app::module_registry;
            let (url, default_name) = module_registry::resolve_module_source(source)?;
            let final_name = name.clone().unwrap_or(default_name);
            let path = layout.module_file(&module_name(&final_name)?);
            refuse_overwrite(&path, &final_name, *force)?;

            let client = reqwest::Client::builder()
                // Honour the configured value (F1); `.max(1)` only rejects a literal 0,
                // which reqwest reads as an instant-fail timeout, not "no timeout".
                .timeout(std::time::Duration::from_secs(
                    app.config.network_timeout_secs.max(1),
                ))
                .user_agent("linix-module")
                .build()?;
            info!("Fetching module from {}", url);
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("fetching {} returned HTTP {}", url, resp.status());
            }
            let body = resp.text().await?;
            if module_registry::looks_like_html(&body) {
                anyhow::bail!(
                    "response from {} looks like an HTML page, not a LiNix module — check the source",
                    url
                );
            }

            tokio::fs::create_dir_all(layout.modules_dir()).await.ok();
            tokio::fs::write(&path, &body).await?;
            let count = module_registry::count_entries(&body);
            println!(
                "Added module `{}` ({} entries) from {}\n  saved to {}\n  \
                 Use it with `use {}` in a profile — nothing reads a module no profile names.",
                final_name,
                count,
                url,
                path.display(),
                final_name
            );
        }
    }
    Ok(())
}

/// Apply a service spec (`service:<name>@<opts>`) through the install path.
async fn service_apply(app: &App, name: &str, opts: &str) -> Result<()> {
    let spec_str = if opts.is_empty() {
        format!("service:{}", name)
    } else {
        format!("service:{}@{}", name, opts)
    };
    let resolved = app.resolve_spec(&spec_str).await?;
    for spec in resolved {
        let b = app
            .registry
            .get(&spec.backend)
            .context("service backend unavailable on this host")?;
        if let Some(inst) = b.as_installable() {
            inst.install(std::slice::from_ref(&spec), b.sudo_for_write())
                .await?;
        }
    }
    Ok(())
}

async fn handle_service(app: &App, cmd: &ServiceCommand) -> Result<()> {
    // Enable/disable/start/stop/restart mutate the system and (enable/disable) the manifest.
    // Honor --dry-run by describing the action without touching either. Status/List are
    // read-only and always run.
    if app.config.dry_run {
        let action = match cmd {
            ServiceCommand::Enable { name } => Some(("enable + start", name)),
            ServiceCommand::Disable { name } => Some(("disable + stop", name)),
            ServiceCommand::Start { name } => Some(("start", name)),
            ServiceCommand::Stop { name } => Some(("stop", name)),
            ServiceCommand::Restart { name } => Some(("restart", name)),
            ServiceCommand::Status { .. } | ServiceCommand::List => None,
        };
        if let Some((what, name)) = action {
            println!("[DRY-RUN] would {} service '{}'.", what, name);
            return Ok(());
        }
    }
    match cmd {
        ServiceCommand::Enable { name } => {
            service_apply(app, name, "enabled=true,status=running").await?;
            // Persist so `sync` keeps the service enabled going forward.
            app.declare(
                &format!("service:{}@enabled=true", name),
                None,
                linix::model::Landing::Imperative,
            )
            .await?;
            println!("Service '{}' enabled and started.", name);
        }
        ServiceCommand::Disable { name } => {
            service_apply(app, name, "enabled=false,status=stopped").await?;
            app.undeclare(&format!("service:{}", name)).await?;
            println!("Service '{}' disabled and stopped.", name);
        }
        ServiceCommand::Start { name } => {
            service_apply(app, name, "status=running").await?;
            println!("Service '{}' started.", name);
        }
        ServiceCommand::Stop { name } => {
            service_apply(app, name, "status=stopped").await?;
            println!("Service '{}' stopped.", name);
        }
        ServiceCommand::Restart { name } => {
            service_apply(app, name, "status=restarted").await?;
            println!("Service '{}' restarted.", name);
        }
        ServiceCommand::Status { name } => {
            let b = app
                .registry
                .get("service")
                .context("service backend unavailable on this host")?;
            match b.as_queryable() {
                Some(q) => match q.info(name).await? {
                    Some(pkg) => {
                        println!("{}: running", name);
                        if let Some(raw) = pkg.properties.get("status_raw") {
                            println!("{}", raw.trim());
                        }
                    }
                    None => println!("{}: not running (or unknown to this init system)", name),
                },
                None => println!("service status is not queryable on this platform"),
            }
        }
        ServiceCommand::List => {
            let b = app
                .registry
                .get("service")
                .context("service backend unavailable on this host")?;
            match b.as_queryable() {
                Some(q) => {
                    let svcs = q.list_installed().await?;
                    if svcs.is_empty() {
                        println!("No running services reported.");
                    } else {
                        println!("Running services ({}):", svcs.len());
                        for s in svcs {
                            println!("  {}", s.name);
                        }
                    }
                }
                None => println!("service listing is not available on this platform"),
            }
        }
    }
    Ok(())
}

async fn handle_hooks(app: &App, cmd: &HooksCommand) -> Result<()> {
    use linix::app::pm_hooks;

    // Path to this very binary, so a hook can call back into `linix`.
    let linix_bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "linix".to_string());

    match cmd {
        HooksCommand::Install { managers } => {
            let specs = pm_hooks::hook_specs(&linix_bin);
            let mut wrote = 0usize;
            for spec in &specs {
                if !managers.is_empty() && !managers.iter().any(|m| m == spec.manager) {
                    continue;
                }
                // Only install hooks for managers actually present on this system.
                if app.registry.get(spec.manager).is_none()
                    && !managers.iter().any(|m| m == spec.manager)
                {
                    continue;
                }
                if let Some(parent) = spec.path.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        warn!(
                            "hooks: cannot create {} ({}). Try with sudo.",
                            parent.display(),
                            e
                        );
                        continue;
                    }
                }
                match tokio::fs::write(&spec.path, &spec.content).await {
                    Ok(()) => {
                        // Make script-style hooks executable on Unix.
                        #[cfg(unix)]
                        if spec.content.starts_with("#!") {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = tokio::fs::set_permissions(
                                &spec.path,
                                std::fs::Permissions::from_mode(0o755),
                            )
                            .await;
                        }
                        println!("  installed  {:<8} {}", spec.manager, spec.path.display());
                        wrote += 1;
                    }
                    Err(e) => warn!(
                        "hooks: failed to write {} ({}). This usually needs root.",
                        spec.path.display(),
                        e
                    ),
                }
            }
            if wrote == 0 {
                println!(
                    "No hooks installed. Named managers may be absent, or writing needs sudo.\n\
                     Hookable managers: {}",
                    pm_hooks::hookable_manager_names().join(", ")
                );
            } else {
                println!(
                    "\nInstalled {wrote} hook file(s). Manual installs now record into LiNix."
                );
            }
        }
        HooksCommand::Uninstall { managers } => {
            let specs = pm_hooks::hook_specs(&linix_bin);
            let mut removed = 0usize;
            for spec in &specs {
                if !managers.is_empty() && !managers.iter().any(|m| m == spec.manager) {
                    continue;
                }
                if tokio::fs::try_exists(&spec.path).await.unwrap_or(false) {
                    match tokio::fs::remove_file(&spec.path).await {
                        Ok(()) => {
                            println!("  removed    {:<8} {}", spec.manager, spec.path.display());
                            removed += 1;
                        }
                        Err(e) => warn!("hooks: failed to remove {} ({})", spec.path.display(), e),
                    }
                }
            }
            println!("Removed {removed} hook file(s).");
        }
        HooksCommand::Status => {
            let specs = pm_hooks::hook_specs(&linix_bin);
            println!("{:<10} {:<9} {:<9} PATH", "MANAGER", "PRESENT", "HOOKED");
            for spec in &specs {
                let present = app.registry.get(spec.manager).is_some();
                let hooked = tokio::fs::try_exists(&spec.path).await.unwrap_or(false);
                println!(
                    "{:<10} {:<9} {:<9} {}",
                    spec.manager,
                    if present { "yes" } else { "no" },
                    if hooked { "yes" } else { "no" },
                    spec.path.display()
                );
            }
        }
        HooksCommand::ShellInit { shell } => {
            print!("{}", pm_hooks::shell_wrappers(&linix_bin, shell));
        }
    }
    Ok(())
}

/// Shared recording path for a single hooked target. Repo installs become declarative
/// (recorded + appended to the active module); local-file installs are recorded imperatively
/// and kept OUT of the modules (not reproducible), so a sync never removes them as drift.
async fn record_hooked_target(
    app: &App,
    manager: &str,
    op: linix::app::pm_hooks::HookOp,
    target: &str,
) -> Result<()> {
    use linix::app::pm_hooks::{classify_install_target, local_file_stem, HookOp, InstallKind};

    match op {
        HookOp::Install => {
            let kind = classify_install_target(target);
            let (name, source, declarative) = match kind {
                InstallKind::Repo => (target.to_string(), format!("hook:{manager}"), true),
                InstallKind::LocalFile => {
                    (local_file_stem(target), "local-file".to_string(), false)
                }
            };
            app.state.lock().await.add(
                manager,
                &name,
                None,
                std::collections::HashMap::new(),
                Some(source),
                false,
            );
            if declarative {
                app.declare(
                    &format!("{manager}:{name}"),
                    None,
                    linix::model::Landing::Hooks,
                )
                .await?;
            }
            info!(
                "hook: recorded install {}:{} ({})",
                manager,
                name,
                if declarative {
                    "managed"
                } else {
                    "imperative/local"
                }
            );
        }
        HookOp::Remove => {
            app.state.lock().await.remove(manager, target);
            app.undeclare(&format!("{manager}:{target}")).await?;
            info!("hook: recorded remove {}:{}", manager, target);
        }
    }
    Ok(())
}

async fn handle_hook_record(app: &App, manager: &str, op: &str, targets: &[String]) -> Result<()> {
    let op = linix::app::pm_hooks::HookOp::parse(op)
        .ok_or_else(|| anyhow::anyhow!("hook-record: --op must be 'install' or 'remove'"))?;
    for target in targets {
        record_hooked_target(app, manager, op, target).await?;
    }
    app.state.lock().await.save()?;
    app.git_autocommit("linix: record hooked package change")
        .await;
    Ok(())
}

async fn handle_hook_reconcile(app: &App, manager: &str) -> Result<()> {
    // Additive reconcile: record packages the manager reports installed that LiNix isn't yet
    // tracking. We never auto-remove here — a missing package could be a transient query
    // hiccup, and destructive action from a background hook would be a nasty surprise.
    let Some(backend) = app.registry.get(manager) else {
        warn!(
            "hook-reconcile: backend '{}' is not available; skipping.",
            manager
        );
        return Ok(());
    };
    let Some(queryable) = backend.as_queryable() else {
        return Ok(());
    };
    let installed = queryable.list_installed().await.unwrap_or_default();
    let mut newly = 0usize;
    {
        let mut state = app.state.lock().await;
        for pkg in &installed {
            if !state.is_managed(manager, &pkg.name) {
                state.add(
                    manager,
                    &pkg.name,
                    pkg.version.clone(),
                    std::collections::HashMap::new(),
                    Some(format!("hook:{manager}")),
                    false,
                );
                newly += 1;
            }
        }
        state.save()?;
    }
    if newly > 0 {
        info!(
            "hook-reconcile: adopted {} new {}-installed package(s).",
            newly, manager
        );
        app.git_autocommit("linix: reconcile hooked manager").await;
    }
    Ok(())
}

async fn handle_hook_observe(
    app: &App,
    manager: Option<&str>,
    learn: bool,
    argv: &[String],
) -> Result<()> {
    use linix::app::pm_hooks::{detect_operation, extract_targets};

    let Some(op) = detect_operation(argv) else {
        // Not an install/remove command (e.g. `apt list`); nothing to record.
        return Ok(());
    };
    // Manager name: explicit, else inferred from argv[0] (the wrapped binary).
    let manager = manager
        .map(|m| m.to_string())
        .or_else(|| argv.first().cloned())
        .unwrap_or_else(|| "unknown".to_string());

    // For a brand-new manager we've never seen, suggest onboarding it properly.
    if learn && app.registry.get(&manager).is_none() {
        info!(
            "Auto-learn: observed unknown manager '{}'. Consider onboarding it with a TOML \
             definition so LiNix knows its full command set.",
            manager
        );
    }

    let targets = extract_targets(argv);
    for target in &targets {
        record_hooked_target(app, &manager, op, target).await?;
    }
    if !targets.is_empty() {
        app.state.lock().await.save()?;
        app.git_autocommit("linix: observed manual package change")
            .await;
    }
    Ok(())
}

/// `linix schedule` — a shortcut for editing the `schedules` file, then converging.
///
/// The file is the state (II.6: being in the file means it's on), so `add` and `remove` write
/// it and `sync` provisions what changed. They do not talk to the OS scheduler directly: a
/// command that registered a timer the file did not describe would be a second store, and the
/// two would disagree about what this machine runs.
async fn handle_schedule(app: &App, cmd: &ScheduleCommand) -> Result<()> {
    use linix::model::schedule::{add_line, remove_line};

    let file = app.config.layout().schedules_file();
    let body = tokio::fs::read_to_string(&file).await.unwrap_or_default();
    let registry = app.registry.clone();
    let known = move |b: &str| registry.get(b).is_some();

    match cmd {
        ScheduleCommand::Add {
            name,
            cron,
            run,
            notify,
        } => {
            let updated = add_line(&body, name, cron, run, notify.as_deref())
                .map_err(|e| anyhow::anyhow!(e))?;
            // Parse what was just written before it is written: a bad cron or an unknown key
            // must be refused at the door, naming the line, not discovered at provision time.
            linix::config::grammar::parse_document(&file, &updated, &known)?;
            tokio::fs::write(&file, &updated).await?;
            println!("Added `schedule:{}` to {}.", name, file.display());
        }
        ScheduleCommand::Remove { name } => {
            let Some(updated) = remove_line(&body, name) else {
                println!("No `schedule:{}` in {}.", name, file.display());
                return Ok(());
            };
            tokio::fs::write(&file, &updated).await?;
            println!("Removed `schedule:{}` from {}.", name, file.display());
        }
        ScheduleCommand::List => {
            let doc = linix::config::grammar::parse_document(&file, &body, &known)?;
            let facts = linix::config::parser::HostFacts::current();
            for (stmt, origin) in doc.statements_for(&facts)? {
                if let linix::config::grammar::Statement::Schedule(name, opts) = stmt {
                    let cfg = linix::model::schedule::schedule_config(&name, &opts, &origin)?;
                    println!("{:<15} {:<15} {}", cfg.name, cfg.cron, cfg.command);
                }
            }
            return Ok(());
        }
    }

    handle_sync(app, false, false).await
}

async fn handle_snapshot(app: &App, cmd: &SnapshotCommand) -> Result<()> {
    match cmd {
        SnapshotCommand::List => {
            let list = app.snapshot_manager.list_snapshots().await?;
            for s in list {
                println!("{:<15} {}", s.backend, s.id);
            }
        }
        SnapshotCommand::Prune { force } => {
            app.prune_snapshots(*force).await?;
        }
    }
    Ok(())
}

/// `linix rollback <ref>` — the one rollback (owner decision, Phase 4): check out the manifests
/// at a past git commit, then `sync` the machine to match. There is no separate generation
/// history — git IS the history (II.1), so a rollback is "point the manifests at then, converge
/// now". Whole-config by nature: git checkout is all-or-nothing, which is why the old
/// per-package / with-config flags are gone.
async fn handle_rollback(app: &App, reference: &str) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "Rollback needs manifest history. Run `linix git init` once to start version-\
             controlling your config; after that every sync commits, and you can roll back to \
             any commit."
        );
    }
    // The bail must come before the checkout, not after. `handle_sync` refuses unconfirmed
    // changes in a non-interactive shell, but by the time it does the manifests have already
    // been overwritten — leaving the files rolled back and the machine not.
    if !app.config.yes {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            anyhow::bail!(
                "Refusing to roll back without confirmation in a non-interactive shell. \
                 Re-run with --yes to proceed, or --dry-run to preview."
            );
        }
    }
    info!("checking out manifests at {}.", reference);
    git.checkout_files(reference)?;
    println!(
        "Manifests restored to {}. Converging the system to match…",
        reference
    );
    // The rollback is not complete until the machine matches the restored manifests.
    handle_sync(app, false, false).await
}

/// `linix diff <from> [to]` — what changed between two commits, in packages (Phase 4). The
/// manifests are package declarations, so a diff of the manifest files IS the package-level
/// change; git already records it. Omitting `to` compares `from` against your working tree.
async fn handle_diff(app: &App, from: &str, to: Option<&str>) -> Result<()> {
    let git = app.git_manager();
    if !git.is_repo() {
        anyhow::bail!(
            "`diff` compares commits of your manifest history, which is git. Run `linix git \
             init` once to start version-controlling your config."
        );
    }
    let changes = git.diff_manifest_changes(from, to)?;
    let target = to.unwrap_or("working tree");
    if changes.is_empty() {
        println!("No manifest changes between {} and {}.", from, target);
        return Ok(());
    }
    println!("Manifest changes {} → {}:", from, target);
    for line in &changes {
        println!("  {}", line);
    }
    let (added, removed) = changes
        .iter()
        .fold((0usize, 0usize), |(a, r), l| match l.chars().next() {
            Some('+') => (a + 1, r),
            Some('-') => (a, r + 1),
            _ => (a, r),
        });
    println!("\n{} added, {} removed.", added, removed);
    Ok(())
}

async fn handle_git(app: &App, cmd: &GitCommand) -> Result<()> {
    let git = app.git_manager();
    match cmd {
        GitCommand::Init => {
            git.init()?;
            println!(
                "Initialized manifest version control at {}.\n\
                 LiNix will now auto-commit config/manifest changes after each command.",
                git.root().display()
            );
        }
        GitCommand::Status => {
            if !git.is_repo() {
                println!("Not a git repo yet. Run `linix git init` to enable manifest history.");
                return Ok(());
            }
            let status = git.status_porcelain()?;
            if status.trim().is_empty() {
                println!("Manifests are clean (no uncommitted changes).");
            } else {
                println!("{}", status);
            }
        }
        GitCommand::Log { limit } => {
            if !git.is_repo() {
                println!("Not a git repo yet. Run `linix git init` first.");
                return Ok(());
            }
            let commits = git.log(*limit)?;
            if commits.is_empty() {
                println!("No commits yet.");
            }
            for c in commits {
                println!("{}  {}  {}", c.short, c.date, c.subject);
            }
        }
        GitCommand::Commit { message } => {
            git.init().ok(); // ensure a repo exists so `commit` is a one-step action
            match git.commit_all(message)? {
                Some(hash) => println!("Committed {} — {}", &hash[..hash.len().min(8)], message),
                None => println!("Nothing to commit; manifests are already up to date."),
            }
        }
        GitCommand::Checkout { reference } => {
            if !git.is_repo() {
                anyhow::bail!("Not a git repo. Run `linix git init` first.");
            }
            git.checkout_files(reference)?;
            println!(
                "Manifests restored to {}. Installed packages are unchanged — run `linix sync` \
                 to converge the system to these manifests.",
                reference
            );
        }
    }
    Ok(())
}

async fn handle_shell(app: &App, packages: &[String]) -> Result<()> {
    app.shell().enter(packages).await.map_err(|e| e.into())
}

async fn handle_run(app: &App, packages: &[String], command: &str) -> Result<()> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let bin = parts.first().unwrap_or(&"");
    let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
    app.runner()
        .run(packages, bin, &args)
        .await
        .map_err(|e| e.into())
}

async fn handle_adopt(app: &App) -> Result<()> {
    app.migrator().migrate().await.map_err(|e| e.into())
}
async fn handle_undo(app: &App) -> Result<()> {
    app.undo_manager()
        .run_interactive()
        .await
        .map_err(|e| e.into())
}
async fn handle_history(app: &App) -> Result<()> {
    use linix::app::ui::{HistoryBrowser, HistoryAction, CommitView};

    let git = app.git_manager();
    if !git.is_repo() {
        println!(
            "The history browses your manifest history, which is git. Run `linix git init` \
             once; after that every `sync` commits, and the history shows the timeline."
        );
        return Ok(());
    }

    // The timeline is the commit log; each row carries the manifest lines that commit changed.
    let commits: Vec<CommitView> = git
        .log(200)?
        .into_iter()
        .map(|c| {
            let changes = git.commit_manifest_changes(&c.hash).unwrap_or_default();
            CommitView {
                short: c.short,
                date: c.date,
                subject: c.subject,
                full_hash: c.hash,
                changes,
            }
        })
        .collect();

    if commits.is_empty() {
        println!("No commits yet. Run `linix sync` (it commits after each successful change).");
        return Ok(());
    }

    let action = HistoryBrowser::new(commits).run()?;
    match action {
        HistoryAction::Quit => Ok(()),
        HistoryAction::Rollback { reference } => {
            println!("Rolling back to {reference}…");
            handle_rollback(app, &reference).await
        }
    }
}

async fn handle_activate(app: &App, profiles: &[String], add: bool) -> Result<()> {
    app.profile_manager()
        .activate(profiles, add)
        .await
        .map_err(|e| e.into())
}

async fn handle_deactivate(app: &App, profiles: &[String]) -> Result<()> {
    app.profile_manager()
        .deactivate(profiles)
        .await
        .map_err(|e| e.into())
}

async fn handle_profile(app: &App, cmd: &ProfileCommand) -> Result<()> {
    let pm = app.profile_manager();
    match cmd {
        ProfileCommand::List => {
            let names = pm.list_profiles().await?;
            let active = pm.active_profiles().await?;
            if names.is_empty() {
                println!("No profiles defined. Create one with `linix profile create <name>`.");
            }
            for n in &names {
                let mark = if active.iter().any(|a| a == n) {
                    "\u{2605}"
                } else {
                    " "
                };
                println!("{} {}", mark, n);
            }
        }
        ProfileCommand::Show { name } => {
            for pkg in pm.show(name).await? {
                println!("{}", pkg);
            }
        }
        ProfileCommand::Create { name } => {
            pm.create(name).await?;
            println!("Created profile '{}' at the profiles directory.", name);
        }
        ProfileCommand::Save { name } => {
            pm.save_current_as(name).await?;
        }
        ProfileCommand::Active => {
            let active = pm.active_profiles().await?;
            if active.is_empty() {
                println!("No profiles are currently active.");
            }
            for a in &active {
                println!("{}", a);
            }
        }
    }
    Ok(())
}
/// `remove-orphans` — each manager's own "no longer needed by anything" set.
///
/// The orphan set is the backend's opinion, not LiNix's model, which is exactly why it gets
/// the same shape as `sync`: name every package first, put it through the guard, then ask.
/// The old `clean` ran `apt autoremove -y` / `pacman -Rs --noconfirm` across every available
/// backend with no preview and outside the guard.
async fn handle_remove_orphans(app: &App) -> Result<()> {
    use linix::app::sync::guard::{enforce, GuardScope};

    let mut listed: Vec<(String, Vec<String>)> = Vec::new();
    let mut unlistable: Vec<String> = Vec::new();

    for backend in app.registry.available() {
        let up = match backend.as_upgradable() {
            Some(u) => u,
            None => continue,
        };
        match up.list_orphans().await {
            Ok(names) if names.is_empty() => {}
            Ok(names) => listed.push((backend.name().to_string(), names)),
            // A backend that cannot list but removes natively is asked about separately,
            // because there is nothing to show. Probing by CALLING clean_orphans would
            // perform the removal it is trying to get permission for.
            Err(linix::core::Error::Unsupported(_)) if up.has_native_orphan_removal() => {
                unlistable.push(backend.name().to_string());
            }
            Err(linix::core::Error::Unsupported(_)) => {}
            Err(e) => warn!("could not list orphans for {}: {}", backend.name(), e),
        }
    }

    if listed.is_empty() && unlistable.is_empty() {
        println!("No orphaned packages.");
        return Ok(());
    }

    let removals: Vec<(String, String)> = listed
        .iter()
        .flat_map(|(b, names)| names.iter().map(move |n| (b.clone(), n.clone())))
        .collect();

    if !listed.is_empty() {
        println!("Planned changes:");
        for (backend, names) in &listed {
            println!("  {} — remove {} package(s):", backend, names.len());
            for n in names {
                println!("      {}:{}", backend, n);
            }
        }
    }

    // The guard sees the whole set at once, so the removal count and the protected list are
    // judged against the total rather than per backend.
    enforce(&app.config, &app.registry, &removals, GuardScope::RemoveOrphans).await?;

    if app.config.dry_run {
        println!("
[DRY-RUN] Nothing was removed.");
        return Ok(());
    }

    if !confirm_orphan_removal(app, &unlistable)? {
        println!("Nothing removed.");
        return Ok(());
    }

    for (backend_name, names) in &listed {
        let backend = match app.registry.get(backend_name) {
            Some(b) => b,
            None => continue,
        };
        if let Some(installable) = backend.as_installable() {
            // Remove exactly the names that were shown and guarded — not the backend's own
            // autoremove, whose set can have moved since the preview.
            installable
                .remove(names, backend.sudo_for_write())
                .await
                .with_context(|| format!("removing orphans from {}", backend_name))?;
            println!("  {}: removed {} package(s)", backend_name, names.len());
        }
    }

    for backend_name in &unlistable {
        let backend = match app.registry.get(backend_name) {
            Some(b) => b,
            None => continue,
        };
        if let Some(up) = backend.as_upgradable() {
            match up.clean_orphans(backend.sudo_for_write()).await {
                Ok(()) => println!("  {}: ran its own orphan removal", backend_name),
                Err(e) => warn!("orphan removal failed for {}: {}", backend_name, e),
            }
        }
    }

    perform_maintenance(app).await
}

/// Confirmation for `remove-orphans`. `unlistable` names the backends whose orphan set could
/// not be enumerated, so the user is told those removals cannot be previewed rather than
/// having them folded silently into a list that looks complete.
fn confirm_orphan_removal(app: &App, unlistable: &[String]) -> Result<bool> {
    if !unlistable.is_empty() {
        println!(
            "
Also: {} cannot list what it would remove, so those packages are not in the list above and cannot be checked against your protected list.",
            unlistable.join(", ")
        );
    }
    if app.config.yes {
        return Ok(true);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "Refusing to remove orphans without confirmation in a non-interactive shell. Re-run with --yes to proceed, or --dry-run to preview."
        );
    }
    Ok(dialoguer::Confirm::new()
        .with_prompt("Remove these packages?")
        .default(false)
        .interact()?)
}

/// `clean-cache` — downloaded archives and build caches. Removes no installed package, so it
/// needs no preview and no guard.
async fn handle_clean_cache(app: &App) -> Result<()> {
    if app.config.dry_run {
        println!("[DRY-RUN] Would clear the package cache for every backend that has one.");
        return Ok(());
    }
    let mut cleaned = Vec::new();
    for backend in app.registry.available() {
        let up = match backend.as_upgradable() {
            Some(u) => u,
            None => continue,
        };
        match up.clean_cache(backend.sudo_for_write()).await {
            Ok(()) => cleaned.push(backend.name().to_string()),
            Err(linix::core::Error::Unsupported(_)) => {}
            Err(e) => warn!("cache clean failed for {}: {}", backend.name(), e),
        }
    }
    if cleaned.is_empty() {
        println!("No backend on this machine has a cache to clear.");
    } else {
        println!("Cleared caches: {}.", cleaned.join(", "));
    }
    perform_maintenance(app).await
}

async fn handle_status(app: &App, json: bool) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    // `status` reports what a full `sync` would do, so it scopes drift the same way.
    let enabled = app.priority_backends().await;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        )
        .with_enabled(enabled);
        planner.plan(&desired, None).await?
    };
    let report = changes.generate_report();
    let unmanaged = app.installed_but_unmanaged().await.unwrap_or_default();

    if json {
        let out = serde_json::json!({
            "to_install": report.install,
            "to_remove": report.remove,
            "unmanaged": unmanaged.iter().map(|p| serde_json::json!({"backend": p.backend, "name": p.name})).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if report.install.is_empty() && report.remove.is_empty() && unmanaged.is_empty() {
        println!(
            "System matches your manifests; nothing to install, no drift, no unmanaged packages."
        );
        return Ok(());
    }
    if !report.install.is_empty() {
        println!("+ to install ({}):", report.install.len());
        for e in &report.install {
            println!(
                "    {}:{}{}",
                e.backend,
                e.name,
                e.version
                    .as_deref()
                    .map(|v| format!(" @ {}", v))
                    .unwrap_or_default()
            );
        }
    }
    if !report.remove.is_empty() {
        println!("- drift / `prune` would remove ({}):", report.remove.len());
        for e in &report.remove {
            println!("    {}:{}", e.backend, e.name);
        }
    }
    if !unmanaged.is_empty() {
        println!(
            "? unmanaged — installed but not in your manifests ({}):",
            unmanaged.len()
        );
        for p in &unmanaged {
            println!("    {}:{}", p.backend, p.name);
        }
    }
    Ok(())
}

/// Write the currently-installed version of every managed package to locks/versions.json so a
/// later `sync --locked` reproduces those exact versions (where the backend supports it).
/// Compute the sync changes for the current desired state (shared by `plan` and `apply`).
async fn compute_full_changes(app: &App) -> Result<linix::app::sync::SyncChanges> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;
    let state_guard = app.state.lock().await;
    let planner = linix::app::sync::planner::ChangePlanner::new(
        app.registry.clone(),
        &state_guard,
        &app.config,
    );
    Ok(planner.plan(&desired, None).await?)
}

async fn handle_plan(app: &App, out: &str) -> Result<()> {
    let changes = compute_full_changes(app).await?;
    let created_at = chrono::Utc::now().timestamp();
    let plan = linix::app::sync::SavedPlan::from_changes(&changes, Some(created_at));
    tokio::fs::write(out, serde_json::to_string_pretty(&plan)?).await?;
    if plan.is_empty() {
        println!(
            "Wrote plan to {} — system already matches desired state (no changes).",
            out
        );
    } else {
        println!(
            "Wrote plan to {} — {} install(s), {} removal(s).\nReview it, then run `linix apply {}`.",
            out,
            plan.installs.len(),
            plan.removals.len(),
            out
        );
        // Writing a plan changes nothing, so this warns rather than refuses — but say it
        // here, where there is still time to fix the manifest, rather than letting the
        // refusal be a surprise at apply time.
        let removal_pairs: Vec<(String, String)> = plan
            .removals
            .iter()
            .map(|r| (r.backend.clone(), r.name.clone()))
            .collect();
        let report =
            linix::app::sync::guard::inspect(&app.config, &app.registry, &removal_pairs).await;
        if !report.is_empty() {
            println!(
                "\nWARNING: `linix apply` will refuse this plan.\n{}",
                report.message(linix::app::sync::guard::GuardScope::Apply)
            );
        }
    }
    Ok(())
}

/// Rebuild a `SyncChanges` graph from a saved plan's install/removal lists, so the shared
/// interactive review screen (which operates on a change graph) can also drive `apply`.
fn saved_plan_to_changes(
    installs: &[linix::core::PackageSpec],
    removals: &[linix::app::sync::saved_plan::PlanRemoval],
) -> linix::app::sync::planner::SyncChanges {
    use linix::core::GraphAction;
    let mut graph = petgraph::stable_graph::StableDiGraph::new();
    for spec in installs {
        graph.add_node(GraphAction::Install(spec.clone()));
    }
    for r in removals {
        graph.add_node(GraphAction::Remove {
            name: r.name.clone(),
            backend: r.backend.clone(),
        });
    }
    linix::app::sync::planner::SyncChanges {
        graph,
        ..Default::default()
    }
}

/// Collect the `backend:name` keys that survived an interactive review, split into
/// (install-keys, removal-keys) so the caller can filter the original plan lists.
fn surviving_keys(
    changes: &linix::app::sync::planner::SyncChanges,
) -> (
    std::collections::HashSet<String>,
    std::collections::HashSet<String>,
) {
    use linix::core::GraphAction;
    let mut installs = std::collections::HashSet::new();
    let mut removes = std::collections::HashSet::new();
    for w in changes.graph.node_weights() {
        match w {
            GraphAction::Install(s) => {
                installs.insert(format!("{}:{}", s.backend, s.name));
            }
            GraphAction::Remove { name, backend } => {
                removes.insert(format!("{}:{}", backend, name));
            }
        }
    }
    (installs, removes)
}

async fn handle_apply(app: &App, plan_path: &str, yes: bool) -> Result<()> {
    let raw = tokio::fs::read_to_string(plan_path)
        .await
        .with_context(|| format!("reading plan file {}", plan_path))?;
    let plan: linix::app::sync::SavedPlan =
        serde_json::from_str(&raw).context("parsing plan file")?;

    if plan.schema != linix::app::sync::PLAN_SCHEMA {
        anyhow::bail!(
            "plan schema {} is unsupported (this linix speaks schema {})",
            plan.schema,
            linix::app::sync::PLAN_SCHEMA
        );
    }
    // Integrity: refuse a hand-edited plan unless forced.
    if plan.recomputed_hash() != plan.desired_hash && !yes {
        anyhow::bail!(
            "plan file looks modified (content hash mismatch). Re-generate with `linix plan`, \
             or pass --yes to force."
        );
    }
    if plan.is_empty() {
        println!("Plan is empty — nothing to apply.");
        return Ok(());
    }

    // Drift detection, and the `[guard]` gate: `compute_full_changes` runs `enforce_policy`,
    // so an `Err` here is a refusal and must not be swallowed. Applying a captured plan to a
    // machine whose manifests no longer resolve is the case this stops.
    {
        let now_changes = compute_full_changes(app).await?;
        let current = linix::app::sync::SavedPlan::from_changes(&now_changes, None);
        if current.desired_hash != plan.desired_hash {
            if yes {
                warn!("apply: system has drifted from the captured plan; applying anyway (--yes).");
            } else {
                println!(
                    "WARNING: the system/manifests have drifted since this plan was captured."
                );
                let proceed = dialoguer::Confirm::new()
                    .with_prompt("Apply the captured plan anyway?")
                    .default(false)
                    .interact()
                    .unwrap_or(false);
                if !proceed {
                    println!("Aborted. Run `linix plan` to capture a fresh plan.");
                    return Ok(());
                }
            }
        }
    }

    if app.config.dry_run {
        println!(
            "[DRY-RUN] would install {} and remove {} package(s).",
            plan.installs.len(),
            plan.removals.len()
        );
        return Ok(());
    }

    // Optional interactive review: the same toggle screen as `sync`/`rollback`, so a captured
    // plan can still be trimmed at apply time. Skipped with --yes or without a terminal.
    let mut installs = plan.installs.clone();
    let mut removals = plan.removals.clone();
    if !yes && !app.config.yes {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            let changes = saved_plan_to_changes(&installs, &removals);
            let mut preview = TuiPreview::new(&changes, HashMap::new());
            if !preview.run()? {
                println!("Apply cancelled.");
                return Ok(());
            }
            let (keep_installs, keep_removes) = surviving_keys(&preview.get_filtered_changes());
            installs.retain(|s| keep_installs.contains(&format!("{}:{}", s.backend, s.name)));
            removals.retain(|r| keep_removes.contains(&format!("{}:{}", r.backend, r.name)));
            if installs.is_empty() && removals.is_empty() {
                println!("All changes deselected — nothing to apply.");
                return Ok(());
            }
        }
    }

    // `apply` executes its removals directly rather than through SyncEngine::sync, so it
    // needs its own call to the same guard. Placed after the interactive trim, so
    // deselecting the dangerous removals clears the guard honestly.
    let removal_pairs: Vec<(String, String)> = removals
        .iter()
        .map(|r| (r.backend.clone(), r.name.clone()))
        .collect();
    linix::app::sync::guard::enforce(
        &app.config,
        &app.registry,
        &removal_pairs,
        linix::app::sync::guard::GuardScope::Apply,
    )
    .await?;
    linix::app::sync::guard::enforce_installs(
        &app.config,
        installs.len(),
        linix::app::sync::guard::GuardScope::Apply,
    )
    .await?;

    let session_active = app.state.lock().await.active_session_id.is_some();
    let mut installed = 0usize;
    let mut removed = 0usize;

    for spec in &installs {
        let Some(b) = app.registry.get(&spec.backend) else {
            warn!(
                "apply: backend '{}' unavailable — skipping {}",
                spec.backend, spec.name
            );
            continue;
        };
        if let Some(inst) = b.as_installable() {
            info!("apply: installing {}:{}", spec.backend, spec.name);
            if let Err(e) = inst
                .install(std::slice::from_ref(spec), b.sudo_for_write())
                .await
            {
                warn!(
                    "apply: install {}:{} failed: {}",
                    spec.backend, spec.name, e
                );
                continue;
            }
            let source = spec
                .options
                .get("__source")
                .cloned()
                .or_else(|| Some("plan".into()));
            app.state.lock().await.add(
                &spec.backend,
                &spec.name,
                None,
                spec.options.clone(),
                source,
                session_active,
            );
            installed += 1;
        }
    }

    for r in &removals {
        let Some(b) = app.registry.get(&r.backend) else {
            continue;
        };
        if let Some(inst) = b.as_installable() {
            info!("apply: removing {}:{}", r.backend, r.name);
            if let Err(e) = inst
                .remove(std::slice::from_ref(&r.name), b.sudo_for_write())
                .await
            {
                warn!("apply: remove {}:{} failed: {}", r.backend, r.name, e);
                continue;
            }
            app.state.lock().await.remove(&r.backend, &r.name);
            removed += 1;
        }
    }

    app.state.lock().await.save()?;
    println!(
        "Applied plan: {} installed, {} removed.",
        installed, removed
    );
    perform_maintenance(app).await
}

/// Build and write `locks/versions.json` from the current managed state (live installed versions
/// preferred, falling back to recorded state). Returns the number of versions pinned. Shared
/// by `linix lock` and `doctor --fix` (lockfile heal).
async fn build_and_write_locks(app: &App) -> Result<usize> {
    let mut locks = serde_json::Map::new();
    {
        let state = app.state.lock().await;
        for pkg in &state.packages {
            // Prefer the live installed version from the backend; fall back to recorded state.
            let version = match app
                .registry
                .get(&pkg.backend)
                .and_then(|b| b.as_queryable().cloned())
            {
                Some(q) => match q.info(&pkg.name).await {
                    Ok(Some(p)) => p.version.or_else(|| pkg.version.clone()),
                    _ => pkg.version.clone(),
                },
                None => pkg.version.clone(),
            };
            if let Some(v) = version {
                if !v.is_empty() && v != "unknown" {
                    locks.insert(
                        format!("{}:{}", pkg.backend, pkg.name),
                        serde_json::Value::String(v),
                    );
                }
            }
        }
    }
    let count = locks.len();
    let path = app.config.config_root().join("locks").join("versions.json");
    // The version pins live in the `locks/` directory (II.6) beside the hook and extras
    // ledgers — not a stray `locks.json` file beside that directory (the old layout).
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.ok();
    }
    let doc = serde_json::json!({ "locks": locks });
    tokio::fs::write(&path, serde_json::to_string_pretty(&doc)?)
        .await
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(count)
}

async fn handle_lock(app: &App) -> Result<()> {
    let count = build_and_write_locks(app).await?;
    info!(
        "Lock: pinned {} package version(s) to {}",
        count,
        app.config.config_root().join("locks").join("versions.json").display()
    );
    // II.12: `lock` is also how you approve hooks. Record the current hash of every hook so a
    // later change to any of them stops the next sync until it is re-approved here. "Hash
    // everything, including your own scripts" — one rule, no exceptions.
    let hooks = app.hooks.approve_all_hooks()?;
    if hooks > 0 {
        info!(
            "Lock: approved {} hook(s) at their current script hash ({}).",
            hooks,
            linix::core::hook_lock::HookLedger::path_in(
                &app.config.config_root().join("locks")
            )
            .display()
        );
    }
    Ok(())
}

async fn handle_update(app: &App) -> Result<()> {
    app.update().await.map_err(|e| e.into())
}

/// `unmanaged` — **what `adopt` would adopt** (II.8), which is the definition E6 asks for.
///
/// It used to answer a different question: every installed package LiNix does not manage,
/// dependency closure and all. On a stock Ubuntu that is ~476 packages where `adopt` takes
/// ~103 — so `unmanaged` and `adopt` disagreed by a factor of four about the same word, and
/// the number you read here was not the number `adopt` would act on. Same crawl, one answer.
async fn handle_unmanaged(app: &App) -> Result<()> {
    let found = app.migrator().discover().await?;

    if found.adopt.is_empty() {
        println!("Nothing to adopt: LiNix already manages everything you chose to install.");
    } else {
        println!(
            "{} package(s) `linix adopt` would take:\n",
            found.adopt.len()
        );
        println!("{:<15} PACKAGE", "BACKEND");
        for p in &found.adopt {
            println!("{:<15} {}", p.backend, p.name);
        }
        println!("\nThis is an estimate — each backend's answer came from:");
        for (backend, source) in &found.sources {
            println!("  {:<10} {}", backend, source);
        }
    }

    if !found.skipped.is_empty() {
        println!(
            "\n{} package(s) the OS reports as essential are left alone.",
            found.skipped.len()
        );
    }
    Ok(())
}

/// `check` (II.8): parse everything the active profiles reach and report errors, changing
/// nothing. Resolution is where every parse/validation error surfaces — a bad line, an
/// unknown option, a `use` cycle — so a clean resolve IS a clean parse; this just says so,
/// and prints the counts a reader wants before running `sync`.
async fn handle_check(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    // `check` claims to parse everything the active profiles reach, and a `schedule:` line is
    // only validated where it is provisioned — so a missing `cron`, or a `run` a timer may not
    // run, surfaced at sync time on a file `check` had already called clean.
    for (name, opts, origin) in state.schedules() {
        linix::model::schedule::schedule_config(name, opts, origin)?;
    }
    println!(
        "OK: everything the active profiles reach parses. {} present, {} absent, {} repo/shim/service/link/schedule line(s).",
        state.total_present(),
        state.absent().count(),
        state.extras.len()
    );
    if !state.lapsed.is_empty() {
        println!(
            "\n{} dated line(s) have lapsed and no longer count:",
            state.lapsed.len()
        );
        for (key, origin) in &state.lapsed {
            println!("  {} at {}", key, origin);
        }
    }
    Ok(())
}

/// `absent` (II.8): every `absent:` line in force, and the module it comes from — what LiNix
/// keeps OFF this machine, and where each rule is written. Read-only.
async fn handle_absent(app: &App) -> Result<()> {
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let state = resolver.resolve_model().await?;
    let mut absent: Vec<_> = state.absent().collect();
    if absent.is_empty() {
        println!("No `absent:` lines are in force.");
        return Ok(());
    }
    absent.sort_by(|a, b| (&a.backend, &a.name).cmp(&(&b.backend, &b.name)));
    println!(
        "{} `absent:` line(s) in force — kept off this machine:\n",
        absent.len()
    );
    println!("{:<15} {:<25} SOURCE", "BACKEND", "PACKAGE");
    for spec in absent {
        let source = spec
            .options
            .get("__source")
            .map(String::as_str)
            .unwrap_or("?");
        println!("{:<15} {:<25} {}", spec.backend, spec.name, source);
    }
    Ok(())
}

/// How little LiNix must manage before "delete the rest" reads as a mistake (II.11).
///
/// A ratio, not a count. On Alpine, `adopt` correctly took 14 packages and a mis-scoped
/// removal scheduled all 14 — under any sane count limit, none protected, all things you
/// would cry about. The count misses it on small machines. Manage a tenth of what you are
/// about to delete and you have made a mistake, on every machine, at every scale (V.20).
const PURGE_RATIO: f64 = 0.1;

/// `purge-unmanaged` (II.11): delete everything LiNix does not manage.
///
/// The residual risk, stated plainly because the docs must state it: `adopt` is an estimate.
/// If it missed something, this deletes it.
async fn handle_purge_unmanaged(app: &App, allow_mass_purge: bool) -> Result<()> {
    let unmanaged = app.installed_but_unmanaged().await?;
    if unmanaged.is_empty() {
        println!("Nothing to do: LiNix manages every installed package.");
        return Ok(());
    }

    let managed = app.state.lock().await.packages.len();
    let removals: Vec<(String, String)> = unmanaged
        .iter()
        .map(|p| (p.backend.clone(), p.name.clone()))
        .collect();

    // The whole list. 576 packages is 576 lines: the pain is the feature, and a summary
    // here is a summary of what you are about to lose.
    println!(
        "LiNix manages {} package(s). This will remove {}:\n",
        managed,
        unmanaged.len()
    );
    for p in &unmanaged {
        println!("  {}:{}", p.backend, p.name);
    }
    println!();

    // The ratio check, before anything else asks anything.
    let ratio = managed as f64 / unmanaged.len() as f64;
    if ratio < PURGE_RATIO && !allow_mass_purge {
        let sample: Vec<String> = unmanaged
            .iter()
            .take(3)
            .map(|p| p.name.clone())
            .collect();
        anyhow::bail!(
            "LiNix manages {} packages.\n\
             This will remove {}, including {}.\n\
             That looks like you haven't adopted this machine yet.\n\
             Run `linix adopt` first, or --allow-mass-purge if you're sure.",
            managed,
            unmanaged.len(),
            sample.join(", ")
        );
    }

    // `max_removals` does not apply: it catches accidents, and this is deliberate. Protection
    // and OS-essential still do — nothing overrides those (II.10, II.11).
    linix::app::sync::guard::enforce_deliberate(
        &app.config,
        &app.registry,
        &removals,
        linix::app::sync::guard::GuardScope::PurgeUnmanaged,
    )
    .await?;

    if app.config.dry_run {
        println!("[DRY-RUN] Nothing removed.");
        return Ok(());
    }

    // Snapshots first, automatically. If none can be taken, say so — "there is no undo for
    // this" is the most important sentence this command can print (II.11).
    let snapshot = match app.snapshot_manager.auto_snapshot(linix::core::snapshot::SnapshotLabel::PurgeUnmanaged).await {
        Ok(Some(snap)) => {
            println!("Snapshot taken: {}. That is your undo.\n", snap.id);
            Some(snap.id)
        }
        Ok(None) => {
            println!(
                "This cannot be undone.\n  \
                 This machine has no snapshot provider (btrfs, ZFS or Timeshift), so nothing \
                 removed here can be brought back.\n"
            );
            None
        }
        Err(e) => {
            println!(
                "This cannot be undone.\n  \
                 The snapshot failed ({}), so nothing removed here can be brought back.\n",
                e
            );
            None
        }
    };

    if !app.config.yes {
        let typed: String = dialoguer::Input::new()
            .with_prompt(format!(
                "Type the number of packages to remove ({}) to confirm",
                unmanaged.len()
            ))
            .allow_empty(true)
            .interact_text()?;
        if typed.trim() != unmanaged.len().to_string() {
            println!("Aborted. Nothing was removed.");
            return Ok(());
        }
    }

    let (mut gone, mut failed) = (0usize, 0usize);
    for (backend_name, name) in &removals {
        let Some(b) = app.registry.get(backend_name) else {
            continue;
        };
        let Some(inst) = b.as_installable() else {
            continue;
        };
        match inst
            .remove(std::slice::from_ref(name), b.sudo_for_write())
            .await
        {
            Ok(_) => gone += 1,
            Err(e) => {
                failed += 1;
                warn!("purge-unmanaged: {}:{} — {}", backend_name, name, e);
            }
        }
    }

    println!("\nRemoved {} package(s); {} failed.", gone, failed);
    if let Some(id) = &snapshot {
        println!("Undo with `linix undo {}`.", id);
    }
    Ok(())
}

/// Stop managing packages without uninstalling them.
///
/// This exists because deleting a manifest line means "uninstall this", not "stop managing
/// this" — so the obvious way to trim `migrate`'s output (keep 15 lines, delete 85) is in
/// fact an order to purge 85 packages. Forgetting has to be its own verb.
///
/// It drops the package from managed state AND from any manifest that declares it. Doing
/// only the first would be undone by the next `sync`, which would see the declaration and
/// re-adopt it.
async fn handle_unmanage(app: &App, packages: &[String], json: bool) -> Result<()> {
    let mut results = Vec::new();

    for spec in packages {
        let (backend, name) =
            linix::config::parser::split_removal_target(spec, |b| app.registry.get(b).is_some());

        // Forget every backend's copy when the target is unqualified, mirroring how
        // `remove` searches all backends for a bare name.
        let mut forgotten = Vec::new();
        {
            let mut state = app.state.lock().await;
            let managed: Vec<(String, String)> = state
                .packages
                .iter()
                .filter(|p| p.name == name)
                .filter(|p| backend.as_deref().is_none_or(|b| b == p.backend))
                .map(|p| (p.backend.clone(), p.name.clone()))
                .collect();
            for (b, n) in managed {
                if state.forget(&b, &n) {
                    forgotten.push(format!("{}:{}", b, n));
                }
            }
        }

        // The line goes too. `forget` means LiNix never touches it again, and a package
        // still declared is a package the next `sync` re-adopts — a command that silently
        // undoes itself.
        let dropped = app.undeclare(spec).await?;

        results.push(serde_json::json!({
            "package": spec,
            "forgotten": forgotten,
            "lines_removed": dropped
                .iter()
                .map(|e| serde_json::json!({
                    "file": e.file.display().to_string(),
                    "line": e.line,
                }))
                .collect::<Vec<_>>(),
            "still_installed": true,
        }));
    }

    app.state.lock().await.save()?;

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
        return Ok(());
    }

    for r in &results {
        let spec = r["package"].as_str().unwrap_or_default();
        let forgotten = r["forgotten"].as_array().map(|a| a.len()).unwrap_or(0);
        let lines = r["lines_removed"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        if forgotten == 0 && lines == 0 {
            println!(
                "{}: not managed and not declared — nothing to forget.",
                spec
            );
            continue;
        }
        println!(
            "{}: no longer managed by LiNix. It is still installed.",
            spec
        );
        for f in r["forgotten"].as_array().into_iter().flatten() {
            println!("  dropped from managed state: {}", f.as_str().unwrap_or(""));
        }
        for l in r["lines_removed"].as_array().into_iter().flatten() {
            println!(
                "  removed declaration `{}` from {}",
                l["line"].as_str().unwrap_or(""),
                l["file"].as_str().unwrap_or("")
            );
        }
    }
    Ok(())
}

/// Show what the removal guard will refuse to touch. The guard is only trustworthy if its
/// rules are inspectable, so this reports the effective rules — and, given package names,
/// answers the question people actually have ("will this be protected?") along with the
/// rule that decides it.
async fn handle_protected(app: &App, packages: &[String], json: bool) -> Result<()> {
    let cfg = &app.config;

    if !packages.is_empty() {
        // Query mode. This MUST reach the same answer as a real removal, so it calls the
        // guard's own decision function rather than re-implementing the rules — an
        // inspector that contradicts the enforcer is worse than none, because it is
        // believed. "backend:name" consults that backend's essential list; a bare name is
        // checked against the config rules only.
        let mut rows = Vec::new();
        for spec in packages {
            let (backend, name) = linix::config::parser::split_removal_target(spec, |b| {
                app.registry.get(b).is_some()
            });
            let backend = backend.unwrap_or_default();
            let os_essential = if backend.is_empty() {
                std::collections::HashSet::new()
            } else {
                let mut set = std::collections::HashSet::new();
                set.insert(backend.clone());
                linix::app::sync::guard::essential_names(&app.registry, &set).await
            };
            let (protected, reason) =
                match linix::app::sync::guard::protection_of(cfg, &backend, &name, &os_essential) {
                    Some(p) => (true, p.reason()),
                    None => match cfg.unprotect_rule(&name) {
                        Some(rule) => (
                            false,
                            format!("exempted by unprotected_packages rule `{}`", rule),
                        ),
                        None => (false, "no rule matches".into()),
                    },
                };
            rows.push((spec.clone(), protected, reason));
        }
        if json {
            let out: Vec<_> = rows
                .iter()
                .map(|(p, prot, why)| {
                    serde_json::json!({ "package": p, "protected": prot, "reason": why })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&out)?);
        } else {
            println!("{:<30} {:<10} REASON", "PACKAGE", "PROTECTED");
            for (p, prot, why) in rows {
                println!("{:<30} {:<10} {}", p, if prot { "yes" } else { "no" }, why);
            }
        }
        return Ok(());
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "protected_packages": cfg.guard.protected_packages,
                "unprotected_packages": cfg.guard.unprotected_packages,
                "max_removals": cfg.guard.max_removals,
            }))?
        );
        return Ok(());
    }

    println!("Removal guard — what LiNix refuses to remove.\n");
    println!("Protected packages ({}):", cfg.guard.protected_packages.len());
    for p in &cfg.guard.protected_packages {
        match p.strip_suffix('*') {
            Some(prefix) => println!("  {:<24} (any name starting with '{}')", p, prefix),
            None => println!("  {}", p),
        }
    }
    if cfg.guard.unprotected_packages.is_empty() {
        println!("\nExemptions: none.");
    } else {
        println!(
            "\nExemptions ({}) — these override the list above:",
            cfg.guard.unprotected_packages.len()
        );
        for p in &cfg.guard.unprotected_packages {
            println!("  {}", p);
        }
    }
    match cfg.guard.max_removals {
        0 => println!("\nMaximum removals in one command: unlimited (max_removals = 0)."),
        n => println!("\nMaximum removals in one command: {} (max_removals).", n),
    }

    println!(
        "\nPackages the OS itself reports as essential are also refused, on top of this list.\n\
         Every command that removes is guarded — there is no way to opt one out.\n\
         Edit `protected_packages`, `unprotected_packages` or `max_removals` under [guard] in {}.\n\
         Check one package:      linix protected apt:python3\n\
         Machine-readable:       linix protected --json\n\
         Allow a big removal:    linix <command> --allow-mass-removal (the count only —\n\
                                 it never lets a protected or essential package through)\n\
         Allow a big install:    linix <command> --allow-mass-install (answers `max_installs`,\n\
                                 off unless you set it)",
        cfg.preferences_file.display()
    );
    Ok(())
}

const CONFIG_TEMPLATE: &str = r#"# LiNix refusals and behaviour (preferences.toml). Nothing writes to this but you.
# Every key is optional; omit a key to use its built-in default.
#
# Where your repo lives is NOT a key here — this file is inside it. Use `linix path --set`.

# Maximum number of packages installed/removed (and searched) in parallel.
# Omit to auto-detect this machine's core count (respecting container CPU limits).
# max_parallel = 4

# Timeout (seconds) for outbound HTTP search requests (npm/PyPI/marketplace).
network_timeout_secs = 15

# Retention window for `nix-collect-garbage --delete-older-than` during cleanup.
nix_gc_age = "30d"

# Default SSH destinations for `linix fleet` when none are given on the command line.
# fleet_hosts = ["user@web-01", "user@web-02"]

# Which backends this host uses, and in what order, live in the `priority` file (II.6) —
# NOT here. One list, with `when` blocks for the per-host case.

# Per-backend settings. Example: install flatpaks into the user scope.
# [backend_settings.flatpak]
# user = "true"

# ---------------------------------------------------------------------------
# [guard] — the nine refusals (II.10). One table, one home.
#
# Drift removal is derived from managed state, and managed state can be wrong: a
# mis-scoped manifest, a bad `migrate`, or a state file from another machine can
# make hundreds of working packages look unwanted. The guard refuses those.
# Every rule here is a refusal, not a preference: `-y` cannot skip any of them.
# `linix protected` shows the effective rules.
# ---------------------------------------------------------------------------
[guard]

# Refuse any single command that removes more than this many packages.
# 0 disables the check entirely (not recommended).
max_removals = 20

# Refuse any single command that installs more than this many at once.
# 0 (the default) leaves it off — installs are additive and far less dangerous.
# max_installs = 500

# Names removal must never touch, ADDED to the built-in list (`linix protected`
# prints the full effective set). Matching is exact and case-insensitive, or a
# prefix if the entry ends in `*` — so `libpam*` covers libpam0g, while `libc`
# still does not cover `libc-bin`.
# protected_packages = ["steam", "nvidia-driver", "libfoo*"]

# Names that are NOT protected even if a built-in rule (or the OS's own
# "essential" flag) says otherwise. This wins over everything.
# unprotected_packages = ["python3-pip"]

# Never install these (matched case-insensitively).
# deny_packages = ["leftpad"]

# Refuse any package that lacks an explicit @version= (no floating installs).
# pinned_only = false

# Refuse to change anything unless a snapshot can be taken first.
# require_snapshot = false

# Refuse to apply when `linix audit` reports a managed package as vulnerable.
# deny_vulnerable = false
"#;

async fn handle_path(cli: &Cli, explain: bool, set: Option<&std::path::Path>) -> Result<()> {
    use linix::app::locate;

    if let Some(dir) = set {
        let written = locate::set_root(dir)?;
        println!("Config repo set to {}", dir.display());
        println!("Stored in {}", written.display());
        return Ok(());
    }

    let resolved = locate::locate(cli.config_dir.as_deref())?;
    println!("{}", locate::render_path(&resolved, explain));
    Ok(())
}

async fn handle_edit(cli: &Cli, file: Option<&str>) -> Result<()> {
    use linix::app::locate;

    let resolved = locate::locate(cli.config_dir.as_deref())?;
    let target = locate::resolve_target(&resolved.path, file)?;
    let editor = locate::editor_command();

    let is_preferences = target.file_name().and_then(|n| n.to_str())
        == Some(linix::config::PREFERENCES_FILE_NAME);
    if is_preferences && !target.exists() {
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&target, CONFIG_TEMPLATE).await?;
        println!("Created {} from the default template.", target.display());
    }

    let status = tokio::process::Command::new(&editor)
        .arg(&target)
        .status()
        .await
        .with_context(|| format!("launching editor '{}'", editor))?;

    if !status.success() {
        anyhow::bail!("editor '{}' exited abnormally.", editor);
    }

    // Catch a typo here rather than at the next run, when the command that fails is
    // unrelated to the edit that broke it.
    if is_preferences {
        let p = target.clone();
        match tokio::task::spawn_blocking(move || linix::config::Config::from_file(&p)).await? {
            Ok(_) => println!("Saved. {} parses cleanly.", target.display()),
            Err(e) => anyhow::bail!(
                "{} no longer parses ({}). Re-run `linix edit {}` to fix it.",
                target.display(),
                e,
                linix::config::PREFERENCES_FILE_NAME
            ),
        }
    }
    Ok(())
}

async fn handle_config(app: &App, cmd: &ConfigCommand) -> Result<()> {
    let path = app.config.preferences_file.clone();
    match cmd {
        ConfigCommand::Show => {
            let source = if path.exists() {
                format!("file: {}", path.display())
            } else {
                "built-in defaults".to_string()
            };
            println!("# source: {}", source);
            println!(
                "{}",
                toml::to_string_pretty(&*app.config).context("Failed to serialize config")?
            );
        }
        ConfigCommand::Init { force } => {
            if path.exists() && !force {
                warn!(
                    "Config already exists at {} (use --force to overwrite).",
                    path.display()
                );
                return Ok(());
            }
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            tokio::fs::write(&path, CONFIG_TEMPLATE)
                .await
                .with_context(|| format!("Failed to write config to {}", path.display()))?;
            info!("Wrote commented default preferences to {}", path.display());
        }
    }
    Ok(())
}
async fn handle_heal(app: &App) -> Result<()> {
    app.sync_engine().await.heal().await.map_err(|e| e.into())
}

/// Health-gated upgrade: snapshot, upgrade, run the test, roll back automatically on
/// failure so a bad upgrade never leaves the machine broken.
async fn handle_canary(app: &App, scope: Option<PlannerScope>, test: &Option<String>) -> Result<()> {
    let test = test
        .clone()
        .ok_or_else(|| anyhow::anyhow!("--canary requires --test <command> (the health check)"))?;
    if !app.snapshot_manager.has_provider() {
        return Err(anyhow::anyhow!(
            "--canary needs a snapshot provider (btrfs/zfs/timeshift/Windows Restore) to guarantee rollback; none is available"
        ));
    }

    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    let changes = {
        let state_guard = app.state.lock().await;
        let planner = linix::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        planner.plan(&desired, scope).await?
    };
    if changes.is_empty() {
        info!("nothing to upgrade.");
        return Ok(());
    }
    print_flight_plan(app, &changes);

    if app.config.dry_run {
        println!(
            "[DRY-RUN] Would snapshot, upgrade, run `{}`, and roll back on failure.",
            test
        );
        return Ok(());
    }

    let snap = app
        .snapshot_manager
        .auto_snapshot(linix::core::snapshot::SnapshotLabel::PreCanary)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to create pre-canary snapshot"))?;
    info!("snapshot {} taken; applying upgrade...", snap.id);
    app.sync_engine()
        .await
        .sync(changes, linix::app::sync::guard::GuardScope::Canary)
        .await?;

    info!("running health check: {}", test);
    if linix::app::bisect::run_test(&test).await {
        println!("Canary: health check passed — upgrade kept.");
        perform_maintenance(app).await
    } else {
        warn!(
            "health check FAILED — rolling back to snapshot {}...",
            snap.id
        );
        app.snapshot_manager.restore_snapshot(&snap.id).await?;
        println!(
            "Canary: rolled back to pre-upgrade snapshot {}. System left unchanged.",
            snap.id
        );
        Ok(())
    }
}

async fn handle_conflicts(app: &App, json: bool) -> Result<()> {
    use linix::app::conflicts::{detect_conflicts, ConflictKind};

    // Resolve the full desired state (all manifests/modules/groups), flatten to specs.
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let specs: Vec<linix::core::PackageSpec> = desired.into_values().flatten().collect();
    let conflicts = detect_conflicts(&specs);

    if json {
        println!("{}", serde_json::to_string_pretty(&conflicts)?);
        return Ok(());
    }

    if conflicts.is_empty() {
        println!(
            "No cross-backend conflicts detected across {} desired package(s).",
            specs.len()
        );
        return Ok(());
    }

    println!("Cross-backend conflicts ({}):", conflicts.len());
    for c in &conflicts {
        let label = match c.kind {
            ConflictKind::VersionMismatch => "VERSION MISMATCH",
            ConflictKind::MultipleProviders => "MULTIPLE PROVIDERS",
        };
        let providers = c
            .providers
            .iter()
            .map(|(b, v)| match v {
                Some(v) => format!("{}@{}", b, v),
                None => b.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        println!("  [{}] {} — provided by: {}", label, c.name, providers);
    }
    println!(
        "\nResolve by removing the duplicate from one backend, or pinning both to the same \
         version. (Shadowing means whichever is first on PATH wins.)"
    );
    Ok(())
}

async fn handle_hold(app: &App, packages: &[String]) -> Result<()> {
    if packages.is_empty() {
        let state = app.state.lock().await;
        let held = state.list_held();
        if held.is_empty() {
            println!("No packages are held.");
        } else {
            println!("Held packages ({}):", held.len());
            for h in held {
                println!("  {}", h);
            }
        }
        return Ok(());
    }
    let mut n = 0usize;
    {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.hold(p) {
                n += 1;
            }
        }
        state.save()?;
    }
    println!(
        "Held {} package(s). `linix upgrade` will skip them until `linix unhold`.",
        n
    );
    Ok(())
}

async fn handle_unhold(app: &App, packages: &[String]) -> Result<()> {
    let mut n = 0usize;
    {
        let mut state = app.state.lock().await;
        for p in packages {
            if state.unhold(p) {
                n += 1;
            }
        }
        state.save()?;
    }
    println!("Released {} hold(s).", n);
    Ok(())
}

/// Enforce the `[guard]` install/change rules against the desired state before any change
/// (II.10). The spec-level rules (`deny_packages`, `pinned_only`) are checked purely by the
/// guard; the two that need runtime state (`require_snapshot`, `deny_vulnerable`) are checked
/// here, where the snapshot provider and the audit report are in hand. All nine refusals now
/// share one decision surface — this replaces the old parallel `policy.toml` gate (II.17).
async fn enforce_policy(
    app: &App,
    desired: &HashMap<String, Vec<linix::core::PackageSpec>>,
) -> Result<()> {
    let guard = &app.config.guard;
    if guard.is_empty() {
        return Ok(());
    }
    let mut violations: Vec<String> = linix::app::sync::guard::inspect_desired(guard, desired)
        .iter()
        .map(linix::app::sync::guard::describe_objection)
        .collect();
    if guard.require_snapshot && !app.snapshot_manager.has_provider() {
        violations.push(
            "requires a snapshot provider but none is available (require_snapshot)".into(),
        );
    }
    if guard.deny_vulnerable {
        match linix::app::insight::audit(app).await {
            Ok(report) => {
                for f in report.findings {
                    violations.push(format!(
                        "{}:{} — known vulnerability {} (deny_vulnerable)",
                        f.backend, f.name, f.id
                    ));
                }
            }
            Err(e) => warn!("vulnerability check skipped ({}).", e),
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    eprintln!("Blocked by [guard] ({} violation(s)):", violations.len());
    for v in &violations {
        eprintln!("  - {}", v);
    }
    Err(anyhow::anyhow!(
        "guard rules prevent this operation; nothing was changed"
    ))
}

/// A concise pre-flight summary of what a sync/upgrade is about to do. Real download-size
/// and time estimates are backend-specific and deliberately not faked.
fn print_flight_plan(app: &App, changes: &linix::app::sync::planner::SyncChanges) {
    if app.config.quiet {
        return;
    }
    let report = changes.generate_report();
    if report.install.is_empty() && report.remove.is_empty() {
        return;
    }
    let mut backends: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut needs_root = false;
    let mut service_ops = 0;
    for e in report.install.iter().chain(report.remove.iter()) {
        backends.insert(e.backend.clone());
        if let Some(b) = app.registry.get(&e.backend) {
            if b.needs_root() {
                needs_root = true;
            }
        }
        if e.backend == "service" {
            service_ops += 1;
        }
    }
    println!("Planned changes:");
    println!(
        "  install {}   remove {}   (total {} change(s))",
        report.install.len(),
        report.remove.len(),
        report.install.len() + report.remove.len()
    );
    println!(
        "  backends: {}",
        backends.into_iter().collect::<Vec<_>>().join(", ")
    );
    if needs_root {
        println!("  privileges: some operations require root/sudo");
    }
    if service_ops > 0 {
        println!(
            "  services: {} change(s) may restart running services",
            service_ops
        );
    }
}

/// `linix policy` — report whether the desired state complies with the `[guard]` rules.
async fn handle_policy(app: &App) -> Result<()> {
    let guard = &app.config.guard;
    if guard.is_empty() {
        println!("No [guard] install/change rules are set — nothing to check.");
        return Ok(());
    }
    let resolver =
        linix::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    let mut violations: Vec<String> = linix::app::sync::guard::inspect_desired(guard, &desired)
        .iter()
        .map(linix::app::sync::guard::describe_objection)
        .collect();
    if guard.require_snapshot && !app.snapshot_manager.has_provider() {
        violations.push("requires a snapshot provider but none is available".into());
    }
    if violations.is_empty() {
        println!("[guard] check passed — the desired state is compliant.");
        if guard.deny_vulnerable {
            println!("(deny_vulnerable is also enforced at sync time via `linix audit`.)");
        }
    } else {
        println!("[guard] violations ({}):", violations.len());
        for v in &violations {
            println!("  - {}", v);
        }
    }
    Ok(())
}

async fn handle_audit(app: &App, json: bool) -> Result<()> {
    let report = linix::app::insight::audit(app).await?;
    linix::app::insight::print_audit(&report, json).map_err(|e| e.into())
}

async fn handle_sbom(app: &App) -> Result<()> {
    println!("{}", linix::app::insight::sbom(app).await?);
    Ok(())
}

async fn handle_export(
    app: &App,
    format: Option<&str>,
    out: &str,
    stdout: bool,
    force: bool,
) -> Result<()> {
    use linix::app::export::{export, Format, Outcome};
    let fmt = match format {
        Some(s) => Some(
            Format::parse(s)
                .with_context(|| format!("unknown export format '{}' (brew|pip|npm|apt)", s))?,
        ),
        None => None,
    };
    if stdout && fmt.is_none() {
        anyhow::bail!("--stdout needs a single --format (brew|pip|npm|apt).");
    }
    let out_dir = std::path::PathBuf::from(out);
    let results = export(app, fmt, &out_dir, stdout, force, app.config.dry_run).await?;
    for (file, outcome) in &results {
        match outcome {
            Outcome::NoPackages => println!("  skipped {} (no matching packages)", file),
            Outcome::Wrote(path) => println!("  wrote   {}", path.display()),
            Outcome::WouldWrite(path) => {
                println!("  [DRY-RUN] would write {}", path.display())
            }
            Outcome::WroteBeside { taken, renamed } => {
                println!("  wrote   {}", renamed.display());
                println!(
                    "          ({} already exists and was left alone; re-run with --force to replace it)",
                    taken.display()
                );
            }
        }
    }
    Ok(())
}

async fn handle_bundle(app: &App, out: &str, artifacts: bool, archive: bool) -> Result<()> {
    let out_path = std::path::PathBuf::from(out);

    // Freeze a plan so the target can review/apply it offline. Computed up front so it can be
    // written into the bundle (and captured inside the archive) by create_bundle.
    let plan_json = match compute_full_changes(app).await {
        Ok(changes) => {
            let plan = linix::app::sync::SavedPlan::from_changes(
                &changes,
                Some(chrono::Utc::now().timestamp()),
            );
            Some(serde_json::to_string_pretty(&plan)?)
        }
        Err(_) => None,
    };

    let report =
        linix::app::bundle::create_bundle(app, &out_path, artifacts, archive, plan_json.as_deref())
            .await?;

    println!(
        "Bundle written to {} — {} config file(s), {} package(s).",
        report.out.display(),
        report.files_copied,
        report.package_count
    );
    // Honest per-part reporting: say plainly what did and did NOT make it into the bundle.
    println!(
        "  manifest history (git bundle): {}",
        if report.git_history_included {
            "included (config.bundle) — `git clone` it to roll back to any past commit"
        } else {
            "NOT included — the config is not a git repo (or has no commits); run `linix git init`"
        }
    );
    println!(
        "  ownership registry (registry.json): {}",
        if report.registry_included {
            "included"
        } else {
            "NOT included — none found"
        }
    );
    if artifacts {
        println!(
            "Artifacts: {} fetched, {} skipped.",
            report.artifacts_fetched.len(),
            report.artifacts_skipped.len()
        );
        // Honest reporting: never let a skipped backend read as "bundled everything".
        for s in &report.artifacts_skipped {
            println!("  skipped {}", s);
        }
    }
    if let Some((path, size)) = &report.archive {
        println!(
            "Archive: {} ({:.1} KiB) — copy this one file to an air-gapped host.",
            path.display(),
            *size as f64 / 1024.0
        );
    }
    println!(
        "See {}/RESTORE.md for offline restore steps.",
        report.out.display()
    );
    Ok(())
}

async fn handle_why(app: &App, package: &str, json: bool) -> Result<()> {
    linix::app::insight::why(app, package, json)
        .await
        .map_err(|e| e.into())
}

/// Scaffold the on-disk layout LiNix expects and drop a starter manifest so a fresh
/// machine (or a freshly-cloned checkout) is immediately usable.
async fn handle_init(app: &App, force: bool, interactive: bool) -> Result<()> {
    let cfg = &app.config;
    scaffold_dirs(cfg).await?;

    if interactive {
        return interactive_init(app, force).await;
    }

    scaffold_repo(app, force).await?;

    println!("(Run `linix config init` to also write a commented config.toml, or `linix init -i` for guided setup.)");
    Ok(())
}

/// Create every on-disk directory LiNix expects. Idempotent.
async fn scaffold_dirs(cfg: &linix::config::Config) -> Result<()> {
    let layout = cfg.layout();
    let modules = layout.modules_dir();
    let profiles = layout.profiles_dir();
    let locks = layout.locks_dir();
    let dirs: [(&str, &std::path::Path); 7] = [
        ("modules", &modules),
        ("profiles", &profiles),
        ("locks", &locks),
        ("tmp", &cfg.tmp_dir),
        ("github", &cfg.github_dir),
        ("web", &cfg.web_dir),
        ("appimages", &cfg.appimage_dir),
    ];
    println!("Scaffolding LiNix directories:");
    for (label, path) in dirs {
        tokio::fs::create_dir_all(path)
            .await
            .with_context(|| format!("Failed to create {} directory {}", label, path.display()))?;
        println!("  created  {:<10} {}", label, path.display());
    }
    Ok(())
}

/// The answers guided setup gathers.
///
/// Deliberately short. Almost every question the old wizard asked has stopped existing:
/// "should sync remove drift?" (sync is drift removal — V.34), "how aggressive?" (the
/// aggressive answer is `purge-unmanaged`, a command, not a mode — V.21), "protect
/// imperative installs?" (they have a line now, so they are declared like everything else),
/// "preferred default backend?" (that is `priority`, generated from what this machine has —
/// V.15). A question whose answer LiNix can work out, or which no longer means anything, is
/// homework (V.41).
#[derive(Debug, Clone, Default)]
struct InitAnswers {
    snapshot_count: Option<u32>,
    starter_packages: Vec<String>,
}

/// Pure: layer the interactive answers onto a base config. No I/O, so it can be tested.
fn apply_init_answers(mut base: linix::config::Config, a: &InitAnswers) -> linix::config::Config {
    if let Some(n) = a.snapshot_count {
        // One dial, not two: `keep_last = 0` is how a user says "keep everything", so an
        // `auto_prune` switch beside it was a second way to answer the same question.
        base.retention.snapshots.keep_last = n as usize;
    }
    base
}

/// Guided setup: write the II.1 repo, then ask the few things LiNix genuinely cannot work
/// out. Refuses to run without a TTY so CI falls back to `linix init` instead of hanging.
async fn interactive_init(app: &App, force: bool) -> Result<()> {
    use dialoguer::Input;
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "`init -i` is interactive but stdin is not a terminal. \
             Run `linix init` (non-interactive) or `linix config init` instead."
        );
    }

    let config_path = app.config.preferences_file.clone();
    if config_path.exists() && !force {
        anyhow::bail!(
            "Config already exists at {}. Re-run `linix init -i --force` to overwrite it.",
            config_path.display()
        );
    }

    println!("\nLet's set up LiNix. Press Enter to accept the [default].\n");

    let defaults = linix::config::Config::default();
    let mut answers = InitAnswers::default();

    let keep: String = Input::new()
        .with_prompt("How many system snapshots to keep (0 keeps every one)")
        .default(defaults.snapshot_retention().keep_last.to_string())
        .interact_text()?;
    answers.snapshot_count = keep.trim().parse::<u32>().ok();

    let starter: String = Input::new()
        .with_prompt("Packages to start with (comma-separated, blank to skip)")
        .allow_empty(true)
        .interact_text()?;
    answers.starter_packages = starter
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut new_cfg = apply_init_answers(defaults, &answers);
    new_cfg.preferences_file = config_path.clone();
    new_cfg.config_root = app.config.config_root();

    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let body = toml::to_string_pretty(&new_cfg).context("Failed to serialize config")?;
    tokio::fs::write(&config_path, body)
        .await
        .with_context(|| format!("Failed to write config to {}", config_path.display()))?;
    println!("\n  wrote    config     {}", config_path.display());

    scaffold_repo(app, force).await?;

    // Starter packages go through the same door as `linix install`: one writer, so what a
    // wizard produces and what a command produces cannot be different shapes.
    for pkg in &answers.starter_packages {
        app.declare(pkg, None, linix::model::Landing::Imperative)
            .await?;
    }
    if !answers.starter_packages.is_empty() {
        println!(
            "\nRun `linix sync` to install {}.",
            answers.starter_packages.join(", ")
        );
    }
    Ok(())
}

/// Render a package as one aligned row: backend, name, version.
fn print_package_row(p: &linix::core::Package) {
    println!(
        "{:<12} {:<32} {}",
        p.backend,
        p.name,
        p.version.as_deref().unwrap_or("")
    );
}

async fn handle_search(app: &App, query: &str, json: bool, installed: bool) -> Result<()> {
    let mut results = app.search(query).await?;
    if installed {
        // Keep only results LiNix already manages, so `search --installed foo` answers
        // "which of my packages match" without a second command.
        let managed: std::collections::HashSet<(String, String)> = {
            let state = app.state.lock().await;
            state
                .packages
                .iter()
                .map(|p| (p.backend.clone(), p.name.clone()))
                .collect()
        };
        results.retain(|p| managed.contains(&(p.backend.clone(), p.name.clone())));
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        if results.is_empty() && installed {
            println!("No installed package matches '{}'.", query);
        }
        for p in &results {
            print_package_row(p);
        }
    }
    Ok(())
}

/// One outdated package: what's installed now vs the newest the backend offers.
#[derive(serde::Serialize)]
struct Outdated {
    backend: String,
    name: String,
    installed: String,
    latest: String,
}

/// Find managed packages whose backend reports a newer version than what's installed. Backends
/// without a `Searchable` capability (no "latest" source) are honestly skipped, not guessed at.
async fn compute_outdated(app: &App, list: &[linix::core::Package]) -> Vec<Outdated> {
    use version_compare::{compare, Cmp};
    let mut out = Vec::new();
    for p in list {
        let Some(cur) = p.version.as_deref() else {
            continue;
        };
        let Some(b) = app.registry.get(&p.backend) else {
            continue;
        };
        let Some(s) = b.as_searchable() else {
            continue;
        };
        let Ok(Some(remote)) = s.remote_info(&p.name).await else {
            continue;
        };
        let Some(latest) = remote.version.as_deref() else {
            continue;
        };
        // A newer remote version than installed → outdated. Unparseable versions compare
        // unequal safely and are simply not reported.
        if compare(latest, cur) == Ok(Cmp::Gt) {
            out.push(Outdated {
                backend: p.backend.clone(),
                name: p.name.clone(),
                installed: cur.to_string(),
                latest: latest.to_string(),
            });
        }
    }
    out
}

async fn handle_list(app: &App, backend: Option<&str>, json: bool, outdated: bool) -> Result<()> {
    let list = app.list(backend).await?;
    if outdated {
        let rows = compute_outdated(app, &list).await;
        if json {
            println!("{}", serde_json::to_string_pretty(&rows)?);
        } else if rows.is_empty() {
            println!("Everything is up to date (for backends that report a latest version).");
        } else {
            println!(
                "{:<12} {:<32} {:<18} LATEST",
                "BACKEND", "PACKAGE", "INSTALLED"
            );
            for r in &rows {
                println!(
                    "{:<12} {:<32} {:<18} {}",
                    r.backend, r.name, r.installed, r.latest
                );
            }
            println!("\nUpgrade all: `linix upgrade --all`  ·  one: `linix upgrade <name>`");
        }
        return Ok(());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else {
        for p in &list {
            print_package_row(p);
        }
    }
    Ok(())
}

async fn handle_info(app: &App, package: &str) -> Result<()> {
    let Some(p) = app.get_info(package).await? else {
        println!("Package '{}' not found in any available backend.", package);
        return Ok(());
    };

    println!("{:<14} {}", "Package:", p.name);
    println!("{:<14} {}", "Backend:", p.backend);
    if let Some(v) = &p.version {
        println!("{:<14} {}", "Version:", v);
    }
    if let Some(d) = p.properties.get("description") {
        println!("{:<14} {}", "Description:", d);
    }
    if let Some(path) = p
        .properties
        .get("install_path")
        .or_else(|| p.properties.get("bin_path"))
    {
        println!("{:<14} {}", "Install path:", path);
    }
    // Any remaining properties, surfaced rather than hidden.
    for (k, v) in &p.properties {
        if matches!(k.as_str(), "description" | "install_path" | "bin_path") {
            continue;
        }
        let label = format!("{}:", k.replace('_', " "));
        println!("{:<14} {}", label, v);
    }
    // Dependencies via the backend's MetadataProvider, if it has one.
    if let Some(b) = app.registry.get(&p.backend) {
        if let Some(mp) = b.as_metadata_provider() {
            if let Ok(deps) = mp.get_dependencies(&p.name).await {
                if !deps.is_empty() {
                    println!("{:<14} {}", "Dependencies:", deps.join(", "));
                }
            }
        }
    }
    Ok(())
}
/// Short label for a health status (human output).
fn status_label(s: linix::core::HealthStatus) -> &'static str {
    use linix::core::HealthStatus::*;
    match s {
        Ok => "OK",
        Degraded => "WARN",
        Critical => "FAIL",
    }
}

/// The status label, colored for a terminal (green/yellow/red) and plain otherwise / under
/// NO_COLOR. Centralizing color here keeps the doctor output readable without a color crate.
fn status_label_colored(s: linix::core::HealthStatus) -> String {
    use linix::core::HealthStatus::*;
    use linix::utils::style::{color_enabled, paint, GREEN, RED, YELLOW};
    let code = match s {
        Ok => GREEN,
        Degraded => YELLOW,
        Critical => RED,
    };
    paint(color_enabled(), code, status_label(s))
}

/// Count backends by status. Pure — unit tested.
fn doctor_tally(reports: &[(String, linix::core::HealthReport)]) -> (usize, usize, usize) {
    use linix::core::HealthStatus::*;
    let mut ok = 0;
    let mut degraded = 0;
    let mut critical = 0;
    for (_, r) in reports {
        match r.status {
            Ok => ok += 1,
            Degraded => degraded += 1,
            Critical => critical += 1,
        }
    }
    (ok, degraded, critical)
}

async fn handle_doctor(app: &App, fix: bool, json: bool) -> Result<()> {
    use linix::core::{HealthReport, HealthStatus};

    // ---- Per-backend health, via each backend's own probe (not a shallow is_available). ----
    let mut reports: Vec<(String, HealthReport)> = Vec::new();
    for b in app.registry.all() {
        let report = match b.core().check_health().await {
            Ok(r) => r,
            Err(e) => HealthReport {
                status: HealthStatus::Critical,
                message: Some(format!("health probe errored: {}", e)),
            },
        };
        reports.push((b.name().to_string(), report));
    }

    // ---- System-level checks + optional repair. ----
    let mut system: Vec<(String, HealthStatus, Option<String>)> = Vec::new();
    let mut fixes: Vec<String> = Vec::new();

    for (label, dir) in [
        ("config root", app.config.config_root()),
        ("modules dir", app.config.config_root().join("modules")),
        ("profiles dir", app.config.config_root().join("profiles")),
    ] {
        if dir.exists() {
            system.push((label.into(), HealthStatus::Ok, None));
        } else if fix {
            match tokio::fs::create_dir_all(&dir).await {
                Ok(_) => {
                    fixes.push(format!("created {}", dir.display()));
                    system.push((label.into(), HealthStatus::Ok, Some("created".into())));
                }
                Err(e) => system.push((
                    label.into(),
                    HealthStatus::Critical,
                    Some(format!("missing; create failed: {}", e)),
                )),
            }
        } else {
            system.push((
                label.into(),
                HealthStatus::Degraded,
                Some(format!("missing: {} (run `doctor --fix`)", dir.display())),
            ));
        }
    }

    // ---- Lockfile integrity: does locks/versions.json still match the managed set? ----
    {
        let lock_path = app.config.config_root().join("locks").join("versions.json");
        if !lock_path.exists() {
            system.push((
                "lockfile".into(),
                HealthStatus::Ok,
                Some("none yet (run `linix lock` to pin versions)".into()),
            ));
        } else {
            let managed: std::collections::HashSet<String> = {
                let state = app.state.lock().await;
                state
                    .packages
                    .iter()
                    .map(|p| format!("{}:{}", p.backend, p.name))
                    .collect()
            };
            let locked_keys: std::collections::HashSet<String> =
                match tokio::fs::read_to_string(&lock_path).await {
                    Ok(data) => serde_json::from_str::<serde_json::Value>(&data)
                        .ok()
                        .and_then(|v| {
                            v.get("locks")
                                .and_then(|l| l.as_object())
                                .map(|o| o.keys().cloned().collect())
                        })
                        .unwrap_or_default(),
                    Err(_) => std::collections::HashSet::new(),
                };
            let missing = managed.difference(&locked_keys).count();
            let stale = locked_keys.difference(&managed).count();
            if missing == 0 && stale == 0 {
                system.push(("lockfile".into(), HealthStatus::Ok, None));
            } else if fix {
                match build_and_write_locks(app).await {
                    Ok(n) => {
                        fixes.push(format!("reconciled locks/versions.json ({} entries)", n));
                        system.push((
                            "lockfile".into(),
                            HealthStatus::Ok,
                            Some("reconciled".into()),
                        ));
                    }
                    Err(e) => system.push((
                        "lockfile".into(),
                        HealthStatus::Degraded,
                        Some(format!("drifted; heal failed: {}", e)),
                    )),
                }
            } else {
                system.push((
                    "lockfile".into(),
                    HealthStatus::Degraded,
                    Some(format!(
                        "drifted: {} unpinned / {} stale (run `doctor --fix` or `linix lock`)",
                        missing, stale
                    )),
                ));
            }
        }
    }

    let (ok, degraded, critical) = doctor_tally(&reports);
    if ok == 0 {
        system.push((
            "package managers".into(),
            HealthStatus::Critical,
            Some("no usable backend detected on this host".into()),
        ));
    }

    if fix {
        // Best-effort metadata refresh so a "degraded, stale index" backend recovers.
        if app.update().await.is_ok() {
            fixes.push("refreshed backend metadata".into());
        }
    }

    // ---- Output ----
    if json {
        let backends: Vec<_> = reports
            .iter()
            .map(|(n, r)| serde_json::json!({ "backend": n, "status": r.status, "message": r.message }))
            .collect();
        let sys: Vec<_> = system
            .iter()
            .map(|(n, s, m)| serde_json::json!({ "check": n, "status": s, "message": m }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "backends": backends,
                "system": sys,
                "summary": { "ok": ok, "degraded": degraded, "critical": critical },
                "fixes_applied": fixes,
            }))?
        );
        return Ok(());
    }

    println!(
        "Backends: {} OK, {} degraded, {} critical (of {} total).",
        ok,
        degraded,
        critical,
        reports.len()
    );
    // Readiness roster: one `[READY] <backend>` line per healthy backend, printed at column 0
    // (unindented, uncolored) so it is both human-readable AND machine-greppable —
    // `linix doctor | grep '^\[READY\]'` enumerates every usable backend on this host. Without
    // this, a healthy `doctor` printed nothing about which package managers actually work.
    for (name, r) in &reports {
        if r.status == HealthStatus::Ok {
            println!("[READY] {}", name);
        }
    }
    // Then surface only the backends that need attention — a long OK list here would be noise.
    for (name, r) in &reports {
        if r.status != HealthStatus::Ok {
            println!(
                "  [{}] {}{}",
                status_label_colored(r.status),
                name,
                r.message
                    .as_deref()
                    .map(|m| format!(" — {}", m))
                    .unwrap_or_default()
            );
        }
    }

    println!("\nSystem:");
    for (name, s, m) in &system {
        println!(
            "  [{}] {}{}",
            status_label_colored(*s),
            name,
            m.as_deref()
                .map(|m| format!(" — {}", m))
                .unwrap_or_default()
        );
    }

    if !fixes.is_empty() {
        println!("\nRepairs applied:");
        for f in &fixes {
            println!("  + {}", f);
        }
    }

    let sys_critical = system.iter().any(|(_, s, _)| *s == HealthStatus::Critical);
    if critical > 0 || sys_critical {
        println!("\nSome checks are CRITICAL. Install the missing tools or re-run with --fix.");
    } else if degraded > 0 {
        println!("\nAll critical checks pass; some backends are degraded (see WARN above).");
    } else {
        println!("\nAll checks pass. System is healthy.");
    }
    Ok(())
}

// ============================================================================
// KERNEL HELPERS
// ============================================================================

async fn attempt_shim_hijack() -> Result<Option<Result<()>>> {
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

async fn load_and_merge_config(cli: &Cli) -> Result<linix::config::Config> {
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
        Some(cli.verbose),
        Some(cli.allow_mass_removal),
        Some(cli.allow_mass_install),
    )?;
    // --quiet has no config-file merge counterpart; apply it directly (a set flag wins).
    if cli.quiet {
        config.quiet = true;
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

async fn perform_maintenance(app: &App) -> Result<()> {
    app.journal.lock().await.cleanup()?;
    // Reclaim expired temporary installs so leases are enforced on every state-changing run.
    if let Err(e) = app.sweep_expired_leases().await {
        warn!("Maintenance: lease sweep failed: {}", e);
    }
    // Restore temporary uninstalls whose timer has elapsed (mirror of the lease sweep).
    if let Err(e) = app.sweep_due_suspensions().await {
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
mod doctor_tests {
    use super::*;
    use linix::core::{HealthReport, HealthStatus};

    fn rep(status: HealthStatus) -> HealthReport {
        HealthReport {
            status,
            message: None,
        }
    }

    #[test]
    fn tally_counts_each_status() {
        let reports = vec![
            ("apt".to_string(), rep(HealthStatus::Ok)),
            ("brew".to_string(), rep(HealthStatus::Ok)),
            ("snap".to_string(), rep(HealthStatus::Degraded)),
            ("nix".to_string(), rep(HealthStatus::Critical)),
        ];
        assert_eq!(doctor_tally(&reports), (2, 1, 1));
    }

    #[test]
    fn status_labels_are_stable() {
        assert_eq!(status_label(HealthStatus::Ok), "OK");
        assert_eq!(status_label(HealthStatus::Degraded), "WARN");
        assert_eq!(status_label(HealthStatus::Critical), "FAIL");
    }
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
}

#[cfg(test)]
mod init_tests {
    use super::*;

    #[test]
    fn answers_layer_onto_config() {
        let base = linix::config::Config::default();
        let answers = InitAnswers {
            snapshot_count: Some(42),
            starter_packages: vec![],
        };
        let cfg = apply_init_answers(base, &answers);
        assert_eq!(cfg.retention.snapshots.keep_last, 42);
    }

    #[test]
    fn omitted_snapshot_count_keeps_base_default() {
        let base = linix::config::Config::default();
        let base_count = base.retention.snapshots.keep_last;
        let answers = InitAnswers {
            snapshot_count: None,
            ..Default::default()
        };
        let cfg = apply_init_answers(base, &answers);
        assert_eq!(cfg.retention.snapshots.keep_last, base_count);
    }

    #[test]
    fn config_from_answers_round_trips_through_toml() {
        // The interactive config must serialize to valid TOML and load back identically —
        // otherwise `init -i` writes a file `linix` cannot read.
        let answers = InitAnswers {
            snapshot_count: Some(7),
            starter_packages: vec![],
        };
        let cfg = apply_init_answers(linix::config::Config::default(), &answers);
        let toml_str = toml::to_string_pretty(&cfg).expect("serializes");
        let back: linix::config::Config = toml::from_str(&toml_str).expect("parses back");
        assert_eq!(back.retention.snapshots.keep_last, 7);
    }

    #[test]
    fn config_template_actually_parses_and_matches_the_defaults() {
        // `linix config init` writes this file verbatim. A template that does not parse
        // hands every new user a broken config, and a template whose keys don't match the
        // struct silently documents settings that do nothing (as `cache_ttl` did).
        let cfg: linix::config::Config =
            toml::from_str(CONFIG_TEMPLATE).expect("CONFIG_TEMPLATE must be valid config.toml");
        assert_eq!(cfg.guard.max_removals, 20);
    }

    #[test]
    fn the_template_documents_no_setting_that_would_disarm_the_guard() {
        // Three of these used to be real, and each was a way to make a routine sync delete
        // something: `[guard.enforce_on]` switched the guard off per command,
        // `prune_scope = "system"` made sync remove software it never installed, and
        // `prune_on_sync` decided whether sync was sync at all. A config file is copied
        // between machines and pasted from the internet — it must not be able to say any
        // of this (V.21, V.34, II.17).
        for gone in [
            "enforce_on",
            "prune_on_sync",
            "prune_scope",
            "protect_imperative",
        ] {
            assert!(
                !CONFIG_TEMPLATE.contains(gone),
                "`{}` is deleted, but the template still offers it",
                gone
            );
        }
    }

}

/// Write the II.1 repo: `priority`, `active`, and a profile to hang things on.
///
/// `priority` is generated from what this machine actually has (V.41: LiNix should look, not
/// ask you to maintain a list by hand on every machine forever), ordered by the one rule
/// that decides anything — a system manager beats a language manager (V.14). The file says
/// why, because a default nobody can explain is a default nobody can safely change (P5).
async fn scaffold_repo(app: &App, force: bool) -> Result<()> {
    let layout = app.config.layout();

    let detected: Vec<String> = app
        .registry
        .available()
        .iter()
        .map(|b| b.name().to_string())
        .collect();
    let ordered = linix::model::priority::starter_order(&detected);

    let priority = layout.priority_file();
    if !priority.exists() || force {
        tokio::fs::write(&priority, linix::model::priority::starter_file(&ordered))
            .await
            .with_context(|| format!("Failed to write {}", priority.display()))?;
        println!(
            "  created  {:<10} {} ({})",
            "priority",
            priority.display(),
            if ordered.is_empty() {
                "no package managers detected — add yours by hand".to_string()
            } else {
                ordered.join(", ")
            }
        );
    } else {
        println!("  kept     {:<10} {}", "priority", priority.display());
    }

    // Something has to be active or nothing is: a module nothing reaches is inert (II.3).
    let profile = layout.profile_file("Main");
    if !profile.exists() || force {
        tokio::fs::write(
            &profile,
            "# What this machine is set to. Add `use <module>` lines, or packages directly.\n\
             #\n\
             # Profiles are Capitalized, modules are lowercase — so `(Work | gaming)` tells\n\
             # you what everything is without extra syntax.\n",
        )
        .await
        .with_context(|| format!("Failed to write {}", profile.display()))?;
        println!("  created  {:<10} {}", "profile", profile.display());
    }

    let active = layout.active_file();
    if !active.exists() || force {
        tokio::fs::write(&active, "Main\n")
            .await
            .with_context(|| format!("Failed to write {}", active.display()))?;
        println!("  created  {:<10} {}", "active", active.display());
    }

    println!("\nReady. `linix install jq` writes a line you own; `linix sync` makes it so.");
    Ok(())
}

#[cfg(test)]
mod purge_tests {
    /// The ratio, as `handle_purge_unmanaged` computes it.
    fn reads_as_a_mistake(managed: usize, to_remove: usize) -> bool {
        (managed as f64 / to_remove as f64) < super::PURGE_RATIO
    }

    #[test]
    fn manage_three_delete_576_is_a_mistake_at_any_scale() {
        // II.11's example, and V.20's rule: a count cannot catch this on a small machine.
        assert!(reads_as_a_mistake(3, 576));
    }

    #[test]
    fn the_ratio_catches_the_small_machine_a_count_misses() {
        // Alpine: adopt correctly took 14 packages, and a mis-scoped removal scheduled all
        // 14 — under any count limit, none protected, all things you would cry about.
        assert!(reads_as_a_mistake(1, 14));
        // And an adopted Alpine is fine: 14 managed, a handful of strays to clear.
        assert!(!reads_as_a_mistake(14, 20));
    }

    #[test]
    fn an_adopted_machine_may_purge_the_rest() {
        // Ubuntu after `adopt`: ~103 manual packages managed, the dependency closure and
        // whatever else is lying around unmanaged. That is the command working as intended.
        assert!(!reads_as_a_mistake(103, 476));
    }
}
