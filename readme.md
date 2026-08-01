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

Either script installs the binary, runs `linix check`, and offers to `adopt` the packages
already on the machine. From a checkout, LiNix is written in Rust:

```bash
cargo build --release
cp target/release/linix ~/.local/bin/
```

## Start

```bash
linix init          # scaffold ~/.config/linix, with one profile (Main) already active
linix install jq    # writes a line you own, then syncs
linix check         # what needs you: drift, unmanaged, health — read-only
linix sync          # make the machine match the files
```

`linix install` is not a separate mechanism — it writes `jq` into a module that the active
profile already reaches, then syncs. Anything it can do, editing the file does too.

Writing a module by hand takes one extra step, because a module is inert until something uses
it:

```bash
echo 'cargo:ripgrep' > ~/.config/linix/modules/tools.txt
echo 'use tools'    >> ~/.config/linix/profiles/Main
linix check                # every question at once; `linix check drift` for one
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
groups         named backend chains, so `tools:rg` means `apt,cargo:rg` (optional)
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

A prefix can be a chain — `apt,cargo:ripgrep` means "apt if it has it, else cargo." If you write
the same chain often, name it once in a `groups` file and use the name:

```
# groups
tools   = apt, dnf, cargo
windows = scoop, winget
all     = tools, windows      # groups can contain groups
```

Then `tools:ripgrep` expands to `apt,dnf,cargo:ripgrep`. It is only a shortcut — it resolves
exactly as the chain would, `priority` is unchanged, and a group that reaches itself is refused.

**A name is whatever the manager calls it.** If `linix list` prints it, you can write it back:

```
npm:@angular/cli                       # a scoped package — the leading @ is part of the name
npm:@angular/cli@version=17.3.0        #   ...and a later @ still opens the options
winget:ARP\Machine\X64\{8BD2A40D-...}    # what winget calls an installed MSI
cargo:serde_json                       # underscores, dots, plus signs, slashes
```

That is a rule rather than a list of exceptions: a manager's names are facts, and where they and
the grammar disagree, the grammar gives way (V.113). The one thing a name may never contain
is `..`.

A config file may also start with a **byte-order mark** — what Notepad writes — and LiNix reads
it anyway (Q22). It is an encoding artefact, not part of your first backend's name.

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
`requires`, `health` (see [Safety](#safety)), `shim` (put a PATH stand-in for this tool in your
`bin_dir`; `sandbox` does that and confines `linix run` too — both declare the same thing a
`shim:` line does, so adding one to a package you already have creates the stand-in and deleting
it takes the stand-in away), the `*_install` hooks, and
per-directive keys like `cron`/`run` on `schedule:` or
`target`/`content`/`template`/`decrypt`/`identity` on `link:` — the last two are
[Secrets](#secrets).

Some keys belong to one family of backends and are refused, by name, anywhere else — `@classic`
on a snap, and the storage keys below. An option no backend would read is an error rather than a
line that quietly does nothing.

**Adding `@classic` to a snap you already installed re-confines it** on the next sync, rather than
waiting for a reinstall. Taking it away manages nothing — LiNix will not silently reconfine a
snap because a word left the file. Writing `@classic=false` on a snap that *is* classic is
refused, because snapd can relax confinement in place but cannot narrow it, and the only way back
is to remove and reinstall — which the error tells you.

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

### Storage you can declare

A btrfs subvolume, a ZFS dataset and an LVM logical volume are declarations like any other —
they have a size and a mountpoint rather than a version, and that is the only thing that makes
them different:

```
btrfs:/mnt/fs/srv@quota=20G,mount=/srv
zfs:tank/media@quota=500G,mount=/mnt/media
lvm:vg0/data@size=100G
```

`@size` is required on `lvm:` — `lvcreate` has no default, so a volume with no size is not a
declaration of anything. `@mount` writes the entry to `/etc/fstab`, which is what makes the
mount survive a reboot; deleting the line takes the entry out **before** the volume is
destroyed, because an fstab entry naming a volume that no longer exists stops the next boot.
`@mount_options` fills that entry's option field — btrfs only, since ZFS keeps its mount
properties on the dataset, and an error without `@mount`, because there is then no entry for it
to fill.

**Editing one of these numbers changes the volume.** Raise `@quota` and the next sync raises the
quota; raise `@size` on an `lvm:` volume and it grows, filesystem and all. Lowering `@size`
shrinks it, and that is the one change here that can lose data — so it needs saying on the line:

```
lvm:vg0/data@size=50G,allow_shrink=true
```

Without `@allow_shrink`, a smaller `@size` is refused and the error names what the volume is now
and what you asked for. With it, LiNix shrinks the filesystem before the volume, so a filesystem
that cannot shrink (xfs) stops the operation rather than losing its tail. `@allow_shrink` is
`lvm:` only — lowering a quota takes nothing away — and an error without `@size`, because on its
own it permits nothing. Dropping an option stops declaring it; it does not lift what it declared,
so deleting `@quota=` leaves the quota where it is.

**Deleting one of these lines destroys a filesystem, and that goes through the ordinary removal
guard** — no special escalation, because ordinary is already the strongest gate here. A volume
is protectable like a package, counts against `max_removals`, and the destruction is previewed
before the guard clears it.

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
state file from another machine. So **every path that removes anything** goes through one guard.
That covers packages *and* the resources a declaration puts in place — a `link:`, `service:`,
`setting:`, `shim:`, `schedule:` or `repo:` line that leaves your modules is torn down under the
same rules, and counts against the same limit. The sentence you just read is checked by
`tests/removal_guard_enumeration_tests.rs`, which counts the removal paths in the source on every
run; it was written because the sentence was false for the whole resource family until
2026-07-28, and nothing had re-counted since it was first written.

The guard refuses when a removal:

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

**`exec:` is for actions with no inverse, not for installing software.** If you find yourself
writing `exec:` lines that install a package — shelling out to some manager LiNix doesn't know —
that is the sign to teach LiNix that manager instead: [a backend is six lines of
TOML](#teaching-linix-a-package-manager-it-has-never-heard-of), and then LiNix can install,
*remove*, list and lock it like any other. An `exec:` that installs something is a one-way door:
deleting the line does not undo it, because a verb has no teardown. The onboarder gives you the
noun, which does. Reach for `exec:` when there is genuinely nothing to declare — a side effect,
not a resource.

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

**Removing the line runs `@undo=`, if you gave one:**

```
exec:./bin/enroll-tpm.sh {
  undo = tpm2_clear
}
```

Delete the line and LiNix runs `tpm2_clear`, then forgets the script. Without an `@undo=`,
removing the line just drops the record — LiNix cannot invent an inverse for a script, and
`plan` says so in those words rather than implying a revert that will not happen.

**A `when` going false is not a removal.** The line is still in your file, so nothing is undone;
that is what stops the enrol script un-enrolling itself on the very next sync. Only deleting the
line from the file counts, and deactivating a profile does not.

**One limit, stated rather than implied:** the hash covers the file LiNix executes. A script that
sources another file, or curls one, changes behaviour without changing its hash, and LiNix cannot
see that.

## The firewall

One line opens a port, and means the same thing on every machine:

```
firewall:22/tcp
firewall:default/incoming @value=deny
```

LiNix drives whichever firewall the machine runs — `ufw`, `firewalld` or Windows Defender —
so the same config opens port 22 on a Debian laptop and a Windows workstation. A firewall LiNix
does not know is a `[[firewall]]` row in `adapters/firewall.toml`, not a new release.

**A declared port is open; deleting the line closes it.** That is what declaring means, so
`firewall:22/tcp` takes no `@value=`. Only `default/incoming` and `default/outgoing` do.

**LiNix will not close the port you are connected over.** Before any command runs, it checks
whether the change would cut the session it is being typed into — including the subtle case
where tightening `default/incoming` closes your port without ever naming it:

```
refusing to apply the firewall change: it would close port 22, which is carrying this session.
  LiNix is being run over that port, so applying this would end the connection and leave no way
  back in.
  Declare `firewall:22/tcp` to keep it open, or make this change from the machine's own console.
