# Three shifts worth their cost — 2026-08-03

> **Status: not spec, not ruled, not a work order. Do not build from this file.**
>
> This is an architecture-direction note written in answer to one question — *is there a
> paradigm-level or language-level move available to LiNix, or only more grinding?* — and the
> answer is that there are three, plus one that looks like a fourth and is a trap.
>
> Nothing here is decided. None of it has a register ID. §7 lists the questions in the form they
> would take in [`spec/decisions.md`](spec/decisions.md) if the owner chooses to register them;
> until then they are *unregistered*, which is a weaker status than OPEN and should not be read as
> one. `CLAUDE.md`'s four stop-and-ask conditions cover all three shifts: each changes behaviour a
> user would notice, and Shift 1 changes what a file on disk *is*.
>
> Written against `ee5adf3` on `grade/2026-07-29`, with the tree in the same 26-modified-file
> state [`GRADE-2026-08-03.md`](GRADE-2026-08-03.md) audited. Every `file:line` below was read in
> that tree; the grade numbers are quoted from that document, not re-measured.

---

## 0. Read this before the rest of it

*This section was written from the grade's letter grades and rewritten after reading the whole
document. The correction matters, so it is made rather than hidden: the first version implied a
long slog stood between here and any of this. It does not.*

[`GRADE-2026-08-03.md`](GRADE-2026-08-03.md) §8 says what the three blockers actually are: **a
missing printed line, a missing `is_absolute()`, and a missing `OnceCell`.** Its own words — *"None
is architectural."* Extensibility graded **A−** with the custom-backend mechanism driven end to
end, usefulness **A−** with the auditor unable to name a missing feature, ~48 `unwrap`s across 88k
lines every one of which was read and cleared, clippy silent, 1711 tests green. **The distance
from C to good here is an afternoon, and pretending otherwise would be its own kind of
inaccuracy.**

What is *not* an afternoon is that grade's **§9**, and §9 is the reason this file still opens with
a warning — a different one than it opened with before.

Four of twelve findings are the same failure mode: **the correct behaviour already exists in this
codebase, with its reasoning written down, and the sibling site never received it.** `rebuild.rs`
reports protected skips and has a test named for it; `planner.rs` calls `debug!` and `continue`.
`locate.rs:38` refuses a relative path and explains why; `resolve_root` accepts anything. `web.rs`
and `appimage.rs` build the expensive object lazily; `github.rs` builds it in the constructor.
`planner.rs` states the rule about caps in a comment; `guard.rs` hardcodes `8`.

That is the constraint on everything below. **You cannot safely move a foundation in a codebase
where one rule is implemented independently at four sites and is wrong at three of them** — not
because the shifts are too ambitious, but because a shift *relocates* rules, and relocating a rule
that exists in four copies produces five. Grade §9 names the cure precisely, and it is not more
review: **invariant tests that quantify over sites rather than naming one.** Every path that drops
something from a plan appends to a reported list. Every source of the config root refuses a
relative path. Every fan-out cap reads a config field. A startup budget that fails when a
no-child command exceeds it.

**That work is the entry fee for this document, and it is also the first instalment of §8 —
the dream this whole file is pointed at.** The two dreams in §8 and §9 are not decoration
appended to the end; they are what grade §9's finding looks like when you generalise it, and they
are the reason the shifts in §2–§4 are worth the trouble.

*(One number in circulation, unresolved and worth resolving: `SPEC.md` says 52 backends registered
and 22 ever run for real; the grade says 48 registered, 24 live on this host. Neither is obviously
stale and this note does not adjudicate them. Three places count the same thing and no two agree —
which is the same disease as §9, in the documentation register.)*

---

## 1. The ceiling — the shift that is not available

Nix's actual advantage is not that it is declarative. LiNix is declarative. Nix's advantage is
the content-addressed store: a rollback is a pointer move, and it **cannot fail**, because
nothing was ever mutated in place.

LiNix cannot have that, and no amount of work will change it, because LiNix's whole premise is
delegation to managers that own mutable global state. `apt` owns `/usr`. `scoop` owns its shim
directory. The moment `apt-get install` returns, the machine has been changed in a way LiNix did
not perform and cannot undo by arithmetic.

Every snapshot provider in the tree — `backends/btrfs.rs`, `core/snapshot.rs`,
`app/snapshot_restore.rs` — is a patch over that gap, and a reasonable one. But they are
*recovery*, not *atomicity*, and the difference is that recovery can fail.

**Consequence, and it is the useful half:** stop treating "get closer to Nix's guarantee" as a
direction. It is a wall, not a horizon. §4 is what to do instead — draw the line where the
guarantee genuinely changes, and say so out loud.

---

## 2. Shift 1 — derive ownership, stop recording it

**The largest surviving instance of "two of everything" in this tree.**

### The finding

`registry.json` is a third source of truth. Your config says what you want; the machine says what
is there; and `registry.json` says what LiNix owns — and it is the only one of the three that
nothing else can reconstruct. Count what exists to serve it:

