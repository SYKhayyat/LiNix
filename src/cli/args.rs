use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Every verb, grouped by what a person is trying to do (AU11).
///
/// **No count is written here.** It said "61" while the map held 60, because a number in prose is
/// a copy of something and nothing checked the copy. What is checked is stronger than a count:
/// `tests/help_map_tests.rs` compares this map against `--help` name for name, so a verb missing
/// from the map and a verb in the map that is not a verb are both build failures. A
/// hand-maintained list of names beside the enum, checked by nothing, is the shape that let
/// `undo` sit in two exemption lists for months after it stopped existing.
///
/// **After the flat list, not instead of it.** clap prints one `Commands:` block in
/// alphabetical-by-declaration order and has no per-subcommand heading, so the choice was a wall
/// with no map or a wall with one. Every verb still appears above, exactly once, where
/// `completions` and the coverage gates look for it — this adds orientation and removes nothing.
const COMMAND_MAP: &str = "\
The map (every command above, by what you are doing):

  Make the machine match your files
    sync · watch · plan · apply · check · rebuild · heal · try · run · shell

  Change what you declare
    install · uninstall · add · remove-orphans · unmanage · adopt · hold · unhold
    update · upgrade · repo · service

  Look at things
    list · search · info · why · diff · eval · history · protected
    policy · vars · sbom · export · repl · adapters

  Undo and time travel
    snapshot · rollback · bisect · teleport · restore · bundle · git

  Files, profiles and modules
    init · path · edit · config · module · profile · activate · deactivate
    lock · unlock · schedule · hooks

  Cleaning up
    clean-cache · purge-undeclared · reset

  Fleet and Shall itself
    fleet · completions · self-upgrade

Start with `shall init`, then `shall edit modules/starter.txt`, then `shall sync`.
`shall <command> --help` explains any one of them.";

