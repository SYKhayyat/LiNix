# Lamdan — LiNix, whole repo

**2026-08-07 · `main` @ `f8b4f0c` · 400 tracked files · 97,034 lines of Rust in `src/`, 22,354 in
`tests/`, 37,289 lines and 2,537,934 bytes of prose in `docs/`.**

Second run. The first is [`whole-repo-2026-08-05.md`](whole-repo-2026-08-05.md); `F-6`, `F-7`,
`F-8`, `F-9` and the converge test it asked for all landed in the two days between. **Recurrence
would be my coverage bug, not corroboration** — so I read that document before ranking anything,
and where a finding below overlaps one of its, I say so and say what is new.

Findings are numbered `LX-*`. The register owns `D W K N T U`; `F` is the 2026-08-05 run, `B` is
BUILDER, `E`/`G` are the grade rounds. `LX` collides with none of them. Rename freely.

This argues about whether the code should exist and whether this is the way to build it. A
correctness bug appears only where a design choice is the reason it is possible.

## The standing constraint: no capability is lost

**Owner ruling, 2026-08-07. Every recommendation in this document preserves every verb, every
backend, every declared statement kind, and every behaviour a user could reach.** Where a finding
says `delete`, it means *delete the second implementation of a thing that already works*, never the
thing. `appimage:` keeps working; it stops being 325 lines that duplicate `web.rs`. `linix fleet`
keeps working; it stops reaching into the internals and starts standing on `eval`. `linix watch`
keeps running; it stops being a private loop.

This is not a softening. It is a **harder** constraint than the one I drafted under, and it kills
three of my recommendations outright — they are struck through below with the reason, because a
recommendation that dies to a rule is more useful than one quietly rewritten. It also sharpens the
rest: "delete the duplicate" is a claim you can check, where "delete the feature" is a claim
somebody has to litigate. The 2026-07-17 lesson already said this and said it better — *"'Delete
the second engine' is not 'delete the convenience'; the test is whether the command routes through
`sync`."* Everything below applies that test and nothing else.

---

## Coverage