| site | what it does for the ledger |
|---|---|
| `core/state.rs:160`, `:488` | is it |
| `core/datalock.rs` | an exclusive writer lock whose stated purpose is protecting it |
| `app/bundle.rs:216-226`, `:364`, `:435` | must copy it out of the data root, because a bundle without it is not a bundle |
| `app/snapshot_restore.rs:174-217` | hunts for it in **three** candidate paths inside a snapshot, and errors if it finds none |
| `main.rs:606` | `READ_ONLY_COMMANDS` exists because "not locking a writer costs an entry out of `registry.json`, which is a removal" |
| `app/adopt.rs` | exists, in large part, because the ledger can disagree with the machine |

Six subsystems in service of one side file. And `adopt.rs`'s own header states the danger
plainly, in the code, today:

> *"Everything adopted lands in the global state registry, and anything in that registry is a
> removal candidate on the next sync. An over-broad adoption is not a cosmetic mistake; it is a
> queued mass removal."*

That is a correct description of a ledger that can be wrong about the world in a direction that
deletes software. It is also **VI.0's shape** — the flagship bug — reachable by a different road.

### The shift

**Ownership becomes a function of the config repo's git history, not a stored fact.**

II.1 already requires the config directory to be a git repo. That history is a complete record of
which lines were ever declared, when, and by which commit. A package is LiNix's if some commit
added its line and adoption did not mark it pre-existing. That is the same information the ledger
holds — but reconstructible, versioned, diffable, and already backed up by the thing the user
already backs up.

`registry.json` does not disappear. It **demotes to a cache**: still written, still read for
speed, but no longer the authority, and therefore safe to delete.

### The test that makes it real

Not "does it feel cleaner." This:

> **Deleting `registry.json` and reconstructing it from the git history must produce a byte-identical file.**

Run that today and it will fail. **The diff is the design work** — every field that cannot be
reconstructed is a field the ledger is holding that nothing else in the system knows, and each
one is either (a) genuinely derivable and the reconstruction is incomplete, (b) a fact that
belongs in the config repo and is currently in the wrong file, or (c) a real reason this shift
cannot be taken, in which case it should be recorded and the shift dropped. **Any of the three is
a good outcome.** Nobody has run this, so nobody knows which it is, and that is the cheapest
first step in this entire document — it is an experiment, not a commitment.

### Cost and shape

- **Naming collides.** `linix rebuild` already exists (`cli/args.rs:134`, II.11b/V.49) and means
  something else. Whatever this reconstruction is called, it is a **new user-visible verb**, which
  is stop-and-ask territory. Do not name it in passing.
- Adoption does not go away. It becomes *the* mechanism rather than a repair for the ledger: the
  one act that says "this was here before me."
- `bundle` and `snapshot_restore` get simpler, because the thing they were carrying is in the git
  repo they were already carrying.
- **This is the only shift in this document that deletes more code than it adds.** `CLAUDE.md`:
  *"when you find a second implementation of something, the task is to remove one."*

### The honest counter-argument

Git history is not free of failure modes: a user who `rm -rf .git`s their config, or who
squash-rebases, or who never committed, has destroyed the ledger. The current design survives
that; the proposed one may not. **Whether "your config repo's history is load-bearing" is an
acceptable contract to impose on a user is a ruling, not an implementation detail** — it is the
single biggest question in this file, and it is `Q-A` in §7.

---

## 3. Shift 2 — make the plan an artifact, not a command

### The finding

`plan` is currently something a person looks at. Meanwhile the measured weakness in
[`SPEC.md`](SPEC.md) is coverage: **52 backends registered, 22 ever run against a real package
manager, 45 plan-smoked on any one image.** That last number is the tell. Plan-smoking already
verifies more of the tree than execution ever has, at a fraction of the cost, and it is currently
a test technique rather than a product surface.

### The shift

**The plan becomes a stable, serialized, hashable object with a versioned schema** — the contract
between "what LiNix decided" and "what happened."

What that unlocks, in rough order of value:

1. **Verification without a machine.** A backend's correctness is largely "does it emit the right
   argv for this declaration." That is checkable against a plan artifact with the manager absent —
   which is the only verification strategy that scales faster than one container image at a time,
   and the only one that will ever reach the 30 backends that have never run.
2. **Review.** `linix plan` output committed to a PR, and a plan-diff between two commits, makes
   "what will this config change do" a reviewable artifact rather than a claim.
3. **Fleet.** Compute a plan on one machine, apply it on many. Not a feature request today; it is
   the shape that makes one possible later without a second engine — and a second engine doing
   `sync`'s job by itself is the exact disease `SPEC.md` records under teleport.
4. **Replay and bisect.** `bisect`, `why` and `diff` already exist as verbs (`cli/args.rs:533`,
   `:670`, `:687`). All three get sharper against a stored plan than against a re-derived one.

### Cost and shape