```

That check runs on every path that can close a port — including an unattended `watch` tick,
which is the dangerous one, because nobody is there to read a refusal.

**Drift is corrected.** A rule someone added by hand is removed on the next sync, like any other
drift — with the one exception above. If your firewall is also configured by a `link:`ed ruleset
file, LiNix warns that two things own the perimeter and lets your declared rules win.

## A folder of dotfiles

If your dotfiles already sit in a tree that mirrors `$HOME`, say so once instead of writing
forty `link:` lines:

```
dotfiles:./dotfiles
```

Every file under `./dotfiles` is linked to the matching place under your home directory —
`./dotfiles/.config/nvim/init.lua` becomes `~/.config/nvim/init.lua`. `@target=` mirrors
somewhere else.

**Files, never directories.** Linking `~/.config/nvim` as a whole would take the directory
hostage: every cache, session file and plugin lockfile the app later writes lands inside your
git repo, and `bundle` would hand it to whoever gets the backup. So each file is linked
individually, and the directory stays yours.

**A destination that already holds your own file stops the run** — all of them at once, listed,
before anything is written:

```
3 destination(s) already hold a file LiNix did not put there:
    /home/me/.bashrc
    ...
Nothing has been written. Move or delete them, or re-run with `--replace-existing`.
```

On a fresh machine those are usually untouched distribution defaults, which is what
`--replace-existing` is for. It is a per-run flag and deliberately not a config key: a machine
that always bypasses the check is one where the check does not exist.

**The tree never decrypts.** A `.age` file in it is linked as the ciphertext it is — deciding by
file extension would be magic that silently writes plaintext. Secrets stay on explicit `link:`
lines where `@decrypt=` is written down.

**Several trees are fine** (`dotfiles:./work` under a `when`). Two trees that would place the
same destination is an error naming both.

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

### One command looks, one command acts

`linix check` answers every "what is going on" question — drift, unmanaged software, `absent:`
lines in force, conflicting declarations, backend health, known advisories, and whether any hook
you wrote is unapproved and so will silently never run. With no argument it
prints a line per section and names the command that acts on each:

```
ok  config      42 package(s) declared
->  drift       3 to install, 1 to remove
                   run `linix sync`
