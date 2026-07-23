# Part X — Proposed: rebuild, caches, desktops, backup, and finding your files

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

**Status: MOSTLY BUILT; header corrected 2026-07-22.** Raised 2026-07-19. Four of the six are
built and migrated: **X.1** `rebuild` (Part II as II.11b, V.49), **X.4** `setting:`, **X.6**
`locate`/`path`/`edit`, and **X.3**'s `reset` and `clean-cache`. **X.2** (`clean_cache_on_remove`)
is genuinely **not built** and says so in place — it needs a download-cache layer that does not
exist, and a version of it was built and reverted. **X.5**'s git-optional half is built
(`GitManager::require()`); its backup half is **K9, answered 2026-07-22** — see V.59.

Six independent requests, recorded together because three of them touch the same two files
(`preferences.toml` and the command table) and two of them **contradict a sentence Part II
states as settled** —
X.5 contradicts *"This is a git repo"* (II.1) and X.6 contradicts the idea that the config's
own location could be a config key. Those are marked where they occur. As with Parts VIII and
IX, anything adopted here moves into Part II and owes a Part V entry naming the bug it prevents.

Decisions are numbered `K1…K16` in X.7.

## X.1 Converge, or rebuild

**Today sync only ever converges.** It computes the difference between the declared state and
the machine, and applies that difference: install what is missing, remove what is no longer
declared (II.7, II.8). It never removes something that is *still declared* in order to put it
back.

That is the right default and it stays the default. But convergence cannot fix a class of
problem it is structurally blind to: **state that is wrong while the difference is empty.**

| what happened | what convergence sees | what is actually true |
|---|---|---|
| a package's dependencies were removed by hand | declared, installed → nothing to do | half the closure is missing |
| a backend orphaned files a later version renamed | declared, installed → nothing to do | dead files, wrong version on `PATH` |
| a failed sync left a package half-configured | declared, installed → nothing to do | it does not run |
| a `github:` extraction was interrupted | declared, present → nothing to do | truncated binary |

In every row the machine and the declaration agree at the level LiNix inspects, so no amount
of re-running `sync` changes anything. **The escape hatch is to stop asking what changed and
assert the whole declared set from scratch** — remove the declared packages and install them
again, so whatever the backend does on a fresh install happens, and whatever the previous
install left behind is collected by the removal.

**This is a repair operation, not a sync mode.** Two reasons it must not be a flag on `sync`:

1. **It is destructive on a machine that is fine.** Removing and reinstalling a hundred
   packages takes network and time, and passes every one of them through a removal — the exact
   operation the guard exists to be paranoid about (II.10). A flag on `sync` is one typo away
   from a routine command.
2. **Scheduled sync must never do it.** `schedules` runs sync unattended (II.6). A mode of
   sync is a mode a schedule can reach.

*Proposed:* a separate command, `linix rebuild [PKG… | --backend NAME | --all]`, which
**removes and reinstalls what is declared**, defaulting to a scope the user names rather than
to everything. It is not a new plan model: it produces an ordinary plan (remove N, install N),
prints it, and goes through the ordinary guard and confirmation. What makes it a rebuild is
only that the removals are of things that will be immediately re-declared.

**The order within a rebuild is the whole problem, and it is K1.** Remove-everything-then-
install-everything is what actually forces orphan collection, and it is also the version that
leaves the machine with no shell in the middle. Remove-and-reinstall one package at a time is
safe and collects almost nothing, because a dependency shared with a still-installed package
is never orphaned at any instant. **These are different features wearing one name.**

**Rebuild never touches undeclared software.** Everything it removes, it removes in order to
put back. That is what separates it from `purge-unmanaged` (II.11), and the separation must
hold even when a package would be caught by both.

## X.2 Removing a package should be able to remove its cache

`clean-cache` exists (Phase 5, R19) and clears every backend's cache wholesale. What does not
exist is the narrow case: **`linix uninstall jq` leaves jq's downloaded archives on disk**,
and on the download-shaped backends (`github:`, `web:`, `appimage:`) that is a real file of
real size that nothing will ever collect, because the thing that knew its name was the
declaration you just deleted.

*Proposed:* a preference, off by default, that makes removal also drop the removed package's
cached artifacts.

```toml
# preferences.toml
clean_cache_on_remove = false
```

**Off by default because a cache is a bet that you will want it again**, and uninstalling is
weak evidence you will not — the common shape is uninstall, discover you needed it, reinstall
within the hour. The preference exists for the machines where disk is the scarce thing.

**It applies to every path that removes** (II.10's list: sync, `absent:`, expiry,
`purge-unmanaged`, `uninstall`), not to `uninstall` alone. A setting honoured by one command
is the same failure as a guard on one command.

