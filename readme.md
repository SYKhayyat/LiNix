# Shall

Declarative configuration for a machine: you write down what it should have, and `sync` makes
the machine match. The name is what a line in that file means: *this machine **shall** have
ripgrep.*

**Packages are the largest kind and not the only one.** Repositories, services, schedules,
symlinks, OS and desktop settings, scripts, generated declarations, dotfile trees and firewall
rules are all declared in the same files, by the same grammar, and converged by the same
`sync`.

Shall does not replace apt, pacman, brew, cargo or npm — nor systemd, ufw or gsettings. It
drives them. One file says what the machine should have; `shall sync` adds what is missing,
removes what is no longer listed, and leaves everything else alone.

```
$ cat ~/.config/shall/modules/tools.txt
apt:ripgrep
cargo:bat
npm:typescript@version=>=5.0.0

$ cat ~/.config/shall/profiles/Main
use tools

$ shall sync
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
curl -fsSL https://raw.githubusercontent.com/SYKhayyat/Shall/HEAD/scripts/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/SYKhayyat/Shall/HEAD/scripts/install.ps1 | iex
```

Either script downloads the published binary for your platform, runs `shall check`, and offers
to `adopt` the packages already on the machine. Nothing else is needed — no toolchain, no
compiler. Published builds are x86_64 Linux, both Apple architectures, and x86_64 Windows;
anywhere else the script falls back to building from source, which needs
[Rust](https://rustup.rs) and takes rather longer than thirty seconds.

> **No release has been tagged yet, so today every platform takes the fallback.** The scripts
> above resolve to a release that does not exist, notice, and build from source — which needs
> Rust and takes about fifteen minutes. Nothing is broken and nothing lies to you at the
> prompt; the sentence above is simply a promise the repository has not kept yet. Delete this
> note in the commit that pushes the first `v*` tag.

`SHALL_REF=v0.8.0` installs an exact release instead of the newest. From a checkout:

```bash
cargo build --release
cp target/release/shall ~/.local/bin/
```

## Start

```bash
shall init          # scaffold ~/.config/shall, with one profile (Main) already active
shall install jq    # writes a line you own, then syncs
shall check         # what needs you: drift, unmanaged, health — read-only
shall sync          # make the machine match the files
```

`shall install` is not a separate mechanism — it writes `jq` into a module that the active
profile already reaches, then syncs. Anything it can do, editing the file does too.

Writing a module by hand takes one extra step, because a module is inert until something uses
it:

```bash
echo 'cargo:ripgrep' > ~/.config/shall/modules/tools.txt
echo 'use tools'    >> ~/.config/shall/profiles/Main
shall check                # every question at once; `shall check drift` for one
shall --dry-run sync       # preview
shall sync
```

`shall check` is the fastest way to confirm a file is actually being read: it reports how many
lines resolved. If you edited a module and `check` still says `0 present`, no active profile
is using it.

## The files

`shall init` creates them under `$SHALL_CONFIG_DIR` (default `~/.config/shall`). **This
directory is meant to be a git repo** — `shall git init` turns on version control, after which
every sync commits, and `shall rollback <commit>` puts the machine back.

`shall path` prints where they are, so you never have to remember. To keep them somewhere else
— a dotfiles repo, a shared drive — `shall path --set ~/dotfiles/shall` records it once and
every later run finds it. For a single run, `--config-dir` wins over everything; the full order
is `--config-dir`, then `$SHALL_CONFIG_DIR`, then the stored path, then the default, and
`shall path --explain` tells you which one answered.

```
modules/       your lists of packages       lowercase names, *.txt
profiles/      named sets you turn on and off       Capitalized names
active         which profiles are on right now
priority       which package managers this machine uses, in order
groups         named backend chains, so `tools:rg` means `apt,cargo:rg` (optional)
vars           your own names for conditions, so `when` can ask about them
schedules      when Shall runs itself
locks/         what everything resolved to, one file per backend
adapters/      what you have taught Shall — see below (optional)
preferences.toml   refusals and behaviour (written by `shall config init`)
```

Shall's own bookkeeping — what it currently owns, snapshot metadata — lives in
`$SHALL_DATA_DIR`, never in the config repo and never in git.

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

**A name is whatever the manager calls it.** If `shall list` prints it, you can write it back:

```
npm:@angular/cli                       # a scoped package — the leading @ is part of the name
npm:@angular/cli@version=17.3.0        #   ...and a later @ still opens the options
winget:ARP\Machine\X64\{8BD2A40D-...}    # what winget calls an installed MSI
cargo:serde_json                       # underscores, dots, plus signs, slashes
```

That is a rule rather than a list of exceptions: a manager's names are facts, and where they and
the grammar disagree, the grammar gives way (V.113). The one thing a name may never contain
is `..`.

A config file may also start with a **byte-order mark** — what Notepad writes — and Shall reads
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
`bin_dir`; `sandbox` does that and confines `shall run` too — both declare the same thing a
`shim:` line does, so adding one to a package you already have creates the stand-in and deleting
it takes the stand-in away), the `*_install` hooks, and
per-directive keys like `cron`/`run` on `schedule:` or
`target`/`content`/`template`/`decrypt`/`identity` on `link:` — the last two are
[Secrets](#secrets).

Some keys belong to one family of backends and are refused, by name, anywhere else — `@classic`
on a snap, `@system` on pip, and the storage keys below. An option no backend would read is an
error rather than a line that quietly does nothing.

**`pip:` on Ubuntu, Debian, Alpine, openSUSE or Fedora.** Those distros mark their Python as
belonging to the system package manager (PEP 668), and pip refuses to install into it — rightly,
because two package managers writing one `site-packages` is how a system python stops working.
Shall refuses with them and names the two things a line can do instead:

```
pipx:black                 # its own environment — the tool built for this, and Shall drives it
pip:black@system=true      # write into the system Python anyway, on this line only
```

`@system` applies to the line that carries it and to nothing else: a wave containing both forms
becomes two commands, so one package's permission is never handed to the ones beside it.

**Adding `@classic` to a snap you already installed re-confines it** on the next sync, rather than
waiting for a reinstall. Taking it away manages nothing — Shall will not silently reconfine a
snap because a word left the file. Writing `@classic=false` on a snap that *is* classic is
refused, because snapd can relax confinement in place but cannot narrow it, and the only way back
is to remove and reinstall — which the error tells you.

**A `link:` line puts your file back when you delete it.** If the destination already held a
file, Shall keeps it as `<target>.shall-backup` before taking the path over; removing the
declaration restores that file and deletes the backup. So a `link:` line that comes and goes
leaves the machine as it found it, and backups do not pile up. If nothing was there to begin
with, removing the line removes the file. **The source in your repo is never touched** — a
declaration owns its destination, not your copy.

### Which file gets installed

`github:sharkdp/fd` names a repo, not a file. One release ships a `.deb`, a `.tar.gz`, an
`.AppImage` and a bare binary, so Shall has to choose — and a declaration that resolves to a
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

When a release ships two files that both fit, Shall picks the one that names your machine
most precisely — `fd-…-x86_64-unknown-linux-gnu.tar.gz` over a bare `amd64` build — then the
shorter name, and **tells you what it chose and what it skipped**. If you write `@formats=`
yourself, that order wins instead: you asked for it. To decide yourself, `@asset=` takes a filename
or a glob that survives version bumps:

```
github:sharkdp/fd@asset=*musl*
```

For an archive, Shall extracts it, finds the executable and shims it onto your `PATH`. When the
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
and what you asked for. With it, Shall shrinks the filesystem before the volume, so a filesystem
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

**And you do not have to write one.** A line naming a manager this machine does not have is
skipped, not an error:

```
apt:ripgrep
winget:BurntSushi.ripgrep
brew:ripgrep
```

That file works on all three machines. Each one installs its own line and reports the other two
as skipped, naming the manager it does not have — `sync` still succeeds. `when` is for when you
want a *different* package on a different host; this is for the same package under three names,
which is most of what a portable config is.

Skipped, never silent: the skips are listed at the end of a run and are in `--json`, and a run
whose every line was skipped does not claim to be up to date. A **misspelled** manager is still
an error — `brwe:ripgrep` is caught when the file is read, because a config that quietly ignored
its own typos would describe a machine nobody has.

None of this applies to a package that genuinely fails to install: that stops the command, on
every machine. Pass `--keep-going` if you would rather have the rest of the run — it installs
each package on its own command line so a bad name cannot take the good ones with it, and it
**still exits non-zero**, naming what failed. Continuing past a failure is not reporting success.

## Profiles

A profile is a named set of modules you can turn on and off live, with no reboot. Several can
be active at once — their package sets are unioned.

```bash
shall activate work           # `active` becomes exactly: work
shall activate -a gaming      # add gaming to what is already on
shall deactivate gaming       # drop it, removing only what nothing else needs
shall profile list
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
same rules, against its own limit. The sentence you just read is checked by
`tests/removal_guard_enumeration_tests.rs`, which counts the removal paths in the source on every
run; it was written because the sentence was false for the whole resource family until
2026-07-28, and nothing had re-counted since it was first written.

The guard refuses when a removal:

- exceeds a ceiling. There is one per kind and one over all of them: `max_removals`
  (packages, default 20), `max_extra_removals` (resource teardowns, 20), `max_port_closures`
  (ports nothing declares, 20), `max_installs` (off by default) and `max_total_changes` —
  everything one command does, installs and upgrades included, off by default. A refusal names
  every ceiling it hit, because a set can be over two at once,
- touches a protected package — a built-in list, anything you add, **and** the OS's own
  essential flags where it has them (`dpkg`'s `Essential` / `Priority: required`),
- or trips one of the `[guard]` policy rules.

`shall protected` prints the effective rules, every ceiling included. The override for a
removal count is `--allow-mass-removal`, for an install count `--allow-mass-install`, and either
answers the total — a total is made of both. **`--yes` is deliberately not an override**, because every script and CI
job passes `-y`, and an unattended run is exactly the one that cannot notice a system being
taken apart. Protection is a refusal, not a confirmation: nothing overrides it.

## Your own conditions

`when` asks about facts Shall detects — `os`, `arch`, `family`, `host`. The `vars` file lets
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
are sitting. Shall enforces this by reading the whole file, not just the blocks that match here.

A value can be built from another variable, with `${}` where the name would otherwise run into
the next character:

```
role = render
tier = ${role}-heavy        # render-heavy
```

Order in the file does not matter; Shall resolves them in dependency order and tells you if two
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
writing `exec:` lines that install a package — shelling out to some manager Shall doesn't know —
that is the sign to teach Shall that manager instead: [a backend is six lines of
TOML](#teaching-shall-a-package-manager-it-has-never-heard-of), and then Shall can install,
*remove*, list and lock it like any other. An `exec:` that installs something is a one-way door:
deleting the line does not undo it, because a verb has no teardown. The onboarder gives you the
noun, which does. Reach for `exec:` when there is genuinely nothing to declare — a side effect,
not a resource.

**It runs once per distinct content.** Shall records the script's SHA-256 in `locks/exec.toml`
with a count. The next sync sees the same content at its ceiling and does nothing; edit one byte
and it is a different script, so it runs again. `@runs=3` raises the ceiling and
`@runs=always` opts out entirely — being explicit is the point, so nothing becomes a per-sync
command by accident.

**`@on=` says which verb runs it.** `sync` is the default; `upgrade` and `both` are the other
two. `shall upgrade` moves managed packages, which is the one thing a package manager can be
asked to do — everything else a machine needs brought forward is a command:

```
exec:./bin/firmware.sh   @runs=always,on=upgrade
exec:./bin/rustup-up.sh  @runs=always,on=both
```

Both lines above run under `shall upgrade`; only the second also runs under `shall sync`. A step
has to name `upgrade` to be run by it, and a line that says nothing runs on `sync` alone — so
adding this changed nothing about scripts you had already written. Approving a script says *this
content may run here*; it does not say *any verb may run it*, and that is a separate sentence you
write on the line.

**The common ones ship with Shall, so you write a name instead of a script:**

```
exec:step/rustup
exec:step/gcloud
```

That is the whole line. Each shipped step knows which verb it belongs to (`upgrade`) and that it
should run every time, so there are no options to remember — and a step whose tool is not on this
machine is skipped rather than failed, which is what makes one config work across a laptop and a
server. `shall check config` lists the names available here, and an unknown name is refused when
the config is read rather than when it runs.

**These need no `shall lock`, and your own scripts still do.** A shipped step is a row compiled
into the binary — a fact about a tool, not code your config carries — reviewed in Shall's own
repository and shipped with the rest of the program. The approval gate exists so that code
*travelling in your configuration* is read by a human before it runs; requiring you to approve
Shall's own rows as well would move that check somewhere it means nothing, and delete the reason
to have a catalogue at all. You approve what you wrote. You approved the rest by installing it.

`exec:step/fwupd` is the exception worth knowing about: it refreshes firmware *metadata* and
never flashes anything. A config file that writes your BIOS unattended on a weekly upgrade is not
a convenience, and no snapshot rolls that back — so if you want it, write the line yourself and
approve it.

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

**It is approved like any other code your repo runs.** `shall lock` approves a script at its
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

Delete the line and Shall runs `tpm2_clear`, then forgets the script. Without an `@undo=`,
removing the line just drops the record — Shall cannot invent an inverse for a script, and
`plan` says so in those words rather than implying a revert that will not happen.

**A `when` going false is not a removal.** The line is still in your file, so nothing is undone;
that is what stops the enrol script un-enrolling itself on the very next sync. Only deleting the
line from the file counts, and deactivating a profile does not.

**One limit, stated rather than implied:** the hash covers the file Shall executes. A script that
sources another file, or curls one, changes behaviour without changing its hash, and Shall cannot
see that.

## The firewall

One line opens a port, and means the same thing on every machine:

```
firewall:22/tcp
firewall:default/incoming @value=deny
```

Shall drives whichever firewall the machine runs — `ufw`, `firewalld` or Windows Defender —
so the same config opens port 22 on a Debian laptop and a Windows workstation. A firewall Shall
does not know is a `[[firewall]]` row in `adapters/firewall.toml`, not a new release.

**A declared port is open; deleting the line closes it.** That is what declaring means, so
`firewall:22/tcp` takes no `@value=`. Only `default/incoming` and `default/outgoing` do.

**Shall will not close the port you are connected over.** Before any command runs, it checks
whether the change would cut the session it is being typed into — including the subtle case
where tightening `default/incoming` closes your port without ever naming it:

```
refusing to apply the firewall change: it would close port 22, which is carrying this session.
  Shall is being run over that port, so applying this would end the connection and leave no way
  back in.
  Declare `firewall:22/tcp` to keep it open, or make this change from the machine's own console.
```

That check runs on every path that can close a port — including an unattended `watch` tick,
which is the dangerous one, because nobody is there to read a refusal.

**Drift is corrected.** A rule someone added by hand is removed on the next sync, like any other
drift — with the one exception above. If your firewall is also configured by a `link:`ed ruleset
file, Shall warns that two things own the perimeter and lets your declared rules win.

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
3 destination(s) already hold a file Shall did not put there:
    /home/me/.bashrc
    ...
Nothing has been written. Move or delete them, or re-run with `--replace-existing`.
```

On a fresh machine those are usually untouched distribution defaults, which is what
`--replace-existing` is for. It is a per-run flag and deliberately not a config key: a machine
that always bypasses the check is one where the check does not exist.

**`--replace-existing` waives the stop, never the backup.** A tree is the `link:` lines it
stands for, so every file it replaces is preserved as `<destination>.shall-backup` first, the
same as a hand-written line. Delete a file from the tree and its link goes and your original
comes back; delete the `dotfiles:` line and that happens for the whole tree. The removals are
counted against the same ceiling every other removal is.

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
  identity = ~/.config/shall/age.key
}
```

`decrypt` takes `age` or `sops` — nothing else, and any other name is an error listing both.
Shall does not implement encryption; it runs the tool you already trust and writes what comes
back, byte for byte.

**The identity** is `@identity=` if you set it, else `$SHALL_AGE_IDENTITY`, else
`~/.config/shall/age.key`. `sops` reads its own configuration and ignores this.

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
- **No backup is taken for a secret.** For ordinary managed files Shall keeps your original as
  `<target>.shall-backup`; for a decrypted one it does not, because that copy would be the
  previous secret sitting in plaintext beside the new one.

Your `identity` key itself is never managed by Shall and never belongs in the repo. Shall's own
credentials work the other way round and are never files at all: `GITHUB_TOKEN` is read from the
environment, so a Shall config is always safe to hand to someone.

## History and rollback

With `shall git init`, your config directory is version-controlled and every sync commits.
History is git — there is no second generation store.

```bash
shall history            # browse commits, see what each changed, roll back from inside
shall diff HEAD~3        # what changed, in packages rather than text
shall rollback HEAD~3    # restore those manifests, then converge the machine to match
shall snapshot restore   # interactive snapshot gallery (btrfs / ZFS / Timeshift / Windows)
```

`rollback` refuses to apply unconfirmed in a non-interactive shell; pass `--yes` for CI.

Commits are made **as you** — Shall sets no git identity of its own and forces no signing flag,
so `commit.gpgsign` decides whether your history is signed. `shall git log` and `shall history`
show what git says about each commit's signature, and a signature git will not vouch for (an
untrusted, expired or revoked key) is never shown as a good one. Set `require_signed_history`
under `[guard]` to refuse a rollback to any commit git does not vouch for; it is off by default,
because a fresh repo signs nothing.

## Commands

### One command looks, one command acts

`shall check` answers every "what is going on" question — drift, unmanaged software, `absent:`
lines in force, conflicting declarations, backend health, known advisories, and whether any hook
you wrote is unapproved and so will silently never run. With no argument it
prints a line per section and names the command that acts on each:

```
ok  config      42 package(s) declared
->  drift       3 to install, 1 to remove
                   run `shall sync`