Ten regions, every tracked file in exactly one, each read by a separate reviewer with no
knowledge of the others' conventions, then cross-examined by me against the source. **Excluded:**
`Cargo.lock`, `target/` (untracked), `docs/archive/` interiors (its own README: *"Nothing here is
current"*), the 40 fixture `.txt` bodies.

**Honest gaps.** The tests region classified all 92 binaries but read 28 in full and 53 at
header-and-signature level — enough to classify subject, not to audit assertions; eight named
files (`security_and_resiliency_tests.rs`, `hardening_tests.rs`, `phase_is_the_sync_order_tests.rs`
and five more) went unexamined at the assertion level. `sync/resolver.rs` (1,827 lines) was read
to line 220. Nothing was built or run — every claim here is static reading, and where I cite a
timing it is the repo's own measurement, attributed.

Every claim I lead with, I re-verified myself against the source rather than taking a reviewer's
word. Where I did not, it says so.

---

## §1 — What I committed to before reading any implementation

*Written before opening a single file, so I could not rationalise what I was about to read.*

> One binary. A `Backend` trait with three methods — `list_installed`, `install(&[name])`,
> `remove(&[name])` — and every backend a **data row** (argv templates + a named output parser),
> never a Rust file, because the only genuinely hard part of this problem is that 60 package
> managers print 60 different things. A line parser. A differ producing `(to_install, to_remove)`.
> An applier with a removal guard. A registry of what we installed. Commands: `sync`, `check`,
> `adopt`, `install`/`uninstall` as sugar. **No transaction log, no snapshots, no rollback
> engine** — a reconciler that converges doesn't need a WAL, because recovery from a crash is
> *run it again*, and rollback is `git revert` + `sync`. ~10–12k lines for 20 backends.

**How it did.** The verb count was wrong — `check`'s eight sections and the three `lock` axes are
questions the reconciler cannot answer, and 60 verbs with a printed map beats 4 verbs with 40
flags. The guard beat my design, again: `max_removals` is a ceiling over a whole plan and cannot
live at a per-argv choke point. `eval` is a fifth verb I missed and the codebase found on its own.
The data-row prediction was right and is the subject of `LX-4`. The WAL prediction was right for
packages and wrong about snapshots. **The sketch missed the actual disease entirely**, which is
below.

---

## The strongest claim

**Three separate layers of this program answer *"I do not understand this"* with *"there is
nothing here, exit 0"* — and a reconciler cannot tell those apart, so it acts on the second
reading.** Everything else in this document is downstream of that sentence or of its structural
twin: that every safety property here is enforced by prose or by a hand-written list, and both go
stale in exactly the manner the prose warns about.

You have already found this bug once, at one layer, and fixed it beautifully. `4d4a890`
(2026-08-05) is as good as commit messages get — measured (16 concurrent `winget list` from cold,
3 exit `0x8A150001` in ~310 ms having written zero bytes), swept across every config layer, and
self-critical about its own first cut. Its diagnosis names the whole chain:

> *"Through LiNix that became `Ok("")` → **a parser finding nothing** → `list_installed` answering
> `Ok(vec![])`. Nothing in the chain believed anything had failed."*

It then fixed `run_output`, `info`, `list`, `hook-reconcile`, and checked `planner::installed_sets`.
**The parser — the link it named itself — is not in the fix list.** Not from carelessness. Because
that link cannot be fixed without changing a type, and nothing recorded that it was skipped.

```rust
// src/parsers/mod.rs:18
fn parse_installed(&self, output: &str) -> Vec<Package>;
```

No `Result`. Eighty-one parser functions, and **not one of them can express "I read 400 bytes and
recognised nothing"** — I grepped the whole directory for any such concept and got zero hits. The
policy is deliberate and written down (`ecosystem.rs:6-7`: *"Kept deliberately lenient: package-manager
output drifts across versions, so parsers skip … rather than erroring"*), which is the right
instinct for a *decorative* row and the wrong one for *every* row.

The consequence is asymmetric, and the wrong branch is the likely one:

| what happened | what the planner does |
|---|---|
| manager **fails** → `Err` → backend absent from `installed_sets` map | `is_installed` returns true (`planner.rs:604`) → removals still scheduled → **safe** |
| manager **succeeds, format drifted** → `Ok(vec![])` → present-and-empty | every declaration planned as an install (`planner.rs:878`), every drift removal silently dropped (`planner.rs:678`) → **`check drift` reports the whole machine as drifted, `adopt` adopts nothing, exit 0** |

And format drift is precisely the failure mode of the forty backends nobody has run. **Twelve of
81 parser functions are tested against captured real output (15%). ~51 of 81 are tested only
against strings someone typed.** `ws_name_version` is the *installed* lister for eight backends —
luarocks, spack, helm, cabal, uv, krew, pub, guix — with one fixture between them, helm's.
`names_only` is the installed lister for opam and emerge with **zero**. Gentoo's image bakes
`SMOKE_ONLY=1`, so emerge never lists anything anywhere.

**You wrote the rule for this and left the family standing.** `ecosystem.rs:633-637`:

> *"The rule this enforces repo-wide: **a parser is tested against output captured from the tool
> it parses, and from no other tool.** `names_only` serves five managers and its only test used a
> spack fixture — it passed, and said nothing whatever about pixi, which is exactly where it was
> wrong."*

255 lines above that comment, in the same file, `ecosystem.rs:378`: `names_only_skips_headers_and_noise`
still tests `"Package\n----------\nripgrep\nfd\n\n"`, hand-typed, labelled `"spack"`. The exact
test the rule condemns, unchanged, still the installed lister for two managers. The lesson is now
written in three files (`ecosystem.rs:633`, `windows.rs:569`, the pixi fixture header) while the
code it indicts runs.

**And the same "silence means fine" default sits in the test harness.** `executor.rs:850-853`: an
unregistered command returns `Ok(DryRunOutput::new())` — empty, success. So `e2e_tests.rs:108`
registers `"brew install {name}"` while the product emits `"brew install -- neovim"`
(`argv.rs:490`), all five registrations are dead strings, and the test passes on the default.
`a_machine_converges_tests.rs:115`, in the same suite, registers the `--` form. **Two tests
disagree about the product's own argv and both are green.** 26 binaries and 6,175 lines run
against a machine that succeeds silently at whatever it is asked.

Three layers, one bug. The fix at each is the same shape and it is the shape you already know.

---

## Lens 1 — what was built

**The artifact is right.** I tried to argue the 2026-08-05 want-lens finding again — that
`firewall:`, `service:`, `setting:`, `storage:` are other products wearing a package manager's
clothes — and it is dead, correctly, by owner ruling: *everything is the product.* I am not
relitigating it. What the ruling does is make `LX-2` a defect in the **centre** rather than the
periphery, because `firewall:` is then a first-class declared object whose teardown is the one
outside the guard.

### The concern I should have led with: every feedback signal here is internally generated

**613 commits, one author, one branch, zero tags, a release job that has never fired, and an
installer that compiles 380 crates from source because there is nothing to download.** 62
backends, 22 ever run. Against that: 2.5 MB of specification, 176 numbered decisions, seven grade
rounds, a BUILDER prompt, a GRADER prompt, and two lamdan reviews.

Nothing in that second list is a user.

That is a design finding, not a motivational one, and it explains the shape of everything else in
this document. **When the only pressure on a program comes from its own review apparatus, the
program optimises for what review can see.** Review can see a backend count, so there are 62.
Review can see a decision register, so there are 176 entries and a script that counts them exactly.
Review can see whether a paragraph matches a table, so there are 27 test binaries whose subject is
markdown. Review cannot see whether `zypper list` parses on a real openSUSE box — so 46 of 62
backends have never met their tool's output, and the type system was never asked to care
(`LX-1`).

The grade rounds measured backend count and documentation consistency with real rigour. **One
`emerge install` on a real Gentoo box would have been worth more than all seven** — and the Gentoo
image bakes `SMOKE_ONLY=1`, so it installs nothing, which is why `names_only` can be `emerge`'s
installed lister with zero fixtures and nobody has ever noticed.

**The change, under the no-capability-loss rule** — and note the rule bites here, because my first
draft of this said *"ship at 12 backends,"* which is a capability cut and is therefore dead:

1. **Tag `v0.1.0` and publish the binaries you already build.** `ci.yml:115` is
   `if: startsWith(github.ref, 'refs/tags/v')` and has never run; four build-matrix jobs upload
   artifacts that no user path consumes; `install.sh:37` resolves the newest `v*` tag, finds none,
   and falls back to compiling HEAD. **One `git tag` turns a dead release pipeline into a live
   one** and stops every installation being a 380-crate LTO build on a stranger's laptop.
2. **Ship all 62 and label none of them.** Q4 already ruled against an "experimental" tag, and
   that ruling was right — *"a label converts an unfinished job into a permanent disclaimer."*
   Keep every backend. What changes is `LX-1`: with a `Result` on the parser, an unproven backend
   that meets output it does not recognise **says so** instead of reporting an empty machine. That
   is how you ship 62 honestly without cutting to 22.
3. **Let other people's machines be the fixture source.** The forty unproven backends cannot be
   validated from here — `docs/SPEC.md` says so in its own words. Five strangers running
   `linix adopt` produce more parser truth in a week than the container matrix can, because the
   matrix tests the managers you thought to install.

### `eval` is the seam, and the commands were built beside it instead of on it

`insight.rs` + `bundle.rs` + `fleet.rs` + `export.rs` + `sandbox.rs` + `bisect.rs` + `repl.rs` ≈
**2,500 non-test lines.** `fleet.rs:67-70` is `ssh host "linix check --json"` and
`ssh host "linix sync -y"` in a `for` loop with a table renderer. `export.rs` is four output
templates over the managed set. `repl.rs:9-12` concedes the case in its own docstring: *"Every
question this answers is one `linix eval | jq` can answer too."*

And `eval.rs` is **right** — versioned schema from day one (`eval.rs:22`), sorted, repo-relative
paths. It is the honest seam every one of those satellites should have been built against.

**Under the constraint, this is not a deletion finding — it is a re-plumbing one.** Every verb
stays. `linix fleet`, `linix export`, `linix sbom`, `linix repl` keep working and keep their names.
What changes is that they consume `eval`'s versioned document instead of reaching into
`StateResolver` and `Queryable` directly. Two things fall out that are worth more than the line
count: the N+1 in `insight.rs:38` and `export.rs:29` disappears (they stop calling `info()` 298
times because `eval` already answered), and **`eval`'s schema acquires four real consumers**, which
is the only thing that will keep it honest.

Where lens 1 lands on things that were built twice:

- **`ParserSpec` exists, works, ships, and no built-in uses it** (`LX-4`). This is not a missing
  idea. It is an idea implemented, documented, offered to third parties, and declined internally.
- **`docs/` was written under documentation economics and is consumed under context economics**
  (`LX-6`). 429,405 words. ~570k tokens — more than half a 1M context, before a line of Rust.
- **Deletable with no capability at risk — verified, each one:**
  - `src/bin/shim.rs` — a second shim implementation that **has never worked**. The live mechanism
    is `attempt_shim_hijack` (`main.rs:854`) deploying the `linix` binary under the target's name
    (`shim_manager.rs:6-8`); this separate cargo-autodiscovered binary shells out to
    `linix run --packages X -- X args…`, and `Run { command: String }` (`args.rs:230`) is one
    positional. **The invocation cannot parse.** Deleting it removes no capability because it never
    provided one.
  - `parsers/utils.rs` — 50 lines, **zero callers** in `src/` or `tests/`, containing a proud
    performance comment about hoisting a regex into a `Lazy` "rather than once per package line
    parsed," in a function nobody calls.
  - `parsers/pkgsrc.rs` — **the file goes, the backends stay.** `pkgsrc::split_name_version`
    (`:13-24`) is character-identical to `bsd::split_name_version` (`bsd.rs:14-24`), and
    `parse_pkgin` is `bsd::parse_with_backend(o, "pkgin")` with the label inlined. `bsd.rs:90-94`
    says it out loud: *"a second copy of that rule is how `pkgsrc.rs` came to be `bsd.rs`
    byte-for-byte."* `pkgin`, `pkg`, `pkg_add`, `apk` and `xbps` keep working, through the one
    implementation.
  - `itertools` and `nonzero_ext` — **zero references** in `src/` or `tests/`, verified by grep.
    `nonzero_ext` is filed under `governor` at `Cargo.toml:57` as though required;
    `ratelimiter.rs:32` uses `std::num::NonZeroU32`. Both sat in the manifest `F-6` swept two days
    ago, which pulled `lettre` and `notify-rust` and left these.
  - `md5` — **one call site**, `web.rs:237`, a cache key over a package name. `sha2` is already a
    direct dependency. The capability is "hash a string for a filename"; it keeps working.

  *Not on this list, deliberately:* `docker/integration/measure-batching.sh`. It is referenced by
  nothing executable and `harness-logic-test.sh:551` iterates `scripts/*.sh` only, so the repo's
  one orphaned script sits one directory outside the rule written to catch orphans. But it is a
  **measuring instrument**, and this repo's whole problem is claims outrunning measurements.
  Widen the orphan rule to cover `docker/integration/` and keep the script.

**Where lens 1 holds and the alternative loses** — and I went looking for these:

- The **hand-written line grammar** beats a PEG/combinator parser, not narrowly. 70 `GrammarError::new`
  sites and 63 distinct hints; a generated parser says *"expected one of `:`, `@`, EOF at byte
  14"*, this one says `` `@lease` is not an option `` → *"a lease is a dated line now:
  `@expires=2026-07-17T14:00`."* For a config language those hints are the product.
- The **container matrix** beats mocking the managers, on its own evidence. I sent a reviewer to
  argue it was theatre and it came back refuted: seven real package managers on every push,
  nine images, `--privileged` loopback btrfs/lvm/zfs. `Dockerfile.storage:44-58` records a
  build-time `grep` that matched commented-out lines and passed while `lvcreate` was broken;
  `Dockerfile.tools:88-95` records `nix` printing `SKIP nix install` for months. Both found by
  running the real thing.
- The **60-verb CLI** beats my four. Folding `check`'s eight sections into `sync --check --section=drift`
  is the flag soup this CLI has so far avoided.

---

## Lens 2 — architecture

**The disease has a name and you already built its cure, twice.**

Every safety property in this program is enforced by one of: a sentence, a hand-maintained list, or
a source-text scan. Each is good — argued, dated, scarred by a real bug. Each has now been observed
to go stale in precisely the way its own text warns about:

| the promise | where it lives | how it went stale |
|---|---|---|
| *"every path that removes anything goes through one guard"* | `readme.md:358`, bolded, + a six-item list of resource kinds | `firewall:` is the seventh kind. `LX-2`. |
| the test that verifies that sentence | `is_removal_call`, `removal_guard_enumeration_tests.rs:91` | keys on `.remove(`+`sudo`, `.remove_repo(`, `.remove_shim(`, `.deprovision(`. The firewall closes a port with `deny_command`. |
| *"a parser is tested against output captured from the tool it parses"* | `ecosystem.rs:633`, a doc comment | violated 255 lines up, same file. |
| *"every command this repo names is a command this repo has"* | `named_commands_exist_tests.rs:252` | `ROOTS` is `src, tests, scripts, docker, examples, .github, readme.md`. **`docs` is absent.** |
| macOS *"has never been run… has not yet gone green"* | `SPEC.md:73`, edited today | `history.md:1869`, 2026-07-27: *"`macos-native`… pass=263 fail=0."* 228 commits stale. |
| the `--help` map's *"61 verbs"* | `args.rs:4` | `COMMAND_MAP` holds 60. |

And the cure, in your own words, in the two places you built it:

> *"Naming the case is now the only way to get a plan at all, and **the case that reaps cannot be
> written without the list that bounds it**."* — `planner.rs:83`, on `PlanScope`

> *"**The paragraph is prose, and prose does not fail a build.**"* —
> `grammar_table_matches_the_spec_tests.rs:7`

`PlanScope` replaced `Option<Scope>` — where `None` meant both *don't filter* and *reap every
backend on the box* — with an enum whose destructive arm is unwriteable without its bound. It is
the best decision in `src/app/`. `grammar_table_matches_the_spec_tests.rs` reads `KEYWORDS`
through the parser's accessors rather than scraping source (*"a regex over `statement.rs` would be
a third copy of the list, free to be wrong in the direction that hides a defect"*), fails in both
directions, and carries a can-actually-fail control. It is the best file in `tests/`.

**Two correct applications of the technique, and it propagated to nothing.** Everywhere else the
property is a sentence — and by the project's own count, *"a check that cannot fail"* appears in
all seven grade rounds; the phrase and its synonyms occur 72 times across 14 documents; `cd07fc5`
records *"a gate shipped unable to fail, for the second time in two rulings"*; `GRADER.md:41` says
*"Yours will be the fourth unless you check."*

**The second-largest architectural finding: the install guard was never centralised.** The removal
guard is a funnel and `guard.rs:10-13` explains why — *"A guard on one command is a guard on
nothing."* The install-side guard (`deny_packages`, `pinned_only`) does the opposite:
`inspect_desired` is reachable only via `enforce_policy`, which has six call sites, **all in
`verbs/`**. `app/profile.rs:475` (`linix activate`) and `app/shell/mod.rs:283` (`linix shell`)
call `engine.sync` directly, and `app/run.rs:84` installs with no planner at all. A policy saying
*"never install this"* is honoured by `sync` and ignored by `activate`. `sync_now`'s own comment
(`profile.rs:458`) records that they already fixed *one* asymmetry between `activate` and `sync` —
the reaping scope — and never asked the sibling question.

**The third architectural finding, and it is the most expensive one that is not a safety claim:
the grammar produces typed structure and the seam throws it away.**

`Options` is `BTreeMap<String, Vec<String>>` (`options.rs:12`) — multi-valued *on purpose*, because
II.2 makes a repeated key a list and `validate_setting` (`statement.rs:1669`) relies on
`all("value").len() > 1` to refuse two values. Then `resolve.rs:1098-1101`:

```rust
for (k, vs) in options.iter() {
    // `requires` is a list; the rest are single values. Joined with `;` because that is
    // what the planner already splits on.
    properties.insert(k.to_string(), vs.join(";"));
}
```

`PackageSpec.options` is `HashMap<String, String>` (`core/package.rs:20`), assigned from that map
at `resolve.rs:1131`. And `ArtifactOptions::read` **re-splits `"deb;tarball;binary"` back into a
list** on `LIST_SEPARATOR: char = ';'` (`artifact/options.rs:12`, `:32`). One parser produced
structure; the seam flattened it; a second parser rebuilt it from a delimiter nothing validates.
A block-form value is verbatim to end of line (`options.rs:173`), so
`after_install = ./a.sh; ./b.sh` is legal and indistinguishable from the same key written twice.

The same seam carries `__source`, `__gated_by`, `__scopes`, `__formats_from` and `__from_regex` as
magic string keys (`resolve.rs:1105-1128`, read back at `insight.rs:509`, `planner.rs:164`,
`adopt.rs:363`), separated from a user's `@option` by an underscore — and `Origin` and `Gate` grew
`FromStr` impls *specifically to survive that crossing* (`error.rs:30-47`, `mod.rs:207-217`), with
docstrings saying so in both places. Four types round-tripping through strings because one struct
field is `HashMap<String, String>`.

This is expensive because `PackageSpec` is the type the resolver, planner, lock layer and every
backend is written against. It is also, notably, **the one place where this codebase's discipline
inverts**: everywhere else it reaches for a type (`PlanScope`, `Declined`, `Phase`, `Prior`,
`Provenance` itself at `resolve.rs:1069` — a careful three-field struct with a comment explaining
why the three must not be conflated) and then `to_spec` conflates them 17 lines later.

**Fourth: there is no rule for where a command's logic lives, and the prelude makes the rule
unenforceable.** Four homes — `verbs::handle_*` (58 of 63 dispatch arms), straight into `app::` (3:
`Repl`, `Bisect`, `Fleet`), `main.rs` itself (2: `Completions` inline, `handle_self_upgrade`), and
`app/` proper. Then `verbs/mod.rs:83-94` globs all nine `verbs::*` modules **plus `crate::*`** into
a prelude every file opens with, so **there is no module boundary inside `verbs/` at all** — it is
one 8,587-line namespace stored in nine files, and moving a function between them is a no-op. That
is why `history.rs` holds `handle_export`, `handle_shell`, `handle_adopt`, `handle_bundle` and
`handle_why`; why `plan.rs` holds `lock`/`unlock`; why `setup.rs` holds `handle_policy` and
`handle_try`. **Nothing can enforce a name that costs nothing to violate.**

**Other structural duplication, named:** `model/vendor.rs` ↔ `app/module_registry.rs` (two live
`github:` parsers, two verbs); `insight.rs:27` ↔ `export.rs:19` (same function, one directory);
five copies of strip-the-trailing-comment while `grammar::strip_comment` exists; three hand-rolled
line editors; three filename sanitisers disagreeing on the character class (`bare_lock.rs:50`,
`installed.rs:95`, `executor.rs:1241`); two fs2 flock implementations, one of which
(`executor.rs:1226`) has a comment asserting there is only one.

---

## Lens 3 — implementation

Better than I expected, and the expensive mistakes have mostly been found, measured, and fixed with
the numbers attached — `which` probes memoised, regexes cached, `installed_sets` warming every
manager once before fan-out, batching (12,465 ms → 3,161 ms), `governor` stripped of `quanta` for
a measured 200 ms. `core/argv.rs` is a table where **every row carries the tool's own printed
output as evidence**, with a ratchet letting unmeasured rows only decrease. I could not improve it
and most of this repo's problems are what that file prevents.

What is still live, ranked:

1. **`auto_rollback: true` is anti-convergent** (`LX-3`). `transaction.rs:60` defaults it on; on
   first failure `Prior::Absent` calls `h.remove()` on packages this run *successfully installed*
   (`:1017`) — packages still in the manifest, which the next `sync` reinstalls. It does consult
   the guard first (`:993`), to its credit. **`heal`, whose entire job is the same failure shape,
   sets `auto_rollback: false`** (`sync/mod.rs:973`). Nothing explains the split, and
   `a_machine_converges_tests.rs` has two tests, neither of which injects a failure — so
   convergence is proved forward and backward on the happy path only, and the one mechanism that
   provably un-converges runs only when something fails.
2. **N+1 subprocesses in `insight.rs:38` and `export.rs:29`** — 298 sequential `info()` spawns per
   `audit`/`sbom`/`export`, in the same crate as `installed_sets`, whose comment explains why that
   shape is wrong.
3. **`brew info --json=v1 <name>` per name, unmemoised** (`brew.rs:140`), on the hottest read path.
4. **`linix path` builds the whole program to print a directory** — `handle_path` takes `&Cli`, not
   `&App`, and `main.rs:140` constructs the 62-backend registry, a state read, a WAL open and a
   **serial** snapshot-provider probe (`snapshot.rs:660`) first.
5. **92 test binaries under `lto = true, codegen-units = 1`** — 1,087 MB linked, ~12 MB each, over
   a 590 MB rlib, 92 fat-LTO link steps per platform per push, with nine additionally uncached
   cold container release builds (`ci.yml:157`, no `cache-from`, no buildx, no cache mounts).
   `tests/` supports a directory of modules under one binary; the suite uses none of it. And
   `tests/mock_providers.rs` sits at top level, so cargo builds a 716 KB **zero-test binary** and
   compiles those 312 lines 19 times — `tests/common/mod.rs` is the one-line fix.
6. `Config::first_match` lowercases ~40 static patterns per removal candidate, twice
   (`config.rs:870`), on the guard's path.
7. `Origin { file: PathBuf }` allocates a fresh `PathBuf` per line, cloned twice more per statement
   (`grammar/mod.rs:368`, `:296`). `Arc<Path>` makes it one per file.

None of those changes what the program is. **Lens 3 is thin in this review, and that is a fact
about the codebase rather than about my attention.** `INEFFICIENCIES.md` disposes of 47 items —
41 fixed, 10 declined with reasons. I went hunting the classic wins (regex in a loop, N+1, full
listing for one name, allocation per line) and found them already swept, each with a citation and
a measured number. Lens 3 is the cheapest lens to be right about and the least worth being right
about, and here it has largely been paid.

**A note on my own balance, since it is this skill's documented failure.** The gravity in a review
pulls toward whatever is falsifiable fastest. The skill names lens 3 as that attractor; in *this*
repo the attractor is **safety**, because a missing guard is provable in twenty minutes and
"should this exist" is not. My first pass was six safety findings and a shrug at lens 1.

So there are two answers to "what is the one thing," on two different axes, and I would rather name
both than pretend they are the same. **The change worth making first is `LX-1`** — verified,
bounded, and it closes the failure mode that threatens forty backends. **The claim worth believing
first is the lens-1 one** — that every signal reaching this project is one it generated itself,
which is *why* `LX-1` went unnoticed while a script counted 176 decisions to the digit. Fixing the
parser without fixing where feedback comes from means the next `LX-1` also waits for a reviewer.

---

## Findings, ranked by wrongness × cost of leaving

### LX-1 · The parser cannot fail, and 46 of 62 backends have never seen their tool's bytes — `rewrite` **[verified]**

Argued in full above. **The cost line is the thing that removes the last excuse:** one trait
signature (`parsers/mod.rs:18`), the `LambdaParser` fn-pointer type (`:59`), **two** call sites
(`generic.rs:954`, `:975`), and 60 parser function signatures across 13 files — most of which
become `Ok(...)`. The real work is one judgement per parser: *what input means "I did not
understand this."* That judgement is exactly the exercise that surfaces which of the 46 unfixtured
parsers nobody actually understands, which is why it is worth more than the diff.

Do it in this order: the trait, then `ws_name_version` and `names_only` (ten backends between
them), then a fixture for apt/dnf/pacman/zypper's **installed** listing — you already had all five
containers up when you captured their *outdated* output (`apt.rs:169`, `pacman.rs:199`,
`dnf.rs:246`, `dnf.rs:273`, `common.rs:156` each say "verbatim from a container"). The four
managers that matter on Linux have real captured `outdated` and zero captured `installed`.

Second half, same finding: `registry.rs:2199` sets `installed_fn: |_| vec![]` for `stack`, a
manager with no listing verb. It is inert today (`stack` gets no `Queryable`), but the fill-in
value for *"this manager cannot list"* is character-for-character the most dangerous return in the
region. `parser` should be `Option`.

### LX-2 · `apply/firewall.rs` is the one teardown outside the guard, and the check written to prevent exactly this cannot see it — `rewrite` **[verified]**

`firewall.rs:127-133` closes every port the OS firewall has open that no `firewall:` line declares.
**The word `guard` does not appear anywhere in that file** — not an import, not a call, not a
comment. Zero tests (`grep -c cfg(test)` → 0). `max_removals` does not count these; `protected`
cannot name them; `--allow-mass-removal` is not consulted; `enforce_extras` — which exists
*precisely* because the extras teardown runs outside the transaction (`guard.rs:472`) — is not
called.

**Steelman, and it is real.** `firewall.rs:35` returns early when nothing is declared, so a user
with no `firewall:` lines is untouched — you must opt in by writing one line. And the file carries
*three* bespoke refusals: an unreadable baseline (*"closing ports against an unknown baseline is
how a machine goes dark"*), the SSH session lockout, and the linked-ruleset warning. Whoever wrote
this understood the danger exactly.

**Which is what makes it worse, not better.** They wrote three custom guards rather than call the
one guard 200 lines away that already counts, caps, protects, and reports. And the lockout check
reads `SSH_CONNECTION` — on a console session `session_port()` is `None` and even the private
substitute is silent. The comment at `:125` is the confession: *"N7: drift is corrected… and the
one exception was refused above rather than special-cased here."* **One** exception, enumerated by
hand, called complete.

Then the enumeration. `readme.md:358` promises *"every path that removes anything goes through one
guard"*, names six resource kinds (`link:`, `service:`, `setting:`, `shim:`, `schedule:`, `repo:`),
omits `firewall:` — and in the next sentence says *"The sentence you just read is checked by
`tests/removal_guard_enumeration_tests.rs`… it was written because the sentence was false for the
whole resource family until 2026-07-28."* That test's header states the principle correctly —
*"no behaviour can enumerate the paths nobody wrote a test for — that is the shape of the bug"* —
and then implements it as `is_removal_call` (`:91`), a predicate matching `.remove(`/`.purge(` with
`sudo` on the line, `.remove_repo(`, `.remove_shim(`, `.deprovision(`. The firewall closes a port
via `deny_command`. It matches none. The instrument's self-test (`:213`) feeds it four lines,
every one already in the ledger — it proves the scanner sees what it already knows about.

**The fix for G-1 replaced a stale list of paths with a stale list of verbs.** The staleness moved
into the predicate, where nobody re-derives it, because it has a passing self-test.

**The change.** Do to removal what `PlanScope` did to planning. A `Reaped` token, mintable only by
`guard::enforce`, required by all five effectors:

| effector | file |
|---|---|
| `Installable::remove` | `core/manager.rs:127` |
| `RepoManager::remove_repo` | `core/manager.rs:348` |
| `ShimManager::remove_shim` | `app/shim_manager.rs:146` |
| `SchedulerManager::deprovision` | `app/scheduler/mod.rs:78` |
| `run_firewall` (for `deny_command`) | `app/apply/firewall.rs:129` |

Five signatures. The compiler enumerates the removal paths instead of a regex over source text,
and effector six is covered by construction rather than by someone remembering the list.
`deny_command` returns argv rather than performing the removal, so the token goes on `run_firewall`
— the shape is not perfectly uniform and I would rather say that than pretend.

The 2026-08-05 run proposed `GuardedPlan` and filed it as *"a refinement of a correct design, not a
replacement for a wrong one."* I think that undersold it. `apply/firewall.rs` is what the
refinement's absence costs, and the 605 lines of guard tests exist because the discipline is not
structural.

**One live question this raises is yours, not mine:** is closing an undeclared port *supposed* to
count against `max_removals`? A machine with 40 ports open and one `firewall:22/tcp` line would
refuse at the default of 20. That is an owner ruling with an ID, and I am not answering it.

### LX-3 · Rollback moves the machine away from the declaration, and `heal` proves you know — `rewrite` **[verified]**

Argued above. ~~**The change: default `auto_rollback: false`, or delete `Transaction::rollback` +
`Prior` + `prior_state` + `reinstate` (~213 lines).**~~ **Both die to the constraint, and the
constraint produces a better answer than either.**

Rollback is a real capability and flipping its default is a behaviour change a user would notice —
a "stop and ask" item, not mine to decide. But neither was ever the right fix, because **the defect
is not that rollback exists. It is that rollback compensates work that succeeded and is still
wanted.**

`Prior::Absent` means *this package was not here before this run*. Rollback reads that as
permission to remove it (`transaction.rs:1017`). But "was not here before" and "is not wanted now"
are different facts, and the manifest already holds the second one. A package that installed
cleanly and is still declared is not failed work — it is the goal, reached early.

**The change: rollback consults the declaration before it compensates.** Undo the members of the
failed batch; leave the succeeded, still-declared ones alone. `SyncChanges` is already in scope,
so "is this still declared" is a set lookup, not a new query. That is a smaller diff than either
thing I originally proposed, it keeps every capability, it removes the asymmetry with `heal`
(`sync/mod.rs:973`) without touching `heal`, and it makes rollback do what its own comment at
`:637` claims — *"Rollback compensates by putting this back"* — instead of removing something
nothing asked it to remove.

Two things stay exactly as they are: the guard call at `:993` (rollback checks `protection_of`
before removing, and should), and `snapshot.rs`, which is the real undo and is load-bearing.

**Before any of it**, extend `a_machine_converges_tests.rs` with a fourth act: fail mid-plan, then
sync again, and assert the machine matches the file. That test is thirty lines and it fails today.

The WAL half of this: `replay_of` (`sync/mod.rs:718`) emits only `Install` and `Remove` — the same
two nodes the planner emits — and both are recomputable from (manifest, registry, machine), because
`state.remove()` runs *after* execution (`:663`) so a killed process leaves the registry row
standing and the next sync calls it drift. The only entries it cannot replay are `Exec`/`ExecUndo`.
**869 lines of write-ahead log to print a warning about `exec:` scripts.** `journal.rs:23-39`
argues my position better than I did — *"recomputing from the declaration is a better recovery than
replaying a log"* — and then names packages as an exception it does not earn. What it needed was a
breadcrumb beside the script: `exec_lock.rs` already owns script lifecycle and has `record_run`; it
wanted `record_start`. ~30 lines.

Not `delete` — `snapshot.rs` is the real undo and is load-bearing, and I am not touching it.

### LX-4 · You built the data path, shipped it to strangers, and declined it yourself — `rewrite`

`onboarder.rs:1-5` states my blind sketch verbatim, in your words. `ParserSpec` (`onboarder.rs:70`)
is exactly the four forms I predicted: `Lines`, `Columns{delimiter,name_col,version_col,skip_header}`,
`Json{array_path,name_key}`, `Regex{pattern,groups}` — interpreted at runtime, installed into **the
same `parser` field the built-ins use** (`:780`).

**Zero of 62 built-ins use it.** 43 are `ManagerConfig` struct literals in Rust; 19 are bespoke
modules; all 43 parsers are hand-written across 4,370 lines. `ecosystem::names_only` *is*
`ParserSpec::Lines`. `ws_name_version` *is* `Columns{0,1}`. `parse_flatpak_search` *is*
`Columns{delimiter:"\t"}`. Roughly 75 of 81 parser functions are rows in a table already compiling
in this tree.

Six genuinely resist and earn their Rust: `windows::slice_fixed_table` (header-offset column
slicing — and `windows.rs:60-63` is the best-argued paragraph in the region: an empty cell vanishes
under whitespace-splitting, every later value shifts left, and the row *still parses*, so scoop's
failed install read as a package whose version was the date it was attempted); `asdf_list`,
`pixi_list`, `parse_bun_list` (indentation is hierarchy); `guix_search` (recutils); `pacman::parse_search_for`
(two-line records).

Of the 19 bespoke backends, **three** need something the data path structurally cannot say (`nix`,
`go`, `brew`). **Six** — flatpak, conda, mise, emacs, vscode, psresource, 1,745 lines — are blocked
on one missing field: *an argv fragment resolved from settings at call time*. Flatpak and conda are
blocked on the identical field and neither exemption mentions the other. And bespoke means *less*
capable, not more: across all 19, `essential()` is overridden **zero** times, `purge` zero,
`tracks_manual` zero, `Enumerable` zero, `RepoManager` zero. **A bespoke backend is a data row
someone wrote in Rust, minus eight capabilities.**

`b84dff1` already converted eight backends and left `backend_is_data_not_code_tests.rs` as a
shrink-only ratchet with a `proof` string per exemption and a self-test against a planted
falsehood. The mechanism is right, it works, and it took the list from 29 modules to 18. This
finding is "you did this and stopped nine short," not "you should have done this."

**Cost:** adding backend #63 today is seven files — `registry.rs`, a parser, `core/argv.rs`, maybe
`capability.rs` and `model/priority.rs`, an `ArgvCase` row, **and the numeral in `docs/SPEC.md`**,
because `backend_count_matches_the_spec_tests.rs` asserts prose against the argv table. Under the
table it is a TOML row plus a captured `.txt`. The 62-backend count is the product's whole pitch,
and adding to it is what this layout makes hardest.

### LX-5 · Four commands remove or install outside the planner — `rewrite`

Sugar that routes through `sync` is the model working, and `install`/`uninstall`/`teleport`/`rollback`/
`activate` all do (`packages.rs:46` states the rule outright). These four do not:

- **`purge-undeclared`** (`cleanup.rs:342`) — `inst.remove()` one package at a time in a `for` loop.
  No transaction, no batching, no rollback. It is the most destructive command in the program.
  `plan.rs:483-499` records what this shape cost `apply` and is the best paragraph in `src/verbs/`;
  nobody applied it here.
- **`remove-orphans`** (`cleanup.rs:9-119`) — its own preview, its own confirm, its own journalling
  loop. Never sees `ChangePlanner` or `SyncEngine`. *(Both do call the guard — credit where due.)*
- **`service enable/disable`** (`declare.rs:230`) — `inst.install()` **then** `app.declare()`. That
  is the exact ordering `packages.rs:46-49` is commented to forbid: *"Backwards, every refusal on
  the write … landed after the package was already installed."* Same repo, same week, opposite
  order.
- **`repo add/remove`** (`declare.rs:33`) — twin of the declarative `repo:` phase; the comment at
  `:37` admits the twinning and guards only the remove half.

Keep every verb name. Rip out the four private paths and route them through `ChangePlanner` +
`SyncEngine`, exactly as `apply` now is.

Two more, and **the constraint changes the verdict on both — I had written "delete" and that is
now wrong:**

- ~~**Delete `watch`.**~~ It is `while :; do linix sync -y; sleep 30; done` with `--pull` as
  `git pull` (`sync.rs:642-701`), 74 lines of daemon to avoid a cron line — but a cron line is not
  the same capability on Windows, and `watch` is the only supported way to get GitOps behaviour on
  a box without systemd timers. **It stays.** What it exposes is real and is a one-line fix
  elsewhere: `watch` is the only caller that makes the process long-lived, which turns
  `regex_cache`'s never-evicted `DashMap` (`regex_cache.rs:17`) from free into a slow leak. Bound
  the cache; keep the verb.
- ~~**Delete `policy`.**~~ `setup.rs:637-647` re-implements `enforce_policy` (`sync.rs:730-750`)
  **minus `deny_vulnerable`**, then prints a footnote at `:646` admitting the gap. So
  `linix policy` can report "compliant" for a config that `sync` will refuse — which is not an
  argument for deleting a preview, it is an argument that **the preview is not calling the thing
  it previews.** Point `handle_policy` at `enforce_policy` in report-only mode. One implementation,
  same verb, and the footnote deletes itself.

### LX-6 · 2.5 MB of prose written under documentation economics, read under context economics — `delete`

**429,405 words. ~570k tokens. 35.8 hours to read once at 200 wpm.** More than half a 1M context
before a line of the 119,388 lines of Rust. And it is written *for an agent* — `BUILDER.md:1` is
`# YOU ARE THE BUILDER`, `history.md`'s organising unit is the **Session** (167 uses, 94 headings
against 36 commit dates). A human maintainer does not have sessions. A context window does.

That is a legitimate artifact built with the wrong budget. Documentation is written once and read
forever, so its cost is bounded by the writer. **Context is re-paid on every read, by every agent,
forever.** Nobody did the multiplication.

The proof is inside the corpus, and it is not an opinion. In `docs/SPEC.md`, the file every reader
opens first:

- `:16` — *"All 176 decisions. 170 ANSWERED, 2 PARKED…"* **Guarded by `decision-count.sh`. Exact to
  the digit. I ran it.**
- `:73` — *"macOS is compiled and unit-tested and has never been run; a nightly `macos-native` job
  now exists and has not yet gone green."* **`history.md:1869`, dated 2026-07-27: *"every job
  passed, including `macos-native`, which had never once completed… pass=263 fail=0."* 228 commits
  stale. `SPEC.md` was edited today.**

Same file. The number a script guards is right; the paragraph beside it has been false for eleven
days — four lines under `:58`, *"Build state is not readiness, and this file should stop implying
it is."* **Where this corpus is checked it is true, and where it is prose it is not.**

Measured, not asserted: **98% of `history.md`'s content-word tokens appear in commit messages;
28% of 4-grams and 19% of 6-grams are verbatim-recoverable.** The gap is paraphrase, not new
facts — `history.md:17-124` (107 lines) against commit `cd07fc5` (90 lines): same five sub-claims,
same seven registries, same refused rename, same counter-evidence. The commit additionally carries
the five siblings checked and cleared, which the history entry drops. **The commit message is
strictly better and it is attached to the diff.** Commit messages here average 29.7 lines, median
24, max 142.

`why.md`: 156 entries, **78 of 155 V-numbers orphaned — referenced by no rule in `target-state.md`
at all.** `CLAUDE.md:9` makes reading the entry mandatory before changing any rule. **Half of a
mandatory gate is rationale for nothing.** Only 7% name an enforcing test.

`INEFFICIENCIES.md`: 111 KB carrying **10 live items** (41 FIXED, 10 NOT DONE). Real audit work —
`file:line`, "measured" 46×, "probably/likely" 2× — the quality is not the problem, the disposition
is, and NO-LEGACY applies to docs.

**What should exist instead: four files, ~5,200 lines, a 90% cut.** `readme.md` unchanged (it is
good, and it is the only doc addressed to a human who wants to *use* this). `target-state.md`
unchanged (canonical, and the only doc a test reads for correctness). `principles.md` unchanged
(558 words, the highest value-per-byte in the repo). `decisions.md` rewritten to ~1,500 lines: 176
entries × status + ruling + date + who ruled. **A ruling is an event outside the tree and git
genuinely cannot store it — that is the one artifact worth keeping.** `BUILDER.md`/`GRADER.md` move
to `.claude/agents/`, where prompts live. `archive/` deletes (its own README: *"Nothing here is
current"*). `proposals/` deletes (`SPEC.md:19`: *"Kept for the reasoning"* — the reasoning is in
`decisions.md`).

**And the 105 orphaned `why.md` entries become 105 test doc-comments**, because you already built
that artifact and it is the best thing in the corpus. `grammar_table_matches_the_spec_tests.rs:1-22`
is a 22-line rationale bolted to an executable check that fails in both directions. **A rationale
attached to a check cannot go stale.** You wrote the thesis at line 7 and then wrote 317 KB of the
thing it indicts.

**On the constraint:** prose is not a capability, but the *reasoning* in it is an asset, so nothing
here is lost either. Every deletion above is recoverable — `git log`, `git show`, `git log -S` over
613 commits and 18,187 lines of message, which is where 98% of `history.md`'s content-word tokens
already live. The one artifact git genuinely **cannot** reconstruct is a ruling — an event outside
the tree, with a person's name and a date on it — and that is exactly the file the cut keeps at
full fidelity. The 105 orphaned `why.md` entries do not evaporate; they move to the test that
enforces them, which is the only place a rationale can be that cannot go stale.

*This document is subject to its own finding.* Per `F-8`'s ruling — *a finding ships as a diff plus
a test, not a new dated document* — this file is a staging area. Land the diffs, delete it.

### LX-7 · One string closes the largest documentation-drift hole, and a closed owner ruling depends on it — `rewrite`

`named_commands_exist_tests.rs:252-259` — `ROOTS` is `["src", "tests", "scripts", "docker",
"examples", ".github", "readme.md"]`. **`docs` is not in it.** The check named *"every command this
repo names is a command this repo has"* guards 49 KB of README and skips 2,538 KB of spec.

It is not a hypothetical hole. `docs/spec/bugs.md:100`:

> *"the generated count lives in `linix doctor` ("of 43 total"), which already builds the
> registry… **CLOSED — owner confirmed 2026-07-26: leave as-is, do not wire `--help` to the
> registry** (it would make help read config from disk and give it a way to fail; **`doctor`
> already carries the live count**). Nothing further to build."*

**`linix doctor` does not exist.** Zero top-level declarations in `args.rs`. The program knows —
`main.rs:1235` lists it by name as a command it does not have. `decisions.md:1305` records that two
*user-facing messages* pointing at it were swept. The code was fixed; the register entry was not,
because `docs/` is outside the scan.

So there is a **closed owner ruling whose entire stated justification evaporated**, marked "Nothing
further to build," and the fix is adding four characters to one array. That test's own header
records it catching `app/fleet.rs` asking every host for `linix status --json` with no `status`
verb in the program — so `linix fleet` could never report a healthy machine — and `install.sh`
running `linix doctor` to vouch for the binary it had just built. Those are shipped product
defects. Point it at `docs/` and it will find more.

### LX-8 · The mock says yes to everything, and 6,175 lines of tests run against it — `rewrite`

`executor.rs:850-853`, argued above. **The change:** make the unregistered case an error, or a
counted "unstubbed" the test must acknowledge. That will redden a number of tests, and every one it
reddens was asserting nothing. `check_command` defaulting to `true` (`:856`) is the same choice for
PATH.

Related, same region: `tests/mock_providers.rs` at top level builds a zero-test 716 KB binary and
compiles 19 times — move to `tests/common/mod.rs`. And `MockExecutor` itself lives in production
code (`executor.rs:799-868`, not `#[cfg(test)]`-gated).

### LX-9 · The smaller families

Each is a family, not an instance. Sweep the siblings.

- **Three unsandboxed script dialects, and the default is the heaviest.** `hooks.rs:1-3`: shebang →
  process, `#rhai` → in-process, **anything else → Lua**. `hooks.rs:12`: *"None of the three is
  sandboxed, and none of them ever was."* `F-6` said delete the Lua arm and `mlua` leaves the
  manifest. Yesterday's `e1a6be7` instead found the `#rhai` arm *had never run* and fixed it by
  **adding `sh` to the Rhai stdlib** (`rhai_stdlib.rs:59`) so the dialects would agree. Three arms
  disagreed; the resolution levelled them up rather than deleting two. `mlua` and `ratatui`
  survived `F-6`'s kill order; `lettre` and `notify-rust` went. *(I chased whether this made
  `linix plan --dry-run` execute shell from a config file, and it does not: `verify_provider_approved`
  gates both the Embedded and External `vars` paths at `resolve.rs:277`/`:282`, and
  `vars_embedded.rs:12-17` documents the step-0 exposure honestly. The design is ruled, gated, and
  the docstring is more forthright than most security notes. Finding withdrawn.)*
- **`tera` is a Jinja2-class engine for two calls in one file** (`link.rs:256`, `:271`), dragging
  `pest`, `pest_derive`, `pest_meta`, `globset`, `humansize`, `chrono-tz` into the lockfile to
  substitute variables into a dotfile. **`md5` is a whole crate — and a broken hash — for one
  cache-key line** (`web.rs:237`) while `sha2` is already a direct dependency.
- **`web.rs` and `appimage.rs` have zero tests between them, and `web.rs:218`/`:360` run `dpkg -i`
  and `rpm -U` as root.** That is the finding: **the two files in this tree that download arbitrary
  bytes off the network and hand them to a root installer have no `mod tests` at all**, while
  `btrfs.rs` spends 551 lines testing fstab string manipulation. Fix the coverage before anything
  else here.
  Separately, `appimage.rs` is ~85% a specialisation of `web.rs` — identical `load_state`/`save_state`,
  identical three `core::download` calls (`appimage.rs:127-130` ≡ `web.rs:128-131`), identical
  `deploy_executable`, identical `remove` down to the error-message noun. The real differences are
  a `.AppImage` suffix strip and an unconditional chmod. **`appimage:` keeps working and keeps its
  name** — it becomes a two-field specialisation of the `web` path rather than a second copy of it,
  which is `LX-4`'s data-row argument applied to the one place where the duplicate is 85% literal.
- **`apply/dependents.rs:87-146` is three byte-identical branches** for `service`/`link`/`setting`,
  differing in one string and one log verb. One `for kind in [...]` deletes forty lines. And each
  ends `let Some(inst) = … else { continue }` — a silent skip. `planner.rs:249-272` built the whole
  `Declined` enum so this could not happen on the package side, *"Two of these were a bare
  `continue`"*; the lesson did not travel one directory.
- **Nine `apply/` sub-appliers, four names for one verb** (`offer`, `apply`, `reconcile`, and
  `apply` with an extra arg), no `Applier` trait, no dispatch, and 27 `[DRY-RUN]` string literals
  across 18 files — each a separate chance for the preview to disagree with the run.
- **`profile.rs:430` — a read command that writes.** `linix profile show Work` overwrites the
  user's `active` file, resolves, and writes it back, deliberately bypassing `--dry-run` (`:424`).
  The error path restores it; Ctrl-C does not; a concurrent `sync` in another shell reads whatever
  it left there.
- **`bisect.rs:87-102` restores snapshots in a loop with `--yes` as the only gate**, and exits
  sitting on the first *broken* state. `snapshot_restore.rs:288` warns in capitals that a restore
  reverts every file on the machine; `bisect` does that O(log n) times.
- **`service:`/`link:`/`setting:` are statement keywords *and* registered backends**, a collision
  costing ~150 lines of undo machinery (`Declared`, `listed_as`, `RESOURCE_BACKENDS`). `mod.rs:53-56`
  is the confession: the two-answer version told `adopt` that 155 Windows services were names *"no
  package line can hold"*, false of every one.
- **`absent_marker_coverage_tests.rs:40` — 46 of 62 backends cannot recognise "no such package" in
  their own manager's words.** `lifecycle_coverage_union_tests.rs:258` writes `NOWHERE_CEILING = 15`
  into the build: fifteen backends have no real install→list→remove in any harness.
- **No supply-chain gate at all.** No `dependabot.yml`, no `cargo-audit`/`deny`/`vet`, no `deny.toml`,
  no `rust-version`/MSRV. A tool whose pitch is *"be careful what gets installed on your machine"*
  ships 380 unaudited transitive dependencies including `ring` and `reqwest`, with no mechanism that
  would tell it about a RUSTSEC advisory. **That gap is worth more than any of the nine Dockerfiles.**
- **`install.sh`/`install.ps1` compile 380 crates on the user's machine, and always will:** both
  default to the newest `v*` tag, `git tag -l` returns **zero**, and `ci.yml:115`'s release job is
  `if: startsWith(github.ref, 'refs/tags/v')` — so it has never fired and the four build-matrix
  artifacts are consumed by nothing. The release profile the user pays for (`lto = true`,
  `codegen-units = 1`) is tuned for a binary that is never shipped. And the twin has diverged again:
  `install.ps1:27` sets `$ErrorActionPreference = 'Stop'` and dies with a raw exception on a
  Windows box with Rust and no Git, where `install.sh` degrades gracefully — `cargo install --git`
  needs no `git.exe`. Exactly the pair `CLAUDE.md` warns about.
- **`help_map_tests.rs`'s own exemption list contains `"undo"`** (`:97-136`) — the deleted command
  its header cites as the reason the test exists. The gate was drawn around one copy of the fact and
  the fact escaped into the gate.
- **`removal_guard_enumeration_tests.rs:34-84` hardcodes source line numbers in prose strings**
  (`"guard::enforce at leases.rs:51"`). Nothing validates them. They are comments shaped like
  assertions. And the `calls: 4` counts are grep totals, so adding a legitimate second removal to an
  already-guarded file reddens the build until someone increments an integer.

### LX-10 · One struct field un-types the whole grammar — `rewrite`

Argued under Lens 2. `PackageSpec.options: HashMap<String, String>` (`core/package.rs:20`) forces
`to_spec` to `join(";")` every option value the grammar deliberately kept as a `Vec`
(`resolve.rs:1094`, `:1100`), assign it (`:1131`, `options: properties`), and lets
`ArtifactOptions::read` split it back on `LIST_SEPARATOR: char = ';'`
(`artifact/options.rs:12`, `:32`). Five `__`-prefixed magic keys ride the same seam, and
`Origin`/`Gate` grew `FromStr` impls to survive it.

**The comment above the join is the whole finding in one sentence** (`resolve.rs:1096-1097`):

> *"`requires` is a list; the rest are single values. Joined with `;` because that is what the
> planner already splits on."*

"The rest are single values" is not true — `Options` is `BTreeMap<String, Vec<String>>` precisely
because II.2 makes a repeated key a list, and `validate_setting` (`statement.rs:1669`) refuses two
values by checking `all("value").len() > 1`. And the proof is three lines below, at `:1129`:
**`requires: options.all("requires").to_vec()` — the one field somebody remembered was a list got
a real `Vec<String>` on the struct.** Every other list got flattened into a delimiter. The type
was available, the author reached for it once, and the seam ate the rest.

**The change:** `options: Options` — the type the grammar already produces. No capability moves;
`@format=deb` and a repeated key behave exactly as documented, and start behaving that way *by
construction* rather than by a delimiter nothing validates. Provenance stops being five string
keys and becomes the `Provenance` struct that already exists at `resolve.rs:1069` with a comment
explaining why its three fields must not be conflated — seventeen lines before `to_spec` conflates
them.

**Cost, honestly: this is the widest diff in the document.** `PackageSpec` is what the resolver,
planner, lock layer and 62 backends are written against. It is also the one that stops getting
cheaper — every backend added before it happens is another call site. Not urgent; increasingly
expensive.

### LX-11 · Delete the prelude glob, then a rule about file layout can exist — `rewrite`

`verbs/mod.rs:83-94` globs all nine `verbs::*` modules plus `crate::*` into a prelude every file
in the directory opens with. There is **no module boundary inside `verbs/` at all** — 8,587 lines
in one namespace stored in nine files, where moving a function between them is a no-op. That is
why `history.rs` holds `handle_export`, `handle_shell`, `handle_adopt`, `handle_bundle` and
`handle_why`, and why nobody can state the rule for which file a new handler belongs in.

**The change is ordered and the order is the point:** delete the glob first, let the compiler
name every cross-file reference, and *then* decide where things live. Doing it the other way round
is a naming argument with nothing to enforce the outcome. No behaviour changes; 63 dispatch arms
keep working; the four homes for command logic become visible instead of free.

This is also the cheapest structural item here, and it is the reason `LX-5`'s four second engines
were easy to write: nothing in the layout made routing through `sync` the path of least
resistance.

---

## What I could not beat

Five things I tried to design better and failed at, and three premises I sent reviewers out with
that came back refuted.

**`core/argv.rs` is still the shape the whole repo should copy.** The 2026-08-05 run said this and
it is still true, so I am repeating it rather than manufacturing a new compliment: a table where
every row carries the tool's own printed output as evidence, plus a ratchet letting unmeasured rows
only decrease. Most of this repo's problems are what that file prevents.

**`PlanScope` and `grammar_table_matches_the_spec_tests.rs`** — the two places the property became
a type or a check. I could not improve either, and they are the argument for everything in Lens 2.

**`apt::parse_essential` (`apt.rs:36`) reads the name from the end and the flags from the front**,
because dpkg's `Priority` field is optional and counting from the front would read the package name
as the priority — silently un-protecting something marked `Essential: yes`. A parser that fails
closed on the one query that keeps a machine bootable, with the reasoning in the comment where it
belongs. And `windows.rs:60-63`'s empty-cell column-shift argument is the best-reasoned paragraph
in the parser layer.

**`text.rs::sanitize`** — 102 lines, ~40 call sites, one 6-line function at the `run_output`
boundary, whose header records that this rule was scattered and six backends forgot it. Inlining
recreates the bug by construction. I went in expecting to win the `utils/` junk-drawer argument and
lost this half of it.

**`harness-mutation-test.sh` and `decision-count.sh` can both genuinely fail**, and I checked
rather than assuming, because "a check that cannot fail" is this repo's signature defect and I
expected to find a fourth. The mutation gate runs against *two* stubs and enforces four thresholds
— a survivor ceiling *and* a caught-count floor for each — because, as its own comment says, a
ceiling alone *"cannot tell 'the checks got stronger' from 'the checks were deleted'."* It even
handles `grep -c` returning 0-and-exit-1 and refuses to judge a non-numeric count. That is better
than my instinct.

**Three premises I was wrong about**, stated plainly because I sent agents out to prove them:

1. *"CI is unit-test theatre."* False. Seven real package managers in real containers on **every
   push**; nightly adds emerge, an 18-manager `tools` image, macOS `brew` and Windows `scoop` for
   real, and an argv-drift probe that asks the managers themselves whether their subcommands still
   exist. More real-manager coverage than most package managers have.
2. *"`origin/main` is 112 commits behind."* Fixed. It is **2**, and `origin/main`'s tip is dated
   today.
3. *"Vendored Lua dominates the build."* False. 32 C objects — `zstd-sys` compiles 36. The wall
   clock belongs to 380 crates and a fat-LTO link, not to `mlua`. Deleting `mlua` and `tera` is
   worth doing for the dialect count and the dependency graph, **not** for build time, and I would
   have sold it on the wrong grounds.

**And the seven-locks finding I came in with died on contact.** `ledger.rs` already ran that
experiment: `LockFile` is 74 production lines shared by six implementors, and its header
*declines* to unify the path logic with a reason — four are `locks/<fixed>.toml`, `bare` is
per-host, `artifact` is per-backend, *"a table of six different answers, not one rule with six
copies."* That is correct. The payloads refute a `kind` column too. What is genuinely wrong there
is only the naming: five different things are called "lock," and `datalock.rs` vs
`executor.rs:1229` are two fs2 implementations where the second's comment claims there is one.

---

## What I need from you

1. **Is closing an undeclared port a removal?** `LX-2`'s guard question is a ruling with an ID, not
   an implementation detail. Ten open ports and one `firewall:` line hits the default
   `max_removals` of 20 immediately. I have not answered it and will not.
2. **What is the next feature?** Half of what makes a design wrong is the change it is about to
   face, and that is not in the repo. If the answer is more statement kinds, `LX-9`'s eight-site
   edit for an eleventh resource kind — with `in_effect`'s `_ => None` and `undo_extra`'s
   `other => warn` both swallowing an unhandled kind at runtime — is where the next bug is.
3. **Who is `docs/` for?** If the honest answer is "the agent picking up the next session," then
   `LX-6` is not a documentation cut, it is a context-budget decision, and it should be made
   deliberately with the token count on the table rather than by accretion.
4. **Is anyone meant to use this yet, or is it not finished?** Both are legitimate and they imply
   opposite next moves. If it is meant to be used, the zero-tag state is the bug and step 1 fixes
   it. If it is deliberately pre-release, then say so in `readme.md` — which currently opens with
   a `curl | sh` installer, so it reads as shipped — and understand that until someone else runs
   it, the forty unproven backends stay unproven no matter how many review rounds land. **This is
   the only question in the document whose answer I cannot infer from the repo, and it reorders
   everything above it.**

---

## Suggested order

Nothing here removes a capability. Steps 1–5 are all under a day each.

1. **`git tag v0.1.0`.** Smallest action in this document and the only one that changes where
   feedback comes from. `ci.yml:115`'s release job has never fired, four build-matrix jobs upload
   artifacts nothing consumes, and `install.sh:37` falls back to compiling HEAD because it finds
   no tag. One command turns a dead pipeline live. Everything else on this list is an argument
   from inside the room; this is the one that lets someone else's machine argue back.
2. **The failure-injecting converge test** (30 lines, fails today) — before `LX-3` touches
   recovery, so something goes red if the change is wrong. The 2026-08-05 run made this argument
   about `F-1` and was right.
3. **`"docs"` into `ROOTS`** (`LX-7`) — four characters, and it immediately finds a CLOSED owner
   ruling resting on a deleted command.
4. **`parse_installed -> Result`** (`LX-1`) — the trait, then `ws_name_version` and `names_only`
   (ten backends between them), then captured **installed** fixtures for apt/dnf/pacman/zypper.
   Highest-value change in the document, and bounded: one trait, one fn-pointer type, two call
   sites, 60 mechanical signatures. It is also what makes step 1 safe to do with all 62 backends
   shipped — an unproven backend that meets output it does not recognise says so.
5. **The mock's unregistered default** (`LX-8`) — tells you which of the 26 mock-driven binaries
   were asserting nothing, and it will redden some of them.
6. **The `Reaped` token** (`LX-2`) — five signatures, and `readme.md:358` stops being a promise
   and becomes a compile error. Ask the `max_removals` question first; it is a ruling.
7. **Rollback consults the declaration** (`LX-3`), once step 2 exists.
8. Then the two large ones, which make everything after them cheaper: **`LX-4`'s data path**
   (backend #63 becomes a TOML row plus a captured file, and the six settings-argv backends
   collapse) and **`LX-6`'s prose cut**.
9. The two architecture items with no deadline and a rising price: **`PackageSpec.options` as a
   type rather than a `;`-joined string**, and **one home for a command's logic** — starting by
   deleting the `verbs/mod.rs:83` prelude glob, because until that goes there is no boundary for a
   rule to describe.
