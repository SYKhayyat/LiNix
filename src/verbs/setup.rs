use crate::verbs::prelude::*;

/// `absent` (II.8): every `absent:` line in force, and the module it comes from — what LiNix
/// keeps OFF this machine, and where each rule is written. Read-only.
/// `vars` (Part IX, W12): the variables resolved on this machine, so a `when $name` block that
/// does not fire can be debugged by seeing the value the machine actually derived.
/// `linix add` — vendor someone else's modules into this repo (7/U14, XIII.14).
///
/// Fetch → plan → refuse-or-copy → optionally approve. The safety story is not the fetch; it is
/// that anything executable lands unapproved and II.12 holds it until `linix lock`. `--trust`
/// runs that lock in the same step; without it, the vendored code sits inert and reviewable.
pub(crate) async fn handle_add(app: &App, source: &str, trust: bool, force: bool) -> Result<()> {
    use linix::model::vendor::{self, Source, Vendored};

    let Some(src) = Source::classify(source) else {
        return Err(linix::core::Error::Validation(format!(
            "`{}` is not a source `add` understands. Use `github:owner/repo`, a git or https \
             URL, a raw file URL, or a path to a module file or repo on this machine.",
            source
        ))
        .into());
    };

    // Fetch into a throwaway directory. A git source is cloned shallow; a raw file is
    // downloaded; a local source is read in place. The temp dir is dropped (and deleted) when
    // this function returns, so nothing a fetch brought outlives the command except what was
    // deliberately copied into the repo.
    let scratch = tempfile::tempdir().map_err(linix::core::Error::from)?;
    let fetched: std::path::PathBuf = match &src {
        Source::Github { .. } | Source::Git(_) => {
            let url = src.clone_url().expect("a git source has a clone url");
            let dest = scratch.path().join("repo");
            info!("cloning {}...", src.label());
            app.executor
                .run(
                    "git",
                    &["clone", "--depth", "1", &url, &dest.to_string_lossy()],
                    false,
                )
                .await
                .map_err(|e| {
                    linix::core::Error::Other(format!("could not clone {}: {}", src.label(), e))
                })?;
            dest
        }
        Source::File(url) => {
            // A single raw module file. It lands under a synthetic `modules/` so the vendor
            // planner treats it as a module — a bare file URL is a module by intent.
            let name = url
                .rsplit('/')
                .next()
                .filter(|n| !n.is_empty())
                .unwrap_or("vendored.txt");
            let name = if name.ends_with(".txt") {
                name.to_string()
            } else {
                format!("{}.txt", name)
            };
            let dir = scratch.path().join("modules");
            std::fs::create_dir_all(&dir).map_err(linix::core::Error::from)?;
            info!("downloading {}...", url);
            let body = reqwest::get(url)
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| linix::core::Error::Other(format!("could not fetch {}: {}", url, e)))?
                .text()
                .await
                .map_err(|e| linix::core::Error::Other(format!("reading {}: {}", url, e)))?;
            std::fs::write(dir.join(name), body).map_err(linix::core::Error::from)?;
            scratch.path().to_path_buf()
        }
        Source::Local(p) => {
            if !p.exists() {
                return Err(linix::core::Error::Validation(format!(
                    "`{}` does not exist on this machine.",
                    p.display()
                ))
                .into());
            }
            p.clone()
        }
    };

    // Every file in the fetched tree, relative to its root.
    let files = collect_relative_files(&fetched);
    let root = app.config.config_root();
    let plan = vendor::plan(&files, &|rel| root.join(rel).exists());

    if plan.placements.is_empty() {
        println!(
            "{} has nothing to vendor — no `modules/`, `adapters/` or `scripts/` files found.",
            src.label()
        );
        return Ok(());
    }

    // U14: refuse a collision and name it, unless --force. Overwriting your own module with a
    // stranger's silently is the case this exists to prevent.
    if !plan.collisions.is_empty() && !force {
        let names: Vec<String> = plan
            .collisions
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        return Err(linix::core::Error::Refused(format!(
            "refusing to overwrite {} file(s) you already have: {}.\n  \
             Rename yours, or pass `--force` to replace them with {}'s.",
            names.len(),
            names.join(", "),
            src.label()
        ))
        .into());
    }

    // Copy. Each destination's parent is created; a placement is repo-relative and already
    // path-safe (the planner dropped any that escaped).
    let mut modules = 0usize;
    let mut code = 0usize;
    for pl in &plan.placements {
        let to = root.join(&pl.to);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(linix::core::Error::from)?;
        }
        std::fs::copy(fetched.join(&pl.from), &to)
            .map_err(|e| linix::core::Error::Io(format!("copying {}: {}", pl.to.display(), e)))?;
        if pl.kind == Vendored::Module {
            modules += 1;
        } else {
            code += 1;
        }
    }

    println!(
        "Vendored {} module(s) and {} code file(s) from {} into your repo.",
        modules,
        code,
        src.label()
    );
    println!("  Review the diff (`linix git status`), then `use` the module(s) by name.");

    // The supply-chain gate. Vendored code is unapproved by default and will not run until
    // `linix lock` — the review the default forces. `--trust` runs that lock now, for a source
    // the user has decided to trust.
    if code > 0 {
        if trust {
            let events = linix::app::events::EventHooks::load(&app.config);
            let _ = events.approve_all();
            let approved = app.hooks.approve_all_hooks().unwrap_or(0);
            approve_adapters(app).ok();
            approve_generate_commands(app).ok();
            approve_exec_scripts(app).await.ok();
            println!(
                "--trust: approved the vendored code ({} hook set(s) + adapters/exec/generate).",
                approved
            );
        } else {
            println!(
                "  {} file(s) it brought can run code and are UNAPPROVED — they will not run \
                 until you review them and `linix lock`.",
                code
            );
        }
    }
    Ok(())
}

