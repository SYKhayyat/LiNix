use crate::verbs::perform_maintenance;
use crate::verbs::plan::{
    approve_adapters, approve_exec_scripts, approve_generate_commands, build_and_write_locks,
    resolve_for_approval,
};
use crate::verbs::prelude::*;
use crate::verbs::sync::{enforce_policy, print_flight_plan};

/// `adapters` — the eight ways to extend LiNix, and what this machine has on each.
///
/// **The surfaces all worked; there was no way to ask about them.** A `[[backend]]` row teaches a
/// package manager, a `[[snapshot]]` row teaches a rollback provider, and every one of the eight
/// goes through II.12's ledger — but they were eight paths, eight readers and eight
/// `warn!("ignoring adapters/x.toml: …")` lines, and nothing in the program could answer *what
/// have I extended, and is it working?*
///
/// The column that matters is the last one. A file can be present, approved and valid TOML and
/// still be doing nothing, which is what `[[backends]]` for a `[[backend]]` reader produces:
/// valid TOML describing a table nobody opens. Rows-in-force is the number that tells you.
pub async fn handle_adapters(app: &App, surface: Option<&str>, out: Output) -> Result<()> {
    use crate::app::adapters::{self, Standing};

    let layout = app.config.layout();
    let mut found = adapters::survey(&layout);

    if let Some(name) = surface {
        if adapters::surface(name).is_none() {
            let known: Vec<&str> = adapters::SURFACES.iter().map(|s| s.name).collect();
            return Err(crate::core::Error::Validation(format!(
                "`{name}` is not an extension surface. There are {}: {}.",
                known.len(),
                known.join(", ")
            ))
            .into());
        }
        found.retain(|e| e.surface.name == name);
    }

    if out.is_json() {
        let rows: Vec<Value> = found
            .iter()
            .map(|e| {
                serde_json::json!({
                    "surface": e.surface.name,
                    "row": e.surface.row(),
                    "teaches": e.surface.teaches,
                    "file": e.path.display().to_string(),
                    "standing": e.standing.word(),
                    "rows_in_force": match e.standing {
                        Standing::InUse { rows } => rows,
                        _ => 0,
                    },
                    "detail": e.standing.detail(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    println!(
        "Extension surfaces — a row in one of these teaches LiNix something it does not ship.\n"
    );
    println!("{:<11} {:<18} {:<12} ROWS", "SURFACE", "ROW", "STANDING");
    for e in &found {
        let rows = match e.standing {
            Standing::InUse { rows } => rows.to_string(),
            _ => "-".to_string(),
        };
        println!(
            "{:<11} {:<18} {:<12} {}",
            e.surface.name,
            e.surface.row(),
            e.standing.word(),
            rows
        );
    }

    // The detail goes below the table rather than in it, because a serde message is a
    // paragraph and a table that wraps is a table nobody reads.
    let wrong: Vec<&adapters::Extension> = found.iter().filter(|e| e.standing.is_wrong()).collect();
    if !wrong.is_empty() {
        println!("\n{} surface(s) are not in force:", wrong.len());
        for e in &wrong {
            println!("\n  {} — {}", e.path.display(), e.standing.word());
            match &e.standing {
                Standing::NoRows => println!(
                    "    the file holds no `{}` rows. A row of another name is valid TOML and \
                     is read by nothing.",
                    e.surface.row()
                ),
                // Re-indented line by line: the ledger's refusal is three lines and pasting it
                // whole puts lines two and three hard against the left margin, under a heading
                // they belong to.
                _ => {
                    for line in e.standing.detail().unwrap_or_default().lines() {
                        println!("    {}", line.trim_start());
                    }
                }
            }
        }
    }

    if found.iter().all(|e| e.standing == Standing::Absent) {
        println!(
            "\nThis machine has extended nothing. Write `{}` to \
             {} and run `linix sync`; the first run asks you to approve it.",
            adapters::SURFACES[0].row(),
            adapters::SURFACES[0].path_in(&layout).display()
        );
    }
    Ok(())
}

/// `absent` (II.8): every `absent:` line in force, and the module it comes from — what LiNix
/// keeps OFF this machine, and where each rule is written. Read-only.
/// `vars` (Part IX, W12): the variables resolved on this machine, so a `when $name` block that
/// does not fire can be debugged by seeing the value the machine actually derived.
/// `linix add` — vendor someone else's modules into this repo (7/U14, XIII.14).
///
/// Fetch → plan → refuse-or-copy → optionally approve. The safety story is not the fetch; it is
/// that anything executable lands unapproved and II.12 holds it until `linix lock`. `--trust`
/// runs that lock in the same step; without it, the vendored code sits inert and reviewable.
pub async fn handle_add(app: &App, source: &str, trust: bool, force: bool) -> Result<()> {
    use crate::model::vendor::{self, Source, Vendored};

    let Some(src) = Source::classify(source) else {
        return Err(crate::core::Error::Validation(format!(
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
    let scratch = tempfile::tempdir().map_err(crate::core::Error::from)?;
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
                    crate::core::Error::Other(format!("could not clone {}: {}", src.label(), e))
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
            crate::utils::file::ensure_dir(&dir)?;
            info!("downloading {}...", url);
            let body = reqwest::get(url)
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| crate::core::Error::Other(format!("could not fetch {}: {}", url, e)))?
                .text()
                .await
                .map_err(|e| crate::core::Error::Other(format!("reading {}: {}", url, e)))?;
            std::fs::write(dir.join(name), body).map_err(crate::core::Error::from)?;
            scratch.path().to_path_buf()
        }
        Source::Local(p) => {
            if !p.exists() {
                return Err(crate::core::Error::Validation(format!(
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
        return Err(crate::core::Error::Refused(format!(
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
            crate::utils::file::ensure_dir(parent)?;
        }
        std::fs::copy(fetched.join(&pl.from), &to)
            .map_err(|e| crate::core::Error::Io(format!("copying {}: {}", pl.to.display(), e)))?;
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
            let events = crate::app::events::EventHooks::load(&app.config);
            let _ = events.approve_all();
            let approved = app.hooks.approve_all_hooks().unwrap_or(0);
            approve_adapters(app).ok();
            approve_generate_commands(app).ok();
            if let Ok(model) = resolve_for_approval(app).await {
                approve_exec_scripts(app, &model).await.ok();
            }
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
pub fn collect_relative_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
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
pub async fn handle_try(app: &App, image: Option<&str>) -> Result<()> {
    use crate::model::rehearsal::{self, Verdict};

    let present = |cmd: &str| app.executor.command_exists_sync(cmd);
    let Some(runtime) = rehearsal::pick_runtime(&present) else {
        return Err(crate::core::Error::Refused(rehearsal::no_runtime_refusal()).into());
    };

    let image = image.unwrap_or(rehearsal::DEFAULT_IMAGE);

    // Asked BEFORE the run: an image that is not there is the ordinary first-run case, and
    // `docker run` reports it as a pull failure — which reads as "your config is broken" when
    // the config has not been looked at yet.
    if !image_exists(app, runtime, image).await {
        return Err(
            crate::core::Error::Refused(rehearsal::missing_image_refusal(runtime, image)).into(),
        );
    }

    let root = app.config.config_root();
    let config_path = root.to_string_lossy().to_string();

    info!("rehearsing on `{}` via {}...", image, runtime);
    let argv = rehearsal::argv(runtime, image, &config_path);
    let (program, args) = argv.split_first().expect("an argv is never empty");

    let mut command = tokio::process::Command::new(program);
    command.args(args);
    // The terminal-handoff door: a container rehearsal prints as it goes and the person is
    // watching it — but owned, so abandoning the rehearsal does not leave the container running.
    let status = crate::core::executor::supervised_status(command, runtime)
        .await
        .map_err(|e| crate::core::Error::Other(format!("could not run `{}`: {}", runtime, e)))?;

    match rehearsal::verdict(status.code()) {
        Verdict::Valid => {
            println!("the config resolves on a clean {} machine.", image);
            Ok(())
        }
        // The container already printed why on its own stderr; repeating it here would be
        // two accounts of one failure.
        Verdict::Rejected(_) => Err(crate::core::Error::Refused(format!(
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
pub async fn image_exists(app: &App, runtime: &str, image: &str) -> bool {
    app.executor
        .run(runtime, &["image", "inspect", image], false)
        .await
        .is_ok()
}

pub const CONFIG_TEMPLATE: &str = r#"# LiNix refusals and behaviour (preferences.toml). Nothing writes to this but you.
# Every key is optional; omit a key to use its built-in default.
#
# Where your repo lives is NOT a key here — this file is inside it. Use `linix path --set`.

# Maximum number of packages installed/removed (and searched) in parallel.
# Omit to auto-detect this machine's core count (respecting container CPU limits).
# max_parallel = 4

# Timeout (seconds) for outbound HTTP search requests (npm/PyPI/marketplace).
network_timeout_secs = 15

# How long a command may print NOTHING before LiNix kills it and says which one.
# Not a cap on how long a command may run: a build that prints for an hour is untouched.
# Raise it if you drive something legitimately silent for longer; 0 removes the bound.
command_idle_timeout_secs = 900

# The same cap, for a READ — a listing or a search. Its own key because the number above is
# sized for a mutation that is legitimately silent for minutes, and a read takes seconds.
query_idle_timeout_secs = 120

# How many times a read that failed TRANSIENTLY is asked again. 1 disables it. Reads only:
# asking twice costs a second, where a mutation retried on a guess installs twice.
read_retry_attempts = 3

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
# scope = "user"

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

# The same, for resource teardowns and for ports closed because nothing declares
# them. Each is its own budget: raising one says nothing about the others.
# max_extra_removals = 20
# max_port_closures = 20

# Refuse any single command that changes more than this many things ALL TOLD —
# installs, upgrades, removals, teardowns, ports opened and closed. The numbers
# above each bound one kind, and twenty of each is sixty. 0 (the default) is off.
# max_total_changes = 0

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

# Refuse to apply when `linix check security` reports a managed package as vulnerable.
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
# never_unattended = ["rebuild", "purge-undeclared"]
"#;

pub async fn handle_path(cli: &Cli, explain: bool, set: Option<&std::path::Path>) -> Result<()> {
    use crate::app::locate;

    if let Some(dir) = set {
        let (settings_file, stored) = locate::set_root(dir)?;
        if stored {
            println!("Config repo set to {}", dir.display());
            println!("Stored in {}", settings_file.display());
        } else {
            crate::would_print!("would set the config repo to {}", dir.display());
            crate::would_print!("would store it in {}", settings_file.display());
        }
        return Ok(());
    }

    let resolved = locate::locate(cli.config_dir.as_deref())?;
    println!("{}", locate::render_path(&resolved, explain));
    Ok(())
}

pub async fn handle_edit(cli: &Cli, file: Option<&str>) -> Result<()> {
    use crate::app::locate;

    let resolved = locate::locate(cli.config_dir.as_deref())?;
    let target = locate::resolve_target(&resolved.path, file)?;
    let editor = locate::editor_command();

    let is_preferences =
        target.file_name().and_then(|n| n.to_str()) == Some(crate::config::PREFERENCES_FILE_NAME);
    if is_preferences && !target.exists() {
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&target, CONFIG_TEMPLATE).await?;
        println!("Created {} from the default template.", target.display());
    }

    let mut command = tokio::process::Command::new(&editor);
    command.arg(&target);
    // The terminal-handoff door: `$EDITOR` owns the terminal for as long as somebody is typing
    // in it, so no bound — but owned, because an editor still holding the config file after
    // LiNix has gone is the state AU6 is about.
    let status = crate::core::executor::supervised_status(command, &editor)
        .await
        .with_context(|| format!("launching editor '{}'", editor))?;

    if !status.success() {
        anyhow::bail!("editor '{}' exited abnormally.", editor);
    }

    // Catch a typo here rather than at the next run, when the command that fails is
    // unrelated to the edit that broke it.
    if is_preferences {
        let p = target.clone();
        match tokio::task::spawn_blocking(move || crate::config::Config::from_file(&p)).await? {
            Ok(_) => println!("Saved. {} parses cleanly.", target.display()),
            Err(e) => anyhow::bail!(
                "{} no longer parses ({}). Re-run `linix edit {}` to fix it.",
                target.display(),
                e,
                crate::config::PREFERENCES_FILE_NAME
            ),
        }
    }
    Ok(())
}

pub async fn handle_config(app: &App, cmd: &ConfigCommand) -> Result<()> {
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
            if !crate::core::dry_run::active() {
                if let Some(parent) = path.parent() {
                    tokio::fs::create_dir_all(parent).await.ok();
                }
            }
            if crate::utils::file::persist(&path, CONFIG_TEMPLATE)
                .with_context(|| format!("Failed to write config to {}", path.display()))?
            {
                println!("Wrote commented default preferences to {}", path.display());
            } else {
                crate::would_print!(
                    "would write commented default preferences to {}",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

pub async fn handle_heal(app: &App) -> Result<()> {
    // **Before the WAL heal, and the ordering is the repair** (`Q50`). While pacman's `db.lck`
    // is on disk every manager command on this machine fails — including the ones the recovery
    // below runs — and `heal()` ends in a `?`, so a lock cleared *after* it is a lock cleared
    // on the runs that did not need it. First written the other way round, under a comment
    // claiming it went first.
    for fixed in settle_manager_locks(app).await {
        println!("heal: {}", fixed);
    }
    // `heal` reads ownership from what this machine declares, so the model has to be resolved
    // first. `sync` has one already and hands it over; this command is on its own.
    //
    // A config that will not resolve must not stop the rest of this command. `heal` is what you
    // run when the machine is broken, and half of what it repairs — interrupted operations,
    // wedged manager locks — has nothing to do with the manifest. So a resolution failure costs
    // the ownership half and is reported, rather than aborting the repair that was asked for.
    let declared: Vec<crate::core::PackageSpec> =
        match crate::app::sync::resolver::StateResolver::new(
            &app.config,
            app.registry.clone(),
            false,
        )
        .await
        .resolve_model()
        .await
        {
            Ok(state) => state.packages.into_values().flatten().collect(),
            Err(e) => {
                warn!(
                    "{e} Skipping the ownership half of the repair; the rest of `heal` continues."
                );
                Vec::new()
            }
        };
    app.sync_engine().await.heal(&declared).await?;
    // U9: `check` looks, `heal` acts. These three repairs used to be `doctor --fix`, which
    // made one command both the diagnosis and the treatment — and a command that changes
    // things is one you cannot run to find out whether you want things changed.
    for fixed in repair_environment(app).await {
        // `heal:` rather than `repaired:`, because each item now carries its own tense and a
        // preview repairs nothing — the label was the last past-tense word left in this output.
        println!("heal: {}", fixed);
    }
    Ok(())
}

/// The package manager's own lock, left behind by a run that was killed (`Q50`).
///
/// **Only in `heal`, never in `sync`.** Deleting another package manager's file is a repair
/// somebody asked for by name, not something a converge does on the way past — and `heal` is
/// the command whose whole subject is *a run was interrupted*.
///
/// Which locks may be cleared, and the proof that one is not held, are
/// [`crate::app::stale_lock`]'s: this is the half that acts, and it is separate so the deciding
/// half can be tested against a machine that is not this one.
/// Clear what is stale — and first, wait out anything that is merely *busy*.
///
/// **A survey is a snapshot, and `heal` was acting on one.** It looked once at the top, found a
/// live `pacman` holding `db.lck`, correctly left it alone — and then that `pacman`, an orphan of
/// the very run `heal` was called to recover from, exited during the recovery. By the time the
/// lock was stale, the only step that could clear it had already run, and `heal` finished by
/// telling the user to run `heal`.
///
/// So the holder is waited out first. That is not a delay bolted on: `heal`'s whole subject is
/// *a run was interrupted*, and an orphan still finishing that run's transaction is the most
/// interesting thing on the machine. Waiting for it is the repair; clearing what it leaves is
/// the rest of the repair. Bounded by `manager_lock_wait_secs`, the same budget `sync` waits
/// under, and announced — a silent `heal` that pauses for five minutes is a `heal` that gets
/// killed, which is how the lock got there.
async fn settle_manager_locks(app: &App) -> Vec<String> {
    let budget = std::time::Duration::from_secs(app.config.manager_lock_wait_secs);
    let mut said = Vec::new();

    for lock in crate::app::stale_lock::MANAGER_LOCKS {
        let Some(backend) = lock.backends.first() else {
            continue;
        };
        let crate::app::stale_lock::Held::Live(who) =
            crate::app::stale_lock::held_for_on_this_machine(backend)
        else {
            continue;
        };
        if budget.is_zero() {
            said.push(format!(
                "{who} is running, so its lock is held — not waiting, because \
                 `manager_lock_wait_secs` is 0"
            ));
            continue;
        }
        println!(
            "heal: waiting up to {}s for {who} to finish — it is holding {}'s lock, and until it \
             lets go nothing can be said about whether that lock is stale",
            budget.as_secs(),
            lock.holder()
        );
        match crate::app::stale_lock::wait_until_not_held(backend, budget, &|| false).await {
            crate::app::stale_lock::Waited::Freed(spent) => said.push(format!(
                "waited {}s for {who} to finish before looking at {}'s lock",
                spent.as_secs(),
                lock.holder()
            )),
            crate::app::stale_lock::Waited::StillHeld => said.push(format!(
                "{who} has been running for {}s and still holds {}'s lock — nothing is broken, \
                 it is working. Run `linix heal` again when it has finished.",
                budget.as_secs(),
                lock.holder()
            )),
            crate::app::stale_lock::Waited::Cancelled => {}
        }
    }

    said.extend(clear_stale_manager_locks(app).await);
    said
}

async fn clear_stale_manager_locks(app: &App) -> Vec<String> {
    let mut fixed = Vec::new();
    let survey = crate::app::stale_lock::find_on_this_machine();
    // Said, not skipped. A lock left in place is the likeliest reason the next command fails,
    // and reporting only removals is what made this decision invisible on the one run where it
    // mattered.
    for left in &survey.left {
        fixed.push(format!(
            "left {} alone — {}",
            left.path.display(),
            left.because
        ));
    }
    for stale in survey.stale {
        if crate::core::dry_run::active() {
            fixed.push(format!(
                "would remove {} — {}",
                stale.path.display(),
                stale.because
            ));
            continue;
        }
        // Through the executor, so it is elevated the way every other privileged step is and
        // shows up in the same log. A failure here is reported and does not stop the rest: a
        // lock LiNix could not remove is still a lock the user can remove, now that they have
        // been told which one and why.
        let path = stale.path.display().to_string();
        match app.executor.run("rm", &["-f", &path], true).await {
            Ok(_) => fixed.push(format!(
                "removed {}'s stale lock at {} — {}",
                stale.holder, path, stale.because
            )),
            Err(e) => warn!(
                "{}'s lock at {} is stale ({}) and could not be removed: {}. \
                 Remove it by hand and re-run.",
                stale.holder, path, stale.because, e
            ),
        }
    }
    fixed
}

/// Put right what `check` can only report: the II.1 directories, the version lockfile, and a
/// stale backend index. Each is best-effort and reported by name — a repair that failed must
/// not stop the ones after it, and a repair nobody sees is the class of silence P3 forbids.
pub async fn repair_environment(app: &App) -> Vec<String> {
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
        // Two tenses, from the writer's own answer rather than from the flag: `--dry-run heal`
        // printed `repaired: reconciled locks/versions.json` beside its own `[DRY-RUN] would
        // write` line for the same file.
        Ok((n, true)) => fixed.push(format!("reconciled locks/versions.json ({} entries)", n)),
        Ok((n, false)) => fixed.push(format!(
            "would reconcile locks/versions.json ({} entries)",
            n
        )),
        Err(e) => warn!("could not reconcile the lockfile: {}", e),
    }

    // A backend reading as "degraded, stale index" recovers from a refresh. Under a preview the
    // executor runs no manager command, so the sentence has to say so.
    match app.update().await {
        Ok(()) if crate::core::dry_run::active() => {
            fixed.push("would refresh backend metadata".into())
        }
        Ok(()) => fixed.push("refreshed backend metadata".into()),
        Err(e) => warn!("could not refresh backend metadata: {}", e),
    }

    fixed
}

/// Health-gated upgrade: snapshot, upgrade, run the test, roll back automatically on
/// failure so a bad upgrade never leaves the machine broken.
pub async fn handle_canary(
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
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    enforce_policy(app, &desired).await?;

    // `--canary` with no `--profile`/`--module` is a whole-machine converge behind a health
    // check, so it reaps — but only what `priority` names. It used to reap every backend on the
    // box, which made the one command that promises to roll back the one most likely to need to.
    //
    // Both variants are named at the call and not folded into a `scope` binding above it: the
    // enumeration gate reads this file's source, and a scope computed out of sight is a scope
    // the gate reports as unreadable — which is how it found this site in the first place.
    let hosts = app.host_backends().await;
    let changes = {
        let state_guard = app.state.lock().await;
        let planner = crate::app::sync::planner::ChangePlanner::new(
            app.registry.clone(),
            &state_guard,
            &app.config,
        );
        match scope {
            Some(s) => planner.plan(&desired, PlanScope::Narrowed(s)).await?,
            None => planner.plan(&desired, PlanScope::Whole(hosts)).await?,
        }
    };
    if changes.is_empty() {
        println!("nothing to upgrade.");
        return Ok(());
    }
    print_flight_plan(app, &changes);

    if app.config.dry_run {
        crate::would_print!(
            "Would snapshot, upgrade, run `{}`, and roll back on failure.",
            test
        );
        return Ok(());
    }

    let snap = app
        .snapshot_manager
        .auto_snapshot(crate::core::snapshot::SnapshotLabel::PreCanary)
        .await?
        .ok_or_else(|| anyhow::anyhow!("failed to create pre-canary snapshot"))?;
    info!("snapshot {} taken; applying upgrade...", snap.id);
    app.sync_engine()
        .await
        .sync(changes, crate::app::sync::guard::GuardScope::Canary)
        .await?;

    info!("running health check: {}", test);
    if crate::app::bisect::run_test(&test).await {
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
pub async fn handle_policy(app: &App) -> Result<()> {
    let guard = &app.config.guard;
    if guard.is_empty() {
        println!("No [guard] install/change rules are set — nothing to check.");
        return Ok(());
    }
    let resolver =
        crate::app::sync::resolver::StateResolver::new(&app.config, app.registry.clone(), false)
            .await;
    let desired = resolver.resolve_desired_state().await?;
    // **The preview calls the thing it previews.** This used to re-implement `enforce_policy`
    // minus `deny_vulnerable`, then print a footnote admitting the gap — so `linix policy` could
    // report "compliant" for a config `sync` would refuse, which is the one thing a preview must
    // never do. The footnote is gone because the gap is.
    let violations = crate::verbs::sync::policy_violations(app, &desired).await;
    if violations.is_empty() {
        println!("[guard] check passed — the desired state is compliant.");
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
pub async fn handle_init(app: &App, force: bool, interactive: bool) -> Result<()> {
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
pub async fn scaffold_dirs(cfg: &crate::config::Config) -> Result<()> {
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
/// aggressive answer is `purge-undeclared`, a command, not a mode — V.21), "protect
/// imperative installs?" (they have a line now, so they are declared like everything else),
/// "preferred default backend?" (that is `priority`, generated from what this machine has —
/// V.15). A question whose answer LiNix can work out, or which no longer means anything, is
/// homework (V.41).
#[derive(Debug, Clone, Default)]
pub struct InitAnswers {
    snapshot_count: Option<u32>,
    starter_packages: Vec<String>,
}

/// Pure: layer the interactive answers onto a base config. No I/O, so it can be tested.
pub fn apply_init_answers(
    mut base: crate::config::Config,
    a: &InitAnswers,
) -> crate::config::Config {
    if let Some(n) = a.snapshot_count {
        // One dial, not two: `keep_last = 0` is how a user says "keep everything", so an
        // `auto_prune` switch beside it was a second way to answer the same question.
        base.retention.snapshots.keep_last = n as usize;
    }
    base
}

/// Guided setup: write the II.1 repo, then ask the few things LiNix genuinely cannot work
/// out. Refuses to run without a TTY so CI falls back to `linix init` instead of hanging.
pub async fn interactive_init(app: &App, force: bool) -> Result<()> {
    use dialoguer::Input;
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
        return Err(crate::core::Error::Refused(
            "`init -i` is interactive but stdin is not a terminal. \
             Run `linix init` (non-interactive) or `linix config init` instead."
                .to_string(),
        )
        .into());
    }

    let config_path = app.config.preferences_file.clone();
    if config_path.exists() && !force {
        anyhow::bail!(
            "Config already exists at {}. Re-run `linix init -i --force` to overwrite it.",
            config_path.display()
        );
    }

    println!("\nLet's set up LiNix. Press Enter to accept the [default].\n");

    let defaults = crate::config::Config::default();
    let mut answers = InitAnswers::default();

    // `init` is a conversation, so every answer here is an unbounded wait on a person.
    let keep: String = crate::core::on_the_terminal(|| {
        Input::new()
            .with_prompt("How many system snapshots to keep (0 keeps every one)")
            .default(defaults.snapshot_retention().keep_last.to_string())
            .interact_text()
    })?;
    answers.snapshot_count = keep.trim().parse::<u32>().ok();

    let starter: String = crate::core::on_the_terminal(|| {
        Input::new()
            .with_prompt("Packages to start with (comma-separated, blank to skip)")
            .allow_empty(true)
            .interact_text()
    })?;
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
        app.declare(pkg, None, crate::model::Landing::Imperative)
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
pub async fn scaffold_repo(app: &App, force: bool) -> Result<()> {
    let layout = app.config.layout();

    let detected: Vec<String> = app
        .registry
        .available()
        .iter()
        .map(|b| b.name().to_string())
        .collect();
    let ordered = crate::model::priority::starter_order(&detected);

    let priority = layout.priority_file();
    if !priority.exists() || force {
        tokio::fs::write(&priority, crate::model::priority::starter_file(&ordered))
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

    // `init --help` promises "a starter module" and `modules/` was created empty, so the one
    // thing a new user was told to look at did not exist. It carries no package: P5 bans the
    // default nobody chose, and a machine that installs something because it was scaffolded is
    // exactly that. What it carries is the shape of a line, in the file where lines go.
    let starter = layout.module_file(&crate::model::ModuleName::literal("starter"));
    if !starter.exists() || force {
        tokio::fs::write(
            &starter,
            "# A module is a list of packages, one per line. This one is yours to edit.\n\
             #\n\
             # Uncomment a line, then run `linix sync`:\n\
             #\n\
             #   jq                 let LiNix pick the manager, best first from `priority`\n\
             #   cargo:ripgrep      or name the manager yourself\n\
             #   git@version=2.44   pin a version\n\
             #\n\
             # Nothing is installed until a line is here and `linix sync` has run. Deleting a\n\
             # line and syncing again removes the package.\n",
        )
        .await
        .with_context(|| format!("Failed to write {}", starter.display()))?;
        println!("  created  {:<10} {}", "module", starter.display());
    } else {
        println!("  kept     {:<10} {}", "module", starter.display());
    }

    // Something has to be active or nothing is: a module nothing reaches is inert (II.3) — so
    // the starter is wired in here, or writing it would have been theatre.
    let profile = layout.profile_file("Main");
    if !profile.exists() || force {
        tokio::fs::write(
            &profile,
            "# What this machine is set to. Add `use <module>` lines, or packages directly.\n\
             #\n\
             # Profiles are Capitalized, modules are lowercase — so `(Work | gaming)` tells\n\
             # you what everything is without extra syntax.\n\
             \n\
             use starter\n",
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
        let base = crate::config::Config::default();
        let answers = InitAnswers {
            snapshot_count: Some(42),
            starter_packages: vec![],
        };
        let cfg = apply_init_answers(base, &answers);
        assert_eq!(cfg.retention.snapshots.keep_last, 42);
    }

    #[test]
    fn omitted_snapshot_count_keeps_base_default() {
        let base = crate::config::Config::default();
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
        let cfg = apply_init_answers(crate::config::Config::default(), &answers);
        let toml_str = toml::to_string_pretty(&cfg).expect("serializes");
        let back: crate::config::Config = toml::from_str(&toml_str).expect("parses back");
        assert_eq!(back.retention.snapshots.keep_last, 7);
    }

    #[test]
    fn config_template_actually_parses_and_matches_the_defaults() {
        // `linix config init` writes this file verbatim. A template that does not parse
        // hands every new user a broken config, and a template whose keys don't match the
        // struct silently documents settings that do nothing (as `cache_ttl` did).
        let cfg: crate::config::Config = toml::from_str(CONFIG_TEMPLATE)
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
        let cfg: crate::config::Config =
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
