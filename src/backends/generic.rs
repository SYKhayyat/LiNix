use crate::backends::capability;
use crate::core::{
    BackendCore, CommandExecutor, Enumerable, Error, Installable, MetadataProvider, Package,
    PackageSpec, Queryable, RepoManager, Result, Searchable, Upgradable,
};
use crate::parsers::OutputParser;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// How a backend expresses an exact version at install time, for reproducible
/// (locked) installs. `{name}` / `{version}` are substituted.
///
/// **The variants say where the version goes, and nothing else.** Whether it is an *option* —
/// the one property that decides if the `--` terminator can precede it — is read off the
/// tokens by [`emits_trailing_option`](VersionPin::emits_trailing_option), because an option
/// starts with `-` and a version does not. It was a third variant for a while, and the third
/// variant is what went wrong: `Flag`, `TrailingPositional` and `RequiredFlag` had
/// character-for-character the same body, so the argv they built was identical and only the
/// *label* decided the terminator. Three backends carrying a bare positional — `luarocks`,
/// `mix`, `pub` — were spread across two of those labels, and the two that guessed wrong
/// dropped the terminator on pinned installs and kept it on unpinned ones. A fact the data
/// already states cannot be restated by hand without eventually disagreeing with itself.
#[derive(Debug, Clone)]
pub enum VersionPin {
    /// One token: apt `name=version`, pip `name==version`, bun `name@version`.
    Inline(String),
    /// Args placed **before** the name, which therefore stays behind the `--` terminator:
    /// cargo `install --version 13.0.0 -- ripgrep`. Available whenever the flag belongs to the
    /// subcommand rather than to the operand, and it is the safer of the two placements.
    ///
    /// **A leading flag cannot be batched.** `cargo install --version 1.0 --version 2.0 -- a b`
    /// is not two pinned installs, it is nonsense — so a backend using this installs one spec
    /// per command. [`After`](VersionPin::After) and [`Inline`](VersionPin::Inline) carry their
    /// version beside or inside the operand and batch fine.
    Before(Vec<String>),
    /// Args placed **after** the name: gem `jq -v 1.6` (an option), luarocks `jq 1.6`,
    /// mix `phx_new 1.6.16`, pub `webdev 2.7.0` (all three operands).
    After {
        args: Vec<String>,
        /// What to ask for when the declaration pins no version, for a manager that
        /// **refuses to install without one**.
        ///
        /// `None` for every ordinary manager: "no version" means "whatever is current", which
        /// is what an unadorned install already does. asdf is the one that needs it —
        /// `asdf install nodejs` answers `No versions specified for nodejs in config files or
        /// environment` and `asdf install nodejs latest` installs (measured, `tools` image
        /// 2026-07-29) — and `luarocks` shares the shape without the requirement.
        unpinned: Option<String>,
    },
}

impl VersionPin {
    /// A version that follows the name and is optional — the common case.
    pub fn after(args: Vec<String>) -> Self {
        VersionPin::After {
            args,
            unpinned: None,
        }
    }

    /// A version that follows the name and without which the manager will not install.
    pub fn after_required(args: Vec<String>, unpinned: &str) -> Self {
        VersionPin::After {
            args,
            unpinned: Some(unpinned.to_string()),
        }
    }

    /// Produce the install argument(s) for `name` pinned to `version`.
    fn apply(&self, name: &str, version: &str) -> Vec<String> {
        match self {
            VersionPin::Inline(tmpl) => {
                vec![tmpl.replace("{name}", name).replace("{version}", version)]
            }
            // The args go into the subcommand args, not beside the name — see the caller.
            VersionPin::Before(_) => vec![name.to_string()],
            VersionPin::After { args, .. } => {
                let mut out = vec![name.to_string()];
                out.extend(
                    args.iter()
                        .map(|f| f.replace("{name}", name).replace("{version}", version)),
                );
                out
            }
        }
    }

    /// The args this pin contributes *before* the name, if any.
    fn leading_args(&self, version: &str) -> Vec<String> {
        match self {
            VersionPin::Before(args) => args
                .iter()
                .map(|f| f.replace("{version}", version))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether this pin puts an **option** after the name.
    ///
    /// The one shape `--` cannot precede: behind a terminator, gem's `-v` stops being an
    /// option and becomes a gem name. Answered by looking at the token rather than by asking
    /// the backend author to declare it, because `-` is what "option" means to every argument
    /// parser and a declaration can be wrong.
    fn emits_trailing_option(&self) -> bool {
        match self {
            VersionPin::After { args, .. } => args.first().is_some_and(|a| a.starts_with('-')),
            _ => false,
        }
    }

    /// Whether this pin forbids batching several specs into one command.
    fn is_one_per_command(&self) -> bool {
        matches!(self, VersionPin::Before(_))
    }

    /// What this manager should be asked for when the declaration names no version.
    fn unpinned(&self) -> Option<&str> {
        match self {
            VersionPin::After {
                unpinned: Some(u), ..
            } => Some(u),
            _ => None,
        }
    }
}

/// True when a version string represents a real pin (not "latest"/"*"/empty).
fn is_concrete_version(v: &str) -> bool {
    !v.is_empty() && v != "latest" && v != "*"
}

/// How a backend answers "which packages did the user actually ask for?" — the question
/// `adopt` must get right before it adopts anything into managed state.
///
/// This is stated per backend rather than inferred from the absence of config: "no manual
/// command configured" is ambiguous between *"listing everything is the correct answer"*
/// (winget has no dependencies) and *"we have no idea"* (pip does). Conflating the two is
/// how an entire dependency graph gets adopted and then purged.
#[derive(Debug, Clone)]
pub enum ManualListing {
    /// Every installed package was user-requested *and* the manager can reinstall all of
    /// them, so `list_installed` *is* the manual set (choco, mas, dotnet).
    ///
    /// Both halves are load-bearing. winget satisfies the first and fails the second, which
    /// is why it is [`ExportFile`](Self::ExportFile) and not this.
    AllInstalled,
    /// The manager reports its explicit set via a command of its own.
    Command {
        /// Binary to run, when it is neither the backend nor `list_binary` (apt's manual
        /// set lives in `apt-mark`, a third binary distinct from its `dpkg-query` lister).
        /// `None` falls back to `list_binary`, then the backend name.
        binary: Option<String>,
        args: Vec<String>,
        format: ManualFormat,
    },
    /// The manager writes its own restorable set to a file it is handed.
    ///
    /// **The only variant that answers "what could I put back" rather than "what is here".**
    /// Every other one reads a listing of the machine; a manager whose listing includes things
    /// it cannot reinstall needs this, because adoption's whole output is declarations that
    /// have to converge later. On winget the two sets differ by 186 of 280 rows.
    ExportFile {
        /// `None` falls back to `list_binary`, then the backend name.
        binary: Option<String>,
        /// `{file}` is replaced with a path in a directory Shall owns for the call. The
        /// manager writes there; nothing reads its stdout for the set.
        args: Vec<String>,
        format: ExportFormat,
    },
    /// The manager installs dependencies but exposes no way to tell them apart from what
    /// the user chose (pip, gem, zypper, pkgin). Adoption must skip the backend entirely.
    Unsupported,
}

/// The shape of a `ManualListing::Command`'s output.
#[derive(Clone)]
pub enum ManualFormat {
    /// Same shape as `list_args` output — reuse the backend's installed parser.
    SameAsInstalled,
    /// One bare package name per line, no versions (`apt-mark showmanual`).
    BareNames,
    /// A shape of its own, read by its own function.
    ///
    /// **The manual set is a different question from the installed set, so it may have a
    /// different answer shape.** conda's is `conda env export --from-history --json`, whose
    /// `dependencies` array holds match-specs (`python=3.13`) rather than the package objects
    /// `conda list --json` returns — the same manager, two formats, and no amount of leniency
    /// in one parser should be asked to cover both. That leniency is what `Q40` was.
    Read(InstalledReader),
}

impl std::fmt::Debug for ManualFormat {
    /// A reader is a closure with no useful rendering, and this type is printed by
    /// `ManagerConfig`'s `Debug` — which `resolve_settings` scans for unresolved placeholders.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SameAsInstalled => f.write_str("SameAsInstalled"),
            Self::BareNames => f.write_str("BareNames"),
            Self::Read(_) => f.write_str("Read(..)"),
        }
    }
}

/// The shape of the file a [`ManualListing::ExportFile`] manager writes.
#[derive(Debug, Clone, Copy)]
pub enum ExportFormat {
    /// `winget export`'s JSON: `Sources[].Packages[].PackageIdentifier`.
    WingetJson,
}

impl std::fmt::Debug for MachineListing {
    /// The reader is a closure and has no useful rendering; the argv is what identifies this.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineListing")
            .field("binary", &self.binary)
            .field("args", &self.args)
            .finish_non_exhaustive()
    }
}

/// The one command that names everything this manager has an update for (`Q44`).
///
/// **A different question from `search`, not a faster route to the same answer.** Without this,
/// `list --outdated` asks each manager about one package at a time — and `Searchable::lookup`
/// defaults to a whole search for that one name, so a machine with 280 packages ran 280
/// registry searches. Measured: 771.4s, against 2.9s for the plain listing that fed it.
#[derive(Clone)]
pub struct OutdatedProbe {
    /// `None` falls back to `list_binary`, then the backend name.
    pub binary: Option<String>,
    pub args: Vec<String>,
    /// Reads the manager's answer into `(name, version available)` pairs. The installed
    /// version is not taken from here — the caller already has it, and a manager that reports
    /// its own idea of "current" has been wrong about it before.
    pub parse: PackageReader,
    /// This manager says *"nothing is out of date"* by exiting non-zero with nothing on either
    /// stream, and documents that exit as the answer.
    ///
    /// `pacman -Qu` is the one. That shape is exactly what `Q40` calls a failed read —
    /// correctly in general, and wrongly here — so it is translated back for the manager whose
    /// meaning is known, rather than by loosening the rule for every read in the program. A
    /// pacman that genuinely failed says so on stderr and still raises.
    pub silence_is_none: bool,
}

/// A machine-readable listing this manager *may* support, and how to read it (`Q43`).
///
/// **Asked for, not assumed.** Every one of these is a flag that arrived in some version of the
/// tool — `dotnet tool list --format json` needs SDK 10, `pixi global list --json` a recent
/// pixi — and Shall does not control which version is installed. Passing an unsupported flag
/// makes the command fail with a usage message, and a caller that reads that as the listing has
/// reproduced `Q40`: a manager silently reporting an empty machine, for users on older tooling
/// only, who are the least likely to notice.
///
/// So it is a negotiation. Ask; if the manager refuses, use [`ManagerConfig::list_args`] and say
/// so at `debug`. It costs one failed invocation on old tooling and nothing on current, and it
/// needs no version table to go stale. The listing memo makes "once per run" free — a run asks
/// each manager for its listing exactly once either way.
/// Reads a manager's output into packages.
///
/// An `Arc<dyn Fn>` rather than a `fn` pointer because a custom backend's parser is a runtime
/// `ParserSpec` it has to close over, and the onboarder's whole claim is that a custom backend
/// is a first-class peer of a built-in (U2). A `fn` pointer would have made these two fields
/// the exception.
pub type PackageReader = std::sync::Arc<dyn Fn(&str) -> Vec<Package> + Send + Sync>;

/// Reads a manager's output into the packages it says are **installed**, or admits it did not
/// recognise the bytes.
///
/// Separate from [`PackageReader`] because the two answer different questions and only one of
/// them is dangerous when it comes back empty. An empty *outdated* list means nothing needs
/// upgrading, which is the common case and a fact the caller can act on. An empty *installed*
/// list means the machine is bare, which the planner answers by installing every declaration
/// and dropping every removal — so a listing nobody could parse must not be able to spell
/// itself that way. See [`crate::parsers::Unrecognised`].
pub type InstalledReader =
    std::sync::Arc<dyn Fn(&str) -> crate::parsers::ParseResult + Send + Sync>;

#[derive(Clone)]
pub struct MachineListing {
    /// `None` falls back to `list_binary`, then the backend name.
    pub binary: Option<String>,
    pub args: Vec<String>,
    /// Reads what those args produce. A *different* function from the text parser, never the
    /// same one made lenient: one parser that accepts two shapes is how a malformed answer in
    /// one of them gets silently read as the other.
    ///
    /// Fallible for the same reason the text parser is. This path is *more* exposed, not less:
    /// it exists because a flag may or may not be present in the installed version of the tool,
    /// so it is the one listing whose shape Shall has already admitted it cannot predict.
    pub parse: InstalledReader,
}

/// Reads a manager's output into bare names — dependencies, and anything else that is a list
/// of names rather than of packages.
pub type NameReader = std::sync::Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// How this manager empties its download cache (`X.3` levels 1–2).
///
/// **`None` is a claim that the manager has no cache verb, not that nobody wrote one.** Until
/// 2026-08-06 it was neither: `ManagerConfig` had no field at all and `GenericUpgradable` did
/// not implement `clean_cache`, so every one of the forty data-path backends answered
/// `Unsupported`. `handle_clean_cache` filters that out silently, so `shall clean-cache` on a
/// Debian machine printed *"No backend on this machine has a cache to clear"* while
/// `/var/cache/apt/archives` sat there. Six hand-written modules had the verb; the shared
/// machinery could not express it, which is the terminator split running the other way.
#[derive(Debug, Clone)]
pub struct CacheClean {
    /// `None` falls back to the backend's own binary. Void empties its cache with
    /// `xbps-remove`, which is its *remover*, not its installer.
    pub binary: Option<String>,
    pub args: Vec<String>,
}

/// How this manager answers "what does this package need?" — **reported, never planned from**.
///
/// A probe rather than a bare argv list because the answer's shape is the manager's: dnf prints
/// one bare name per line, pacman prints several on one labelled row, apt prints one per
/// labelled line. One parser made lenient enough for all three is one parser that reads a
/// malformed answer in one shape as a valid answer in another.
#[derive(Clone)]
pub struct DependsProbe {
    /// `None` falls back to the backend's own binary. Void asks `xbps-query`, which is neither
    /// the installer it is named for nor its remover.
    pub binary: Option<String>,
    /// `{name}` is the package asked about.
    pub args: Vec<String>,
    pub parse: NameReader,
}

impl std::fmt::Debug for DependsProbe {
    /// The reader is a closure and has no useful rendering; the argv is what identifies this.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DependsProbe")
            .field("binary", &self.binary)
            .field("args", &self.args)
            .finish_non_exhaustive()
    }
}

/// What a manager's repository listing prints.
///
/// Stated per backend rather than guessed from the output, because the two shapes are not
/// distinguishable from one line: a bare name and a `name url` row whose url happens to be
/// missing look identical, and reading the first as the second silently reports every
/// repository as having no source.
#[derive(Debug, Clone)]
pub enum RepoListing {
    /// One row per repository, whose first two whitespace columns are the name and the source.
    /// Every manager with a `repo list` verb of its own.
    Columns,
    /// Bare names, one per line, and the source is a second question about one of them.
    /// `{name}` is the repository.
    ///
    /// pacman is the one: `pacman-conf --repo-list` prints names and nothing else, and
    /// `pacman-conf -r <name> Server` prints that one repository's mirror.
    NamesThenDetail(Vec<String>),
}

/// A dry run of the manager's own orphan verb, and how to read the names back out of it.
///
/// `apt-get autoremove --dry-run` prints `Remv libfoo1 [1.2-3]` per package; the prefix is
/// what separates those lines from the summary counts and the "0 upgraded" line.
#[derive(Debug, Clone)]
pub struct OrphanDryRun {
    /// Binary to run, when it is not the backend's own (apt's autoremove is `apt-get`).
    pub binary: Option<String>,
    pub args: Vec<String>,
    pub removes_line_prefix: String,
}