/// Shall - declarative configuration for a machine: you edit a file saying what it
/// should have, and `sync` makes the machine match it.
#[derive(Parser, Debug)]
#[command(
    name = "shall",
    version = env!("CARGO_PKG_VERSION"),
    about = "Declare a machine - packages, services, settings, firewall, dotfiles - and sync makes it match",
    after_help = COMMAND_MAP,
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

    /// Replace files a `dotfiles:` tree would overwrite, instead of refusing
    ///
    /// The refusal exists so a home directory full of distribution defaults is not silently
    /// backed up forty times. This is the per-run acknowledgement that they are expected.
    /// Deliberately not a config key: a machine that always bypasses the check is a machine
    /// where the check does not exist.
    #[arg(long, global = true)]
    pub replace_existing: bool,

    /// Finish the packages that still can when one of them fails, instead of stopping
    ///
    /// Off by default: a plan is one change to one machine, so a member that fails makes the
    /// whole plan wrong and half-applying the rest is worse than stopping. This is the
    /// per-run opt-in for a fleet rollout that would rather take what it can get.
    ///
    /// Global, and not a flag on `sync`, because `apply`, `upgrade` and `rebuild` execute
    /// plans through the same engine — an opt-in on one command is an opt-in nobody can find
    /// from the other three.
    ///
    /// It has nothing to do with a manager this machine does not have: that is not a failure
    /// (II.7c), needs no flag, and is skipped whether or not this is set.
    #[arg(long, global = true)]
    pub keep_going: bool,

    /// Path to a preferences.toml, overriding the one in the config repo
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Use this directory as the config repo, for this run only.
    ///
    /// Outranks $SHALL_CONFIG_DIR and the stored path in Shall's settings file.
    /// `shall path` says which of the four sources won.
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,

    /// Use this directory for Shall's own state — the registry, journal and snapshots.
    ///
    /// The flag form of $SHALL_DATA_DIR, and the other half of isolating a run: --config-dir
    /// moves your files, this moves what Shall records about them. Without both, a fresh
    /// sandbox plans against the real machine's managed state. Must be absolute.
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,

    /// Carry out a removal the guard refuses: one over `max_removals`, or touching a
    /// protected/essential package. Global because every command that can delete needs
    /// it. Deliberately NOT implied by --yes: scripts and CI pass -y everywhere, and an
    /// unattended run is the one that cannot notice a system being dismantled.
    /// See `shall protected` for what is guarded and why.
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

    /// Say more about what Shall is doing: `-v` for progress, `-vv` for debug detail.
    ///
    /// An ordinary run prints its answer and nothing else. This turns the running commentary
    /// back on. `RUST_LOG` outranks it, for anyone who wants per-module control.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Quiet mode: suppress the planned-changes list, the transaction summary and every
    /// warning. Errors still print, and `-q` beats `-v` if both are given.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Ask every manager afresh, ignoring any cached installed listing.
    ///
    /// The escape hatch for `installed_cache_secs`. A cache that cannot be bypassed for one
    /// run is a cache the user has to turn off in a file and remember to turn back on, and
    /// the moment they need it is the moment they already suspect Shall is wrong about the
    /// machine. Does nothing when the cache is off, which is the default.
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Print where the run's time went: every command Shall ran, slowest first.
    ///
    /// Shall spends its life waiting on other people's processes, so "why was that slow" is
    /// almost always "which manager was slow". The report says the wall clock, the summed
    /// child time and the ratio between them — that ratio is how much of the waiting was
    /// overlapped. It goes to stderr, so `--timings` never disturbs output being parsed.
    #[arg(long, global = true)]
    pub timings: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Add, remove and update whatever your files declare until the machine matches
    Sync {
        /// Refuse to proceed unless every package is in the lock and agrees with it
        #[arg(long)]
        locked: bool,

        /// Take the versions the managers offer now, instead of the ones the lock recorded
        #[arg(long)]
        upgrade: bool,

        /// Output the transition plan as JSON (requires --dry-run)
        //
        // Enforced by `dispatch`, not by `requires = "dry_run"`. Clap resolves `requires`
        // against the subcommand's own arguments and `--dry-run` is global, so the constraint
        // compiles, never fires, and takes the flag's whole document with it when it does —
        // measured: it turned `sync --dry-run --json` into a usage error. See
        // `refuse_json_without_dry_run` (B7).
        #[arg(long)]
        json: bool,
    },

    /// Remove and reinstall what is declared, to repair state `sync` is blind to.
    ///
    /// `sync` applies the difference between your files and the machine. When a package is
    /// declared and installed but broken — a half-configured install, an interrupted download,
    /// a closure someone removed by hand — that difference is empty and `sync` does nothing.
    /// `rebuild` asserts the declared set from scratch instead. One backend at a time: all of
    /// its packages come down, then all of them go back up, then the next backend.
    ///
    /// It never touches undeclared software, and it never removes a protected package — those
    /// are named and skipped, not rebuilt.
    Rebuild {
        /// Packages to rebuild (`fd`, or `cargo:fd` to pick one backend's copy)
        packages: Vec<String>,

        /// Rebuild everything this backend declares
        #[arg(long, conflicts_with_all = ["packages", "all"])]
        backend: Option<String>,

        /// Rebuild every declared package on this machine
        #[arg(long, conflicts_with = "packages")]
        all: bool,
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
        /// Arguments forwarded to that command, verbatim
        //
        // Without this the shim could not run. `src/bin/shim.rs` builds
        // `shall run --packages X -- X <args...>` — the whole mechanism behind a `@shim=true`
        // line — and `command` was a single positional, so clap refused the invocation with
        // *unexpected argument* for anything the shimmed program was given. A user typing
        // `shall run -p jq -- jq -r .name` met the same refusal.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Recover the system from an interrupted or crashed transaction (WAL)
    Heal,

    /// Remove packages no longer needed by anything (each manager's own orphan set).
    /// Shows what it will remove and asks first; protected packages are never touched.
    #[command(name = "remove-orphans")]
    RemoveOrphans,

    /// Delete downloaded package archives and caches. Frees disk; removes no package.
    #[command(name = "clean-cache")]
    CleanCache {
        /// Also clear Shall's own download cache and extracted artifacts (X.3 level 2), not
        /// just each backend's cache. Still removes no installed package.
        #[arg(long)]
        all: bool,
    },

    /// Make Shall forget it manages anything — the registry and snapshots are deleted, and
    /// nothing on the machine is undone (X.3, level 3).
    ///
    /// This is not a cleanup. Losing the registry means Shall can no longer tell software you
    /// declared from software that was already there, which is the one distinction the whole
    /// removal model rests on. After a reset, every managed package looks unmanaged and
    /// `shall adopt` is how you get them back — by guessing. Refuses while a config repo
    /// exists unless `--force`, because forgetting the registry while the declarations remain
    /// leaves Shall believing it manages nothing and the files saying otherwise.
    Reset {
        /// Reset even though a config repo (modules/profiles) still exists.
        #[arg(long)]
        force: bool,
    },

    /// Look at the machine: drift, unmanaged software, conflicts, backend health and more
    ///
    /// One section per question. With no section it prints a line for each and names the
    /// command that acts on it. `check` only ever looks — `shall heal` is what repairs.
    Check {
        /// One of: config, drift, unmanaged, absent, conflicts, health, security
        section: Option<String>,

        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },

    /// Print the variables (Part IX) resolved on this machine — each name, its typed value,
    /// and the provider that set it. The first thing to reach for when a `when $name` block
    /// does not fire.
    Vars,

    /// Delete everything Shall does not manage. Shows the whole list first.
    ///
    /// This is the strict "make this machine exactly match my files" command. It is a
    /// command and not a setting on purpose: no config anyone can flip, inherit, or copy
    /// from a dotfiles repo makes a routine `sync` delete software it did not install.
    #[command(name = "purge-undeclared")]
    PurgeUndeclared {
        /// Proceed even though Shall manages very little of this machine — which usually
        /// means it has not been adopted yet, not that you want the rest deleted.
        #[arg(long = "allow-mass-purge")]
        allow_mass_purge: bool,
    },

    /// Stop managing a package WITHOUT uninstalling it. Shall forgets it exists; the
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

    /// List the eight ways this repo can extend Shall, and what this machine has on each:
    /// which surface, which file, whether the approval ledger cleared it, and how many rows
    /// are actually in force
    Adapters {
        /// One surface by name (`backends`, `settings`, `init`, `firewall`, `snapshot`,
        /// `secret`, `prereq`, `bootstrap`) instead of all eight
        surface: Option<String>,

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

    /// Compute what `sync` would do and freeze it to a reviewable file, so the exact plan you
    /// inspect is the one you later `apply` (Terraform-style plan/apply).
    Plan {
        /// Where to write the plan (default: shall-plan.json)
        #[arg(long, default_value = "shall-plan.json")]
        out: String,
    },

    /// Execute a previously saved plan file, applying exactly the captured changes. Warns if
    /// the system/manifests have drifted since the plan was frozen.
    Apply {
        /// Path to a plan file produced by `shall plan`
        plan: String,

        /// Apply even if the system has drifted from the captured plan
        #[arg(short, long)]
        yes: bool,
    },

    /// Freeze what a sync would otherwise decide again — on one axis, or all three.
    ///
    /// `lock versions` records the installed version of every managed package, so `sync`
    /// converges back to it and `sync --locked` reproduces it elsewhere. `lock backends`
    /// records which manager each unpinned bare name resolved to. `lock scripts` approves every
    /// hook, event hook, adapter, `exec:`, `generate:`, health-check command and `vars` provider
    /// at its current hash, without which none of them may run.
    ///
    /// With no argument, everything. `--list` shows what is locked and changes nothing.
    Lock {
        /// What to freeze: `everything` (the default), a group, a kind, or a comma-separated
        /// list of them. Narrow a kind with `kind:sub` — `versions:apt`, `hooks:after_install`.
        ///
        /// Groups: everything, packages (versions + backends), scripts (the seven approvals).
        /// Kinds: versions, backends, hooks, events, adapters, exec, generate, health, vars.
        #[arg(default_value = "everything")]
        what: String,

        /// Name(s) to scope to. Empty = every entry of whatever `what` selected.
        names: Vec<String>,

        /// Leave these out, in the same vocabulary. Repeatable.
        ///
        /// The reason the scope is a word rather than a flag: `--except versions:cargo` keeps
        /// every other manager's pins, and no per-command flag can express that.
        #[arg(long, value_name = "WHAT")]
        except: Vec<String>,

        /// List what is locked and change nothing
        #[arg(long)]
        list: bool,
    },

    /// Release a lock, so the next sync decides it again — on one axis, or all three.
    ///
    /// `unlock versions` drops the pins, so sync takes what the managers offer now. `unlock
    /// backends` forgets which manager an unpinned name resolved to — use it when a better
    /// source appears, and note that a name which then moves manager is installed from the new
    /// one and **the old copy is uninstalled**, because two of the same package is what this
    /// avoids. `unlock scripts` withdraws approval, so the next sync refuses to run them until
    /// `lock scripts` approves them again.
    ///
    /// With no axis, all three.
    Unlock {
        /// What to release, in the same vocabulary `lock` takes. A scope you can freeze with
        /// and cannot release with is a one-way door, so the two accept exactly the same words.
        #[arg(default_value = "everything")]
        what: String,

        /// Name(s) to scope to. Empty = every entry of whatever `what` selected.
        names: Vec<String>,

        /// Leave these out, in the same vocabulary. Repeatable.
        #[arg(long, value_name = "WHAT")]
        except: Vec<String>,

        /// List what is locked and change nothing
        #[arg(long)]
        list: bool,
    },

    /// Move a declared package to another package manager, then sync.
    ///
    /// `teleport ripgrep apt` rewrites wherever `ripgrep` is declared to `apt:ripgrep`, in the
    /// module it already lives in. The sync that follows installs it from `apt` and removes the
    /// old copy as drift — no second copy is left behind. It is edit-the-line-then-sync like
    /// `install`; to bring in a package that is not declared yet, use `install BACKEND:NAME`.
    Teleport {
        /// The package to move (bare name, or `oldbackend:name` to disambiguate).
        package: String,

        /// The package manager to move it to.
        backend: String,
    },

    /// Parallel search across all searchable repositories
    Search {
        /// Search query string
        query: String,

        /// Output results as JSON
        #[arg(long)]
        json: bool,

        /// Only show results that are already installed / managed by Shall
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

        /// Upgrade only packages that `shall check security` reports as vulnerable, to their fixed version
        #[arg(long)]
        security: bool,

        /// Package name(s) to hold back / exclude from this upgrade (repeatable)
        #[arg(long, value_name = "PACKAGE")]
        except: Vec<String>,

        /// Run the declared non-package steps too, on a run that named a package
        ///
        /// The `@on=upgrade` steps — firmware, `rustup`, a plugin manager — belong to "bring
        /// this whole machine forward", so a bare `shall upgrade` runs them and a run narrowed
        /// to one package, one manager or `--security` does not. This asks for them anyway;
        /// `--no-steps` declines them on a run that would otherwise have had them.
        #[arg(long, overrides_with = "no_steps")]
        steps: bool,

        /// Skip the declared non-package steps on a whole-machine upgrade
        #[arg(long = "no-steps", overrides_with = "steps")]
        no_steps: bool,

        /// Run the native whole-system upgrade even though it cannot honour holds
        ///
        /// The whole-system path runs each manager's own upgrade-all (`apt upgrade`,
        /// `npm install -g`), which Shall cannot tell to skip a package — so a held package is
        /// bumped by it. Without this flag that run is refused and names what would have moved;
        /// per-package and `--backend` upgrades honour holds and need no opt-in.
        #[arg(long)]
        ignore_holds: bool,

        /// Limit upgrade to a specific profile
        #[arg(long)]
        profile: Option<String>,

        /// Limit upgrade to a specific module
        #[arg(long)]
        module: Option<String>,

        /// Output the plan, or what was actually upgraded, as JSON
        //
        // **The one of the four that must NOT gain `requires = "dry_run"`.** All four carried
        // the sentence "(requires --dry-run)" and none enforced it (B7); three of them are
        // genuinely dry-run-only and now say so to clap. This one is not: `upgrade --security
        // --json` prints what it remediated *after* remediating it, so enforcing the sentence
        // would have deleted a working answer to make a wrong help string true. The help was
        // the thing that was wrong.
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

    /// Install one or more packages
    Install {
        /// Package strings (e.g. "apt:curl", "cargo:exa")
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        //
        // Enforced by `dispatch`, not by `requires = "dry_run"`. Clap resolves `requires`
        // against the subcommand's own arguments and `--dry-run` is global, so the constraint
        // compiles, never fires, and takes the flag's whole document with it when it does —
        // measured: it turned `sync --dry-run --json` into a usage error. See
        // `refuse_json_without_dry_run` (B7).
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

    /// Uninstall one or more packages
    Uninstall {
        /// Names of packages to uninstall
        packages: Vec<String>,

        /// Output the resulting changes as JSON (requires --dry-run)
        //
        // Enforced by `dispatch`, not by `requires = "dry_run"`. Clap resolves `requires`
        // against the subcommand's own arguments and `--dry-run` is global, so the constraint
        // compiles, never fires, and takes the flag's whole document with it when it does —
        // measured: it turned `sync --dry-run --json` into a usage error. See
        // `refuse_json_without_dry_run` (B7).
        #[arg(long)]
        json: bool,

        /// Also destroy the package's configuration (Debian's `purge`), for this run only.
        /// The machine-wide form is `[remove] purge` in preferences.toml.
        #[arg(long)]
        purge: bool,

        /// Temporary uninstall: reinstall the package(s) later. With a DURATION
        /// (e.g. `--temp=2h`) they return when it elapses; bare `--temp` inside a
        /// `shall shell` restores them when that ephemeral session ends. The duration must
        /// be attached with `=` so it is never confused with a package name.
        #[arg(long, value_name = "DURATION", num_args = 0..=1, require_equals = true)]
        temp: Option<Option<String>>,

        /// Remove it even when Shall has no record of installing it, by declaring it
        /// `absent:` — the one declaration that reaches outside what Shall manages.
        /// The line stays, so the package stays gone.
        #[arg(long, conflicts_with = "temp")]
        absent: bool,
    },

    /// Manage source repositories (PPA, Taps, Buckets, etc.)
    Repo(RepoArgs),

    /// Take over the machine: write the packages you installed by hand into a module
    Adopt {
        /// Only these backends — and these are taken even when a bare `adopt` leaves them
        /// alone. `shall adopt service` writes the machine's services; a bare `shall adopt`
        /// does not, because an init records what is running and never who chose it.
        #[arg(value_name = "BACKEND")]
        backends: Vec<String>,

        /// Only what this machine starts on its own, rather than what happens to be running
        /// right now. A backend that cannot answer that question is skipped and named, never
        /// quietly widened back to everything.
        #[arg(long)]
        enabled_only: bool,
    },

    /// Vendor someone else's modules into your repo (XIII.14).
    ///
    /// Fetches a source — `github:owner/repo`, a git URL, a raw file URL, or a local path —
    /// and copies its shareable files (`modules/`, `adapters/`, `scripts/`) into your config
    /// repo as a reviewable diff. Their `profiles/`, `active` and `priority` are left behind:
    /// those are the other machine's choices. `use` the vendored module by name afterward.
    ///
    /// Anything the source can execute (an `exec:` verb, a backend definition) arrives
    /// UNAPPROVED and does not run until `shall lock` — pass `--trust` to lock in the same
    /// step, for a source you already trust.
    Add {
        /// `github:owner/repo`, a git/https URL, a raw file URL, or a local path.
        source: String,
        /// Approve the vendored code (`exec:`, adapters) in the same step. Only for a source
        /// you have decided to trust — it skips the review the unapproved default forces.
        #[arg(long)]
        trust: bool,
        /// Overwrite a module you already have, instead of refusing on the name collision.
        #[arg(long)]
        force: bool,
    },

    /// Enter an ephemeral shell with specific packages loaded
    Shell {
        /// Packages to load into the ephemeral shell
        packages: Vec<String>,
    },

    /// Browse your manifest history: commits (left), the packages and config a commit
    /// changed (right), and a shell line (bottom). Roll back from within.
    #[command(alias = "tui")]
    History,

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
    /// whatever is no longer required by any remaining active profile. Live; no reboot.
    Deactivate {
        /// Profile name(s) to deactivate
        #[arg(required = true)]
        profiles: Vec<String>,
    },

    /// Manage system profiles / identities (list, show, create, save, active)
    Profile(ProfileArgs),

    // --- NEW FOR 3.6.0 ---
    /// Reusable declaration lists (`modules/`, referenced with `use`)
    Module(ModuleArgs),

    /// System snapshots and atomic rollbacks
    Snapshot(SnapshotArgs),

    /// Roll back to a past commit: restore the manifests it recorded, then converge the
    /// machine to match them.
    Rollback {
        /// The git commit (or ref like HEAD~1) to roll back to. See `shall git log`.
        /// Rollback checks out the manifests at that commit, then syncs the machine to match —
        /// one mechanism, no separate generation history (II.1: git IS the history).
        reference: String,
    },

    /// Show what changed between two commits, in packages (not text): the manifest lines added
    /// and removed going from `from` to `to`. Omit `to` to diff `from` against your current
    /// manifests. See `shall git log` for commit refs.
    Diff {
        /// The older commit (baseline).
        from: String,
        /// The newer commit. Omit to compare against the working tree (HEAD + uncommitted).
        to: Option<String>,
    },

    /// Rehearse this config on a clean machine in a container, and touch nothing on this one.
    ///
    /// Answers what `plan` cannot: would this config work somewhere that is not your laptop?
    /// A plan is computed against what you already have installed, so a config that only works
    /// because of something already here looks fine until it reaches the second machine.
    ///
    /// Reuses the integration images built from `docker/integration/`. Needs docker or podman;
    /// with neither it refuses rather than rehearsing on this machine.
    Try {
        /// The image to rehearse on. Defaults to the ubuntu/apt one.
        #[arg(long)]
        image: Option<String>,
    },

    /// Print the resolved desired state as JSON: every `when` decided, every bare name given a
    /// backend, every variable substituted.
    ///
    /// Answers "what did my configuration actually resolve to", which no other command does —
    /// `plan` compares that answer against the machine, and mixes the two. Takes no locks,
    /// touches no backend and changes nothing, so it is safe to run mid-sync and safe to put
    /// in a pipeline. The output carries a top-level `schema` version.
    Eval,

    /// An interactive prompt over the one resolver: resolve a name against this machine,
    /// evaluate a `when` predicate here, or inspect the resolved model — by trying, not by
    /// reading the manual. Read-only; it shares the parser and resolver `sync` uses and never
    /// touches the machine. `:help` inside for commands.
    Repl,

    /// Version-control your manifests/config directory with git: init, status, log, commit,
    /// and checkout (roll the *config* back to a past commit without touching packages).
    Git(GitArgs),

    /// Native system-level task scheduling (systemd, launchd, task-scheduler)
    Schedule(ScheduleArgs),

    /// Inspect and scaffold the Shall application configuration file
    Config(ConfigArgs),

    /// Print the config repo directory, so `cd $(shall path)` works
    Path {
        /// Also say which of --config-dir, $SHALL_CONFIG_DIR, the settings file or the
        /// built-in default decided it, and where the settings file lives
        #[arg(long)]
        explain: bool,

        /// Store this directory in Shall's settings file as the repo location, so every
        /// later run finds it without a flag or an environment variable
        #[arg(long, value_name = "DIR")]
        set: Option<PathBuf>,
    },

    /// Open the config repo — or one file in it — in $VISUAL/$EDITOR
    Edit {
        /// A file inside the repo (`priority`, `active`, `modules/dev.txt`). Without one,
        /// the repo directory itself is opened.
        file: Option<String>,
    },

    /// Scaffold the Shall repo (modules, profiles, active, priority) and a starter
    /// module, so a fresh machine is ready for `shall sync`
    Init {
        /// Reset the starter manifest even if one already exists
        #[arg(long)]
        force: bool,

        /// Interactive setup: ask about snapshot retention and starter packages, then
        /// write the answers into preferences.toml and a starter module.
        #[arg(short, long)]
        interactive: bool,
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

        /// Overwrite an existing file of the same name. Without it, an export whose
        /// filename is taken is written beside the real file instead of replacing it.
        #[arg(long)]
        force: bool,
    },

    /// Pack a portable, offline/air-gapped bundle of your declarative config, lockfile and
    /// resolved package list. With `--artifacts`, also pre-download package files for the
    /// backends that support offline fetch.
    Bundle {
        /// Directory to write the bundle into
        #[arg(long, default_value = "shall-bundle")]
        out: String,

        /// Also pre-download package artifacts (apt/dnf/pip/npm/brew/pacman/apk)
        #[arg(long)]
        artifacts: bool,

        /// Also pack the bundle into a single portable `<out>.tar.gz` for easy transfer
        #[arg(long)]
        archive: bool,
    },

    /// Restore a bundle into this machine's config: the other half of `bundle`.
    ///
    /// Copies the bundle's declarations, `locks/` and registry back. Refuses a config
    /// directory that is not empty unless `--force`, because a restore writes over what is
    /// there. It restores files; run `shall sync --locked` afterward to reproduce the exact
    /// versions.
    Restore {
        /// The bundle directory to restore from.
        dir: String,

        /// Restore even though the config directory is not empty (overwrites).
        #[arg(long)]
        force: bool,
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

    /// Find which system snapshot first breaks a test command.
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

    /// Auto-record manual package-manager use into Shall (native hooks + shell wrappers),
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

    /// Hold packages so `upgrade` does not bump them. Run with no names to list current holds.
    ///
    /// Honoured by per-package and `--backend` upgrades. The native whole-system upgrade runs
    /// each manager's own upgrade-all, which cannot be told to skip a package, so that run is
    /// refused while anything is held — `--ignore-holds` does it anyway. Naming a held package
    /// explicitly in `upgrade <pkg>` still upgrades it, with a warning.
    //
    // The claim that used to be here was *"like `apt-mark hold` / dnf versionlock"*, and both
    // of those hold across a bulk upgrade — that is the entire reason they exist. Shall's does
    // not, because it filters its own plan and then hands the whole-system path to a manager
    // that never sees the filtered plan. A `note:` on stderr under exit 0 was the only thing
    // between a user who pinned a known-good version and a weekly `shall upgrade` that moved
    // it two major versions (B9). The comparison is gone and the run is refused.
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

    /// Check the desired system state against your [guard] install/change rules
    Policy,

    /// Generate a shell completion script (bash, zsh, fish, powershell, elvish)
    Completions {
        /// Target shell
        shell: Shell,
    },

    /// Update Shall itself: rebuild and install the latest from source with cargo
    /// (the same mechanism as the install script). Requires a Rust toolchain.
    SelfUpgrade {
        /// Git repository to install from (default: $SHALL_REPO, else the upstream repo)
        #[arg(long)]
        git: Option<String>,

        /// Just report the current version and where an upgrade would come from
        #[arg(long)]
        check: bool,
    },
}

/// When a command may hold the exclusive data lock.
///
/// **Three answers, because two were never enough.** The question `writes()` asked was "does
/// this write `data/`", and the lock it decided was held for the whole process. That conflates
/// two things: *whether* a command writes, and *for how long it is entitled to stop everybody
/// else*. Six commands write state and are unbounded in time — `edit` blocks on `$EDITOR`,
/// `history` opens a TUI somebody reads at their own pace, `watch` never returns at all — and
/// each one was found separately, patched separately, and its sibling left live. `edit`,
/// `history` and `fleet` were fixed in three commits; `watch`, `shell` and `run` were not
/// touched, and `watch` is the documented GitOps deployment: while it runs, every other writing
/// Shall command on the machine waits 120 seconds and then fails.
///
/// Making it one enum is what stops the seventh instance. A new subcommand does not compile
/// until it answers, and `no_unbounded_command_holds_the_lock_for_its_lifetime` fails if an
/// answer of [`Deferred`](LockScope::Deferred) ever turns back into a whole-run lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockScope {
    /// Reads the machine, the config or a remote, and writes no state under `data/`. Takes
    /// nothing. `fleet` is here because it drives *other* machines over SSH and touches no
    /// local state at all — it took the 120-second lock for a purely remote report.
    Reader,
    /// Writes state, and its duration is the package work it performs. Holds the lock for the
    /// whole run, which is what makes a set of files that must agree agree.
    Writer,
    /// Writes state, but its duration is a person's or a loop's rather than the work's. Takes
    /// the lock **at each mutating action** and releases it in between, so the waiting is over
    /// the write and not over the reading, the typing or the sleep.
    Deferred,
}

impl Commands {
    /// Whether this command may hold the exclusive data lock, and for how long.
    ///
    /// **Exhaustive, and that is the whole point.** This was a hand-maintained `&[&str]` of
    /// twenty-one names sitting seventy lines from the enum, and its own test docstring
    /// recorded that twelve of its thirty-three entries once named commands the program did
    /// not have. Two tests guarded it and both guarded *invention*; nothing guarded omission
    /// or misclassification, which is the half that costs an entry out of `registry.json`.
    /// A new subcommand now fails to compile until it says which it is — strictly stronger
    /// than defaulting to a writer, because a default is a guess that nobody revisits.
    ///
    /// The lock is over the DATA directory. Commands that write into the CONFIG repo —
    /// `path --set`, `config init`, `edit` — are readers here and go through
    /// `utils::file::persist`, which is atomic; `edit` is the sharpest, because it blocks on
    /// `$EDITOR` and once stopped every other Shall on the machine for as long as somebody
    /// read a manifest in vim (AU6).
    ///
    /// **`--dry-run` never exempts anything** (S25): a preview of a writer reads the same
    /// state a concurrent writer is rewriting, and the run that proved it mattered was a
    /// `--dry-run sync` that entered recovery.
    pub fn lock_scope(&self) -> LockScope {
        use LockScope::{Deferred, Reader, Writer};
        match self {
            // **The duration is not the work's.** Each of these writes state and each is
            // unbounded in time, so the lock is taken where the write happens:
            //
            // - `watch` is an unbounded `loop` — the GitOps daemon, meant to be left running.
            //   It reconciles on a timer, so the lock belongs around the tick and not around
            //   the daemon. Held for the run, it disabled `install`, `sync` and the
            //   `hook-reconcile` a hand-typed `apt install` fires, for as long as the daemon
            //   was up. The user who followed the documented deployment bricked their own CLI.
            // - `shell` launches `$SHELL` and awaits it: the lock covers provisioning the
            //   session packages and tearing them down, not the session.
            // - `run` provisions what a command needs and then runs a command Shall does not
            //   own. Same shape as `shell`, one layer smaller.
            // - `history` opens a TUI a person reads for as long as they like. Its one
            //   mutating action — `HistoryAction::Rollback`, which reaches `handle_rollback` →
            //   `handle_sync`, the entire install/remove path — takes the lock where it fires.
            //   Without that, one function had two doors and only the one reached through
            //   `Commands::Rollback` was locked.
            Self::Watch { .. } | Self::Shell { .. } | Self::Run { .. } | Self::History { .. } => {
                Deferred
            }

            // Reads the machine, the config or a remote, and writes neither.
            Self::Plan { .. }
            | Self::Check { .. }
            | Self::List { .. }
            | Self::Search { .. }
            | Self::Diff { .. }
            | Self::Vars { .. }
            | Self::Export { .. }
            | Self::Sbom { .. }
            | Self::Why { .. }
            | Self::Info { .. }
            | Self::Completions { .. }
            | Self::Eval { .. }
            | Self::Repl { .. }
            | Self::Adapters { .. }
            | Self::Protected { .. }
            | Self::Policy { .. }
            | Self::Path { .. }
            | Self::Config { .. }
            | Self::Edit { .. } => Reader,

            // `plan --save` writes a plan FILE, not state, so it is not a counter-example.
            // `try` runs the rehearsal inside a container and touches nothing here.
            Self::Try { .. } => Reader,

            // **`fleet` was a writer by omission.** It drives other machines over SSH and
            // touches no local state at all, and it took the 120-second exclusive lock for a
            // purely remote report — the mirror image of `history`'s bug, from the same list.
            Self::Fleet { .. } => Reader,

            Self::Sync { .. }
            | Self::Rebuild { .. }
            | Self::Heal { .. }
            | Self::RemoveOrphans { .. }
            | Self::CleanCache { .. }
            | Self::Reset { .. }
            | Self::PurgeUndeclared { .. }
            | Self::Unmanage { .. }
            | Self::Apply { .. }
            | Self::Lock { .. }
            | Self::Unlock { .. }
            | Self::Teleport { .. }
            | Self::Update { .. }
            | Self::Upgrade { .. }
            | Self::Install { .. }
            | Self::Uninstall { .. }
            | Self::Repo { .. }
            | Self::Adopt { .. }
            | Self::Add { .. }
            | Self::Activate { .. }
            | Self::Deactivate { .. }
            | Self::Profile { .. }
            | Self::Module { .. }
            | Self::Snapshot { .. }
            | Self::Rollback { .. }
            | Self::Git { .. }
            | Self::Schedule { .. }
            | Self::Init { .. }
            | Self::Bundle { .. }
            | Self::Restore { .. }
            | Self::Service { .. }
            | Self::Bisect { .. }
            | Self::Hooks { .. }
            | Self::HookRecord { .. }
            | Self::HookReconcile { .. }
            | Self::HookObserve { .. }
            | Self::Hold { .. }
            | Self::Unhold { .. }
            | Self::SelfUpgrade { .. } => Writer,
        }
    }

    /// Does this command take the exclusive lock for its whole run?
    ///
    /// The question `main` and `run_user_verb` ask before dispatching. A [`Deferred`] command
    /// answers `false` here and takes the lock itself, at the write.
    ///
    /// [`Deferred`]: LockScope::Deferred
    pub fn writes(&self) -> bool {
        self.lock_scope() == LockScope::Writer
    }

    /// Is this a callback a package manager fires, rather than a command a person types?
    ///
    /// **Asked as a property, not enumerated in a `matches!`.** The re-entrancy stand-down in
    /// `main` matched `HookReconcile` alone — the one apt, dnf, zypper, apk, xbps, portage and
    /// eopkg invoke — and left the other two live. `HookRecord` is what Shall installs as
    /// pacman's `PostTransaction` hook and `HookObserve` is what the shell wrappers call; both
    /// are writers by [`writes`](Self::writes), so both took the 120-second exclusive lock
    /// *inside* the sync that was holding it. pacman waits on its own hook, so that is two
    /// minutes of silence per transaction on every Arch machine with `shall hooks` installed,
    /// and the record is lost at the end of it anyway.
    ///
    /// Naming a third arm would have left the same hole for a fourth subcommand. clap's name
    /// for the variant is the answer instead: everything Shall writes into a manager's hook
    /// file is spelled `hook-*` on the command line, and `every_hook_shall_installs_stands_down`
    /// asserts that against `app/pm_hooks.rs` itself rather than against this sentence.
    /// `hooks` — the verb a person types to install them — is deliberately not one.
    pub fn is_manager_hook(&self) -> bool {
        crate::core::latency::subcommand_name(self).starts_with("hook-")
    }
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Write a commented default preferences.toml (refuses to overwrite unless --force)
    Init {
        /// Overwrite an existing preferences file
        #[arg(long)]
        force: bool,
    },
    /// Print the active configuration and its source (file or built-in defaults)
    Show,
}

#[derive(Args, Debug)]
pub struct HooksArgs {
    #[command(subcommand)]
    pub command: HooksCommand,
}

#[derive(Subcommand, Debug)]
pub enum HooksCommand {
    /// Install native package-manager hooks (writes to system hook dirs; usually needs root).
    /// With no managers named, installs every hook Shall knows and whose manager is present.
    Install {
        /// Limit to specific managers (e.g. pacman apt dnf)
        managers: Vec<String>,
    },
    /// Remove Shall's native hooks.
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
        #[arg(short, long, default_value = "shall: manual manifest commit")]
        message: String,
    },
    /// Roll the *config* (manifests) back to a past commit WITHOUT touching installed
    /// packages — the config half of a rollback. Pair with `shall rollback` for the system.
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
    /// Fetch a shared module into `modules/`, e.g. `shall module add github:acme/rust-dev`.
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
    /// Pick a snapshot from the gallery and put the filesystem back to it.
    ///
    /// This is the filesystem half of going back, and it is here rather than at the top level
    /// under the name `undo` because that name did not say which of the two mechanisms it
    /// meant. The other half is the manifest history: `shall history` to browse it, `shall
    /// rollback <ref>` to go to one.
    Restore,
    /// Prune snapshots based on age and count limits defined in config
    Prune {
        /// Force removal without verification
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug)]
pub struct ScheduleArgs {
    #[command(subcommand)]
    pub command: ScheduleCommand,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCommand {
    /// Write a `schedule:` block into the `schedules` file, then sync
    Add {
        /// Unique name for the scheduled task
        name: String,
        /// Cron-style execution string (e.g. "0 2 * * *")
        #[arg(long)]
        cron: String,
        /// Shall command to run (e.g. "upgrade --profile dev")
        #[arg(long)]
        run: String,
        /// Notification channel (desktop, email, or none)
        #[arg(long)]
        notify: Option<String>,
    },
    /// List what the `schedules` file declares for this host
    List,
    /// Take a `schedule:` block out of the `schedules` file, then sync
    Remove { name: String },
}

#[derive(Args, Debug)]
pub struct FleetArgs {
    /// SSH destinations (user@host ...). If omitted, falls back to config `fleet_hosts`.
    pub hosts: Vec<String>,

    /// After reporting, run `shall sync` on the machines that DRIFTED to reconcile them
    #[arg(long)]
    pub sync: bool,

    /// Push `shall sync` to EVERY reachable machine, whether or not it drifted (fleet-wide apply)
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