/// Every file under `root`, as paths relative to `root`. Symlinks are not followed — a
/// stranger's symlink is a path-traversal vector, and `safe_relative` would reject its target
/// anyway, so not following it is the honest version of the same refusal.
pub(crate) fn collect_relative_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip the repo's own git directory and any nested VCS metadata.
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    if let Ok(rel) = path.strip_prefix(root) {
                        out.push(rel.to_path_buf());
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// `linix try` — rehearse this config on a clean machine (7h/U12).
///
/// Read-only on the host by construction: the config is mounted `:ro`, the container's LiNix
/// data lives in the container, and nothing here is consulted except the config's path. It is
/// therefore in `READ_ONLY_COMMANDS` and takes no data lock — a rehearsal has no business
/// blocking a real sync.
pub(crate) async fn handle_try(app: &App, image: Option<&str>) -> Result<()> {
    use linix::model::rehearsal::{self, Verdict};

    let present = |cmd: &str| app.executor.command_exists_sync(cmd);
    let Some(runtime) = rehearsal::pick_runtime(&present) else {
        return Err(linix::core::Error::Refused(rehearsal::no_runtime_refusal()).into());
    };

    let image = image.unwrap_or(rehearsal::DEFAULT_IMAGE);

    // Asked BEFORE the run: an image that is not there is the ordinary first-run case, and
    // `docker run` reports it as a pull failure — which reads as "your config is broken" when
    // the config has not been looked at yet.
    if !image_exists(app, runtime, image).await {
        return Err(
            linix::core::Error::Refused(rehearsal::missing_image_refusal(runtime, image)).into(),
        );
    }

    let root = app.config.config_root();
    let config_path = root.to_string_lossy().to_string();

    info!("rehearsing on `{}` via {}...", image, runtime);
    let argv = rehearsal::argv(runtime, image, &config_path);
    let (program, args) = argv.split_first().expect("an argv is never empty");

    let status = tokio::process::Command::new(program)
        .args(args)
        .status()
        .await
        .map_err(|e| linix::core::Error::Other(format!("could not run `{}`: {}", runtime, e)))?;

    match rehearsal::verdict(status.code()) {
        Verdict::Valid => {
            println!("the config resolves on a clean {} machine.", image);
            Ok(())
        }
        // The container already printed why on its own stderr; repeating it here would be
        // two accounts of one failure.
        Verdict::Rejected(_) => Err(linix::core::Error::Refused(format!(
            "this config did not survive a clean {} machine — the rehearsal's output above says \
             why. Nothing on this machine was touched.",
            image
        ))
        .into()),
    }
}

/// Whether the runtime has this image locally.
///
/// `run`, not `run_output`: the latter tolerates a non-zero exit on purpose (an empty result
/// is an answer for the reads it was built for), so it reports success for an image that does
/// not exist — and `try` then blamed the config for what was a missing image.
pub(crate) async fn image_exists(app: &App, runtime: &str, image: &str) -> bool {
    app.executor
        .run(runtime, &["image", "inspect", image], false)
        .await
        .is_ok()
}

pub(crate) const CONFIG_TEMPLATE: &str = r#"# LiNix refusals and behaviour (preferences.toml). Nothing writes to this but you.
# Every key is optional; omit a key to use its built-in default.
#
# Where your repo lives is NOT a key here — this file is inside it. Use `linix path --set`.

# Maximum number of packages installed/removed (and searched) in parallel.
# Omit to auto-detect this machine's core count (respecting container CPU limits).
# max_parallel = 4

# Timeout (seconds) for outbound HTTP search requests (npm/PyPI/marketplace).
network_timeout_secs = 15

# How long to wait out a remote rate limit (GitHub) before giving up and naming it.
# The wait happens while the data directory is locked, so a long one looks like a hang.
# Raise it for an unattended CI job that would rather wait than fail.
rate_limit_max_wait_secs = 30

# Retention window for `nix-collect-garbage --delete-older-than` during cleanup.
nix_gc_age = "30d"

# When a package installed by a DOWNLOAD backend (github:/web:/appimage:) is removed, also
# delete any cached copy of the fetched file. Download-backends only: on apt/dnf/pacman the
# manager owns its own cache and this does nothing.
# clean_cache_on_remove = false
# Extra directories to search when clean_cache_on_remove cleans up. $XDG_CACHE_HOME, ~/.cache
# and /var/cache are always searched; add anywhere else this machine keeps downloads.
# cache_dirs = ["/opt/downloads"]

# Default SSH destinations for `linix fleet` when none are given on the command line.
# fleet_hosts = ["user@web-01", "user@web-02"]

# Which backends this host uses, and in what order, live in the `priority` file (II.6) —
# NOT here. One list, with `when` blocks for the per-host case.

# Per-backend settings. Example: install flatpaks into the user scope.
# [backend_settings.flatpak]
# user = "true"

# ---------------------------------------------------------------------------
# [guard] — the ten refusals (II.10). One table, one home.
#
# Drift removal is derived from managed state, and managed state can be wrong: a
# mis-scoped manifest, a bad `adopt` run, or a state file from another machine can
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

# Refuse to put a downloaded file anywhere but the backend's own bin directory,
# whatever the `@bin` option or the URL's last path segment says. On by default.
# Off restores an unchecked join, where a copied install line can name your shell
# profile and have it replaced by a symlink to a download. Not one of the nine:
# it is checked where a backend deploys, the only place that destination exists.
# confine_bin = true

# Refuse to roll back to a commit git does not vouch for (II.13). Off by default:
# a fresh repo signs nothing, and a refusal that fires on every rollback out of
# the box gets turned off before it ever catches anything. Turn it on together
# with `git config commit.gpgsign true`.
# require_signed_history = false

# Commands a `schedules` entry may not run. Matched against the first word of a
# `run =` line, so `run = sync --locked` is untouched by anything below. Both
# shipped names remove declared software, and a timer runs with nobody there to
# read a refusal. Take a name out to permit it on this machine.
# never_unattended = ["rebuild", "purge-unmanaged"]
"#;

pub(crate) async fn handle_path(
    cli: &Cli,
    explain: bool,
    set: Option<&std::path::Path>,
) -> Result<()> {
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

pub(crate) async fn handle_edit(cli: &Cli, file: Option<&str>) -> Result<()> {
    use linix::app::locate;

    let resolved = locate::locate(cli.config_dir.as_deref())?;
    let target = locate::resolve_target(&resolved.path, file)?;
    let editor = locate::editor_command();

    let is_preferences =
        target.file_name().and_then(|n| n.to_str()) == Some(linix::config::PREFERENCES_FILE_NAME);
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

pub(crate) async fn handle_config(app: &App, cmd: &ConfigCommand) -> Result<()> {
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
            println!("Wrote commented default preferences to {}", path.display());
        }
    }
    Ok(())
}

pub(crate) async fn handle_heal(app: &App) -> Result<()> {
    app.sync_engine().await.heal().await?;
    // U9: `check` looks, `heal` acts. These three repairs used to be `doctor --fix`, which
    // made one command both the diagnosis and the treatment — and a command that changes
    // things is one you cannot run to find out whether you want things changed.
    for fixed in repair_environment(app).await {
        println!("repaired: {}", fixed);
    }
    Ok(())
}

/// Put right what `check` can only report: the II.1 directories, the version lockfile, and a
/// stale backend index. Each is best-effort and reported by name — a repair that failed must
/// not stop the ones after it, and a repair nobody sees is the class of silence P3 forbids.
pub(crate) async fn repair_environment(app: &App) -> Vec<String> {
    let mut fixed = Vec::new();

    for dir in [
        app.config.config_root(),
        app.config.config_root().join("modules"),
        app.config.config_root().join("profiles"),
    ] {
        if !dir.exists() {
            match tokio::fs::create_dir_all(&dir).await {
                Ok(()) => fixed.push(format!("created {}", dir.display())),
                Err(e) => warn!("could not create {}: {}", dir.display(), e),
            }
        }
    }

    match build_and_write_locks(app).await {
        Ok(n) => fixed.push(format!("reconciled locks/versions.json ({} entries)", n)),
        Err(e) => warn!("could not reconcile the lockfile: {}", e),
    }

    // A backend reading as "degraded, stale index" recovers from a refresh.
    match app.update().await {
        Ok(()) => fixed.push("refreshed backend metadata".into()),
        Err(e) => warn!("could not refresh backend metadata: {}", e),
    }

    fixed
}

/// Health-gated upgrade: snapshot, upgrade, run the test, roll back automatically on
/// failure so a bad upgrade never leaves the machine broken.
pub(crate) async fn handle_canary(
    app: &App,
    scope: Option<PlannerScope>,
    test: &Option<String>,
) -> Result<()> {
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
        println!("nothing to upgrade.");
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

/// `linix policy` — report whether the desired state complies with the `[guard]` rules.
pub(crate) async fn handle_policy(app: &App) -> Result<()> {
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

/// Scaffold the on-disk layout LiNix expects and drop a starter manifest so a fresh
/// machine (or a freshly-cloned checkout) is immediately usable.
pub(crate) async fn handle_init(app: &App, force: bool, interactive: bool) -> Result<()> {
    let cfg = &app.config;
    scaffold_dirs(cfg).await?;

    if interactive {
        return interactive_init(app, force).await;
    }

    scaffold_repo(app, force).await?;

    println!("(Run `linix config init` to also write a commented preferences.toml, or `linix init -i` for guided setup.)");
    Ok(())
}

/// Create every on-disk directory LiNix expects. Idempotent.
pub(crate) async fn scaffold_dirs(cfg: &linix::config::Config) -> Result<()> {
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
pub(crate) struct InitAnswers {
    snapshot_count: Option<u32>,
    starter_packages: Vec<String>,
}

/// Pure: layer the interactive answers onto a base config. No I/O, so it can be tested.
pub(crate) fn apply_init_answers(
    mut base: linix::config::Config,
    a: &InitAnswers,
) -> linix::config::Config {
    if let Some(n) = a.snapshot_count {
        // One dial, not two: `keep_last = 0` is how a user says "keep everything", so an
        // `auto_prune` switch beside it was a second way to answer the same question.
        base.retention.snapshots.keep_last = n as usize;
    }
    base
}

/// Guided setup: write the II.1 repo, then ask the few things LiNix genuinely cannot work
/// out. Refuses to run without a TTY so CI falls back to `linix init` instead of hanging.
pub(crate) async fn interactive_init(app: &App, force: bool) -> Result<()> {
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

/// Write the II.1 repo: `priority`, `active`, and a profile to hang things on.
///
/// `priority` is generated from what this machine actually has (V.41: LiNix should look, not
/// ask you to maintain a list by hand on every machine forever), ordered by the one rule
/// that decides anything — a system manager beats a language manager (V.14). The file says
/// why, because a default nobody can explain is a default nobody can safely change (P5).
pub(crate) async fn scaffold_repo(app: &App, force: bool) -> Result<()> {
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

    // II.1 lists `vars` beside `active` and `priority`. It is scaffolded empty: a name LiNix
    // invented would be a condition nobody chose (IX.3 makes every reference to an undefined
    // name an error, and a helpful `role = desktop` is exactly the default P5 bans).
    let vars = layout.vars_file();
    if !vars.exists() || force {
        tokio::fs::write(
            &vars,
            "# Your own names for conditions. Each needs a top-level default before it can be\n\
             # used, and a `when` block may override it but may not introduce it.\n\
             #\n\
             #   role = desktop\n\
             #\n\
             #   when host in [thinkpad, x220] {\n\
             #     role = travel\n\
             #   }\n\
             #\n\
             # Then `when $role == travel { … }` anywhere `when` is legal.\n",
        )
        .await
        .with_context(|| format!("Failed to write {}", vars.display()))?;
        println!("  created  {:<10} {}", "vars", vars.display());
    } else {
        println!("  kept     {:<10} {}", "vars", vars.display());
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
        let cfg: linix::config::Config = toml::from_str(CONFIG_TEMPLATE)
            .expect("CONFIG_TEMPLATE must be valid preferences.toml");
        assert_eq!(cfg.guard.max_removals, 20);
    }

    #[test]
    fn the_shipped_example_parses_too() {
        // `examples/preferences.toml` is the long-form copy of the template, and it is the
        // one a reader is most likely to paste from. Nothing checked it: it carried
        // `[retention.generations]` and `[retention.manifests]` for a whole phase after the
        // generation format was deleted, and both were silently ignored — a documented
        // setting that does nothing is worse than an undocumented one.
        let text = include_str!("../../examples/preferences.toml");
        let cfg: linix::config::Config =
            toml::from_str(text).expect("examples/preferences.toml must parse");
        assert_eq!(
            cfg.rate_limit_max_wait_secs, 30,
            "the example and the built-in default disagree"
        );
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