/// Configuration for the Generic Manager Strategy.
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub name: String,
    /// The program to run, when it is not the backend's own name (XIII.12). `None` — every
    /// built-in — means the name is the command.
    pub binary: Option<String>,
    pub install_args: Vec<String>,
    pub remove_args: Vec<String>,
    /// Optional: the program that runs the REMOVE commands, when a manager uninstalls with a
    /// *separate* binary from the one it installs with — OpenBSD installs with `pkg_add` and
    /// removes with `pkg_delete`. `None` = removal uses the same binary as everything else.
    pub remove_binary: Option<String>,
    /// Args that also destroy the package's configuration (Debian's `purge`). `None` means
    /// this manager draws no such distinction, and `--purge` on it is refused rather than
    /// quietly doing an ordinary removal.
    pub purge_args: Option<Vec<String>>,
    pub list_args: Vec<String>,
    pub manual: ManualListing,
    /// Optional: args (run with `list_binary`) that report the packages the OS treats as
    /// essential, for the removal guard. `None` = the manager has no such concept.
    pub essential_args: Option<Vec<String>>,
    pub search_args: Vec<String>,
    /// Optional: if specified, use this binary for search instead of the backend name.
    pub search_binary: Option<String>,
    /// Optional: args that print every installable package name, one per line, and nothing
    /// else — what II.15's `re:` expands against. `None` means this manager cannot list its
    /// catalogue, which is the honest answer for every language registry, and a `re:` line
    /// naming it is refused rather than expanded to nothing.
    pub enumerate_args: Option<Vec<String>>,
    /// Optional: binary for `enumerate_args`, when the catalogue lives in a separate program
    /// (apt's is `apt-cache`, not `apt`).
    pub enumerate_binary: Option<String>,
    /// Optional: binary to run the LIST commands (`list_args`/`essential_args`) with,
    /// instead of the backend name. Required when a manager's query tool is a *separate*
    /// program — e.g. apt lists installed packages via `dpkg-query`, not `apt dpkg-query`.
    pub list_binary: Option<String>,
    pub upgrade_args: Vec<String>,
    pub update_args: Option<Vec<String>>,
    /// How to ask the manager what its own orphan verb *would* remove, without removing it.
    /// `None` means this manager cannot say, and a manager that cannot say does not remove.
    pub orphan_dry_run: Option<OrphanDryRun>,
    pub repo_add_args: Option<Vec<String>>,
    pub repo_remove_args: Option<Vec<String>>,
    pub repo_list_args: Option<Vec<String>>,
    /// Optional: the program that runs `repo_add_args`/`repo_remove_args`, when a manager
    /// edits its sources with a *separate* tool. apt's is `add-apt-repository` and apk's is a
    /// line appended to a file by `sh` — neither is `apt` or `apk`, and running them as
    /// subcommands of the manager is the same defect `list_binary` exists to prevent.
    pub repo_binary: Option<String>,
    /// Optional: the program that runs `repo_list_args`. Separate from `repo_binary` because a
    /// manager can write its sources one way and read them another (apk writes with `sh` and
    /// reads with `cat`). Falls back to `binary`, not to `repo_binary`.
    pub repo_list_binary: Option<String>,
    /// Optional: the program that runs `repo_remove_args`, when a manager adds a source one way
    /// and drops it another. Falls back to [`repo_binary`](Self::repo_binary).
    ///
    /// dnf adds with its own `config-manager` plugin and has no verb that removes: the drop-in
    /// file is deleted with `rm`. Two directions of the same operation, two programs — which is
    /// the argument `repo_list_binary` already carries for reading.
    pub repo_remove_binary: Option<String>,
    /// The shape of what `repo_list_args` prints.
    pub repo_list_shape: RepoListing,
    /// How to ask this manager what one package needs. `None` means it is not asked, which is
    /// the right answer for every manager whose own installer resolves its closure.
    pub depends: Option<DependsProbe>,
    /// How to empty this manager's download cache. `None` means it has no such verb.
    pub clean_cache: Option<CacheClean>,
    /// Native syntax for pinning an exact version at install (None = no version pinning).
    pub version_pin: Option<VersionPin>,
    /// Optional: the option key holding what `install_args` takes, when that is not the
    /// package's own name. `helm plugin install` takes a URL while `plugin list` and `plugin
    /// uninstall` speak the name from the plugin's `plugin.yaml` — so the name has to stay the
    /// identity (a declaration that names the URL installs once and can never be removed or
    /// recognised again), and the URL rides in an option. `None` = the name is the argument.
    pub install_source_option: Option<String>,
    pub needs_root: bool,
    pub is_exclusive: bool,
    /// Properties `info` learns by asking the manager a second question. Empty for a manager
    /// that reports none, which is most of them.
    pub property_probes: Vec<PropertyProbe>,
    /// A machine-readable listing to try before [`Self::list_args`], where the manager may
    /// have one (`Q43`). `None` means the text listing is all there is.
    pub machine_list: Option<MachineListing>,
    /// The manager's own "what has an update" command, where it has one (`Q44`). `None` means
    /// callers must ask about each package separately.
    pub outdated: Option<OutdatedProbe>,
    /// Where `search` gets its answers. Defaults to running `search_args`.
    pub search_source: SearchSource,
    /// This manager has no upgrade-all verb: upgrading means re-installing each installed
    /// package unpinned, with THESE args rather than `install_args`.
    ///
    /// The args are separate because they are not always the same: pub re-activates with the
    /// verb it installs with, while `cargo install foo` on an already-installed foo declines
    /// and needs `install --force`. A boolean would have upgraded cargo by asking it to do
    /// nothing, and reported success. `upgrade_args` is unused when this is `Some`.
    pub upgrade_reinstall_args: Option<Vec<String>>,
    /// Programs that must ALSO be present for this backend to be usable, beyond `binary`.
    /// `None` = the binary is the whole requirement, which is true of every manager that is a
    /// program rather than a plugin of one. See [`BackendCore::is_available`].
    pub extra_probes: Option<Vec<String>>,
    // `flag_map` was here: declared once, assigned at twenty-five registration sites, and read
    // at none. It was also absent from `CustomBackendDef`, so a user could not have set it even
    // if something had read it — a field that could never carry a fact into the program.
}

