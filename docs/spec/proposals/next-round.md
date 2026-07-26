# Part XIII — Proposed: the next round (2026-07-23)

*[LiNix v7](../../SPEC.md) — the map is there; this is one part of it.*

Seven proposals from one conversation. They are one part rather than seven because they share a
question — *what is LiNix allowed to do without being told twice* — and because five of them
turn out to be **compositions of things already built**, which is the cheapest kind of feature
and the easiest to miss while reading the tree file by file.

Decisions in this part are numbered **U**.

## XIII.1 The kernel, answered: LiNix already upgrades it, and should finish the job

**Does LiNix upgrade the kernel? Yes — incidentally, and with no special knowledge of what a
kernel is.** `linux-image-generic`, `linux`, `linux-lts`, `kernel-core` are packages, and
`upgrade` upgrades them through the backend like anything else. There is no kernel concept
anywhere in the tree. The single mention is `config/config.rs:380`, where `linux-image`,
`linux-headers` and `kernel` sit in the guard's protected-prefix list so that no removal path
can ever take them — a refusal, not an understanding.

**So why not `nvidia` and the like? Mostly the distribution already does it, and on a
single-manager machine there is nothing for LiNix to add.** An out-of-tree module —
`nvidia-dkms`, `virtualbox-modules`, `zfs-dkms` — is also just a package. Declaring
`pacman:nvidia-dkms` installs it, and the rebuild against a newly-installed kernel is done by
that package manager's own DKMS hook, at the moment the kernel lands.

**The hole is the machine LiNix exists to make possible.** The whole premise is that software
comes from several managers at once. The kernel comes from `apt`; the driver may come from
`github:` or `web:` as a blob, or from another backend entirely. **No package manager's hook
fires for a module a different manager installed** — so the rebuild a single-manager machine
gets for free silently does not happen, and the symptom is a black screen after the next
reboot rather than an error during the sync that caused it.

**LiNix is the only thing on the machine that can see both halves.** It knows a kernel package
changed, because it changed it; it knows which out-of-tree modules are declared, because they
are in the config.

*Proposed:* after a sync that changed a kernel package, **rebuild the declared out-of-tree
modules, and fail loudly before the reboot** — while there is still a working shell to fix it
from. Constraints:

- **Declared modules only.** A module LiNix did not install is not LiNix's to rebuild.
- **It rebuilds; it does not advise (P8).** `dkms autoinstall`, or `dkms build`/`install` per
  module where the version is known.
- **A module that will not build is a loud failure naming the module, the kernel version and
  the log path** — the one moment where saying nothing costs the user their display.
- **P7:** Windows and macOS have no DKMS. Windows' nearest analogue is a driver package bound
  to an OS build, and the honest answer may be *nothing to do here* — but under P7 that answer
  gets written down after someone looks, not assumed because the feature smelled like Linux.

**Withdrawn: the `hardware` command that printed suggestions (owner ruling, 2026-07-23).** It
would have detected an undeclared GPU or fingerprint reader and printed lines for the user to
paste into a module. **That is precisely what P8 forbids** — LiNix does the thing, it does not
hand you the thing to do. If hardware detection is ever worth it, its output is an edit LiNix
offers to make (the `install`/`adopt` shape: ask, then do), never a block of text to retype.

## XIII.2 Custom backends already exist — and they live in the wrong place

**`src/backends/onboarder.rs` (593 lines, shipped) already lets a user teach LiNix a package
manager with no source change.** `~/.config/linix/custom_backends.toml`:

```toml
[[backend]]
name         = "paru"
install_args = ["-S", "--noconfirm"]
remove_args  = ["-R", "--noconfirm"]
list_args    = ["-Qm"]
search_args  = ["-Ss"]
needs_root   = false
[backend.parser]
format      = "columns"
name_col    = 0
version_col = 1
```

Both halves of a backend are data: the argv come from the TOML, and the output parser is a
declarative `ParserSpec` (`lines` / `columns` / `json` / `regex`) interpreted at runtime by
`ConfiguredParser`. `paru:yay-bin` then works everywhere a built-in prefix works. Custom
backends register **last** and never override a built-in (collisions are skipped with a
warning), and `is_valid_backend_name` refuses any name the grammar spends — no whitespace, no
`/`, no `:`, no `,`, nothing in `RESERVED_BACKEND_NAMES`.

**This document has mentioned it twice, in passing, and never described it.** That is the first
defect: a capability nobody knows about is a capability nobody uses, and the next session
reimplements it.

**The second defect is where the file lives, and it is the real one.** `custom_backends.toml`
is read from `safe_config_dir()` — LiNix's own settings directory, the machine-local one that
II.1 says is *never in git*. So:

> A config repo that declares `paru:yay-bin` works on the machine where somebody once hand-wrote
> a TOML file, and fails on every other machine with an unknown-backend error — including the
> fresh machine the repo exists to set up.

**The one thing that cannot travel is the thing that teaches LiNix what the lines mean.** That
contradicts the model's central claim (one repo describes the machine) more sharply than any
missing backend does. See **U1**.

**Known capability gaps** (a custom backend is not yet a peer of a built-in): no repository
management, no orphan/enumerate support, no dependency query, no `is_essential`. Each is a
field `ManagerConfig` already has and `CustomBackendDef` does not expose. Filling them is
mechanical; deciding whether a custom backend *should* be a full peer is **U2**.

## XIII.3 Running a script, and naming a command — what exists, and the one shape that does not

**Asked 2026-07-23: can a user point LiNix at a file and have it run, or alias a name to a
command? Four ways already exist, and each is deliberately attached to something.**

| you want | write | what it is |
|---|---|---|
| a script around a package | `apt:nginx@after_install=./setup.sh` | II.12 hooks — `before_`/`after_install`, `after_remove` |
| a script on a clock | a line in `schedules`, `@cron=`/`@run=` | the scheduler |
| a name that resolves to a binary | `shim:node` | `ShimManager` — a PATH stand-in |
| a name that resolves to argv | `[[backend]]` in `custom_backends.toml` | XIII.2 — a whole manager, from data |

**What does not exist is a bare script with nothing to attach it to** — the NixOS
`activationScripts` shape, "run this as part of making the machine right".

**And the reason it does not exist is not an oversight.** A statement that just runs a command
has no state to converge to, so it either runs on every sync — which makes it a cron job with
worse ergonomics, and makes `sync` non-idempotent, the one property the whole model rests on —
or it runs once and never again, which makes it an install script with no record. Neither is a
declaration. `setting:`'s ruling (X.4) is the precedent: *a `setting:` that shells out
unconditionally is a command that runs every sync; one that reads the current value first and
writes only on a difference is a declaration, and only the second belongs in this model.*

**`exec:` is approved (owner, 2026-07-23), with two rulings that between them supply the
missing state.**

### The condition is `when`. There is no `@unless=` (owner ruling, 2026-07-23)

The first draft of this section proposed `@unless=<command>` and `@creates=<path>` as new option
keys. **Ruled against, and they are not to be built** — the condition system already exists and
adding a second one is the failure this rewrite is for.

`when` gates every line in every file, and Part IX made the variables it reads
*user-programmable*: a provider file (`vars.py`, `vars.sh`, `vars.linix`, or the plain `vars`
line file) is run by LiNix, is handed the machine's facts as `LINIX_OS`/`LINIX_ARCH`/
`LINIX_HOST`/`LINIX_FAMILY`, and returns `name = value` pairs. **So "unless this command
succeeds" is already expressible, with nothing new in the grammar:**

```
# vars.sh — a program, so it may ask the machine anything
tpm_enrolled = $(tpm2_getcap properties-fixed >/dev/null 2>&1 && echo yes || echo no)
```

```
when $tpm_enrolled == no {
  exec:./bin/enroll-tpm.sh
}
```