->  unmanaged   103 package(s) LiNix does not manage
                   run `linix adopt`
ok  health      26 backend(s) ready
```

`linix check health` (or `drift`, `unmanaged`, `absent`, `conflicts`, `config`, `security`,
`approvals`) prints that section in full.

**`check` never changes anything.** What used to be `doctor --fix` — creating missing
directories, reconciling the lockfile, refreshing a stale backend index — is `linix heal`, along
with recovering an interrupted run. A command that both diagnoses and repairs is one you cannot
run to find out whether you want a repair.

Run `linix --help` for the full list with current wording, and `linix check health` for what this
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
| `add` | Vendor someone else's modules into your repo from `github:owner/repo`, a git/file URL, or a path. Their code arrives unapproved until `linix lock` |
| `unmanage` | Stop managing a package **without** uninstalling it |
| `remove-orphans` | Remove what each manager considers orphaned — shows the list and asks first |
| `clean-cache` | Delete downloaded archives and caches; removes no installed package |
| `purge-unmanaged` | Delete everything LiNix does not manage. Shows the whole list first |

**Plan, lock, reproduce**

| | |
|---|---|
| `plan` / `apply` | Freeze what `sync` would do to a file, review it, then apply exactly that |
| `eval` | Print the resolved config as versioned JSON — every `when` decided, every bare name given a backend. Takes no locks |
| `try` | Rehearse this config on a clean machine in a container. Answers what `plan` cannot: would it work somewhere that is not here? |
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
- **Dry run.** `linix --dry-run sync` previews without touching anything — and so does every
  other command, because the flag is honoured by the single function every file LiNix owns is
  written through, not by each command remembering to ask. That includes `data/registry.json`,
  the record of what LiNix manages: a preview that quietly recorded it would leave your packages
  managed and undeclared, which is the state the next `sync` reads as *remove all of these*.
- **Non-interactive refusals.** `sync`, `rollback` and `remove-orphans` refuse to apply
  unconfirmed changes in a pipe, cron job or CI run without `--yes`.

> **Filesystem-level rollback is Linux-first.** The pre-sync snapshot, `rebuild`'s revert and
> `rollback`'s safety net all depend on a snapshot provider — btrfs, ZFS or Timeshift on Linux,
> Windows System Restore on Windows. **macOS has no adapted provider yet**, so on macOS those
> commands run without a filesystem restore point: the git history still records every change
> and `linix rollback <commit>` still re-syncs packages, but there is no block-level undo. A
> health check that would revert (`@health=`) is *refused before the change* on a machine with
> no provider rather than run without a way back (see above), so this never fails silently.
- **Hooks are locked.** `after_install` and friends are hashed; a changed hook must be
  re-approved with `linix lock`, so a pulled config cannot quietly start running new code.
- **Hooks on LiNix's own events.** Put a script at `hooks/after_sync`, `hooks/on_drift` or
  `hooks/on_guard_refusal` and it runs with the details on stdin as JSON — notify a channel,
  push the repo, open a ticket, without any of that having to become a LiNix feature. The same
  three may live in `preferences.toml`'s `[events]` table for hooks that are this machine's business rather than
  the repo's; **both run**, so adding a local one never silently disables the shared one. They
  are locked like any other script, and one that fails warns without failing the sync.
  - **Slack, ntfy, webhooks, Telegram, paging — any channel — go through that hook, not a
    separate setting.** There is no `[[channel]]` block, because a hook already sends anything a
    `curl` can send, and two ways to do one thing is the thing this design removes. A copyable
    `hooks/after_sync` that posts to Slack:
    ```sh
    #!/bin/sh
    # stdin is the event as JSON. The webhook URL is an ENV var, never the repo — a secret in
    # a committed file is a leaked secret (secrets are environment-only in LiNix).
    payload=$(cat)
    curl -sf -X POST -H 'Content-type: application/json' \
      --data "$(printf '{"text":"linix on %s: %s"}' "$(hostname)" "$payload")" \
      "$LINIX_SLACK_WEBHOOK"
    ```
    Approve it once with `linix lock` (it runs code, so the ledger gates it), and swap the
    `curl` line for ntfy, a Telegram bot URL, or a PagerDuty event — the mechanism is the same.
    The built-in `desktop`/`email` channels stay for the common case; everything else is this.
- **Health checks revert.** `apt:nginx@health=port:80` on a line, or a machine-wide
  `health = [...]` in `preferences.toml`. A failing check restores the snapshot the sync took
  before it started — and a health check declared on a machine with no snapshot provider is
  refused *before* the change, because telling you the machine broke without being able to put
  it back is worse than not checking.

### Exit codes

The same four everywhere, so a script can branch on them:

| code | meaning |
|---|---|
| `0` | converged — what you declared is what is there |
| `1` | failed — something went wrong |
| `2` | differences — a read-only command looked and found work to do |
| `3` | refused — LiNix said no, and there is no flag for it |

**`3` covers every refusal, not only the guard's.** Refusing to download over plain HTTP, to
install something with no `@sha256`, to write a secret the filesystem cannot protect, to decrypt
into the git repo, to run an unapproved hook, to overwrite a file LiNix did not create, or to
place files outside `$HOME` all return `3` — the same code as refusing to remove too many
packages. Until 2026-07-28 those nine returned `1`, so a script could not tell "I refused" from
"I broke", and the `on_guard_refusal` hook never fired for any of them. Both halves are now
checked by `tests/grader_refusal_exit_code_tests.rs` rather than asserted in a comment.

`2` is why `linix check` in CI tells you a machine has drifted without failing the job the way
an error would, and `3` is distinct from `1` because "I will not do this" is not "this broke".

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
that has a CLI behind it. And `binary` may be an absolute path (`/opt/vendor/tool`, `~/bin/x`),
not just a `$PATH` name — a missing one is a named diagnosis in `check health`, not a refusal.

**A custom backend is a full peer of a built-in** — the same optional keys the shipped
backends use are available to yours, and an absent key means *this backend cannot answer that*,
never *the answer is none* (so `re:` against a backend with no `enumerate_args` is refused, not
expanded to nothing):

```toml
[[backend]]
name = "mymgr"
install_args = ["add"]
remove_args  = ["rm"]
list_args    = ["list"]
# first-class extras, each optional:
essential_args   = ["essential"]         # what the removal guard must never take
enumerate_args   = ["list", "--all"]     # the catalogue `re:` expands against
depends_args     = ["deps"]              # a package's dependencies
repo_add_args    = ["repo", "add"]       # `repo:` lines
repo_remove_args = ["repo", "rm"]
repo_list_args   = ["repo", "list"]
repo_binary      = "mymgr-sources"       # when sources are edited by another program
repo_list_binary = "cat"                 # …and read by another one again
purge_args       = ["rm", "--purge"]     # config-destroying removal
manual = "all_installed"                 # so `adopt` takes what you chose, not deps
[backend.orphan_dry_run]                 # what its autoremove WOULD remove
args = ["autoremove", "--dry-run"]
removes_line_prefix = "Remv"
```

**They live in the repo, so they travel**, which is the point: a definition on one machine
makes every other machine fail on a line it cannot resolve. And because each is a list of
commands your repo can run on any machine that clones it, each is approved the way a hook is —
`linix lock` approves them, and any later edit stops that file loading until you look at the
change and approve it again. **Each file is approved separately**: approving the backends you
added is not a review of the settings adapters.

**Overriding a built-in.** Custom definitions load last, and a name that is already taken is
skipped — being named `apt` is not a way to become `apt`. To replace one on purpose, say so:

```toml
[[backend]]
name = "apt"
overrides = true          # take the name from the built-in
binary = "apt-fast"
install_args = ["install", "--assume-yes"]
```

This is for the day a manager changes its CLI and LiNix has not caught up yet: you can correct
it on your machines without waiting for a release. It costs two deliberate acts — the
`overrides = true` line, and `linix lock` approving the file — and neither one alone does
anything. LiNix says so on every run that loads it, naming the backend and the program it now
runs, and `check health` then reports on *your* definition: if `apt-fast` is not installed, `apt`
is critical, because on this machine that is the truth.

Everything you teach LiNix lives in one folder, one file per question:

```
adapters/backends.toml    how to drive a package manager LiNix does not ship
adapters/settings.toml    how to read and write a settings store
adapters/bootstrap.toml   how to obtain a manager this machine does not have
adapters/prereq.toml      the setup a manager needs before it can install anything
```

**The last one is for a manager that is installed and still cannot install anything.** `mix`
needs Hex, `asdf` needs the plugin for the tool you named, `opam` needs a switch — each of them
fails every install until one command has been run. LiNix ships those three and asks before
running any of them; `--yes` answers in advance, and a run with no terminal says what it would
have asked and changes nothing.

```toml
[[prereq]]
manager      = "mix"
missing      = "Hex, the package client `mix archive.install hex …` fetches through"
probe        = ["mix", "hex.info"]      # exit 0 means it is already there
run          = ["mix", "local.hex", "--force"]

[[prereq]]
manager      = "asdf"
missing      = "asdf's `{name}` plugin"
probe        = ["asdf", "plugin", "list"]
probe_output = "{name}"                 # this row reads OUTPUT: `asdf plugin list` exits 0
run          = ["asdf", "plugin", "add", "{name}"]   # `{name}` = once per declared package
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

`@scope=user` (the default) or `@scope=system` chooses which store a setting goes to — `HKCU`
or `HKLM` on Windows, and the same word on every other store. Writing the default is fine; it
just says out loud what you would have got anyway. **A store with no machine-wide commands
refuses `@scope=system` by name** rather than quietly writing the per-user value, so a line
that says "every account" never silently applies to one. The same key works on `link:` and
`shim:`.

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