impl ManagerConfig {
    /// Substitute `{setting.KEY}` and `{setting.KEY|DEFAULT}` in every argv template from this
    /// backend's `[backend_settings]` block.
    ///
    /// **The one thing the data path could not say.** `conda` is environment-scoped: every verb
    /// carries `-n <env>`, where the env is a user choice read from
    /// `backend_settings.conda.env`. A `ManagerConfig` row is fixed at registration, so an argv
    /// that depends on the machine's settings had no way to be written as data — and the answer,
    /// for 319 lines, was a hand-written backend. `backend_is_data_not_code_tests.rs` records six
    /// exemptions blocked on the same shape.
    ///
    /// **Resolved once, at registration, not per call.** `backend_settings` is read from
    /// preferences before any backend registers and does not change during a run, so a per-call
    /// substitution would be the same answer computed N times — and the hand-written backends
    /// this replaces already resolved at registration (`conda.rs`'s `resolve_env(cfg)`).
    ///
    /// A placeholder that names a key with no value and no `|DEFAULT` is a **refusal**, not an
    /// empty string: `conda list -n --json` would ask conda about a flag rather than an
    /// environment, and a manager handed a malformed argv is exactly the silent-wrong-answer
    /// shape this repo keeps finding.
    pub fn resolve_settings(
        &mut self,
        settings: Option<&std::collections::HashMap<String, String>>,
    ) -> crate::core::Result<()> {
        let backend = self.name.clone();
        let lookup = |token: &str| -> Option<String> {
            let (key, fallback) = match token.split_once('|') {
                Some((k, d)) => (k, Some(d)),
                None => (token, None),
            };
            settings
                .and_then(|s| s.get(key))
                .filter(|v| !v.trim().is_empty())
                .cloned()
                .or_else(|| fallback.map(str::to_string))
        };

        let mut missing: Vec<String> = Vec::new();
        walk_args(self, &mut |arg: &mut String| {
            *arg = substitute(arg, &lookup, &mut missing);
        });

        if !missing.is_empty() {
            return Err(crate::core::Error::Config(format!(
                "`{backend}` needs `[backend_settings.{backend}]` to set {} — its argv is written \
                 against {} and there is no default. Add the key, or the commands this backend \
                 runs would be missing an operand.",
                missing.join(", "),
                missing
                    .iter()
                    .map(|k| format!("`{{setting.{k}}}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        // **The independent check, and the reason a forgotten field cannot ship.** `walk_args`
        // is a hand-written list of every argv-bearing field, and the failure mode of a hand-
        // written list is the field somebody adds next year — a placeholder in it would reach
        // the manager *literally*, as `-n {setting.env}`. This reads the whole struct through
        // `Debug`, which cannot miss a field because it did not write the list.
        let rendered = format!("{self:?}");
        if let Some(at) = rendered.find("{setting.") {
            return Err(crate::core::Error::Config(format!(
                "`{backend}`: a `{{setting.…}}` placeholder survived resolution — \
                 `generic::walk_args` does not visit the field holding it, so it would be passed \
                 to the manager verbatim. Near: {}",
                rendered[at..].chars().take(80).collect::<String>()
            )));
        }
        Ok(())
    }
}

/// Replace every `{setting.KEY}` / `{setting.KEY|DEFAULT}` in `arg`, recording keys that resolve
/// to nothing. Substitution is *inside* the token, so `--{setting.scope|system}` is one argument.
fn substitute(
    arg: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    missing: &mut Vec<String>,
) -> String {
    const OPEN: &str = "{setting.";
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('}') else {
            break; // an unclosed placeholder is not one; the Debug scan will catch it
        };
        out.push_str(&rest[..start]);
        let token = &after[..end];
        match lookup(token) {
            Some(value) => out.push_str(&value),
            None => {
                missing.push(token.split('|').next().unwrap_or(token).to_string());
                // Left in place so the Debug scan is not the thing that reports it; the
                // `missing` list names the key, which is what the user has to act on.
                out.push_str(&rest[start..start + OPEN.len() + end + 1]);
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Visit every argv token in a [`ManagerConfig`].
///
/// Hand-written, and checked by the `Debug` scan in [`ManagerConfig::resolve_settings`] rather
/// than trusted — a list like this is exactly the kind of thing that goes stale, and the whole
/// point of the pair is that going stale is a build failure rather than a wrong command.
fn walk_args(config: &mut ManagerConfig, visit: &mut dyn FnMut(&mut String)) {
    fn each(args: &mut [String], visit: &mut dyn FnMut(&mut String)) {
        for a in args {
            visit(a);
        }
    }
    fn maybe(args: &mut Option<Vec<String>>, visit: &mut dyn FnMut(&mut String)) {
        if let Some(a) = args {
            each(a, visit);
        }
    }

    each(&mut config.install_args, visit);
    each(&mut config.remove_args, visit);
    each(&mut config.list_args, visit);
    each(&mut config.search_args, visit);
    each(&mut config.upgrade_args, visit);
    maybe(&mut config.purge_args, visit);
    maybe(&mut config.essential_args, visit);
    maybe(&mut config.enumerate_args, visit);
    maybe(&mut config.update_args, visit);
    maybe(&mut config.repo_add_args, visit);
    maybe(&mut config.repo_remove_args, visit);
    maybe(&mut config.repo_list_args, visit);
    maybe(&mut config.upgrade_reinstall_args, visit);
    maybe(&mut config.extra_probes, visit);

    match &mut config.manual {
        ManualListing::Command { args, .. } | ManualListing::ExportFile { args, .. } => {
            each(args, visit)
        }
        ManualListing::AllInstalled | ManualListing::Unsupported => {}
    }
    if let Some(m) = &mut config.machine_list {
        each(&mut m.args, visit);
    }
    if let Some(o) = &mut config.outdated {
        each(&mut o.args, visit);
    }
    if let Some(o) = &mut config.orphan_dry_run {
        each(&mut o.args, visit);
    }
    if let Some(c) = &mut config.clean_cache {
        each(&mut c.args, visit);
    }
    if let Some(d) = &mut config.depends {
        each(&mut d.args, visit);
    }
    for p in &mut config.property_probes {
        each(&mut p.args, visit);
        visit(&mut p.template);
    }
    match &mut config.version_pin {
        Some(VersionPin::Inline(s)) => visit(s),
        Some(VersionPin::Before(args)) => each(args, visit),
        Some(VersionPin::After { args, unpinned }) => {
            each(args, visit);
            if let Some(u) = unpinned {
                visit(u);
            }
        }
        None => {}
    }
}

pub struct GenericBackendCore {
    pub name: String,
    pub executor: CommandExecutor,
    pub config: ManagerConfig,
    pub parser: Arc<dyn OutputParser>,
}

impl GenericBackendCore {
    /// The program this backend runs. `name` is the prefix a line is written with, and for
    /// every built-in the two are the same word — but a user-defined noun (`firewall:`) runs
    /// something else (`ufw`), so a command position must ask for this and never for `name`
    /// (XIII.12). `list_binary`/`search_binary`/`enumerate_binary` are narrower overrides and
    /// fall back to this, not to the name.
    pub fn binary(&self) -> &str {
        self.config.binary.as_deref().unwrap_or(&self.name)
    }

    /// What this backend's mutations are exclusive over.
    ///
    /// **The manager, never the program.** OpenBSD installs with `pkg_add` and removes with
    /// `pkg_delete`, and keying on the program gave those two verbs two different locks over one
    /// package database — so a `shall` installing and a `shall` removing could hold both at
    /// once. Every hand-written backend already named its manager here; the shared machinery was
    /// the one place that named the binary.
    /// What this backend takes an exclusive lock on — the **manager**, not the backend.
    ///
    /// Usually the same string, and for most backends there is no difference: one name, one
    /// program, one database. It matters where several backends drive one manager. `pacman` and
    /// `yay` in one config is an ordinary Arch machine — the repos from one, the AUR from the
    /// other — and both write `/var/lib/pacman/`. Keyed by their own names they were two locks
    /// over one database, so a sync touching both ran them concurrently and let pacman's own
    /// `db.lck` decide, which it does by failing the loser. The same holds for `apt`/`apt-get`
    /// and for `dnf`/`yum`/`microdnf`.
    ///
    /// The family table lives in `app::stale_lock`, because *which backends share a manager
    /// lock* and *which lock is left behind when one is killed* are the same fact, and the
    /// second copy of it would be the one that went stale.
    pub fn lock_key(&self) -> &str {
        crate::app::stale_lock::lock_of(&self.name).map_or(self.name.as_str(), |l| l.holder())
    }

    /// The program that removes. Falls back to [`binary`](Self::binary), not to the name, so a
    /// user-defined noun with a separate remover still removes with the right tool.
    pub fn remove_binary(&self) -> &str {
        self.config
            .remove_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }

    /// The program that adds and removes repositories.
    pub fn repo_binary(&self) -> &str {
        self.config
            .repo_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }

    /// The program that removes a repository. Falls back to the one that adds it.
    pub fn repo_remove_binary(&self) -> &str {
        self.config
            .repo_remove_binary
            .as_deref()
            .unwrap_or_else(|| self.repo_binary())
    }

    /// The program that lists repositories.
    pub fn repo_list_binary(&self) -> &str {
        self.config
            .repo_list_binary
            .as_deref()
            .unwrap_or_else(|| self.binary())
    }
}

#[async_trait]
impl BackendCore for GenericBackendCore {
    fn name(&self) -> &str {
        &self.name
    }

    /// Every probe must be present, not just the first.
    ///
    /// A manager reached as a *plugin* of another program needs both halves: `kubectl krew …`
    /// works only because krew installed `kubectl-krew` on PATH, and a host with kubectl and no
    /// krew reported this backend READY and then failed every command with `unknown command
    /// "krew"` — including `shall update`, which refreshes every backend at once. That was
    /// found and fixed once, in a hand-written backend; expressing it here is what lets the
    /// hand-written one be deleted instead of kept for the one thing it knew.
    fn is_available(&self) -> bool {
        self.probes()
            .iter()
            .all(|p| self.executor.command_exists_sync(p))
    }

    fn probes(&self) -> Vec<String> {
        match &self.config.extra_probes {
            Some(extra) => std::iter::once(self.binary().to_string())
                .chain(extra.iter().cloned())
                .collect(),
            None => vec![self.binary().to_string()],
        }
    }

    fn needs_root(&self) -> bool {
        self.config.needs_root
    }

    // No `check_health` here. This was the better of two implementations of one sentence, and
    // the way to have one implementation is to have one — so it moved to
    // `core::manager::missing_program`, which every backend now shares, and this override was
    // deleted rather than reconciled.
}

#[async_trait]
impl MetadataProvider for GenericBackendCore {
    async fn get_dependencies(&self, name: &str) -> Result<Vec<String>> {
        let Some(probe) = &self.config.depends else {
            return Ok(vec![]);
        };

        // The operand is the argument that IS `{name}`, never one that merely contains it:
        // dnf asks with `--queryformat %{name}`, where those six characters are rpm's own
        // format language and substituting the package into them produces `%jq`.
        let mut final_args: Vec<String> = probe
            .args
            .iter()
            .filter(|a| a.as_str() != "{name}")
            .cloned()
            .collect();
        let bin = probe.binary.as_deref().unwrap_or(self.binary());
        if probe.args.iter().any(|a| a.as_str() == "{name}") {
            // A read's operand is where a leading dash is least expected and most reachable,
            // so it goes behind the terminator by the same rule the install path uses.
            crate::core::argv::push_names(&mut final_args, bin, [name]);
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        // Dependency resolution is a read-only query — never escalate with sudo.
        let output = self.executor.run_output(bin, &arg_refs, false).await?;

        Ok((probe.parse)(&output))
    }
}

/// Pull the dependency names out of a `depends` / `info --requires` report.
///
/// **Only what a dependency label introduces counts.** Most of what these commands print is not
/// a dependency: `zypper info --requires` opens with `Loading repository data...`, reports
/// `Installed      : No`, and prints a paragraph of prose — and taking the first word of every
/// non-empty line, which is what this did until 2026-07-30, yields packages called `Loading`,
/// `Reading` and `No`. The planner adds every dependency as an install node and then asks *that*
/// node for its dependencies, which returns the same three words, so the first real `zypper` run
/// in this project's history could not install anything at all: it died on a `requires` cycle
/// between three adverbs.
///
/// Two shapes are handled, and they are the two that exist:
///
/// - one dependency per labelled line — `  Depends: libc6` (apt)
/// - a labelled header and an indented block — `Requires : [4]` then the names (zypper)
///
/// A `Key : Value` row that is not a dependency label closes the block, which is what keeps
/// `Description`'s indented prose out.
pub(crate) fn parse_dependency_output(output: &str) -> Vec<String> {
    fn is_dependency_label(key: &str) -> bool {
        let k = key.trim().to_ascii_lowercase();
        k.starts_with("depends") || k.starts_with("requires") || k.starts_with("pre-depends")
    }
    // A count (`[4]`), a placeholder (`(none)`) or an empty value is a header, not a name.
    fn name_of(value: &str) -> Option<String> {
        let first = value.split_whitespace().next()?;
        if first.starts_with('[') || first.starts_with('(') || first == "<none>" {
            return None;
        }
        Some(first.to_string())
    }

    let mut deps = Vec::new();
    let mut in_block = false;
    for raw in output.lines() {
        if raw.trim().is_empty() {
            in_block = false;
            continue;
        }
        let indented = raw.starts_with([' ', '\t']);
        let trimmed = raw.trim();

        match trimmed.split_once(':') {
            Some((key, value)) if is_dependency_label(key) => {
                in_block = true;
                if let Some(n) = name_of(value) {
                    deps.push(n);
                }
            }
            // Any other top-level `Key : Value` row is metadata and ends the block. Indented
            // rows are left to fall through, because a dependency may legitimately carry a
            // colon (`libc.so.6(GLIBC_2.38)(64bit)` does not, but a path-shaped one would).
            Some(_) if !indented => in_block = false,
            _ => {
                if in_block && indented {
                    if let Some(n) = name_of(trimmed) {
                        deps.push(n);
                    }
                }
            }
        }
    }
    deps
}

pub struct GenericInstallable {
    pub core: Arc<GenericBackendCore>,
}

/// The argument `install_args` takes for a backend whose install speaks a different vocabulary
/// than its list and remove (`install_source_option`).
///
/// Refusing beats guessing: deriving `diff` from `.../helm-diff` is right often enough to be
/// trusted and wrong often enough to install a plugin under a name nothing can remove, and the
/// name lives in the plugin's own `plugin.yaml`, which cannot be read before it is fetched.
fn install_source(backend: &str, spec: &PackageSpec, key: &str) -> Result<String> {
    spec.options
        .one(key)
        .filter(|v| !v.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            crate::core::Error::Validation(format!(
            "{backend}:{name} needs `@{key}=…`. {backend} installs from that value but lists and \
             removes by name, so the declaration has to carry both: \
             `{backend}:{name}@{key}=<source>`.",
            backend = backend,
            name = spec.name,
            key = key,
        ))
        })
}

#[async_trait]
impl Installable for GenericInstallable {
    async fn install(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        // A signature check this manager does is turned off by one line at a time, so specs
        // that disagree cannot share a command: the flag on a batch would hand one line's
        // opt-out to the next, which is the global switch `@unverified` is per-line to avoid.
        if capability::unverified_arg(&self.core.name).is_some() {
            let (opted, rest): (Vec<PackageSpec>, Vec<PackageSpec>) = specs
                .iter()
                .cloned()
                .partition(crate::core::download::is_unverified);
            if !opted.is_empty() && !rest.is_empty() {
                self.install_group(&opted, sudo).await?;
                return self.install_group(&rest, sudo).await;
            }
        }
        // `@system` splits a batch for the same reason and it is a sharper one: the flag says
        // *write into the environment the OS owns*, and handing one line's permission to the
        // next would install packages into a system python that nobody said that about (`Q49`).
        if capability::os_owned_env_arg(&self.core.name).is_some() {
            let (opted, rest): (Vec<PackageSpec>, Vec<PackageSpec>) = specs
                .iter()
                .cloned()
                .partition(crate::core::download::is_system);
            if !opted.is_empty() && !rest.is_empty() {
                self.install_group(&opted, sudo).await?;
                return self.install_group(&rest, sudo).await;
            }
        }
        // A leading-flag pin belongs to the subcommand, not to an operand, so it cannot be
        // shared: `cargo install --version 1.0 --version 2.0 -- a b` is not two pinned
        // installs. One command per spec, and only for the managers that spell it that way.
        if self
            .core
            .config
            .version_pin
            .as_ref()
            .is_some_and(VersionPin::is_one_per_command)
            && specs.len() > 1
        {
            for spec in specs {
                self.install_group(std::slice::from_ref(spec), sudo).await?;
            }
            return Ok(());
        }
        self.install_group(specs, sudo).await
    }

    async fn remove(
        &self,
        names: &[String],
        sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        // Some managers (e.g. Haskell's cabal/stack) genuinely have no uninstall verb.
        // An empty `remove_args` encodes that — UNLESS the manager removes with a separate
        // binary that is itself the verb (OpenBSD's `pkg_delete <name>`, no subcommand). A
        // separate remove binary means removal is supported however few args it takes.
        if self.core.config.remove_args.is_empty() && self.core.config.remove_binary.is_none() {
            return Err(crate::core::Error::Unsupported(self.core.name.clone()));
        }
        self.run_removal(self.core.config.remove_args.clone(), names, sudo)
            .await
    }

    fn supports_purge(&self) -> bool {
        self.core.config.purge_args.is_some()
    }

    /// The pin syntax is the whole answer: a config that names one can build an install argument
    /// from a version, and a config that names none cannot (`Q53`).
    fn pins_version(&self) -> bool {
        self.core.config.version_pin.is_some()
    }

    async fn purge(
        &self,
        names: &[String],
        sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        let Some(args) = self.core.config.purge_args.clone() else {
            return Err(crate::core::Error::Unsupported(format!(
                "{} has no purge — it does not keep a package's configuration apart from the \
                 package",
                self.core.name
            )));
        };
        self.run_removal(args, names, sudo).await
    }
}

impl GenericInstallable {
    /// One install command for specs that agree about verification.
    async fn install_group(&self, specs: &[PackageSpec], sudo: bool) -> Result<()> {
        if specs.is_empty() {
            return Ok(());
        }

        let mut final_args: Vec<String> = self.core.config.install_args.clone();
        let mut names: Vec<String> = Vec::with_capacity(specs.len());
        // A pin that puts an option *after* the name it pins (`gem install jq -v 1.6`) is the
        // one shape the terminator cannot precede — behind `--` that `-v` is a package.
        let mut trailing_option = false;
        let mut leading: Vec<String> = Vec::new();
        for spec in specs {
            if let Some(key) = &self.core.config.install_source_option {
                names.push(install_source(&self.core.name, spec, key)?);
                continue;
            }
            // Honor an exact version pin (reproducible/locked installs) using the
            // backend's native syntax, when both a pin syntax and a concrete version exist.
            match (spec.options.one("version"), &self.core.config.version_pin) {
                (Some(ver), Some(pin)) if is_concrete_version(ver) => {
                    // A leading arg goes into the subcommand args, ahead of the terminator,
                    // and the name stays behind it — which is the safer of the two shapes and
                    // is available whenever the flag is an option of the verb, not the operand.
                    leading.extend(pin.leading_args(ver));
                    trailing_option |= pin.emits_trailing_option();
                    names.extend(pin.apply(&spec.name, ver));
                }
                // A manager that will not install without a version gets the one it accepts
                // for "current". Without this, `asdf:nodejs` builds `asdf install nodejs` and
                // asdf rejects it — an argv Shall constructs perfectly and the tool refuses,
                // which is E13's family.
                (_, Some(pin)) if pin.unpinned().is_some() => {
                    let fallback = pin.unpinned().unwrap_or_default().to_string();
                    // Asked of the pin, not asserted: this branch used to set the flag
                    // unconditionally, so a *fallback* version dropped the terminator even
                    // when the fallback was an operand (`asdf install nodejs latest`). Both
                    // branches now answer the same question the same way.
                    trailing_option |= pin.emits_trailing_option();
                    names.extend(pin.apply(&spec.name, &fallback));
                }
                // A concrete version with no syntax to spend it on. The planner refuses this by
                // name before anything runs (`Q53`), so reaching here means something built a
                // spec without going through a plan — and dropping the pin silently is the one
                // outcome worse than either honouring it or refusing it, because the install
                // then reports success at a version nobody asked for.
                (Some(ver), None) if is_concrete_version(ver) => {
                    return Err(crate::core::Error::Unsupported(format!(
                        "`{}` cannot install an exact version, so `{}@version={}` cannot be met{}",
                        self.core.name,
                        spec.name,
                        ver,
                        match capability::cannot_pin_reason(&self.core.name) {
                            Some(why) => format!(" — {}", why),
                            None => String::new(),
                        }
                    )));
                }
                _ => names.push(spec.name.clone()),
            }
        }
        final_args.extend(leading);

        // Before the terminator: behind `--` this is a package name.
        let opting_out = specs.iter().any(crate::core::download::is_unverified);
        if opting_out {
            if let Some(arg) = capability::unverified_arg(&self.core.name) {
                // Asked, not assumed (G-8). `--verify=false` is helm 4's flag; helm 3 rejects
                // it with `unknown flag: --verify`, so emitting it unconditionally traded
                // E11's argv defect for another one on every helm 3 in existence. The
                // subcommand chain is the non-flag prefix of what has been built so far —
                // `plugin install` for helm — because that is the help that documents it.
                let chain: Vec<String> = final_args
                    .iter()
                    .take_while(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                // Withheld only on positive evidence that the tool rejects it. `None` — no
                // such program here, or its help would not run — leaves the capability table
                // in charge, because a probe that cannot ask has learned nothing.
                //
                // And withheld SILENTLY (Q14, ruled 2026-07-30; V.104). helm 3 does not verify
                // plugins at all, so the state `@unverified` asks for is the state the machine
                // is already in: "accepted and already true" is a correct no-op, not a defect,
                // and a warning on a run that did the right thing teaches people that warnings
                // are noise. The case where silence WOULD be wrong — a tool that verifies under
                // a flag it has since renamed — is not lost: the install then fails, and
                // `verification_note` says so at the one moment the distinction matters.
                if crate::core::tool_help::accepts_flag(self.core.binary(), &chain, arg)
                    != Some(false)
                {
                    final_args.push(arg.to_string());
                }
            }
        }

        // Also before the terminator, and asked of the tool for the same reason the flag above
        // is: `--break-system-packages` arrived in pip 23.0.1, and an older pip answers
        // `no such option`, which would turn a refusal a user can act on into an argv defect
        // they cannot (`Q49`).
        let into_os_env = specs.iter().any(crate::core::download::is_system);
        if into_os_env {
            if let Some(arg) = capability::os_owned_env_arg(&self.core.name) {
                let chain: Vec<String> = final_args
                    .iter()
                    .take_while(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if crate::core::tool_help::accepts_flag(self.core.binary(), &chain, arg)
                    != Some(false)
                {
                    final_args.push(arg.to_string());
                }
            }
        }

        if trailing_option {
            final_args.extend(names);
        } else {
            crate::core::argv::push_names(&mut final_args, self.core.binary(), names);
        }

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        let outcome = if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(self.core.lock_key(), self.core.binary(), &arg_refs, sudo)
                .await
        } else {
            self.core
                .executor
                .run(self.core.binary(), &arg_refs, sudo)
                .await
        };
        // The chain the flag would ride on, for the advice below: `plugin install` for helm.
        let chain: Vec<String> = final_args
            .iter()
            .take_while(|a| !a.starts_with('-'))
            .cloned()
            .collect();
        outcome
            .map_err(|e| self.explain_verification(e, opting_out, &chain))
            .map_err(|e| self.explain_os_owned_environment(e))?;
        Ok(())
    }

    /// PEP 668, in words that name the two things a user can actually do (`Q49`).
    ///
    /// pip's own refusal is a wall of text about `--break-system-packages`, virtual
    /// environments and `pipx`, addressed to somebody typing `pip install` — not to somebody
    /// who wrote a line in a manifest and does not know which of those Shall supports. Both
    /// answers it names here are ones a declaration can hold: `pipx:` is a backend Shall ships,
    /// and `@system=true` is the flag that flips this exact refusal.
    fn explain_os_owned_environment(&self, e: crate::core::Error) -> crate::core::Error {
        if capability::os_owned_env_arg(&self.core.name).is_none() {
            return e;
        }
        let crate::core::Error::CommandFailed {
            message,
            retry,
            absent_name,
        } = e
        else {
            return e;
        };
        // The marker pip prints, and nothing else: matching on "externally managed" prose alone
        // would also catch a package whose description contains it.
        if !message.contains("externally-managed-environment") {
            return crate::core::Error::CommandFailed {
                message,
                retry,
                absent_name,
            };
        }
        crate::core::Error::CommandFailed {
            message: format!(
                "{}\n  This Python belongs to the operating system's package manager, which is \
                 what PEP 668's marker file says, and `{}` will not write into it.\n  \
                 What a declaration can do about it:\n    \
                 pipx:{name}         install it in its own environment — the tool built for \
                 this, and a backend Shall already drives\n    \
                 {backend}:{name}@system=true   write into the system Python anyway, on this \
                 line only",
                message.trim_end(),
                self.core.binary(),
                backend = self.core.name,
                // The refusal is about a set; naming one of them is enough to show the shape,
                // and a list of forty would bury the two verbs that matter.
                name = "<package>",
            ),
            retry,
            absent_name,
        }
    }

    /// A manager that refuses an unsignable source names its own flag, which no declaration
    /// can write. Point at the one that can — and only when there is one.
    fn explain_verification(
        &self,
        e: crate::core::Error,
        opting_out: bool,
        chain: &[String],
    ) -> crate::core::Error {
        let Some(flag) = capability::unverified_arg(&self.core.name) else {
            return e;
        };
        let crate::core::Error::CommandFailed {
            message,
            retry,
            absent_name,
        } = e
        else {
            return e;
        };
        // Asked, not assumed — the same probe the emission path uses, so the advice and the
        // argv cannot disagree about whether the flag went out.
        let withheld =
            crate::core::tool_help::accepts_flag(self.core.binary(), chain, flag) == Some(false);
        let note = verification_note(
            &self.core.name,
            self.core.binary(),
            flag,
            opting_out,
            withheld,
            &message,
        );
        crate::core::Error::CommandFailed {
            message: match note {
                Some(note) => format!("{}\n  {}", message.trim_end(), note),
                None => message,
            },
            retry,
            absent_name,
        }
    }

    async fn run_removal(&self, mut args: Vec<String>, names: &[String], sudo: bool) -> Result<()> {
        if names.is_empty() {
            return Ok(());
        }
        let bin = self.core.remove_binary();
        crate::core::argv::push_names(&mut args, bin, names);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(self.core.lock_key(), bin, &arg_refs, sudo)
                .await?;
        } else {
            self.core.executor.run(bin, &arg_refs, sudo).await?;
        }
        Ok(())
    }
}

pub struct GenericQueryable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Queryable for GenericQueryable {
    fn installed_cache(&self) -> (&crate::core::installed::InstalledListings, &str) {
        (self.core.executor.installed_listings(), &self.core.name)
    }

    async fn fetch_installed(&self) -> Result<Vec<Package>> {
        // Ask for the machine-readable listing first, if this manager might have one (Q43).
        // A refusal is an answer — this tool is too old — and costs one invocation per run,
        // because the listing memo means a run gets here once per manager.
        if let Some(machine) = &self.core.config.machine_list {
            let bin = machine
                .binary
                .as_deref()
                .or(self.core.config.list_binary.as_deref())
                .unwrap_or(self.core.binary());
            let args: Vec<&str> = machine.args.iter().map(|s| s.as_str()).collect();
            match self.core.executor.probe_output(bin, &args).await {
                Ok(output) => return Ok((machine.parse)(&output)?),
                Err(e) => debug!(
                    "`{} {}` was refused, so `{}` is being read from its text listing instead \
                     — an older {} that does not have the flag: {e}",
                    bin,
                    machine.args.join(" "),
                    self.core.name,
                    self.core.name,
                ),
            }
        }
        let args: Vec<&str> = self
            .core
            .config
            .list_args
            .iter()
            .map(|s| s.as_str())
            .collect();
        // Use the configured list binary if the query tool is a separate program (e.g.
        // apt -> dpkg-query); otherwise the backend's own binary.
        let bin = self
            .core
            .config
            .list_binary
            .as_deref()
            .unwrap_or(self.core.binary());
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_installed(&output)?)
    }

    async fn list_manual(&self) -> Result<Vec<Package>> {
        match &self.core.config.manual {
            ManualListing::AllInstalled => self.list_installed().await,
            ManualListing::Command {
                binary,
                args,
                format,
            } => {
                let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                let bin = binary
                    .as_deref()
                    .or(self.core.config.list_binary.as_deref())
                    .unwrap_or(self.core.binary());
                let output = self.core.executor.run_output(bin, &args, false).await?;
                Ok(match format {
                    ManualFormat::BareNames => {
                        crate::parsers::parse_bare_names(&output, &self.core.name)?
                    }
                    ManualFormat::SameAsInstalled => self.core.parser.parse_installed(&output)?,
                    ManualFormat::Read(read) => read(&output)?,
                })
            }
            ManualListing::ExportFile {
                binary,
                args,
                format,
            } => {
                self.list_from_export(binary.as_deref(), args, *format)
                    .await
            }
            // Deliberately empty, not `list_installed`. Callers gate on `tracks_manual`;
            // returning the installed set here would be a confident wrong answer.
            ManualListing::Unsupported => Ok(Vec::new()),
        }
    }

    fn tracks_manual(&self) -> bool {
        !matches!(self.core.config.manual, ManualListing::Unsupported)
    }

    fn manual_source(&self) -> String {
        match &self.core.config.manual {
            ManualListing::AllInstalled => format!(
                "everything {} installed ({0} installs no dependencies of its own)",
                self.core.name
            ),
            ManualListing::Command { binary, args, .. } => {
                let bin = binary
                    .as_deref()
                    .or(self.core.config.list_binary.as_deref())
                    .unwrap_or(self.core.binary());
                format!("{} {}", bin, args.join(" "))
            }
            ManualListing::ExportFile { binary, args, .. } => {
                let bin = binary
                    .as_deref()
                    .or(self.core.config.list_binary.as_deref())
                    .unwrap_or(self.core.binary());
                format!(
                    "{} {} — what {} can reinstall, not everything it can see",
                    bin,
                    args.join(" "),
                    self.core.name
                )
            }
            ManualListing::Unsupported => {
                format!(
                    "{} cannot tell your choices from dependencies",
                    self.core.name
                )
            }
        }
    }

    async fn essential(&self) -> Result<Vec<String>> {
        let Some(ref essential_args) = self.core.config.essential_args else {
            return Ok(Vec::new());
        };
        let args: Vec<&str> = essential_args.iter().map(|s| s.as_str()).collect();
        let bin = self
            .core
            .config
            .list_binary
            .as_deref()
            .unwrap_or(self.core.binary());
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_essential(&output))
    }

    async fn info(&self, name: &str) -> Result<Option<Package>> {
        let all = self.installed_listing().await?;
        // Windows package managers use CASE-INSENSITIVE ids, but their list output frequently
        // returns a different casing than the install id: choco installs "wget" yet lists the
        // Title "Wget", so a case-sensitive `p.name == name` misses it and the remove is
        // silently skipped (package + manifest left behind). Match case-insensitively for
        // those. winget additionally records a vendor-qualified Id ("jqlang.jq") that is
        // commonly installed/removed by its bare moniker ("jq"), so also accept the trailing
        // dot-segment. Kept scoped to Windows managers to avoid mis-matching legitimately
        // case-distinct or dotted names elsewhere (e.g. npm "socket.io").
        let b = self.core.name.as_str();
        let ci = matches!(b, "choco" | "scoop" | "winget");
        let winget = b == "winget";
        let found = all
            .iter()
            .find(|p| {
                p.name == name
                    || (ci && p.name.eq_ignore_ascii_case(name))
                    || (winget
                        && p.name
                            .rsplit('.')
                            .next()
                            .is_some_and(|s| s.eq_ignore_ascii_case(name)))
            })
            .cloned();
        let Some(mut pkg) = found else {
            return Ok(None);
        };
        for probe in &self.core.config.property_probes {
            if let Some(value) = probe.resolve(&self.core, name).await {
                pkg.properties.insert(probe.property.clone(), value);
            }
        }
        Ok(Some(pkg))
    }
}

impl GenericQueryable {
    /// Run a manager's own export and read the set back out of the file it wrote.
    ///
    /// The temp directory must outlive the read, so it is bound to a name rather than
    /// left temporary in an expression — dropping it deletes the file the manager just
    /// wrote, and the read that follows would find nothing.
    ///
    /// **A missing file is an error, never an empty set.** An export that did not run is
    /// indistinguishable from a machine with nothing on it if this returns `Ok(vec![])`, and
    /// the caller is `adopt` — which would take that as "nothing to adopt" and say so.
    async fn list_from_export(
        &self,
        binary: Option<&str>,
        args: &[String],
        format: ExportFormat,
    ) -> Result<Vec<Package>> {
        let bin = binary
            .or(self.core.config.list_binary.as_deref())
            .unwrap_or(self.core.binary());
        let dir = tempfile::Builder::new()
            .prefix("shall-export-")
            .tempdir()
            .map_err(|e| {
                Error::Io(format!(
                    "could not make a directory for `{bin}`'s export: {e}"
                ))
            })?;
        let path = dir.path().join("export.json");
        let path_arg = path.to_string_lossy().to_string();
        let rendered: Vec<String> = args
            .iter()
            .map(|a| a.replace("{file}", &path_arg))
            .collect();
        let argv: Vec<&str> = rendered.iter().map(|s| s.as_str()).collect();
        // The manager's stdout names what it declined to export, one line per package, in the
        // user's own display language. It is not the set and is not parsed for one; the count
        // below is a set difference, which no localisation changes.
        let _ = self.core.executor.run_output(bin, &argv, false).await?;
        let text = tokio::fs::read_to_string(&path).await.map_err(|e| {
            Error::Io(format!(
                "`{bin} {}` wrote no export for Shall to read ({e}). Nothing was adopted from \
                 `{}` — rather than reporting the machine as empty.",
                rendered.join(" "),
                self.core.name
            ))
        })?;
        let restorable = match format {
            ExportFormat::WingetJson => crate::parsers::windows::parse_winget_export(&text)?,
        };
        self.report_unrestorable(&restorable).await;
        Ok(restorable)
    }

    /// Say how much of the machine this manager cannot put back, and name a few.
    ///
    /// Adoption that silently drops two thirds of what `list` shows reads as a bug in
    /// adoption. It is not: those rows name things the manager can uninstall and can never
    /// install. Counted as a set difference against the listing rather than by matching the
    /// manager's own message, which is localised and would silently count zero abroad.
    async fn report_unrestorable(&self, restorable: &[Package]) {
        let Ok(installed) = self.list_installed().await else {
            return;
        };
        let keep: std::collections::HashSet<&str> =
            restorable.iter().map(|p| p.name.as_str()).collect();
        let dropped: Vec<&str> = installed
            .iter()
            .map(|p| p.name.as_str())
            .filter(|n| !keep.contains(n))
            .collect();
        if dropped.is_empty() {
            return;
        }
        let sample: Vec<&str> = dropped.iter().take(3).copied().collect();
        // `warn`, not `info`: a default run prints `warn` and above, and "two thirds of your
        // installed software is outside management" is a fact about the outcome the user just
        // asked for. At `info` it would be invisible to everyone who did not already suspect it.
        warn!(
            "{name} lists {total} installed entries and can reinstall {kept} of them, adopted \
             here as {distinct} declaration(s). The other {gone} — e.g. {sample} — are entries \
             {name} can remove but never install, so a declaration naming one could never \
             converge, and they were left out.",
            name = self.core.name,
            total = installed.len(),
            kept = installed.len() - dropped.len(),
            distinct = restorable.len(),
            gone = dropped.len(),
            sample = sample.join(", "),
        );
    }
}

impl std::fmt::Debug for OutdatedProbe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutdatedProbe")
            .field("binary", &self.binary)
            .field("args", &self.args)
            .finish_non_exhaustive()
    }
}

/// A property of an installed package that only the manager can answer, and the command that
/// asks it.
///
/// `npm prefix -g`, `pipx environment --value PIPX_HOME`, `yarn global bin`: a second command
/// whose stdout is a directory, and a template that turns it into this package's value. Six
/// hand-written backends existed largely for this — it is what `info` needed and the generic
/// queryable could not do.
///
/// A **list**, not one install path, because `shall info` prints every property a package
/// carries: npm, pnpm and yarn each report `bin_path` beside `install_path`, and collapsing
/// that to one probe would have quietly removed a line from a user's output.
#[derive(Debug, Clone)]
pub struct PropertyProbe {
    /// The property key, as `shall info` prints it — `install_path`, `bin_path`.
    pub property: String,
    /// Argv run against the backend's binary; its stdout is the base value.
    pub args: Vec<String>,
    /// `{base}` and `{name}` substituted. `{base}` alone is a legitimate template: `bin_path`
    /// is the directory itself, with no per-package component.
    pub template: String,
}

impl PropertyProbe {
    /// `None` rather than an error on every failure path: these are enrichment for `shall
    /// info`, and a manager that will not answer must not turn a working `info` into a failed
    /// one — which is what the hand-written backends did, each in its own words.
    async fn resolve(&self, core: &GenericBackendCore, name: &str) -> Option<String> {
        let args: Vec<&str> = self.args.iter().map(String::as_str).collect();
        let base = core
            .executor
            .run_output(core.binary(), &args, false)
            .await
            .ok()?;
        let base = base.trim();
        if base.is_empty() {
            return None;
        }
        Some(
            self.template
                .replace("{base}", base)
                .replace("{name}", name),
        )
    }
}

/// Where a manager's `search` answers come from.
///
/// Not every manager has one. npm's is slow and output-unstable, pnpm has none, and yarn
/// removed its own in Berry — all three resolve from the same npm registry, which is why
/// `node_registry.rs` exists and why three separate backends reached for it. A search that is
/// an HTTP call rather than a subcommand is still this backend's search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSource {
    /// Run `search_args` and parse stdout. Every manager that has a real search subcommand.
    Command,
    /// Query the public npm registry over HTTP, tagging results with this backend's name.
    NpmRegistry,
    /// Look the exact name up on PyPI over HTTP.
    ///
    /// pip's own `search` was disabled upstream — PyPI withdrew the XML-RPC endpoint over
    /// abuse — and there is no public full-text replacement, so this is name resolution rather
    /// than discovery. That is a smaller answer than a real search and still this backend's
    /// search; answering "not configured" instead would refuse a question pip can answer.
    PyPi,
}

pub struct GenericSearchable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Searchable for GenericSearchable {
    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        match self.core.config.search_source {
            SearchSource::NpmRegistry => {
                return crate::backends::node_registry::registry_search(query, &self.core.name, 25)
                    .await
            }
            SearchSource::PyPi => {
                return crate::backends::pip_search::registry_search(query, &self.core.name).await
            }
            SearchSource::Command => {}
        }
        let bin = self
            .core
            .config
            .search_binary
            .as_deref()
            .unwrap_or(self.core.binary());
        // A backend reachable as `Searchable` only because it declared an outdated verb has
        // no search command, and running its binary with a bare query would be a different
        // command entirely. "Not configured" is refused by name, never answered as "no
        // results" — the same distinction U2 draws for every other optional capability.
        if self.core.config.search_args.is_empty() {
            return Err(Error::Validation(format!(
                "`{}` was not told how to search, so it cannot answer what it carries. Add                  `search_args` to its definition.",
                self.core.name
            )));
        }
        let mut owned: Vec<String> = self.core.config.search_args.clone();
        crate::core::argv::push_names(&mut owned, bin, [query]);
        let args: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
        let output = self.core.executor.search_output(bin, &args, false).await?;
        Ok(self.core.parser.parse_search(&output))
    }

    async fn outdated_all(&self) -> Result<Option<Vec<Package>>> {
        let Some(probe) = &self.core.config.outdated else {
            return Ok(None);
        };
        let bin = probe
            .binary
            .as_deref()
            .or(self.core.config.list_binary.as_deref())
            .unwrap_or(self.core.binary());
        let args: Vec<&str> = probe.args.iter().map(|s| s.as_str()).collect();
        // `run_output`, not `probe_output`: several managers report "there are updates" with a
        // non-zero exit — `dnf check-update` returns 100 — and that is an answer, not a fault.
        // A genuine failure still arrives as an error, because Q40 made silence one.
        let output = match self.core.executor.run_output(bin, &args, false).await {
            Ok(o) => o,
            // Asked, and the manager's documented way of saying "none". `Some(vec![])` and not
            // `None`: `None` would send the caller round the per-package path for an answer it
            // already has, which is the 771s `Q44` measured.
            Err(e) if probe.silence_is_none && e.to_string().contains("no output") => {
                return Ok(Some(Vec::new()))
            }
            Err(e) => return Err(e),
        };
        Ok(Some((probe.parse)(&output)))
    }
}

pub struct GenericEnumerable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Enumerable for GenericEnumerable {
    async fn available_names(&self) -> Result<Vec<String>> {
        let Some(args) = &self.core.config.enumerate_args else {
            return Err(Error::Other(format!(
                "`{}` cannot list every package it could install.",
                self.core.name
            )));
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let bin = self
            .core
            .config
            .enumerate_binary
            .as_deref()
            .unwrap_or(self.core.binary());
        let output = self.core.executor.run_output(bin, &args, false).await?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }
}

pub struct GenericUpgradable {
    pub core: Arc<GenericBackendCore>,
}

#[async_trait]
impl Upgradable for GenericUpgradable {
    async fn update(&self, sudo: bool) -> Result<()> {
        if let Some(ref update_args) = self.core.config.update_args {
            let args: Vec<&str> = update_args.iter().map(|s| s.as_str()).collect();
            self.core
                .executor
                .run(self.core.binary(), &args, sudo)
                .await?;
        }
        Ok(())
    }

    async fn upgrade(&self, sudo: bool) -> Result<()> {
        // A manager with no upgrade-all verb upgrades by re-installing what is there. `dart pub
        // global activate <name>` with no version moves that package to latest, and running it
        // over the installed list is the whole of "upgrade everything" for pub. Expressed here
        // rather than in a module, because the shape is the manager's, not the module's.
        if let Some(reinstall_args) = self.core.config.upgrade_reinstall_args.clone() {
            let installed = GenericQueryable {
                core: self.core.clone(),
            }
            .fetch_installed()
            .await?;
            // The install path, with the reinstall verb swapped in — so the terminator rule,
            // the exclusivity rule and every other thing `install` knows apply here too. A
            // second hand-rolled loop is how the six modules this replaces each got their own
            // slightly different one.
            let mut cfg = self.core.config.clone();
            cfg.install_args = reinstall_args;
            let reinstaller = Arc::new(GenericBackendCore {
                name: self.core.name.clone(),
                executor: self.core.executor.duplicate(),
                config: cfg,
                parser: self.core.parser.clone(),
            });
            let specs: Vec<PackageSpec> = installed
                .into_iter()
                .map(|pkg| PackageSpec {
                    name: pkg.name,
                    backend: self.core.name.clone(),
                    ..Default::default()
                })
                .collect();
            if specs.is_empty() {
                return Ok(());
            }
            let installer = GenericInstallable {
                core: reinstaller.clone(),
            };
            // One command for the lot — `npm install -g a b c` is one resolution and one
            // registry conversation, where forty global packages meant forty of each (`Q46`).
            //
            // **The per-package loop below is not dead code, and removing it would lose the
            // property the old loop existed for**: one package that will not reinstall must
            // not stop the other thirty-nine. So the batch is tried first and the loop is what
            // happens when it fails — the fast path in the ordinary case, the isolating path
            // exactly when something is wrong, which is the only time it was ever worth paying
            // for.
            if installer.install(&specs, sudo).await.is_ok() {
                return Ok(());
            }
            debug!(
                "`{}` could not upgrade {} package(s) in one command; re-trying them                  individually so one bad package does not hold up the rest",
                self.core.name,
                specs.len()
            );
            for spec in specs {
                // Deliberately not `?`: the executor has already reported which one and why.
                let _ = installer.install(&[spec], sudo).await;
            }
            return Ok(());
        }
        let args: Vec<&str> = self
            .core
            .config
            .upgrade_args
            .iter()
            .map(|s| s.as_str())
            .collect();
        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(self.core.lock_key(), self.core.binary(), &args, sudo)
                .await?;
        } else {
            self.core
                .executor
                .run(self.core.binary(), &args, sudo)
                .await?;
        }
        Ok(())
    }

    async fn list_orphans(&self) -> Result<Vec<String>> {
        let Some(dry) = &self.core.config.orphan_dry_run else {
            return Err(crate::core::Error::Unsupported(self.core.name.clone()));
        };
        let args: Vec<&str> = dry.args.iter().map(String::as_str).collect();
        let binary = dry.binary.as_deref().unwrap_or(self.core.binary());
        let out = self.core.executor.run_output(binary, &args, false).await?;
        Ok(out
            .lines()
            .filter_map(|l| l.trim().strip_prefix(&dry.removes_line_prefix))
            .filter_map(|rest| rest.split_whitespace().next())
            .map(|n| n.to_string())
            .collect())
    }

    async fn clean_cache(&self, sudo: bool) -> Result<()> {
        let Some(clean) = &self.core.config.clean_cache else {
            return Err(crate::core::Error::Unsupported("cache cleaning".into()));
        };
        let bin = clean.binary.as_deref().unwrap_or(self.core.binary());
        let args: Vec<&str> = clean.args.iter().map(String::as_str).collect();
        info!("{}: clearing the package cache...", self.core.name);
        // Exclusive on the manager for the same reason an install is: emptying the cache and
        // filling it are the same directory, and `is_exclusive` is the row that says so.
        if self.core.config.is_exclusive {
            self.core
                .executor
                .run_exclusive(self.core.lock_key(), bin, &args, sudo)
                .await?;
        } else {
            self.core.executor.run(bin, &args, sudo).await?;
        }
        Ok(())
    }
}

pub struct GenericRepoManager {
    pub core: Arc<GenericBackendCore>,
}

/// Some backends interpolate repo `{name}`/`{url}` into `sh -c` strings
/// (e.g. apk/apt). Reject shell metacharacters so a crafted argument cannot break out
/// of the intended command.
/// What to add to a manager's own verification refusal, if anything.
///
/// Pure, and enumerated over every case by `verification_advice_tests`, because the wrong
/// sentence here is a message that asks the user for exactly what they already wrote. That is
/// the user-visible half of N-4: `tool_help::accepts_flag` withholds a flag the installed tool
/// does not document, and the advice was `Add @unverified to the line` regardless — so a helm
/// with no way to skip verification answered a refusal by naming a flag it would drop.
fn verification_note(
    backend: &str,
    binary: &str,
    flag: &str,
    opting_out: bool,
    withheld: bool,
    message: &str,
) -> Option<String> {
    if opting_out {
        // The flag went out and the manager still refused: its own words are the whole story,
        // and Shall has nothing to add.
        if !withheld {
            return None;
        }
        return Some(format!(
            "the line already says `@unverified`, and this `{}` has no flag that turns its \
             verification off — `{}` is not in its help, so Shall did not send it. There is \
             nothing to add to the line: this version cannot install a source it cannot verify.",
            binary, flag
        ));
    }
    // Only a refusal *about verification* earns either sentence. A network failure carrying no
    // such word is not answered with advice about signatures.
    if !message.to_lowercase().contains("verification") {
        return None;
    }
    if withheld {
        return Some(format!(
            "{} checks a signature before it installs, this source carries none, and this \
             version has no flag that skips the check — `{}` is not in its help. `@unverified` \
             would not help here.",
            backend, flag
        ));
    }
    Some(format!(
        "{} checks a signature before it installs, and this source carries none. Add \
         `@unverified` to the line to accept it as-is.",
        backend
    ))
}

#[cfg(test)]
mod verification_advice_tests {
    use super::verification_note;

    const REFUSAL: &str = "Error: plugin source does not support verification";

    /// Four cases, enumerated, because the interesting one was reachable and unwritten: a tool
    /// that does not document the flag got the same "Add `@unverified`" advice as one that does,
    /// and a user who *had* written `@unverified` got no explanation at all (N-4).
    #[test]
    fn advice_never_names_a_flag_the_tool_would_drop() {
        // The line does not opt out, and the tool accepts the flag: name it.
        let accepted = verification_note("helm", "helm", "--verify=false", false, false, REFUSAL)
            .expect("a verification refusal earns advice");
        assert!(
            accepted.contains("Add `@unverified`"),
            "the one case where the flag helps must name it: {accepted}"
        );

        // The line does not opt out, and the tool would drop the flag: do not name it.
        let withheld = verification_note("helm", "helm", "--verify=false", false, true, REFUSAL)
            .expect("a verification refusal still earns an explanation");
        assert!(
            !withheld.contains("Add `@unverified`"),
            "advised a flag this tool does not document, which Shall would withhold: {withheld}"
        );
        assert!(
            withheld.contains("--verify=false") && withheld.contains("not in its help"),
            "the explanation must say which flag is missing and from where: {withheld}"
        );

        // The line already opts out and the flag went out: the manager's own words stand.
        assert_eq!(
            verification_note("helm", "helm", "--verify=false", true, false, REFUSAL),
            None,
            "nothing to add when the flag was sent and the manager refused anyway"
        );

        // The line already opts out and the flag was withheld: say so, and do not ask for it.
        let already = verification_note("helm", "helm", "--verify=false", true, true, REFUSAL)
            .expect("a withheld flag must be explained to someone who asked for it");
        assert!(
            already.contains("already says `@unverified`"),
            "a user who wrote the flag must be told it never went out: {already}"
        );
        assert!(
            !already.contains("Add `@unverified`"),
            "asked the user for what they already wrote: {already}"
        );
    }

    /// And the limit on both: a failure that is not about verification is not answered with
    /// advice about signatures.
    #[test]
    fn an_unrelated_failure_earns_no_verification_advice() {
        for withheld in [false, true] {
            assert_eq!(
                verification_note(
                    "helm",
                    "helm",
                    "--verify=false",
                    false,
                    withheld,
                    "Error: could not resolve host github.com"
                ),
                None,
                "a network failure was answered with advice about signatures (withheld={withheld})"
            );
        }
    }
}

fn reject_shell_meta(field: &str, value: &str) -> Result<()> {
    if value.chars().any(|c| {
        matches!(
            c,
            '\'' | '"' | '`' | '$' | ';' | '&' | '|' | '<' | '>' | '\n' | '\r' | '\\'
        )
    }) {
        return Err(crate::core::Error::Other(format!(
            "Unsafe characters in repo {}: '{}'",
            field, value
        )));
    }
    Ok(())
}

/// A repository name that is about to become part of a FILE PATH.
///
/// `{name}` is an argument a manager parses; `{name_component}` is a path segment Shall builds —
/// `/etc/yum.repos.d/<name>.repo`, `/etc/pacman.d/shall-<name>.conf` — and the difference is
/// that `../../../etc/cron.d/x` is a perfectly ordinary argument and a directory escape. Both
/// hand-written modules validated it and the shared repo path did not, because until dnf and
/// pacman became rows no row put a name in a path.
///
/// Deliberately narrower than [`reject_shell_meta`], which passes `/` and `..`: this is the set
/// a repository is actually named from. apt's PPA identifiers (`ppa:git-core/ppa`) carry a
/// colon and a slash and are *arguments*, not paths — they use `{name}` and are unaffected.
fn validate_path_component(backend: &str, name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        return Ok(());
    }
    Err(crate::core::Error::Validation(format!(
        "`{backend}` writes a repository into a file named after it, so `{name}` has to be a \
         single path segment: letters, digits, `-`, `_` and `.` only. Refusing."
    )))
}

/// The first `{placeholder}` still standing in a template argument, if any.
fn find_placeholder(s: &str) -> Option<String> {
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        // An unclosed `{` ends the search: everything after it is one unterminated run, so
        // there is no later `}` that could close a different placeholder.
        let close = after.find('}')?;
        let inner = &after[..close];
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            return Some(format!("{{{}}}", inner));
        }
        rest = &after[close..];
    }
    None
}

