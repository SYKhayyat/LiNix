# LiNix v7 — the declarative model

**Status:** ~~Phases 0 and 1 complete~~ — **audited 2026-07-17 and both are false.** Phase 0
deleted the `-g` flag but not the model beneath it; Phase 1 built the grammar but **added a
ninth `backend:name` parser instead of removing eight**, six of which still skip validation.
Phase 2 is in progress and its own account is accurate. **See Part VII → "Audit: two ✅ that
are not true" before planning anything — the deletions are owed, and II.8 sits on top of
them.**

**Part VII holds the current state — read it after Part II and before you touch anything.**

Supersedes [`docs/AUDIT-v6.org`](AUDIT-v6.org) — the audit that found all of this — except
where Part VI carries an item forward explicitly. Part VI carries everything you need;
read the audit only for the underlying evidence (the measurements and the `file:line`
citations behind each finding).

---

## PROMPT — read this first, then follow it

You are implementing LiNix v7 on branch `v6` at `C:\Users\Administrator\Videos\Nexus\linix`.
This document is your specification. It was produced by a long design conversation with the
owner; **every rule in it was argued for and chosen, and Part V records why.**

**Before you write a line of code:** read Part I and Part II in full. Read Part III's "What
already exists". You cannot implement this correctly from a summary.

### Rules of engagement

1. **Part II is canonical.** If the code disagrees with Part II, the code is wrong. If Part
   II seems wrong, **stop and ask** — do not fix it yourself.
2. **Never change a Part II rule without reading its Part V entry first.** Each is the scar
   of a real bug. Most "obvious improvements" here are things we already tried and rejected;
   Part V says why. If Part V doesn't cover your case, that is a real gap — **ask.**
3. **Ask before every real decision.** The owner makes the decisions; you are responsible for
   bringing things to their attention. Explain clearly, in plain words, no jargon, as if to a
   smart new intern. **Do not use metaphors.** Give real context and a recommendation.
4. **Never remove a feature without asking**, even one this document doesn't mention. Some
   may be genuinely important. The deletion list in II.17 is already approved — anything
   beyond it is a question.
5. **Do not invent.** If the spec doesn't say, it's a gap. Ask. Do not guess and do not
   quietly pick a default — that is how this codebase got eleven magic numbers nobody can
   change (V-P5).
6. **Commit at every major step**, with a message that says what changed and what it does not
   yet do.
7. **Check everywhere. We cannot afford bugs here.** This codebase's flagship bug ran
   `apt-get purge` on hundreds of system packages during a routine test.
8. **Report honestly.** If tests fail, say so and paste the output. If you skipped a step,
   say that. If you're unsure something works, say you're unsure. Never describe unverified
   work as done.
9. **A ✅ is earned by a command, not by a belief.** Rule 8 was already here, in these words,
   and **Phases 0 and 1 were both marked ✅ while untrue anyway** — so the rule is not
   enough on its own. **Before writing ✅ on a phase, re-run that phase's Exit criterion and
   paste the result.** Before *trusting* one, re-run it. **A phase that deletes things is
   done when the greps are quiet, not when the new thing works** — Phase 0 and Phase 1 both
   failed exactly here: the replacement was built, the replaced was left standing, the tests
   went green, and green was read as done. **Green means the old code still works. That is
   the thing you were trying to remove.**
10. **At every phase change, run Part VII's audit section.** It is a list of commands, not
    prose. Delete each finding as its command goes quiet — **in the same commit as the fix**,
    because an audit nobody retires becomes the next thing nobody believes.
11. **A green suite is not success. It is the absence of one kind of failure.** The tests
    cannot see the plan. They do not know Phase 0 asked for a deletion, that II.6 asked for
    three verbs and got two, or that the grammar was supposed to *replace* the eight parsers
    rather than become the ninth. **Nothing in this document is verified by `cargo test`** —
    every ✅ that turned out false was green when it was written. So green is a floor, not a
    finding: it says you broke nothing that was already covered, which is the least
    interesting thing you could report and never the thing that was asked. **The question is
    never "do the tests pass?" It is "did I do what the plan said, in full?"** — and that is
    answered by re-reading the phase and checking yourself against it, line by line, not by
    reading a number. A partial implementation passes. A plan followed for three steps of
    five passes. The wrong design, built perfectly, passes.

### How to work

- **Follow Part III's phases in order.** Phase 0 is pure deletion and comes first
  deliberately: do not carefully port something you are about to delete.
- **Phase 2 cannot be split, and the branch is red for a long stretch.** That is expected. Do
  **not** run the old and new models side by side behind a flag — that is the exact "two ways
  to do one thing" disease this whole design cures, applied to ourselves.
- Every phase has an **exit condition**. Meet it before moving on. The exit condition is the
  bar — **not the test suite** (rule 11). Read the Exit lines and notice what they actually
  ask for: Phase 0 wants the codebase *smaller* and a line count reported; Phase 4 wants a
  test **per removal path proving the guard fires**; Phase 6 wants an **air-gapped container**
  to restore. None of those is "the suite is green", and no amount of green implies any of
  them. Phase 1's Exit is the one that reads like tests — "unit tests for every grammar rule
  above, including every error case" — and note that it names a *surface to cover*, not a
  result to observe; note also that **Phase 1 is one of the two phases that was falsely marked
  ✅.** Its tests were written and they passed. The phase still wasn't done, because covering
  the new grammar was never the same as unifying the parsers onto it.
- `cargo test` and `cargo clippy` must be green at every commit outside Phase 2's interior.
  Necessary, nowhere near sufficient: a phase can be green and untouched.
- Part IV lists the specific proofs. They are not optional.

### The three principles that decide arguments

- **Fail loud, never silent.** Every bug in this codebase is the same bug: something didn't
  work and said nothing. Given a choice between a wrong answer and a visible error, take the
  error. Always.
- **There is no legacy.** No users exist. No migration path, no compatibility shim, no
  deprecation warning, no old-format reader. Delete legacy branches on sight.
- **A comment states a constraint the code can't show. Nothing else.** Not what the line does.
  Not where it came from. Not that it's good. This repo has ~884 comments that break this
  rule, written by models congratulating themselves; do not add the next one.
  *(The figure was 139 in the first draft, measured against an older, smaller tree. Re-measured
  2026-07-16 across 2,147 comment blocks.)*

---

**How to use this document.**
- **Part I** — principles. Never violate these. If a decision seems to conflict with one,
  the decision is wrong, not the principle.
- **Part II** — the target state. This is what to build. It is canonical: if code disagrees
  with Part II, the code is wrong.
- **Part III** — the work, in dependency order.
- **Part IV** — how to know it's right.
- **Part V** — why. **Every decision in Part II has an entry here.** Do not change a
  Part II rule without reading its Part V entry first; each one is the scar of a real bug.
- **Part VI** — bugs: killed by this design, and carried forward as real work.

Facts marked **(measured)** were verified against real containers or real code with a
citation. Everything else is design.

---

# Part I — Principles

**P1. Every imperative command is a shortcut for editing a file and syncing.** Nothing can
be done only imperatively. If a command can make a change that no file could have made,
that command is a bug.

**P2. There is no legacy.** There are no existing users and we do not want legacy. No
migration path, no converter, no compatibility markers, no deprecation warnings, no
old-format readers. Every "legacy" branch in the codebase is dead weight for nobody. Delete
on sight.

**P3. Fail loud, never silent.** Every bug in this codebase is the same bug: something
didn't work and said nothing. When the choice is between a wrong answer and a visible
error, take the error.

**P4. A fact lives in one place.** A fact stored twice is a fact that will disagree with
itself. Compute, don't copy.

**P5. A default without a reason cannot be safely changed.** If you add a number, add the
reason. If you can't state the reason, don't add the number.

**P6. A comment states a constraint the code can't show. Nothing else.** Not what the line
does — the line does that. Not where it came from — git does that. Not that it's good —
that's the reader's call.

---

# Part II — The target state

## II.1 Files on disk

**Your repo** — `$LINIX_CONFIG_DIR` or `~/.config/linix`. **This is a git repo.**

```
modules/            your lists              lowercase names       *.txt
profiles/           your choices           Capitalized names
active              which profiles are on
priority            which backends, in order
schedules           when LiNix runs itself
locks/              what everything resolved to    one file per backend
preferences.toml    refusals and behaviour
```

**LiNix's data** — `$LINIX_DATA_DIR` or the platform data dir. **Never in git. Never in a
folder LiNix scans.**

```
registry.json       what LiNix currently owns
snapshots/          snapshot metadata, tagged with commit hashes
```

**Secrets** — the environment only. `LINIX_GITHUB_TOKEN`. Never a file.

**Facts about this machine** — **detected, never configured.** Core count, whether btrfs /
ZFS / Timeshift exists and where, which backends are installed. LiNix looks; it does not
ask you to maintain them by hand on every machine forever.

## II.2 Grammar

### Lines

A file is lines. A line is blank, a comment, a statement, or a block.

```
# whole-line comment                          anywhere
apt:curl                    # trailing        anywhere on a statement
```

**An unrecognised line is an error.** Not a package name. The error names the file, the
line number, and what was expected.

### Statements

```
NAME                          bare package — backend resolved via `priority`, then locked
BACKEND:NAME                  explicit backend
BACKEND:re:PATTERN            regex — matches names in that backend
absent:BACKEND:NAME           declare it must not exist
repo:SPEC                     a repository
shim:NAME                     a shim
schedule:NAME                 a scheduled task (only in `schedules`)
service:NAME                  a service
link:SOURCE                   a managed file
use NAME                      reference a module (lowercase) or profile (Capitalized)
```

`use` takes **a name. Never a path, never a URL.** A file from the internet is a fetch step
that puts a module on disk; then you `use` it by name like everything else.

### Options — two forms

**Short form.** `@key=value,key2=value2`.
**A comma in a value is an error**, not a guess: *"commas need the block form."*

```
apt:jq@version=1.6                    ok
apt:jq@version=>=1.0,<2.0             ERROR → "commas need the block form"
apt:curl@2.0                          ERROR → "did you mean @version=2.0?"
```

**Block form.** Everything after the first `=` to end of line is the value: **verbatim,
trimmed.** No escaping is possible and none is needed.

```
apt:nginx {
  after_install = ./setup.sh --flag=a,b
  requires      = apt:libfoo
  requires      = apt:libbar          # a key given twice makes a list
}
```

- **A value cannot contain a newline.** If you need one, that's a file, not an option.
- **`#` does not start a comment inside a block value.** The value includes it.
- A block value containing ` # ` triggers a hint: *"block values are verbatim — did you mean
  a comment? Put it on its own line."*

### Blocks

**The header decides what the body is.** `module` and `when` are keywords; their bodies are
lines. Anything else is a declaration; its body is options.

```
module fancy {          keyword → body is lines
  apt:neovim
}

when os == linux {      keyword → body is lines
  apt:htop
}

apt:nginx {             declaration → body is options
  after_install = ./setup.sh
}
```

**`when` gates the lines inside it. One rule, everywhere** — in a module those lines are
packages; in a profile they're imports; in `priority` they're backends; in `active` they're
profile names. To gate a whole file, wrap it. Keys: `os`, `arch`, `host`, `hostname`, `family`. Operators: `==`, `!=`,
`in [a, b]`.

### Option keys

| key | meaning |
|---|---|
| `version` | exact or range |
| `hold` | never upgrade. **`@hold` + `@version=` is a contradiction → error** |
| `expires` | **absolute** datetime. Present now, absent after |
| `until` | **absolute** datetime, on `absent:` only. Absent now, present after |
| `requires` | `BACKEND:NAME` — install that first. **A bare name is an error** |
| `after_install`, `before_install`, … | a hook. Hashed and locked |
| `source` | on `shim:` |
| `cron`, `run` | on `schedule:` |
| `target`, `content`, `template`, `decrypt`, `identity` | on `link:` |
| `enabled`, `status` | on `service:` |

## II.3 Modules

- A module is a **list of lines**.
- **The filename is the module name, lowercased.** `Editors.txt` → module `editors`. A file
  with no `module` block is one module named after the file. Anything outside a block
  belongs to the file's own module.
- **A module can `use` other modules. A module can NEVER reference a profile.** The layering
  rule. **A `use` loop is an error** (II.7).
- **`modules/*.txt`. The folder decides.** Anything else in `modules/` is silently ignored,
  so a `README.md` costs nothing.
- **LiNix only parses what the active profiles reach.** `linix check` parses everything on
  demand.
- **No `present:`.** A bare line already means present.
- `-` subtraction does not exist in modules. `absent:` does.

## II.4 Profiles

- Set math over modules and profiles: `|` union, `&` intersect, `\` difference, `-`
  subtract, parentheses. Directives `exclude` / `intersect`; **`use` is union** (V.46).
- **Set math produces packages, so a profile that uses it resolves to packages, not to
  modules** (V.46). It operates on lines, not on names, so **every surviving package still
  knows the file it came from** and `upgrade --module` still finds it.
- **Order is fixed: gather, then narrow by each `intersect`, then subtract. Subtraction
  always wins**, whatever order you wrote the lines in — otherwise `use gaming` below
  `-steam` quietly puts steam back.
- `intersect` narrows and never adds: a package only the other side has does not appear.
- **A profile MAY hold package lines directly.** Cost, accepted knowingly: a module can
  never reach them (layering rule), so they are unshareable, permanently.
- **Only profiles can be activated.** By name, in `active` — by hand or via `activate` /
  `activate -a` / `deactivate` (II.6).
- **A profile may reference profiles. A `use` loop is an error** (II.7).
- `absent:` does not exist in profiles. `-` does.

## II.5 Naming

- **Profiles are Capitalized. Modules are lowercase.** `(Work | gaming) & security` tells
  you what everything is with zero noise.
- `use` disambiguates a reference from a package. Case disambiguates profile from module.
- Filenames are lowercased into module names, so a filename can never mint a profile.
- **Error messages must teach the rule:** *"no profile named `Editors` — did you mean the
  module `editors`? Profiles are Capitalized, modules are lowercase."*

## II.6 The other files

**`active`** — a plain list of profile names, unioned. Answers exactly one question: *what
is this machine set to right now?* Nothing else goes in it.

**Names, never expressions.** The set math lives inside profiles (II.4). `active` is the one
file you read to know what is on, so it stays a list you can read at a glance. `when` gates
it like any other file (II.2).

```
Work
Gaming

