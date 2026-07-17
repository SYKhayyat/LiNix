use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// LiNix - Universal Mission-Critical Package Manager
/// High-performance, DAG-based orchestration for 50+ backends.
/// Version 6.0.0: cross-ecosystem audit/SBOM, provenance (`why`), health-gated canary
/// upgrades, snapshot bisect, SSH clone/fleet, a policy gate, and system-scope pruning.
#[derive(Parser, Debug)]
#[command(
    name = "linix",
    version = "6.0.0",
    about = "Universal Mission-Critical Package Manager"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Run without making actual system changes
    #[arg(short = 'n', long, global = true)]
    pub dry_run: bool,

    /// Skip confirmation prompts
    #[arg(short, long, global = true)]
    pub yes: bool,

    /// Path to custom config.toml
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Carry out a removal the guard refuses: one over `max_removals`, or touching a
    /// protected/essential package. Global because every command that can delete needs
    /// it. Deliberately NOT implied by --yes: scripts and CI pass -y everywhere, and an
    /// unattended run is the one that cannot notice a system being dismantled.
    /// See `linix protected` for what is guarded and why.
    #[arg(long, global = true)]
    pub allow_mass_removal: bool,

    /// Carry out an install the guard refuses for being over `max_installs` (II.10). The
    /// symmetric partner to --allow-mass-removal, and — like it — NOT implied by --yes: a
    /// script passing -y everywhere must not also silently green-light a ten-thousand-package
    /// install a mis-globbed manifest produced.
    #[arg(long, global = true)]
    pub allow_mass_install: bool,

    /// Hide progress indicators (spinners/bars). Progress shows by default; this turns it off.
    ///
    /// Was `--progress` with `default_value = "true"` on a `bool`, which clap derives as
    /// `SetTrue` — so it was stuck on with no way to disable it (S5), and nothing read it. A
    /// plain `--no-progress` flag is a real off-switch, and it overrides `show_progress` in
    /// config.
    #[arg(long, global = true)]
    pub no_progress: bool,

    /// Enable debug-level logging
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Quiet mode: suppress the flight plan and transaction summary (errors still print)
    #[arg(short, long, global = true)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Synchronize system state with declarative configuration (DAG-based)
    Sync {
        /// Force strict version matching against locked state
        #[arg(long)]
        locked: bool,

        /// Output the transition plan as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,
    },

    /// Continuously reconcile the system to your manifests (GitOps for one machine): on each
    /// tick, optionally `git pull` the config, then apply any changes automatically. Unattended
    /// by design — it applies without prompting. Ctrl-C to stop.
    Watch {
        /// Seconds between reconcile checks
        #[arg(long, default_value = "30")]
        interval: u64,

        /// Only reconcile when a manifest file changed since the last tick (otherwise every tick)
        #[arg(long)]
        on_change: bool,

        /// `git pull --ff-only` the config directory before each reconcile
        #[arg(long)]
        pull: bool,

        /// Run a single reconcile pass and exit (for cron/testing)
        #[arg(long)]
        once: bool,
    },

    /// Run a command within an ephemeral package environment
    Run {
        /// Packages to make available in the environment
        #[arg(short, long)]
        packages: Vec<String>,
        /// The command to execute
        command: String,
    },

    /// Create a permanent high-performance Rust shim for a package
    Shim {
        /// The name of the binary to create
        binary: String,
        /// The source package spec (e.g. "cargo:ripgrep")
        #[arg(short, long)]
        source: String,
    },

    /// Recover the system from an interrupted or crashed transaction (WAL)
    Heal,

    /// Perform a deep system cleanup (orphans, cache, temp files)
    Clean,

    /// Identify all packages installed on the OS but not managed by LiNix
    Unmanaged,

    /// Parse everything the active profiles reach and report any errors — without planning
    /// or changing anything (II.8). A clean parse says how many packages resolved.
    Check,

    /// Show every `absent:` line in force and the module it comes from (II.8) — what LiNix
    /// is keeping OFF this machine, and where each rule is written.
    Absent,

    /// Delete everything LiNix does not manage. Shows the whole list first.
    ///
    /// This is the strict "make this machine exactly match my files" command. It is a
    /// command and not a setting on purpose: no config anyone can flip, inherit, or copy
    /// from a dotfiles repo makes a routine `sync` delete software it did not install.
    #[command(name = "purge-unmanaged")]
    PurgeUnmanaged {
        /// Proceed even though LiNix manages very little of this machine — which usually
        /// means it has not been adopted yet, not that you want the rest deleted.
        #[arg(long = "i-really-mean-it")]
        i_really_mean_it: bool,
    },

    /// Stop managing a package WITHOUT uninstalling it. LiNix forgets it exists; the
    /// package stays on your system. This is the counterpart to deleting a manifest line,
    /// which means "uninstall this" — not "stop managing it"
    Unmanage {
        /// Packages to forget ("apt:jq", or a bare name to search every backend)
        #[arg(required = true)]
        packages: Vec<String>,

        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Show what the removal guard protects: the packages removal will refuse to touch,
    /// your exemptions, and the maximum removal count
    Protected {
        /// Check specific packages instead of listing the rules ("apt:python3" or "jq"),
        /// reporting whether each is protected and which rule decides it
        packages: Vec<String>,

        /// Emit JSON
        #[arg(long)]
        json: bool,
    },

    /// Show what `sync` would change (to install / drift to remove / unmanaged) — read-only
    #[command(alias = "diff")]
    Status {
        /// Output the report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Compute what `sync` would do and freeze it to a reviewable file, so the exact plan you
    /// inspect is the one you later `apply` (Terraform-style plan/apply for packages).
    Plan {
        /// Where to write the plan (default: linix-plan.json)
        #[arg(long, default_value = "linix-plan.json")]
        out: String,
    },

    /// Execute a previously saved plan file, applying exactly the captured changes. Warns if
    /// the system/manifests have drifted since the plan was frozen.
    Apply {
        /// Path to a plan file produced by `linix plan`
        plan: String,

        /// Apply even if the system has drifted from the captured plan
        #[arg(short, long)]
        yes: bool,
    },

    /// Record the installed version of every managed package to locks.json, so
    /// `sync --locked` reproduces those exact versions on another machine
    Lock,

    /// Parallel search across all searchable repositories
    Search {
        /// Search query string
        query: String,

        /// Output results as JSON
        #[arg(long)]
        json: bool,

        /// Only show results that are already installed / managed by LiNix
        #[arg(long)]
        installed: bool,
    },

    /// Refresh repository metadata for all backends
    Update,

    /// Upgrade managed packages to their latest versions.
    ///
    /// With no arguments, runs each backend's native batch upgrade (e.g. `apt upgrade`).
    /// Name one or more PACKAGES to upgrade just those. `--backend` scopes to one manager,
    /// `--all` forces the native whole-system upgrade, and `--security` upgrades only the
    /// packages `audit` flags as vulnerable. `--except` subtracts packages from any of these.
    Upgrade {
        /// Specific package(s) to upgrade (optionally `backend:name`). Empty = whole system.
        packages: Vec<String>,

        /// Upgrade only packages managed by this backend
        #[arg(long)]
        backend: Option<String>,

        /// Native whole-system upgrade across every backend (e.g. `apt upgrade` + `brew upgrade`)
        #[arg(long)]
        all: bool,

        /// Upgrade only packages that `linix audit` reports as vulnerable, to their fixed version
        #[arg(long)]
        security: bool,

        /// Package name(s) to hold back / exclude from this upgrade (repeatable)
        #[arg(long, value_name = "PACKAGE")]
        except: Vec<String>,

        /// Limit upgrade to a specific profile
        #[arg(long)]
        profile: Option<String>,

        /// Limit upgrade to a specific module
        #[arg(long)]
        module: Option<String>,

        /// Output potential changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,

        /// Health-gated upgrade: snapshot first, then run --test after upgrading, and
        /// automatically roll back to the snapshot if the test fails
        #[arg(long)]
        canary: bool,

        /// Health-check command run after a --canary upgrade (non-zero exit = roll back)
        #[arg(long)]
        test: Option<String>,
    },

    /// List all installed packages
    List {
        /// Filter results by a specific backend
        #[arg(short, long)]
        backend: Option<String>,

        /// Output the list in machine-readable JSON format
        #[arg(long)]
        json: bool,

        /// Show only packages with a newer version available (installed vs latest)
        #[arg(long)]
        outdated: bool,
    },

    /// Fetch detailed metadata and properties for a specific package
    Info {
        /// Name of the package
        package: String,
    },

    /// Imperatively install one or more packages
    Install {
        /// Package strings (e.g. "apt:curl", "cargo:exa")
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,

        /// Temporary install: uninstall itself after this duration (e.g. "2h", "30d").
        /// Written as a dated line — `@expires=<absolute time>` (II.16) — not a lease.
        #[arg(long, value_name = "DURATION")]
        temp: Option<String>,

        /// Which file the line goes in: a module (lowercase) or a profile (Capitalized).
        /// Without it, the line lands in the `imperative` module.
        #[arg(long, value_name = "NAME")]
        into: Option<String>,
    },

    /// Imperatively uninstall one or more packages
    Uninstall {
        /// Names of packages to purge
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        #[arg(long)]
        json: bool,

        /// Temporary uninstall: reinstall the package(s) later. With a DURATION
        /// (e.g. `--temp=2h`) they return when it elapses; bare `--temp` inside a
        /// `linix shell` restores them when that ephemeral session ends. The duration must
        /// be attached with `=` so it is never confused with a package name.
        #[arg(long, value_name = "DURATION", num_args = 0..=1, require_equals = true)]
        temp: Option<Option<String>>,
    },

    /// Manage source repositories (PPA, Taps, Buckets, etc.)
    Repo(RepoArgs),

    /// Deep system health check: per-backend readiness/severity (via each backend's own
    /// health probe), config/state integrity, and directory layout. `--fix` repairs what it
    /// safely can (missing directories, stale metadata).
    Doctor {
        /// Attempt to auto-repair fixable problems (create missing dirs, refresh metadata)
        #[arg(long)]
        fix: bool,

        /// Emit the full report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Take over the machine: write the packages you installed by hand into a module
    Adopt,

    /// Move a package from one backend to another (e.g. apt -> snap)
    Teleport {
        /// Name of the package to move
        package: String,
        /// Name of the destination backend
        to: String,
    },

    /// Enter an ephemeral shell with specific packages loaded
    Shell {
        /// Packages to load into the ghost shell
        packages: Vec<String>,
    },

    /// Interactive snapshot gallery and system rollback
    Undo,

    /// Time-travel cockpit: browse generations (left), inspect a generation's package set and
    /// config diff (right), and run commands from a shell line (bottom). Roll back from within.
    #[command(alias = "tui")]
    Cockpit,

    /// Set what this machine is: `active` becomes exactly these profiles, then converge.
    ///
    /// Several profiles can be active at once — their package sets are unioned. This is the
    /// set form: it overwrites the file, `when` blocks included, because a form that quietly
    /// kept part of the old file would leave you somewhere you did not type. Use `-a` to add
    /// to the list instead. Live; no reboot.
    Activate {
        /// Profile name(s). Without `-a`, these become the whole list.
        ///
        /// Not `required` at the clap layer on purpose: an empty list has a specific
        /// meaning worth a specific refusal, and clap's generic "the following required
        /// arguments were not provided" does not teach it (II.6).
        profiles: Vec<String>,

        /// Add to the list rather than replacing it.
        #[arg(short = 'a', long = "add")]
        add: bool,
    },

    /// Deactivate one or more profiles: drop each from the active set and converge, removing
    /// packages no longer required by any remaining active profile. Live; no reboot.
    Deactivate {
        /// Profile name(s) to deactivate
        #[arg(required = true)]
        profiles: Vec<String>,
    },

    /// Manage system profiles / identities (list, show, create, save, switch, active)
    Profile(ProfileArgs),

    // --- NEW FOR 3.6.0 ---
    /// Reusable package modules (@module syntax)
    Module(ModuleArgs),

    /// System snapshots and atomic rollbacks
    Snapshot(SnapshotArgs),

    /// Generations: list saved system states, pin them, or roll back to one
    Generation(GenerationArgs),

    /// Roll back to a saved generation by id: realizes its package set on the system
    /// (drive backends), and for a full rollback also restores its manifests. Scope with
    /// `--package` and/or the global `--backend` to roll back just part of the system.
    Rollback {
        /// Generation id to restore (see `linix generation list`)
        id: String,
        /// Only roll back this package (name or backend:name)
        #[arg(long)]
        package: Option<String>,
        /// Also check out the manifest git commit stamped on this generation, so config and
        /// system are rolled back together (the "grab the other half" convenience).
        #[arg(long)]
        with_config: bool,
    },

    /// Version-control your manifests/config directory with git: init, status, log, commit,
    /// and checkout (roll the *config* back to a past commit without touching packages).
    Git(GitArgs),

    /// Native system-level task scheduling (systemd, launchd, task-scheduler)
    Schedule(ScheduleArgs),

    /// Inspect and scaffold the LiNix application configuration file
    Config(ConfigArgs),

    /// Scaffold the LiNix directory structure (groups, modules, data dirs) and a
    /// starter manifest, so a fresh machine is ready for `linix sync`
    Init {
        /// Reset the starter manifest even if one already exists
        #[arg(long)]
        force: bool,

        /// Interactive setup: ask about preferred backend, sync/prune behavior, snapshots,
        /// and starter packages, then write the answers into config.toml and local.txt.
        #[arg(short, long)]
        interactive: bool,
    },

    /// Scan every managed package across all backends for known security
    /// vulnerabilities (via the OSV.dev database)
    Audit {
        /// Output the findings as JSON
        #[arg(long)]
        json: bool,
    },

    /// Emit a CycloneDX software bill of materials (SBOM) spanning every backend
    Sbom,

    /// Export the managed package set as NATIVE manifests (Brewfile, requirements.txt,
    /// package.json, Aptfile) — the no-lock-in escape hatch and a way to interop with other
    /// tools. With no `--format`, writes every applicable file into `--out`.
    Export {
        /// One of: brew | pip | npm | apt. Omit to emit all applicable formats.
        #[arg(long)]
        format: Option<String>,

        /// Directory to write the manifest file(s) into
        #[arg(long, default_value = ".")]
        out: String,

        /// Print a single `--format` to stdout instead of writing a file
        #[arg(long)]
        stdout: bool,
    },

    /// Pack a portable, offline/air-gapped bundle of your declarative config, lockfile and
    /// resolved package list. With `--artifacts`, also pre-download package files for the
    /// backends that support offline fetch.
    Bundle {
        /// Directory to write the bundle into
        #[arg(long, default_value = "linix-bundle")]
        out: String,

        /// Also pre-download package artifacts (apt/dnf/pip/npm/brew/pacman/apk)
        #[arg(long)]
        artifacts: bool,

        /// Also pack the bundle into a single portable `<out>.tar.gz` for easy transfer
        #[arg(long)]
        archive: bool,
    },

    /// Explain why a package is installed: its provenance and what depends on it
    Why {
        /// Package name (optionally `backend:name`)
        package: String,

        /// Emit the provenance as JSON
        #[arg(long)]
        json: bool,
    },

    /// Manage system services declaratively across init systems (systemd, OpenRC, SysVinit,
    /// launchd, Windows sc). `enable`/`disable` persist to your manifest; `start`/`stop`/
    /// `restart` are one-shot controls.
    Service(ServiceArgs),

    /// Find which system snapshot first breaks a test command (system time-travel bisect).
    /// Restores snapshots and runs --test to converge on the change that introduced a
    /// regression. Filesystem-restore backends may require a reboot between steps.
    Bisect {
        /// Command whose success (exit 0) means "good" and failure means "broken"
        #[arg(long)]
        test: String,

        /// Skip the interactive confirmation before restoring snapshots
        #[arg(short, long)]
        yes: bool,
    },

    /// Compare a set of machines over SSH against your manifests and report drift
    Fleet(FleetArgs),

    /// Auto-record manual package-manager use into LiNix (native hooks + shell wrappers),
    /// so `apt install foo` (etc.) updates your declarative state without changing workflow.
    Hooks(HooksArgs),

    /// Internal: called by an installed native hook to record a transaction's targets.
    #[command(hide = true)]
    HookRecord {
        /// The package manager that ran (e.g. "pacman")
        #[arg(long)]
        manager: String,
        /// "install" or "remove"
        #[arg(long)]
        op: String,
        /// Target package names (or local file paths)
        targets: Vec<String>,
    },

    /// Internal: reconcile declarative state by diffing a manager's installed set (for hooks
    /// that cannot pass explicit targets, e.g. apt/dnf Post-Invoke).
    #[command(hide = true)]
    HookReconcile {
        /// The package manager to reconcile against
        #[arg(long)]
        manager: String,
    },

    /// Internal: observe a wrapped command line (from the shell integration) and record it,
    /// detecting the operation and targets from the arguments. `--learn` accepts unknown
    /// managers via the keyword heuristic.
    #[command(hide = true)]
    HookObserve {
        /// The manager name, if known
        #[arg(long)]
        manager: Option<String>,
        /// Learn an unknown manager from keywords in the command line
        #[arg(long)]
        learn: bool,
        /// The full observed command line, after `--`
        #[arg(last = true)]
        argv: Vec<String>,
    },

    /// Hold packages so `upgrade` never bumps them (like `apt-mark hold` / dnf versionlock).
    /// Run with no names to list current holds. Naming a held package explicitly in
    /// `upgrade <pkg>` still upgrades it (with a warning) — hold guards bulk/auto upgrades.
    Hold {
        /// Package(s) to hold (`name` or `backend:name`). Empty = list current holds.
        packages: Vec<String>,
    },

    /// Release a hold so the package can be upgraded again.
    Unhold {
        /// Package(s) to unhold (`name` or `backend:name`)
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// Detect cross-backend conflicts in your desired state: the same tool pinned to different
    /// versions by different backends, or provided by more than one (a PATH shadowing risk).
    /// Something no single-backend resolver can see. Read-only.
    Conflicts {
        /// Emit the findings as JSON
        #[arg(long)]
        json: bool,
    },

    /// Check the desired system state against your [guard] install/change rules
    Policy,

    /// Generate a shell completion script (bash, zsh, fish, powershell, elvish)
    Completions {
        /// Target shell
        shell: Shell,
    },

    /// Update LiNix itself: rebuild and install the latest from source with cargo
    /// (the same mechanism as the install script). Requires a Rust toolchain.
    SelfUpgrade {
        /// Git repository to install from (default: $LINIX_REPO, else the upstream repo)
        #[arg(long)]
        git: Option<String>,

        /// Just report the current version and where an upgrade would come from
        #[arg(long)]
        check: bool,
    },
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Write a commented default config.toml (refuses to overwrite unless --force)
    Init {
        /// Overwrite an existing config file
        #[arg(long)]
        force: bool,
    },
    /// Print the resolved configuration file path
    Path,
    /// Print the active configuration and its source (file or built-in defaults)
    Show,
    /// Open the config in $VISUAL/$EDITOR (creating it from the template if absent) and
    /// re-validate it on save, so a typo can't silently break your configuration.
    Edit,
}

#[derive(Args, Debug)]
pub struct HooksArgs {
    #[command(subcommand)]
    pub command: HooksCommand,
}

#[derive(Subcommand, Debug)]
pub enum HooksCommand {
    /// Install native package-manager hooks (writes to system hook dirs; usually needs root).
    /// With no managers named, installs every hook LiNix knows and whose manager is present.
    Install {
        /// Limit to specific managers (e.g. pacman apt dnf)
        managers: Vec<String>,
    },
    /// Remove LiNix's native hooks.
    Uninstall {
        /// Limit to specific managers
        managers: Vec<String>,
    },
    /// Show which managers are hookable and whether their hook is installed.
    Status,
    /// Print shell functions to auto-record manual manager use (source from your rc file).
    ShellInit {
        /// Target shell (bash or zsh)
        #[arg(default_value = "bash")]
        shell: String,
    },
}

#[derive(Args, Debug)]
pub struct GitArgs {
    #[command(subcommand)]
    pub command: GitCommand,
}

#[derive(Subcommand, Debug)]
pub enum GitCommand {
    /// Initialize the config directory as a git repo (enables manifest auto-commit).
    Init,
    /// Show uncommitted manifest/config changes.
    Status,
    /// Show recent manifest commits (newest first).
    Log {
        /// How many commits to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Commit the current manifest/config state now.
    Commit {
        /// Commit message
        #[arg(short, long, default_value = "linix: manual manifest commit")]
        message: String,
    },
    /// Roll the *config* (manifests) back to a past commit WITHOUT touching installed
    /// packages — the config half of a rollback. Pair with `linix rollback` for the system.
    Checkout {
        /// Commit hash or ref to restore the manifests to
        reference: String,
    },
}

#[derive(Args, Debug)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommand,
}

#[derive(Subcommand, Debug)]
pub enum ServiceCommand {
    /// Enable at boot and start now; records the service in your manifest so `sync` keeps it.
    Enable {
        /// Service name (e.g. nginx, sshd)
        name: String,
    },
    /// Disable at boot and stop now; removes it from your manifest.
    Disable {
        /// Service name
        name: String,
    },
    /// Start the service now (does not change its boot setting).
    Start {
        /// Service name
        name: String,
    },
    /// Stop the service now (does not change its boot setting).
    Stop {
        /// Service name
        name: String,
    },
    /// Restart the service now.
    Restart {
        /// Service name
        name: String,
    },
    /// Show a service's current status.
    Status {
        /// Service name
        name: String,
    },
    /// List running services this host reports.
    List,
}

#[derive(Args, Debug)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub command: RepoCommand,
}

#[derive(Subcommand, Debug)]
pub enum RepoCommand {
    /// Add a new source repository
    Add {
        name: String,
        url: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    /// Remove an existing source repository
    Remove {
        name: String,
        #[arg(short, long)]
        backend: Option<String>,
    },
    /// List all configured repositories for a backend
    List {
        #[arg(short, long)]
        backend: Option<String>,
    },
}

#[derive(Args, Debug)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommand,
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    /// List all defined profiles (a ★ marks the currently-active ones)
    List,
    /// Show the resolved package set a profile expands to (after include/exclude/-pkg)
    Show { name: String },
    /// Scaffold a new, empty profile definition file
    Create { name: String },
    /// Save the current desired state as a new standalone profile
    Save { name: String },
    /// List only the currently-active profiles
    Active,
}

#[derive(Args, Debug)]
pub struct ModuleArgs {
    #[command(subcommand)]
    pub command: ModuleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ModuleCommand {
    /// List the modules in your repo
    List,
    /// Display the contents of a module
    Show { name: String },
    /// Create a new, empty module
    Create {
        name: String,
        /// Overwrite a module that already exists
        #[arg(long)]
        force: bool,
    },
    /// Fetch a shared module into `modules/`, e.g. `linix module add github:acme/rust-dev`.
    ///
    /// This is the fetch step: `use` takes a name, never a URL, so a module from the
    /// internet lands on disk first and then you `use <name>` it like any other (II.2).
    Add {
        /// Source: `github:user/repo[@ref][/path]` or an `https://…` raw URL
        source: String,
        /// Save the module under this name (default: derived from the source)
        #[arg(long)]
        name: Option<String>,
        /// Overwrite a module that already exists
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug)]
pub struct SnapshotArgs {
    #[command(subcommand)]
    pub command: SnapshotCommand,
}

#[derive(Subcommand, Debug)]
pub enum SnapshotCommand {
    /// List all system-level snapshots
    List,
    /// Prune snapshots based on age and count limits defined in config
    Prune {
        /// Force removal without verification
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug)]
pub struct GenerationArgs {
    #[command(subcommand)]
    pub command: GenerationCommand,
}

#[derive(Subcommand, Debug)]
pub enum GenerationCommand {
    /// List saved generations (newest first)
    List,
    /// Roll back to a generation: realize its package set (and, for a full rollback,
    /// restore its manifests). Scope with `--package` / the global `--backend`.
    Rollback {
        /// Generation id (see `list`)
        id: String,
        /// Only roll back this package (name or backend:name)
        #[arg(long)]
        package: Option<String>,
    },
    /// Pin a generation so retention never deletes it
    Pin {
        /// Generation id
        id: String,
    },
    /// Remove a generation's pin
    Unpin {
        /// Generation id
        id: String,
    },
    /// Compact history, one line per generation (git-log style). Newest first.
    Log {
        /// Ultra-compact: id, package count, and label only
        #[arg(long)]
        oneline: bool,

        /// Emit the history as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show what changed between two generations: packages added, removed, and version-changed.
    /// `from` is the older baseline. Omit `to` to compare `from` against the live system.
    Diff {
        /// Older generation id (baseline)
        from: String,
        /// Newer generation id (omit to diff against the current live state)
        to: Option<String>,
        /// Emit the delta as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// Add a new background task to the system scheduler
    Add {
        /// Unique name for the scheduled task
        name: String,
        /// Cron-style execution string (e.g. "0 2 * * *")
        #[arg(long)]
        cron: String,
        /// Command to execute within LiNix (e.g. "upgrade --profile dev")
        #[arg(long)]
        command: String,
        /// Notification channel (desktop, email, or none)
        #[arg(long)]
        notification: Option<String>,
    },
    /// List all tasks currently registered in the native scheduler
    List,
    /// Remove a task from the native scheduler
    Remove { name: String },
}

#[derive(Args, Debug)]
pub struct FleetArgs {
    /// SSH destinations (user@host ...). If omitted, falls back to config `fleet_hosts`.
    pub hosts: Vec<String>,

    /// After reporting, run `linix sync` on the machines that DRIFTED to reconcile them
    #[arg(long)]
    pub sync: bool,

    /// Push `linix sync` to EVERY reachable machine, whether or not it drifted (fleet-wide apply)
    #[arg(long)]
    pub apply: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    // clap derives the value name from the variant as kebab-case (`power-shell`), but every
    // other tool — and every user — spells it `powershell` (one word). Accept the natural
    // spelling as the canonical value and keep `power-shell` as an alias for back-compat.
    #[value(name = "powershell", alias = "power-shell")]
    PowerShell,
    Elvish,
    Nushell,
}

impl Shell {
    /// Map to a `clap_complete` built-in generator. Returns `None` for shells whose
    /// generator lives in a dedicated crate (NuShell → `clap_complete_nushell`);
    /// the completions command handles those separately.
    pub fn builtin(self) -> Option<clap_complete::Shell> {
        match self {
            Shell::Bash => Some(clap_complete::Shell::Bash),
            Shell::Zsh => Some(clap_complete::Shell::Zsh),
            Shell::Fish => Some(clap_complete::Shell::Fish),
            Shell::PowerShell => Some(clap_complete::Shell::PowerShell),
            Shell::Elvish => Some(clap_complete::Shell::Elvish),
            Shell::Nushell => None,
        }
    }
}