/// Refuse an argv that still carries a template placeholder.
///
/// apk's removal row is `sed -i '\|{url}|d' /etc/apk/repositories`. With `{url}` never filled,
/// sed searched for the literal text `{url}`, matched nothing, and **exited 0** — so `run()`
/// saw success and Shall reported a repository removed that was still there. An unfilled
/// placeholder has to be a loud failure, or the next row with a new placeholder repeats that
/// silently.
fn reject_unsubstituted(backend: &str, args: &[String]) -> Result<()> {
    for a in args {
        if let Some(ph) = find_placeholder(a) {
            return Err(crate::core::Error::Other(format!(
                "the `{}` backend's repository command still contains `{}` after substitution, \
                 so it would run against the literal text. Refusing. This is a defect in the \
                 backend definition, not in what you asked for.",
                backend, ph
            )));
        }
    }
    Ok(())
}

impl GenericRepoManager {
    /// The URL of the repository the user named.
    ///
    /// A few managers know a repository only by its URL — `gem sources -r <url>`, apk's line in
    /// `/etc/apk/repositories` — so their removal rows carry `{url}` while the caller has one
    /// identifier. Ask the manager's own listing first; a manager whose listing is the URL
    /// itself (apk prints one field per line, which `list_repos` cannot read as a pair) leaves
    /// the identifier as the only thing that can be it.
    async fn url_for(&self, ident: &str) -> Result<String> {
        if let Ok(repos) = self.list_repos().await {
            if let Some((_, url)) = repos.iter().find(|(n, _)| n == ident) {
                return Ok(url.clone());
            }
            if repos.iter().any(|(_, url)| url == ident) {
                return Ok(ident.to_string());
            }
        }
        if ident.contains("://") || ident.starts_with('/') {
            return Ok(ident.to_string());
        }
        Err(crate::core::Error::Other(format!(
            "`{backend}` identifies a repository by its URL, and `{ident}` is neither a URL nor \
             a name `{backend}` reports.\n  \
             Run `shall repo list -b {backend}` and pass the source exactly as it appears there.",
            backend = self.core.name,
            ident = ident
        )))
    }
}

