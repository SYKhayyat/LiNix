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

```bash
curl -fsSL https://raw.githubusercontent.com/SYKhayyat/LiNix/HEAD/scripts/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/SYKhayyat/LiNix/HEAD/scripts/install.ps1 | iex
```

Either script installs the binary, runs `linix doctor`, and offers to `adopt` the packages
already on the machine. From a checkout, LiNix is written in Rust:

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

`linix path` prints where they are, so you never have to remember. To keep them somewhere else
— a dotfiles repo, a shared drive — `linix path --set ~/dotfiles/linix` records it once and
every later run finds it. For a single run, `--config-dir` wins over everything; the full order
is `--config-dir`, then `$LINIX_CONFIG_DIR`, then the stored path, then the default, and
`linix path --explain` tells you which one answered.

```
modules/       your lists of packages       lowercase names, *.txt
profiles/      named sets you turn on and off       Capitalized names
active         which profiles are on right now
priority       which package managers this machine uses, in order
vars           your own names for conditions, so `when` can ask about them
schedules      when LiNix runs itself
locks/         what everything resolved to, one file per backend
adapters/      what you have taught LiNix — see below (optional)
preferences.toml   refusals and behaviour (written by `linix config init`)
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
`target`/`content`/`template`/`decrypt`/`identity` on `link:` — the last two are
[Secrets](#secrets).

**A `link:` line puts your file back when you delete it.** If the destination already held a
file, LiNix keeps it as `<target>.linix-backup` before taking the path over; removing the
declaration restores that file and deletes the backup. So a `link:` line that comes and goes
leaves the machine as it found it, and backups do not pile up. If nothing was there to begin
with, removing the line removes the file. **The source in your repo is never touched** — a
declaration owns its destination, not your copy.

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

When a release ships two files that both fit, LiNix picks the one that names your machine
most precisely — `fd-…-x86_64-unknown-linux-gnu.tar.gz` over a bare `amd64` build — then the
shorter name, and **tells you what it chose and what it skipped**. If you write `@formats=`
yourself, that order wins instead: you asked for it. To decide yourself, `@asset=` takes a filename
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

`os` is the kernel — `linux`, `macos`, `windows`. `family` is the distribution — `debian`,
`fedora`, `arch`, `suse`, `alpine` — so `when family == debian` also covers Ubuntu and Mint,
which is usually what you meant when you asked.

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

## Your own conditions

`when` asks about facts LiNix detects — `os`, `arch`, `family`, `host`. The `vars` file lets
you name your own, and `$` is how you tell the two apart:

```
# vars
role = desktop
gpu  = none

when host in [thinkpad, x220] {
  role = travel
}

when hostname == render-01 {
  role = workstation
  gpu  = nvidia
}
```

```
# modules/tools.txt
when $role == travel {
  apt:mosh
  apt:tlp
}
```

**Every variable needs a default at the top of the file.** A `when` block may override one but
never introduce it — otherwise `role` set only inside `when host == thinkpad` is undefined
everywhere else, and `when $role == travel` on your desktop has no answer. Requiring the
default means `$role` is defined on every machine and a typo is always an error, wherever you
are sitting. LiNix enforces this by reading the whole file, not just the blocks that match here.

A value can be built from another variable, with `${}` where the name would otherwise run into
the next character:

```
role = render
tier = ${role}-heavy        # render-heavy
```

Order in the file does not matter; LiNix resolves them in dependency order and tells you if two
variables define each other in a loop. Write `$$` for a literal dollar sign. A name never starts
with a digit, so `$1` in a value is left alone.

Two `when` blocks that both match and set the same variable to different values is an error
naming both lines — the same rule as two contradicting package declarations.

## Running a script

Some machine state is not a package or a file — enrolling a TPM, importing a keyring, running a
one-off migration. `exec:` declares a script the config carries:

```
exec:./bin/enroll-tpm.sh
```

**It runs once per distinct content.** LiNix records the script's SHA-256 in `locks/exec.toml`
with a count. The next sync sees the same content at its ceiling and does nothing; edit one byte
and it is a different script, so it runs again. `@runs=3` raises the ceiling and
`@runs=always` opts out entirely — being explicit is the point, so nothing becomes a per-sync
command by accident.

**The condition is `when`, and there is no second condition system.** "Run this unless X" is a
variable your `vars` provider computes:

```
# vars.sh
tpm_enrolled = $(tpm2_getcap properties-fixed >/dev/null 2>&1 && echo yes || echo no)
```

```
when $tpm_enrolled == no {
  exec:./bin/enroll-tpm.sh
}
```

**A false `when` does not mean "undo".** This is the one place `exec:` differs from every other
statement, and it is deliberate: a script that succeeds makes its own condition false, so
treating false as removal would un-enrol the TPM on the very next sync and flap forever. A false
`when` runs nothing, undoes nothing, and **keeps the count** — so a condition that comes and goes
does not re-run the script each time it swings back.

**It is approved like any other code your repo runs.** `linix lock` approves a script at its
current hash; an unapproved or edited script stops the sync until you have looked at it, and
`-y` cannot approve. `plan` prints the hash, the run count and the decision before anything
happens:

```
Scripts:
  exec:./bin/enroll-tpm.sh  (modules/tools.txt:4)
    sha256:f7cba99726d4 — will run