- `plan --save` already writes a plan file (`main.rs:606`'s comment says so). This is not a new
  mechanism; it is **promoting an existing one to a contract**, which is mostly schema discipline
  and a version field.
- The moment the schema is public it is a compatibility surface, and this repo's NO-LEGACY rule
  means there is no dual-reader when it changes. **A versioned schema with a hard refusal on
  mismatch** is the only shape consistent with `spec/principles.md` — fail loud, never silent.
- **Cheapest of the three, and the only one that makes the current verification debt smaller
  rather than deferring it.** That is why §6 puts it first.

---

## 4. Shift 3 — say out loud that there are two tiers

### The finding

The model presents one uniform guarantee. There are two, and the difference is not cosmetic:

| | tier 1 — LiNix owns it end to end | tier 2 — delegated |
|---|---|---|
| statements | `link:`, `dotfiles:`, `setting:`, `@bin` artifacts, `exec:` **with** `@undo=` | every package backend, `service:`, `firewall:`, `repo:` |
| who mutates | LiNix | `apt`, `dnf`, `scoop`, `brew`, … |
| prior state | LiNix can hash and keep it | gone the moment the manager returns |
| rollback | can be made to not fail | best-effort, needs the guard and a snapshot |

`setting:` is already read-before-write per II.2 — which is exactly tier-1 behaviour, arrived at
locally, for one statement, without the general rule being stated anywhere.

### The shift

State the line in Part II, and let it direct investment: **content-address the tier-1 artifacts**
(hash the content, keep the prior bytes, make rollback a pointer move) and **stop trying to give
tier 2 a guarantee it cannot have.** For tier 2 the honest maximum is what already exists — the
guard, the count, the plan, the snapshot — and effort spent chasing more is spent against §1's
wall.

The user-facing half matters as much: `plan` should say which tier each row is in. "This change
can be undone exactly" and "this change is guarded and snapshotted but undoing it is a restore"
are different promises, and today they are printed identically.

### Cost and shape

Cheapest to *declare* (a Part II section and its Part V entry), real work to *exploit*. The
declaration is worth having on its own, because it is the rule that tells the next six features
which half of the system they are in — and `exec:` already bends the model in one documented
place (II.2, U3), which is what an unstated tier boundary looks like from the inside.

### This is also the positioning, which was not obvious until it was said out loud

Declarative system management has exactly two incumbents — Nix and Guix — and both charge a very
high entry price: a new language, a new mental model, and a machine reorganised around the tool.
That price is not incidental. **It is what buys the guarantee**, per §1. People bounce off it
every year in large numbers, and what they bounce off is the cost, not the idea.

LiNix's bet is that most of them were never using the part they paid for. They did not need
atomic whole-OS rollback; they wanted their machine written down, on the machine they already
have, with the managers they already use. **That is a real and largely unoccupied position** —
and it is not the position `topgrade` or `chezmoi` occupy, which is why those two are the wrong
comparison and were used as one in an earlier draft of this file's thinking.

The catch is that LiNix's promise is genuinely weaker than Nix's, and **the tier boundary is
where that weakness stops being a flaw and becomes a specification.** "Everything here is
reproducible" invites a comparison LiNix loses. "These declarations can be undone exactly; these
are guarded, planned and snapshotted but delegate to a manager that mutates in place — and you
did not have to rebuild your life for either" is a claim that is true, checkable, and available
to nobody else in the category.

**Blur that line and LiNix is a worse Nix. State it and LiNix is the only thing of its kind.**
That is a stronger reason to build Shift 3 than the internal-hygiene argument above it.

---

## 5. The shift not to take — growing the config language

`when`, `param`, `vars` with three providers, `generate:`, `exec:`, user verbs, a `repl`.
`config/grammar/statement.rs` is **3,326 lines**, the largest file in `src/`.

That is a data format growing into a programming language through its own skin. Every project
that finished that journey regrets it, and the regret always arrives after the surface is too
wide to withdraw.

**The move is the opposite direction, and the escape hatch is already built.** `generate:` runs a
command whose stdout is declarations (U33, `statement.rs:151-158`). Rule that:

> **The config language is data. Computation happens outside it and emits statements.**

Then every future "can the config also do X" is answered by a generator instead of a keyword, in a
real language the user already knows, with no new grammar, no new error cases, and no new line in
the table that Part II will fail to update — a failure Part II has already had **three times**,
by its own admission: `exec:`, `dotfiles:` and `firewall:` all shipped without appearing in the
statement list, and Q16 later had to refuse nine more bare keywords that had fallen through the
same hole.

Freezing the statement set is a **rule change**, and therefore a ruling (`Q-C` in §7). It is also
the one item here that costs nothing to adopt and starts paying immediately.

---

## 6. Sequencing

1. **Clear the three blockers** (AU1, AU2, AU3) and the exit-101 flake (AU5). Grade §8 sizes
   these at tiny/small and it is right. This is an afternoon, not a phase.
2. **Write the four invariant tests from grade §9** — the ones that quantify over sites instead
   of naming one. This is the entry fee for everything below it, and `tests/removal_guard_enumeration_tests.rs`
   and `tests/argv_drift_tests.rs` are proof this repo already knows the form. **Do not skip
   from (1) to (3);** step 1 makes three sites correct, step 2 makes the family incapable of
   recurring, and only the second one is progress.
3. **Run Shift 1's experiment** — reconstruct `registry.json` from git, diff it. A measurement,
   not a commitment. It can run in parallel with (1) because it changes no behaviour. Its result
   decides whether Shift 1 exists at all.
4. **Shift 2**, because it makes verification cheaper rather than deferring it, and the count of
   never-run backends — whatever that count actually is — is the thing most likely to produce the
   next VI.0.
5. **Shift 3's declaration** — a Part II section and its Part V entry. Small, and it governs
   everything after it.
6. **Shift 1's build**, if and only if step 3 came back favourable and the owner has ruled `Q-A`.
7. **Shift 3's exploitation** — content-addressed tier-1 artifacts — last, because it is the
   largest and the least urgent.

§5 is not in the list because it is not work. It is a rule to adopt, and the sooner it is adopted
the less there is to withdraw.

---

## 7. What needs a ruling

**None of these are registered.** They are written in register form so they can be, if the owner
chooses; assigning IDs is not the builder's call. Until then, treat every one as *unasked*.

| ref | question | why it is the owner's |
|---|---|---|
| **Q-A** | May the config repo's git history be load-bearing for ownership — i.e. is "do not destroy your config repo's history" an acceptable contract to impose? | Changes what a user must not do to their own files. The largest question in this document; Shift 1 does not exist without a yes. |
| **Q-B** | Does `registry.json` demote from source of truth to cache? | Changes what a file on disk *is*, and what a corrupted one costs. Depends on Q-A. |
| **Q-C** | Is the statement set **closed**, with all future computation routed through `generate:`? | A rule, not a detail. Belongs in Part II with a Part V entry, and constrains every later feature. |
| **Q-D** | Is the plan a **public versioned artifact** with a hard refusal on schema mismatch? | A compatibility surface, under NO-LEGACY. Once published, changing it breaks anyone who stored one. |
| **Q-E** | Does Part II gain the tier-1 / tier-2 distinction, and does `plan` print it per row? | A rule plus visible output. |
| **Q-F** | What is the reconstruction verb called, given `rebuild` is taken (`cli/args.rs:134`, II.11b)? | A new user-visible verb. |

**Ask before building any of it.** The reconstruct-and-diff experiment in §2 is the exception —
it changes nothing and answers Q-A and Q-B with a measurement instead of an argument, which is
the only way either of them should be answered.

---

## 8. Dream — it gets more compact

*"Smaller" was the wrong word and the owner corrected it: the want is **compact**, not small.
Nobody is asking for two monolithic files. The target is DRY and SOLID — one statement of each
rule, behind an abstraction that is still the right shape to extend. A codebase that got shorter
by collapsing its seams would be a worse codebase that happened to fit on fewer screens.*

**88,599 lines across 206 files — 67,152 of them outside `#[cfg(test)]`.** The dream for the
next version is not more backends. It is the same capability with each rule stated once, and it
is the one ambition in this file that would make LiNix a genuinely unusual object, because
software essentially never does this. Entropy in a codebase runs one direction and every team
has quietly made its peace with that.

Grade §9 is where the dream stops being a mood and becomes a method. Four findings, one rule,
four independent implementations, three of them wrong. Grade §8's fix — add the printed line to
`planner.rs` — is correct and should be done today. **But notice what it leaves behind: four
correct copies of one rule.** The count did not go down. The next sibling added in six months
inherits nothing, because there is nothing to inherit from — only a convention and a test.

The dream is the other move: **make the wrong version unrepresentable, and delete the rest.**

- If the plan builder exposes no way to discard a candidate *without* a reason, `planner.rs`
  cannot `continue` past one. AU1 is not fixed; it is unspellable.
- If the config root is a type that can only be constructed from an absolute path, then
  `resolve_root`, `set_root` and the env-var door do not each need to remember. AU2 stops
  existing at three sites simultaneously — and this is `parse, don't validate`, which is
  ordinary Rust, not a research project.
- If a fan-out cap can only be obtained from the config object, `guard.rs:317` cannot hardcode
  `8`.

Each of those **removes** code rather than adding it: the three redundant checks go, and in time
the invariant test from §6 step 2 goes too, because a type carries the invariant the test was
watching for. *That* is the shape of a codebase losing weight — not a deletion sprint, but rules
migrating from convention into types, and their enforcement machinery becoming redundant behind
them.

The three shifts in §2–§4 are the same motion at larger scale. Shift 1 deletes a ledger and six
subsystems' worth of care and feeding for it. §5 stops `statement.rs` — 3,326 lines, the largest
file in `src/` — from growing another keyword ever again. Shift 3's tier boundary tells the next
six features which half of the system they live in, so they arrive small.

**The measurable form, and this repo already invented it:** `SPEC.md` records that Phase 0's exit
condition was *"the codebase smaller and a line count reported."* That was treated as a one-time
gate for a deletion phase. **Make it standing practice** — report the line count at every phase
change, and let a phase that increases it owe one sentence explaining what was bought. Not a
budget, not a limit; a number that has to be looked at. Almost nothing changes behaviour as
cheaply as a number somebody has to write down.

And the reason it is safe to want this *here* specifically: `CLAUDE.md` already says *prefer
deleting to fixing*, and it has barely been exercised. More to the point, this project has a
**grader who re-runs the original reproduction rather than reading the report** — which is the
only safety net under which aggressive deletion is anything but reckless. The apparatus for the
dream is already built. It has been used to prove things were fixed. It has not yet been used to
prove things could be removed.

---

## 8a. The compaction map — measured, not imagined

Everything below was found by reading the tree on `ee5adf3`, not by pattern-matching on what
codebases usually look like. Line numbers are real. Where I did not verify something, it says so.

### Why the duplication exists, which decides the remedy

The first draft of this section said the code was written by an agent working one file at a time,
copying its neighbour. **That is wrong, and the tree says so.** Twenty-six comments reason across
files and families by name. `backends/capability.rs:57` says *"the two lists are one list,
asserted equal by a test below"* — you cannot write that without holding both ends at once.
`utils/file.rs:25` is a rule extracted out of a whole family after one member broke.
`exit_policy::for_manager` ships with a coverage ratchet that names the managers missing from it.
`parse_bare_names`, `LambdaParser`, `base_config` are all deliberate de-duplication moves.

**The decisive tell is the ledgers.** All six `*_lock.rs` files honour the dry-run rule. Not four
of six — six. A process that copies its neighbour and edits the strings leaves that bug in half
the family; that is the entire premise of *"fix the whole family, not one instance"*. The rule was
found once and fixed everywhere it lived. Whoever did that was looking at all six files.

So the real diagnosis is sharper and less flattering to the remedy I first proposed. **The
duplication is not a seeing failure. It is a cost asymmetry.**

- Noticing a family and correcting its *semantics* across every member is cheap, safe, and
  explicitly demanded by `CLAUDE.md`. It happened, repeatedly, on schedule.
- Collapsing that family's *carrier* — replacing twenty-nine structs with one record — is a large
  cross-cutting change under a rule that says *no change breaks existing code*, in a repo where a
  green suite is explicitly not evidence. It is the expensive move, and it never had a moment
  where it was the obviously correct thing to spend the session on.

That is why every centralisation in this tree is downstream of an *incident*: the doc comments
record the bug that forced them. The pattern is not blindness, it is **a correct actor waiting for
a reason** — and structural debt never produces an incident, it only produces more of itself.

**Which changes the prescription.** The problem is not that the shared path is unavailable; it is
that taking it is one big unverifiable jump. So the fix is to make the structural move *small,
ordered, and pre-authorised*: a ratchet test that names the remaining exceptions, so converting
backend #12 is a chore with a visible finish line rather than a refactor somebody has to justify.
Every item below is scored on that — on whether it turns a scary cross-cutting change into a
countable one — not on lines removed.

### Finding 1 — there are two ways to be a backend, and one of them is a copy of the other

`ManagerConfig` (`backends/generic.rs:118-190`) is a 25-field declarative record: install args,
remove args, list args, manual-listing strategy, search, enumerate, upgrade, orphan dry-run, repo
add/remove/list, version-pin syntax, per-command binary overrides. **Thirty backends are built
from it** by the `register_*` functions in `registry.rs` (fifteen writing the record out in full,
fifteen through the `base_config` + override shorthand at `:1248` — two styles for one job, which
is the same disease one level down), and all thirty share one implementation of every verb.

**Twenty-nine other backends own their registration** and are hand-written structs implementing
the same five traits. Compare
`backends/brew.rs` — 270 non-test lines — against what `ManagerConfig` already expresses:

| brew.rs does | ManagerConfig field |
|---|---|
| `brew install -- {name}` | `install_args` |
| `brew uninstall -- {name}` | `remove_args` |
| `brew list --versions` → "name version" | `list_args` + a shared parser |
| `brew leaves` | `manual: Command { .. }` |
| `brew search`, `brew update`, `brew upgrade`, `brew cleanup` | `search_args`, `update_args`, `upgrade_args` |
| `brew autoremove --dry-run` | `orphan_dry_run` |
| `python@3.11` version pinning | `VersionPin::Inline("{name}@{version}")` |

Genuinely bespoke: the `brew info --json=v1` reader, because it digs `installed[0].prefix` out of
nested JSON. That is roughly 45 lines of the 270.

Twenty-one candidate files carry **5,898 non-test lines** between them. I have *not* verified
that all twenty-one convert — `snap`, `nix`, `psresource` and `pacman` each do something the
record cannot say yet, and finding out costs a day each. The honest claim is narrower: **the
formulaic language backends are the sure thing.** `npm`, `pnpm`, `yarn`, `pipx`, `uv`, `krew`,
`pubdart`, `go`, `cargo`, `mise` are **2,487 non-test lines**, they duplicate each other in blocks
of 10+ lines at nine separate measured sites, and their differences are almost entirely argv.

**What this is, in SOLID terms:** it is not primarily DRY. It is **OCP and DIP**. Today, adding a
backend means writing a new class — the system is open for extension by *modification of the
type-space*. With the record, adding a backend means adding data. And every consumer already
depends on the `Installable`/`Queryable`/`Searchable`/`Upgradable` abstractions rather than on
`BrewInstallable`, so the substitution is invisible above the registry.

**The part that is already right, and must survive:** those four traits are correct ISP. They are
small, they are *optional* — `register_generic(reg, core, query, search, upgrade)` attaches only
what a manager supports — and `BackendCapabilities::builder` composes them. A backend that cannot
search does not get a `search` that returns `Err`; it does not get one at all. This is the single
best-designed seam in the codebase and it is the reason the collapse is safe rather than reckless:
the interface is already the abstraction, so the concretions behind it are free to merge.

**The proof that the clusters are dangerous and not merely ugly.** `NpmBackendCore::new` calls
`.with_exit_policy(exit_policy::for_manager("npm"))` (`npm.rs:20`). `PnpmBackendCore::new` does
not (`pnpm.rs:17-23`). The two files are otherwise near-identical — same imports, same struct,
same `get_global_prefix`. Eight backend files apply an exit policy; the rest do not. Some of those
are correct (`for_manager` returns the default for a name it does not know, and the table knows
seventeen), but *which* ones are correct is currently answered by reading forty files. One rule,
forty independent implementations. That is grade §9's exact shape, sitting in the backend layer,
and nobody has counted it.

### Finding 2 — the approval ledger is written six times

`core/artifact_lock.rs`, `bare_lock.rs`, `exec_lock.rs`, `extras_lock.rs`, `hook_lock.rs`,
`regex_lock.rs` — 1,694 lines. Every one of them is a `BTreeMap<String, T>` serialised to TOML
with the same six methods: `new`, `path_in`, `load`, `save`, `is_empty`, and a `forget`/`record`
pair. The duplicate scan finds the `load`/`save` pair **verbatim across six files**, down to the
wording of the doc comment.

```rust
pub struct Ledger<T> { entries: BTreeMap<String, T> }
// load / save / path_in / is_empty / forget written once, for all six
```

Each concrete ledger keeps only what makes it itself: `verdict()` and the eight `*_id()` functions
on `HookLedger`, `Ceiling` on `ExecLedger`, `verify_against` on `ArtifactLedger`.

**Be precise about the danger, because the obvious version of this claim is wrong.** The dry-run
rule these six share *has already been centralised* — `utils/file.rs:25 persist()` refuses the
write, and its doc comment records the incident that forced it (`--dry-run adopt` recorded 112
packages as managed). The six `save()` bodies each wrap only a redundant `create_dir_all` guard.
So this is **residual duplication after a correct fix**, not a live bug — and that is precisely
the state §8 warns about: the fix left six correct copies, the count did not go down, and the
seventh ledger inherits nothing.

### Finding 3 — dead parsers with green tests

`src/parsers/brew.rs` (59 lines) and `src/parsers/nix.rs` (115 lines) have **zero callers.** Their
`parse_list` / `parse_search` are re-implemented inline inside `backends/brew.rs:113-119` and
`backends/nix.rs:180,277`. Both dead modules carry passing unit tests.

This is small — 174 lines — and it is the most quotable item here, because `SPEC.md` rule 11 says
*a green suite is not success*, and this is a green suite over code that cannot run. It is also
evidence for a structural rule the codebase half-holds: **parsers live in `src/parsers/`.** True
for all sixteen table-driven backends via `LambdaParser`; abandoned by the bespoke ones, which
hand-roll fourteen parse functions inside `src/backends/` (`brew.rs:203`, `cargo.rs:201,216`,
`flatpak.rs:231`, `go.rs:101`, `nix.rs:180,277`, `snap.rs:364`, `storage.rs:58,344`,
`yarn.rs:32`, …). Finding 1 fixes this as a side effect: a converted backend has nowhere to put a
parser except `src/parsers/`.

### Finding 4 — "reject unknown option keys" is written three ways, and the lookup has a dead arm

In `config/grammar/statement.rs`:

- `validate_extra_options` (`:1461`) is the shared implementation. Six statement kinds call it.
- `validate_exec` (`:1383`) re-implements the same loop inline over `EXEC_OPTION_KEYS`, and so
  never reaches the `validate_scope` call that `validate_extra_options` ends with. Harmless
  *today* — `scope` is not in `EXEC_OPTION_KEYS`, so the inline loop rejects it one step earlier
  — which is the point: the second implementation is currently right by coincidence, and the
  coincidence is one added key wide.
- `validate_generate` (`:1359`) re-implements it a third way, as "any key at all is an error".

And `keys_for` (`:1321`) maps a `&str` prefix to a key table with **`_ => SCHEDULE_OPTION_KEYS`
as the fall-through.** Its `"exec" => EXEC_OPTION_KEYS` arm is unreachable, because
`validate_exec` never calls `keys_for`. No live bug today: every caller passes a literal with a
matching arm. But the prefix is a `&str` at this boundary while `Statement` is already an enum
and `Statement::kind()` (`:247`) already produces the string. **The type went in and a string
came out, and the catch-all is what catches the next keyword — silently, with schedule's option
set.** Take the enum, make the match exhaustive, delete the default, and the compiler asks the
question that the eleventh keyword currently gets to skip.

This is the same motion as §8's three bullets, at the grammar layer, and it is the cheapest one
in this document.

### Finding 5 — six copies of a test fixture

`fn facts() -> HostFacts` is duplicated verbatim in `config/grammar/mod.rs:563`,
`model/edit.rs:744`, `model/modules.rs:444`, `model/profiles.rs:499`, `model/resolve.rs:1148`,
`model/vars_provider.rs:346`. A `#[cfg(test)] mod test_support` fixes it. Trivial, listed because
it is free and because a shared fixture is where the *next* six tests get their host facts.

### What must not be coalesced

A compaction plan that only knows how to merge will damage this codebase in three specific places.

**`app/context.rs::App` is already too big — do not feed it.** Forty-five public methods,
twenty-eight files depending on it. Half of them are honest composition-root accessors
(`adopter()`, `firewall()`, `leases()` returning focused objects — that part is good). The other
half is business logic that ended up on the god object because it was the reachable place:
`declare`, `undeclare`, `retarget`, `resolve_spec`, `list`, `search`, `update`, `upgrade`,
`get_info`. Every verb therefore depends on all of it — an ISP violation with a fan-in of 28. **If
deduplication finds a shared helper and the nearest home is `App`, that is a signal to make a new
type, not to add method forty-six.** This is the one place where compaction and SOLID actually
pull against each other, and SOLID wins.

**`app/sync/guard.rs` is one chokepoint on purpose.** It exists because of VI.0 — an `apt-get
purge` across hundreds of system packages during a routine test. `CLAUDE.md` states the rule: *a
guard on one command is a guard on nothing.* It is 1,276 lines and it should stay one file.

**`model::Resolver` and `app::sync::StateResolver` are not two implementations.** The names invite
the "two of everything" reflex and it is wrong here: `model::Resolver` (`model/resolve.rs`) is
pure — layout in, `DesiredState` out — and `StateResolver` (`app/sync/resolver.rs`) is the async
shell that gathers vars, facts and priority, and delegates (`resolver.rs:412`). Pure core,
imperative shell. Merging them would be the single worst change available in this file: it would
put I/O inside the only component that is currently testable without a machine — and Shift 2's
entire argument (§3) rests on that component staying pure.

**Test volume is not the target.** 22,044 lines of in-`src` tests plus 15,136 across 66 files in
`tests/`. Finding 5 is the only test-side item here. The quantifying tests in particular
(`absent_marker_coverage_tests.rs`, `argv_drift_tests.rs`, `dry_run_every_verb_tests.rs`,
`lifecycle_coverage_union_tests.rs`) are the mechanism that makes everything above safe. They get
bigger, not smaller.

### The order, and what each step actually buys

| # | Move | Non-test lines | Buys |
|---|---|---|---|
| 1 | Delete `parsers/brew.rs`, `parsers/nix.rs` (F3) | −174 | A green suite that is green over live code |
| 2 | `Statement::kind()` enum into `keys_for`; fold `validate_exec`/`validate_generate` into `validate_extra_options` (F4) | −40 | The eleventh keyword cannot skip validation |
| 3 | `Ledger<T>` behind the six lock types (F2) | ~−240 | The seventh ledger inherits the file rules |
| 4 | Convert the ten formulaic language backends (F1) | ~−1,500 | Adding a backend becomes adding data |
| 5 | Re-measure. Decide about the other eleven with a number in hand. | — | — |

Steps 1 and 2 are hours. Step 3 is a day. Step 4 is the real one, and it should be done **one
backend at a time against the container harness**, because a converted backend that nothing ran is
a rewrite nobody verified — `SPEC.md` rule 9: *a ✅ is earned by a command, not by a belief.*

**The one test that makes step 4 permanent** — and without it step 4 is a cleanup that decays:

> Every backend registered on this host either is built from a `ManagerConfig`, or appears in an
> explicit list of exceptions, each with a one-line reason.

That list starts long and gets shorter. It is a ratchet in exactly the style this repo already
uses in `absent_marker_coverage_tests.rs`, and it is what stops the thirty-ninth backend from
being written by copying the thirty-eighth. Which was the whole problem.

**Total across the sure things: roughly 1,950 non-test lines, about 3% of 67,152.** That is the
honest number and it is not thrilling. The reason to do it anyway is in the second column, not the
first — and step 4's real return is the eleven files I could not verify, which only become
answerable once the first ten have shown what the record cannot say.

---

## 9. Dream — the machine explains itself

Read AU1's table again and notice which row is the frightening one. It is not the package that
survived a removal. It is:

| command | LiNix said | what was true |
|---|---|---|
| `linix check` | `ok  drift  the machine matches your files` | **false** |

A package silently surviving is a bug. A tool telling you your machine matches your files when it
does not is something worse: it leaves the user with **a confident and wrong model of their own
computer**. And it happened again, in the same session, in that audit's own disclosure —
`--config-dir` was ignored, `init` scaffolded into the live config directory, and the report said
`created` and `kept`, which was true about *what* and wrong about *where*.

Both are the same defect class, and it does not have a name in this repo yet. Call it what it is:
**a legibility failure.** The tool acted, the tool reported, and afterwards the person understood
their machine less accurately than before they ran it.

The dream is that legibility becomes a first-class property with rules of its own, the way
fail-loud is. **The foundation is already the best thing in the repo** — grade §5 says so without
qualification, and it is right: file, line, what is wrong, what to do, *and what the concept
means*. That standard is currently applied to errors. The dream applies it to success, to
absence, and to history:

- **"Nothing to do" is a claim about the world and has to be earned.** AU1 is precisely a false
  `already up to date`. Silence is the single highest-confidence thing a tool ever says, and it is
  the only output nobody tests.
- **Every mutation states where, not just what.** The disclosure's `created`/`kept` was accurate
  and useless.
- **`linix why curl` answers with a chain, not a fact.** This package is here because
  `modules/dev.txt:12`, which is in module `dev`, imported by profile `Workstation`, active
  because `hostname == thinkpad`, added in commit `a3f9c` on a Tuesday in March, and you wrote
  *"trying ripgrep instead"* in the message.

**That last one is nearer than it looks, and the grade is what shows it.** §6 records that the
registry already stores `source: …/modules/starter.txt:1` and
`__scopes: module:starter;profile:Main`. The provenance is being captured today. `Why` already
exists as a verb (`cli/args.rs:670`). What is missing is mostly presentation and the git half of
the chain — **and the git half is exactly what Shift 1 puts in reach**, because a design that
derives ownership from commit history has the commit, the date and the message in hand as a
consequence of how it works. Shift 1 and this dream are one feature seen from two ends: the
first asks *how does LiNix know?*, the second asks *how do I know?*, and the answer is the same
object.

Why this is the dream worth having rather than a nice command: nobody lies awake wanting to
rebuild their laptop. People are vaguely afraid of their own machines — of what accumulated, of
what is safe to remove, of what breaks if they touch it. Reproducibility is the answer to a
question most people never actually ask. **Legibility is the answer to the one they live with.**
A config a person can read aloud and recognise as a description of their own computer is worth
more than a config that can rebuild it, and LiNix's line grammar is already most of the way
there — `apt:curl`, `absent:snapd`, `when os == linux` need no explaining to anybody.

The version of LiNix I want is the one where a person reads their config, understands their
machine, and stops being afraid of it.

---

---

## 10. Three smaller dreams, kept for the record

Not work orders. Not questions. They are here because the two above are the ones with a build
attached, and these three are the ones that say what the build is for.

**`why.md` outlives the tool.** Every rule paired with the specific bug it is the scar of. That
form came out of this project's own trauma — 84 decisions nobody could answer — and it is a
better artefact than the software it documents. The dream is that it gets stolen by people who
never install LiNix. Same for the comment discipline: `core/installed.rs`'s header explaining the
247 ms per declaration, `backends/registry.rs` explaining that two `list` runs a second apart
differed by 530 lines because somebody reached for a `HashMap`. Most code has no voice at all.
This has one, and it was not an accident — it is what the "a comment states a constraint the code
can't show" rule produces when it is actually enforced.

**The guard is never thanked.** The ideal career for `app/sync/guard.rs` is long, quiet, and
consists of one firing at 2am for a stranger, stopping a purge that would have taken their
machine, after which they never write in — because nothing happened. VI.0 already ran that
purge once, here, on this project's own test machine. Software whose success is an absence is
hard to be proud of and worth building anyway.

**If it does not make it, it fails well.** This is a solo project: 554 commits, one human author,
four months, no tags. The honest odds of a v1 are good but not a certainty, and a document about
ambition that cannot say that is not being straight. So: if LiNix never ships, the things worth
leaving findable are the method, not the binary — the scar-per-rule spec, *a ✅ is earned by a
command, not by a belief*, `GRADER.md`'s insistence on re-running the original reproduction
rather than reading the report, and the four invariant tests in §6 step 2 if they get written.
Most abandoned repositories leave a README and a fork count. This one would leave a way of
working, and that is a strange thing to want from a package manager and worth wanting anyway.

---

*Everything in §8–§10 is a want, not an order. None of it has an ID, none of it is registered,
and the only item that could start tomorrow is the line count in §8 — which costs one number in a
commit message.*