#[async_trait]
impl RepoManager for GenericRepoManager {
    async fn add_repo(&self, name: &str, url: &str, sudo: bool) -> Result<()> {
        reject_shell_meta("name", name)?;
        reject_shell_meta("url", url)?;
        let base_args = self.core.config.repo_add_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository addition not supported for this backend".into())
        })?;

        let mut final_args = Vec::new();
        let component = if base_args.iter().any(|a| a.contains("{name_component}")) {
            validate_path_component(&self.core.name, name)?;
            Some(name)
        } else {
            None
        };
        for arg in base_args {
            let filled = arg
                .replace("{name_component}", component.unwrap_or_default())
                .replace("{name}", name)
                .replace("{url}", url);
            final_args.push(filled);
        }
        reject_unsubstituted(&self.core.name, &final_args)?;

        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();
        info!("Repo: Adding {} to {}...", name, self.core.name);
        self.core
            .executor
            .run(self.core.repo_binary(), &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn remove_repo(
        &self,
        name: &str,
        sudo: bool,
        _reaped: crate::app::sync::guard::Reaped,
    ) -> Result<()> {
        reject_shell_meta("name", name)?;
        let base_args = self.core.config.repo_remove_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository removal not supported for this backend".into())
        })?;

        // Resolved before anything runs: the URL comes from the manager's own listing or from
        // the identifier, and either way it can land inside an `sh -c` string.
        let url = if base_args.iter().any(|a| a.contains("{url}")) {
            let resolved = self.url_for(name).await?;
            reject_shell_meta("url", &resolved)?;
            Some(resolved)
        } else {
            None
        };

        let component = if base_args.iter().any(|a| a.contains("{name_component}")) {
            validate_path_component(&self.core.name, name)?;
            Some(name)
        } else {
            None
        };
        let final_args: Vec<String> = base_args
            .iter()
            .map(|a| {
                let filled = a
                    .replace("{name_component}", component.unwrap_or_default())
                    .replace("{name}", name);
                match &url {
                    Some(u) => filled.replace("{url}", u),
                    None => filled,
                }
            })
            .collect();
        reject_unsubstituted(&self.core.name, &final_args)?;
        let arg_refs: Vec<&str> = final_args.iter().map(|s| s.as_str()).collect();

        self.core
            .executor
            .run(self.core.repo_remove_binary(), &arg_refs, sudo)
            .await?;
        Ok(())
    }

    async fn list_repos(&self) -> Result<Vec<(String, String)>> {
        let base_args = self.core.config.repo_list_args.as_ref().ok_or_else(|| {
            crate::core::Error::Other("Repository listing not supported for this backend".into())
        })?;
        let arg_refs: Vec<&str> = base_args.iter().map(|s| s.as_str()).collect();
        let output = self
            .core
            .executor
            .run_output(self.core.repo_list_binary(), &arg_refs, false)
            .await?;

        // Bare names, and the source is a separate question per name. Asked once each, and a
        // name whose detail cannot be read keeps its row with an empty source: a repository
        // Shall cannot describe is still a repository the user has, and dropping it here would
        // make `repo remove` unable to find something `repo list` was hiding.
        if let RepoListing::NamesThenDetail(detail) = &self.core.config.repo_list_shape {
            let mut repos = Vec::new();
            for name in output.lines().map(str::trim).filter(|l| !l.is_empty()) {
                let filled: Vec<String> =
                    detail.iter().map(|a| a.replace("{name}", name)).collect();
                let refs: Vec<&str> = filled.iter().map(String::as_str).collect();
                let source = self
                    .core
                    .executor
                    .run_output(self.core.repo_list_binary(), &refs, false)
                    .await
                    .ok()
                    .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
                    .unwrap_or_default();
                repos.push((name.to_string(), source));
            }
            return Ok(repos);
        }

        let mut repos = Vec::new();
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Skip dashed separator rows ("--------").
            if trimmed.chars().all(|c| c == '-' || c == '=') {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            // Skip obvious table headers (e.g. winget "Name Argument Explicit",
            // scoop "Name Source Updated", dnf "repo id  repo name  status") so they don't show
            // up as repositories.
            let is_header = matches!(
                parts[0],
                "Name" | "NAME" | "Repository" | "Repo" | "Bucket" | "Source" | "repo"
            ) && matches!(
                parts[1],
                "Argument" | "URL" | "Url" | "Source" | "Updated" | "Explicit" | "Enabled" | "id"
            );
            if is_header {
                continue;
            }
            repos.push((parts[0].to_string(), parts[1].to_string()));
        }
        Ok(repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::executor::{CommandExecutor, DryRunOutput, MockExecutor};
    use crate::parsers::LambdaParser;
    use dashmap::DashMap;

    fn apt_like_core(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericBackendCore {
        let exec = CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
        apt_like_core_named("apt", exec)
    }

    fn apt_like_core_named(name: &str, exec: CommandExecutor) -> GenericBackendCore {
        GenericBackendCore {
            name: name.into(),
            executor: exec,
            config: ManagerConfig {
                name: name.into(),
                binary: None,
                remove_binary: None,
                install_args: vec![],
                remove_args: vec![],
                list_args: vec![],
                manual: ManualListing::AllInstalled,
                essential_args: None,
                search_args: vec![],
                search_binary: None,
                enumerate_args: None,
                enumerate_binary: None,
                list_binary: None,
                upgrade_args: vec![],
                update_args: None,
                purge_args: None,
                orphan_dry_run: None,
                repo_add_args: None,
                repo_remove_args: None,
                repo_list_args: None,
                repo_binary: None,
                repo_list_binary: None,
                repo_remove_binary: None,
                repo_list_shape: RepoListing::Columns,
                depends: Some(DependsProbe {
                    binary: None,
                    args: vec![
                        "depends".into(),
                        "--no-recommends".into(),
                        "--no-suggests".into(),
                        "{name}".into(),
                    ],
                    parse: Arc::new(parse_dependency_output),
                }),
                clean_cache: None,
                version_pin: None,
                needs_root: true, // apt needs root for writes — but reads must NOT escalate
                is_exclusive: true,
                install_source_option: None,
                extra_probes: None,
                upgrade_reinstall_args: None,
                property_probes: Vec::new(),
                machine_list: None,
                outdated: None,
                search_source: SearchSource::Command,
            },
            parser: Arc::new(LambdaParser {
                installed_fn: |_| Ok(vec![]),
                search_fn: |_| vec![],
            }),
        }
    }

    #[tokio::test]
    async fn get_dependencies_parses_names_without_sudo() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        // Respond to the NON-sudo command; if get_dependencies escalated, this wouldn't
        // match and the result would be empty.
        mock.set_response(
            "apt depends --no-recommends --no-suggests -- curl",
            Ok(DryRunOutput {
                stdout: b"Depends: libc6\nDepends: bash\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let core = apt_like_core(mock.clone(), vfs);
        let deps = core.get_dependencies("curl").await.unwrap();
        // "Depends: libc6" -> "libc6" (label + constraints stripped)
        assert_eq!(deps, vec!["libc6".to_string(), "bash".to_string()]);
        // And the name went behind the terminator, like every other operand this program
        // sends. A read is where a leading dash is least expected and most reachable — the
        // package asked about comes from a declaration, the same place an install's does.
        assert_eq!(
            mock.get_calls().await,
            vec!["apt depends --no-recommends --no-suggests -- curl"]
        );
    }

    /// Everything a manager prints that is *not* a dependency has to be thrown away, and until
    /// 2026-07-30 none of it was: this parser took the first word of every non-empty line.
    ///
    /// The first real `zypper` run in the project's history could not install a single package
    /// because of it. `zypper info --requires jq` opens with `Loading repository data...` and
    /// `Reading installed packages...`, and reports `Installed      : No` — so the dependency
    /// list came back as `Loading`, `Reading`, `No` and a dozen other words. The planner adds
    /// every dependency as an install node and then asks *it* for its dependencies, which
    /// returned the same three words, and the sweep died on:
    ///
    /// ```text
    /// Error: `requires` forms a cycle
    ///   zypper:No requires zypper:Loading
    ///   zypper:Loading requires zypper:Reading
    ///   zypper:Reading requires zypper:No
    /// ```
    ///
    /// **Backends that drive one manager take one lock.** `pacman` and `yay` in one config is
    /// an ordinary Arch machine, and both of them write `/var/lib/pacman/`. Keyed by their own
    /// names they were two locks over one database: a sync touching both ran them at the same
    /// time and let pacman's `db.lck` arbitrate, which it does by failing whichever lost.
    ///
    /// A backend that shares its manager with nobody keys on itself, and that is checked too —
    /// folding every backend onto one lock would serialise `npm` behind `apt` and throw away
    /// the parallelism that makes a mixed sync worth running.
    #[test]
    fn backends_that_share_a_package_manager_share_its_lock() {
        let key = |name: &str| {
            let vfs = Arc::new(DashMap::new());
            let mock = Arc::new(MockExecutor::new(vfs.clone()));
            let exec =
                CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
            apt_like_core_named(name, exec).lock_key().to_string()
        };
        for (backend, expected) in [
            ("pacman", "pacman"),
            ("yay", "pacman"),
            ("paru", "pacman"),
            ("apt", "apt"),
            ("apt-get", "apt"),
            ("dnf", "dnf"),
            ("yum", "dnf"),
            ("microdnf", "dnf"),
            ("zypper", "zypper"),
            // No shared manager, no shared lock.
            ("npm", "npm"),
            ("cargo", "cargo"),
            ("flatpak", "flatpak"),
        ] {
            assert_eq!(key(backend), expected, "{backend} locks the wrong manager");
        }
    }

    /// The fixture is that command's real output, captured from the openSUSE image.
    #[tokio::test]
    async fn preamble_and_metadata_are_not_dependencies() {
        let fixture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/zypper/info-requires.txt"
        ))
        .expect("the zypper fixture");

        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "zypper info --requires -- jq",
            Ok(DryRunOutput {
                stdout: fixture.into_bytes(),
                stderr: vec![],
            }
            .into()),
        );
        let exec = CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
        let mut core = apt_like_core_named("zypper", exec);
        core.config.depends = Some(DependsProbe {
            binary: None,
            args: vec!["info".into(), "--requires".into(), "{name}".into()],
            parse: Arc::new(parse_dependency_output),
        });

        let deps = core.get_dependencies("jq").await.unwrap();

        for junk in [
            "Loading",
            "Reading",
            "No",
            "Information",
            "openSUSE",
            "openSUSE-Tumbleweed-Oss",
            "jq-1.8.2-1.3.src",
            "[4]",
            "A",
            "JSON",
            "not",
        ] {
            assert!(
                !deps.iter().any(|d| d == junk),
                "`{junk}` is a word zypper printed, not a package it requires — got {deps:?}"
            );
        }
        assert_eq!(
            deps,
            vec![
                "libc.so.6(GLIBC_2.38)(64bit)".to_string(),
                "libonig.so.5()(64bit)".to_string(),
                "libjq.so.1()(64bit)".to_string(),
                "libjq1".to_string(),
            ],
            "only the four lines under `Requires` are requirements"
        );
    }

    fn queryable_with(
        manual: ManualListing,
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericQueryable {
        let mut core = apt_like_core(mock, vfs);
        core.config.list_binary = Some("dpkg-query".into());
        core.config.list_args = vec!["-W".into(), "-f=${Package} ${Version}\\n".into()];
        core.config.manual = manual;
        core.parser = Arc::new(crate::parsers::apt::AptParser);
        GenericQueryable {
            core: Arc::new(core),
        }
    }

    /// Q43: **a manager too old for the machine-readable listing must fall back, not vanish.**
    ///
    /// `--format json` needs dotnet SDK 10, `--json` a recent pixi, and Shall does not choose
    /// which is installed. An unsupported flag exits non-zero with a usage message, and every
    /// other reader here hands that back as an empty result — so asking for the better format
    /// without negotiating would report an empty machine to exactly the users on older
    /// tooling, which is `Q40` wearing a new hat.
    #[tokio::test]
    async fn a_manager_that_refuses_the_machine_format_is_read_from_its_text_listing() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        // The new flag is rejected, the way an older CLI rejects one it has never heard of.
        mock.set_response(
            "dpkg-query --format json",
            Ok(crate::core::executor::spoken_failure(
                2,
                "",
                "error: unexpected argument '--format' found",
            )),
        );
        // ...and the listing it does understand still answers.
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\n",
            Ok(DryRunOutput {
                stdout: b"jq 1.7.1
ripgrep 15.2.0
"
                .to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let mut core = apt_like_core(mock.clone(), vfs);
        core.config.list_binary = Some("dpkg-query".into());
        core.config.list_args = vec!["-W".into(), "-f=${Package} ${Version}\n".into()];
        core.config.machine_list = Some(MachineListing {
            binary: None,
            args: vec!["--format".into(), "json".into()],
            parse: std::sync::Arc::new(|_: &str| Ok(vec![Package::new("WRONG", "apt")])),
        });
        core.parser = Arc::new(crate::parsers::apt::AptParser);
        let q = GenericQueryable {
            core: Arc::new(core),
        };

        let names: Vec<String> = q
            .list_installed()
            .await
            .expect("a refused flag is not a failed listing")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(
            names,
            vec!["jq", "ripgrep"],
            "the text listing must answer when the machine format is refused"
        );
        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("--format json")),
            "the machine format was never asked for: {calls:?}"
        );
    }

    /// The other side: when the manager *does* support it, the text listing is never run.
    /// Asking both every time would double every listing on every current machine.
    #[tokio::test]
    async fn a_manager_that_answers_the_machine_format_is_not_asked_twice() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query --format json",
            Ok(DryRunOutput {
                stdout: b"[]".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let mut core = apt_like_core(mock.clone(), vfs);
        core.config.list_binary = Some("dpkg-query".into());
        core.config.list_args = vec!["-W".into()];
        core.config.machine_list = Some(MachineListing {
            binary: None,
            args: vec!["--format".into(), "json".into()],
            parse: std::sync::Arc::new(|_: &str| Ok(vec![Package::new("from-json", "apt")])),
        });
        core.parser = Arc::new(crate::parsers::apt::AptParser);
        let q = GenericQueryable {
            core: Arc::new(core),
        };

        let pkgs = q.list_installed().await.unwrap();
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "from-json");
        let calls = mock.get_calls().await;
        assert!(
            !calls.iter().any(|c| c == "dpkg-query -W"),
            "the text listing ran even though the machine format worked: {calls:?}"
        );
    }

    /// The link that turned an executor detail into a wrong answer about the machine.
    ///
    /// `run_output` handed back `Ok("")` for a lister that died without a word, the parser
    /// found no packages in the empty string, and `list_installed` reported `Ok(vec![])` — a
    /// manager with nothing installed. Nothing in the chain thought anything had failed.
    /// Measured on winget: 1 run in 16 under concurrent cold start, and `shall list --backend
    /// winget` printed nothing and exited 0 with 280 packages on the machine.
    #[tokio::test]
    async fn a_lister_that_died_silently_is_a_failure_not_an_empty_machine() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(crate::core::executor::silent_failure(1)),
        );
        let q = queryable_with(ManualListing::AllInstalled, mock.clone(), vfs);

        let err = q.list_installed().await.expect_err(
            "a lister that exited non-zero without a word must not report an empty machine",
        );
        assert!(
            err.to_string().contains("no output"),
            "the failure must say the lister produced nothing: {err}"
        );
    }

    /// The boundary the fix must not cross. A manager with genuinely nothing installed exits
    /// 0 and says nothing, and that empty listing is a real answer — turning *it* into an
    /// error would break every clean machine.
    #[tokio::test]
    async fn a_lister_that_succeeded_with_nothing_to_say_still_reports_an_empty_machine() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: vec![],
                stderr: vec![],
            }
            .into()),
        );
        let q = queryable_with(ManualListing::AllInstalled, mock.clone(), vfs);
        assert!(
            q.list_installed()
                .await
                .expect("exit 0 is an answer")
                .is_empty(),
            "an empty listing at exit 0 is a machine with nothing installed, not a failure"
        );
    }

    #[tokio::test]
    async fn apt_manual_list_asks_apt_mark_not_dpkg_query() {
        // The bug: apt had no manual command, so `list_manual` fell back to `dpkg-query
        // -W` — every installed package, dependencies included (579 vs 103 on the real
        // ubuntu image). It must ask `apt-mark showmanual` instead.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "apt-mark showmanual",
            Ok(DryRunOutput {
                stdout: b"apt\nbase-files\njq\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        // If it wrongly fell back, it would hit this instead — and adopt a dependency. That
        // sentence is the test's whole claim, and until this stub was registered as one that
        // must not be used, nothing checked it: the stub sat dead, and a product wired to the
        // wrong listing would have been caught only by the overlap between the two answers.
        mock.set_response_that_must_not_be_used(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\njq 1.7.1\nlibperl5.38t64 5.38.2\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let q = queryable_with(
            ManualListing::Command {
                binary: Some("apt-mark".into()),
                args: vec!["showmanual".into()],
                format: ManualFormat::BareNames,
            },
            mock.clone(),
            vfs,
        );

        let names: Vec<String> = q
            .list_manual()
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["apt", "base-files", "jq"]);
        assert!(
            !names.contains(&"libperl5.38t64".to_string()),
            "a pure dependency must never be reported as user-chosen"
        );
        assert!(q.tracks_manual());

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c == "apt-mark showmanual"),
            "{:?}",
            calls
        );
    }

    #[tokio::test]
    async fn unsupported_backend_reports_nothing_rather_than_everything() {
        // The safety backstop: a manager with dependencies and no way to name the user's
        // choices must return an empty list AND admit it via tracks_manual, so adoption
        // skips it. Returning list_installed here is a confident wrong answer.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"apt 2.7.14\nlibperl5.38t64 5.38.2\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );

        let q = queryable_with(ManualListing::Unsupported, mock, vfs);
        assert!(!q.tracks_manual());
        assert!(
            q.list_manual().await.unwrap().is_empty(),
            "adopting nothing is safe; adopting everything is catastrophic"
        );
        // list_installed still works — only the *intent* question is unanswerable.
        assert_eq!(q.list_installed().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn all_installed_backends_still_report_their_installed_set() {
        // winget/choco/mas install no dependencies, so everything listed was asked for.
        // The Unsupported default must not silently swallow these.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "dpkg-query -W -f=${Package} ${Version}\\n",
            Ok(DryRunOutput {
                stdout: b"jq 1.7.1\n".to_vec(),
                stderr: vec![],
            }
            .into()),
        );
        let q = queryable_with(ManualListing::AllInstalled, mock, vfs);
        assert!(q.tracks_manual());
        assert_eq!(q.list_manual().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn essential_query_is_absent_unless_a_backend_declares_it() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let q = queryable_with(ManualListing::AllInstalled, mock, vfs);
        // apt_like_core sets essential_args: None → no OS essential list, and no crash.
        assert!(q.essential().await.unwrap().is_empty());
    }

    #[test]
    fn version_pin_renders_native_syntax() {
        // inline forms (apt/pip/bun)
        assert_eq!(
            VersionPin::Inline("{name}={version}".into()).apply("curl", "7.81.0"),
            vec!["curl=7.81.0"]
        );
        assert_eq!(
            VersionPin::Inline("{name}=={version}".into()).apply("requests", "2.31.0"),
            vec!["requests==2.31.0"]
        );
        // flag forms (winget/choco/gem)
        assert_eq!(
            VersionPin::after(vec!["--version".into(), "{version}".into()])
                .apply("Git.Git", "2.54.0"),
            vec!["Git.Git", "--version", "2.54.0"]
        );
    }

    /// Whether the `--` terminator survives is read off the tokens, never off a label.
    ///
    /// The bug this replaces: `Flag`, `TrailingPositional` and `RequiredFlag` built identical
    /// argv and only their names decided the terminator, so `luarocks` and `mix` — both
    /// carrying a bare positional version — lost it on pinned installs while keeping it on
    /// unpinned ones. Same command, same tool, protection that came and went with whether
    /// someone had written a version on the line.
    #[test]
    fn an_option_after_the_name_is_recognised_by_its_dash_and_not_by_its_variant() {
        // Options: the terminator cannot precede these.
        assert!(VersionPin::after(vec!["-v".into(), "{version}".into()]).emits_trailing_option());
        assert!(
            VersionPin::after(vec!["--version".into(), "{version}".into()]).emits_trailing_option()
        );
        // Operands: it can, and does. luarocks, mix and pub all measured in the `tools` image
        // on 2026-08-04 — `luarocks install -- <rock> <version>` and
        // `dart pub global activate -- <pkg> <version>` produce output identical to the same
        // command without the terminator, and `luarocks install --` answers
        // `Error: missing argument 'rock'` with usage `<rock> [<version>]`.
        assert!(!VersionPin::after(vec!["{version}".into()]).emits_trailing_option());
        assert!(
            !VersionPin::after_required(vec!["{version}".into()], "latest").emits_trailing_option()
        );
        // A required version does not change the answer — only the token does.
        assert!(
            VersionPin::after_required(vec!["-v".into(), "{version}".into()], "latest")
                .emits_trailing_option()
        );
        // Neither of the other placements puts anything after the name.
        assert!(!VersionPin::Inline("{name}@{version}".into()).emits_trailing_option());
        assert!(
            !VersionPin::Before(vec!["--version".into(), "{version}".into()])
                .emits_trailing_option()
        );
    }

    #[tokio::test]
    async fn list_orphans_reports_unsupported_without_a_dry_run() {
        // A generic backend with no `orphan_dry_run` cannot say what its orphan verb would
        // delete, so it reports Unsupported and never removes blind.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(apt_like_core(mock, vfs)); // apt_like_core sets orphan_dry_run: None
        let up = GenericUpgradable { core };
        match up.list_orphans().await {
            Err(crate::core::Error::Unsupported(name)) => assert_eq!(name, "apt"),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    /// A `helm`-shaped core: installs from an option, lists and removes by name.
    fn source_option_core(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericBackendCore {
        let mut core = apt_like_core(mock, vfs);
        core.name = "helm".into();
        core.config.name = "helm".into();
        core.config.install_source_option = Some("url".into());
        core.config.install_args = vec!["plugin".into(), "install".into()];
        core.config.remove_args = vec!["plugin".into(), "uninstall".into()];
        core.config.needs_root = false;
        core
    }

    fn spec_with(name: &str, opts: &[(&str, &str)]) -> PackageSpec {
        PackageSpec {
            name: name.into(),
            backend: "helm".into(),
            options: opts
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn install_from_a_source_option_sends_the_source_and_removes_by_name() {
        // U39. The whole bug in one test: what goes out at install is the URL, what goes out
        // at remove is the name, and they come from the same one-line declaration.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };

        let url = "https://github.com/databus23/helm-diff";
        inst.install(&[spec_with("diff", &[("url", url)])], false)
            .await
            .unwrap();
        inst.remove(
            &["diff".to_string()],
            false,
            crate::app::sync::guard::Reaped::for_reason(
                crate::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .unwrap();

        let calls = mock.get_calls().await;
        assert!(
            calls
                .iter()
                .any(|c| c.contains("plugin install") && c.contains(url)),
            "install must send the url: {:?}",
            calls
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("plugin install") && c.contains(" diff")),
            "install must not send the name: {:?}",
            calls
        );
        assert!(
            calls
                .iter()
                .any(|c| c.contains("plugin uninstall") && c.contains("diff")),
            "remove must send the name: {:?}",
            calls
        );
    }

    /// A pip core, for the `@system` opt-in (`Q49`).
    fn pip_core(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericBackendCore {
        let exec = CommandExecutor::with_layer(true, false, mock, vfs, Arc::new(DashMap::new()));
        let mut core = apt_like_core_named("pip", exec);
        core.config.install_args = vec!["install".into()];
        core
    }

    fn pip_spec(name: &str, system: bool) -> PackageSpec {
        PackageSpec {
            name: name.into(),
            backend: "pip".into(),
            options: if system {
                [("system".to_string(), "true".to_string())]
                    .into_iter()
                    .collect()
            } else {
                Default::default()
            },
            ..Default::default()
        }
    }

    /// **`@system` reaches pip as `--break-system-packages`, and only for the line that said
    /// it** (`Q49`, owner ruling 2026-08-10).
    ///
    /// The batch split is the half that matters. Without it, one line's permission to write
    /// into an OS-owned Python would be handed to every other package in the same wave — which
    /// is the exact failure `@unverified` is partitioned to avoid, with a worse blast radius:
    /// packages nobody said that about, installed into the system interpreter.
    #[tokio::test]
    async fn system_reaches_pip_as_its_own_flag_and_is_never_shared() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let inst = GenericInstallable {
            core: Arc::new(pip_core(mock.clone(), vfs)),
        };

        inst.install(&[pip_spec("black", true), pip_spec("httpie", false)], false)
            .await
            .unwrap();

        let calls = mock.get_calls().await;
        let opted: Vec<&String> = calls.iter().filter(|c| c.contains("black")).collect();
        let plain: Vec<&String> = calls.iter().filter(|c| c.contains("httpie")).collect();
        assert!(
            opted.iter().all(|c| c.contains("--break-system-packages")),
            "the line that opted in must carry the flag: {calls:?}"
        );
        assert!(
            plain.iter().all(|c| !c.contains("--break-system-packages")),
            "the line that did NOT opt in must not inherit it: {calls:?}"
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("black") && c.contains("httpie")),
            "the two must not share a command at all: {calls:?}"
        );
    }

    /// And with nobody opting in, the flag is simply absent — the default is the refusal.
    #[tokio::test]
    async fn pip_without_the_opt_in_sends_no_flag() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let inst = GenericInstallable {
            core: Arc::new(pip_core(mock.clone(), vfs)),
        };
        inst.install(&[pip_spec("black", false)], false)
            .await
            .unwrap();
        assert!(
            mock.get_calls()
                .await
                .iter()
                .all(|c| !c.contains("--break-system-packages")),
            "nothing asked for it"
        );
    }

    /// Does the helm on THIS machine, if any, accept the opt-out flag?
    ///
    /// The two tests below assert that `@unverified` reaches the command line as helm's own
    /// flag. Since G-8 that is conditional on what the installed helm documents, so on a
    /// helm 3 host the premise is false and the right outcome is to say so rather than to
    /// fail. `None` (no helm here, as on every CI runner) leaves the capability table in
    /// charge, so the flag is emitted and the assertion holds.
    fn this_hosts_helm_takes_the_opt_out() -> bool {
        crate::core::tool_help::accepts_flag(
            "helm",
            &["plugin".to_string(), "install".to_string()],
            "--verify=false",
        ) != Some(false)
    }

    /// Q5. `@unverified` is what turns off a verification the *manager* does, and it reaches
    /// the command line as that manager's own flag.
    #[tokio::test]
    async fn unverified_becomes_the_managers_own_opt_out_flag() {
        if !this_hosts_helm_takes_the_opt_out() {
            eprintln!("this host's helm rejects --verify; the premise of this test is false here");
            return;
        }
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let inst = GenericInstallable {
            core: Arc::new(source_option_core(mock.clone(), vfs)),
        };

        let url = "https://github.com/databus23/helm-diff";
        inst.install(
            &[spec_with("diff", &[("url", url), ("unverified", "true")])],
            false,
        )
        .await
        .unwrap();

        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("--verify=false")),
            "the opt-out must reach the command: {:?}",
            calls
        );
    }

    /// A manager that refuses to install without a version gets one, and the right one.
    ///
    /// Measured in the `tools` image on 2026-07-29, after the `--` fix stopped masking it:
    ///
    /// ```text
    /// $ asdf install nodejs           -> No versions specified for nodejs in config files or environment
    /// $ asdf install nodejs latest    -> Installed node-v26.5.0-linux-x64
    /// ```
    ///
    /// The pinned case must keep working too, or this trades one broken argv for another —
    /// which is exactly what E11's fix did, and why it came back as G-8.
    #[tokio::test]
    async fn a_manager_that_demands_a_version_is_given_one() {
        let unpinned = VersionPin::after_required(vec!["{version}".into()], "latest");
        assert_eq!(
            unpinned.apply("nodejs", "latest"),
            vec!["nodejs".to_string(), "latest".to_string()],
            "an unpinned line must still name a version"
        );
        assert_eq!(
            unpinned.apply("nodejs", "20.1.0"),
            vec!["nodejs".to_string(), "20.1.0".to_string()],
            "and a pinned one must reach the tool unchanged"
        );
        assert_eq!(unpinned.unpinned(), Some("latest"));

        // The control: the ordinary positional pin is unchanged and asks for nothing when the
        // line pins nothing. luarocks shares asdf's shape and resolves the newest itself, so
        // giving it a fallback would put a token on its command line that nobody asked for.
        let ordinary = VersionPin::after(vec!["{version}".into()]);
        assert_eq!(ordinary.unpinned(), None);
        assert_eq!(
            ordinary.apply("luafilesystem", "1.8.0"),
            vec!["luafilesystem".to_string(), "1.8.0".to_string()]
        );
    }

    /// The default is untouched: a line that did not ask keeps the manager's verification on.
    #[tokio::test]
    async fn a_line_that_did_not_ask_keeps_verification_on() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let inst = GenericInstallable {
            core: Arc::new(source_option_core(mock.clone(), vfs)),
        };

        inst.install(
            &[spec_with(
                "diff",
                &[("url", "https://github.com/databus23/helm-diff")],
            )],
            false,
        )
        .await
        .unwrap();

        let calls = mock.get_calls().await;
        assert!(
            !calls.iter().any(|c| c.contains("--verify")),
            "verification must stay on unless the line said otherwise: {:?}",
            calls
        );
    }

    /// One package's opt-out is not another's. A batch where the specs disagree has to become
    /// two commands, or the flag silently covers a line that never asked for it — which is the
    /// global-switch failure `@unverified` is per-line to avoid.
    #[tokio::test]
    async fn a_mixed_batch_does_not_hand_one_lines_opt_out_to_another() {
        if !this_hosts_helm_takes_the_opt_out() {
            eprintln!("this host's helm rejects --verify; the premise of this test is false here");
            return;
        }
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let inst = GenericInstallable {
            core: Arc::new(source_option_core(mock.clone(), vfs)),
        };

        inst.install(
            &[
                spec_with("diff", &[("url", "https://example.com/helm-diff")]),
                spec_with(
                    "secrets",
                    &[
                        ("url", "https://example.com/helm-secrets"),
                        ("unverified", "true"),
                    ],
                ),
            ],
            false,
        )
        .await
        .unwrap();

        let calls = mock.get_calls().await;
        let with_flag: Vec<&String> = calls
            .iter()
            .filter(|c| c.contains("--verify=false"))
            .collect();
        assert_eq!(
            with_flag.len(),
            1,
            "exactly one of the two asked to skip verification: {:?}",
            calls
        );
        assert!(
            with_flag[0].contains("helm-secrets") && !with_flag[0].contains("helm-diff"),
            "the flag went out with the wrong package: {:?}",
            calls
        );
    }

    /// helm's own failure names `--verify=false`, an argv no declaration can write. The advice
    /// a user can act on is the flag on the line.
    #[tokio::test]
    async fn a_verification_failure_names_the_flag_a_declaration_can_write() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        mock.set_response(
            "helm plugin install -- https://github.com/databus23/helm-diff",
            Err(crate::core::Error::CommandFailed {
                message: "Error: plugin source does not support verification. Use --verify=false \
                          to skip verification"
                    .into(),
                retry: crate::core::Retryability::Permanent,
                // Permanent and the plugin's name is fine — the fixture for the distinction
                // `absent_name` exists to draw.
                absent_name: false,
            }),
        );
        let inst = GenericInstallable {
            core: Arc::new(source_option_core(mock.clone(), vfs)),
        };

        let msg = inst
            .install(
                &[spec_with(
                    "diff",
                    &[("url", "https://github.com/databus23/helm-diff")],
                )],
                false,
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("@unverified"),
            "the failure must name the flag the line can carry: {}",
            msg
        );
    }

    #[tokio::test]
    async fn install_without_the_source_option_refuses_and_names_the_fix() {
        // Refusing beats guessing a URL→name mapping: the old behaviour installed happily and
        // then failed every later sync, because nothing could remove what it had installed.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };

        let err = inst
            .install(&[spec_with("diff", &[])], false)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("helm:diff@url="), "{}", msg);
        assert!(
            mock.get_calls().await.is_empty(),
            "nothing may reach the machine when the declaration is incomplete"
        );
    }

    #[tokio::test]
    async fn an_empty_source_option_is_as_missing_as_no_option() {
        // `@url=` with nothing after it would otherwise run `helm plugin install ''`.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(source_option_core(mock.clone(), vfs));
        let inst = GenericInstallable { core };
        assert!(inst
            .install(&[spec_with("diff", &[("url", "  ")])], false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn remove_reports_unsupported_with_empty_remove_args() {
        // A manager with no uninstall verb encodes it as empty remove_args → Unsupported,
        // rather than running the bare binary against the package names.
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let core = Arc::new(apt_like_core(mock, vfs)); // apt_like_core sets remove_args: vec![]
        let inst = GenericInstallable { core };
        match inst
            .remove(
                &["ghc".to_string()],
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
        {
            Err(crate::core::Error::Unsupported(name)) => assert_eq!(name, "apt"),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    fn repo_mgr(
        name: &str,
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
        edit: impl FnOnce(&mut ManagerConfig),
    ) -> GenericRepoManager {
        let mut core = apt_like_core(mock, vfs);
        core.name = name.to_string();
        core.config.name = name.to_string();
        edit(&mut core.config);
        GenericRepoManager {
            core: Arc::new(core),
        }
    }

    fn apk_repo(
        mock: Arc<MockExecutor>,
        vfs: Arc<DashMap<std::path::PathBuf, String>>,
    ) -> GenericRepoManager {
        repo_mgr("apk", mock, vfs, |c| {
            c.repo_add_args = Some(vec![
                "-c".into(),
                "echo '{url}' >> /etc/apk/repositories".into(),
            ]);
            c.repo_remove_args = Some(vec![
                "-c".into(),
                "sed -i '\\|{url}|d' /etc/apk/repositories".into(),
            ]);
            c.repo_list_args = Some(vec!["/etc/apk/repositories".into()]);
            c.repo_binary = Some("sh".into());
            c.repo_list_binary = Some("cat".into());
        })
    }

    /// The finding: `{url}` was never substituted on the removal path, so `sed` searched for
    /// the literal text `{url}`, matched nothing, and **exited 0** — Shall reported a
    /// repository removed that was still in the file.
    #[tokio::test]
    async fn apk_repo_removal_carries_the_real_url_and_no_placeholder() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = apk_repo(mock.clone(), vfs);
        mgr.remove_repo(
            "https://dl-cdn.alpinelinux.org/alpine/edge/testing",
            false,
            crate::app::sync::guard::Reaped::for_reason(
                crate::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .expect("a URL is a repository apk can be told to forget");
        let calls = mock.get_calls().await;
        assert!(
            calls.iter().any(|c| c.contains("dl-cdn.alpinelinux.org")),
            "{:?}",
            calls
        );
        assert!(
            !calls.iter().any(|c| c.contains("{url}")),
            "the placeholder reached the machine: {:?}",
            calls
        );
    }

    /// A removal Shall cannot address must refuse, not run a command that matches nothing.
    #[tokio::test]
    async fn a_repo_named_by_something_that_is_not_a_url_is_refused() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = apk_repo(mock.clone(), vfs);
        let err = mgr
            .remove_repo(
                "testing",
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .expect_err("apk knows no repository called `testing`")
            .to_string();
        assert!(err.contains("testing"), "{}", err);
        assert!(err.contains("repo list"), "{}", err);
        assert!(
            !mock
                .get_calls()
                .await
                .iter()
                .any(|c| c.contains("sed") || c.contains("echo")),
            "nothing may run when the repository cannot be identified"
        );
    }

    /// The other `{url}` template: gem removes by source URL, and a name the listing knows
    /// resolves to one.
    #[tokio::test]
    async fn a_listed_name_resolves_to_its_url() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mut listed: std::process::Output = DryRunOutput::new().into();
        listed.stdout = b"internal https://gems.example.invalid/\n".to_vec();
        mock.set_response("gem sources", Ok(listed));
        let mgr = repo_mgr("gem", mock.clone(), vfs, |c| {
            c.repo_remove_args = Some(vec!["sources".into(), "-r".into(), "{url}".into()]);
            c.repo_list_args = Some(vec!["sources".into()]);
        });
        mgr.remove_repo(
            "internal",
            false,
            crate::app::sync::guard::Reaped::for_reason(
                crate::app::sync::guard::GuardScope::Remove,
                "a unit test of the effector itself",
            ),
        )
        .await
        .unwrap();
        let calls = mock.get_calls().await;
        assert!(
            calls
                .iter()
                .any(|c| c == "gem sources -r https://gems.example.invalid/"),
            "{:?}",
            calls
        );
    }

    /// The part that makes this a fixed *class* rather than a fixed instance: a template
    /// carrying a placeholder nothing fills is refused before it runs, whatever the
    /// placeholder is.
    #[tokio::test]
    async fn a_template_with_an_unfillable_placeholder_is_refused() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = repo_mgr("weird", mock.clone(), vfs, |c| {
            c.repo_remove_args = Some(vec!["drop".into(), "{name}".into(), "{channel}".into()]);
        });
        let err = mgr
            .remove_repo(
                "internal",
                false,
                crate::app::sync::guard::Reaped::for_reason(
                    crate::app::sync::guard::GuardScope::Remove,
                    "a unit test of the effector itself",
                ),
            )
            .await
            .expect_err("an unfilled placeholder is not a repository name")
            .to_string();
        assert!(err.contains("{channel}"), "{}", err);
        assert!(
            mock.get_calls().await.is_empty(),
            "the template ran with a placeholder in it"
        );
    }

    /// `add_repo` substitutes both keys and always did — asserted so the guard cannot break
    /// the path that was working.
    #[tokio::test]
    async fn add_repo_still_substitutes_name_and_url() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mgr = repo_mgr("winget", mock.clone(), vfs, |c| {
            c.repo_add_args = Some(vec![
                "source".into(),
                "add".into(),
                "--name".into(),
                "{name}".into(),
                "--arg".into(),
                "{url}".into(),
            ]);
        });
        mgr.add_repo("internal", "https://feed.example.invalid/", false)
            .await
            .unwrap();
        assert_eq!(
            mock.get_calls().await,
            vec!["winget source add --name internal --arg https://feed.example.invalid/"]
        );
    }

    #[test]
    fn a_placeholder_is_recognised_wherever_it_sits_in_the_argument() {
        assert_eq!(find_placeholder("{url}").as_deref(), Some("{url}"));
        assert_eq!(
            find_placeholder("sed -i '\\|{url}|d' /etc/apk/repositories").as_deref(),
            Some("{url}")
        );
        assert_eq!(find_placeholder("--name").as_deref(), None);
        // A brace that is not a placeholder must not become a refusal: shell brace expansion
        // and printf formats both use them.
        assert_eq!(find_placeholder("printf '%s\\n'").as_deref(), None);
        assert_eq!(find_placeholder("{NAME}").as_deref(), None);
        assert_eq!(find_placeholder("{}").as_deref(), None);
        assert_eq!(find_placeholder("a{b").as_deref(), None);
    }

    #[test]
    fn concrete_version_rejects_floating() {
        assert!(is_concrete_version("1.2.3"));
        assert!(!is_concrete_version("latest"));
        assert!(!is_concrete_version("*"));
        assert!(!is_concrete_version(""));
    }

    /// `Q46`: **upgrading by reinstalling is one command, and still isolates a bad package.**
    ///
    /// A manager with no upgrade-all verb upgrades by re-installing what it has. That ran one
    /// `npm install -g <name>` per package — forty global packages, forty resolutions, forty
    /// registry conversations. Batching alone would have thrown away the reason the loop
    /// existed: one package that will not reinstall must not stop the other thirty-nine. So the
    /// batch is the fast path and the loop is the recovery, which costs nothing when everything
    /// works and loses nothing when it does not.
    #[tokio::test]
    async fn upgrading_by_reinstall_is_one_command_when_it_works() {
        let vfs = Arc::new(DashMap::new());
        let mock = Arc::new(MockExecutor::new(vfs.clone()));
        let mut core = apt_like_core(mock.clone(), vfs);
        core.name = "npm".into();
        core.config.name = "npm".into();
        core.config.list_binary = None;
        core.config.list_args = vec!["ls".into()];
        core.config.install_args = vec!["install".into(), "-g".into()];
        core.config.upgrade_reinstall_args = Some(vec!["install".into(), "-g".into()]);
        core.parser = Arc::new(crate::parsers::LambdaParser {
            installed_fn: |_| {
                Ok(vec![
                    Package::new("a", "npm"),
                    Package::new("b", "npm"),
                    Package::new("c", "npm"),
                ])
            },
            search_fn: |_| Vec::new(),
        });
        let u = GenericUpgradable {
            core: Arc::new(core),
        };
        u.upgrade(false).await.unwrap();

        let installs: Vec<String> = mock
            .get_calls()
            .await
            .into_iter()
            .filter(|c| c.contains("install"))
            .collect();
        assert_eq!(
            installs.len(),
            1,
            "three packages must be one command, got {:?}",
            installs
        );
        for name in ["a", "b", "c"] {
            assert!(installs[0].contains(name), "{:?}", installs);
        }
    }
}

#[cfg(test)]
mod settings_interpolation_tests {
    use super::*;
    use std::collections::HashMap;

    fn settings(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn row(name: &str) -> ManagerConfig {
        ManagerConfig {
            name: name.into(),
            binary: None,
            remove_binary: None,
            install_args: vec![],
            remove_args: vec![],
            purge_args: None,
            list_args: vec![],
            manual: ManualListing::Unsupported,
            essential_args: None,
            search_args: vec![],
            search_binary: None,
            enumerate_args: None,
            enumerate_binary: None,
            list_binary: None,
            upgrade_args: vec![],
            update_args: None,
            orphan_dry_run: None,
            repo_add_args: None,
            repo_remove_args: None,
            repo_list_args: None,
            repo_binary: None,
            repo_list_binary: None,
            repo_remove_binary: None,
            repo_list_shape: RepoListing::Columns,
            depends: None,
            clean_cache: None,
            version_pin: None,
            install_source_option: None,
            needs_root: false,
            is_exclusive: false,
            property_probes: Vec::new(),
            machine_list: None,
            outdated: None,
            search_source: SearchSource::Command,
            upgrade_reinstall_args: None,
            extra_probes: None,
        }
    }

    #[test]
    fn a_setting_reaches_the_argv_it_is_named_in() {
        let mut cfg = row("conda");
        cfg.install_args = vec!["install".into(), "-n".into(), "{setting.env|base}".into()];
        cfg.resolve_settings(Some(&settings(&[("env", "ml")])))
            .expect("resolves");
        assert_eq!(cfg.install_args, vec!["install", "-n", "ml"]);
    }

    #[test]
    fn the_default_is_used_when_the_key_is_absent_or_blank() {
        for value in [None, Some("   ")] {
            let mut cfg = row("conda");
            cfg.list_args = vec!["-n".into(), "{setting.env|base}".into()];
            let s = value.map(|v| settings(&[("env", v)]));
            cfg.resolve_settings(s.as_ref()).expect("resolves");
            assert_eq!(cfg.list_args, vec!["-n", "base"], "for {value:?}");
        }
    }

    /// The value substitutes *inside* the token, so one argument comes out — not two, and not a
    /// token with a space in it that the shell would never see because there is no shell.
    #[test]
    fn substitution_happens_inside_a_token() {
        let mut cfg = row("flatpak");
        cfg.install_args = vec!["install".into(), "--{setting.scope|system}".into()];
        cfg.resolve_settings(Some(&settings(&[("scope", "user")])))
            .expect("resolves");
        assert_eq!(cfg.install_args, vec!["install", "--user"]);
    }

    /// **A key with no value and no default is a refusal.** `conda list -n --json` would hand
    /// conda a flag where an environment belongs, and conda answers *something* — which is the
    /// silent-wrong-answer shape, not a crash.
    #[test]
    fn a_placeholder_with_nothing_behind_it_refuses_rather_than_emptying() {
        let mut cfg = row("conda");
        cfg.list_args = vec!["list".into(), "-n".into(), "{setting.env}".into()];
        let err = cfg
            .resolve_settings(None)
            .expect_err("an unresolvable placeholder must not ship");
        let msg = err.to_string();
        assert!(msg.contains("backend_settings.conda"), "{msg}");
        assert!(msg.contains("env"), "{msg}");
    }

    /// Every argv-bearing field, not the four somebody remembered. A placeholder that reaches a
    /// manager literally is the failure this whole mechanism would otherwise introduce.
    #[test]
    fn every_argv_bearing_field_is_visited() {
        let mut cfg = row("m");
        let p = || "{setting.k}".to_string();
        cfg.install_args = vec![p()];
        cfg.remove_args = vec![p()];
        cfg.list_args = vec![p()];
        cfg.search_args = vec![p()];
        cfg.upgrade_args = vec![p()];
        cfg.purge_args = Some(vec![p()]);
        cfg.essential_args = Some(vec![p()]);
        cfg.enumerate_args = Some(vec![p()]);
        cfg.update_args = Some(vec![p()]);
        cfg.repo_add_args = Some(vec![p()]);
        cfg.repo_remove_args = Some(vec![p()]);
        cfg.repo_list_args = Some(vec![p()]);
        cfg.upgrade_reinstall_args = Some(vec![p()]);
        cfg.extra_probes = Some(vec![p()]);
        cfg.version_pin = Some(VersionPin::Inline(p()));
        cfg.property_probes = vec![PropertyProbe {
            property: "x".into(),
            args: vec![p()],
            template: p(),
        }];
        cfg.manual = ManualListing::Command {
            binary: None,
            args: vec![p()],
            format: ManualFormat::BareNames,
        };
        cfg.clean_cache = Some(CacheClean {
            binary: None,
            args: vec![p()],
        });
        cfg.orphan_dry_run = Some(OrphanDryRun {
            binary: None,
            args: vec![p()],
            removes_line_prefix: String::new(),
        });

        cfg.resolve_settings(Some(&settings(&[("k", "V")])))
            .expect("every field resolves");
        let rendered = format!("{cfg:?}");
        assert!(
            !rendered.contains("{setting."),
            "a placeholder survived: {rendered}"
        );
    }

    /// **The guard's own self-test.** The `Debug` scan exists because `walk_args` is a
    /// hand-written list, and a check that cannot fail is worse than no check — so this plants
    /// exactly what `walk_args` does not visit (a *binary* name, which is not argv and is
    /// deliberately outside the walk) and requires the scan to catch it.
    #[test]
    fn the_leftover_scan_can_actually_fail() {
        let mut cfg = row("m");
        cfg.list_binary = Some("{setting.tool}".into());
        let err = cfg
            .resolve_settings(Some(&settings(&[("tool", "dpkg-query")])))
            .expect_err("the scan must catch a placeholder walk_args does not reach");
        assert!(
            err.to_string().contains("survived resolution"),
            "wrong failure: {err}"
        );
    }

    /// A row with no placeholders is untouched, which is every other backend in the tree.
    #[test]
    fn a_row_that_names_no_setting_is_unchanged() {
        let mut cfg = row("apt");
        cfg.install_args = vec!["install".into(), "-y".into()];
        cfg.resolve_settings(None).expect("resolves");
        assert_eq!(cfg.install_args, vec!["install", "-y"]);
    }
}