->  unmanaged   103 package(s) Shall does not manage
                   run `shall adopt`
ok  health      26 backend(s) ready
```

`shall check health` (or `drift`, `unmanaged`, `absent`, `conflicts`, `config`, `security`,
`approvals`) prints that section in full.

**`check` never changes anything.** What used to be `doctor --fix` — creating missing
directories, reconciling the lockfile, refreshing a stale backend index — is `shall heal`, along
with recovering an interrupted run. A command that both diagnoses and repairs is one you cannot
run to find out whether you want a repair.

Run `shall --help` for the full list with current wording, and `shall check health` for what this
machine actually supports — that is generated from the registry, so it cannot go stale the way
a number typed into a README does.

**Everyday**

| | |
|---|---|
| `sync` | Install, remove and update until the machine matches your files |
| `check` | Read-only: drift, unmanaged software, backend health — what needs you |
| `install` / `uninstall` | Edit the file and sync |
| `list` / `search` / `info` | What is installed, what exists, what a package is |
| `update` / `upgrade` | Refresh metadata; upgrade managed packages |
| `hold` / `unhold` | Stop a package from being upgraded |
| `rebuild` | Remove and reinstall what is declared, to repair what `sync` cannot see |

**Understanding the machine**

| | |
|---|---|
| `why` | Why a package is installed: where it is declared and what depends on it |
| `check config` | Parse everything the active profiles reach; report errors, change nothing |
| `check unmanaged` | What `shall adopt` would take: installed, you chose it, nothing declares it |
| `check absent` | Every `absent:` rule in force, and which module it comes from |
| `check conflicts` | The same tool pinned to different versions by different backends |
| `check health` | Per-backend readiness. It only reports — `shall heal` is what repairs |
| `path` | Print your config repo directory, so `cd $(shall path)` works. `--explain` says what decided it; `--set DIR` stores it |
| `edit` | Open the repo, or one file in it, in `$VISUAL`/`$EDITOR` |

**Cleaning up**

| | |
|---|---|
| `adopt` | Write the packages you installed by hand into a module |
| `add` | Vendor someone else's modules into your repo from `github:owner/repo`, a git/file URL, or a path. Their code arrives unapproved until `shall lock` |
| `unmanage` | Stop managing a package **without** uninstalling it |
| `remove-orphans` | Remove what each manager considers orphaned — shows the list and asks first |
| `clean-cache` | Delete downloaded archives and caches; removes no installed package |
| `purge-undeclared` | Delete every installed package nothing declares — a wider set than `unmanaged`, because it includes the dependency closure. Shows the whole list first |

**Plan, lock, reproduce**

| | |
|---|---|
| `plan` / `apply` | Freeze what `sync` would do to a file, review it, then apply exactly that |
| `eval` | Print the resolved config as versioned JSON — every `when` decided, every bare name given a backend. Takes no locks |
| `try` | Rehearse this config on a clean machine in a container. Answers what `plan` cannot: would it work somewhere that is not here? |
| `lock` | Record every managed package's version so `sync --locked` reproduces it elsewhere |
| `export` | Emit native manifests (Brewfile, requirements.txt, package.json, Aptfile) |
| `bundle` | An offline/air-gapped bundle of config, lockfile and resolved package list |
| `sbom` / `check security` | CycloneDX bill of materials; scan managed packages against OSV.dev |

**Running things**

| | |
|---|---|
| `shell` | An ephemeral shell with specific packages loaded, cleaned up on exit |
| `run` | One command in a throwaway environment |
| `watch` | Reconcile continuously (GitOps for one machine); unattended, applies without prompting |
| `schedule` | Native scheduled tasks (systemd, launchd, Task Scheduler) |
| `fleet` | Compare machines over SSH against your manifests and report drift |

`export` never silently overwrites: if `package.json` already exists, the export is written
beside it as `package.shall.json` and says so. `--force` overwrites deliberately.

### When `sync` says "nothing to do" and something is still broken

`sync` applies the *difference* between your files and the machine. A package that is declared
and installed but broken — a half-configured install, an interrupted download, a closure
something else removed — produces no difference, so `sync` will report success over it forever.

`shall rebuild` stops asking what changed and asserts the declared set from scratch:

```
shall rebuild fd ripgrep       one or more packages (cargo:fd picks a backend)
shall rebuild --backend cargo  everything that backend declares
shall rebuild --all            every declared package on this machine
```

There is no default scope — it removes software in order to put it back, so it makes you say
what. It works **one backend at a time**: all of that backend's packages come down together
(which is what actually lets a shared dependency become an orphan and get collected), then all
of them go back up, then the next backend. Backends that need root go first, because a crate can
need a system compiler and no system package has ever needed a crate.

It never touches undeclared software, and it never removes a protected package — those are
named and skipped rather than rebuilt. It cannot be put in `schedules`.

## Safety

- **Atomic transactions.** A write-ahead log records every mutation that cannot be recomputed —
  every package, every `exec:` script, every `@undo=` — before it runs, **whichever command
  issues it.** `sync`, `apply`, `upgrade`, `remove-orphans`, `purge-undeclared`, an expiring
  lease, a `shell` restore: they all write the record first, because being killed part-way
  through does not care which verb you typed. If Shall is killed mid-command, the next run
  heals it: packages are replayed or reverted, and an interrupted
  script is reported by name, because a half-run script has no recorded progress and re-running
  it would repeat the half that already ran. A crash that goes unattended for hours is still
  healable. Resources declared as an end state — a `service:`, a `setting:`, a `firewall:` rule,
  a placed `link:` — are not logged and do not need to be: the next sync reads the machine and
  finishes the job, which also corrects drift no log would have seen.
- **Snapshots.** btrfs, ZFS, Timeshift and Windows Restore Points, taken automatically before a
  sync or upgrade where a provider exists.
- **Dry run.** `shall --dry-run sync` previews without touching anything — and so does every
  other command, because the flag is honoured by the single function every file Shall owns is
  written through, not by each command remembering to ask. That includes `data/registry.json`,
  the record of what Shall manages: a preview that quietly recorded it would leave your packages
  managed and undeclared, which is the state the next `sync` reads as *remove all of these*.
- **Non-interactive refusals.** `sync`, `rollback` and `remove-orphans` refuse to apply
  unconfirmed changes in a pipe, cron job or CI run without `--yes`.

> **Filesystem-level rollback is Linux-first.** The pre-sync snapshot, `rebuild`'s revert and
> `rollback`'s safety net all depend on a snapshot provider — btrfs, ZFS or Timeshift on Linux,
> Windows System Restore on Windows. **macOS has no adapted provider yet**, so on macOS those
> commands run without a filesystem restore point: the git history still records every change
> and `shall rollback <commit>` still re-syncs packages, but there is no block-level undo. A
> health check that would revert (`@health=`) is *refused before the change* on a machine with
> no provider rather than run without a way back (see above), so this never fails silently.
- **Hooks are locked.** `after_install` and friends are hashed; a changed hook must be
  re-approved with `shall lock`, so a pulled config cannot quietly start running new code.
  - **A hook's first line picks its language.** A shebang runs it as a process in whatever it
    names; `#rhai` runs it in-process with Shall's own script library (`sh`, `read_file`, `env`,
    `http_get`, `parse_json`, the clock); anything else is Lua. All three are handed `PKG_NAME`,
    `HOOK_TYPE`, `OS` and `ARCH` — as `SHALL_`-prefixed environment variables for a process —
    and all three go through the same approval.
  - **A shebang works on Windows too.** Shall reads that first line itself rather than handing
    the file to the OS, which has no shebang mechanism of its own — so `#!/usr/bin/env python3`
    runs your Python on Linux, macOS and Windows alike. `python3` is looked up as `python` and
    `py` as well, because that is what a Windows install is usually called. An interpreter the
    machine does not have is refused by name, so a `#!/bin/bash` hook on a box without bash says
    which program is missing instead of failing as if the script were broken.
    ```toml
    [hooks.after_install]
    docker = '''
    #rhai
    sh("systemctl enable docker");
    '''
    ```
