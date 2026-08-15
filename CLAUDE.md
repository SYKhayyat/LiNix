# Working in this repo

`docs/SPEC.md` is the way in: it holds the instructions and the map, and the spec itself is one
file per part under `docs/spec/`. Read it before changing behaviour, and record what you did
after — `spec/plan.md` stays the plan and the commit message says how far it got. Every rule in
`spec/target-state.md` has a matching entry in `spec/why.md` explaining the bug it exists to
prevent; **do not change a target-state rule without reading its why entry first.**

**Every open question lives in `docs/spec/decisions.md`, with a status.** Do not answer one in
code. A decision the register calls open is the owner's to make; a decision it marks *built,
never ruled* is code that ran ahead of a ruling and is still theirs to reverse.

## Asking while building (owner ruling, 2026-07-23)

Build without stopping for permission. **Stop and ask only for these four:**

1. Anything with an ID in the register — `D*`, `W*`, `K*`, `N*`, `T*`, `U*`.
2. Anything that changes behaviour a user would notice.
3. Anything that would remove a feature.
4. Anything where Part II looks wrong. **Do not fix Part II yourself.**

**Do not stop for** implementation detail, naming, file layout, test structure, or a choice
between two options that is invisible from outside the program. Make the call and put the
reasoning in the commit message.

**When a question is answered, the ruling ships in that same commit** — rewritten into
`decisions.md`, and into `target-state.md` plus `why.md` if it is a rule rather than a detail. A
ruling that lives only in a chat log is exactly the drift that made 84 decisions unanswerable.

## Comments (P6)

**A comment states a constraint the code can't show. Nothing else.**

- Not *what* the line does — the line does that.
- Not *where it came from* — git does that.
- Not *that it's good*, or which spec paragraph blessed it — that's narration, and V.42
  bans it. A comment that cites `V.n` to explain a design is usually narration; a comment
  that says "this must run before the snapshot, or the rollback has nothing to revert to"
  is a constraint. Prefer the second.

If you can express the constraint in a name or a type instead of a comment, do that.

## NO LEGACY

This is a rewrite, not a migration. There are **no compatibility shims, no old-format
readers, no dual code paths kept "just in case".** When a thing is replaced, the old thing is
deleted in the same change — including the config keys, the docs, and the tests that named
it. A green test suite over the old model is not progress; it means the old model still runs.

## Fix the whole family, not one instance

A bug you find is a **representative of a family**, not a lone instance. Before you call a
fix done, find the siblings and fix them in the same change:

- the same bug class in the adjacent code path
- the parallel field / the other branch of the same enum / the twin adapter
- every layer that carries the same value (the in-code default *and* the YAML/JSON default
  *and* the CLI flag *and* the docs)
- the other caller that copy-pasted the same pattern

A fix that patches only the one line reported and leaves its siblings live has not fixed the
bug — it has hidden it, because the reported symptom disappears while the class survives.

Also binding:

- **No change breaks existing code.** Every change must leave existing callers and behaviour
  working — verify that, don't assume it.
- **Test the family, not the finding.** Bug fix = write the failing test first, watch it fail,
  then fix. Then extend the test to the neighbouring cases too — each enum value, each score,
  each config layer, the twin field — so a sibling can't reappear unnoticed. Don't write a test
  that pins only the exact reported case.
- **Say what you covered.** When you report the fix, list the sibling sites you checked,
  including ones you decided were *not* affected and why.

Case in point: the container harness was fixed to stop asserting a package was gone with
`command -v` — which answers from the shell's hash table and keeps naming a deleted file —
while the identical check, and its twin bug of calling the `lx` shell function inside
`sh -c` where no subshell can see it, sat untouched in the Windows script for another hour.
One reported symptom, two live siblings.

## Changes

- One parser for `backend:name` (the grammar in `config/grammar/`). Anything that splits on
  `:` and trusts the prefix is a bug.
- Every path that removes calls the guard (`app/sync/guard.rs`); every install/change path
  calls the `[guard]` gate. A guard on one command is a guard on nothing.
- Prefer deleting to fixing. Two of everything is how this repo got into trouble; when you
  find a second implementation of something, the task is to remove one, not to reconcile them.

## Never wait