Three things this buys that a dedicated key would not: the condition is **named**, so the reason
the script runs is legible at the point of declaration; it is **reusable**, because the same
variable can gate a package, a `setting:` and an `exec:` at once; and it is **one mechanism**,
so a user who has learned `when` has learned this. `@creates=<path>` is likewise a variable a
provider computes, not a key.

### The lock is the script's hash and how many times that hash has run (owner ruling, 2026-07-23)

`when` decides *whether the machine wants this*. It cannot decide *whether this already
happened* — variables are resolved once and frozen into the plan (W4/W13), so a condition that
the script itself would have falsified is still true within the run that executes it. That is
what the lock is for.

`locks/exec.toml`, keyed by **the hash of the script's contents**, recording **the number of
times that hash has run** and when it last did.

- **The hash is the identity, not the path.** Editing the script makes it a different script, so
  it runs again. Renaming it does not. That is content-addressing, and it is the same reasoning
  II.12 uses for artifacts: what you declared is the content, and a changed content is a changed
  declaration.
- **The default is: run once per distinct content.** `@runs=` sets the ceiling; `@runs=always`
  is the explicit opt-out for a script that genuinely must run every sync, and being explicit is
  the point — nothing becomes a per-sync command by accident.
- **`plan` can now say the true thing:** *hash `a1b2…`, run 0 times, `$tpm_enrolled` is `no` →
  this will run*. A statement whose preview cannot be computed has no business in this model,
  and this one's can.
- **The limit gets written down, not hidden:** the hash covers the file LiNix executes. A script
  that sources another file, or curls one, changes behaviour without changing its hash, and
  LiNix cannot see that. Say so in the readme rather than implying a guarantee.
- **Removing the line drops the lock row.** What it does *not* do is undo the script's effects —
  see U3.

### Three states, and the one place `exec:` is not like everything else

**Note this, because it is the exception that will be "fixed" by someone who has not read it
(owner, 2026-07-23).** Every other statement has two states that collapse into each other: a
line whose `when` is false is *exactly* a line that is not there. `when $gaming { apt:steam }`
on a machine where `$gaming` is false means Steam is not declared, which means Steam is drift,
which means Steam is removed. **False and absent are the same fact for a noun.**

**For a verb they are not**, and the reason is the example this feature was designed around:

```
when $tpm_enrolled == no {
  exec:./bin/enroll-tpm.sh
}
```

**The script's success flips its own condition false.** If a false `when` meant "removed", and
removed meant "undo", the sync that enrolled the TPM would immediately un-enroll it — and every
sync after would flap. A verb that succeeds makes itself unwanted; that is what success *means*,
and it must not be read as a request to undo.

So `exec:` has three states, not two:

| state | what it means | what happens | lock row |
|---|---|---|---|
| declared, `when` **true** | wanted, and possibly not done yet | runs if this hash's count is below its ceiling | created / incremented |
| declared, `when` **false** | not wanted *on this machine, right now* | **nothing runs, and nothing is undone** | **kept, untouched** |
| **not declared** (line deleted) | no longer part of the configuration | `@undo=` if one was given, else nothing (U3) | dropped |

Two consequences that are easy to get wrong and expensive to debug:

- **The lock row survives a false `when`.** Dropping it would make a condition that flaps — a
  laptop on battery, a host that comes and goes — re-run the script every time the condition
  swung back true, because the count would have been forgotten. Keeping the row is what makes
  the count mean *"this content has run n times on this machine"* rather than *"n times since
  the last time the condition happened to be false"*.
- **`exec:` is not in the extras teardown path.** `reconcile_extras` (S20) undoes a `service:`,
  `link:`, `shim:`, `repo:` or `schedule:` that stopped being declared, and `extra_key` must
  either exclude `exec:` or handle it as its own case. Wiring a verb into a ledger built for
  nouns is how the un-enrol bug gets in through the back door.

Still open: what an `exec:` **removal** should mean (U3), and whether this becomes the way people
avoid writing a backend (U4).

## XIII.4 Parity is a gap list, not a sentiment (P7)

P7 says a feature is unfinished until Windows and macOS have an equivalent or a written reason
there can be none. **Applying the rule to the tree as it stands produces a short, concrete
list** — and one entry that turns out to be fine, which is worth recording so nobody re-audits
it:

| capability | Linux | macOS | Windows | verdict |
|---|---|---|---|---|
| `service:` | systemd, OpenRC, SysVinit | launchd | `sc` | **done** — `InitSystem` covers all five |
| `setting:` | `gsettings` (GNOME) | **nothing** | **nothing** | **the gap** |
| snapshot / rollback safety net | btrfs, ZFS, Timeshift | **nothing** | **nothing** | **the gap** |
| packages | apt, dnf, pacman, … | brew, mas | winget, scoop, choco, psresource | done |

**The Windows registry is the highest-value single adapter in this document.** `setting:`'s
whole mechanism is read-before-write, and the registry answers a typed read-then-write cleanly
— better than KDE's schemaless ini files, which is why the KDE adapter (K7) is still blocked.
`setting:HKCU\Software\...\Key@value=` would make LiNix the only tool that declares a Windows
machine's *configuration* and not merely its software. macOS's counterpart is `defaults
read`/`write`, which has the same shape. See **U5**.

**The snapshot gap is the more serious one, and it is quiet.** Every dangerous path in this
document — `rebuild`'s revert (K3), the guard's pre-sync snapshot, `rollback` — is written as
though a snapshot provider exists. On Windows and macOS none does, so those paths fall back to
their no-provider branch and the safety net silently is not there. Windows has VSS; macOS has
APFS local snapshots. **Until one of them is adapted, this document should say plainly which
guarantees are Linux-only** rather than describing them unqualified. See **U6**.

## XIII.5 Health-checked upgrades, with automatic rollback

**Every part of this is built. What is missing is the wiring, and the wiring is the feature.**

An upgrade today: snapshot (where a provider exists), upgrade, done. Whether the machine still
works is discovered by the human, later, by using it. That is acceptable when a person ran the
command and is watching; it is **not** acceptable for `watch`, the unattended reconcile, which
is exactly where an upgrade that breaks the box does so with nobody present.

*Proposed:* a declared health check, run after the upgrade, whose failure reverts it.

```
apt:postgresql {
  version = 16
  health  = pg_isready -q
}
```

- The check runs after the transaction that touched the package, not on every sync.
- A failing check **restores the snapshot** — the mechanism K3 already ruled for `rebuild`'s
  failed reinstall, reused rather than reinvented.