- **Hooks on Shall's own events.** Put a script at `hooks/after_sync`, `hooks/on_drift` or
  `hooks/on_guard_refusal` and it runs with the details on stdin as JSON — notify a channel,
  push the repo, open a ticket, without any of that having to become a Shall feature. The same
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
    # a committed file is a leaked secret (secrets are environment-only in Shall).
    payload=$(cat)
    curl -sf -X POST -H 'Content-type: application/json' \
      --data "$(printf '{"text":"Shall on %s: %s"}' "$(hostname)" "$payload")" \
      "$SHALL_SLACK_WEBHOOK"
    ```
    Approve it once with `shall lock` (it runs code, so the ledger gates it), and swap the
    `curl` line for ntfy, a Telegram bot URL, or a PagerDuty event — the mechanism is the same.
    The built-in `desktop`/`email` channels stay for the common case; everything else is this.
- **Health checks revert.** `apt:nginx@health=port:80` on a line, or a machine-wide
  `health = [...]` in `preferences.toml`. A failing check restores the snapshot the sync took
  before it started — and a health check declared on a machine with no snapshot provider is
  refused *before* the change, because telling you the machine broke without being able to put
  it back is worse than not checking.

### What has been driven, and what has only been argv-checked

Shall ships 52 backends. That is a count of the managers it knows how to drive, and it is not a
claim that every one of them has been driven — so here is the difference, taken from the
harnesses' own tables rather than from anybody's memory.

**Most of them get a real install → list → binary-on-PATH → remove round trip**, on every
nightly, against the actual manager: apt, dnf, pacman, apk, zypper, xbps and brew on their own
container image or runner, and cargo, npm, pnpm, yarn, bun, pip, pipx, uv, gem, go, composer,
opam, cabal, conda, mix, nix, spack, luarocks, nimble, helm, krew, pixi, dotnet, pub, mise,
scoop, winget, choco, github and web on the images that carry them. btrfs, LVM and ZFS run
against real loopback block devices on a privileged image.

**These are argv-tested only** — Shall builds the command line and a test asserts it is the
right one, and no machine in this project's CI has ever run it:

| backend | why nothing has driven it |
|---|---|
| `flatpak` | needs a session bus; the container matrix has none |
| `snap` | snapd is a systemd daemon, and no image here runs systemd |
| `macports` | a second package tree under `/opt/local`; never attempted on a runner |
| `pkg`, `pkg_add`, `pkgin` | FreeBSD, OpenBSD and pkgsrc — no BSD host exists in this CI |
| `eopkg`, `slackpkg` | Solus and Slackware — no image |
| `guix` | needs its own daemon and store |
| `yay`, `paru` | AUR helpers build from source and refuse to run as root, which every container here is |
| `emerge`, `stack` | reached, but smoke-only: both build the package from source, which is minutes per run for ever |

Two code paths are also unexecuted rather than untested: the `dpkg -i` / `rpm -U` local-file
handoff, and storage removal (`U30`). An argv test proves a command line was constructed
correctly. It does not prove the manager accepts it.

### Exit codes

The same four everywhere, so a script can branch on them:

| code | meaning |
|---|---|
| `0` | converged — what you declared is what is there |
| `1` | failed — something went wrong |
| `2` | differences — a read-only command looked and found work to do |
| `3` | refused — Shall said no, and there is no flag for it |

**`3` covers every refusal, not only the guard's.** Refusing to download over plain HTTP, to
install something with no `@sha256`, to write a secret the filesystem cannot protect, to decrypt
into the git repo, to run an unapproved hook, to overwrite a file Shall did not create, or to
place files outside `$HOME` all return `3` — the same code as refusing to remove too many
packages. Until 2026-07-28 those nine returned `1`, so a script could not tell "I refused" from
"I broke", and the `on_guard_refusal` hook never fired for any of them. Both halves are now
checked by `tests/grader_refusal_exit_code_tests.rs` rather than asserted in a comment.

`2` is why `shall check` in CI tells you a machine has drifted without failing the job the way
an error would, and `3` is distinct from `1` because "I will not do this" is not "this broke".

**`shall plan` exits `2` when the plan it wrote is not empty** (2026-08-13). It answers the same
question `check` answers *and* writes the artifact a script consumes, so a pipeline that branches
on drift will reach for it — and until this it was told `0` every time, including while printing
the work on the line above. `shall list --outdated` deliberately still exits `0`: a listing's
subject is inventory rather than a verdict, and one that failed for having contents would
surprise every script that has ever piped one.

**`shall sync` exits `1` when a declaration could not be acted on** (2026-08-13) — one that names
a manager this machine cannot reach, counted per declaration so a partial skip is caught too. It
used to warn and return `0`, which matters most where nobody is reading the warnings: `sudo`'s
stock `secure_path` hides `~/.cargo/bin`, `~/.bun/bin` and `~/.local/bin`, so an unattended sync
could install nothing and report success. A *removal* the guard declines is not this — that is
the guard working, and it is the ordinary state of every adopted machine.

## Teaching Shall a package manager it has never heard of

If a manager's CLI has plain install/remove/list verbs, Shall can learn it from data — no
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
depends_args     = ["deps"]              # a package's dependencies, for `packages` and `why`
repo_add_args    = ["repo", "add"]       # `repo:` lines
repo_remove_args = ["repo", "rm"]
repo_list_args   = ["repo", "list"]
repo_binary        = "mymgr-sources"     # when sources are edited by another program
repo_list_binary   = "cat"               # …and read by another one again
repo_remove_binary = "rm"                # …and dropped by a third
clean_cache_args   = ["cache", "clean"]  # `shall clean-cache`; absent = it has no cache
clean_cache_binary = "mymgr-gc"          # when a different program empties it
purge_args       = ["rm", "--purge"]     # config-destroying removal
manual = "all_installed"                 # so `adopt` takes what you chose, not deps
[backend.orphan_dry_run]                 # what its autoremove WOULD remove
args = ["autoremove", "--dry-run"]
removes_line_prefix = "Remv"
```