**Only for backends where "this package's cache" is a question with an answer.** LiNix knows
the artifact it downloaded for `github:`/`web:`/`appimage:` — it is in `locks/` (VIII.2). For
apt or pacman, the per-package cache file is the backend's business, and asking it to drop one
package's entry is a different capability per backend; K4 decides whether that is worth having
or whether the preference is honestly documented as download-backends-only.

**NOT BUILT — blocked on a prerequisite that does not exist (2026-07-20, owner ruling).** The
investigation for this feature found that **no download backend retains a separable cache**, so
there is nothing for the preference to act on:

- `github` and `web` download the archive into a `tempfile::tempdir()` and extract from it; the
  tempdir is dropped during install. What remains on disk *is* the extracted install, which
  ordinary removal already takes. There is no retained archive.
- `appimage` stores the `.AppImage` in its store dir, but that file is **the installed program
  itself** — the PATH symlink points straight at it. It is not a cache alongside the install;
  it is the install. Keeping it after removal would leave the whole program on disk while
  claiming to have uninstalled it, and re-install re-downloads rather than reusing it, so the
  "bet you'll want it again" never pays off.

So `clean_cache_on_remove` as specified assumes a download cache **that LiNix does not keep**.
A version was built (appimage-only, keeping the `.AppImage` by default) and **reverted**,
because a preference that is inert on every backend is the "option nobody reads" failure this
document warns against, and the appimage reading turned uninstall into "leave a ~200 MB file
behind by default". *(Options offered: revert and record the prerequisite; build a real
download cache first; or keep the appimage-only version.)*

**What X.2 actually needs first:** a download cache — a place github/web/appimage retain the
downloaded archive separate from the install, reused on reinstall. Only then does dropping it
on removal mean something. That cache is new work and its own decision; until it exists, the
preference has nothing to honour and is deliberately absent rather than present-and-dead.

## X.3 Starting over

Separate from both of the above: **a way to return the machine to as-if-LiNix-had-never-run**,
for the case where the answer is not "repair this" but "throw it out."

`clean-cache` already clears caches. What "from scratch" additionally means has to be stated,
because it ranges over four increasingly violent things and the phrase does not distinguish
them:

| level | drops | reversible by |
|---|---|---|
| 1 | every backend's cache | re-downloading |
| 2 | + LiNix's own download cache and artifacts | re-downloading |
| 3 | + `registry.json` and `snapshots/` (II.1, the data dir) | **nothing.** LiNix forgets what it owns |
| 4 | + the installed packages themselves | `sync` |

*Proposed:* levels 1 and 2 are `clean-cache --all` — a widening of a command that exists,
carrying no risk beyond bandwidth. **Level 3 is a different command** and must be, because
losing the registry is not a cleanup: it is LiNix forgetting the difference between *software
you declared* and *software that was already there*, which is the one distinction the entire
removal model rests on (II.9, II.11). After a level-3 reset every managed package looks
unmanaged, and the recovery is `linix adopt` guessing.

Level 4 is not proposed. It is `purge-unmanaged` with different marketing.

**A level-3 reset must print what the machine will look like afterwards** — *"LiNix will forget
it manages 214 packages. They stay installed. `linix adopt` is how you get them back, and it
will guess."* — and take the typed confirmation `purge-unmanaged` takes (II.10). K5 decides
whether it may run at all while a config repo exists, or only after the repo is gone.

**BUILT, 2026-07-20.**

- **Level 2 is `clean-cache --all`**, and it clears LiNix's own transient download area
  (`tmp_dir`) on top of each backend's cache. It **deliberately does not touch the installed
  artifact directories** (`github_dir`/`web_dir`/`appimage_dir`): those hold software that is
  on `PATH`, and deleting them is a removal (level 4), not a cache clean. The table's phrase
  "and artifacts" is narrowed here on purpose, and the reason is written into the command.
- **Level 3 is `linix reset`** — a separate command, as X.3 requires. It deletes `registry.json`
  and `snapshots/`, prints the "LiNix will forget it manages N packages" notice, and takes the
  same typed-the-count confirmation `purge-unmanaged` uses.
- **K5 ruled: it refuses while a config repo exists unless `--force`**, because forgetting the
  registry while the declarations remain leaves LiNix believing it manages nothing and the
  files saying otherwise. The refusal names the repo and says how to proceed. *(This is the
  recommendation in K5, adopted.)*

## X.4 Configuring a desktop environment

**Most of this already works, and the part that does not is not the packages.** Recorded here
because the request was to check, and the answer is worth writing down.

Installing a DE or WM is packages, and packages are the thing LiNix does. Its config files are
`link:` (II.2), which is what `link:` is for — `~/.config/i3/config`, `~/.config/sway/config`,
`~/.config/hypr/hyprland.conf` are files, and a file with a `when` around it is a per-machine
desktop config with no new mechanism at all. Its daemons are `service:`. **A tiling WM is
already fully declarable today**, because a tiling WM is a package, a config file, and a
session — three things that already exist.