- **No snapshot provider is not a refusal** (K3's second ruling, again): the check still runs
  and still fails loudly, it simply cannot revert, and it says so before it starts.
- A failed *restore* is its own reported outcome (K3's third ruling). The machine is then both
  broken and un-restored, and saying "rolled back" would be a lie.
- **P7:** the check is a command the user wrote; it is portable if theirs is. Nothing here is
  Linux-shaped.

The scope question is whether health is per-package or per-sync (**U7**).

## XIII.6 The question `why` cannot answer

`linix why <package>` says where a package came from. **The reverse question is the one people
actually hesitate over**, and nothing answers it: *if I deactivate this profile, or delete this
module, what leaves this machine?*

The computation already exists — `deactivate` runs exactly this set-math today, then acts on
it. This proposal is that pressing it is not the only way to see it. *Proposed:* a preview mode
on the existing commands rather than a new verb (`deactivate --dry-run`, `why --if-removed`),
because a new verb for an existing computation is the two-of-everything failure. See **U8**.

## XIII.7 Cross-machine diff

`app/fleet.rs` SSHes to each declared host, runs `linix status --json`, and reports **each
host's drift against its own manifests** — optionally reconciling. It cannot answer the
question two machines actually raise: *the laptop and the desktop run the same profile, so why
does one have ripgrep 14 and the other 13?*

That is the same parse `fleet` already does, on a different axis: compare hosts to each other
rather than each host to itself. Version skew inside one declared set is real drift — the two
machines match their manifests and do not match each other, which is what a version range or a
stale index does — and it is invisible today. Small, given `HostDrift` exists.

## XIII.8 Sixty commands, one question

**`linix --help` lists around sixty subcommands. Ten of them answer a version of "is my machine
all right?"** — `status`, `check`, `doctor`, `heal`, `unmanaged`, `absent`, `conflicts`,
`insight`, `metrics`, `audit`. Nobody can hold that in their head, and the practical result is
that a user runs the one they remember and never learns the other nine exist. **The
consolidation is worth more than any new backend**, and it is the repo's own rule applied to
its command surface: *prefer deleting to fixing; when you find a second implementation of
something, remove one rather than reconcile them.*

**This needs a decision before any work (U9), because it is the one change here that breaks
existing invocations** — and "no change breaks existing code" is binding. Under P2 there are no
users to migrate and no deprecation period, so the honest form is: one command, the old names
gone in the same change, the docs and tests renamed with them.

The shape to aim at: **one `linix check`, sectioned**, with flags to narrow it — drift,
unmanaged, absent, conflicts, health, policy. `heal` stays separate because it *acts*; the
other nine only look.

## XIII.9 The one kind of software LiNix cannot install: its own backends

**Approved 2026-07-23.** Declare `brew:ripgrep` on a fresh Mac with no Homebrew and the line
fails: the backend is unavailable, and LiNix — a program whose entire job is installing software
— cannot install the thing it installs *with*. The same holds for `scoop` on a new Windows box,
`krew` without `kubectl`, `cargo` without a toolchain, `pipx` without Python.

**This is the first ten minutes of every new machine, which is precisely the ten minutes the
tool exists to delete.** A config repo that describes a machine completely still requires a
human to read a wiki and paste an install line before LiNix can act on any of it.

`priority` already names every backend the machine uses, in order, and V.15 already refuses a
line whose backend is not listed — so the file that knows which backends matter is the file that
should know how to obtain one.

*Proposed shape:* an optional bootstrap per backend, used only when the backend is declared,
absent from the machine, and the user has said yes.

- **It is a refusal by default, never a silent fetch.** Installing a package manager is running
  someone's shell script as root; it is II.12's supply-chain surface at its widest. LiNix says
  *`brew` is declared and missing; here is exactly what I would run to get it* and stops. That
  is *ask, then do* (P8), not *inform, then leave* — the difference is that the answer to the
  question performs the work.
- **A bootstrap that is already a package is a package.** `pipx` is `apt:pipx`; `krew` is a
  `github:` release. Those need no new mechanism at all and should not get one — the mechanism
  is for the managers that genuinely ship as a shell script (`brew`, `rustup`, `scoop`).
- **P7:** this is *more* valuable on Windows and macOS than on Linux, where the system manager
  is already present. A fresh Windows box has no scoop and no choco; that is the machine with
  the longest manual prelude today.
- **The bootstrap is recorded like any other install**, so `unmanaged` and the registry do not
  suddenly disagree about where `brew` came from.

Open: does a bootstrap live in `priority` beside the backend it obtains, or in
`custom_backends.toml` beside the definition (U10)?

## XIII.10 `sync --locked` — the difference between describing and reproducing

**Approved 2026-07-23.** `locks/` records what each declaration resolved to. Nothing lets a user
say **converge, and fail if the lock would change.**

That single flag is the line between *"my config describes this machine"* and *"my config
reproduces this machine"*, and the second is the claim people actually adopt a declarative tool
for. Today an unpinned line resolves to whatever the index offers this morning, the lock is
updated to match, and the machine has drifted from its sibling with nothing anywhere reporting
it — every file matches, every check is green.

- **`--locked` fails; it does not fix.** A resolution that differs from the lock is an error
  naming the package, the locked version and the offered one. Exactly `cargo --locked`'s
  contract, and worth matching deliberately, because the audience already knows it.
- **This is what CI runs**, and it is what makes a fleet reproducible rather than merely
  convergent (XIII.7's cross-machine skew is this bug seen from the other end).
- **`watch` should probably default to it** — an unattended reconcile that silently accepts a
  new upstream version is the least supervised place for that to happen (U11).
- Backends that cannot pin a version cannot honour `--locked`, and must say so rather than
  passing. `ManualListing`'s distinction is the precedent: *"cannot answer"* must never be
  reported as *"nothing changed"*.

## XIII.11 `linix try` — rehearse the config before it touches the machine

**Approved 2026-07-23.** Phase 6 built container images to test LiNix. Point that machinery at
**the user's config** and it becomes the feature this tool most obviously lacks: *run my config
in a clean container and tell me whether it converges — before it runs on my laptop.*

`sync` is the scariest command in the product. It installs and removes real software on a real
machine, and the guard, the plan, the snapshot and the confirmation all exist to make that
survivable. **`try` makes it rehearsable, which is strictly better than survivable**, and the
harness for it already exists.

- **A `--dry-run` is not this.** Dry-run predicts from LiNix's model of the world; `try` finds
  out what the package manager actually does — the conflict, the missing dependency, the
  post-install script that fails on a clean box. Every bug Phase 6 found was invisible to
  `cargo test` for exactly this reason (the twelfth session's entry is the evidence).
- **The output is a verdict and a transcript**, not a shell to poke at.
- **P7 makes this bigger than it looks:** `try --on debian` and `try --on windows` is how a user
  finds out that the config they wrote on a laptop works on the desktop they have not booted
  yet — and it is how *this project* enforces P7 on itself instead of asserting it.
- **It needs no daemon and no privilege on the host beyond a container runtime**, and where
  there is none, it refuses and names what is missing rather than falling back to running on the
  real machine. That fallback would be the single most dangerous line of code in the repo.

Open: does `try` reuse the Phase 6 images or build from the config's own declared base (U12)?

## XIII.12 One field away from user-defined nouns

**Approved 2026-07-23.** In `custom_backends.toml`, `name` is both the backend id **and the
binary invoked** — the loader requires them to match. Separate the two and the onboarder stops
being "teach LiNix a package manager" and becomes **"teach LiNix a noun"**:

```toml
[[backend]]
name         = "firewall"      # the prefix a line is written with
binary       = "ufw"           # the program actually run
install_args = ["allow"]
remove_args  = ["delete", "allow"]
list_args    = ["status", "numbered"]
[backend.parser]
format = "regex"
```

`firewall:22/tcp` then resolves, installs, lists and removes **with no Rust at all**.

**This does not replace Part XI, and Part XI stays.** The two answer different questions and the
distinction is the point:

- **A user-defined noun is one person's machine, one firewall, one spelling.** It runs `ufw`
  because that user has `ufw`. It cannot know that `firewalld` and `nft` and Windows Defender
  Firewall are the same idea wearing three commands, which is XI.2's entire justification for a
  built-in backend — *one spelling across five firewalls* — and it is the half a TOML file
  cannot supply.
- **A built-in backend is the portable spelling.** Under P7 that is not a nicety; a config that
  opens port 22 must mean the same thing on the Debian laptop and the Windows workstation, and
  a definition naming `ufw` means nothing on Windows.
- **The escape hatch is what makes the built-in optional rather than urgent.** If N3 comes back
  *"only one adapter is in reach"*, the honest answer stops being "build nothing" and becomes
  "ship the noun mechanism, and let the user who has `ufw` write six lines of TOML today."

**What it needs beyond the field split** is `U2` — the capability gaps that make a custom
backend a peer of a built-in — because a noun that can install and remove but cannot *report
what is currently there* is not declarative, and read-before-write is the line X.4 drew.

**The field split is small and it is not cosmetic.** `is_valid_backend_name` refuses a name the
grammar spends; that check now guards a prefix rather than an executable, and the executable
needs its own validation, because a `binary` from a shared repo is a command from a shared repo
(U1's trust question, arriving a second time by a different door).

## XIII.13 Hooks on LiNix's own events

**Approved 2026-07-23.** Hooks today are per-package: `apt:nginx@after_install=./setup.sh`
(II.12). There is no way to say *"whenever a sync finishes"*, *"whenever drift is found"*,
*"whenever the guard refuses something"*.

That gap is why every integration request becomes a feature request. Notify me on Slack when a
machine drifts; push the repo after every sync; open a ticket when the guard refuses a removal;
run the fleet report nightly — **none of those should be LiNix features, and today each of them
would have to be.**

*Proposed:* a small, closed set of LiNix-level events, declared where the machine's own settings
live rather than in a module, because they describe *this installation's* behaviour rather than
*this machine's* software:

```
# preferences.toml
[events]
after_sync      = ./bin/notify.sh
on_drift        = ./bin/alert.sh
on_guard_refusal = ./bin/ticket.sh
```

- **The set is closed and small**, for VIII.2's reason: a closed vocabulary can name the legal
  values in the error, and an open one cannot.
- **The event is handed its context on stdin as JSON** — what synced, what drifted, what was
  refused — so a hook does not have to re-derive the state LiNix already computed.
- **A failing event hook does not fail the sync**, and never silently: it warns, naming the hook
  and its exit code. An observer that can break the observed is not an observer.
- **`on_guard_refusal` is deliberately included.** The guard's refusals are the most important
  events LiNix produces and today they are visible only to whoever is watching the terminal.
- **Same trust model as II.12's hooks** — this is argv from a file, and the file may travel.

## XIII.14 Sharing — RULED TO NEED A DECISION (U14)

**`use` takes a name, never a path or a URL** (II.2). That is deliberate, and it means there is
**no way to consume anyone else's module or backend definition.** Sharing a working config today
is copy and paste. Emacs has MELPA; LiNix has nothing, and the closer the three extension axes
get to done, the more that absence costs — a mechanism nobody can share definitions through
produces the same six TOML files written a thousand times.

**The version that does not break the rule is vendoring, not importing.** `linix add <git-url>`
copies the module (or backend definition) **into your repo as ordinary files**, once, at the
moment you ask:

- It is still `use <name>`. The grammar does not change and no line ever names a URL.
- The files are **yours** — in your git, in your diff, reviewable before they run, and offline
  forever after.
- There is no fetch at sync time, so no network dependency and no upstream that can change what
  your machine does without a commit of yours.
- Updating is `linix add` again, which shows up as a diff you approve — the `adopt` shape.

**What must be decided (U14), because it is not obvious:** whether this is wanted at all;
whether a vendored module records where it came from (provenance is useful, and it is also a
second place a URL lives); and what stops the first person who publishes a module with an
`exec:` in it from owning every machine that vendors it. **The last is the real question** —
vendoring makes the code visible in a diff, which is a genuine defence, and "visible in a diff"
has never been much of one in practice.

## XIII.15 `linix eval` — hand the resolved state to everything else

**Approved 2026-07-23.** LiNix computes something no other tool on the machine has: the fully
resolved desired state — every module and profile flattened, every `when` decided, every
variable substituted, every bare name resolved to a backend. It is used internally and then
discarded.

*Proposed:* print it, as JSON, on demand.

This is not a feature for LiNix. **It is the feature that stops LiNix needing a new feature
every time someone wants to know something.** Which machines declare `openssl`? Is this package
declared but only under a `when` that is false here? What does the desktop's resolved set have
that the laptop's does not? Each of those is a question someone will eventually ask for a
command, and each of them is `linix eval | jq` if the resolved model is readable from outside.

- **It is read-only and takes no locks.** It resolves and prints; it touches nothing.
- **It prints the *resolved* state, not the files.** Re-parsing the config is what other tools
  do today when they have no choice, and every one of them becomes a second parser (C13's whole
  family of bugs).
- **The shape is versioned**, because the moment anything consumes it, it is an interface.

## XIII.16 Grouped backends and per-group priority — MAYBE (U18)

**Raised 2026-07-23, recorded as a maybe rather than an approval**, because the workaround is
real and the cost is a rule that has never had an exception.

`priority` is **one ordered list for the whole machine**. A bare `ripgrep` walks it and takes the
first backend that has the package. That single ordering has to serve every kind of thing at
once — and the right answer genuinely differs by kind:

- **CLI tools:** `cargo`, then a `github:` release, then the distro — you want the current one.
- **System libraries and daemons:** the distro, and nothing else — you want the one the rest of
  the system was built against.
- **Language runtimes:** `mise` or `asdf` before either.

One list cannot say that. Today you say it by writing the prefix — `cargo:ripgrep` — **and that
works**, which is why this is a maybe. What it costs is the thing a bare name is for: a bare name
resolves per machine, so the same config installs from `apt` on the server and `brew` on the Mac.
Spelling the backend out to get the right *kind* of ordering throws away the portability to buy
the precision.

*Proposed shape:* named groups in the file that already answers "which backends, in what order",
and a module selects one.

```
# priority
group tools  { cargo, github, apt }
group system { apt }
group runtime { mise, asdf }
```

```
# modules/dev.txt
priority tools

ripgrep       # cargo, then github, then apt
fd
```

- **A module is the right scope, not a line.** Per-line selection is exactly writing the prefix,
  which already exists and reads better. A module is already a coherent set of things of one
  kind, which is why the grouping wants to attach there.
- **No group declared means today's behaviour**, the whole list in order. Nothing breaks, and a
  config that never uses this never sees it.
- **A backend may be in several groups**, and one not in `priority` at all is still refused
  (V.15) — groups narrow and reorder, they never admit.

**The rule this has to not break, and the reason it is a maybe.** II.7 rule 5: two active
declarations that disagree are an error naming both. If `ripgrep` in `modules/dev.txt` resolves
through `tools` and `ripgrep` in `modules/server.txt` resolves through `system`, **the same word
means two packages in one config**, and the machine ends up with two `ripgrep` binaries fighting
over `$PATH`. That failure is not hypothetical — `app/conflicts.rs` already detects exactly it
(*"the same tool would be installed by more than one backend"*), which is evidence the model has
met this before.

*Recommendation if it is built:* **a name still resolves once per machine.** Two modules that
would resolve the same bare name through different groups is an error naming both files and both
lines — the existing rule applied to a new way of reaching it, not a new rule. Groups then buy
the ordering without buying the ambiguity.

## XIII.17 User or system — the question 7e asks first (U19, DECISION OWED)

**Recorded 2026-07-23 as a decision to be made, not a proposal.** LiNix installs to *a machine*.
A machine has users, and the model has never said which one it is acting for:

- `apt` is system-wide; `cargo`, `pipx`, `npm -g` and `krew` are per-user by default.
- `gsettings` is per-user. There is no such thing as a system-wide GNOME setting in `setting:`'s
  current adapter.
- `link:@target=~/.npmrc` resolves `~` to *the home of whoever ran the command* — so the same
  config, run once with `sudo` and once without, writes two different files and both are
  "correct".
- `shim:` deploys into `~/.local/bin`.

**Today the answer is implicit and inconsistent: whoever ran the command.** That has survived
because the Linux backends mostly agree with it by accident.

**7e ends that immediately.** The Windows registry's first question is `HKCU` or `HKLM` — there
is no third option and no default that is right for both — and **there is currently nothing in
this document to answer it with.** Whatever the registry adapter picks becomes the convention by
precedent, and then spreads to the macOS `defaults` adapter, which has the same split.

This is why it is filed as a decision owed rather than a feature: **it should be answered before
7e is written, not discovered during it.** The candidate answers are in U19.

## XIII.18 An editor that knows the grammar — MAYBE (U20)

**Recorded as a maybe.** The config language is a real grammar with a real parser: errors name
the file, the line and what was expected, and every vocabulary in it is closed — backends,
option keys per statement kind, `formats`, `when` keys and operators. That is exactly the raw
material a language server consumes, and almost none of it would have to be written twice.

What it would give: completion for backend prefixes and for the option keys legal on *this*
statement kind; hover showing what a bare name resolves to **on this machine**, through this
`priority`; the error at the moment of typing rather than at the moment of syncing. **It turns
the closed vocabulary from a thing you must remember into a thing the editor tells you** —
which is the strongest argument for closed vocabularies and currently the argument nobody gets
to feel.

**Why it is a maybe and not an approval:** it is a second program with its own protocol, its own
release cadence and its own way of going stale, and this repo's history is that a second
implementation of anything eventually disagrees with the first. It is only worth it if the
server is a thin front end over the same parser and the same resolver the binary uses — never a
reimplementation — and `linix eval` (XIII.15) already supplies most of what the interesting half
needs. See U20.

## XIII.19 `git blame` for a declaration

**Approved 2026-07-23.** History is git (II.13), so the data is already there and nothing else
has to be recorded. What is missing is the question, asked package-shaped:

> *When did `openssl` enter my config, in which commit, and what landed alongside it?*

That is `git log -S` over the config repo with the answer rendered as a declaration rather than
as a diff hunk. It is thin by construction, and it is the question people actually ask — on a
config that has been running a year, the line you do not remember writing is most lines.

- **It reads git and nothing else.** No new store, no new record kept at sync time. The moment
  this needs its own ledger it has stopped being worth building.
- **It answers for any declaration**, not only packages — a `setting:`, a `link:`, a `repo:` and
  an `exec:` all have the same question and the same answer shape.
- It pairs with `why` (which says *what makes this package present*) and completes it: `why` is
  the current state, this is how the state got here.

## XIII.20 Exit codes are an interface now

**Approved 2026-07-23.** Once `sync --locked`, `try`, `check` and event hooks exist, LiNix's exit
codes are consumed by scripts and CI rather than read by a person. **A script has to be able to
tell "the command ran and the answer is no" from "the command broke"**, and today it cannot.

The failure this prevents is specific and quiet: a CI job that treats every non-zero as a crash
will retry a legitimate *drift found*, and one that treats every non-zero as drift will report a
crashed LiNix as a healthy divergence.

*Proposed, and small enough to settle now rather than discover one command at a time:*

| code | means |
|---|---|
| **0** | converged / no differences / the answer is yes |
| **1** | LiNix failed — a bug, a missing backend, an unreadable file |
| **2** | ran fine, and the answer is **differences found** — drift, a lock mismatch, a failed `try` |
| **3** | **refused by the guard** — the plan was understood and declined |

**3 is the one worth separating.** A guard refusal is not a failure and not a difference: it is
LiNix working exactly as designed, and it is the event an operator most wants routed somewhere
special (XIII.13's `on_guard_refusal` is the same event from the other side).

**Decide it once, in one table, before the commands that need it are written** (U21) — an exit
code decided per command is a convention nobody can rely on.

## XIII.21 A folder of dotfiles, linked where they belong (owner request, 2026-07-23)

`link:` places one file per line. A dotfiles collection is thirty of them, and thirty
`link:./dotfiles/vimrc@target=~/.vimrc` lines is a file whose every line says the same thing
twice — once as a path in the repo, once as a path in the home directory. **That repetition is
what people write a shell script for, and the shell script is what LiNix exists to delete.**

**The proposal: a directory whose layout is the declaration.** One tree in the config repo,
mirroring the home directory, and one statement that adopts it:

```
# modules/desktop.txt
dotfiles:./dotfiles
```

```
dotfiles/
  .gitconfig            →  ~/.gitconfig
  .config/nvim/init.lua →  ~/.config/nvim/init.lua
  .config/i3/config     →  ~/.config/i3/config
```

**The destination is the path, so nothing states it.** Adding a file to the tree is the whole
edit; the next `sync` links it. This is the shape `stow` has had for thirty years and the
reason it survives — there is no metadata to keep in step with the files, so the two cannot
disagree.

**What it must inherit rather than reinvent** — the list is the point, because every item is a
place where a second implementation would appear:

- **`backends::link::resolve_target`**, the one function that turns a declared destination into
  a path (V.62). A tree walker that builds its own destination paths is a second resolver, and
  the two will disagree about `~` on Windows within a release.
- **`deploy_executable`'s ownership rule**, in the shape `link:` already uses: a destination is
  LiNix's to write when it is absent, or when it is the link LiNix recorded making. Anything
  else is an error naming the file. A folder makes this rule matter forty times per sync
  instead of once.
- **The `extras_lock` teardown** (S20). Deleting a file from the tree must remove its link on
  the next sync, through the ledger every other extra uses — not a bespoke sweep.
- **`when`**, so a tree can be gated per machine like any other declaration.
- **SEC3's outside-home confirmation**, asked once for the tree rather than once per file.

**What it is not.** Not a sync engine for arbitrary directories, and not a second way to write
file content — `link:`'s `@content`/`@template`/`@decrypt` modes stay the way to *generate* a
file. This statement only says *these files, at their mirrored destinations*.

## XIII.22 BSD — the platform P7 implies and nothing here has costed (owner request, 2026-07-23)

P7 says LiNix is not Linux-first and that a feature is unfinished until every platform has an
equivalent or a written reason there can be none. **That sentence names Windows and macOS and
stops.** The BSDs are the third case, nobody has priced them, and the question is worth a
decision rather than a drift into "we support what somebody happened to test".

**More of it already works than the name suggests.** `pkgin` is registered
(`registry.rs:907`) with a real parser (`parsers/pkgsrc.rs`), so pkgsrc — NetBSD, SmartOS,
illumos, and pkgsrc-on-anything — resolves packages today. `service:` covers five init systems.
What is untested is everything else, and *untested* is the operative word: nothing in Part IV
runs on a BSD, so any claim about this is design, not measurement.

**The three real gaps, and they are not equal:**

1. **`pkg` (FreeBSD) and `pkg_add` (OpenBSD/NetBSD base) are not registered at all.** This is
   the cheap half — both are ordinary argv-and-parse backends of the kind
   `custom_backends.toml` already describes, and XIII.12's field split makes them possible
   without Rust. FreeBSD's `pkg` even has a machine-readable `-q` output, which is better than
   several backends already in the tree.
2. **`when family` has no answer on a BSD.** `parse_os_release_family` reads `/etc/os-release`,
   which FreeBSD does not ship and OpenBSD does not have. So every `when family == …` block
   silently takes the else branch — the failure mode P3 exists to forbid, and the one that makes
   a config *quietly wrong* rather than loudly unsupported. **This is the blocking half**, and it
   is blocking for the same reason the Windows registry question is: whatever the first
   implementation guesses becomes the convention.
3. **The snapshot safety net does not exist there.** U6 already records that `rebuild`'s revert
   and the guard's pre-sync snapshot are written as unqualified promises that hold only on Linux
   filesystems. **ZFS is the answer on FreeBSD and it is a better one than anything Linux
   offers** — boot environments are exactly what this document keeps describing — so BSD is the
   platform where the safety net could be *stronger*, not weaker, if anyone built it.

**The case against, stated so it is not skipped:** there is no BSD in Phase 6's images, no BSD
in CI, and P7 is already unpaid on two platforms — `setting:` is GNOME-only and the snapshot
promise is Linux-only. Adding a third platform before the second one is honest is how a parity
principle becomes a parity slogan. **XIII.4 says parity is a gap list, not a sentiment**; the
answer to this section may legitimately be *"listed, dated, and not scheduled"*.


## XIII.23 The snapshot layer is the one extension surface that never became data

**Found by re-reading `core/snapshot.rs` against `backends/registry.rs` (owner conversation,
2026-07-25).** The backend layer is this design's proof that a whole category of thing can be
data instead of code: a package manager is a `ManagerConfig` (argv templates) plus a
`ParserSpec`, registered in a `BackendRegistry`, and `custom_backends.toml` (XIII.2) adds one
with no source change. Most of a new PM is thirty lines of data.

**The snapshot/rollback layer never got that treatment.** `SnapshotProvider` is a seven-method
trait (`create`/`list`/`delete`/`restore`/`restore_capability`/`is_available`/`name`), each
provider is ~60 lines of hand-written Rust, and the four that exist — btrfs, zfs, timeshift,
`windows_restore` (VSS) — are **hardcoded into a `Vec` in `SnapshotManager::new`
(`snapshot.rs:528`), of which only the first available one is ever active.** Adding APFS, LVM
thin, bcachefs, Stratis, or btrfs-on-Windows means editing that vec and writing a full trait
impl. This is the "ninth parser" shape the backend work was built to kill, sitting untouched in
the safety layer.

**Why it matters more here than for backends, not less.** A package-listing parser that is
wrong reports the wrong packages. A snapshot provider that is wrong *loses the machine*: V.60
exists because `restore` once ran `btrfs subvolume snapshot <snap> /`, which exits 0 and rolls
nothing back. So the surface that most wants to be easy to extend is also the one where a
careless extension is most expensive — and that tension is the decision, not a reason to avoid
it.

*Proposed:* a `SnapshotProviderRegistry` mirroring `BackendRegistry`, and a config-driven
provider so a create/list/delete/restore-shaped filesystem is data — argv templates, an
id/timestamp format, an ownership marker (S3's `linix_`), and a declared `RestoreCapability`.

Constraints that are not negotiable:

- **`restore_capability` has no inferable default, and the default must be the safe one.** A
  provider whose config does not state that it can restore a *running* system is create-only —
  snapshot yes, restore no (`RestoreCapability::NotFromRunningSystem`), never `Live`. The
  failure mode of getting this wrong must stay "LiNix declined to roll back", never "LiNix ran a
  command that did not roll back" — the exact V.60 bug, one config field away from returning.
- **Retention still only reaps `is_linix_owned` snapshots (S3).** A config-driven provider must
  be able to express where its ownership marker lands (the id, or the description), or retention
  is disabled for it — it never guesses and reaps a user's own snapshot.
- **A custom provider registers last and never shadows a built-in** — the `custom_backends.toml`
  rule (XIII.2), applied to the new surface.

**Decisions: U27** (is the layer opened to a registry + config-driven/custom providers at all,
or do new providers stay hand-written Rust?) and **U28** (does a machine use one provider or
several, and is the active one chosen by *capability* — prefer live-restore — rather than list
order?).

## XIII.24 macOS has no safety net, and APFS is sitting right there (P7)

**U6 is ruled: the pre-sync snapshot, `rebuild`'s revert and `rollback` are
Linux-filesystem-only, and the docs now say so.** That paid the honesty debt; it did not pay the
*parity* debt. macOS ships APFS on every machine, APFS has local snapshots (`tmutil
localsnapshot`, `diskutil apfs`), and nothing in the tree uses them — so the second platform
LiNix claims to support has the one guarantee that turns a bad sync from a reboot into an
`undo`, and it is absent.

This is the concrete first customer for XIII.23's registry: an `ApfsProvider` is exactly the
shape the generic model is for, and building it as the second real provider is how the registry
earns the abstraction rather than speculating one. **The BSD half of the same story is already
written (XIII.22): ZFS is runtime-gated not OS-gated, so it works on FreeBSD today and gives
*live* rollback — the safety net is stronger there than on Linux, if anyone turns it on.**
Between APFS on macOS and ZFS on BSD, the "Linux-only safety net" is a gap of wiring, not of
platform.

**Decision: U29** — is APFS local-snapshot the macOS safety net, and does an APFS restore count
as `Live` or `NotFromRunningSystem`? (The second half is V.60 again: a restore that needs a
reboot into the recovery environment is not `Live`, and claiming it is would re-open the exact
hole.)

## XIII.25 One btrfs backend, and no way to declare a dataset or a volume

**`backends/btrfs.rs` declares btrfs subvolumes as objects** — `btrfs:/path` with `quota` and
`mount` options, create on install, `subvolume delete` on remove. There is no equivalent for a
**ZFS dataset** (`zfs create`, quotas, mountpoints — the same nouns) or an **LVM logical
volume**, though each is the same idea: a declared, sized, mounted storage object. These do not
fit `ManagerConfig` (they are not argv-with-`{name}={version}`), so it is Rust either way; the
question is whether they are three separate backends or one storage-object family with a shared
trait and grammar.

**The safety edge that must be answered first: does the guard cover a declared storage object?**
`btrfs:` remove already runs `subvolume delete` — which destroys a filesystem and everything on
it — and every removal path is supposed to call the guard (`app/sync/guard.rs`; "a guard on one
command is a guard on nothing"). A zfs-dataset backend that grows a `remove` path is a new way
to destroy data at scale, and it needs the guard **before** the first one ships, not after.

**Decision: U30** — is "declare a storage object" a family (zfs datasets, lvm volumes, btrfs
subvolumes) worth one trait and one grammar, or separate backends? And what does the guard owe a
removal that destroys a filesystem rather than a package?

## XIII.26 The other closed vocabularies, and which are worth opening

The design is full of closed vocabularies — that is a feature (a closed set names its legal
values in the error, VIII.2) — but "closed" and "not extensible" are not the same choice, and
they have been made together by default rather than on purpose. A survey, so the ones worth
opening are not each re-discovered as a one-off:

- **Health checks (XIII.5) are a fixed set.** A health-checked upgrade rolls back if the machine
  is "unhealthy", but health is whatever LiNix already knows how to test. A user with a service
  that must answer on a port, or a config file that must parse, cannot say so. A check is the
  most check-shaped thing there is — argv, exit 0 is healthy — which is why it is the closed
  vocabulary most obviously ready to open. **Decision: U31.**
- **`setting:` has exactly one adapter (GNOME/gsettings).** macOS `defaults`, the Windows
  registry (7e), KDE `kwriteconfig` are all the same shape and none exist; the adapter *set* is
  itself an extension surface, and opening it is the same registry idea as XIII.23. This is not
  a new decision — it is **U19** (user vs system, `HKCU` vs `HKLM`) and 7e, which must answer
  first, because an adapter that cannot say *whose* setting it writes is worse than no adapter.
  Recorded here only so the adapters are seen as one surface, not four features.
- **`when` facts are closed, and this one is already answered.** `when family/os/hostname/…` is a
  fixed vocabulary — and **XIII.12 plus the W-series already open it** with user-defined `when`
  variables. Named here so it is not re-proposed: the extension point exists, it is `vars`.
- **Service/init managers** are enumerated in `service:`. Opening that set is possible but has no
  demand behind it yet — listed, not proposed (XIII.4: parity is a gap list, and this is an
  entry on it, not a plan).

The pattern across all four: **an extension surface is worth opening when the thing it adds is
data the user already has and LiNix cannot hear** — a check command, a settings key, a machine
fact. It is not worth opening for symmetry alone.

## XIII.27 FreeBSD `pkg` and OpenBSD `pkg_add` — the package half of BSD, unblocked

**U26 is ruled and this needs no further decision: it is a build item, listed here so it is not
lost inside a ruling.** `when family` already answers correctly on the BSDs (`HostFacts::current`
falls back to `std::env::consts::OS` → `freebsd`/`openbsd`/`netbsd`), so the one thing that made
a BSD config *silently* wrong is closed. What remains is ordinary backend work:

- **FreeBSD `pkg`** is the easy one, and better-shaped than several backends already shipped:
  `pkg install -y` / `pkg delete -y`, and `pkg query -a '%n %v'` gives machine-readable
  name-and-version with no column-guessing. `pkg query '%a'` marks automatically-installed
  packages, so the user-chosen set — the `manual` list X.4 needs for `adopt` — is answerable, not
  `Unsupported`.
- **OpenBSD `pkg_add` / `pkg_delete`** with `pkg_info`; version pinning is by flavour, so like
  MacPorts it declines an exact pin rather than record a wrong one.
- **Both fit `custom_backends.toml` (XIII.12) today** — a user on FreeBSD can teach LiNix `pkg` in
  six lines before either is a built-in. The reason to ship the built-in anyway is XIII.12's own:
  the built-in is the portable spelling (P7), a definition naming `pkg` means nothing on Debian,
  and a bare name must resolve per machine.

Of the three BSD gaps U26 named, two are now closed — `family`, and the safety net (**ZFS,
already working**, XIII.24) — and only the backends remain. No decision; whenever.

## XIII.28 How open should this get — the Lisp question, asked on purpose

**Owner direction, 2026-07-25: "as open as possible, as if it were Lisp."** The sections above
open one surface at a time — a provider here, a check there. This one asks the question the others
are instances of: **how much of LiNix should be programmable from the config, and where is the
line openness must not cross?**

Lisp is open in three ways worth separating, because LiNix already has the first, is one field
from the second, and the third is where the danger lives:

1. **Code is data.** The config already is data, and `linix eval` (XIII.15) prints the *resolved*
   model as JSON — the image is inspectable. Done.
2. **Anything can be named and reused.** Backends (XIII.2), nouns (XIII.12), `when` variables
   (W-series), snapshot providers (U27), checks (U31) — the vocabulary is becoming user-extensible
   surface by surface. This is the work in flight.
3. **Anything can be *abstracted* — a name that expands into other names.** This is `defmacro`,
   and LiNix has no equivalent. A module groups declarations but cannot take an argument; there is
   no way to write "a dev workstation for *this* user with *this* GPU" once and apply it twice.
   **This is the real frontier, and the one that reopens the security question every time.**

XIII.29–XIII.31 are the three pieces of (3), safest to most dangerous. The principle that keeps
them from becoming a footgun is XIII.32 — not a decision, the line.

## XIII.29 Parameterized modules — the macro LiNix doesn't have (U32)

A module is a named set of declarations. It cannot take an argument, so the moment two machines
want *almost* the same set, the set is copied and the two drift — XIII.14's "six TOML files
written a thousand times", one level up.

*Proposed:* a module may declare parameters and be applied with values.

```
# modules/workstation.txt
param user
param gpu = none

link:@target=/home/{user}/.gitconfig  ./gitconfig
when gpu == nvidia
  pacman:nvidia-dkms
```

```
# hosts/desktop.txt
use workstation(user=shaul, gpu=nvidia)
```

- **It expands to ordinary declarations.** Nothing a macro produces is a thing `sync` could not
  already do; the expansion is visible in `linix eval` and in the removal preview before it runs
  (the `adopt` shape). A macro that could produce an action you cannot see is the one thing this
  must not be.
- **Substitution is the `when`/`vars` interpolation that already exists**, not a new language.
  `{user}` is `vars`' machinery (W-series) reaching one scope wider — into the module's own
  parameters — which is why this is a grammar extension, not a second engine.
- **The closed-vocabulary rule survives (VIII.2):** a `param` the caller does not supply and that
  has no default is a **loud error naming the module and the missing parameter** — never an empty
  string that makes a `when` silently false (the P3 failure `vars` was hardened against).

**Decision: U32** — do modules take parameters, and if so is a parameter's type checked (a `gpu`
that must be one of a named set, versus free text)? A typed parameter is a second closed
vocabulary the user defines — powerful, and also a second place a name can be misspelled.

## XIII.30 Generated declarations — the `eval` inside the config (U33)

The dangerous half of Lisp: a declaration produced by *running something*, not by writing it.
`vars` (W-series) already lets a value come from a command through the hook ledger; the next step
is a whole declaration from a command — "install whatever `./pick-python.sh` prints", "declare a
`link:` for every file this generator emits".

**This is `read`/`eval`, and it carries `read`/`eval`'s whole liability.** It is the difference
between a config that *is* the state and one that *computes* the state, and the second can do
anything the program it runs can do. LiNix already has one of these and treats it as radioactive:
`exec:` (U3, U4) is exactly "run a thing", and the ruling was *`exec:` is for actions with no
inverse, not for installing software.* A generator that emits install declarations walks straight
back to that line.

*Proposed, if at all, only under the constraints `exec:` taught:*

- **The output is declarations, and they pass through the same guard and the same removal preview
  as if typed.** Generated is not trusted; it is shown.
- **It runs through the II.12 hook ledger** — argv from a file that may travel, the trust model
  `vars` was forced onto (V.55) after it was handed the shell "because it is trusted like a hook".
- **A generator that fails is a failed sync, loudly** — never a silently empty declaration set,
  which is a mass-removal input (VI.0's whole family).

**Decision: U33 — is this wanted at all?** It is the most Lisp-like feature on this list and the
most likely to become the hole through which a shared config owns a machine (XIII.14's `exec:`
fear, now able to *generate* the `exec:`). Honest recommendation: **not yet, and maybe not ever** —
`vars` covers the value case, parameterized modules (U32) cover the reuse case, and what is left
is the case where the config's behaviour is not knowable by reading it, the one property this
whole design exists to refuse.

## XIII.31 `linix repl` and user-defined verbs — the interactive image (U34, U35)

Two smaller Lisp affordances that cost little because the engine already does the work:

- **`linix repl` (U34, maybe).** A prompt that resolves names, evaluates `when` against *this*
  machine, and expands a macro — read-only, no sync — so "what does `ripgrep` resolve to here", "is
  this `when` true on the server", "what does `use workstation(gpu=nvidia)` expand to" are answered
  by trying them, not by reading the manual. It is `linix eval` (XIII.15) with a cursor, and it
  shares the parser and resolver rather than reimplementing them (the U20 rule).
- **User-defined verbs (U35).** LiNix has sixty commands (XIII.8) and a user cannot add the
  sixty-first. A verb that is a *named composition of existing verbs* — `linix refresh` = `sync`,
  then `upgrade`, then the fleet report — is the Lisp `defun` over the command surface, and it is
  safe in a way generated declarations are not, because it composes audited operations rather than
  producing new ones. The line: a user verb may **sequence** built-in verbs and nothing else; the
  moment it can run arbitrary argv it is `exec:` wearing a command's clothes, which is U4's settled
  "no".

**Decisions: U34** (is a REPL worth a second entry point, or is `eval | jq` enough?) and **U35**
(may a user name a new verb, and is it strictly a composition of built-ins?).

## XIII.32 The line openness must not cross — stated so it is not eroded one section at a time

Every section above opens something, and the sum of "just one more surface" is how a system that
values closed vocabularies for their error messages ends up with none. The counterweight, as a
principle rather than a decision:

- **Open the surface where the thing added is *data the user already has and LiNix cannot hear*** —
  a check command, a machine fact, a settings key, a package manager's argv. These make LiNix fit a
  machine it did not anticipate. **Do not open the surface where the thing added is *behaviour LiNix
  cannot see*** — a declaration it cannot show before it runs, an action with no inverse it did not
  sanction. Those make LiNix a language for owning the machine that runs the config, which is
  XIII.14's unsolved problem and the reason `use` takes a name and never a URL (II.2).
- **Closed is a feature, not a limitation (VIII.2).** A closed set names its legal values in the
  error; that is the strongest thing the grammar does for a user, and every surface opened trades a
  little of it away. The trade is worth it for a firewall spelling and is not worth it for symmetry
  — "we opened X, so we should open Y" is not a reason, it is the erosion.
- **The security model is the gate, and it is still owed (U14).** Every openness on this list ends
  at one question: what stops a shared definition — a backend, a module, a generator — from running
  code the reader did not see? "Visible in a diff" is the current answer and a weak one. **No
  surface here ships past the point where a bad definition can act before it is shown** — the
  invariant the whole list is measured against, and why U33 is recommended *no* while U32 is
  recommended *yes*.

## XIII.33 The mechanism all the open surfaces share: one declared provider, not four

**The realization behind U27, U36, U37 and U38 (owner conversation, 2026-07-25): they are not
four features, they are one mechanism applied four times.** `custom_backends.toml` already proved
that a package manager is data — a name, the argv for each operation, a parser for the output,
read from a file, registered last, never shadowing a built-in (XIII.2). A snapshot provider, an
init system, a notification channel and a secret method are the **same three things**: a name,
some argv, a way to read the result. So the answer to "I don't want to write a plugin for every
filesystem" is not a plugin system — it is **one more data file**, and the same one covers all
four surfaces.

**Direction (owner, 2026-07-25): the outcome is that these surfaces get opened — the *whether* is
settled, and the decisions below are the *how*.** Every closed provider-list in the tree —
snapshot/rollback providers (U27), storage objects (U30), init systems (U36), notification
channels (U37), secret methods (U38), and health checks (U31) — is to become **reachable without a
source change**: a declared provider in a file, not a Rust `match`, or (where one already suffices)
an existing event hook. LiNix should not need a rebuild or a release to reach a filesystem, an
init, a channel or a secret manager it did not ship with. The per-surface decisions are therefore
**not "should we open this"** — that is answered, yes — **but "how, and in what safety order"**:
what capability a provider must declare before it is trusted (U27), whether a parameter is typed
(U32), which surface waits on which ruling (U38 waits on the T-series so an unaudited command is
never handed a plaintext secret). A surface may reach the outcome by its own block or by pointing
at a mechanism that already exists (U37's event hook) — what is fixed is that it is reached.

**The one exclusion, stated so it is not read into the direction:** this covers *provider-lists* —
finite sets of interchangeable "run these commands" things. It does **not** cover XIII.30 (U33,
generated declarations), which is not a closed list being opened but behaviour LiNix cannot see
before it runs; XIII.32's line still refuses that. "Open every provider-list" and "let the config
run anything" are different sentences, and only the first is the outcome here.

*Proposed:* a `providers.toml` beside `custom_backends.toml` (machine-local, II.1), one block per
kind:

```toml
[[snapshot]]
name    = "lvm"
create  = ["lvcreate", "--snapshot", "--name", "{id}", "{source}"]
list    = ["lvs", "--noheadings", "-o", "lv_name"]
delete  = ["lvremove", "-y", "{id}"]
restore = ["lvconvert", "--merge", "{id}"]
restores_running_system = false   # the safe default — omit it and it is false (U27)

[[init]]
name   = "dinit"
enable = ["dinitctl", "enable", "{name}"]
start  = ["dinitctl", "start", "{name}"]
stop   = ["dinitctl", "stop", "{name}"]

[[channel]]
name = "slack"
send = ["curl", "-sXPOST", "{webhook}", "-d", "{body}"]
```

**The one rule that makes this safe across every kind: a capability a provider does not declare,
it does not have.** U27's `restores_running_system = false` default is the pattern — an operation
the block does not spell out is assumed impossible, never attempted-and-hoped. A snapshot provider
that does not say it can restore a running system will not try (V.60); a secret provider that does
not say it can inject into memory writes to disk under the T-series rules, or refuses. **The
unsafe reading is never the default.** This is what lets the mechanism be open without being a
footgun: the file can only *add* a capability by naming it, and naming it is the thing a reviewer
sees in the diff.

This section has no decision of its own — it is the shape U27 recommends (option (a)), stated once
so U36–U38 can point at it instead of re-deriving it. Build it for snapshots first (U27); the
other three kinds are then a schema addition, not a new mechanism.

## XIII.34 Init systems are a closed enum — s6, dinit, runit, Shepherd can't be added (U36)

`backends/service.rs` is a fixed `enum InitSystem` — Systemd, OpenRC, SysVinit, launchd, Windows
`sc` — behind a hardcoded `(InitSystem, ServiceAction)` command table. It covers the five that
matter to most people and **nothing else**: s6, dinit, runit, GNU Shepherd, and every appliance
init are simply unreachable, and a `service:` line on such a host does not fall back — it has no
branch to take. This is the snapshot vec's problem in a different file: interchangeable providers,
each one just argv, frozen into a Rust `match`.

It is also the **lowest-risk** surface to open — enable/disable/start/stop/restart are ordinary,
mostly reversible operations with no data to destroy — which is why it is the natural second
customer of XIII.33's mechanism after snapshots.

**Decision: U36** — are init systems a declared-provider kind (a `[[init]]` block), or does the
built-in enum stay closed and niche inits go through `exec:`? *Recommendation:* open it; it is the
cleanest fit the mechanism has, and P7's "a written reason or an equivalent" is better served by
"write six lines" than by "unsupported".

## XIII.35 Notifications are desktop-plus-email, and nothing else reaches them (U37)

`app/scheduler/notify.rs` matches exactly `desktop`, `email`, `both`, and warns "unknown channel"
for everything else. So Slack, ntfy, a webhook, Telegram, Pushover, a paging service — every
channel a real fleet actually uses — is absent, and each one is a feature request waiting to
happen. A declared channel ("run this command, hand it the subject and body") closes the whole
category with one block.

**This overlaps XIII.13's event hooks, and the overlap is the decision.** An event hook already
lets a script run when a sync finishes or the guard refuses — so "notify me on Slack" is *already*
possible as a hook that shells out to `curl`. The question U37 asks is whether notification
*channels* are worth being a first-class declared-provider kind on top of that, or whether the
honest answer is "channels are `desktop`/`email` as built-ins, and anything else is an event hook,
by design". **Decision: U37.** *Recommendation:* do not add a second mechanism — point channels at
the event-hook path that already exists, and spend the effort on documenting it, unless a channel
needs something a hook cannot express (per-level routing, say), which is the only thing that would
justify a `[[channel]]` block of its own.

## XIII.36 Secret decryption is age-shaped, and no other secret manager can plug in (U38)

`model/secret.rs` is built around `age` — age plugins, hardware-token identities, the touch
timeout. That is a good default and it is the *only* door: sops, HashiCorp Vault, 1Password, AWS
and GCP secret managers, and plain GPG have no way in, though each is again "run a command that
turns a reference into a plaintext". The shape fits XIII.33 exactly.

**This is the one surface where openness is not cheap, and it opens last.** A decrypt provider's
output *is* a secret. A backend that lists the wrong package is a nuisance; a secret provider that
writes plaintext to disk, or leaves it in the process table, or logs it, is the failure the whole
`secret:` feature exists to prevent. So a declared secret provider is bound by the T-series rules
LiNix already argued for age — the plaintext obeys the same no-disk / in-memory / no-log handling
(T7, reopened), and a provider that cannot promise that is refused, not trusted.

**Decision: U38** — is secret decryption a declared-provider kind, and if so, gated behind which
T-series rulings? *Recommendation:* yes in principle — the mechanism is identical and users
genuinely have other secret managers — but **not before the T-series settles how plaintext is
handled**, because opening this surface before that is decided is handing an unaudited command the
one thing LiNix promises to guard. The safe order is: rule the T-series, then open the door the
mechanism already makes trivial.

---

**Decisions: U1–U38.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