**They live in the repo, so they travel**, which is the point: a definition on one machine
makes every other machine fail on a line it cannot resolve. And because each is a list of
commands your repo can run on any machine that clones it, each is approved the way a hook is —
`shall lock` approves them, and any later edit stops that file loading until you look at the
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

This is for the day a manager changes its CLI and Shall has not caught up yet: you can correct
it on your machines without waiting for a release. It costs two deliberate acts — the
`overrides = true` line, and `shall lock` approving the file — and neither one alone does
anything. Shall says so on every run that loads it, naming the backend and the program it now
runs, and `check health` then reports on *your* definition: if `apt-fast` is not installed, `apt`
is critical, because on this machine that is the truth.

Everything you teach Shall lives in one folder, one file per question:

```
adapters/backends.toml    how to drive a package manager Shall does not ship
adapters/settings.toml    how to read and write a settings store
adapters/bootstrap.toml   how to obtain a manager this machine does not have
adapters/prereq.toml      the setup a manager needs before it can install anything
```

**The last one is for a manager that is installed and still cannot install anything.** `mix`
needs Hex, `asdf` needs the plugin for the tool you named, `opam` needs a switch — each of them
fails every install until one command has been run. Shall ships those three and asks before
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

**Its sibling teaches Shall a settings store.** `setting:` writes desktop configuration that
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