when host == laptop {
  Travel
}
```

**Three commands write it. Nothing else does.**

| form | does |
|---|---|
| `activate NAME…` | `active` becomes exactly this list |
| `activate -a NAME…` | adds to the list |
| `deactivate NAME…` | takes away from the list |

All three **write the file and sync** — the same as editing it by hand, because the file is
the state. Each prints what it touched: `active is now Work, Gaming`.

- **`activate -a` and `deactivate` write names at the top level and never touch a `when`
  block.** A block is something you wrote; those two add to it and subtract from it by hand,
  or not at all. **`activate` is the exception, and it is the whole exception** — see the next
  bullet. *(This bullet used to say "the CLI" and applied to all three verbs, which
  contradicted the one below it. Owner decided 2026-07-17: the set form sets.)*
- **`activate NAME…` overwrites the file — blocks included.** It is the set form; it sets.
  **This is not a special case and gets no extra refusal** (V.44). It does not ask, because
  overwriting the list *is* the command's job — but **it is not silent** (S6): it names every
  block it removed. *"active is now Work, Gaming. Removed the `when host == laptop` block on
  line 4."* **Automatic and silent are different things, and only one of them is a decision
  the user gets to review after the fact.**
- **The asymmetry is the point, and it is the reason `-a` exists.** `activate` is the blunt
  verb: it makes the file say exactly what you typed, and a block is part of what the file
  says. If you want your blocks kept, you want `activate -a` or `deactivate` — the surgical
  pair. **Two verbs that both half-preserve blocks would be two ways to do one thing** (P1);
  one that replaces and two that edit is one way each.
- **`deactivate NAME` removes the name from the top level AND from every `when` block that
  applies to this host.** *"Deactivate" must mean it. A verb that removed the top-level line
  and left the name switched on by a block two lines down would be reporting a state it did
  not reach* — the same defect as `activate` "setting" a list that a block then contradicts.
  If that empties a block, **the block goes too, and it says so**: *"Removed Travel. Removed
  the now-empty `when host == laptop` block on line 4."*
- **A `when` block that does NOT apply to this host is never touched, and that is not an
  exception — it is the same rule.** On the desktop, `when host == laptop { Travel }` is not
  activating anything, so there is nothing there to deactivate. **`active` is a file you
  commit and share; reaching into another host's block from this one would change a machine
  you are not sitting at.** So it changes nothing and says why: *"Travel is not active on this
  host. `active` line 4 activates it when host == laptop — edit that by hand if you meant
  every machine."*
- **This is the one place `deactivate` edits a block and `activate -a` does not**, and the
  asymmetry is not arbitrary: **adding has a choice of where to put the name and removing does
  not.** `-a` appends at the top level because a block is a rule you wrote and it has no
  business joining it. `deactivate` has no such freedom — the name is where it is, and leaving
  it there would make the verb a lie.
- **`activate` with no names is an error:** *"activate needs a profile name. To turn
  everything off, edit `active` yourself."* An unset `$PROFILE` must not empty the machine.
- **`activate -a` on a name already listed, and `deactivate` on one that isn't, say so and
  change nothing.** Not errors — the end state is what was asked for.
- **A name that isn't a profile is an error, and it teaches II.5:** *"no profile named
  `editors` — profiles are Capitalized, modules are lowercase."*
- **`deactivate` removes packages, so it goes through the plan and the guard** like every
  other removal.

**`priority`** — an ordered list of backends, with `when` blocks.

```
when host == laptop {
  apt
  cargo
}

apt
dnf
cargo
snap
```

**Listed = available to LiNix, in this order. Not listed = LiNix does not use it at all** —
`snap:foo` errors with *"snap isn't in your priority list."*

**`schedules`** — lines, with `when` blocks. **Being in the file means it's on.** No
active-list.

```
schedule:nightly {
  cron = 0 3 * * *
  run  = sync
}
```

`run=` is hashed and locked exactly like a hook.

**`locks/`** — one file per backend. **Generated. In git. Yours.** Records:

| | |
|---|---|
| version | `apt:curl → 7.81.0` |
| **resolved backend for a bare name** | `ripgrep → cargo:ripgrep@14.1.0` |
| **regex expansion** (only if frozen) | `re:^texlive- → [312 names]` |
| **hook script hash** | `fonts:after_install → sha256:a3f1…` |

`linix lock` regenerates. `linix lock <name>` regenerates one. `linix lock --backend cargo`
regenerates one file.

**`preferences.toml`** — refusals and behaviour. **Nothing writes to it but you.**

## II.7 Resolution

1. Read `active` → the profile names, unioned.
2. Resolve profiles → the module set. Profiles may reference profiles; modules may not.
3. Parse **only** the modules reached. Apply `when`.
4. Resolve each line. Bare names use `priority`, then the lock.
5. **Two active declarations that contradict = ERROR.** Stop, show both, name both files.
   Not first-wins, not file order.
6. **Dated lines:**
   - **A dated line stops counting once its date passes.**
   - **While it is counting, a dated line beats an undated one.** *(The only exception to
     rule 5.)*
7. Produce the desired state.

### Cycles

**A `use` cycle is an error, at both layers.** `Work` uses `Gaming` uses `Work`. Module `a`
uses `b` uses `a`. **Self-reference is the one-element case** and is the same error.

**`@requires` cycles are the same error** — `apt:a@requires=apt:b` and
`apt:b@requires=apt:a`. Same graph, same walk, same answer: the planner orders by the native
dependency graph plus `@requires` edges, and a loop has no order. **It owes the same error as
a `use` loop**: which packages, and the file and line each edge came from.

**The error names every file and line in the loop, in order, and stops.** It does not dedupe
and carry on (V.45):

```
ERROR: profiles reference each other in a loop

  profiles/Work.txt:3     use Gaming
  profiles/Gaming.txt:7   use Servers
  profiles/Servers.txt:2  use Work
                          ^ back to Work
```

**A diamond is not a cycle.** `Work` and `Gaming` may both `use base`. Reaching a module
twice by two routes is not an error — sharing a module is what modules are for (V.2). Only a
path that **returns to where it started** is a loop. *(So the check is a path, not a set of
everything ever visited.)*

**`linix check` catches cycles no active profile reaches** — consistent with II.3: LiNix
parses what the active profiles reach, `check` parses everything on demand.

**Ordering is the planner's job, never the file layout's.** Repos first → refresh indexes →
packages (native dependency graph + `@requires` edges) → things depending on packages
(services, shims, links).

**What LiNix may remove: what it manages and you stopped declaring. Plus `absent:`. Nothing
else, ever.**

## II.8 Commands

| command | does |
|---|---|
| `install PKG… [--into NAME]` | write the line, sync |
| `uninstall PKG… [--temp]` | remove the line from every active module, sync |
| `forget PKG…` | drop from the registry. Stays installed. LiNix never touches it again |
| `adopt [PKG]` | take over the machine, or one package |
| `sync` | make the machine match |
| `plan` | show what sync would do |
| `check` | parse everything, report errors |
| `lock [NAME]` | freeze versions / expansions, approve hooks |
| `purge-unmanaged` | delete everything LiNix doesn't manage |
| `clean` | ask each backend to tidy its own orphans |
| `unmanaged` | what `adopt` would adopt |
| `absent` | every `absent:` line in force, and its module |
| `diff COMMIT COMMIT` | the change in **packages**, not text |
| `teleport PKG BACKEND` | edit the line, sync |
| `shell` | throwaway shell. Outside the model |
| `bundle` | git bundle + artifacts + registry |
| `export FORMAT` | Brewfile / requirements.txt / package.json |
| `activate NAME… [-a]` | write `active` — the list, or `-a` to add to it (II.6), sync |
| `deactivate NAME…` | take away from `active` (II.6), sync |
| `upgrade`, `list`, `status`, `doctor`, `profile`, `service`, `repo`, `hold` | as today, all reduced to file edits |

**`shell` must be honest about being outside the model:** it writes no module, and **stops
recording transient packages in the registry** — which is what lets a session's leftovers
look like managed drift later.

**Destroying a file you wrote** (e.g. `module create` over an existing file) is a **plain
refusal plus `--force`**, like every other tool. It has nothing to do with packages and must
not be wired to a setting about removals.

**Every command prints the file it touched:**
`Added jq to modules/imperative.txt (used by profile Work)`

**`--into` takes a module (lowercase) or a profile (Capitalized).**

**Three landing modules, named for how the package arrived:**

| module | arrived via |
|---|---|
| `imperative` | `linix install` |
| `hooks` | `apt install`, caught by the hook |
| `adopted` | `linix adopt` |

The first time LiNix writes to one, it adds `use <name>` to the active profile and **says
so**. A normal line you can read and delete. **Never implicit.**

**`uninstall` warns about inactive declarations:** *"jq is still declared in module
`gaming`, which isn't active. It will come back if you activate Gaming."*

**`uninstall PKG --temp` on an undeclared package is an error:** *"steam isn't declared, so
there's nothing for it to come back to. Did you mean a plain uninstall?"*

**`--backend` is allowed on read-only and upgrade; REFUSED on anything that removes.**
`plan`, `list`, `upgrade` → yes. `sync`, `purge-unmanaged` → error: *"scoping a removal
isn't safe; use a profile."*

**`clean` goes through the guard** — ask the backend what it intends, check the list against
protection, refuse if it touches something protected. **Sync nudges:** *"3 packages are now
orphaned; run `linix clean`."* Want it automatic? `schedule:tidy { run = clean }`.

## II.9 Adopt

**Adopt takes manually-installed packages only. Never the dependency closure.**

**(measured)**

| Backend | Record | Result |
|---|---|---|
| apt | `apt-mark showmanual` | 103 of 579 |
| pacman | `-Qqe` | 11 of 173 |
| conda | `env export --from-history` | 4 of 88 |
| winget / choco / scoop | installs no dependencies — everything **is** chosen | exact |
| **pip** | **none.** No flag separates dependencies | **adopt nothing, say why** |

**Base-image packages ARE adopted** — `grub-pc`, `linux-image-generic`. They keep the
machine bootable, and `purge-unmanaged` deletes what isn't declared.

**Output:** one `modules/adopted.txt`, grouped by backend with comment headers, sorted.
Header states: this is an estimate; deleting a line uninstalls; `linix forget` is the way
out. A second section lists OS-essential packages, commented out.

**Adopt does NOT consult `protected_packages`.** This resolves **E7**, where "protected"
means two opposite things: *never remove* in the guard, *never adopt* in `migrate.rs`.
**Protection means one thing: never remove.** So adopt takes every manual package including
protected ones; protection then prevents their removal. This is a **change from what Stage 2
built** — Stage 2 routed adopt's skipping through `guard::protection_of`, which unified the
code while keeping the word ambiguous. Adopting a protected package is correct: it belongs
in your file, and deleting that line is refused (V.26).

## II.10 The guard — nine refusals, one function

| | |
|---|---|
| `protected_packages` | never remove this |
| `unprotected_packages` | …unless I say so. **Wins over everything, including OS-essential** |
| OS-essential | never remove what the OS says is load-bearing |
| `max_removals` (default **20**) | never remove more than this at once |
| `max_installs` (default **unset**) | never install more than this at once |
| `deny_packages` | never install this |
| `pinned_only` (default **off**) | never install anything without an explicit `@version=` |
| `require_snapshot` (default **off**) | never change anything when no snapshot can be taken |
| `deny_vulnerable` (default **off**) | never apply when `audit` reports a managed package vulnerable |

All in `[guard]` in `preferences.toml`. One decision function. **Every removal path calls
it** — sync, `absent:`, expiry, `purge-unmanaged`, `clean`, shell exit, `uninstall`. The
last three also gate *installs* and *changes*, so the install paths call it too.

**A confirmation asks; a refusal says no.**

| | `-y` |
|---|---|
| sync shows the plan and asks | **skips** |
| `max_removals` exceeded | **cannot skip.** `--allow-mass-removal` |
| `max_installs` exceeded | **cannot skip.** `--allow-mass-install` |
| hook script new or changed | **cannot skip.** `linix lock` |
| protected / OS-essential | **nothing overrides** |
| `purge-unmanaged` | **cannot skip.** Typed confirmation |
| `pinned_only` / `require_snapshot` / `deny_vulnerable` | **cannot skip.** They are refusals (V.43) |

**The plan always leads with the counts** — not a threshold, not a warning, just the plan
being readable:

```
Plan: install 30,207 · remove 0 · upgrade 3
  30,102  re:^lib
      98  apt
       7  cargo
```

## II.11 `purge-unmanaged`

- **The guard is a RATIO, not a count:**
  ```
  LiNix manages 3 packages.
  This will remove 576, including python3, libc6, and bash.
  That looks like you haven't adopted this machine yet.
  Run `linix adopt` first, or --i-really-mean-it if you're sure.
  ```
- `max_removals` does **not** apply (it catches accidents; this is deliberate).
  `protected_packages` and OS-essential **always** apply.
- **Snapshots first**, automatically. **If none is available, say so loudly** — *"there is no
  undo for this"* is the most important sentence this command can print.
- **Shows the whole list.** 576 packages is 576 lines. The pain is the feature.
- Docs state the residual risk in these words: adopt is an estimate; if it missed something,
  this deletes it.

## II.12 Hooks and the supply chain

**The lock is the approval.** `locks/` records each hook script's hash. Hash mismatch →
**stop**:

```
module `fonts` (from github:x/y) changed its after_install script since you approved it.
  was: sha256:a3f1…   now: sha256:9c2e…
Run `linix lock fonts` to see the new script and approve it.
```

**Hash everything, including your own scripts.** One rule, no exceptions.

**`plan` shows the trust, before anything happens:**

```
module `fonts` (github:x/y)
  adds repository  ppa:fonts/testing
  runs script      after_install: ./setup.sh   [approved]
module `dev` (local)
  runs script      after_install: ./build.sh   [CHANGED — needs approval]