Two gaps, and they are different sizes:

1. **A DE is a package *set*, not a package**, and the set has a different name on every
   distro — `kde-plasma-desktop` on Debian, `plasma-meta` on Arch, `@kde-desktop` as a dnf
   group. This is not a desktop problem; it is the same problem as any package whose name
   varies, and the honest answer today is a `when family` block listing each. Whether LiNix
   should know group syntax per backend is K6.
2. **The settings-store desktops are not files.** GNOME and KDE keep their configuration in
   dconf and in kconfig's own format, so `link:` cannot express *"tap-to-click on, dark theme,
   these six keybindings"* — the state lives in a binary database written through
   `gsettings`/`dconf`/`kwriteconfig`. **This is the actual gap**, and it is a new statement
   shape: a key-value setting is not a package, and `service:`/`link:`/`shim:` do not fit it.

*Proposed (owner ruling, 2026-07-19): `setting:` is in scope.* **BUILT, 2026-07-20.** A fourth
extra-statement kind alongside `service:`, `link:` and `shim:`, applied in the same
after-packages phase, with a per-desktop adapter behind it.

```
setting:org.gnome.desktop.peripherals.touchpad/tap-to-click @value=true
setting:org.gnome.desktop.interface/color-scheme          @value=prefer-dark
```

**It is a declaration like any other, so it inherits the model rather than extending it:**
`when` wraps it, removing the line reverts the setting, two active declarations of the same key
disagreeing is an error (II.7 rule 5), and `plan` shows it before it happens. Nothing here is a
new rule — which is the test the statement had to pass to be worth adding.

**The adapter is the work, and it is per-desktop, not per-distro.** `gsettings` covers GNOME's
schema-backed keys, `kwriteconfig` covers KDE's ini files, and neither covers the other. K7 is
now scope-of-adapters, not whether-to-build.

**Read-before-write is what makes it declarative rather than a hook.** A `setting:` that shells
out unconditionally is a command that runs every sync; a `setting:` that reads the current value
first and writes only on a difference is a declaration, and only the second belongs in this
model.

**Built as `src/backends/setting.rs`, on the `service:`/`link:` pattern exactly:** pure
`read_command`/`write_command`/`reset_command`/`already_set` functions (unit-tested with no
desktop), a `detect_store` that finds `gsettings`, and an `Installable` that reads before it
writes. It is a dependent extra, applied after packages, and its removal runs through the same
`extras_lock` drift path `service:` uses.

**Removal resets to the schema default (owner ruling, 2026-07-20), not to the prior value.**
`gsettings reset` drops LiNix's value and lets the desktop's own default apply. There is no
per-key store of prior values to keep, and "undeclared means the desktop's own default" is the
shape every other statement's removal already has. *(Options offered: record and restore the
prior value, reset to the schema default, or leave the value and only drop the record.)* The
one cost, recorded rather than hidden: a key you had customised by hand *before* adopting LiNix
returns to the schema default, not to your hand-set value, when the line leaves.

**KDE is not adapted yet (K7).** `gsettings` reads a current value cleanly; `kwriteconfig` over
KDE's schemaless ini files does not, and read-before-write is the whole mechanism. A desktop
with no adapter is an error naming the gap, never a silent write — `setting:` refuses rather
than applying something the desktop does not read.

**No `de:` or `wm:` statement.** A desktop is not a backend. It is packages plus files plus a
session, all of which have statements already, and inventing a fourth spelling of the same
three things is precisely the "two of everything" failure the rewrite exists to end.

## X.5 Backup, and working without git

Two requests, one section, because the second is why the first matters.

**LiNix must run without git, and git is not a dependency** (owner ruling, 2026-07-19). II.1
says *"This is a git repo"* flatly; the ruling is that **git buys rollback and history, and
nothing else**. Everything else — parsing, resolution, sync, the guard, locks, schedules —
reads files off a disk and does not care whether a `.git` exists beside them.

**Largely already true, and closed 2026-07-20 — with one sentence of it wrong, corrected
2026-07-22.** The audit found the core paths already degrade rather than fail:
`git_autocommit` no-ops without a repo, and K8's standing notice in `doctor` was built. But
*"`is_repo()` guards every history command"* was **false in the way that matters**:
`is_repo()` tests whether `.git` exists, which is a question about the directory, not about
git. Only `init` ever asked whether the binary was there. So on a machine with no git,
`linix git log` printed an empty history — the answer of a repo with no commits yet — and
`linix git status` advised running `linix git init`, which could only refuse. Found by the
gentoo image, whose stage3 base ships no git; unreachable on every other image, all of
which have one.

