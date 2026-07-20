# LiNix

A declarative package manager: you list the packages you want in a file, and `sync` makes the
machine match the list — across every package manager on the box.

LiNix does not replace apt, pacman, brew, cargo or npm. It drives them. One file says what
should be installed; `linix sync` installs what is missing, removes what is no longer listed,
and leaves everything else alone.

```
$ cat ~/.config/linix/modules/tools.txt
apt:ripgrep
cargo:bat
npm:typescript@version=>=5.0.0

$ cat ~/.config/linix/profiles/Main
use tools

$ linix sync
Planned changes:
  install 3   remove 0   (total 3 change(s))
```

Delete the `cargo:bat` line and sync again, and `bat` is uninstalled. That is the whole idea:
**the file is the truth, and every command is a shortcut for editing it and syncing.**

Note the second file. A module is a *list*; it does nothing until an active profile `use`s it.
That indirection is what lets one repo describe several machines — see [Profiles](#profiles).

---

## Install

LiNix is written in Rust.

```bash
cargo build --release
cp target/release/linix ~/.local/bin/
```

## Start

```bash
linix init          # scaffold ~/.config/linix, with one profile (Main) already active
linix install jq    # writes a line you own, then syncs
linix status        # what sync would change, read-only
linix sync          # make the machine match the files
```

`linix install` is not a separate mechanism — it writes `jq` into a module that the active
profile already reaches, then syncs. Anything it can do, editing the file does too.

Writing a module by hand takes one extra step, because a module is inert until something uses
it:

```bash
echo 'cargo:ripgrep' > ~/.config/linix/modules/tools.txt
echo 'use tools'    >> ~/.config/linix/profiles/Main
linix check                # parses everything the active profiles reach
linix --dry-run sync       # preview
linix sync
```

`linix check` is the fastest way to confirm a file is actually being read: it reports how many
lines resolved. If you edited a module and `check` still says `0 present`, no active profile
is using it.

## The files

`linix init` creates them under `$LINIX_CONFIG_DIR` (default `~/.config/linix`). **This
directory is meant to be a git repo** — `linix git init` turns on version control, after which
every sync commits, and `linix rollback <commit>` puts the machine back.

```
modules/       your lists of packages       lowercase names, *.txt
profiles/      named sets you turn on and off       Capitalized names
active         which profiles are on right now
priority       which package managers this machine uses, in order
schedules      when LiNix runs itself
locks/         what everything resolved to, one file per backend
config.toml    settings (written by `linix config init`)
```

LiNix's own bookkeeping — what it currently owns, snapshot metadata — lives in
`$LINIX_DATA_DIR`, never in the config repo and never in git.

Facts about the machine are **detected, not configured**: core count, whether btrfs/ZFS/
Timeshift exists, which managers are installed. The one exception is `max_parallel`, which you
may set by hand to cap concurrency below the core count.

## The grammar

A file is lines. A line is blank, a comment, a statement, or a block. **An unrecognised line is
an error** naming the file, the line, and what was expected — never a silently ignored typo.

```
# a comment
apt:curl                  # explicit backend
ripgrep                   # bare name: resolved via `priority`, then locked
apt:re:^python3-.*        # regex against that backend's names
absent:snap:firefox       # must NOT be installed
repo:apt:ppa:foo/bar      # a repository
shim:node                 # a PATH stand-in
service:nginx             # a service
link:./dotfiles/vimrc     # a managed file
use editors               # pull in another module
```

`use` takes **a name, never a path or a URL.**

### Options

Short form for simple values, block form for anything with a comma:

```
apt:jq@version=1.6
npm:typescript@version=>=5.0.0

apt:nginx {
  after_install = ./setup.sh --flag=a,b
  requires      = apt:libfoo
  requires      = apt:libbar        # a key given twice makes a list
}
```

A comma inside a short-form value is an error that tells you to use the block form, rather than
a guess about where the value ended. In a block, everything after the first `=` is the value,
verbatim and trimmed — no escaping exists because none is needed.

Common keys: `version`, `hold` (never upgrade), `expires` / `until` (absolute datetimes),
`requires`, the `*_install` hooks, and per-directive keys like `cron`/`run` on `schedule:` or
`target`/`content` on `link:`.

### Which file gets installed

`github:sharkdp/fd` names a repo, not a file. One release ships a `.deb`, a `.tar.gz`, an
`.AppImage` and a bare binary, so LiNix has to choose — and a declaration that resolves to a
different file on two machines is not declarative.

`formats` is an ordered preference. First match wins; a later entry is a fallback:

```
github:BurntSushi/ripgrep {
  formats = appimage
  formats = tarball
  formats = binary
}
```

The vocabulary is closed — `deb rpm appimage tarball zip exe msi pkg dmg binary` — and an
unrecognised name is an error listing the legal set. **You do not need to write any of this**:
the default order comes from your OS and distribution, so a fresh repo installs the right thing
with no `formats` line anywhere.

Your architecture is not a preference. Assets that cannot run on this machine are filtered out
before `formats` is consulted, so there is no `@arch=` to get wrong.

When a release ships two files that both fit — `fd_10.2.0_amd64.deb` and
`fd-musl_10.2.0_amd64.deb` — LiNix picks the more specific one, then the shorter name, and
**tells you what it chose and what it skipped**. To decide yourself, `@asset=` takes a filename
or a glob that survives version bumps:

```
github:sharkdp/fd@asset=*musl*
```

For an archive, LiNix extracts it, finds the executable and shims it onto your `PATH`. When the
archive holds several and the guess would be wrong, `@bin=` names it:

```
github:foo/bar@bin=build/bar
```

Finding no executable, or several, is an error listing what the archive held — never a silent
pick.

`channel` is the other half, for backends that ship one artifact in several version streams:

```
snap:code@channel=stable
```

Both keys are errors on a backend they do not apply to, rather than being quietly ignored.

### Host conditions

`when` gates the lines inside it, and it works the same way in every file — packages in a
module, imports in a profile, backends in `priority`, profile names in `active`:

```
when os == linux {
  apt:htop
}

when host in [laptop, tablet] {
  apt:tlp
}
```

Keys: `os`, `arch`, `host`, `hostname`, `family`. Operators: `==`, `!=`, `in [a, b]`.

## Profiles

A profile is a named set of modules you can turn on and off live, with no reboot. Several can
be active at once — their package sets are unioned.

```bash
linix activate work           # `active` becomes exactly: work
linix activate -a gaming      # add gaming to what is already on
linix deactivate gaming       # drop it, removing only what nothing else needs
linix profile list
```

`activate` sets, `activate -a` adds, `deactivate` removes. `activate` overwrites the whole
`active` file including any `when` blocks in it, and says which ones it removed; `activate -a`
and `deactivate` never rewrite a block that does not apply to this host, because `active` is a
shared file and another host's block is another machine's business.

## The removal guard

Drift is derived from managed state, and managed state can be wrong — a mis-scoped manifest, a
state file from another machine. So **every path that removes anything** goes through one guard,
which refuses when a removal:

- exceeds `max_removals` (default 20),
- touches a protected package — a built-in list, anything you add, **and** the OS's own
  essential flags where it has them (`dpkg`'s `Essential` / `Priority: required`),
- or trips one of the `[guard]` policy rules.

`linix protected` prints the effective rules. The only override for the count is
`--allow-mass-removal`. **`--yes` is deliberately not an override**, because every script and CI
job passes `-y`, and an unattended run is exactly the one that cannot notice a system being
taken apart. Protection is a refusal, not a confirmation: nothing overrides it.

## History and rollback

With `linix git init`, your config directory is version-controlled and every sync commits.
History is git — there is no second generation store.

```bash
linix history            # browse commits, see what each changed, roll back from inside
linix diff HEAD~3        # what changed, in packages rather than text
linix rollback HEAD~3    # restore those manifests, then converge the machine to match
linix undo               # interactive snapshot gallery (btrfs / ZFS / Timeshift / Windows)
```

`rollback` refuses to apply unconfirmed in a non-interactive shell; pass `--yes` for CI.

## Commands

Run `linix --help` for the full list with current wording, and `linix doctor` for what this
machine actually supports — that is generated from the registry, so it cannot go stale the way
a number typed into a README does.

**Everyday**

| | |
|---|---|
| `sync` | Install, remove and update until the machine matches your files |
| `status` | Read-only: what `sync` would change |
| `install` / `uninstall` | Edit the file and sync |
| `list` / `search` / `info` | What is installed, what exists, what a package is |
| `update` / `upgrade` | Refresh metadata; upgrade managed packages |
| `hold` / `unhold` | Stop a package from being upgraded |

**Understanding the machine**

| | |
|---|---|
| `check` | Parse everything the active profiles reach; report errors, change nothing |
| `why` | Why a package is installed: where it is declared and what depends on it |
| `unmanaged` | Installed on the OS but not managed by LiNix |
| `absent` | Every `absent:` rule in force, and which module it comes from |
| `conflicts` | The same tool pinned to different versions by different backends |
| `doctor` | Per-backend readiness, config and layout integrity; `--fix` repairs what is safe |

**Cleaning up**

| | |
|---|---|
| `adopt` | Write the packages you installed by hand into a module |
| `unmanage` | Stop managing a package **without** uninstalling it |
| `remove-orphans` | Remove what each manager considers orphaned — shows the list and asks first |
| `clean-cache` | Delete downloaded archives and caches; removes no installed package |
| `purge-unmanaged` | Delete everything LiNix does not manage. Shows the whole list first |

**Plan, lock, reproduce**

| | |
|---|---|
| `plan` / `apply` | Freeze what `sync` would do to a file, review it, then apply exactly that |
| `lock` | Record every managed package's version so `sync --locked` reproduces it elsewhere |
| `export` | Emit native manifests (Brewfile, requirements.txt, package.json, Aptfile) |
| `bundle` | An offline/air-gapped bundle of config, lockfile and resolved package list |
| `sbom` / `audit` | CycloneDX bill of materials; scan managed packages against OSV.dev |

**Running things**

| | |
|---|---|
| `shell` | An ephemeral shell with specific packages loaded, cleaned up on exit |
| `run` | One command in a throwaway environment |
| `watch` | Reconcile continuously (GitOps for one machine); unattended, applies without prompting |
| `schedule` | Native scheduled tasks (systemd, launchd, Task Scheduler) |
| `fleet` | Compare machines over SSH against your manifests and report drift |

`export` never silently overwrites: if `package.json` already exists, the export is written
beside it as `package.linix.json` and says so. `--force` overwrites deliberately.

## Safety

- **Atomic transactions.** A write-ahead log records every mutation before it runs. If LiNix is
  killed mid-transaction, the next run heals it — replaying or reverting what was in flight.
  A crash that goes unattended for hours is still healable.
- **Snapshots.** btrfs, ZFS, Timeshift and Windows Restore Points, taken automatically before a
  sync or upgrade where a provider exists.
- **Dry run.** `linix --dry-run sync` previews without touching anything. Every destructive
  command honours it.
- **Non-interactive refusals.** `sync`, `rollback` and `remove-orphans` refuse to apply
  unconfirmed changes in a pipe, cron job or CI run without `--yes`.
- **Hooks are locked.** `after_install` and friends are hashed; a changed hook must be
  re-approved with `linix lock`, so a pulled config cannot quietly start running new code.

## Configuration

`linix config init` writes a commented `config.toml`. Every key is optional. Settings cover
timeouts, concurrency (`max_parallel`), snapshot retention, notification channels, and the
`[guard]` block that holds the removal rules described above.

## Contributing

`docs/SPEC.md` is the source of truth for design; `CLAUDE.md` is the working agreement. Verify
with `cargo build --all-targets`, `cargo test`, `cargo clippy --all-targets`.
