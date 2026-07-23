# Working in this repo

`docs/SPEC.md` is the way in: it holds the instructions and the map, and the spec itself is one
file per part under `docs/spec/`. Read it before changing behaviour, and record what you did
after — `spec/plan.md` stays the plan, `spec/history.md` says how far it got (P4). Every rule in
`spec/target-state.md` has a matching entry in `spec/why.md` explaining the bug it exists to
prevent; **do not change a target-state rule without reading its why entry first.**

**Every open question lives in `docs/spec/decisions.md`, with a status.** Do not answer one in
code. A decision the register calls open is the owner's to make; a decision it marks *built,
never ruled* is code that ran ahead of a ruling and is still theirs to reverse.

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

## Verify

`cargo build --all-targets` → `cargo test` → `cargo clippy --all-targets`. Report honestly:
unverified is not done, and a skipped step is a said-so, not a done.