All three commands are required. Shall reads before it writes — that is what makes a setting a
declaration rather than a command that runs on every sync — and `reset` is what removing the
line does. A machine whose store has no row gets an error naming what Shall looked for, never a
key that silently did nothing.

### The eight things you can teach it

Everything above is one of these. A row in one of eight files in your repo teaches Shall
something it does not ship, and a thing you teach it is a full peer of a thing it ships — the
built-in package managers go through the same table your `[[backend]]` row goes through, which
is the only reason that sentence is true rather than aspirational.

| file | row | teaches |
|---|---|---|
| `adapters/backends.toml` | `[[backend]]` | how to drive a package manager |
| `adapters/settings.toml` | `[[setting_store]]` | how to read and write a settings store |
| `adapters/init.toml` | `[[init]]` | how to drive an init system |
| `adapters/firewall.toml` | `[[firewall]]` | how to drive a firewall |
| `adapters/snapshot.toml` | `[[snapshot]]` | how to take and restore a filesystem snapshot |
| `adapters/secret.toml` | `[[secret]]` | how to decrypt a secret |
| `adapters/prereq.toml` | `[[prereq]]` | the setup a manager needs before it can install |
| `adapters/bootstrap.toml` | `[[bootstrap]]` | how to obtain a manager this machine lacks |