```

**One limit, stated rather than implied:** the hash covers the file LiNix executes. A script that
sources another file, or curls one, changes behaviour without changing its hash, and LiNix cannot
see that.

## Secrets

**Your config repo can be public.** A secret is committed encrypted and decrypted onto the
machine at sync time, so what is in git is ciphertext and what your program reads is a normal
file with the plaintext in it.

```
link:./secrets/npmrc.age {
  target   = ~/.npmrc
  decrypt  = age
  identity = ~/.config/linix/age.key
}
```

`decrypt` takes `age` or `sops` — nothing else, and any other name is an error listing both.
LiNix does not implement encryption; it runs the tool you already trust and writes what comes
back, byte for byte.

**The identity** is `@identity=` if you set it, else `$LINIX_AGE_IDENTITY`, else
`~/.config/linix/age.key`. `sops` reads its own configuration and ignores this.

```
# encrypt once, commit the .age file, never the plaintext
age -r age1ql3z... -o secrets/npmrc.age ~/.npmrc
```

Four things that follow from this being an ordinary declaration:

- **`when` works on it.** `when hostname == build-01 { link:./secrets/ci-token.age { … } }` is
  a secret that exists on one machine.
- **`--dry-run` never decrypts.** It tells you what it would write and stops, because a dry run
  that produced a plaintext file would be the leak.
- **Removing the line removes the plaintext**, the same as any other managed file.
- **The plaintext is restricted before it exists.** On Linux and macOS it is owner-only
  (`0600`); on Windows the file is created with inherited access stripped and only your account
  granted, using `icacls`. Either way the restriction is applied to a temporary file which is
  then renamed into place, so there is no moment when the destination holds a readable secret.
- **A `target` inside your config repo is refused.** The repo is git and `sync` commits it, so a
  plaintext there would be a plaintext in history — and a secret in history has to be rotated,
  not deleted. The error names the path and the repo.
- **No backup is taken for a secret.** For ordinary managed files LiNix keeps your original as
  `<target>.linix-backup`; for a decrypted one it does not, because that copy would be the
  previous secret sitting in plaintext beside the new one.

Your `identity` key itself is never managed by LiNix and never belongs in the repo. LiNix's own
credentials work the other way round and are never files at all: `GITHUB_TOKEN` is read from the
environment, so a LiNix config is always safe to hand to someone.

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

Commits are made **as you** — LiNix sets no git identity of its own and forces no signing flag,
so `commit.gpgsign` decides whether your history is signed. `linix git log` and `linix history`
show what git says about each commit's signature, and a signature git will not vouch for (an
untrusted, expired or revoked key) is never shown as a good one. Set `require_signed_history`
under `[guard]` to refuse a rollback to any commit git does not vouch for; it is off by default,
because a fresh repo signs nothing.

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
| `rebuild` | Remove and reinstall what is declared, to repair what `sync` cannot see |

**Understanding the machine**

| | |
|---|---|
| `check` | Parse everything the active profiles reach; report errors, change nothing |
| `why` | Why a package is installed: where it is declared and what depends on it |
| `unmanaged` | Installed on the OS but not managed by LiNix |
| `absent` | Every `absent:` rule in force, and which module it comes from |
| `conflicts` | The same tool pinned to different versions by different backends |
| `doctor` | Per-backend readiness, config and layout integrity; `--fix` repairs what is safe |
| `path` | Print your config repo directory, so `cd $(linix path)` works. `--explain` says what decided it; `--set DIR` stores it |
| `edit` | Open the repo, or one file in it, in `$VISUAL`/`$EDITOR` |

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

### When `sync` says "nothing to do" and something is still broken

`sync` applies the *difference* between your files and the machine. A package that is declared
and installed but broken — a half-configured install, an interrupted download, a closure
something else removed — produces no difference, so `sync` will report success over it forever.

`linix rebuild` stops asking what changed and asserts the declared set from scratch:

```
linix rebuild fd ripgrep       one or more packages (cargo:fd picks a backend)
linix rebuild --backend cargo  everything that backend declares
linix rebuild --all            every declared package on this machine
```

There is no default scope — it removes software in order to put it back, so it makes you say
what. It works **one backend at a time**: all of that backend's packages come down together
(which is what actually lets a shared dependency become an orphan and get collected), then all
of them go back up, then the next backend. Backends that need root go first, because a crate can
need a system compiler and no system package has ever needed a crate.

It never touches undeclared software, and it never removes a protected package — those are
named and skipped rather than rebuilt. It cannot be put in `schedules`.

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

## Teaching LiNix a package manager it has never heard of

If a manager's CLI has plain install/remove/list verbs, LiNix can learn it from data — no
Rust, no release. Write `adapters/backends.toml` in your repo:

```toml
[[backend]]
name   = "firewall"        # the prefix a line is written with
binary = "ufw"             # the program actually run; defaults to `name`
install_args = ["allow"]
remove_args  = ["delete", "allow"]
list_args    = ["status", "numbered"]
[backend.parser]           # how to read `list` output
format = "columns"
name_col = 0
```

`firewall:22/tcp` then works everywhere a built-in prefix works. Because `name` and `binary`
are separate, the prefix does not have to be a package manager's name — it can be any noun
that has a CLI behind it.

**They live in the repo, so they travel**, which is the point: a definition on one machine
makes every other machine fail on a line it cannot resolve. And because each is a list of
commands your repo can run on any machine that clones it, each is approved the way a hook is —
`linix lock` approves them, and any later edit stops that file loading until you look at the
change and approve it again. **Each file is approved separately**: approving the backends you
added is not a review of the settings adapters. Custom definitions load last and can never take
over a built-in name.

Everything you teach LiNix lives in one folder, one file per question:

```
adapters/backends.toml    how to drive a package manager LiNix does not ship
adapters/settings.toml    how to read and write a settings store
adapters/bootstrap.toml   how to obtain a manager this machine does not have
```

**Its sibling teaches LiNix a settings store.** `setting:` writes desktop configuration that
does not live in a file — GNOME's store via `gsettings` is shipped, and any other is a row in
`adapters/settings.toml`:

```toml
[[setting_store]]
name   = "kde"
detect = "kwriteconfig6"            # its presence means this machine runs this store
read   = ["kreadconfig6",  "--file", "{schema}", "--key", "{key}"]
write  = ["kwriteconfig6", "--file", "{schema}", "--key", "{key}", "{value}"]
reset  = ["kwriteconfig6", "--file", "{schema}", "--key", "{key}", "--delete"]
```

All three commands are required. LiNix reads before it writes — that is what makes a setting a
declaration rather than a command that runs on every sync — and `reset` is what removing the
line does. A machine whose store has no row gets an error naming what LiNix looked for, never a
key that silently did nothing.

## Configuration

`linix config init` writes a commented `preferences.toml` into your repo; `linix edit
preferences.toml` opens it and re-checks that it still parses when you save. Every key is
optional. Settings cover timeouts, concurrency (`max_parallel`), snapshot retention,
notification channels, and the `[guard]` block that holds the removal rules described above.

**Where your repo lives is not a key in it.** `preferences.toml` sits *inside* the repo, so a
key there could only be read from the directory it was trying to move away from. That one
setting lives in LiNix's own settings file, beside the repo rather than in it — set it with
`linix path --set DIR`, override it for one command with `--config-dir`, and ask which of the
four sources won with `linix path --explain`.

## Contributing

`docs/SPEC.md` is the source of truth for design — the map, with the parts themselves under
`docs/spec/`; `docs/spec/decisions.md` is every open question. `CLAUDE.md` is the working
agreement. Verify with `cargo build --all-targets`, `cargo test`, `cargo clippy --all-targets`.
