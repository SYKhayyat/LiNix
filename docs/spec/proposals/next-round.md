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


---

**Decisions: U1–U26.** They live in [the decision register](../decisions.md), with a status on
each — this part states the shape, the register states what is still unanswered.