**`shall adapters` says what this machine has on each**, and the column that matters is the
last one:

```
SURFACE     ROW                STANDING     ROWS
backends    [[backend]]        in use       2
settings    [[setting_store]]  absent       -
init        [[init]]           absent       -
firewall    [[firewall]]       no rows      -
...
```

`no rows` is the one worth having a command for. A file can be present, approved and perfectly
valid TOML and still be doing nothing: write `[[backends]]` where the reader wants
`[[backend]]` and you have described a table nobody opens — no parse error, no warning, and a
`mymgr:` line that fails much later with a message about an unknown backend. `shall adapters
<surface>` narrows to one, and `--json` is the same answer for a script.

Every one of these files runs on your machine, so every one goes through the approval ledger:
the first `sync` after you write or change one refuses it by name and tells you to run `shall
lock`. That is the same rule hooks and `exec:` scripts follow, and it is why a repo you cloned
cannot teach your machine anything you have not read.

## Configuration

`shall config init` writes a commented `preferences.toml` into your repo; `shall edit
preferences.toml` opens it and re-checks that it still parses when you save. Every key is
optional. Settings cover timeouts, concurrency (`max_parallel`), snapshot retention,
notification channels, and the `[guard]` block that holds the removal rules described above.

**Where your repo lives is not a key in it.** `preferences.toml` sits *inside* the repo, so a
key there could only be read from the directory it was trying to move away from. That one
setting lives in Shall's own settings file, beside the repo rather than in it — set it with
`shall path --set DIR`, override it for one command with `--config-dir`, and ask which of the
four sources won with `shall path --explain`.

## Contributing

`docs/SPEC.md` is the source of truth for design — the map, with the parts themselves under
`docs/spec/`; `docs/spec/decisions.md` is every open question. `CLAUDE.md` is the working
agreement. Verify with `cargo build --all-targets`, `cargo test --no-fail-fast`, `cargo clippy
--all-targets`, `cargo fmt -- --check`.

Then, once per clone:

```sh
git config core.hooksPath .githooks
```

That installs `.githooks/pre-commit`, which refuses a commit `cargo fmt -- --check` would fail.
Formatting is the only CI gate a change containing no logic can break, and it takes about a second
to check; everything slower stays CI's job.

## Licence

Shall is dual-licensed under **MIT** ([`LICENSE-MIT`](LICENSE-MIT)) **or Apache-2.0**
([`LICENSE-APACHE`](LICENSE-APACHE)), at your option — the Rust ecosystem's default pair. MIT is
the shortest permissive licence anyone will actually read; Apache-2.0 carries the explicit patent
grant a company's lawyer looks for. Take whichever you can accept.

Contributions are accepted under the same terms, as `Apache-2.0` §5 states for its half.
