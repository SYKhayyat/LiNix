# Contributing to Shall

Thanks for looking. This document is the working agreement: what to read before you change
something, the conventions that are load-bearing rather than stylistic, and how to get a change
accepted.

**New here?** Read these three, in order:

1. [`README.md`](README.md) — what Shall does, from a user's side.
2. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — how the code is arranged and how a `sync`
   flows through it.
3. [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — build, run safely, test, debug.

Then come back here.

---

## Before you write code

**`docs/SPEC.md` is the way in.** It holds the map; the specification itself is one file per part
under `docs/spec/`. Read it before changing behaviour, and record what you did afterwards.

Two rules about it that are not negotiable:

* **Every rule in `spec/target-state.md` has a matching entry in `spec/why.md`** explaining the
  bug it exists to prevent. Do not change a target-state rule without reading its why entry
  first. Most of them look arbitrary until you read what they cost.
* **Every open question lives in `docs/spec/decisions.md`, with a status. Do not answer one in
  code.** A decision the register calls OPEN is the owner's to make; one marked *BUILT, NEVER
  RULED* is code that ran ahead of a ruling and is still theirs to reverse. If your change would
  settle a question in that register, say so in the PR and let it be ruled.

When a decision *is* ruled, the ruling ships in the same commit — rewritten into `decisions.md`,
and into `target-state.md` plus `why.md` if it is a rule rather than a detail. A ruling that
lives only in a conversation is exactly the drift that once made 84 decisions unanswerable.

`scripts/decision-count.sh --check` verifies that every number written about the register matches
the register. Run it if you touch it.

## Conventions that are load-bearing

### No legacy

This is a rewrite, not a migration. **No compatibility shims, no old-format readers, no dual code
paths kept "just in case".** When a thing is replaced, the old thing is deleted in the same
change — including its config keys, its docs, and the tests that named it. A green test suite
over the old model is not progress; it means the old model still runs.

Related: **prefer deleting to fixing.** Two of everything is how this codebase got into trouble.
When you find a second implementation of something, the task is to remove one, not to reconcile
them.

### Fix the whole family, not one instance

A bug you find is a **representative of a family**. Before calling a fix done, find the siblings
and fix them in the same change:

* the same bug class in the adjacent code path;
* the parallel field, the other branch of the same enum, the twin adapter;
* every layer carrying the same value — the in-code default *and* the TOML default *and* the CLI
  flag *and* the docs;
* the other caller that copy-pasted the pattern.

A fix that patches only the reported line has not fixed the bug, it has hidden it: the symptom
disappears and the class survives.

**Say what you covered.** When you report a fix, list the sibling sites you checked — including
the ones you decided were *not* affected, and why.

### Tests

* **Bug fix means the failing test comes first.** Write it, watch it fail, then fix. A test you
  never saw fail is a test you have not shown can fail.
* **Test the family, not the finding.** Extend it to the neighbouring cases — each enum value,
  each config layer, the twin field — so a sibling cannot reappear unnoticed.
* **No change breaks existing callers.** Verify that; do not assume it.
* Name test files as sentences describing the property, ending `_tests.rs`, and add each one as a
  `mod` in `tests/main.rs` — **a file that is not listed there does not run.**

### Comments

**A comment states a constraint the code cannot show. Nothing else.**

* Not *what* the line does — the line does that.
* Not *where it came from* — git does that.
* Not *that it is good*, or which spec paragraph blessed it — that is narration.

A comment citing a spec section to explain a design is usually narration. A comment saying "this
must run before the snapshot, or the rollback has nothing to revert to" is a constraint. Write the
second kind. If the constraint fits in a name or a type instead, do that and skip the comment.

You will notice existing comments that are long. They are long where they carry a measurement or
an incident that would otherwise have to be rediscovered — not as decoration. Match the standard,
not the length.

## Verifying

```sh
cargo build --all-targets
cargo test --no-fail-fast
cargo clippy --all-targets
cargo fmt -- --check
scripts/unix-check.sh
```

**Report honestly: unverified is not done, and a skipped step is a said-so, not a done.**

`--no-fail-fast` matters — without it, one failing target abandons the rest and you learn about
one defect out of however many exist. `scripts/unix-check.sh` matters most on Windows, where the
other four steps compile one platform of two; [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md#the-verify-chain)
explains what that costs when it is skipped.

Once per clone:

```sh
git config core.hooksPath .githooks
```

This is not automatic, and a clone without it has no hook at all. The hook checks formatting only.

## Commits and pull requests

* One logical change per commit. The commit message says what it did and **how far it got** —
  including what it did not finish.
* Put the reasoning for a judgement call in the message. Implementation detail, naming, file
  layout and test structure are yours to decide; write down *why* so the next person does not
  re-litigate it.
* Reference the decision ID (`D*`, `W*`, `K*`, `N*`, `T*`, `U*`) if your change touches one.
* Plain professional English in commits, PRs, code comments, docs and identifiers.
* If CI is red, check `gh run list --workflow=CI --event=schedule` before assuming it is you —
  push runs and nightly runs fail for different reasons, and the nightly jobs are the ones that
  drive real package managers.

## What to expect from review

Changes are read against the spec, not just against taste. The questions that get asked most:

* Which why-entry covers the rule you changed?
* Where are the siblings of this bug?
* What did you verify, and on which platform?
* Is this a second implementation of something that already exists?
* Does this answer a question that belongs to the owner?

## Licence

Dual-licensed **MIT or Apache-2.0**, at the user's option. Contributions are accepted under the
same terms, as Apache-2.0 §5 states for its half.