**Never run a command over ~30 seconds in the foreground.** Background it and start the next
independent piece of work in the same message. Waiting, polling and status-reporting are not
work, and a foreground long command blocks everything until it returns.

This repo is full of them — the suite is ~20 minutes, an image build 15-30, the container harness
another 10, `cargo mutants` longer than all of it. Completion arrives as a notification either
way, so backgrounding costs nothing and foregrounding costs the whole duration.

Two corollaries, both learned the hard way here:

- **Launch in the same breath as the edit that precedes it.** Do not finish the edit, narrate it,
  and launch afterwards.
- **Order by what unblocks the most.** Two image builds and a harness run have no reason to be
  sequential; a `cargo` command and another `cargo` command do, because they share the target
  lock. Contention is the only thing to schedule around.

## Verify

`cargo build --all-targets` → `cargo test --no-fail-fast` → `cargo clippy --all-targets` →
`cargo fmt -- --check` → `scripts/unix-check.sh`. Report honestly: unverified is not done, and a
skipped step is a said-so, not a done.

**This chain runs on Windows, so four of its five steps verify one platform of two — and nothing
used to say so.** `scripts/unix-check.sh` is the fifth step and the only one that compiles the
**45 `cfg`-gated blocks across 17 source files** the other four cannot see. It runs `cargo check`
in a container, because the cheap alternative does not exist: `cargo check --target
x86_64-unknown-linux-gnu` from this host dies in `mlua`'s vendored C build for want of
`x86_64-linux-gnu-gcc`.

**What it costs to skip it, measured rather than argued.** `d1b3618` named one private associated
const across a module boundary, under `#[cfg(unix)]` on both sides. The local chain was clean.
Every Apple, Linux and MSRV job went red, and so did all seven distro integration jobs — and
because the container harness *builds its binary in-image*, a tree that does not compile on Linux
takes every fault-injection check offline with it. So the same commit introduced a blocker and
disabled the only instrument that could have reported it, and both sat for 26 commits. The first
thing the restored harness did was hand back a second blocker from that same commit.

**Which behaviours only the container harness can verify — state this, do not re-run and hope.**
The Rust suite is hermetic: it drives mock providers, so any behaviour that depends on a *real*
manager's answer is outside what it can reach. A red harness is therefore not "a job to re-run
later", it is **this list of properties currently unverified**:

- the removal guard's OS-essential protection against a manager that actually reports one
  (the hermetic half of this now has a fixture — `guard.rs`'s `Essentials` — but only the
  harness sees the real query);
- crash and fault injection: `crash/midway`, `crash/completed`, `crash/groupkill`, and whether a
  killed process leaves the state the recovery tests *construct by hand*;
- a backend's real install → list → binary-on-PATH → remove lifecycle;
- argv and terminator behaviour of a manager as installed, rather than as the table infers it.

**`cargo fmt -- --check` is part of this chain, not a release-time afterthought.** CI rates it
fatal on every push, and it is the one gate a change with no logic in it can break: renaming
`nexus::` to `shall::` re-sorted two import groups past `petgraph`, which turned the whole board
red — main plus every open dependabot PR — on a commit that touched only names. The release
scripts catch it, but they run at release; this chain runs per change. Spelled as `ci.yml` spells
it, because a local gate stricter than CI refuses work CI would take.

**The hook is not optional, and it is not automatic.** `.githooks/pre-commit` refuses a commit
that gate would reject; `git config core.hooksPath .githooks` turns it on, once per clone, and a
clone that has not run it has no hook at all. `git commit --no-verify` is the bypass when you mean
it. The hook is formatting only — clippy and the suite take minutes, and a pre-commit hook that
takes minutes gets bypassed until it gates nothing.

**`--no-fail-fast`, always.** Without it, cargo stops at the first test *target* that fails, so a
failure in the lib abandons the whole integration suite and the run tells you about one defect out
of however many there are. Both release scripts and CI already pass it — this line was the only
place that did not.

**The integration suite is one target, `suite`, listed module by module in `tests/main.rs`.**
Cargo does not auto-discover `tests/*.rs` (`autotests = false`), so **a new test file does not run
until it is a `mod` in that file** — `every_test_file_is_in_the_suite` fails when the two
disagree, which is the only reason the arrangement is safe. To run one file:
`cargo test --test suite <module_name>::`.