**Degrading is not answering.** `GitManager::require()` is now asked by every history verb
(`init`, `log`, `pull`, `checkout_files`, `signature_of`, and the whole `linix git`
dispatch), and it names git as what is missing. That is the II.1 amendment below being kept:
history is unavailable **and LiNix says so**. What X.5 forbids is a *non-history* command
failing for want of git, and none does — `head()` and `show_at_head()` stay best-effort
because they feed baselines, not answers.

No path requires the git binary to be installed; a machine with no git is supported. **K9 is
answered (2026-07-22): the backup command is `bundle`, finished with a `restore DIR` that is a
command rather than a `RESTORE.md`** — not a second archive writer, which is the constraint this
section set. Until that half exists, the git-less case this section calls supported has no
tested way to get a config off a machine at all: git carries history, and `bundle` was carrying
the rest one direction only. See **V.59** and **II.8**.

**Not a dependency means not a dependency**: no git binary required to install LiNix, no git
call on a path that is not history, and no command that fails because git is absent rather than
degrading. A machine with no git installed at all is a supported machine.

> **This changes a Part II sentence.** II.1's *"This is a git repo"* becomes *"This should be a
> git repo; without one, history and rollback are unavailable and LiNix says so."* Per
> CLAUDE.md, adopting it requires reading the matching Part V entry first, and it owes one of
> its own.

What goes missing without git is exactly the Part-IV list and no more: generations are commits,
so there are none; `rollback` has nothing to check out; `linix diff COMMIT COMMIT` has no
arguments to take; `bundle`'s history half is empty (it already reports this honestly — Phase
4). **LiNix must degrade by saying so, once, plainly** — *"No git repo here, so there is no
history to roll back to. `git init` in this folder turns it on."* — not by failing, and not by
silently doing less. K8 decides where that notice lives.

**Backup, given all that.** With git and a remote, a backup command is `git push` with extra
steps. Without git it is the only thing standing between the user and losing the config — which
is why it moves from optional to necessary the moment the line above is adopted. `bundle`
already copies the config root, artifacts and the registry (Phase 4).

**No implementation is proposed here yet (owner, 2026-07-19).** What is recorded is the
requirement — *the config must be recoverable on a machine with no git* — and one constraint on
whatever satisfies it: **not a second archive writer.** Two of everything is how this repo got
into trouble; if `bundle` cannot serve the case, the fix is to change `bundle`. K9 stays open.

**Whatever it is called, it never writes secrets.** Secrets are environment-only (II.1), so
there are none in the config to catch, and that invariant is what makes a backup safe to hand
to someone. It holds only as long as II.1 holds.

## X.6 Finding your files

`linix edit` — open the config repo, or one file in it, in `$EDITOR`; `linix path` — print the
directory, so `cd $(linix path)` works and scripts can use it. Small, and the reason it is
worth a section is that **the alternative is every user memorising `~/.config/linix` and every
script hard-coding it**, which is how a configurable path stops being configurable in practice.
K10 decides the exact spelling.

**Where that directory lives is set in LiNix's own settings** (owner ruling, 2026-07-19) — and
that is a different file from anything in the repo, which is the distinction that makes it work.

| file | lives | holds | in git |
|---|---|---|---|
| LiNix's settings | a fixed OS location (`$LINIX_DATA_DIR` or the platform config dir) | **where your repo is**, and nothing else it can help | no |
| `preferences.toml` | inside your repo | refusals and behaviour (II.1) | yes |

**The ordering resolves because the two files answer different questions.** A key inside the
repo saying where the repo is would have to be read out of the file whose location it defines —
no ordering resolves that. A key in a fixed-location settings file saying where the repo is
resolves in one step: LiNix reads its own settings from a place it always knows, learns the repo
path, and everything after that is the model as written. **Nothing about resolution, `when`, or
II.1's "detected, never configured" changes**, because the repo path is not a fact about the
machine — it is where you put your files, which only you know.

**Precedence:** `--config-dir` flag → `$LINIX_CONFIG_DIR` → the settings file → `~/.config/linix`.
Command-line beats environment beats stored beats default, which is the ordinary shape and needs
no argument.

*Proposed:* `linix path` prints the resolved directory *and which of those four set it*, so a
wrong answer is debuggable in one command; `doctor` reports the same; `linix edit` opens it. K11
covers whether the settings file is allowed to hold anything else — **the answer should be no,
and the reason to write it down is that a file holding one key is exactly the file that grows a
second one**, at which point there are two preference systems and the question of which wins.
K12 asks whether a symlink at the default path also stays supported for the dotfiles-repo case;
it costs LiNix nothing, because a symlink is the operating system already solving this.


---

**Decisions: K1–K16.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