```

## II.13 History

**Git is your intent. Snapshots are your machine.** Two jobs, two mechanisms, neither
pretending to be the other.

**A generation IS a git commit. LiNix commits only on a successful sync** — so every commit
in your history is a state your machine **actually reached**.

- `git log` = where your machine has been.
- **`git diff` and `linix plan` are the same question.**
- Rollback can never take you somewhere that never worked.

**Order: snapshot → apply → commit.** On failure, restore the snapshot and don't commit —
files and machine agree, because the snapshot brought `registry.json` back with it.
**Tag the snapshot with the commit hash.**

**Rollback = `git checkout` + `sync`.** The registry is always current; its history is not
stored, because declaration + convergence reproduces it. **There is no generation format.**

**Snapshots are a preference**, default on if the machine can do it (btrfs, ZFS, or
Timeshift). Retention prunes — **one engine** (`retention`), not two.

**No commit algebra.** Git covers what's real:

| you want | git |
|---|---|
| union of commits | `merge` |
| take that one change | `cherry-pick` — "roll back but keep the jq I added" |
| undo that one thing, keep the rest | `revert` |
| chained and nested | branches |
| **intersect of commits** | **nothing. No such operation, no use case found** |

**Integrity is `git commit -S`.** LiNix checks that git says the commit is signed, and by
whom. **`locksig.rs`, `.linix-lock.key`, and the fail-open branch are deleted.**

## II.14 Version pins — precedence

1. **`@version=` in a module** — you wrote it. **It wins.**
2. **`locks/`** — generated; fills in everything you didn't pin.
3. **Nothing** — whatever's current.

A hand-written pin disagreeing with the lock is **not an error** (today it fails the run).
You wrote it, it wins, LiNix regenerates the lock to agree and says so.

## II.15 Regex

**`re:` prefix. Live by default, lockable when you want it frozen.**

**The lock file IS the switch.** Entry in `locks/` → frozen. No entry → live, re-evaluated
every sync. `linix lock texlive` freezes one; delete the entry and it's live again.

**`plan` shows which is which:**
```
re:^fonts-      live    1,043 packages today
re:^texlive-    frozen  312 packages
```

**(measured)** `apt:re:^python3-.*` → 4,447. `apt:re:^lib` → 30,207.

**Residual hole, accepted:** `texlive-foo` renamed to `tex-foo` silently drops one package.
One package, recoverable, snapshot has your back.

## II.16 Everything is a line

| Today | Becomes |
|---|---|
| `linix repo add` (**stores nothing**) | `repo:ppa:deadsnakes/ppa` |
| `linix shim jq --source cargo:jq` (**`--source` discarded unread**) | `shim:jq@source=cargo:jq` |
| `linix hold jq` (machine-local `registry.json`) | `apt:jq@hold` |
| hooks table in config | `apt:nginx@after_install=./setup.sh` |
| `linix schedule add` (**writes config**) | a line in `schedules` |
| `@lease=2h` (**inert today**) | `apt:jq@expires=2026-07-17T14:00` |
| `remove --temp` (**loses to sync**) | `absent:apt:jq@until=…` |
| `bloatware.txt` | `absent:apt:libreoffice` in a module |

A repo and the package needing it are **one fact**:
```
module python-latest {
  repo:ppa:deadsnakes/ppa
  apt:python3.12
}
```

**Expired lines linger.** LiNix must not rewrite your files. It mentions them, **naming the
exact file and line** — never vaguely. Only the dated line is dead; the undated one is doing
real work and must stay.

## II.17 Deleted

**Commands:** `prune` · `orphans` · `clone` · `migrate` (→ `adopt`) · `remove` (→
`uninstall`)

**Flags:** `-g` / `--groups-dir` · `--no-global` · `--allow-regex-expansion` ·
`--backend` on removing commands

**Syntax:** `group:` · `include:` · `host-*.txt` · `_active_profiles.txt` · `local.txt`'s
special status · `-vim` in modules

**Config:** `[groups]` · `[hostname_packages]` · `[managed_files]` · `[hooks]` ·
`[schedules]` · `backend_priority` · `enabled_backends` · `hostname_backends` ·
`default_backend` · `prune_on_sync` · `prune_scope` · `purge_orphans` · `cache_ttl` ·
`confirm_destructive` · `protect_imperative` · `remove_bloatware` · `timeshift_path` ·
`max_parallel` · `config.snapshots` · `github_token` (→ env)

**Files:** `keep.txt` (→ `forget`) · `policy.toml` (→ `[guard]`) · `bloatware.txt` (→
`absent:`) · `.linix-lock.key` · `locks.json` (→ `locks/`) · `ghosts.json`

**Code:** `locksig.rs` · the generation format · `ManifestArchive` · `quick()` ·
`ScopedFilter::None` as a spare-everything switch · every legacy branch

---

# Part III — The work

## What already exists on branch `v6`

Four stages of the old plan are committed. **Read this before deleting anything.**

| | commit | fate |
|---|---|---|
| **Stage 1** — the guard, backend manual-listing labels, apt `showmanual`, conda `--from-history`, essential parsing, `unmanage` | `47f82b6` | **Keep.** Becomes Phase 3's foundation. The `ManualListing` taxonomy and `guard::protection_of` are the right shape. |
| **Stage 2** — `Migrator::discover()`, one crawl shared by migrate and audit, `manual_source()`, atomic manifest write | `9847544` | **Keep.** Becomes `adopt` (II.9). **Except** its protected-skip — see II.9, E7. |
| **Stage 3** — harness config isolation, the `okf` coverage ratchet, the JSON-check fix | `d1b1edc` | **Mostly superseded.** The harness is rebuilt in Phase 5. Keep the isolation and the ratchet idea. |
| **Stage 4** — the `-g` overlay model, `wish_dirs()`, `config_root()`, `is_reserved_manifest` | `fb9f08c` | **Thrown away.** Phase 0 deletes `-g` entirely. This is real work that this design discards, knowingly (V.1). |

**Stage 4 is a deliberate write-off.** It correctly fixed `-g` by making it additive; the
new model deletes the flag instead. Do not try to preserve it.

## Phase 0 — Delete

> **⚠ Marked ✅ elsewhere in this document. It is not done** (audited 2026-07-17, twice;
> Part VII). The `-g` *flag* is gone. **`keep.txt` and `_active_profiles.txt` are now genuinely
> dead** (Phase 2e/2f); `groups_dir` (≈51 refs, was 84), `prune` and `migrate` (606 lines) are
> still live, and **`local.txt` still has readers** — `insight.rs:418` `line_declares`, which
> Part VII wrongly recorded as deleted.
> **The reason it matters is in this section's own first line:** *do this first so nothing is
> carefully ported that was about to be deleted.* That is now happening.

**Pure subtraction. Nothing new can break. Tests stay green except those testing deleted
features.** Do this first so nothing is carefully ported that was about to be deleted.

Delete everything in II.17. Delete the ~884 marketing comments. Delete every legacy branch
(`generation.rs` bare-filename keys, the `<name>/`-directory profile form).

**Exit:** `cargo test` green. Codebase measurably smaller. Report the line count removed.

## Phase 1 — One parser and the grammar

> **⚠ Marked ✅ elsewhere in this document. Half done** (audited 2026-07-17, Part VII).
> **The grammar is built and it is good. The unification never happened:** `grammar/statement.rs`
> was added *alongside* the other parsers rather than substituted for them. The bullet directly
> below is the unmet one — it is a *replacement*, not an addition, and the ✅ was awarded for the
> addition. **Re-audited 2026-07-17: it is now three skippers, not six** (`insight.rs:428`,
> `manifest.rs:90`, `main.rs:1378`) — **and the bullet's own citation has rotted: `resolver.rs:212`
> no longer parses anything**, because Phase 2d rewired it onto `model::Resolver`. The count in
> that bullet is wrong in the direction this document never errs in — **the tree got better and
> the doc did not notice.** Do not read this as licence to trust it; read it as the tripwire
> working.

**C13 and the grammar are one job, not two.** The grammar *is* the parser; unifying five
parsers against the old grammar just to rewrite them is work done twice.

- One `backend:name` parser. **(re-measured 2026-07-16: EIGHT exist, SIX skip backend
  validation)** — including `resolver.rs:212`, the one that builds every `PackageSpec`.
  Only `split_removal_target` and one inline site at `main.rs:647` consult the registry.
  Every new prefix (`absent:`, `repo:`, `shim:`, `schedule:`, `re:`) is a thing a
  non-validating parser reads as a backend name. *(The first draft said five and three.)*
- Reserve `re` against the onboarder's custom backends.
- `{ }` blocks. Header decides body kind (keyword → lines, declaration → options).
- Comments: whole-line, trailing on statements, **never inside block values**.
- Options: short form (no commas), block form (verbatim to EOL), repeated key = list.
- `@2.0` → error. `@requires=bar` (bare) → error.
- **Unknown line → error**, naming file, line, and what was expected.

**Exit:** unit tests for every grammar rule above, including every error case.

> **Three II.2 rules have no implementation (audited 2026-07-17). Every one of them has a
> passing test file next to it — they are untested, not failing, which is rule 11's whole
> point: the Exit above names a *surface to cover*, and these are the parts of the surface
> nobody thought to cover.**
>
> - **`@until` "on `absent:` only" (II.2:273) is not enforced.** `grammar/statement.rs:407` has
>   a comment stating the rule and then `Ok(())`. `apt:jq@until=2026-01-01` parses clean. **The
>   comment reads exactly like a check.**
> - **II.2's option-key table is not a whitelist.** `options.rs:124` `is_key` says so out loud
>   (*"not a judgement about which keys exist — II.2's table decides that"*) and **nothing
>   downstream makes that judgement.** `apt:jq@versionn=1.6` is accepted silently — a typo'd
>   key, which is the same defect class as a typo'd package name that II.2 exists to refuse.
> - **`link:` cannot take a Windows path, or any source containing `\ | & (`.** The expression
>   check (`statement.rs:153`) runs **before** the `link:`/`absent:`/`repo:`/`shim:` branches
>   (`:167-206`), and `profile_expr.rs:57` calls any line containing those characters an
>   expression. So `link:C:\Users\me\.vimrc` parses as `Statement::Expr`. **This is II.4's set
>   math eating II.2's statements** — the two grammars overlap and precedence decided it
>   silently. Untested in either direction.
>
> Also: `Exclude`/`Intersect`/`Subtract`/`Expr` exist (`statement.rs:79-90`) but are **not in
> II.2's statement list**, while `statement.rs:66` calls that list *"II.2's full list"*. And
> `schedule:NAME` "(only in `schedules`)" has **no file-context check** — it parses in a module.

## Phase 2 — The model (the cliff)

**Cannot be split.** Everything above the seam breaks at once. Do not run two models behind
a flag — that is the "two ways to do one thing" disease, done to ourselves.

- The layout (II.1). `modules/`, `profiles/`, `active`, `priority`, `schedules`, `locks/`,
  `preferences.toml`.
- The resolver (II.7): profiles choose, lazy parsing, conflicts are errors, the layering
  rule, dated lines.
- Profile set algebra, resolved at read time. **No `_active_profiles.txt`, no
  materialization.**
- `PackageSpec` gains **present/absent**. That is the only new thing the desired-state map
  can't already carry.
- Ordering phases in the planner: repos → index refresh → packages → dependents.
- The command surface (II.8).

**The seam:** everything upstream produces `HashMap<backend, Vec<PackageSpec>>`; everything
downstream consumes it. `src/backends/` (11,193 lines), `src/core/` (4,499), and
`src/parsers/` (2,275) — **~45% of the codebase — never notice this happened.**

**Exit:** the harness green on one distro.

## Phase 3 — The guard

- 16 → 9 (II.10). One decision function. *(The first draft said five, then six. The owner
  chose to keep all three orphaned `policy.toml` rules rather than delete them — V.43.)*
  **Audited 2026-07-17 — the starting point is not what II.10 implies.** Four of the nine are in
  `guard.rs` (`protected_packages`, `unprotected_packages`, OS-essential, `max_removals`); four
  are in a **separate `Policy` struct** (`app/policy.rs`) loaded from `groups_dir/policy.toml` —
  **a file II.17 deletes** — with `require_snapshot`/`deny_vulnerable` enforced ad-hoc in
  `main.rs:3176`/`:3181` rather than in any guard; and **`max_installs` does not exist anywhere
  in `src/`**. `policy.rs:25` also has a **tenth rule the spec never mentions**
  (`allow_backends`). **"One decision function" is the work, not the summary:** today there are
  three (`guard::protection_of`, `guard::inspect`, `Policy::check_specs`), and the real ceiling
  is `Objection` (`guard.rs:114`), which has **two variants** — `Protected` and `TooMany`. Nine
  refusals cannot be expressed by a two-variant verdict; **that enum is the first thing to
  change.** `--allow-mass-install` (II.10:578) does not exist either.
- **Every removal path calls it.** Today's misses: `uninstall` (C1), leases and `absent:`
  (C3), ghost-shell exit (C8), `clean`.
- One lease-expiry implementation (C9 — two exist today with different semantics).
- The ratio check and `purge-unmanaged` (II.11).
- `unprotected_packages` must beat OS-essential (B3 — the code clears the config rule, then
  falls through to the OS check, which fires anyway).

**Exit:** a test per removal path proving the guard fires.

## Phase 4 — Locks and git

- `locks/` (II.6): version, resolved backend, frozen regex expansions, hook hashes.
- Commit on successful sync only. snapshot → apply → commit. Tag the snapshot.
- `git checkout` + `sync` = rollback. Delete the generation format.
- `linix diff COMMIT COMMIT` in packages, not text.
- `bundle` = `git bundle` + artifacts + registry, **honest per-backend about what can't be
  bundled**.
- One retention engine.

**Exit:** an air-gapped container restores from a bundle, or bundle says why it can't.

## Phase 5 — Harness and docs

- Rebuild the harness for the new model.
- **G2:** 104 of 245 assertions are `soft` and cannot fail. Convert or register as debt.
- **G3:** teleport, adopt, shim, cockpit, undo are effectively unverified.
- **H2:** two error-swallows on safety paths — `sync/mod.rs:463` (failed rollback-remove
  goes unreported), `shell/mod.rs:126` (dropped state write).
- **F4:** `--help` asks the registry for the backend count. The README line is generated.
- **F1:** `network_timeout_secs` — **honour it** (today every consumer applies an
  undocumented `.max(10)` floor, so setting 5 silently gives you 10).
- **F1:** `max_parallel` — detect the core count.
- **F1:** the generated `priority` file carries its reason in a comment (V.14).
- **F5:** fix the false doc comments.
- **P6** goes in `CLAUDE.md`.

## Phase 6 — The five containers

`DISTROS="ubuntu fedora arch alpine tools" ./docker/integration/run.sh jq`

Owed from the last sprint; not run since Stage 2.

---

# Part IV — Verification

**The specific proofs, on the ubuntu image:**
- After `adopt`, the registry holds ~103 packages, not ~579, and does **not** contain
  `libperl5.38t64`.
- `python3` is still installed at the end of the run.
- A large removal is refused without `--allow-mass-removal`.
- `purge-unmanaged` with an unadopted machine is refused by the ratio check.

**Grammar:** a test for every error in Part II.2. Each must produce an error, not a guess.

**Resolution:** two modules declaring the same package differently → error naming both files.

**Guard:** one test per removal path in Phase 3.

**Hooks:** a changed script hash refuses under `-y`.

---

# Part V — Why

> **Do not change a Part II rule without reading its entry here.** Each is the scar of a
> real bug.

**V.1 — Why `-g` died.** `Config::groups_dir` meant two things: the wish-list folder, and
the anchor for `locks.json` / `keep.txt` / `local.txt` / profiles. `-g` moved both, while
`registry.json` — the ownership record — never moved. So `plan -g /B` read /B's one package
against an ownership record claiming 579, called 578 of them drift, and purged the machine.
`-g` is gone because "which folder" stopped being a question anyone asks: files are storage,
modules are the unit, profiles choose.

**V.2 — Why profiles choose and modules hold.** It's the one sentence that explains the
whole system. The moment profiles hold things or modules make choices, it stops being true.
A module can never reference a profile (the layering rule) because otherwise "what does
`editors` contain?" has a different answer depending on what you activated — the library
cannot depend on the app.

**V.3 — Why a profile may still hold packages.** Decided knowingly against V.2's tidiness,
because `--into Work` is a real want. The cost is real: those packages are unshareable
forever, and you find out the day you want to share them.

**V.4 — Why `group:` and `include:` died.** `group:editors` pointing at a file was **already
a no-op** — the resolver seeded every `.txt` unconditionally, so the file was loaded before
you named it. It looked like opt-in and wasn't, which taught people a wrong model of how
LiNix decides things. `include:` strictly superseded it.

**V.5 — Why conflicts are errors.** Files were read in filesystem order and first
declaration won. `a.txt: jq@1.6` vs `b.txt: jq@1.7` was decided by the disk. Sorting the
read order only makes the wrong answer deterministic.

**V.6 — Why `keep.txt` died.** It lived in the groups folder and ended in `.txt`, so the
resolver ate it: *"never remove firefox"* also silently meant *"install firefox"*. It was
held back by a hardcoded one-element denylist. **Separate by location, not by denylist** —
and `forget` gives people the thing they actually wanted, which was a way to make LiNix let
go.

**V.7 — Why `absent:` is the one exception to "only removes what it manages".** Because you
named it. Everything else LiNix touches, it owns. `absent:` is you reaching outside that,
deliberately, by name. It stays a line rather than a file because a file can't be turned off
per profile, can't be shared, and puts LiNix's bookkeeping back in a folder you author.

**V.8 — Why blocks use `{ }` and not `( )` or `end`.** `( )` is already the grouping operator
in profile math — same character, two meanings, the trap we removed from `include:`. `end` is
clumsy. "Pick your own delimiter" means nobody can read anyone else's files.

**V.9 — Why block values are verbatim and `#` doesn't comment inside them.** Fail loud. If
`#` commented there, `after_install = curl -H "X: #tag"` silently truncates and runs the
wrong command. The other way, `version = 1.6 # my pin` gives a version the parser visibly
rejects. **You reached for the block form precisely because you needed a value the short
form couldn't hold. Verbatim is what you asked for.**

**V.10 — Why no quotes.** `"` needs `\"` needs `\\` needs a newline rule. The block form
makes the problem stop existing rather than giving it a rule.

**V.11 — Why the extension is cosmetic.** Nothing is active unless a profile names it, so
`use editors` against a misnamed file says *"no module named `editors`"* with a list. **The
reference is the safety net**, not the extension.

**V.12 — Why adopt takes manual-only.** Not because 579 is a big number. **Declaring a
dependency breaks dependency management.** Put `libgpm2` in a module and you've declared it,
so LiNix keeps it forever; remove vim and it stays, because apt says "orphan" and your file
says "I want this" and the file wins. Monday's bug was claiming ownership of a set that was
never LiNix's.

**V.13 — What "estimate" means.** apt records that something was **explicitly requested** —
not **who** requested it. Canonical's installer marked ~90 packages manual at image-build
time; they are indistinguishable from the `apt install vim` you typed. There is no field for
"a human, on purpose." **(measured)**

**V.14 — Why the priority order.** Most of the current 10-backend order is **meaningless** —
apt, pacman and dnf never coexist. The order that decides something is **system manager vs
language manager**: if both apt and cargo have `ripgrep`, the **system one wins**, because
your distro maintains it and updates it with everything else. Language managers are for what
your distro doesn't carry. That also explains pip last: it installs into your system Python
and can break it. *(uv and pipx being absent from the order is simply a bug.)*

**V.15 — Why `priority` also means "enabled".** One list, one question: *which package
managers does this setup use, and in what order.* It replaces four settings for one fact
(`backend_priority`, `enabled_backends`, `hostname_backends`, `default_backend`) of which
only two merge today. An explicit `snap:foo` failing when snap isn't listed is a feature: it
catches typos and makes your backend set declared rather than inherited.

**V.16 — Why bare names get locked.** LiNix *probes* — "does apt have ripgrep?" So `ripgrep`
lands on cargo today, Ubuntu adds it tomorrow, and the same unchanged line resolves to apt:
LiNix uninstalls from cargo and installs from apt because a repo you don't control changed.
**The bare name is the question; the lock is the answer.**

**V.17 — Why regex is live by default.** "Give me all the fonts, including ones that don't
exist yet" is real. Mandatory locking turns a living pattern into a frozen list and defeats
the point of writing a pattern. **The lock file is the switch** — that's how every lockfile
already works.

**V.18 — Why regex matches names, not meaning.** `photo*` finds `photocollage`,
`photoprint`, `photoqt` — and misses `gimp`, `darktable`, `krita`, `rawtherapee`,
`shotwell`, `digikam`, `inkscape`: every actual photo editor. Real prefix *families* are the
good use (`texlive-*`, `fonts-*`). Debian's own answer to a family is a **metapackage** —
someone's judgement rather than a naming coincidence — and better where one exists.

**V.19 — Why `max_removals = 20` works and `max_installs` has no default.** **20 is more
than a person removes on purpose** — calibrated against human behaviour, so a plan removing
50 is wrong at any scale on any machine. **Installs have no equivalent ceiling: the biggest
install you'll ever do is the correct one** (a fresh machine). So `max_installs` exists but
defaults to unset — the number is yours, for your reason. *(Rejected: screen height — the
same command would behave differently on different machines. Rejected: a ratio — a fresh
machine's ratio is undefined.)*

**V.20 — Why the ratio catches Monday and a count doesn't.** On Alpine, `adopt` correctly
took 14 packages and a mis-scoped `prune` scheduled all 14 for removal — **under the count
limit, none protected, all things you'd cry about**. The count misses it on small machines.
**Manage 3, delete 576 → you have made a mistake, on every machine, always.**

**V.21 — Why `purge-unmanaged` is a command and not a mode.** **Sync is then never
dangerous** — not "safe by default", but safe permanently. No setting anyone can flip,
inherit, or copy from a dotfiles repo makes a routine sync delete something it didn't
install.

**V.22 — Why `-y` cannot skip a refusal.** Every CI job and every script passes `-y`, and an
unattended run cannot notice a machine being dismantled. **`-y` means "don't ask me". It has
never meant "ignore your safety rails", and every place it currently does is a bug.**

**V.23 — Why `confirm_destructive` died.** In a declarative system, **deleting a line is the
confirmation.** You said what you wanted; asking whether you meant it is asking twice. And
the setting named after removals gated a module-file overwrite (not a removal) while missing
both `prune` and `sync`.

**V.24 — Why the plan always leads with counts.** **A warning that only fires sometimes is a
mechanism that can be miscalibrated. A summary that's always there can't be.**

**V.25 — Why the 16 protections became 5.** **Eleven of them were never protections — they
were declarations wearing a protection costume.** "Don't remove this, it's leased" →
`@expires=`. "…you installed it imperatively" → it's in the `imperative` module like
everything else. "…it's held" → `@hold`. "Do remove this, it's bloatware" → `absent:`. Each
existed because there was **no way to say the thing directly**, so someone bolted an
exception onto the removal path instead. `protect_imperative` is the clearest: it exists
*purely* to stop drift-pruning deleting `linix install`-ed packages, because they lived in
`local.txt`, which `-g` could move out from under the registry. **Someone met Monday's bug,
understood the symptom exactly, and patched it with a flag.** Not one behaviour was deleted;
they moved to where they were always trying to be.

**V.26 — Why protection is a refusal, not a declaration.** Everything else is a statement of
intent ("I want this"). Protection is "I will not do that, and there is no flag." It doesn't
care whether the package is managed, declared, adopted, or predates LiNix. That's why it
lives in preferences and not in a module — and why deleting a declared `apt:python3` line
makes LiNix refuse until you unprotect it.

**V.27 — Why hooks are lines despite the supply chain.** `use` is **already** a trust
decision: a `repo:` line in someone's module means they can ship you any package with any
script in it. Hooks make that road shorter, not different in kind. **The lock is the
approval** — because you approve a script once and they edit it three months later, which is
how most npm incidents actually worked: the malicious version was never the one anyone
reviewed. **Hash everything, including your own scripts**, because "did I write this?" has
no clean answer once you've cloned your own repo onto a second machine — and the friction
that catches you editing `setup.sh` is the same friction that catches a teammate's `git
pull`.

**V.28 — Why schedules got their own file.** `active` answers exactly one question: *what is
this machine set to right now?* A schedule is written once and forgotten — a fact, not a
switch. An active-list for schedules would invent a state that needn't exist ("defined but
off"), so you'd check two files for one fact. And the separate file means a cron job can't
arrive via `use` at all. **Door left open, deliberately unbuilt:** "sync nightly when I'm in
Work" — a `schedule:` line can live in a module and be selected by a profile; the grammar
already allows it.

**V.29 — Why `@requires` survives.** **(verified, `planner.rs:407-426`)** `spec.requires`
becomes a real `graph.add_edge` — install **ordering**. A module is a *set* and says nothing
about order. `@requires` is the one thing modules can't say. It matters only for things
outside a package manager (a `.deb` from a URL, a GitHub binary) — things with **no one to
ask**. apt's own dependencies are ordered for free at `planner.rs:427`.

**V.30 — Why git is the history.** **LiNix commits only on a successful sync, so every
commit is a state your machine actually reached** — not one you asked for. `git log` is
where your machine has been; `git diff` and `linix plan` are the same question; rollback can
never take you somewhere that never worked. And the registry needs no history, because
declaration + convergence reproduces it.

**V.31 — Why no commit algebra.** Set math works on profiles because they're choices you're
making *now*. Commits are moments that already happened, and "the union of March and today"
isn't a machine anyone asked for. Git covers what's real. **Intersect of commits does not
exist in git and no use case was found** — twenty years of git not having it is evidence.

**V.32 — Why lock signing died.** **Signing one file in a folder of unsigned files protects
nothing.** Anyone who can edit `locks.json` can edit your modules — they'd change `apt:jq`
to `apt:evil` and no signature would notice. It guards one door in a building with no walls.
Ours was `sha256(key + "|" + text)` — a construction cryptographers warn against — compared
with `==`, which leaks timing. And **appearance is worse than nothing, because you stop
looking.** `git commit -S` signs everything, with real crypto, verified by a tool that's been
attacked for twenty years.

**V.33 — Why `clone` died.** It copied **the installed set, not the intent** — you got a
machine with the same packages and no idea why. `git clone && linix sync` gives the intent,
the history, the pins, and the ability to change it afterwards.

**V.34 — Why `prune` and `orphans` died.** sync removes drift by definition, so `prune` is
sync with the install half amputated. "Prune" meant four unrelated things; deleting the
command leaves exactly one meaning ("delete old history") for the first time. `orphans`
shows what sync would remove, which is `plan` — and its message named two commands and
described neither.

**V.35 — Why `--backend` is refused on removals.** A scoped removal is Monday's exact shape:
**you narrow what LiNix looks at without narrowing what it owns**, so everything outside the
scope looks like drift.

**V.36 — Why `clean` survives.** It's apt's housekeeping, not LiNix's drift, and only apt
knows about it. It goes through the guard because `autoremove` is a mass removal LiNix
didn't plan and has famously eaten desktop environments. It stays explicit because automatic
cleanup is a surprise removal.

**V.37 — Why suspensions survive.** Nearly deleted — "I want this and I don't want this"
smells like a contradiction with a timer. The case that saves it: **"take the game away
until the weekend."** People genuinely do that; nothing else here does it; and once leases
exist, suspensions are the same machinery pointed the other way.

**V.38 — Why times are absolute.** "2 hours" can't work in a file: the machine reading it
next week has no idea when you wrote it. That's exactly why `@lease=2h` is inert today.

**V.39 — Why `install`/`uninstall`/`forget`.** A symmetric pair plus one word that can't be
misread. `remove` and `unmanage` sat one word apart and did opposite things to your disk —
reach for the wrong one and you don't get an error, you get a deleted package.

**V.40 — Why three landing modules.** Provenance ends up in the filename: open
`modules/hooks.txt` and see exactly what got in behind LiNix's back. One `local.txt` mixes
them and forgets which was which.

**V.41 — Why "detected, not configured".** LiNix should not be *told* you have btrfs; it
should look. Not told you have four cores. Almost every "local fact" in `config.toml` is
something LiNix could work out in a second and instead asks you to maintain by hand, forever,
on every machine. **That is not configuration, it's homework.** (And `max_parallel` is
overwritten at `sync/mod.rs:296` anyway, so the setting is already a lie.)

**V.43 — Why the guard has nine refusals and not five.** The first draft said five (then
listed six). It was written before anyone re-read `policy.toml`, which held five rules and
was marked in II.17 as moving to `[guard]`. Two of them had somewhere to go —
`deny_packages` was already in the list, and `allow_backends` is what the `priority` file
means (V.15). **The other three had nowhere, and "delete" was never decided — it was
overlooked.** `pinned_only`, `require_snapshot` and `deny_vulnerable` are all exactly the
shape V.26 defines: not "I want this" but "I will not do that". They are refusals, so they
live where refusals live, and `-y` cannot skip them for the same reason it cannot skip any
other (V.22). *Corrected knowingly against the headline: a wrong number in a document is
cheaper than three deleted safety rails. If a rule here ever stops being a refusal and
starts being a preference, that is the signal it does not belong in `[guard]`.*

**V.46 — Why set math costs a package its module name, and why `include` died.** *(Decided
2026-07-17, during Phase 2f. II.4 required set math and nothing implemented it:
`model::profiles::evaluate_expression` had no caller outside its own tests, and the only
working implementation was `compose()` in the old `app/profile.rs`, over flat strings.)*

**The shape does not fit, and pretending otherwise is the bug.** Resolution is
`profiles → the modules they reach → the packages in those modules`. Set math breaks that
chain: `(Work | gaming) & security` is **an intersection of package sets**, and there is no
module whose contents are that intersection. So a profile using set math resolves to packages
directly rather than naming modules.

Making `&` operate on module *names* was the alternative, and it answers a different question
than the one asked: the intersection of `{editors}` and `{security}` is empty even when both
hold `vim`. Inventing a synthetic module to hold the result was the other, and it names a
module that does not exist on disk, so `upgrade --module` would match something nobody can
open.

**The predicted cost turned out not to exist, and that is worth stating plainly because this
document predicted it wrongly.** The first draft of this entry said set math costs a package
its module name. It does not: the implementation maps expression atoms back to **the
statements they came from**, not to strings, so a package that survives an intersection still
carries its `Origin` — its file, and therefore its module. `upgrade --module editors` finds
`vim` through an `exclude`. There is a test (`a_package_surviving_set_math_still_knows_its
module`). The only lines that get profile scope alone are ones written in the profile itself,
including a bare package atom inside an expression — which is correct, because that line
really is in the profile. **Keep mapping back to statements. Mapping back to strings is what
would make the predicted cost real.**

**`include` died because `use` already is it.** II.4 listed `include`/`exclude`/`intersect` as
the three directives while II.2 listed `use NAME` as the way to reference a module or profile
— and for the union case those are the same operation with two names, which is the exact
"two ways to do one thing" disease this design exists to cure, sitting inside the spec. `use`
wins: it is II.2's word, it is the one modules use too, and one word for "bring this in"
everywhere beats two. `include` is an error that says so.

**V.42 — Why the comment rule.** This codebase has been touched by many AIs, and this is what
that leaves behind: models narrate what they just wrote and congratulate themselves for it,
because that reads like effort, and each one looks fine on its own. The repo already proves
the rule works — `core/manager.rs:86-93` explains *why* the `tracks_manual` gate exists and
what happens if it's wrong; `generic.rs:363-370` explains in nine lines that choco lists
Title-case "Wget" for install-id "wget" so `remove` silently no-ops, and why the fix must be
Windows-only because npm has `socket.io`. **Those two are worth more than the other 137
combined, and they're the same length.** The cost of the rest is that **they trained everyone
to skip** — the reason 32 comments in this repo are outright false, each of which someone read
past. *(The first draft's example, `audit()` documented as "a **destructive** Discovery cycle …
without generating files or acquiring state", has since been fixed in the code and now reads
correctly. The measured 32 are the ones that remain.)*

**V.44 — Why `activate` writes a list and there is no `-r`.** The file is the state, so a
command that activated *without* writing `active` would be a second place the answer lives —
the exact defect `-g` and `keep.txt` died of (V.1, V.6). Set, add, subtract, because those
are the three things you do to a list. **`deactivate` rather than `activate -r`** because
`install`/`uninstall` already settled that the opposite of a verb is a verb (V.39), and a
flag that silently inverts a command is how you delete something at 2am by leaving off one
character. The empty list is the one refusal: `linix activate $PROFILE` with `$PROFILE`
unset would otherwise read as *"turn everything off"* and be perfectly valid. The guard would
catch it (V.19) — but the guard is for decisions you meant, and this one nobody means.
**`activate NAME…` still overwrites `when` blocks without asking**, and that is not an
oversight: it is the set form, it sets, and a form that quietly kept part of the old file
would leave the machine in a state you did not type. The file is in git; that is what git is
for (V.30). **It does not ask and it does not stay quiet** — it names each block it removed.
*Asking and reporting got argued as one thing and they are not: the case against a prompt is
that overwriting the list is the command's own job (S6), and none of that is a case for
hiding what the job did.*

**Why `deactivate` reaches into a `when` block when `activate -a` does not** *(decided
2026-07-17, after the first draft of this entry said the opposite)*. The first rule here was
that LiNix never edits a block — a block is something you wrote — so `deactivate Travel` would
remove the top-level line and report *"it is still activated by the `when` block on line 4."*
**That sentence is the argument against itself.** It is a command named "deactivate"
announcing that it did not deactivate. **A verb that reports the state it failed to reach is
the `-g` disease in miniature: the name says one thing, the file says another, and you find
out later.** So it removes the name wherever this host would read it, and the empty block goes
with it.

**The asymmetry with `activate -a` is real and it is not a compromise: adding has a choice of
where to put the name, removing has none.** `-a` appends at the top level because a block is a
rule you wrote and a new name has no business joining it — there is a right answer and it is
"outside". `deactivate` gets no such freedom; the name is where it is, and the only way to
leave the block untouched is to not do the job.

**And why it stops at blocks that do not apply to this host.** Not caution — the same rule,
read carefully. `deactivate` turns off what is on; on the desktop, `when host == laptop {
Travel }` has nothing on, so there is nothing to turn off, and removing the line would be a
different command (*"never activate Travel anywhere"*) that nobody typed. **`active` is a file
you commit and share (V.30), which makes "edit it wherever the name appears" a way to change a
machine you are not sitting at from one you are.** The blast-radius reasoning is V.22's, and
it lands in the same place: **the refusal is cheap and the mistake is not.** It says why, and
names the line, so the hand-edit is one keystroke away for the person who did mean every
machine.

**V.45 — Why a cycle is an error and not deduped.** If `active` were the only consumer you
could visit each profile once and move on, because union doesn't care how many times it sees
a name. But profiles have `&`, `\` and `-` (II.4), so `Work include Gaming` /
`Gaming exclude Work` has no answer to settle on — not a redundant answer, **no answer**.
Deduping picks whichever order the resolver happened to walk in, which is V.5's defect
wearing a different hat: files were read in filesystem order and first won, and the fix was
to stop guessing and say so. Naming the whole loop instead of the last edge is II.2's rule —
the error names the file and the line — and it is the difference between *"there is a cycle"*
and a user who can see which of the three lines they meant to delete.

---

# Part VI — Bugs

## VI.1 Killed by this design — no work needed

- `-g` replaced the wish list while the ownership record never moved → phantom drift → the
  purge. **(C2, C4, C5, C6, C14, D1, D2)**
- `keep.txt` read as a wish list. **(E2, E10)**
- Undefined manifest read order. **(C4)**
- `upgrade --profile` matches zero packages, always — builds `manifest:work`, tags are
  `manifest:work.txt`, and per-profile provenance is destroyed at materialize time anyway.
  **(verified)**
- `-vim` in a manifest becomes a package named `-vim` while the README documents it as
  exclusion. **(verified)**
- Generations record absolute paths → rollback writes to a dead directory and reports
  success. **(verified)**
- Generation ids are epoch seconds → two syncs in one second silently overwrite history.
  **(verified)**
- `config.toml` never travels with history.
- "Rollback" means two things depending on flags; `undo` assigns the registry by fiat.
- Manifests stored twice on independent budgets.
- Two snapshot retention engines. **(E4)**
- Every bundle restore is unverified by construction. **(H1)**
- `linix repo add` records nothing → modules are not portable.
- `linix shim --source` is required, documented, and thrown away. **(verified)**
- Holds (machine-local) silently beat signed locks (portable).
- `@lease=` in a manifest is inert. `remove --temp` is undone by the next sync.
- `orphans` message false on both counts. **(C12)**
- "prune" means four things. **(E3)** "orphans" means two plus a dead third. **(E5)**
- `confirm_destructive` gates the wrong thing. **(E12)**
- Any typo becomes a package name.
- `ScopedFilter::None` — an implicit spare-everything switch.

## VI.2 Carried forward — still real work

| | |
|---|---|
| **C1** | `uninstall` never consults protection. `linix uninstall libc6` proceeds → **Phase 3** |
| **C3** | lease and `absent:` removals skip protection, three lines above a drift branch that checks it → **Phase 3** |
| **C8** | ghost-shell exit force-removes with no protection, no guard, no confirmation → **Phase 3** |
| **C9** | lease expiry implemented twice with different semantics; the sweep runs on every state-changing command → **Phase 3** |
| **C13** | ~~five~~ ~~**eight**~~ ~~**NINE**~~ **EIGHT** `split_once(':')` sites outside `src/parsers/` (7 code + 1 comment); ~~six~~ **three** skip validation. ~~**Phase 1** ✅ done~~ — **NOT done.** Phase 1 *added* `grammar/statement.rs:304` alongside the others; it removed none. **Re-audited 2026-07-17: four of the six sites this row named did not exist** (`config/manifest.rs:218` is past the end of a 178-line file). The three live non-validators: **`app/insight.rs:428`, `config/manifest.rs:90`, `main.rs:1378`**. *(`config/parser.rs:199` validates — do not delete it. `model/resolve.rs:491` is a new-model name helper.)* ~~**Blocks II.8**~~ — **that argument rested on `remove_package_from_local`, which no longer exists; S9 is fixed.** → **Phase 2** |
| **B3** | `unprotected_packages` doesn't beat OS-essential; `linix protected` reports the opposite of what the guard does → **Phase 3** |
| **E6** | "unmanaged" has two implementations that will disagree. Resolve as *"what `adopt` would adopt"* — one function → **Phase 2** |
| **E11** | suspension restore implemented twice → **Phase 3** |
| **F1** | `network_timeout_secs` lies (`.max(10)` floor); `max_parallel` detected; `priority` reason in a comment → **Phase 5** |
| **F3** | ~884 marketing comments + **32 false ones** → **Phase 0**. The rule → `CLAUDE.md` → **Phase 5** |
| **F4** | 33 vs 50 backends. **(measured: 41 registration sites)** Compute it → **Phase 5** |
| **F5** | false doc comments → **Phase 5** |
| **G2** | 104 of 245 assertions are `soft` and cannot fail → **Phase 5** |
| **G3** | teleport, adopt, shim, cockpit, undo unverified → **Phase 5** |
| **H2** | `sync/mod.rs:463`, `shell/mod.rs:126` → **Phase 5** |
| **S1** | ~~`reconcile_shims` is never called~~ — **this was false.** `sync` calls `reconcile_all_shims` on every successful run, which calls `remove_shim` for every managed package that is not shimmed. `remove_shim` deleted `~/.local/bin/<name>` by filename alone, with no check that LiNix created it — so a managed package named `jq` made every sync delete the user's own `~/.local/bin/jq`. `~/.local/bin` is shared. **Fixed in Phase 0f**: ownership is now tested (a shim is the linix binary under another name — same file as `current_exe`, or a byte-identical copy). Three regression tests added; they could not exist before because `bin_dir` was private with no injection point, which is why G3 lists shims as unverified. |
| — | `bundle` has no restore code and no end-to-end test → **Phase 4** |
| — | air-gap artifacts need the whole dependency tree, and most backends can't → **Phase 4** |

### Found during implementation

Each verified against the code at the time it was found, with the evidence in the commit
that recorded it. Assigned to the phase that owns the mechanism, not the phase that found it.

| | |
|---|---|
| **S2** | **Age-based snapshot retention is dead for btrfs and ZFS.** Both providers' `list()` hardcode `timestamp: Utc::now()`, so `Snapshot::parse_time()` always returns *now* and every snapshot reads as zero seconds old. `max_age_days` and `RetentionPolicy::keep_days` can therefore never fire; only `keep_last` works. `retention.rs` is correct — the data feeding it is not. A retention policy that silently keeps everything is exactly P3's failure mode → **Phase 4** (one retention engine) |
| **S3** | **Snapshot retention never prunes Windows restore points.** The ownership filter is `id.contains("linix")`, but `WindowsRestoreProvider` sets `id` to a bare `SequenceNumber` (`"12"`) and puts the `LiNix:` marker in `description`. Nothing LiNix creates on Windows is ever reclaimed → **Phase 4** |
| **S4** | **`create_shim` overwrites any existing file at the target without asking.** Deliberate for redeploying a shim, but it silently destroys a same-named binary the user owns in `~/.local/bin`. `remove_shim` was fixed in Phase 0f to test ownership; the create path still does not. Once shims are declared lines (II.16), this is an install that clobbers an unmanaged file, and belongs behind the guard → **Phase 3** |
| **S5** | **`--progress` is a lie.** `#[arg(long, global = true, default_value = "true")]` on a `bool` — clap derives `ArgAction::SetTrue`, so it is always true and there is no way to turn it off. A setting that cannot be changed is F1's shape → **Phase 5** |
| **S6** | **`sync` heals without asking. DECIDED 2026-07-16: it should, and that is what declarative means.** A half-finished transaction is a state nobody declared — it is drift, and removing drift is sync's job. Asking permission to fix it would ask permission to do sync's own job, and would make `sync` mean two different things depending on the answer. So: automatic, NOT a prompt. But it must **say** what it recovered (P3 — every bug here is something that didn't work and said nothing), and a rollback-removal is a removal, so it goes through the guard like every other (II.10) → **Phase 3** |
| **S7** | **A crash left unhealed for 4 hours becomes unhealable.** `Journal::cleanup` reclassifies stale `InProgress` entries to `Abandoned`, and `get_incomplete_actions` (what `heal` acts on) excludes `Abandoned`. So the window to recover a crashed transaction silently closes. The 4h threshold is also a magic number with no stated reason (P5) → **Phase 5** |
| **S8** | **`undo` lies about scope; there is no safety hole. DECIDED 2026-07-16.** What `undo` does: list filesystem snapshots, mount the chosen one read-only, read the `registry.json` *inside* it, diff that against now, show a package-level summary, and on confirmation hand the snapshot to btrfs/timeshift to restore. `FORBIDDEN_PATHS` guards step 3 only — which directory `undo` will read a registry out of, so a crafted path cannot make it parse `/etc/shadow` as JSON. That is a real check doing a real job. Its *name and comment* claim "paths NEVER allowed to be accessed", and restore goes over `/` including all of them. So the defect is the false claim, not the check. **Keep** the check (renamed to say it guards the snapshot-read path); **delete** the global claim; **keep** `undo` (nothing else turns a snapshot into a package diff), but restore must state plainly that it rolls back the entire filesystem before asking. Gating restore on the list would refuse every root snapshot, i.e. delete `undo` by accident → **Phase 3** |
| **S9** | ~~`remove_package_from_local` (`parser.rs:290`) matches a bare target against the BACKEND prefix~~ — **FIXED in Phase 2e, and this row was stale in three ways (2026-07-17).** The function is gone (`grep` empty). The removal path is now `model/edit.rs:378` `matches()`, which parses each line **through the grammar** and compares `d.selector`, never the prefix; regression test at `edit.rs:669` (`npm:typescript` survives, `apt:npm` dies). **It did not "die with `local.txt`" — it died of `edit.rs`, and `local.txt` still has readers** (`insight.rs:418`). ~~→ **Phase 2**~~ **Nothing owed.** *(Both surviving prefix-splitters were checked for this defect shape and do not have it — `insight.rs:429` requires both halves, `manifest.rs:90` matches the name half.)* |
| **S10** | **`cargo test` wrote to the developer's REAL data dir**, and one bad file bricks every command. `TestKernel` (named `linix_hermetic_`) isolated `registry.json`, groups and tmp, but `Journal::new()` hardcoded `safe_data_dir()` — found at 733KB of test noise in `%APPDATA%/linix/journal.json`. Fixed in Phase 2b by injection (`Journal::at`). **The remaining half is real:** `Journal::load_sync` errors on a bad parse -> `App::new` fails -> EVERY command fails, with no message saying which file to delete or how to recover. Failing loud is right (P3); having no way out is not → **Phase 5** |
| **S12** | **`repo:`, `shim:`, `service:`, `link:` and `schedule:` lines parse, resolve, and are then dropped on the floor.** The seam is `HashMap<backend, Vec<PackageSpec>>` and these are not packages, so `Resolver::resolve` collects them into `DesiredState::extras` and `resolve_desired_state` — which returns only `.packages` — discards them. Nothing downstream has ever consumed them. A `repo:ppa:deadsnakes/ppa` line therefore does exactly nothing, silently, which is VI.1's disease with new syntax. Not a regression (the syntax is new in Phase 1) but it must not ship: `sync` warns for now, naming each ignored line and its file. The fix is the ordering phases — repos → index refresh → packages → dependents — which is what `extras` was collected for → **Phase 2** (planner ordering) |
| **S13** | **A bare name and an explicit one were two packages, not one.** `model::resolve` keys the merge on `backend:name`, and a bare `ripgrep` is keyed `?:ripgrep` until something probes it — so `ripgrep` in one module and `cargo:ripgrep` in another never met, never reconciled, and both reached the planner. Found while wiring the seam and **fixed there**: `Resolver::statements()` and `Resolver::collect()` are now separate, the caller probes in between, and `with_bare` hands the answers back so the merge sees real backends. II.7 rule 5 was silently not applying to every bare line → **Phase 2** (fixed) |
| **S19** | **`@lease=2h` still worked by hand, and it was the one option key that could uninstall your package. FIXED.** II.16 retired it — nothing LiNix writes used it — but `StateRegistry::add` still read `options["lease"]` and turned it into a real `expires_at`, and **the grammar validated no option keys at all**, so a hand-written `apt:jq@lease=2h` was silently a package that uninstalls itself, on the `sweep_expired_leases` path C3 says bypasses the guard. Both halves closed: **II.2's key table is now enforced by the grammar** (an unknown key is an error naming the file and line, and `@lease` gets a hint pointing at `@expires=<absolute>`), and `state.rs` no longer reads `lease`/`duration`. **This was Phase 1's job** — "unit tests for every grammar rule above, including every error case" — and II.2's table was the one rule with no test → **Phase 1** (fixed in 2l) |
| **S18** | **`auto_lock_checksums` rewrites YOUR module files on every sync, and it defaults to true.** `sync/mod.rs::attempt_auto_lock` hashes a `github`/`appimage` artifact after install and calls `ManifestEngine::update_package` to splice `@sha256=…` into the line you wrote. Two rules say no: **II.16 — "LiNix must not rewrite your files"** — and II.6, which puts generated facts in `locks/` ("Generated. In git. Yours."), which is exactly what a checksum is. It is also the last caller of `ManifestEngine`, whose `update_package`/`find_all_packages` are a second file-editor with their own `split_once(':')` parsers (C13), and the last real reason `groups_dir` exists. **The fix is `locks/<backend>.toml`, so this is Phase 4's thread, not a patch** → **Phase 4** (locks and git) |
| **S16** | **`--allow-mass-removal` deleted protected packages. FIXED.** `guard::enforce` returned `Ok(())` for *every* objection once the flag was set, so the flag meaning "yes, 50 packages is what I meant" also carried `python3` through. II.10 is explicit — `max_removals` exceeded → "cannot skip, `--allow-mass-removal`"; protected / OS-essential → **"nothing overrides"**. A confirmation asks; a refusal says no (V.26). The flag now clears only the count objection. **There was a test asserting the old behaviour** (`enforce_refuses_without_opt_in_and_proceeds_with_it`, which asserted the flag lets `python3` through) — the bug was written down as an expectation, which is why nothing caught it → **Phase 3** (fixed) |
| **S17** | **`[guard.enforce_on]` was a config key that switched the guard off, per command. DELETED.** Ten booleans — `apply`, `prune`, `sync`, `watch`, `upgrade`, `rollback`, `canary`, `remove`, `shell-exit`, `leases` — each of which made that command able to remove **anything, without limit**, protected and OS-essential included. It is not one of II.10's nine refusals; it is a switch that turns off all nine. V.21 says **no setting anyone can flip, inherit, or copy from a dotfiles repo** makes a routine sync delete something it did not install, and this was exactly that setting. The config template documented it, and `linix protected` printed which commands were unguarded. All gone → **Phase 3** (fixed) |
| **S15** | **`install` had P1 backwards: it installed first and wrote the line second. FIXED for `install`; `uninstall` is still inverted.** P1 says an imperative command *is* a shortcut for editing a file and syncing, so the edit is the operation and the install is what convergence then does about it. Backwards, every refusal on the write landed *after* the package was on the machine: installed, undeclared, drift by the next sync. `let _ = add_package_to_local(...)` hid it by making the write unfailable. `install` is now `declare` -> `sync`, which also puts an imperative install behind the guard (II.10) for the first time, and `--temp 2h` now writes `@expires=<absolute>` (II.16, V.38) instead of a lease nothing could read. **`uninstall` still removes first and undeclares second, so the pair is asymmetric (V.39 says they are a symmetric pair).** It cannot be flipped yet: `undeclare` -> `sync` only removes the package if `sync` removes drift, and `handle_sync` still passes `.with_prune(config.prune_on_sync)`, default **false**. **`prune_on_sync` is in II.17's delete list and V.34 says sync removes drift by definition — that deletion is the blocker, and it is the same one blocking `uninstall --temp` from becoming `absent:...@until=` (II.16).** → **Phase 2** (II.8 command surface) |

| **S14** | **The generated `priority` lists things that are not package managers.** `linix init` fills it from `registry.available()`, which includes the pseudo-backends `service`, `link`, `web` and `github` — so a fresh file answers II.6's question ("which package managers does this setup use, and in what order") with 26 entries, four of which cannot install a package. Harmless today because the model only consults `priority` for package statements, but it is the first file a new user reads and it is teaching them the wrong thing. The registry has no "is this a package manager" answer to ask; capability probing (`as_installable` + `as_searchable`) is the likely shape → **Phase 5** (F1/F4 own the generated files) |
| **S11** | **The test harness is not hermetic by construction, only by remembering.** `LINIX_DATA_DIR` exists precisely so tests do not touch real state (`safe_data_dir` says so), the docker/windows integration scripts set it, and the cargo tests never did — nothing enforced it, so it rotted silently for as long as the journal has existed. G3's "unverified" list and this are the same problem: isolation that depends on each test author remembering → **Phase 5** (make it structural, not remembered) |

## VI.3 Do not re-decide these

Three suspicions did not survive scrutiny:

- `matches!(b, "choco"|"scoop"|"winget")` at `generic.rs:363` is the **only** such site, and
  its comment is the best in the repo.
- `.unwrap()` density: 192 total looks alarming, but outside tests the max is **5 in one
  file** and ≤2 elsewhere.
- `bisect`, `fleet`, `conflicts`, `generation` are real, unit-tested implementations, not
  stubs.

---

# Part VII — Where the work stands

**Living section. It is the one place that records progress — Part III stays the plan, this
says how far it got (P4).** Update it at the end of every session. Everything below was
verified against the tree at the commit that last touched this section, not recalled.

## The state at `HEAD` (2026-07-17)

- **26 commits** since `d49d28c`.
- **517 tests passing, 0 failing. `cargo clippy --all-targets` silent.** *(The audit above
  caught this tree mid-deletion and said so rather than filing it as a finding — that was the
  right call, and the deletion has landed.)*
- *Those two numbers tell you nothing about the line below them, and never could — every false
  ✅ in this document was green when it was written (rule 11). They are here because a **red**
  suite would be worth reporting, not because a green one is progress. **The 2026-07-17 audit
  found four more false claims and the suite never moved off 0 failing.***
- ~~Phase 0 ✅ · Phase 1 ✅~~ — **both false. See the audit immediately below.** ·
  **Phase 2 — the cliff is jumped; the command surface remains** · Phases 3–6 not started.

## Audit: the ✅ that are not true (last run 2026-07-17, second pass)

**Phases 0 and 1 were marked ✅ before this section existed, and neither survives a grep.**
Checked by hand against the working tree, not recalled. **The suite is green and will stay
green through all of it: green means the old model still works, which is precisely the
problem.**

> **The second pass (2026-07-17) re-ran every command in this section and found the section
> itself was wrong in both directions.** One entry was **fixed and still filed**
> (`_active_profiles.txt` — with a retirement grep that could never have gone quiet), and
> C13's evidence had rotted so far that **four of the six sites it named did not exist** and its
> "Fixed when: 1" would have driven someone to delete a working validator. **Meanwhile it had
> missed three live ones** — `linix why`, `init -i`, and `activate`.
>
> **Phase 2's own account was the trusted one — "written from the work" — and it is where two of
> the three new findings were hiding.** `linix init` is ticked *"verified by running it"* and the
> wizard writes the old model; the `local.txt` row says `line_declares` is deleted and it is
> live with passing tests. **"Written from the work" is the same claim as "the tests pass": an
> observation standing in for the plan (rule 11).** Trust this section's *commands*. Do not
> trust its *prose*, including this paragraph.

> ### How to use this section — run it, don't read it
>
> **Every finding below is a command and the answer that means it is fixed.** At each phase
> change: **run the commands. Do not read the prose and believe it.** This section was
> written on a tree that is already moving, and by the time you read it some of it may be
> false — **that is the intended failure mode.** A finding that no longer reproduces is
> **not** a finding to argue with: **delete the entry, in the same commit as the fix.**
>
> When every entry is gone, **delete this whole section and restore the ✅** — to Phase 0 and
> Phase 1 in Part III, to C13 in VI.2, and to the status header at the top of this document.
> Those four places are where the false ✅ lived; they are where the true one goes.
>
> **Do not add findings here that are merely unbuilt.** This section is only for **work this
> document claims is finished and is not** — the failure that costs a reader their trust in
> every other ✅. Unbuilt work goes on the phase checklists, where it is honest.
>
> *(Why this is a ritual and not a note: this document has now been wrong about its own state
> every single time anyone checked — **always in the direction that flatters the tree**, and
> **every one was found by a grep, none by a reading.** That is the whole argument for making
> this a command you run rather than a paragraph you trust. See "A warning about this
> document" below.)*

### Phase 1 ✅ / C13 "done" — "one `backend:name` parser". There are nine.

**This one blocks the next action, so it comes first.** The grammar parser was **added
alongside** the eight, not substituted for them — and **six still trust the prefix without
validating it**, which is the entire defect C13 names.

**Re-run 2026-07-17: the finding stands and every number and line in it was wrong.** It said
nine sites and six skippers. It is **eight hits — 7 code + 1 comment — and three skippers**,
and **four of the six sites it named do not exist** (`config/manifest.rs:218` is past the end
of a 178-line file). Corrected:

| | site |
|---|---|
| **validates** | `grammar/statement.rs:304` (the new one, the one to keep) · `config/parser.rs:199` `split_removal_target` — *consults the registry, and is not a defect* · `main.rs:643` — `registry.get(b).is_some()` |
| **skips validation** | `app/insight.rs:428` · `config/manifest.rs:90` · `main.rs:1378` (`lease set`) |
| **uncounted, and new** | `model/resolve.rs:491` — `same_package`, in the **new model**. A name-comparison helper, not a backend parser; judge it, don't just count it. |

```
grep -rn "split_once(':')" src/ | grep -v "^src/parsers/"
```
**Now: 8 (7 code + 1 comment). Fixed when: 3 remain** — `grammar/statement.rs`,
`config/parser.rs:199` (validates, belongs) and `model/resolve.rs:491` (a helper).
**"Fixed when: 1" was never reachable and would have driven someone to delete a working
validator.** *(`src/parsers/` is excluded deliberately: it parses backend CLI **output**, a
different concern that happens to share a word. Do not count it, and do not delete it.)*

**The "why it blocks II.8" argument this entry used to make was built on a function that no
longer exists.** It cited `remove_package_from_local` comparing the backend half — `grep -rn
"remove_package_from_local" src/` is **empty**; it died in Phase 2e, and **S9 died with it**,
of `model/edit.rs`, not of `local.txt`. The two remaining skippers were checked for S9's shape
and **do not have it**: `insight.rs:429` requires both halves (`b == backend && n == name`) and
`manifest.rs:90` matches the *name* half. **The reason to finish C13 is C13, not S9** — do not
re-argue this from the old text.

**The test at `grammar/statement.rs:472` still says the quiet part:** *"Six of the **eight old**
parsers did `split_once(':')` and trusted the prefix."* **Both numbers are now wrong and the
word "old" is still unearned** — it is three, of eight, and they are current.

### Phase 0 ✅ — "delete everything in II.17". Roughly 15% happened.

| marked deleted | actually | evidence |
|---|---|---|
| the `-g` model | **the flag is gone; the model it anchored is not.** `groups_dir` ≈**51** refs across 14 files (was 84 — moving, not gone), `config_root` ≈**24**; `config_root()` is still literally `groups_dir.parent()`. | `config/config.rs:579` |
| `local.txt` (V.1) | ~~alive in 10 files~~ ~~**dead as of Phase 2e**~~ — **the write is dead; the readers are not, and this row named a deletion that did not happen.** `add_package_to_local`, `remove_package_from_local`, `remove_package_from_manifests`, `get_user_group_file` and `ManifestEngine::add_to_local` are genuinely gone, and `model/edit.rs` replaces them — **S9 really did die** (`edit.rs:378` parses via the grammar; test at `:669`), **but of `edit.rs`, not of `local.txt`.** `line_declares` is **NOT deleted**: `insight.rs:418`, live, called at `:447`/`:463`, with passing tests at `:695`. `main.rs:3616` still wrote `local.txt` in `init -i` (being removed in the current working tree). **See the `linix why` entry above — this row is how it hid.** | `insight.rs:418` |
| `keep.txt` (V.6) | ~~alive~~ — **dead as of Phase 2e.** It was never *read*: the whole `RESERVED_MANIFEST_NAMES` / `is_reserved_manifest` mechanism existed only to keep one file out of a crawl. Mechanism and all four exclusion sites deleted. | fixed |
| `_active_profiles.txt` | ~~still written on every `activate`~~ — **dead as of Phase 2f.** `materialize()`, `compose()` (the second profile engine) and `RESERVED_MANIFEST` are deleted; `ProfileManager` runs on the model and `activate` edits one file, `active`. 657 lines -> 348. | fixed |
| `prune` (V.34) | **partly fixed in Phase 2h.** `prune_on_sync`, `prune_scope` and `protect_imperative` are deleted, and sync removes drift by definition. `snapshot prune` stays — V.34 says deleting the command leaves exactly one meaning of the word ("delete old history"), and that is it. `auto_prune` is snapshot retention, the same one surviving meaning. | fixed |
| `migrate` | **606 live lines**, called by `adopt`. Renamed, not deleted — and `migrate.rs:283` still tells the user to *"run `linix migrate` again"*, a command that does not exist. | `main.rs:2153` |
| `clone` (V.33) | ~~a CLI command~~ — the command was gone, but **`fleet::clone` + `install_one` were still `pub` with zero callers**: Phase 0 deleted the flag and left the implementation. Deleted in Phase 2e. | fixed |
| ~884 marketing comments | **≈3,700 comment lines remain** in ≈41,600 lines of `src/` (~8.9%). **The count proves nothing on its own and is not the finding** — this one cannot be greped, so judge it by reading. The finding is that **the new `model/` files are writing spec-narration comments as the rule against them is being implemented**: `model/layout.rs:9` *"the fix for the shape of Monday's bug"*, `:135` *"which is Monday's bug (V.1)"*, `model/resolve.rs:162` *"the cost II.4 accepts knowingly"*. **V.42 is being broken by the code that cites V.42.** | `model/layout.rs:9,135` |

**Each row is one grep.** Fixed when each returns nothing:

```
grep -rn  "groups_dir" src/                                                  # ≈84
grep -rn  "config_root" src/                                                 # ≈25
grep -rln "local.txt" src/                                                   # ≈11 files
grep -rn  "_active_profiles\|RESERVED_MANIFEST" src/ | grep -v ^src/model/   # ≈7
grep -rn  "keep.txt" src/                                                     # live
grep -rn  "prune_on_sync\|auto_prune\|Prune" src/cli/ src/config/             # live
wc -l src/app/migrate.rs                                                      # ≈606, called by adopt
```

> **The counts are `≈` on purpose — do not treat them as a target, and do not "correct" them.**
> They drifted while this audit was being written (`config_root` 23→25, `local.txt` 10→11,
> minutes apart) because **more than one agent is committing to this tree.** A count is a
> tripwire, not a metric. **Only `0` means anything here.** If a number is merely different,
> the finding stands; it is retired when the grep is **quiet**, and never on the strength of
> the number having moved.

Delete each row as its grep goes quiet; restore Phase 0's ✅ when the last one does.

### Phase 2 — the II.1 layout: three of six paths have no reader

`Layout` defines the paths; only `priority` and `active` are actually read. **Zero callers
outside `layout.rs` and its own tests** for:

- `preferences_file()` — `layout.rs:84`. Nothing reads `preferences.toml`. II.10's refusals
  and V.43's nine guard rules have no source.
- `schedules_file()` — `layout.rs:69`. Live scheduling still reads `config.schedules` from
  `config.toml` (`scheduler/mod.rs:101`).
- ~~`locks_dir()` / `lock_file()` — `layout.rs:74-79`.~~ **Partly retired 2026-07-17:
  `locks_dir()` now has one real caller — `main.rs:3448`, which creates the directory in
  `scaffold_dirs`. Creating it is not reading it:** live locks still read the old
  `groups_dir.join("locks.json")` (`sync/resolver.rs:52`), so `lock_file()` remains unread and
  Phase 4 still owns this. **Two paths to one fact, again.**

```
grep -rn "preferences_file\|schedules_file\|locks_dir" src/ | grep -v model/layout.rs
```
**Now: one hit (`main.rs:3448`, `locks_dir`). Fixed when: a real caller for each of the three**
— which is the inverse of every other entry here, so read the result carefully. **A missing
name in that output means it is still broken**, and a name appearing does not mean it is read
— `locks_dir` is proof: it appears, and nothing reads a lock.
This entry is unusual: unlike the rest of the audit it is *unbuilt work, not a false ✅* —
it sits here only because Phase 2 is where it belongs and it is easy to mistake a path
helper with a passing test for a working file.

### `linix why` answers from the old model, and cannot see the new one (found 2026-07-17) — **FIXED**

**Fixed in Phase 2j. The diagnosis was right, including that it outranked C13.** `why` now
asks the resolver — `StateResolver::resolve_model()` — instead of re-reading the files with
its own parser. That is the point: `why` answers *"where is this declared?"*, which is the
question the model exists to answer, so a second implementation here was always going to be a
second answer, and it was the one a user reaches for when they already distrust the state.

- `scan_declarations`, `line_declares` (the 9th `split_once(':')` parser) and
  `interpret_source` are **deleted**, along with the test asserting `@module:` behaviour for a
  syntax this document deleted.
- **The answer is now a file and a line**: `at modules/dev.txt:2 (module:dev, profile:Work)`,
  taken from `__source` and `__scopes`. Verified against the binary on a II.1 repo — the exact
  case it previously could never see.
- **A `why` that cannot read your files now says so** rather than reporting "declared
  nowhere": `declarations_of` returns `Result` and the error names the file and line. Reporting
  a broken config as an absent declaration is the same failure in a quieter voice.
- **"Declared nowhere" is now an answer, not a blank**: *"in no active file — the next `sync`
  will remove it"*. That is drift, and saying it without saying what it means is how a true
  sentence still misleads.
- A lapsed line is reported as lapsed (II.16), not as the reason a package is present.

**The rest of the finding stands and is the next action.** `insight.rs` was one of nine files
holding `groups_dir`/`modules_dir`; 77 references remain across the tree.

**This is the new "most dangerous artifact", and it outranks C13.** Phase 2e's row below says
`line_declares` is **deleted**. It is alive at `app/insight.rs:418`, with its own passing unit
tests at `:695-705` — including `assert!(!line_declares("@module:htop", …))`, a test asserting
the behaviour of a syntax this document deleted.

```
grep -c "layout()\|Layout\|profiles_dir\|active_file" src/app/insight.rs      # 0. Fixed when: not 0.
```
**`insight.rs` does not contain the word `Layout`.** `why` (`insight.rs:477`) calls
`scan_declarations` (`:436`), which crawls `config.groups_dir` for `*.txt` and
`config.modules_dir` for **`*.module.txt`** (`:459`). II.1 modules are `modules/<name>.txt`
(`model/layout.rs:102`). **So `linix why` can never see a II.1 module** — wrong suffix — and it
never opens `profiles/` or `active` at all.

**Why this is the worst one in the section:** resolution runs on the new model (Phase 2d), so
`why` is now the command that *answers the question the new model exists to answer* — "where is
this declared?" — by reading the model that no longer decides. It does not error. It prints a
confident, sourced, wrong sentence, and **it is the command a user reaches for precisely when
they already distrust the state.** A false ✅ makes a reader skip work; this makes the tool lie
to a user who is checking.

**It is not a one-off.** `grep -rln "config\.groups_dir\|config\.modules_dir" src/` → **9 files**;
`services.rs`, `bundle.rs` and `config/manifest.rs` also score **0** for any Layout reference.
"The seam held" is true of `src/backends/`, `src/core/` and `src/parsers/` — the seam was never
what these were on the wrong side of.

### `linix init -i` writes the old model — the ✅ verified one of two paths — **FIXED 2026-07-17**

**Fixed in Phase 2h.** `interactive_init` calls `scaffold_repo`, the `local.txt` write is
gone, and the three prompts for deleted settings are gone with the settings. **The lesson is
kept below verbatim, because it is the part worth keeping**: *"verified by running it"* was
the strongest evidence claim in this document and it was still partial — it ran the path the
author was thinking about.

Phase 2's checklist marks `linix init` done: *"Verified by running it: it produces a repo that
resolves."* True — of `linix init`. **`handle_init` (`main.rs:3429`) has two paths**, and
`interactive_init` (`:3457`) `return`s at `:3434` **before `scaffold_repo` (`:4294`) — the
only thing that writes `priority`, `active` and a profile — ever runs.** It wrote
`groups_dir/local.txt` (`:3616`) and interactively prompted for `prune_on_sync`, `prune_scope`
and `auto_prune`: three settings II.17 deletes and "the next action" is deleting now.

**Being fixed in the working tree as this was written** — the `local.txt` write is already gone
from the uncommitted diff. **Kept anyway, because the lesson is not the bug.** *"Verified by
running it"* is the strongest evidence claim in this document and it was still a partial
verification — it ran the path the author was thinking about. **Rule 11 is usually read as being
about `cargo test`. It is not: it is about any observation standing in for the plan.** A wizard
is a second path, and a second path is a second implementation.

### `activate` does not do what II.6 says (found 2026-07-17 — **being fixed in flight**)

> **Status at the time of writing: the working tree already fixes most of this.** `activate`
> takes `add: bool`, `-a`/`--add` exists, `Switch` is deleted, `parse_active` reads `when`
> blocks, and II.6's empty-names refusal is in the code verbatim. **The owner's four decisions
> (above) match what was built** — they were made from this entry and landed within the hour.
> **Retire the rest of this entry when that commit lands. Two things are not fixed:**
>
> 1. **`activate` overwrites a `when` block and does not say so.** `profile.rs` prints
>    `active is now {names}.` and never names the block it deleted. **Automatic is not silent
>    (S6)** — the owner chose "no refusal", not "no receipt".
> 2. **`deactivate` implements a rule the owner has since reversed.** `profile.rs:189` prints
>    *"Removed {} from the list. It is still activated by the `when {}` block…"* — **that is
>    the behaviour II.6 no longer specifies.** `deactivate` must now remove the name from the
>    matching block too, so that sentence is unreachable by construction. **It was built
>    correctly against the spec as it read at the time; the spec moved.** The replacement
>    message is for the *other* case only — a block that does not apply to this host, which is
>    left alone. `model/profiles.rs:459`'s comment says it exists to support the old sentence;
>    **the machinery it feeds is still needed, for the opposite purpose.** Do not delete the
>    block-awareness — re-point it. *(Related, unjudged: these go
> to `info!`/`tracing`, not stdout. II.6 says the commands **print** what they touched. Whether
> a user with default settings ever sees them is untraced.)*

Not marked ✅ — **worse: II.6 was written as a specification of existing behaviour and it
describes the opposite of the code.**

| II.6 says (`:345`) | `app/profile.rs` does |
|---|---|
| `activate NAME…` — *the file becomes exactly this list* | **adds.** `:78-96` reads `active`, pushes what's missing, writes it back. This is `activate -a`. |
| `activate -a NAME…` — *adds* | **no `-a` flag exists.** `cli/args.rs:334-338` takes `profiles: Vec<String>` and nothing else. |
| — | **`profile switch NAME` (`args.rs:737`, `profile.rs:123`) is the set form**, one name only, and appears nowhere in this document. |

`args.rs:332` documents `activate` as *"add each to the active set"* — **the CLI help and II.6
contradict each other in the same repo.** The refusal at II.6:360 (*"activate needs a profile
name…"*) does not exist; `#[arg(required = true)]` prints clap's generic error. `-r` is
genuinely gone (verified).

**And `active` cannot hold a `when` block, which II.6:330 shows it holding.**
`write_active` (`profile.rs:257-269`) rebuilds the file from a flat `Vec<String>` — **any
activate/deactivate silently destroys every block in it** — and `parse_active`
(`model/profiles.rs:194`) rejects any line with more than one word, so `when host == laptop {`
is a hard error: *"`when host == laptop {` is not a profile name"*. The II.6 example file does
not parse. **Decide which is right before building the verb, not after.**

### VI.1 "killed by this design — no work needed" — two rows are live bugs (found 2026-07-17)

**VI.1 is a list of finished-claims and belongs under this section's rule.** *"Killed by this
design"* is only true once the design is **built**; two rows are killed by designs that are
**not started**, so the bug is live and filed as needing no work — the exact failure a false ✅
causes, wearing a different word.

- **`linix shim --source` — marked "(verified)". It is a live bug.** `cli/args.rs:100` makes
  `--source` **required**; `main.rs:2883` → `context.rs:612`
  `pub async fn create_shim(&self, binary_name: &str, _source_spec: &str)` — **the underscore
  is the bug.** The user is forced to supply a value that is discarded, so
  `shim rg --source cargo:ripgrep` and `--source apt:nonsense` do the same thing. What kills
  it is II.16 (shims as declared lines) — **unbuilt**, and S4 routes the shim path to Phase 3,
  **not started**.
- **E12 `confirm_destructive` gates the wrong thing.** **`module` overwrite fixed in Phase 2k**
  — II.8 says destroying a file you wrote is a plain refusal plus `--force`, and it is now
  exactly that, wired to nothing. **The other two sites are live**: `main.rs:966` and `:1118`
  gate `install`/`uninstall` confirmation, which is the II.10 guard's job — Phase 3, not
  started. `confirm_destructive` itself is in II.17's delete list and dies with them.

**Also contradictory:** E4 says two snapshot retention engines need no work; **S2 routes its own
fix to "Phase 4 (one retention engine)"** — the document says both that the consolidation is
unnecessary and that a bug waits on it. *(A check for the second engine found only one — both
`snapshot.rs:455` and `generation.rs:262` feed `core::RetentionPolicy`. E4 may simply be stale
wording rather than a real pair. **Decide it; do not leave it readable both ways.**)*

**The rule this earns:** VI.1's rows must name the phase that kills each bug, and a row whose
phase is unstarted is **not** "no work needed" — it is *work scheduled elsewhere*, which is a
different sentence with a different reader response.

### The shape of all of it

**The new model was built alongside the old one, and the old one was never removed.** That is
the "two ways to do one thing" disease Phase 2 explicitly forbids — *"do not run two models
behind a flag"* — arrived at by accretion rather than by decision, which is why no one chose
it and no test objects.

**The most dangerous artifact is not a false ✅ — it is a comment asserting a deletion that
did not happen.** ~~`model/profiles.rs:24` states there is no `_active_profiles.txt` and no
materialization. `app/profile.rs:347` writes it on every `activate`.~~ — **retired 2026-07-17:
the comment came true.** Phase 2f deleted the writer; `grep -rn "_active_profiles\.txt" src/`
now returns that comment and nothing else. **The example is retired; the rule it teaches is
not: trust neither the ✅ nor the comment. Grep.** The replacement example is one line down,
and it is worse, because a comment can only lie about one file — **`linix why` lies about the
whole model** (below).

> **The retirement grep in this section was broken and could never have gone quiet.**
> `grep -rn "_active_profiles\|RESERVED_MANIFEST"` matches the *test name*
> `the_seam_carries_what_the_active_profiles_reach` (`sync/resolver.rs:466`) — the `_active_profiles`
> substring sits inside `what_the_active_profiles_reach`. A fixed entry read as unfixed forever.
> **Anchor a retirement grep on the filename (`_active_profiles\.txt`), never the bare stem** —
> a tripwire that cannot go quiet is not a tripwire, it is a permanent false finding, and this
> section's whole ritual is "run it and believe the result."

**Recommended order, given the above:** finish the deletions **before** II.8, not after.
Every verb added on top of the six non-validating parsers, `groups_dir`, and `local.txt` is a
verb that has to be rewritten when they go — and II.8 is the biggest surface in the plan.

## The one thing to understand before continuing

**The model now runs resolution.** `StateResolver::resolve_desired_state` builds the II.1
`Layout` from config, loads `priority`, runs `model::Resolver`, and probes bare names. The
seam signature is unchanged, so `src/backends/`, `src/core/` and `src/parsers/` never
noticed. `tests/e2e_tests.rs` now drives module → profile → resolve → plan → execute →
registry on the new model.

**The branch did not stay red, and that is not a warning sign** — the seam held. What broke
was six tests written against the deleted model (`@module:`, `.module.txt`, the `groups_dir`
crawl, `manifest:` tags). Each was ported to the II.1 layout with its guarantee intact, not
deleted.

**Two things the wiring changed that the plan did not anticipate.** Both are in VI.2:

- **S13 (fixed).** Probing had to move *between* reading the files and merging them. Keyed
  `?:ripgrep` until probed, a bare `ripgrep` never met an explicit `cargo:ripgrep`, so rule
  5 silently did not apply to bare lines and both were installed. `model::Resolver` now
  splits: `statements()` → the caller probes → `with_bare(answers)` → `collect()`.
  `resolve()` still does both, for callers that need no probe.
- **`__scopes` is new, and `__source` changed meaning.** They were one tag doing two jobs,
  which is how `upgrade --module dev` came to be matched against a filename. `__source` is
  now `file:line` — what II.8's messages and every error need. `__scopes` carries
  `module:dev;profile:Work` — what `--module` / `--profile` match on. The model writes it
  because it is the only thing that knows profile→module. `collect` keeps **every** origin
  of a merged package, not just the winner's: the loser's scope is not thereby untrue.

## The next action, precisely

**`uninstall` — the last inverted verb (S15).** `install` is now `declare` -> `sync`;
`uninstall` still removes first and undeclares second, so the pair V.39 calls symmetric is
not. The blocker is gone: sync removes drift by definition now, so `undeclare` -> `sync`
removes the package. This is a small change and the guard is ready for it.

Then the remaining deletions: **`groups_dir` (84 references)**, whose `config_root()` is
still literally `groups_dir.parent()`; the **seven non-validating `split_once(':')`
parsers**; `migrate` (606 lines, renamed to `adopt` rather than deleted, still telling users
to run a command that does not exist); and **E6** — "unmanaged" has two implementations that
will disagree.

## Done in Phase 2l — `uninstall`, and the symmetric pair is symmetric again

**`uninstall` obeys P1 (S15 closed).** It removed the package first and edited the file
after; now the file edit IS the command and sync carries it out — so the removal goes through
the guard, the plan and the counts like every other removal, rather than reaching for the
backend directly and asking the guard on the side. `install` and `uninstall` are the
symmetric pair V.39 describes again.

**Both of II.8's `uninstall` rules are built, and verified against the binary:**

- *"jq is still declared in module `gaming`, which isn't active. It will come back if you
  activate Gaming."* — deleting the line you can see while an identical line waits in a
  module you forgot about is a package that returns the next time you switch profiles.
- *"steam isn't declared, so there's nothing for it to come back to. Did you mean a plain
  uninstall?"* — `--temp` on something undeclared.

**`uninstall --temp=2h` is a suspension now (II.16, V.37).** It writes
`absent:cargo:ripgrep@until=2026-07-17T16:17` and the model does the rest: a dated line beats
an undated one (II.7 rule 6), so the module that wants it loses until the date passes, and
then wins again. **Verified end to end** — the package is not wanted while suspended and is
wanted again once the date is in the past. "Take the game away until the weekend" works, with
no timer and no sweep: the same dated-line machinery `install --temp` uses, pointed the other
way.

Bare `--temp` (restore on shell exit) stays outside the model on purpose (II.8): a shell
session is not a declaration, so it writes no file — and it now calls the guard, which it
did not before (C1).

**Found: S19** — `@lease=2h` still works by hand, and it is the one option key that can
uninstall your package, on a path C3 says bypasses the guard.

## Done in Phase 2j/2k — the commands that still read the deleted model

**`why` asks the model** — the audit's worst finding, answered in full above.

**`adopt` writes `modules/adopted.txt` (II.9).** It wrote `migrated_<timestamp>.txt` into
`groups_dir`: a folder nothing reads any more, under a name that made **the second `adopt`
declare every package twice** — which the resolver then refuses as a contradiction (II.7 rule
5). One file, overwritten, so adopting again answers "the machine as it is now" rather than
"the machine plus history". It goes through the editor, so `use adopted` reaches the active
profile and says so.

**`module` runs on II.1.** It listed `*.module.txt` — a suffix II.1 does not have — so
`module list` printed **nothing** on a real repo, and `module show`/`create` addressed files
the resolver never reads. Now `modules/<name>.txt`, and **the folder decides**: `module list`
asks `ModuleLoader::available()`, so a `README.md` in `modules/` costs nothing (II.3).

**E12 is fixed for `module` (II.8).** Overwriting a module you wrote was gated on
`confirm_destructive` — a setting about *package removals* deciding whether your file
survives, which is one prompt meaning two unrelated things. It is now a plain refusal plus
`--force`, like every other tool, and wired to nothing.

**`bundle` copies your repo, whole.** It copied `groups/` and `modules/` by name, so under
II.1 it silently left `profiles/`, `active`, `priority`, `locks/` and `preferences.toml`
behind — a bundle that restores half your declarations is worse than one that fails.

**Found, not fixed: S18** — `auto_lock_checksums` rewrites your module files on every sync,
and defaults to true. It is Phase 4's thread (a checksum is a lock), and it is the last
caller of `ManifestEngine` and the last real reason `groups_dir` exists.

## Done in Phase 2i — `activate` does what II.6 says, and `active` gained `when`

Answered the audit's `activate` finding in full; the details are in that section, marked
FIXED. The shape of it: **II.6 described the opposite of the code**, the CLI help contradicted
II.6 in the same repo, and `profile switch` — the set form under a second name — was not in
this document at all. `activate` sets, `activate -a` adds, `switch` is deleted, and the empty
list gets II.6's refusal instead of clap's.

**`active` holds `when` blocks.** It was the one file that broke II.2's "one rule, everywhere"
— `parse_active` rejected any line with more than one word, so **II.6's own example file was a
hard error**, and `write_active` rebuilt the file from a flat list so any activate/deactivate
silently destroyed every block in it. `read_active` now gates them, `deactivate` edits
top-level lines only, and a name a block also holds is reported rather than silently ignored.

## Done in Phase 2h — the guard, and sync becoming sync

**Two live safety bugs, both contradicting II.10's "nothing overrides". S16 and S17 in
VI.2.** `--allow-mass-removal` cleared every objection including protected and OS-essential;
`[guard.enforce_on]` was a config key that switched the guard off per command. **The first
had a test asserting the broken behaviour** — the bug was written down as an expectation,
which is why it survived review. Both fixed, with the tests II.10 actually implies.

**`prune_on_sync`, `prune_scope`, `with_prune` and `protect_imperative` are deleted, and
sync removes drift because that is what sync IS (V.34).**

- `prune_on_sync` made "does sync converge?" a setting. A sync that does not remove drift is
  not sync; it is `prune` with the install half amputated, which is the thing V.34 deleted.
- `prune_scope = "system"` was **the setting V.21 forbids in so many words**: flip it and a
  routine sync deletes software LiNix never installed. That is `purge-unmanaged` — **a
  command you type, not a mode you inherit** — which is why deleting the mode and building
  the command had to be one change. Deleting it alone would have removed a real feature.
- `protect_imperative` guarded against a bug that no longer exists. An imperative install
  had no line, so it read as drift the moment it was recorded; it has a line now
  (`modules/imperative.txt`), so it is declared like everything else.

**`purge-unmanaged` is built (II.11).** The ratio, not the count (V.20 — on Alpine a count
misses 14-of-14); `max_removals` does not apply because it catches accidents and this is
deliberate; protection and OS-essential still do; snapshot first, automatically, and **"THERE
IS NO UNDO FOR THIS" when none can be taken**; the whole list, every line, because the pain
is the feature; typed confirmation.

**Guided setup lost four questions that stopped existing**: "should sync remove drift?" (it
is drift removal), "how aggressive?" (that is `purge-unmanaged`), "protect imperative
installs?" (they are declared now), "preferred default backend?" (that is `priority`,
detected). A question whose answer LiNix can work out is homework (V.41).

## Superseded — the note this section carried before

This section previously warned that making sync remove drift by default was "the shape of
this repo's flagship bug". **That was overcautious, and reading V.21 and II.7 carefully is
what corrected it.** Sync removes *what it manages and you stopped declaring* — bounded by
the registry, not by the machine. The flagship bug was the opposite shape: `-g` moved the
wish list while the registry stayed put, so owned-but-unwished read as drift (V.1). The
dangerous thing was never sync converging; it was `prune_scope = "system"`, which let sync
reach outside what it owns. That is now impossible rather than merely off by default —
which is what V.21 means by "not safe by default, but safe permanently".

## Done in Phase 2f — set math, and the last of the old profile engine

**`app/profile.rs` was the last whole copy of the old model.** 657 lines that still write
`_active_profiles.txt` on every `activate`, and still hold `compose()`: a second, complete
profile-composition engine duplicating `model/profiles.rs`. Its `active` file is in the wrong
place (`profiles/active`, not the config root) and its profiles are `<name>.profile`, not
II.5's Capitalized `profiles/<Name>`. Nothing about it agrees with the model that now runs
resolution.

It is now 348 lines and runs on the model. `materialize()` is gone (a materialised copy is a
second place the same fact lives), and `compose()` — a second complete profile engine — is
gone with it.

**II.4's set math is implemented, and the decision behind it is V.46.** It was specified and
never built: `ProfileLoader::resolve` handled only `use`, and `evaluate_expression` had no
caller outside its own tests. It now works end to end — verified against the binary, not just
tests. `exclude`, `intersect`, `-pkg` and full expressions like `(Work | gaming) & security`,
with II.4's fixed order: gather, narrow, subtract, and **subtraction always wins** whatever
order you wrote the lines in.

**V.46 predicted a cost that turned out not to exist, and the entry says so.** The prediction
was that set math would cost a package its module name. It does not: atoms map back to the
**statements** they came from, so a package keeps its `Origin`, its file, and its module.
`upgrade --module editors` still finds `vim` through an `exclude`.

Also settled there: **`include` is gone** (V.46). `use` already meant union, and two words for one
operation is the disease this design cures, sitting inside the spec. `include x` is an error
that says `use x`.

## Phase 2's remaining checklist

- [x] Wire `model::Resolver` into `StateResolver`.
- [x] **`local.txt` and `keep.txt` are deleted, and S9 with them.** `model/edit.rs` is the one
      writer now: `install`, `uninstall`, `forget`, `teleport`, `service enable/disable` and
      the package-manager hook all go through it, and it parses every line with the grammar
      rather than splitting on `:`. `--into` exists (II.8). The three landing modules exist,
      and the first write to one adds `use <name>` to the active profile and says so.
- [x] **`linix init` writes the II.1 repo** — `priority` generated from what this machine
      actually has (V.41), ordered by V.14's one real rule, with its reason in the file (P5).
      Verified by running it: it produces a repo that resolves. **S14**: the generated list
      still includes `service`/`link`/`web`/`github`, which are not package managers.
- [ ] **S12** — `repo:`, `shim:`, `service:`, `link:` and `schedule:` lines resolve into
      `DesiredState::extras` and are then dropped at the seam, because the seam carries
      packages. `sync` warns for each by file and line so it is not silent, but they do
      nothing. The ordering phases below are the fix — that is what `extras` was collected
      for.
- [x] **`local.txt` and `_active_profiles.txt` are deleted** (Phase 2e, 2f). S9 died with
      `local.txt`. *Corrected: Phase 2e's commit said `line_declares` was deleted; that was
      true of `config/parser.rs` and **there was a second copy in `insight.rs`** — deleted in
      Phase 2j. "Deleted" means the grep is empty, not that the copy you were looking at is
      gone.*
- [ ] Delete the remaining `config.toml` sections superseded by `priority` / `preferences.toml`.
- [ ] Delete the old parsers now that `config/grammar/` is the one parser (C13).
- [ ] **E6** — "unmanaged" has two implementations that will disagree. One function, defined
      as *"what `adopt` would adopt"*.
- [ ] Planner ordering phases: repos → index refresh → packages → dependents.
- [ ] **Cycle detection (II.7) — already caught in all three places. Do not rewrite; improve
      the errors.** `use` loops: `model/profiles.rs:62` and `model/modules.rs:146`, a
      push/pop path stack, so diamonds correctly pass. `@requires` loops:
      `planner.rs:323` — `is_cyclic_directed` after `build_execution_graph`, which is what
      stops `transaction.rs:179-190`'s ready-set loop spinning to its timeout. What is owed
      is **the error text, not the detection** (V.45): the two `use` errors name the path but
      carry a single `Origin` — the edge that closed the loop — and the planner's is
      *"Circular dependency detected in graph construction."*, which names nothing at all.
      **Re-verified 2026-07-17 (line numbers rotted: `planner.rs:323`→`:276`,
      `profiles.rs:62`→`:88`, `modules.rs:146`→`:165`; the claim itself holds).** Two things the
      entry did not say: **`GrammarError` (`grammar/error.rs:45`) holds one `origin`, so it
      cannot structurally carry II.7's trace** — the `seen: Vec<String>` path stack keeps names
      only, and the file/line of every edge but the last is discarded at push time. Fixing the
      text means changing the error type or the stack, not the message. **The planner's is
      easier than it looks:** `PackageSpec` has no `Origin` field, but `options["__source"]` is
      `file:line` (`model/resolve.rs:533`) and **the planner already reads it**
      (`planner.rs:119`) — so II.7's *"which packages, and the file and line each edge came
      from"* is reachable there today without a new field.
- [ ] The II.8 command surface (`main.rs`, ~4,370 lines). **Done so far:** `install`
      (P1 order, `--into`, `--temp` -> `@expires`), `forget`, `teleport`, `service
      enable/disable`, the hook, `purge-unmanaged`, and `activate` / `activate -a` /
      `deactivate` / `why` — all three `activate` decisions were taken and are recorded above.
      `uninstall` (P1 order, both II.8 warnings, `--temp` -> `absent:@until`), `module`,
      `adopt`, `bundle`. **Left:** the read-only verbs (`status`, `list`, `unmanaged`) have
      not been checked against II.8 at all, and **E6** — `unmanaged` has two implementations
      that will disagree.

## Decisions the owner has made — do not re-open

| | |
|---|---|
| **V.43** | Keep all nine guard refusals, including the three orphaned `policy.toml` rules (`pinned_only`, `require_snapshot`, `deny_vulnerable`). II.10's "five" was wrong. |
| **S6** | `sync` heals **automatically**. Asking permission to fix drift asks permission to do sync's own job. Automatic ≠ silent: it must say what it did, and the removal still goes through the guard. |
| **S8** | Keep `undo`. Keep its path check (renamed to say it guards the snapshot-read path). Delete the false global claim. Restore must state it rolls back the whole filesystem before asking. |
| **II.6 verbs** (2026-07-17) | **Three verbs, as II.6 already said: `activate` SETS, `activate -a` ADDS, `deactivate` REMOVES.** The code had `activate` adding and the CLI help documenting it that way — **the spec was right and the code was wrong.** Not a re-opening: II.6 was already correct, the audit found the drift. |
| **`profile switch`** (2026-07-17) | **Dies.** Once `activate` sets, `switch NAME` *is* `activate NAME` with a worse name and a one-name limit. It was the set form only because `activate` had wrongly taken the add form's job. **Two ways to do one thing** (P1). |
| **`when` in `active`** (2026-07-17) | **`active` holds `when` blocks.** `when` gates every other file (II.2); `active` being the exception was an accident of `parse_active` rejecting any multi-word line — which made **II.6's own example file fail to parse.** One rule, everywhere. |
| **`deactivate` vs blocks** (2026-07-17) | **`deactivate` removes the name from the top level and from every `when` block that applies to this host** — empty blocks go with it, and it says so. **Reverses II.6's old *"it is still activated by the `when` block on line 4"* bullet**, which described a verb that removed the line and left the thing on. **A block that does not apply to this host is never touched**: nothing there is active, so there is nothing to deactivate, and `active` is a shared file — editing another host's block from this one changes a machine you are not at. It says why and changes nothing. **This is the one place `deactivate` edits a block and `activate -a` does not: adding has a choice of where to put the name; removing does not.** |
| **`activate` vs blocks** (2026-07-17) | **`activate` overwrites the file, blocks included.** It is the set form; it sets, and a block is part of what the file says. **`activate -a` and `deactivate` never touch a block** — they are the surgical pair, and that asymmetry is why `-a` exists. **It does not ask** (declarative: overwriting the list is the command's job) **but it does not do it silently** (S6) — it names every block it removed. *Asking and reporting are not the same thing, and the argument against the first is not an argument against the second.* |

## A warning about this document

**Every "(verified)" / "(measured)" fact in this spec that has been checked has been wrong —
seven for seven, always under-reporting.** They were measured against an older tree. Corrected
so far: the comment count (139 → 884 + 32 false), both good-comment exemplar citations, the
parser count (5/3 → 8/6 → **9/6**, wrong twice, the second time by a re-measurement that was
itself called "(re-measured)"), the backend count, and — the expensive one —
**"`reconcile_shims` is written and never called (verified)"**, which was false and was the
sentence hiding a bug that made every `sync` delete the user's own files out of
`~/.local/bin` (S1).

**The ✅ markers fail the same way, and worse.** Phases 0 and 1 were both marked complete
while untrue (2026-07-17 audit, Part VII) — under a rule, already written at the top of this
document, that says *"never describe unverified work as done."* **The rule was not enough.**
A wrong measurement makes a reader over-confident; **a wrong ✅ makes a reader skip the work
entirely**, and nothing downstream will object, because the code it should have deleted is
still there passing its tests.

**The bias was one bias, in two costumes: this document flatters the codebase.** Measurements
understate the mess; status markers overstate the progress.

> **The 2026-07-17 second pass produced the first counter-examples, and the rule has to change.**
> `_active_profiles.txt` was **fixed and still filed as broken**; C13's skipper count was **six
> and is three**; `resolver.rs:212` was cited as a non-validating parser and **no longer parses
> anything**. In each, the **tree was better than the sentence** — the first time that has ever
> happened here.
>
> **This is not good news and it is not permission to relax.** The old rule ("assume the tree is
> worse") was a *heuristic that let you skip checking*, and it has now failed in both directions,
> which is the only thing that could have shown it was always the wrong kind of rule. **The
> document is not biased pessimistic or optimistic. It is stale**, and stale drifts whichever way
> the tree moved. **The replacement rule is not a direction, it is a refusal to guess: no claim
> here is evidence of anything. Run the command.** Note which way this one broke — a *fixed* entry
> read as broken costs someone a day re-fixing it, and a *stale* count read as a target
> (**"Fixed when: 1"**) would have had them delete a working validator to hit a number.

**Line numbers rot faster than claims — and this very example rotted again.** V.29 cites
`planner.rs:407-426` for `spec.requires`; **this document corrected that to `:431`, and it is now
`:384`.** The *reasoning* is still correct. **The correction needed a correction inside one
session** — which is the argument, not a footnote to it. Treat a citation as a place to start
looking, never as proof — **grep the symbol, don't trust the line.** Every cycle-detection
citation on Phase 2's checklist has rotted the same way (`planner.rs:323` → `:276`,
`profiles.rs:62` → `:88`, `modules.rs:146` → `:165`); the claim they support — *"already caught
in all three places, do not rewrite"* — **re-verified true.**

**Verify any Part II–VI citation against `HEAD` before you act on it.** Part V's *reasoning*
has been consistently right and is worth following; its *measurements* are not.

## Bugs found while implementing

**S1–S11 in VI.2 → "Found during implementation".** Each is assigned to the phase that owns
the mechanism. Four were live defects already fixed (S1 shim deletion, S10 tests writing to
the real data dir, and two parser bugs); the rest are scheduled. Add to that table rather
than to a commit message — a bug recorded only in a commit message is a bug nobody will find.

## Not started, and owed

`README.md` (28k) still documents `-g`, `prune`, `clone` and `migrate` — **all four deleted in
Phase 0.** `CHANGELOG.md` likewise. Both are Phase 5 (docs), and both are cleanly separable
from the code work if a second session ever runs in parallel.
